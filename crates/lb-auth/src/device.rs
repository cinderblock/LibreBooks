//! Device keys: sign-in for clients without WebAuthn.
//!
//! The desktop app's thin-client mode runs in a webview, and WebKitGTK (Linux)
//! has no WebAuthn. So the app holds an Ed25519 key pair (private half in the
//! OS keychain), registers the public half with an enrolment token, and signs
//! in by signing a server challenge:
//!
//! 1. `device_challenge()` → 32 random bytes, single use, valid 2 minutes.
//! 2. The client signs [`device_login_message`]`(challenge, origin)`, where
//!    `origin` is the server address it connected to
//!    (`https://books.example.com`).
//! 3. `device_login(…)` checks the signature (`verify_strict`) and makes a
//!    session.
//!
//! The message is domain-separated (so a signature can't be lifted into any
//! other protocol) and names the origin (so a server the client was tricked
//! into talking to can't relay our challenge and replay the answer here).

use ed25519_dalek::{Signature, VerifyingKey};
use lb_core::UserId;
use rusqlite::{OptionalExtension as _, params};

use crate::ceremony::Ceremony;
use crate::clock::{self, from_ms, ms, opt_from_ms};
use crate::error::{AuthError, Result};
use crate::model::{
    ClientInfo, DeviceChallenge, DeviceEnrol, DeviceKeyId, DeviceKeyInfo, DeviceLogin, NewSession,
    Principal, SessionKind, User, clean_name,
};
use crate::perm::Permission;
use crate::{AuthStore, enrol, secret, session, users};

/// What a device signs to sign in:
/// `b"librebooks device login v1\0" || challenge || origin`.
pub fn device_login_message(challenge: &[u8; 32], origin: &str) -> Vec<u8> {
    const DOMAIN: &[u8] = b"librebooks device login v1\0";
    let mut m = Vec::with_capacity(DOMAIN.len() + 32 + origin.len());
    m.extend_from_slice(DOMAIN);
    m.extend_from_slice(challenge);
    m.extend_from_slice(origin.as_bytes());
    m
}

fn parse_key_id(s: &str) -> Result<DeviceKeyId> {
    s.parse()
        .map_err(|_| AuthError::Corrupt(format!("bad device key id {s:?}")))
}

impl AuthStore {
    /// Register a device's public key with an enrolment token
    /// (`POST /auth/device/enroll`). A `new_user` or `first_admin` token makes
    /// the user too. The device then signs in with [`AuthStore::device_login`].
    pub fn enrol_device(
        &self,
        req: &DeviceEnrol,
        client: &ClientInfo,
    ) -> Result<(User, DeviceKeyInfo)> {
        self.guarded(client, || {
            let name = clean_name("the device name", &req.name)?;
            let key = secret::unb64(&req.public_key)
                .and_then(|k| <[u8; 32]>::try_from(k).ok())
                .and_then(|k| VerifyingKey::from_bytes(&k).ok())
                .filter(|k| !k.is_weak())
                .ok_or_else(|| {
                    AuthError::Invalid(
                        "the device's public key isn't a valid Ed25519 key (32 bytes, base64url)"
                            .to_owned(),
                    )
                })?;
            let now = self.now();
            let hash = secret::hash(&req.token);
            self.write(|tx| {
                let redeemed =
                    enrol::redeem(tx, &hash, now, UserId::new(), req.display_name.as_deref())?;
                let id = DeviceKeyId::new();
                let inserted = tx.execute(
                    "INSERT INTO device_key (id, user_id, public_key, name, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (public_key) DO NOTHING",
                    params![
                        id.to_string(),
                        redeemed.user.id.to_string(),
                        key.as_bytes().as_slice(),
                        name,
                        ms(now)
                    ],
                )?;
                if inserted == 0 {
                    return Err(AuthError::AlreadyRegistered);
                }
                tracing::info!(user = %redeemed.user.id, key = %id, "enrolled a device key");
                Ok((
                    redeemed.user.clone(),
                    DeviceKeyInfo {
                        id,
                        user_id: redeemed.user.id,
                        name,
                        created_at: from_ms(ms(now)),
                        last_used_at: None,
                    },
                ))
            })
        })
    }

    /// Start a device sign-in (`POST /auth/device/challenge`).
    pub fn device_challenge(&self, client: &ClientInfo) -> Result<DeviceChallenge> {
        let now = self.now();
        self.inner.limiter.check(&client.rate_key(), now)?;
        let challenge = secret::random_bytes::<32>();
        let expires = clock::add(now, self.inner.config.device_challenge_ttl);
        let challenge_id =
            self.inner
                .ceremonies
                .insert(Ceremony::Device { challenge }, now, expires)?;
        Ok(DeviceChallenge {
            challenge_id,
            challenge: secret::b64(&challenge),
        })
    }

