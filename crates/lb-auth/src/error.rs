//! What can go wrong, in words a person can act on.
//!
//! Each error knows its API error code ([`AuthError::code`]) so the server can
//! map it to the `{ error: { code, message } }` envelope in `plans/api.md`
//! without a big match of its own. Messages never contain a secret.

use std::time::Duration;

/// Everything that can go wrong in auth.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("auth configuration: {0}")]
    Config(String),
    #[error(
        "this auth database is from a newer LibreBooks (schema {found}, this build knows {known})"
    )]
    TooNew { found: i64, known: i64 },
    #[error("bad data in the auth database: {0}")]
    Corrupt(String),

    #[error(
        "this enrolment link isn't valid; check it was copied whole, or ask an admin for a new one"
    )]
    TokenNotFound,
    #[error("this enrolment link has expired; ask an admin for a new one")]
    TokenExpired,
    #[error("this enrolment link has already been used; ask an admin for a new one")]
    TokenUsed,
    #[error("this enrolment link was cancelled; ask an admin for a new one")]
    TokenRevoked,
    #[error(
        "this server already has an admin, so the first-run link no longer works; ask an admin for a new link"
    )]
    AdminExists,

    #[error("this page's address ({origin}) isn't one this server accepts sign-ins from")]
    UnknownOrigin { origin: String },
    #[error("this sign-in attempt expired or was already used; please try again")]
    ChallengeNotFound,
    #[error("the passkey could not be verified: {0}")]
    Passkey(String),
    #[error("this passkey isn't registered here (it may have been removed); sign in another way")]
    UnknownCredential,
    #[error("this passkey or device is already registered")]
    AlreadyRegistered,
    #[error("this device key isn't registered here or was revoked")]
    UnknownDeviceKey,
    #[error("the device's signature didn't verify")]
    BadSignature,

    #[error("you're not signed in, or your session has ended; please sign in again")]
    Unauthenticated,
    #[error("this account is disabled; ask an admin")]
    UserDisabled,
    #[error("you don't have permission to {0}")]
    Forbidden(String),
    #[error("{kind} not found")]
    NotFound { kind: &'static str, id: String },
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("too many failed attempts; try again in {} seconds", retry_after.as_secs().max(1))]
    RateLimited { retry_after: Duration },
    #[error("the server is handling too many sign-ins right now; try again in a minute")]
    Busy,
}

impl AuthError {
    /// The API error code (`plans/api.md`, "Errors"). The HTTP status follows
    /// from it: `invalid` 400, `unauthenticated` 401, `forbidden` 403,
    /// `not_found` 404, `conflict` 409, `rate_limited` 429, `internal` 500.
    pub fn code(&self) -> &'static str {
        match self {
            AuthError::Sqlite(_)
            | AuthError::Json(_)
            | AuthError::Config(_)
            | AuthError::TooNew { .. }
            | AuthError::Corrupt(_) => "internal",
            AuthError::TokenNotFound
            | AuthError::TokenExpired
            | AuthError::TokenUsed
            | AuthError::TokenRevoked
            | AuthError::AdminExists
            | AuthError::UnknownOrigin { .. }
            | AuthError::ChallengeNotFound
            | AuthError::Passkey(_)
            | AuthError::Invalid(_) => "invalid",
            AuthError::UnknownCredential
            | AuthError::UnknownDeviceKey
            | AuthError::BadSignature
            | AuthError::Unauthenticated
            | AuthError::UserDisabled => "unauthenticated",
            AuthError::Forbidden(_) => "forbidden",
            AuthError::NotFound { .. } => "not_found",
            AuthError::AlreadyRegistered | AuthError::Conflict(_) => "conflict",
            AuthError::RateLimited { .. } | AuthError::Busy => "rate_limited",
        }
    }

    /// Whether this failure counts against the client's rate limit: wrong
    /// guesses and failed proofs do; our own problems and "slow down" don't.
    pub(crate) fn counts_as_failure(&self) -> bool {
        matches!(
            self,
            AuthError::TokenNotFound
                | AuthError::TokenExpired
                | AuthError::TokenUsed
                | AuthError::TokenRevoked
                | AuthError::AdminExists
                | AuthError::ChallengeNotFound
                | AuthError::Passkey(_)
                | AuthError::UnknownCredential
                | AuthError::UnknownDeviceKey
                | AuthError::BadSignature
                | AuthError::UserDisabled
                | AuthError::Unauthenticated
        )
    }
}

pub type Result<T, E = AuthError> = std::result::Result<T, E>;
