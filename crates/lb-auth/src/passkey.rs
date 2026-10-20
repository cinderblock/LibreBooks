//! Passkeys (WebAuthn), through `webauthn_rp`.
//!
//! Two ceremonies, each a start/finish pair:
//!
//! - **Enrolment** (`/auth/enroll/start|finish`): with an enrolment token,
//!   register a passkey, creating the user for `first_admin`/`new_user` tokens
//!   or adding to an existing one for `add_device`. Start only checks the
//!   token; finish verifies the attestation and then, in one transaction,
//!   redeems the token, creates the user and passkey, and signs them in.
//! - **Sign-in** (`/auth/login/start|finish`): usernameless. The options
//!   carry no credential list, so the browser offers every passkey it has for
//!   this site (or fills one in with conditional UI), and the response's user
//!   handle and credential id find it here.
//!
//! What we ask for and check (webauthn_rp does the WebAuthn Level 3 steps):
//! discoverable credentials, user verification required on both ceremonies,
//! `none` attestation, ES256/EdDSA/ES384/RS256, the exact page origins of the
//! relying party chosen by the request's `Origin`, and signature counters
//! (`0` forever is fine, as synced passkeys do; going backwards is refused as a
//! sign of a cloned authenticator). The user handle is the user's 16-byte
//! UUID.
//!
//! The options are the Level 3 `PublicKeyCredential*OptionsJSON` dictionaries,
//! which `@simplewebauthn/browser` v14 takes as `optionsJSON` unchanged, and
//! the responses are what its `startRegistration`/`startAuthentication`
//! resolve to.

use std::num::NonZeroU32;

use lb_core::UserId;
use rusqlite::{Connection, OptionalExtension as _, params};
use webauthn_rp::bin::{Decode as _, Encode as _};
use webauthn_rp::request::auth::{
    AuthenticationVerificationOptions, AuthenticatorAttachmentEnforcement,
    SignatureCounterEnforcement,
};
use webauthn_rp::request::register::{
    Nickname, PublicKeyCredentialUserEntity, RegistrationVerificationOptions, UserHandle16,
    Username,
};
use webauthn_rp::request::{BackupReq, PublicKeyCredentialDescriptor};
use webauthn_rp::response::register::{CompressedPubKey, DynamicState, StaticState};
use webauthn_rp::response::{AuthTransports, Backup, CredentialId};
use webauthn_rp::{
    AuthenticatedCredential, DiscoverableAuthentication16, DiscoverableCredentialRequestOptions,
    PublicKeyCredentialCreationOptions, Registration,
};

use crate::ceremony::Ceremony;
use crate::clock::{self, dur_ms, from_ms, ms, opt_from_ms};
use crate::error::{AuthError, Result};
use crate::model::{
    ClientInfo, EnrolFinish, EnrolStart, EnrolStartRequest, EnrolmentPurpose, LoginFinish,
    LoginStart, NewSession, PasskeyId, PasskeyInfo, Principal, SessionKind, User, clean_name,
};
use crate::perm::Permission;
use crate::{AuthStore, enrol, secret, session, users};

/// A stored public key, as webauthn_rp decodes it.
type StoredKey = CompressedPubKey<[u8; 32], [u8; 32], [u8; 48], Vec<u8>>;

fn handle_for(user: UserId) -> UserHandle16 {
    match UserHandle16::decode(*user.0.as_bytes()) {
        Ok(h) => h,
    }
}

/// The `user.name` authenticators show beside the passkey. WebAuthn wants it
/// to be a PRECIS username (no spaces and the like), so a display name like
/// "Ada Lovelace" becomes "Ada.Lovelace".
fn username(display: &str) -> Username<'static> {
    if let Ok(u) = Username::try_from(display.to_owned()) {
        return u;
    }
    let dotted: String = display
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(".")
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | '@'))
        .collect();
    Username::try_from(dotted)
        .or_else(|_| Username::try_from("librebooks".to_owned()))
        .unwrap_or_else(|_| unreachable!("\"librebooks\" is a valid username"))
}

