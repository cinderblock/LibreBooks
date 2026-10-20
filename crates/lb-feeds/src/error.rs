//! What can go wrong getting transactions in. Messages are for the person
//! importing: they say what happened and, where they can, what to do.

use lb_store::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum FeedError {
    /// The file or response could not be read at all.
    #[error("{format} could not be read: {message}")]
    Parse {
        format: &'static str,
        message: String,
    },
    /// The request itself makes no sense (a missing account, a bad rule).
    #[error("{0}")]
    Invalid(String),
    #[error("{kind} not found")]
    NotFound { kind: &'static str, id: String },
    /// The bank or provider refused, with its reason.
    #[error("{provider}: {message}")]
    Provider {
        provider: &'static str,
        message: String,
    },
    /// The credentials were refused or are missing. Never carries the credentials.
    #[error("{provider}: {message}")]
    Unauthorized {
        provider: &'static str,
        message: String,
    },
    /// The network failed (no connection, timeout, TLS). The text never includes a
    /// URL, because provider URLs can carry credentials.
    #[error("couldn't reach {provider}: {message}")]
    Network {
        provider: &'static str,
        message: String,
    },
    #[error("this source isn't supported yet: {0}")]
    Unsupported(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<rusqlite::Error> for FeedError {
    fn from(e: rusqlite::Error) -> Self {
        FeedError::Store(StoreError::Sqlite(e))
    }
}

impl From<serde_json::Error> for FeedError {
    fn from(e: serde_json::Error) -> Self {
        FeedError::Store(StoreError::Json(e))
    }
}

impl FeedError {
    pub(crate) fn parse(format: &'static str, message: impl Into<String>) -> FeedError {
        FeedError::Parse {
            format,
            message: message.into(),
        }
    }

    /// A network error from reqwest, with any URL (and so any credentials in it)
    /// removed.
    pub(crate) fn network(provider: &'static str, e: reqwest::Error) -> FeedError {
        let e = e.without_url();
        let message = if e.is_timeout() {
            "the request timed out".to_owned()
        } else if e.is_connect() {
            "couldn't connect".to_owned()
        } else {
            e.to_string()
        };
        FeedError::Network { provider, message }
    }
}

/// Feed functions return this; a `StoreError` converts into it.
pub type Result<T, E = FeedError> = std::result::Result<T, E>;

/// Turn a feed error into a store error, for functions that run inside
/// `Books::write` (whose closures return `lb_store::Result`).
impl From<FeedError> for StoreError {
    fn from(e: FeedError) -> Self {
        match e {
            FeedError::Store(s) => s,
            FeedError::NotFound { kind, id } => StoreError::NotFound { kind, id },
            other => StoreError::Invalid(other.to_string()),
        }
    }
}
