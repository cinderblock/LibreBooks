//! The manifest stored next to every backup, and how backups are named.
//!
//! A backup is two objects in a destination, in a folder per company:
//!
//! ```text
//! acme-widgets/acme-widgets-20261019T023000.123Z.lbdb.zst.age   the artifact
//! acme-widgets/acme-widgets-20261019T023000.123Z.manifest.json  this manifest
//! ```
//!
//! The manifest is written last and deleted first, so its presence means the
//! artifact is complete. Listing a destination reads manifests only.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::{BackupError, Result};

/// The manifest format this build writes and the newest it reads.
pub const FORMAT_VERSION: u32 = 1;

/// Suffix of a manifest object.
pub const MANIFEST_SUFFIX: &str = ".manifest.json";

/// Everything known about one backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// [`FORMAT_VERSION`] when written.
    pub format: u32,
    /// The LibreBooks version that made it.
    pub app_version: String,
    pub company_name: String,
    /// The folder and file-name stem derived from the company name ([`slug`]).
    pub company_slug: String,
    /// The company file's schema (`PRAGMA user_version`).
    pub schema_version: i64,
    pub created_at: Timestamp,
    /// The stem shared by the artifact and the manifest, e.g.
    /// `acme-20261019T023000.123Z`. Unique per company.
    pub id: String,
    /// Object name of the stored bytes, relative to the destination's root.
    pub artifact: String,
    /// Size and SHA-256 (hex) of the database file inside.
    pub database_size: u64,
    pub database_sha256: String,
    /// Size and SHA-256 (hex) of the stored, compressed (and encrypted) bytes.
    pub stored_size: u64,
    pub stored_sha256: String,
    /// Always `zstd` today.
    pub compression: String,
    pub encryption: EncryptionKind,
}

/// How a backup's bytes are encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncryptionKind {
    None,
    /// age, to X25519 public keys.
    X25519,
    /// age, to a passphrase (scrypt).
    Passphrase,
}

impl Manifest {
    /// Object name of the manifest itself.
    pub fn object_name(&self) -> String {
        manifest_name(&self.company_slug, &self.id)
    }

    /// Parse a manifest, refusing formats newer than this build understands.
    ///
    /// # Errors
    /// [`BackupError::Manifest`] for bad JSON, [`BackupError::TooNew`] for a newer
    /// format.
    pub fn from_json(bytes: &[u8]) -> Result<Manifest> {
        #[derive(Deserialize)]
        struct Probe {
            format: u32,
        }
        let probe: Probe =
            serde_json::from_slice(bytes).map_err(|e| BackupError::Manifest(e.to_string()))?;
        if probe.format > FORMAT_VERSION {
            return Err(BackupError::TooNew {
                found: probe.format,
                known: FORMAT_VERSION,
            });
        }
        serde_json::from_slice(bytes).map_err(|e| BackupError::Manifest(e.to_string()))
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).expect("a manifest always serializes")
    }
}

/// A file- and URL-safe name for a company: lowercase ASCII letters and digits,
/// runs of anything else as one hyphen. `"Acme Widgets, Inc."` → `acme-widgets-inc`.
/// A name with no letters or digits becomes `company`.
pub fn slug(company_name: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in company_name.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.push(c.to_ascii_lowercase());
        } else {
            gap = true;
        }
        if out.len() >= 60 {
            break;
        }
    }
    if out.is_empty() {
        "company".to_owned()
    } else {
        out
    }
}

/// `acme-20261019T023000.123Z`: sorts by time, safe on every file system.
pub fn backup_id(company_slug: &str, at: Timestamp) -> String {
    format!("{company_slug}-{}", at.strftime("%Y%m%dT%H%M%S%.3fZ"))
}

/// The manifest's object name.
pub fn manifest_name(company_slug: &str, id: &str) -> String {
    format!("{company_slug}/{id}{MANIFEST_SUFFIX}")
}

/// The artifact's object name.
pub fn artifact_name(company_slug: &str, id: &str, encrypted: bool) -> String {
    let age = if encrypted { ".age" } else { "" };
    format!("{company_slug}/{id}.lbdb.zst{age}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_safe_and_readable() {
        assert_eq!(slug("Acme Widgets, Inc."), "acme-widgets-inc");
        assert_eq!(slug("  TWILL Technology  "), "twill-technology");
        assert_eq!(slug("Café №5"), "caf-5");
        assert_eq!(slug("../../etc"), "etc");
        assert_eq!(slug("!!!"), "company");
        assert!(slug(&"x".repeat(500)).len() <= 60);
    }

    #[test]
    fn names_sort_by_time() {
        let a = backup_id("acme", "2026-10-19T02:30:00Z".parse().unwrap());
        let b = backup_id("acme", "2026-10-19T02:30:00.5Z".parse().unwrap());
        assert_eq!(a, "acme-20261019T023000.000Z");
        assert!(a < b);
        assert_eq!(
            artifact_name("acme", &a, true),
            "acme/acme-20261019T023000.000Z.lbdb.zst.age"
        );
        assert_eq!(
            manifest_name("acme", &a),
            "acme/acme-20261019T023000.000Z.manifest.json"
        );
    }

    #[test]
    fn newer_formats_are_refused_in_words() {
        let err = Manifest::from_json(br#"{"format": 99}"#).unwrap_err();
        assert!(matches!(err, BackupError::TooNew { found: 99, .. }));
        assert!(Manifest::from_json(b"{").is_err());
    }
}