/// A default name for a passkey, from the browser that made it.
fn default_passkey_name(user_agent: Option<&str>) -> String {
    let Some(ua) = user_agent else {
        return "Passkey".to_owned();
    };
    let browser = if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Firefox/") {
        "Firefox"
    } else if ua.contains("Chrome/") || ua.contains("CriOS/") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        "Browser"
    };
    let os = if ua.contains("iPhone") {
        "iPhone"
    } else if ua.contains("iPad") {
        "iPad"
    } else if ua.contains("Android") {
        "Android"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Mac OS X") || ua.contains("Macintosh") {
        "Mac"
    } else if ua.contains("CrOS") {
        "ChromeOS"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        return format!("{browser} passkey");
    };
    format!("{browser} on {os}")
}

/// The passkeys `user` already has on this RP, so the authenticator doesn't
/// make a second one on the same device.
fn exclude_list(
    c: &Connection,
    user: UserId,
    rp_id: &str,
) -> Result<Vec<PublicKeyCredentialDescriptor<Vec<u8>>>> {
    let mut stmt = c.prepare(
        "SELECT credential_id, transports FROM passkey WHERE user_id = ?1 AND rp_id = ?2",
    )?;
    let rows = stmt.query_map(params![user.to_string(), rp_id], |r| {
        Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, transports) = row?;
        let id = CredentialId::<Vec<u8>>::decode(id)
            .map_err(|e| AuthError::Corrupt(format!("bad credential id: {e}")))?;
        let transports = u8::try_from(transports)
            .ok()
            .and_then(|t| AuthTransports::decode(t).ok())
            .ok_or_else(|| AuthError::Corrupt("bad passkey transports".to_owned()))?;
        out.push(PublicKeyCredentialDescriptor { id, transports });
    }
    Ok(out)
}

/// A name the person typed: trimmed, checked, and `None` if blank.
fn typed_name(name: Option<&str>) -> Result<Option<String>> {
    name.map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| clean_name("a name", n))
        .transpose()
}

fn backup_flags(b: Backup) -> (bool, bool) {
    match b {
        Backup::NotEligible => (false, false),
        Backup::Eligible => (true, false),
        Backup::Exists => (true, true),
    }
}

impl AuthStore {
    /// Which relying party serves a page at `origin`.
    fn rp_for(&self, origin: &str) -> Result<usize> {
        self.inner
            .rps
            .iter()
            .position(|rp| rp.origins.iter().any(|o| o == origin))
            .ok_or_else(|| AuthError::UnknownOrigin {
                origin: origin.to_owned(),
            })
    }

    fn ceremony_timeout(&self) -> NonZeroU32 {
        let ms = dur_ms(self.inner.config.ceremony_ttl).clamp(1, i64::from(u32::MAX));
        NonZeroU32::new(ms as u32).unwrap_or(NonZeroU32::MIN)
    }

