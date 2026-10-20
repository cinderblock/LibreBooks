//! Backup settings, as saved in the server's configuration and edited in the UI.
//!
//! Everything has a sensible default so that `{"destinations":[{"kind":"local",
//! "dir":"/var/backups/librebooks"}]}` is a complete configuration: daily, zstd,
//! unencrypted, the default retention.

use std::path::PathBuf;

use jiff::SignedDuration;
use jiff::civil::Time;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};

use crate::secret::SecretRef;
use crate::{BackupError, Result};

/// How, where and how often to back up one company.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupConfig {
    pub schedule: Schedule,
    pub destinations: Vec<Destination>,
    pub encryption: Encryption,
    pub retention: Retention,
    /// zstd level, 1–22. 9 compresses a company file to roughly a fifth in well under
    /// a second per 100 MB.
    pub compression_level: i32,
    /// IANA time zone for the daily time and for retention's day/week/month/year
    /// boundaries. `None`: the system's.
    pub time_zone: Option<String>,
    /// Where the working copy is made. `None`: next to the company file (same disk,
    /// and `/tmp` is often a small RAM disk).
    pub work_dir: Option<PathBuf>,
    /// Which backups in a destination retention may prune.
    pub retention_scope: RetentionScope,
}

/// Which backups retention considers its own. Backups are filed under the
/// company's name slug, so a renamed company starts a new folder; whether the
/// old folder is still "this company" depends on who else shares the
/// destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RetentionScope {
    /// Only backups under the current name's slug: safe when companies share a
    /// destination, but a renamed company's older backups are never pruned.
    #[default]
    CompanySlug,
    /// Everything in the destination: for a destination that holds one company
    /// (the server gives each company its own folder or key prefix), so backups
    /// made under earlier names age out too.
    Destination,
}

impl Default for BackupConfig {
    fn default() -> Self {
        BackupConfig {
            schedule: Schedule::default(),
            destinations: Vec::new(),
            encryption: Encryption::None,
            retention: Retention::default(),
            compression_level: 9,
            time_zone: None,
            work_dir: None,
            retention_scope: RetentionScope::CompanySlug,
        }
    }
}

impl BackupConfig {
    /// The configured time zone, or the system's.
    ///
    /// # Errors
    /// [`BackupError::Config`] for a name the time zone database does not know.
    pub fn tz(&self) -> Result<TimeZone> {
        match &self.time_zone {
            None => Ok(TimeZone::system()),
            Some(name) => TimeZone::get(name)
                .map_err(|_| BackupError::Config(format!("unknown time zone {name:?}"))),
        }
    }

    /// Problems a person must fix before this configuration can run.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.destinations.is_empty() {
            out.push("add at least one destination".to_owned());
        }
        if !(1..=22).contains(&self.compression_level) {
            out.push("compression level must be between 1 and 22".to_owned());
        }
        if let Err(e) = self.tz() {
            out.push(e.to_string());
        }
        match &self.encryption {
            Encryption::Recipients { recipients } => {
                if recipients.is_empty() {
                    out.push("encryption to keys needs at least one public key".to_owned());
                }
                for r in recipients {
                    if r.parse::<age::x25519::Recipient>().is_err() {
                        out.push(format!("{r:?} is not an age public key (age1…)"));
                    }
                }
            }
            Encryption::None | Encryption::Passphrase { .. } => {}
        }
        if let Some(every) = self.schedule.every
            && every < SignedDuration::from_mins(5)
        {
            out.push("back up at most every 5 minutes".to_owned());
        }
        for d in &self.destinations {
            if let Destination::S3 { bucket, .. } = d
                && bucket.is_empty()
            {
                out.push("an S3 destination needs a bucket name".to_owned());
            }
        }
        out
    }
}

/// When backups run. Both rules may be set; whichever comes first wins.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    pub enabled: bool,
    /// At most this long after the previous backup (ISO 8601 in JSON: `"PT6H"`).
    pub every: Option<SignedDuration>,
    /// Every day at this local time (`"02:30:00"`). A time missed while the
    /// machine was off runs as soon as it is back.
    pub daily_at: Option<Time>,
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule {
            enabled: true,
            every: Some(SignedDuration::from_hours(24)),
            daily_at: None,
        }
    }
}

