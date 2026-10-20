//! The shapes auth deals in. Types the web UI sees derive `ts_rs::TS` and are
//! exported to `web/src/api/generated/` when the tests run.

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

use jiff::Timestamp;
use lb_core::{CompanyId, UserId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::ceremony::ChallengeId;
use crate::error::{AuthError, Result};

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
        #[serde(transparent)]
        #[ts(export, type = "string")]
        pub struct $name(pub Uuid);

        impl $name {
            /// A fresh, time-ordered id.
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    };
}

id_type!(
    /// A signed-in session (not its secret token).
    SessionId
);
id_type!(
    /// A registered passkey (our id, not the WebAuthn credential id).
    PasskeyId
);
id_type!(
    /// A registered device key.
    DeviceKeyId
);
id_type!(
    /// An enrolment token (not the secret token itself).
    EnrolmentId
);

/// What someone may do. Ordered by power: `Viewer < Bookkeeper < Admin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Role {
    /// Reads books and reports; changes nothing.
    Viewer,
    /// Reads and writes books; no user or server administration.
    Bookkeeper,
    /// Everything, in every company.
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Bookkeeper => "bookkeeper",
            Role::Admin => "admin",
        }
    }
}

impl FromStr for Role {
    type Err = AuthError;
    fn from_str(s: &str) -> Result<Role> {
        match s {
            "viewer" => Ok(Role::Viewer),
            "bookkeeper" => Ok(Role::Bookkeeper),
            "admin" => Ok(Role::Admin),
            _ => Err(AuthError::Corrupt(format!("unknown role {s:?}"))),
        }
    }
}

/// A person who can sign in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct User {
    pub id: UserId,
    pub display_name: String,
    /// Server-wide role. See [`crate::Permission`].
    pub role: Role,
    pub created_at: Timestamp,
    /// Set when an admin disabled the account; a disabled user can't sign in
    /// and has no sessions.
    pub disabled_at: Option<Timestamp>,
}

/// An admin's change to a user (`PATCH /users/:id`). Absent fields stay as
/// they are.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, TS)]
#[ts(export)]
pub struct UserUpdate {
    #[ts(optional)]
    pub display_name: Option<String>,
    #[ts(optional)]
    pub role: Option<Role>,
    /// `true` disables the account (ending its sessions), `false` re-enables it.
    #[ts(optional)]
    pub disabled: Option<bool>,
}

/// How a session was signed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SessionKind {
    /// A browser with a passkey.
    Passkey,
    /// A desktop thin client (or CLI) with a device key.
    Device,
    /// The desktop app talking to its own in-process server.
    Local,
}

impl SessionKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            SessionKind::Passkey => "passkey",
            SessionKind::Device => "device",
            SessionKind::Local => "local",
        }
    }
}

impl FromStr for SessionKind {
    type Err = AuthError;
    fn from_str(s: &str) -> Result<SessionKind> {
        match s {
            "passkey" => Ok(SessionKind::Passkey),
            "device" => Ok(SessionKind::Device),
            "local" => Ok(SessionKind::Local),
            _ => Err(AuthError::Corrupt(format!("unknown session kind {s:?}"))),
        }
    }
}

/// One signed-in device, as listed on "my sessions" (`GET /auth/sessions`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct SessionInfo {
    pub id: SessionId,
    pub kind: SessionKind,
    /// The browser's user agent or the device key's name.
    pub label: Option<String>,
    pub ip: Option<String>,
    pub created_at: Timestamp,
    pub last_seen_at: Timestamp,
    /// When the session ends if not used again: the earlier of the absolute
    /// expiry and the idle timeout from `last_seen_at`.
    pub expires_at: Timestamp,
    /// This is the session making the request.
    pub current: bool,
}

/// A freshly made session. `token` is the secret cookie value; it is returned
/// exactly once and never stored. Its `Debug` hides the token.
#[derive(Clone, PartialEq, Eq)]
pub struct NewSession {
    pub token: String,
    pub info: SessionInfo,
}

impl fmt::Debug for NewSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewSession")
            .field("token", &"<secret>")
            .field("info", &self.info)
            .finish()
    }
}