    /// Finish a device sign-in (`POST /auth/device/login`): check the
    /// signature over [`device_login_message`] and make a session. `origin` is
    /// the server address the client connected to (the request's `Origin`, or
    /// the server's public URL when the client sends none); it must be one this
    /// server is configured for.
    pub fn device_login(
        &self,
        req: &DeviceLogin,
        origin: &str,
        client: &ClientInfo,
    ) -> Result<(User, NewSession)> {
        self.guarded(client, || {
            let known = self
                .inner
                .rps
                .iter()
                .any(|rp| rp.origins.iter().any(|o| o == origin))
                || self
                    .inner
                    .config
                    .extra_device_origins
                    .iter()
                    .any(|o| o == origin);
            if !known {
                return Err(AuthError::UnknownOrigin {
                    origin: origin.to_owned(),
                });
            }
            let now = self.now();
            let Some(Ceremony::Device { challenge }) =
                self.inner.ceremonies.take(&req.challenge_id, now)
            else {
                return Err(AuthError::ChallengeNotFound);
            };
            let row: Option<(String, Vec<u8>, String, Option<i64>)> = self.read(|c| {
                Ok(c.query_row(
                    "SELECT user_id, public_key, name, revoked_at FROM device_key WHERE id = ?1",
                    [req.key_id.to_string()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?)
            })?;
            let Some((user_id, public_key, name, None)) = row else {
                return Err(AuthError::UnknownDeviceKey);
            };
            let key = <[u8; 32]>::try_from(public_key)
                .ok()
                .and_then(|k| VerifyingKey::from_bytes(&k).ok())
                .ok_or_else(|| {
                    AuthError::Corrupt(format!("bad public key for device {}", req.key_id))
                })?;
            let signature = secret::unb64(&req.signature)
                .and_then(|s| <[u8; 64]>::try_from(s).ok())
                .map(|s| Signature::from_bytes(&s))
                .ok_or(AuthError::BadSignature)?;
            key.verify_strict(&device_login_message(&challenge, origin), &signature)
                .map_err(|_| AuthError::BadSignature)?;
            let user_id = users::parse_user_id(&user_id)?;
            let config = &self.inner.config;
            self.write(|tx| {
                let user = users::get_enabled(tx, user_id)?;
                // Revoked between our read and now? Then no.
                let still = tx.execute(
                    "UPDATE device_key SET last_used_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
                    params![req.key_id.to_string(), ms(now)],
                )?;
                if still != 1 {
                    return Err(AuthError::UnknownDeviceKey);
                }
                let session = session::create(
                    tx,
                    user.id,
                    &session::Spec {
                        kind: SessionKind::Device,
                        credential_id: Some(req.key_id.to_string()),
                        label: Some(name.clone()),
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

    /// `user`'s device keys that still work, newest first.
    pub fn list_device_keys(&self, user: UserId) -> Result<Vec<DeviceKeyInfo>> {
        self.read(|c| {
            let mut stmt = c.prepare(
                "SELECT id, user_id, name, created_at, last_used_at FROM device_key
                 WHERE user_id = ?1 AND revoked_at IS NULL ORDER BY created_at DESC, id DESC",
            )?;
            let rows = stmt.query_map([user.to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, user_id, name, created, used) = row?;
                out.push(DeviceKeyInfo {
                    id: parse_key_id(&id)?,
                    user_id: users::parse_user_id(&user_id)?,
                    name,
                    created_at: from_ms(created),
                    last_used_at: opt_from_ms(used),
                });
            }
            Ok(out)
        })
    }

    /// Revoke a device key and end the sessions it signed in. Your own, or
    /// anyone's if you're an admin. You can't revoke your own last way of
    /// signing in (see [`AuthStore::delete_passkey`]).
    pub fn revoke_device_key(&self, by: &Principal, id: DeviceKeyId) -> Result<()> {
        let now = self.now();
        self.write(|tx| {
            let owner: Option<String> = tx
                .query_row(
                    "SELECT user_id FROM device_key WHERE id = ?1 AND revoked_at IS NULL",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            let not_found = || AuthError::NotFound {
                kind: "device",
                id: id.to_string(),
            };
            let owner = users::parse_user_id(&owner.ok_or_else(not_found)?)?;
            if owner != by.user_id && !by.can(Permission::ManageUsers) {
                return Err(not_found());
            }
            if owner == by.user_id {
                users::ensure_not_last_credential(tx, owner)?;
            }
            tx.execute(
                "UPDATE device_key SET revoked_at = ?2 WHERE id = ?1",
                params![id.to_string(), ms(now)],
            )?;
            session::revoke_by_credential(tx, &id.to_string(), now)?;
            Ok(())
        })
    }
}