    /// Start registering a passkey with an enrolment token
    /// (`POST /auth/enroll/start`). `origin` is the request's `Origin`.
    pub fn start_enrolment(
        &self,
        req: &EnrolStartRequest,
        origin: &str,
        client: &ClientInfo,
    ) -> Result<EnrolStart> {
        self.guarded(client, || {
            let rp_index = self.rp_for(origin)?;
            let rp = &self.inner.rps[rp_index];
            let now = self.now();
            let hash = secret::hash(&req.token);
            let typed = typed_name(req.display_name.as_deref())?;
            let (purpose, user_id, display_name, exclude) = self.read(|c| {
                let token = enrol::check(c, &hash, now)?;
                match token.purpose {
                    EnrolmentPurpose::FirstAdmin => {
                        if users::enabled_admins(c)? > 0 {
                            return Err(AuthError::AdminExists);
                        }
                        let name = typed.clone().or(token.display_name);
                        Ok((token.purpose, UserId::new(), name, Vec::new()))
                    }
                    EnrolmentPurpose::NewUser => {
                        let name = typed.clone().or(token.display_name);
                        Ok((token.purpose, UserId::new(), name, Vec::new()))
                    }
                    EnrolmentPurpose::AddDevice => {
                        let id = token.user_id.ok_or_else(|| {
                            AuthError::Corrupt("add_device token without a user".to_owned())
                        })?;
                        let user = users::get_enabled(c, id)?;
                        let exclude = exclude_list(c, id, &rp.id)?;
                        Ok((token.purpose, id, Some(user.display_name), exclude))
                    }
                }
            })?;
            let handle = handle_for(user_id);
            let shown = display_name.as_deref().unwrap_or("LibreBooks user");
            let user = PublicKeyCredentialUserEntity {
                name: username(shown),
                id: &handle,
                display_name: Nickname::try_from(shown.to_owned()).ok(),
            };
            let mut options =
                PublicKeyCredentialCreationOptions::passkey(&rp.webauthn_id, user, exclude);
            options.timeout = self.ceremony_timeout();
            let (state, client_state) = options
                .start_ceremony()
                .map_err(|e| AuthError::Passkey(e.to_string()))?;
            let mut json = serde_json::to_value(&client_state)?;
            // webauthn_rp names the RP after its id; show people the real name.
            json["rp"]["name"] = serde_json::Value::String(rp.name.clone());
            let challenge_id = self.inner.ceremonies.insert(
                Ceremony::Register {
                    state,
                    token_hash: hash,
                    purpose,
                    rp: rp_index,
                    user_id,
                    display_name: typed,
                },
                now,
                clock::add(now, self.inner.config.ceremony_ttl),
            )?;
            Ok(EnrolStart {
                challenge_id,
                options: json,
                purpose,
                display_name,
            })
        })
    }