/// Who is making a request, once their session checked out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    pub user_id: UserId,
    pub display_name: String,
    pub role: Role,
    pub session_id: SessionId,
    pub kind: SessionKind,
}

/// Facts about the client making a request: for rate limiting and for the
/// session list. Both are as the server sees them (behind a trusted proxy, the
/// server should pass the forwarded address).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientInfo {
    pub ip: Option<IpAddr>,
    pub user_agent: Option<String>,
}

impl ClientInfo {
    pub(crate) fn rate_key(&self) -> String {
        self.ip
            .map_or_else(|| "unknown".to_owned(), |ip| ip.to_string())
    }
}

/// A passkey, as listed on "my passkeys" (`GET /auth/passkeys`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct PasskeyInfo {
    pub id: PasskeyId,
    pub name: String,
    /// The domain it works on.
    pub rp_id: String,
    pub created_at: Timestamp,
    pub last_used_at: Option<Timestamp>,
    /// Synced passkeys (iCloud Keychain, Google Password Manager…) are
    /// backup-eligible; hardware keys aren't.
    pub backup_eligible: bool,
    pub backed_up: bool,
}

/// A device key, as listed on "my devices".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct DeviceKeyInfo {
    pub id: DeviceKeyId,
    pub user_id: UserId,
    pub name: String,
    pub created_at: Timestamp,
    pub last_used_at: Option<Timestamp>,
}

/// What an enrolment token is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EnrolmentPurpose {
    /// The first admin of a new server (printed at startup).
    FirstAdmin,
    /// A new person, with the token's role.
    NewUser,
    /// Another passkey or device for an existing person.
    AddDevice,
}

impl EnrolmentPurpose {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            EnrolmentPurpose::FirstAdmin => "first_admin",
            EnrolmentPurpose::NewUser => "new_user",
            EnrolmentPurpose::AddDevice => "add_device",
        }
    }
}

impl FromStr for EnrolmentPurpose {
    type Err = AuthError;
    fn from_str(s: &str) -> Result<EnrolmentPurpose> {
        match s {
            "first_admin" => Ok(EnrolmentPurpose::FirstAdmin),
            "new_user" => Ok(EnrolmentPurpose::NewUser),
            "add_device" => Ok(EnrolmentPurpose::AddDevice),
            _ => Err(AuthError::Corrupt(format!(
                "unknown enrolment purpose {s:?}"
            ))),
        }
    }
}

/// A request to mint an enrolment token (`POST /auth/enrollments`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, TS)]
#[ts(export)]
pub struct NewEnrolment {
    /// `new_user` or `add_device` (`first_admin` only comes from
    /// [`crate::AuthStore::bootstrap`]).
    pub purpose: EnrolmentPurpose,
    /// The new user's role (`new_user`; defaults to bookkeeper).
    #[serde(default)]
    #[ts(optional)]
    pub role: Option<Role>,
    /// Whose device to add (`add_device`; defaults to the caller).
    #[serde(default)]
    #[ts(optional)]
    pub user_id: Option<UserId>,
    /// Give the new user access to this company (`new_user`).
    #[serde(default)]
    #[ts(optional)]
    pub company_id: Option<CompanyId>,
    /// Suggested name for the new user; they can change it when enrolling.
    #[serde(default)]
    #[ts(optional)]
    pub display_name: Option<String>,
    /// Lifetime; defaults to the configured 15 minutes.
    #[serde(default)]
    #[ts(optional)]
    pub ttl_minutes: Option<u32>,
}

/// A freshly minted enrolment token. `token` is the secret: show it once (as
/// a link and QR code); it is never stored.
#[derive(Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct MintedEnrolment {
    pub id: EnrolmentId,
    pub token: String,
    pub purpose: EnrolmentPurpose,
    pub expires_at: Timestamp,
}

impl fmt::Debug for MintedEnrolment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MintedEnrolment")
            .field("id", &self.id)
            .field("token", &"<secret>")
            .field("purpose", &self.purpose)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// An enrolment token as admins see it in the list of outstanding ones (no
/// secret).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct EnrolmentInfo {
    pub id: EnrolmentId,
    pub purpose: EnrolmentPurpose,
    pub role: Option<Role>,
    pub user_id: Option<UserId>,
    pub company_id: Option<CompanyId>,
    pub display_name: Option<String>,
    pub created_by: Option<UserId>,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

