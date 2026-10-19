//! Secrets by reference.
//!
//! Backup settings are saved, shown in the UI and sent over the API, so they must
//! never hold a password or an access key. They hold a [`SecretRef`] instead, which
//! names where the secret lives, and whoever runs the backup supplies a
//! [`SecretResolver`] to look it up at the moment it is needed:
//!
//! - `env:NAME` — an environment variable (containers, systemd `Environment=`).
//! - `file:/path/to/file` — a file's contents, trailing newline removed (Docker and
//!   systemd credentials, Kubernetes secrets).
//! - anything else — for the caller's resolver, e.g. the server's encrypted
//!   settings store (`secret:<id>`).
//!
//! [`EnvAndFiles`] handles the first two and refuses the rest.

use std::fmt;

use age::secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::{BackupError, Result};

/// Where a secret lives, e.g. `env:LB_S3_SECRET` or `file:/run/secrets/s3`.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretRef(pub String);

impl SecretRef {
    pub fn new(reference: impl Into<String>) -> Self {
        SecretRef(reference.into())
    }

    /// The scheme and the rest: `("env", "NAME")`. `None` without a colon.
    pub fn parts(&self) -> Option<(&str, &str)> {
        self.0.split_once(':')
    }
}

impl fmt::Debug for SecretRef {
    // A reference is not secret, but printing it is all Debug should ever do.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretRef({:?})", self.0)
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Looks secrets up when they are needed.
pub trait SecretResolver: Send + Sync {
    /// The secret `reference` names.
    ///
    /// # Errors
    /// [`BackupError::Secret`] when it does not exist or cannot be read.
    fn resolve(&self, reference: &SecretRef) -> Result<SecretString>;
}

/// Resolves `env:` and `file:` references.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvAndFiles;

impl SecretResolver for EnvAndFiles {
    fn resolve(&self, reference: &SecretRef) -> Result<SecretString> {
        let fail = |reason: String| BackupError::Secret {
            reference: reference.to_string(),
            reason,
        };
        match reference.parts() {
            Some(("env", name)) => std::env::var(name)
                .map(SecretString::from)
                .map_err(|_| fail(format!("the environment variable {name} is not set"))),
            Some(("file", path)) => std::fs::read_to_string(path)
                .map(|s| SecretString::from(s.trim_end_matches(['\r', '\n']).to_owned()))
                .map_err(|e| fail(e.to_string())),
            _ => Err(fail(
                "only env:NAME and file:/path references can be used here".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use age::secrecy::ExposeSecret;

    #[test]
    fn env_and_file_references_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s");
        std::fs::write(&path, "hunter2\r\n").unwrap();
        let r = SecretRef::new(format!("file:{}", path.display()));
        assert_eq!(EnvAndFiles.resolve(&r).unwrap().expose_secret(), "hunter2");

        // PATH exists everywhere tests run.
        assert!(EnvAndFiles.resolve(&SecretRef::new("env:PATH")).is_ok());
        let missing = EnvAndFiles
            .resolve(&SecretRef::new("env:LB_BACKUP_SURELY_UNSET_VARIABLE"))
            .unwrap_err();
        assert!(missing.to_string().contains("not set"), "{missing}");
        assert!(EnvAndFiles.resolve(&SecretRef::new("vault:x")).is_err());
        assert!(EnvAndFiles.resolve(&SecretRef::new("plain")).is_err());
    }

    #[test]
    fn references_serialize_as_plain_strings() {
        let r = SecretRef::new("env:X");
        assert_eq!(serde_json::to_string(&r).unwrap(), "\"env:X\"");
        assert_eq!(format!("{r:?}"), "SecretRef(\"env:X\")");
    }
}