    /// Finish registering a passkey (`POST /auth/enroll/finish`): verify it,
    /// redeem the token, and sign the person in.
    pub fn finish_enrolment(
        &self,
        req: &EnrolFinish,
        origin: &str,
        client: &ClientInfo,
    ) -> Result<(User, NewSession)> {
        self.guarded(client, || {
            let rp_index = self.rp_for(origin)?;
            let now = self.now();
            let hash = secret::hash(&req.token);
            let passkey_name = match req.passkey_name.as_deref().map(str::trim) {
                Some(n) if !n.is_empty() => clean_name("the passkey name", n)?,
                _ => default_passkey_name(client.user_agent.as_deref()),
            };
            let typed = typed_name(req.display_name.as_deref())?;
            let Some((ceremony, expires)) =
                self.inner.ceremonies.take_entry(&req.challenge_id, now)
            else {
                return Err(AuthError::ChallengeNotFound);
            };
            let Ceremony::Register {
                token_hash,
                purpose,
                rp,
                user_id,
                ref display_name,
                ..
            } = ceremony
            else {
                return Err(AuthError::ChallengeNotFound);
            };
            if rp != rp_index {
                return Err(AuthError::UnknownOrigin {
                    origin: origin.to_owned(),
                });
            }
            if !secret::same(&hash, &token_hash) {
                return Err(AuthError::TokenNotFound);
            }
            let name = typed.or_else(|| display_name.clone());
            // A new user must be named. Check before verifying, and keep the
            // ceremony if not, so the page can ask and resubmit the same
            // response without another authenticator prompt.
            let token = self.read(|c| enrol::check(c, &hash, now))?;
            if token.purpose == EnrolmentPurpose::NewUser
                && name.is_none()
                && token.display_name.is_none()
            {
                self.inner
                    .ceremonies
                    .put_back(req.challenge_id.clone(), ceremony, expires);
                return Err(AuthError::Invalid("please enter your name".to_owned()));
            }
            let Ceremony::Register { state, .. } = ceremony else {
                unreachable!("matched as Register above")
            };
            let rp = &self.inner.rps[rp];
            let json = serde_json::to_vec(&req.credential)?;
            let registration = Registration::from_json_relaxed(&json)
                .map_err(|e| AuthError::Passkey(format!("the browser's response is unreadable: {e}")))?;
            let options = RegistrationVerificationOptions::<String, String> {
                allowed_origins: &rp.origins,
                allowed_top_origins: None,
                backup_requirement: BackupReq::None,
                error_on_unsolicited_extensions: false,
                require_authenticator_attachment: false,
                client_data_json_relaxed: true,
            };
            let credential = state
                .verify(&rp.webauthn_id, &registration, &options)
                .map_err(|e| AuthError::Passkey(e.to_string()))?;
            let (cred_id, transports, handle, static_state, dynamic_state, _metadata) =
                credential.into_parts();
            if handle != handle_for(user_id) {
                return Err(AuthError::Passkey("the passkey is for a different user".to_owned()));
            }
            let Ok(cred_id) = cred_id.encode();
            let Ok(static_blob) = static_state.encode();
            let Ok(dynamic_blob) = dynamic_state.encode();
            let Ok(transports) = transports.encode();
            let (eligible, backed_up) = backup_flags(dynamic_state.backup);
            let config = &self.inner.config;
            self.write(|tx| {
                let redeemed = enrol::redeem(tx, &hash, now, user_id, name.as_deref())?;
                if redeemed.token.purpose != purpose || redeemed.user.id != user_id {
                    return Err(AuthError::TokenNotFound);
                }
                let id = PasskeyId::new();
                let inserted = tx.execute(
                    "INSERT INTO passkey (id, credential_id, user_id, rp_id, user_handle, static_state,
                                          dynamic_state, transports, sign_count, backup_eligible,
                                          backed_up, name, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                     ON CONFLICT (credential_id) DO NOTHING",
                    params![
                        id.to_string(),
                        cred_id,
                        user_id.to_string(),
                        rp.id,
                        user_id.0.as_bytes().as_slice(),
                        static_blob,
                        dynamic_blob.as_slice(),
                        transports,
                        dynamic_state.sign_count,
                        eligible,
                        backed_up,
                        passkey_name,
                        ms(now),
                    ],
                )?;
                if inserted == 0 {
                    return Err(AuthError::AlreadyRegistered);
                }
                let session = session::create(
                    tx,
                    user_id,
                    &session::Spec {
                        kind: SessionKind::Passkey,
                        credential_id: Some(id.to_string()),
                        label: None,
                        client,
                        idle: config.session_idle,
                        absolute: config.session_absolute,
                    },
                    now,
                )?;
                tracing::info!(user = %user_id, passkey = %id, ?purpose, "enrolled a passkey");
                Ok((redeemed.user, session))
            })
        })
    }

    /// Start a passkey sign-in (`POST /auth/login/start`). Usernameless: the
    /// options list no credentials, so they also suit conditional UI
    /// (`startAuthentication({ optionsJSON, useBrowserAutofill: true })`).
    pub fn start_login(&self, origin: &str, client: &ClientInfo) -> Result<LoginStart> {
        let now = self.now();
        self.inner.limiter.check(&client.rate_key(), now)?;
        let rp_index = self.rp_for(origin)?;
        let rp = &self.inner.rps[rp_index];
        let mut options = DiscoverableCredentialRequestOptions::passkey(&rp.webauthn_id);
        options.0.timeout = self.ceremony_timeout();
        let (state, client_state) = options
            .start_ceremony()
            .map_err(|e| AuthError::Passkey(e.to_string()))?;
        let json = serde_json::to_value(&client_state)?;
        let challenge_id = self.inner.ceremonies.insert(
            Ceremony::Login {
                state,
                rp: rp_index,
            },
            now,
            clock::add(now, self.inner.config.ceremony_ttl),
        )?;
        Ok(LoginStart {
            challenge_id,
            options: json,
        })
    }