/// Where backups go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Destination {
    /// A folder: a second disk, a NAS mount, a synced folder.
    Local { dir: PathBuf },
    /// Any S3-compatible bucket.
    S3 {
        /// `https://s3.us-west-002.backblazeb2.com`, `https://<account>.r2.cloudflarestorage.com`,
        /// `http://minio.lan:9000`. Empty: AWS (`https://s3.<region>.amazonaws.com`).
        #[serde(default)]
        endpoint: String,
        /// `us-east-1`; `auto` for R2.
        region: String,
        bucket: String,
        /// A folder inside the bucket, e.g. `librebooks/` (optional).
        #[serde(default)]
        prefix: String,
        access_key_id: String,
        secret_access_key_ref: SecretRef,
        /// `endpoint/bucket/key` instead of `bucket.endpoint/key`. MinIO and most
        /// self-hosted services need it; AWS, B2 and R2 work either way.
        #[serde(default)]
        path_style: bool,
    },
}

impl Destination {
    /// A short description for people: `local folder D:\Backups`, `s3://bucket/prefix`.
    pub fn describe(&self) -> String {
        match self {
            Destination::Local { dir } => format!("local folder {}", dir.display()),
            Destination::S3 { bucket, prefix, .. } => {
                format!("s3://{bucket}/{}", prefix.trim_start_matches('/'))
            }
        }
    }
}

/// How the stored backups are encrypted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Encryption {
    #[default]
    None,
    /// To one or more age X25519 public keys (`age1…`). Restoring needs a matching
    /// identity (`AGE-SECRET-KEY-1…`), which should not live on the server.
    Recipients { recipients: Vec<String> },
    /// To a passphrase (age's scrypt). The server can verify these on its own.
    Passphrase {
        passphrase_ref: SecretRef,
        /// scrypt cost as log2(N). Leave unset: age picks about a second of work.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        work_factor: Option<u8>,
    },
}

/// Grandfather-father-son retention. See [`crate::plan_retention`] for the exact
/// rules. All zeros keeps everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Retention {
    /// The most recent N backups, whenever they were made.
    pub keep_last: u32,
    /// The newest backup of each of the last N days that have backups.
    pub daily: u32,
    /// … of each of the last N ISO weeks that have backups.
    pub weekly: u32,
    /// … of each of the last N months that have backups.
    pub monthly: u32,
    /// … of each of the last N years that have backups.
    pub yearly: u32,
}

impl Retention {
    /// Keep everything.
    pub const ALL: Retention = Retention {
        keep_last: 0,
        daily: 0,
        weekly: 0,
        monthly: 0,
        yearly: 0,
    };
}

impl Default for Retention {
    /// Ten most recent, a week of dailies, a month of weeklies, a year of monthlies,
    /// and seven years (a common retention requirement for accounting records).
    fn default() -> Self {
        Retention {
            keep_last: 10,
            daily: 7,
            weekly: 4,
            monthly: 12,
            yearly: 7,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_config_gets_the_defaults() {
        let c: BackupConfig =
            serde_json::from_str(r#"{"destinations":[{"kind":"local","dir":"/b"}]}"#).unwrap();
        assert_eq!(c.schedule, Schedule::default());
        assert_eq!(c.retention, Retention::default());
        assert_eq!(c.encryption, Encryption::None);
        assert_eq!(c.compression_level, 9);
        assert!(c.problems().is_empty(), "{:?}", c.problems());
    }

    #[test]
    fn config_round_trips_through_json() {
        let c = BackupConfig {
            schedule: Schedule {
                enabled: true,
                every: Some(SignedDuration::from_hours(6)),
                daily_at: Some(jiff::civil::time(2, 30, 0, 0)),
            },
            destinations: vec![Destination::S3 {
                endpoint: "https://s3.us-west-002.backblazeb2.com".into(),
                region: "us-west-002".into(),
                bucket: "books".into(),
                prefix: "lb/".into(),
                access_key_id: "AKID".into(),
                secret_access_key_ref: SecretRef::new("env:S3_SECRET"),
                path_style: false,
            }],
            encryption: Encryption::Passphrase {
                passphrase_ref: SecretRef::new("file:/run/secrets/pp"),
                work_factor: None,
            },
            retention: Retention::ALL,
            compression_level: 3,
            time_zone: Some("America/Los_Angeles".into()),
            work_dir: None,
            retention_scope: RetentionScope::Destination,
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"every\":\"PT6H\""), "{json}");
        assert!(json.contains("\"kind\":\"s3\""), "{json}");
        let back: BackupConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn problems_are_reported_in_words() {
        let c = BackupConfig {
            compression_level: 30,
            time_zone: Some("Mars/Olympus_Mons".into()),
            encryption: Encryption::Recipients {
                recipients: vec!["not-a-key".into()],
            },
            schedule: Schedule {
                every: Some(SignedDuration::from_secs(10)),
                ..Schedule::default()
            },
            ..BackupConfig::default()
        };
        let p = c.problems().join("\n");
        for needle in [
            "destination",
            "compression",
            "time zone",
            "not an age public key",
            "5 minutes",
        ] {
            assert!(p.contains(needle), "{needle} missing from {p}");
        }
    }
}
