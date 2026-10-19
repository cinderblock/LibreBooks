//! Credentials for online sources, kept out of the company file and out of logs.
//!
//! A connection's settings (stored in the company file, backed up, copied
//! around) hold only a [`SecretRef`]: an opaque name. The server decides where
//! secrets really live (OS keychain on the desktop, an encrypted secrets file on
//! a headless server) and hands this crate a [`Secrets`] implementation to
//! resolve them. A resolved [`Secret`] can't be printed by accident: its `Debug`
//! is redacted and it has no `Display`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{FeedError, Result};

/// The name of a secret, as stored in connection settings. Not the secret.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(transparent)]
#[ts(export, type = "string")]
pub struct SecretRef(pub String);

impl SecretRef {
    pub fn new(name: impl Into<String>) -> SecretRef {
        SecretRef(name.into())
    }
}

/// A credential: a SimpleFIN access URL, a bank password, an API token.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    /// The secret itself, for the one place that sends it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

/// Where the server keeps secrets. Implemented by the server; this crate only
/// reads.
pub trait Secrets: Send + Sync {
    fn get(&self, r: &SecretRef) -> Option<Secret>;
}

/// Resolve a reference or say plainly that it's missing.
pub fn resolve(secrets: &dyn Secrets, r: &SecretRef, provider: &'static str) -> Result<Secret> {
    secrets.get(r).ok_or_else(|| FeedError::Unauthorized {
        provider,
        message: "the saved credentials are missing; reconnect this account".into(),
    })
}

/// Secrets in a map: for tests and for a CLI that reads them from the
/// environment.
#[derive(Default)]
pub struct MemorySecrets(pub std::collections::HashMap<String, Secret>);

impl std::fmt::Debug for MemorySecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemorySecrets")
            .field("names", &self.0.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl MemorySecrets {
    pub fn insert(&mut self, name: &str, value: Secret) -> SecretRef {
        self.0.insert(name.to_owned(), value);
        SecretRef::new(name)
    }
}

impl Secrets for MemorySecrets {
    fn get(&self, r: &SecretRef) -> Option<Secret> {
        self.0.get(&r.0).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_do_not_print() {
        let s = Secret::new("https://user:hunter2@bridge.example/simplefin");
        assert_eq!(format!("{s:?}"), "Secret([redacted])");
        let mut m = MemorySecrets::default();
        let r = m.insert("sfin", s);
        assert!(!format!("{m:?}").contains("hunter2"));
        assert!(
            resolve(&m, &r, "test")
                .unwrap()
                .expose()
                .contains("hunter2")
        );
        assert!(resolve(&m, &SecretRef::new("nope"), "test").is_err());
    }
}