    /// Finish a passkey sign-in (`POST /auth/login/finish`).
    pub fn finish_login(
        &self,
        req: &LoginFinish,
        origin: &str,
        client: &ClientInfo,
    ) -> Result<(User, NewSession)> {
        self.guarded(client, || {
            let rp_index = self.rp_for(origin)?;
            let now = self.now();
            let Some(Ceremony::Login { state, rp }) =
                self.inner.ceremonies.take(&req.challenge_id, now)
            else {
                return Err(AuthError::ChallengeNotFound);
            };
            if rp != rp_index {
                return Err(AuthError::UnknownOrigin {
                    origin: origin.to_owned(),
                });
            }
            let rp = &self.inner.rps[rp];
            let json = serde_json::to_vec(&req.credential)?;
            let auth = DiscoverableAuthentication16::from_json_relaxed(&json).map_err(|e| {
                AuthError::Passkey(format!("the browser's response is unreadable: {e}"))
            })?;
            let raw_id = auth.raw_id();
            let row: Option<(String, String, String, Vec<u8>, Vec<u8>, Vec<u8>)> =
                self.read(|c| {
                    Ok(c.query_row(
                        "SELECT id, user_id, rp_id, user_handle, static_state, dynamic_state
                     FROM passkey WHERE credential_id = ?1",
                        [raw_id.as_ref()],
                        |r| {
                            Ok((
                                r.get(0)?,
                                r.get(1)?,
                                r.get(2)?,
                                r.get(3)?,
                                r.get(4)?,
                                r.get(5)?,
                            ))
                        },
                    )
                    .optional()?)
                })?;
            let Some((passkey_id, user_id, rp_id, handle, static_blob, dynamic_blob)) = row else {
                return Err(AuthError::UnknownCredential);
            };
            if rp_id != rp.id {
                return Err(AuthError::UnknownCredential);
            }
            let corrupt =
                |what: &str| AuthError::Corrupt(format!("passkey {passkey_id}: bad {what}"));
            let handle = <[u8; 16]>::try_from(handle)
                .map_err(|_| corrupt("user handle"))
                .map(|h| match UserHandle16::decode(h) {
                    Ok(h) => h,
                })?;
            let static_state = StaticState::<StoredKey>::decode(&static_blob)
                .map_err(|_| corrupt("public key"))?;
            let dynamic_state = <[u8; 7]>::try_from(dynamic_blob)
                .ok()
                .and_then(|d| DynamicState::decode(d).ok())
                .ok_or_else(|| corrupt("state"))?;
            let mut credential =
                AuthenticatedCredential::new(raw_id, &handle, static_state, dynamic_state)
                    .map_err(|e| AuthError::Corrupt(format!("passkey {passkey_id}: {e}")))?;
            let options = AuthenticationVerificationOptions::<String, String> {
                allowed_origins: &rp.origins,
                allowed_top_origins: None,
                backup_requirement: None,
                error_on_unsolicited_extensions: false,
                // A synced passkey may sign in on the device itself one day
                // and through a phone (hybrid) the next.
                auth_attachment_enforcement: AuthenticatorAttachmentEnforcement::Update(false),
                update_uv: true,
                sig_counter_enforcement: SignatureCounterEnforcement::Fail,
                client_data_json_relaxed: true,
            };
            state
                .verify(&rp.webauthn_id, &auth, &mut credential, &options)
                .map_err(|e| AuthError::Passkey(e.to_string()))?;
            let dynamic_state = credential.dynamic_state();
            let Ok(dynamic_blob) = dynamic_state.encode();
            let (eligible, backed_up) = backup_flags(dynamic_state.backup);
            let user_id = users::parse_user_id(&user_id)?;
            let config = &self.inner.config;
            self.write(|tx| {
                let user = users::get_enabled(tx, user_id)?;
                let updated = tx.execute(
                    "UPDATE passkey SET dynamic_state = ?2, sign_count = ?3, backup_eligible = ?4,
                                        backed_up = ?5, last_used_at = ?6
                     WHERE id = ?1",
                    params![
                        passkey_id,
                        dynamic_blob.as_slice(),
                        dynamic_state.sign_count,
                        eligible,
                        backed_up,
                        ms(now)
                    ],
                )?;
                if updated != 1 {
                    // Deleted while we were verifying.
                    return Err(AuthError::UnknownCredential);
                }
                let session = session::create(
                    tx,
                    user.id,
                    &session::Spec {
                        kind: SessionKind::Passkey,
                        credential_id: Some(passkey_id.clone()),
                        label: None,
                        client,
                        idle: config.session_idle,
                        absolute: config.session_absolute,
                    },
                    now,
                )?;
                Ok((user, session))
            })
        })
    }