/// A user's role in one company.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CompanyAccess {
    pub company_id: CompanyId,
    pub role: Role,
}

/// Body of `POST /auth/enroll/start`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, TS)]
#[ts(export)]
pub struct EnrolStartRequest {
    pub token: String,
    /// The person's name, if the page asked for it before the passkey prompt
    /// (new users). It labels the passkey in their password manager, which
    /// can't be renamed later, so it's worth asking first.
    #[serde(default)]
    #[ts(optional)]
    pub display_name: Option<String>,
}

/// Response to `POST /auth/enroll/start`: pass `options` to
/// `startRegistration({ optionsJSON: options })`.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct EnrolStart {
    pub challenge_id: ChallengeId,
    #[ts(type = "import(\"@simplewebauthn/browser\").PublicKeyCredentialCreationOptionsJSON")]
    pub options: serde_json::Value,
    pub purpose: EnrolmentPurpose,
    /// The person's name as far as it's known: theirs (`add_device`), what
    /// they typed at the start, or what the admin suggested.
    pub display_name: Option<String>,
}

/// Body of `POST /auth/enroll/finish`.
#[derive(Clone, Debug, PartialEq, Deserialize, TS)]
#[ts(export)]
pub struct EnrolFinish {
    pub token: String,
    pub challenge_id: ChallengeId,
    /// What `startRegistration` resolved to.
    #[ts(type = "import(\"@simplewebauthn/browser\").RegistrationResponseJSON")]
    pub credential: serde_json::Value,
    /// The person's name (new users; ignored when adding a device). Defaults
    /// to the name given at the start, then the admin's suggestion.
    #[serde(default)]
    #[ts(optional)]
    pub display_name: Option<String>,
    /// A name for this passkey ("Work laptop"); defaults to one made up from
    /// the browser.
    #[serde(default)]
    #[ts(optional)]
    pub passkey_name: Option<String>,
}

/// Response to `POST /auth/login/start`: pass `options` to
/// `startAuthentication({ optionsJSON: options })` (with
/// `useBrowserAutofill: true` for conditional UI).
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct LoginStart {
    pub challenge_id: ChallengeId,
    #[ts(type = "import(\"@simplewebauthn/browser\").PublicKeyCredentialRequestOptionsJSON")]
    pub options: serde_json::Value,
}

/// Body of `POST /auth/login/finish`.
#[derive(Clone, Debug, PartialEq, Deserialize, TS)]
#[ts(export)]
pub struct LoginFinish {
    pub challenge_id: ChallengeId,
    /// What `startAuthentication` resolved to.
    #[ts(type = "import(\"@simplewebauthn/browser\").AuthenticationResponseJSON")]
    pub credential: serde_json::Value,
}

/// Body of `POST /auth/device/enroll`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, TS)]
#[ts(export)]
pub struct DeviceEnrol {
    pub token: String,
    /// The Ed25519 public key, 32 bytes, base64url.
    pub public_key: String,
    /// What to call this device ("Cameron's ThinkPad").
    pub name: String,
    /// The person's name, when the token makes a new user.
    #[serde(default)]
    #[ts(optional)]
    pub display_name: Option<String>,
}

/// Response to `POST /auth/device/challenge`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct DeviceChallenge {
    pub challenge_id: ChallengeId,
    /// 32 random bytes, base64url. Sign
    /// [`crate::device_login_message`]`(challenge, origin)`.
    pub challenge: String,
}

/// Body of `POST /auth/device/login`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, TS)]
#[ts(export)]
pub struct DeviceLogin {
    pub challenge_id: ChallengeId,
    pub key_id: DeviceKeyId,
    /// The Ed25519 signature, 64 bytes, base64url.
    pub signature: String,
}

/// Trim a name a person typed and check it's reasonable.
pub(crate) fn clean_name(what: &str, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AuthError::Invalid(format!("{what} can't be empty")));
    }
    if name.chars().count() > 100 {
        return Err(AuthError::Invalid(format!(
            "{what} is too long (100 characters at most)"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(AuthError::Invalid(format!(
            "{what} can't contain control characters"
        )));
    }
    Ok(name.to_owned())
}