    /// `user`'s passkeys, newest first (`GET /auth/passkeys`).
    pub fn list_passkeys(&self, user: UserId) -> Result<Vec<PasskeyInfo>> {
        self.read(|c| {
            let mut stmt = c.prepare(
                "SELECT id, name, rp_id, created_at, last_used_at, backup_eligible, backed_up
                 FROM passkey WHERE user_id = ?1 ORDER BY created_at DESC, id DESC",
            )?;
            let rows = stmt.query_map([user.to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, bool>(5)?,
                    r.get::<_, bool>(6)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, name, rp_id, created, used, backup_eligible, backed_up) = row?;
                out.push(PasskeyInfo {
                    id: id
                        .parse()
                        .map_err(|_| AuthError::Corrupt(format!("bad passkey id {id:?}")))?,
                    name,
                    rp_id,
                    created_at: from_ms(created),
                    last_used_at: opt_from_ms(used),
                    backup_eligible,
                    backed_up,
                });
            }
            Ok(out)
        })
    }

    /// Rename one of your passkeys.
    pub fn rename_passkey(&self, by: &Principal, id: PasskeyId, name: &str) -> Result<()> {
        let name = clean_name("the passkey name", name)?;
        self.write(|tx| {
            let changed = tx.execute(
                "UPDATE passkey SET name = ?3 WHERE id = ?1 AND user_id = ?2",
                params![id.to_string(), by.user_id.to_string(), name],
            )?;
            if changed == 0 {
                return Err(AuthError::NotFound {
                    kind: "passkey",
                    id: id.to_string(),
                });
            }
            Ok(())
        })
    }

    /// Remove a passkey and end the sessions it signed in
    /// (`DELETE /auth/passkeys/:id`). Your own, or anyone's if you're an admin.
    /// You can't remove your own last way of signing in: you'd be locked out.
    pub fn delete_passkey(&self, by: &Principal, id: PasskeyId) -> Result<()> {
        let now = self.now();
        self.write(|tx| {
            let owner: Option<String> = tx
                .query_row(
                    "SELECT user_id FROM passkey WHERE id = ?1",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            let not_found = || AuthError::NotFound {
                kind: "passkey",
                id: id.to_string(),
            };
            let owner = users::parse_user_id(&owner.ok_or_else(not_found)?)?;
            if owner != by.user_id && !by.can(Permission::ManageUsers) {
                return Err(not_found());
            }
            if owner == by.user_id {
                users::ensure_not_last_credential(tx, owner)?;
            }
            tx.execute("DELETE FROM passkey WHERE id = ?1", [id.to_string()])?;
            session::revoke_by_credential(tx, &id.to_string(), now)?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_are_made_precis_safe() {
        assert_eq!(username("ada").as_ref(), "ada");
        assert_eq!(username("Ada Lovelace").as_ref(), "Ada.Lovelace");
        assert_eq!(username("  ").as_ref(), "librebooks");
    }

    #[test]
    fn passkey_names_from_user_agents() {
        let chrome_win = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36";
        assert_eq!(default_passkey_name(Some(chrome_win)), "Chrome on Windows");
        let safari_iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
        assert_eq!(
            default_passkey_name(Some(safari_iphone)),
            "Safari on iPhone"
        );
        assert_eq!(default_passkey_name(None), "Passkey");
    }
}
