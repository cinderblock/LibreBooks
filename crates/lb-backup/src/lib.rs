//! Backups: verified, compressed, optionally encrypted snapshots of company files.
//!
//! A company's books are one SQLite file, so a backup is a copy of that file taken
//! while it stays open (SQLite's online backup API, through
//! [`lb_store::Books::backup_to`]). The rest is about trusting the copy and keeping
//! it safe:
//!
//! 1. **Verified before it is kept.** The copy is opened as a company file and
//!    checked ([`lb_store::Books::check_integrity`]: SQLite's own check plus "every
//!    transaction balances"). A copy that fails is not stored, and nothing is pruned
//!    that day, so older good backups survive a damaged live file.
//! 2. **Small and private.** zstd compression, then optional [age] encryption to
//!    public keys (the server never holds what it takes to read them) or to a
//!    passphrase.
//! 3. **Self-describing.** Each backup has a JSON [`Manifest`] next to it: who, when,
//!    which schema, sizes and SHA-256 hashes of both the stored bytes and the
//!    database inside. Listing a destination needs nothing else.
//! 4. **Anywhere.** A local folder or any S3-compatible bucket (AWS, Backblaze B2,
//!    Cloudflare R2, MinIO …), each written atomically: the artifact first, the
//!    manifest last, so a half-written backup is never listed.
//! 5. **Pruned sensibly.** Grandfather-father-son retention ([`plan_retention`]),
//!    a pure function so the rules are tested exhaustively.
//! 6. **Restored safely.** [`restore`] checks every hash, decrypts, decompresses,
//!    opens and integrity-checks the result, and writes it to a *new* path. It never
//!    overwrites anything; the caller closes the old file and swaps.
//!
//! [`Scheduler`] runs backups on a timetable for the server; [`export_lbk`] and
//! [`import_lbk`] make the portable `.lbk` company file.
//!
//! [age]: https://age-encryption.org

pub mod config;
pub mod dest;
pub mod lbk;
pub mod manifest;
mod pipeline;
pub mod retention;
mod run;
pub mod schedule;
pub mod scheduler;
pub mod secret;
pub mod sigv4;

pub use config::{BackupConfig, Destination, Encryption, Retention, RetentionScope, Schedule};
pub use lbk::{PortableManifest, export_lbk, import_lbk};
pub use manifest::{EncryptionKind, Manifest, slug};
pub use retention::{Dated, plan_retention};
pub use run::{
    BackupEntry, BackupRecord, DestinationOutcome, Unlock, VerifyReport, backup_now, list_backups,
    restore, verify,
};
pub use schedule::next_run;
pub use scheduler::{LastRun, Scheduler, SchedulerStatus};
pub use secret::{EnvAndFiles, SecretRef, SecretResolver};

/// Everything that can go wrong. Messages are written for the person who has to do
/// something about it.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("could not {what}: {source}")]
    Io {
        what: String,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Store(#[from] lb_store::StoreError),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("the copy failed its integrity check, so it was not kept: {}", .0.join("; "))]
    Integrity(Vec<String>),
    #[error("could not encrypt the backup: {0}")]
    Encrypt(String),
    #[error("could not decrypt the backup: {0}")]
    Decrypt(String),
    #[error(
        "{what} does not match its recorded checksum; it was changed or damaged after the backup was made"
    )]
    Checksum { what: &'static str },
    #[error("the backup's manifest is unreadable: {0}")]
    Manifest(String),
    #[error(
        "this backup was made by a newer LibreBooks (backup format {found}, this build reads up to {known})"
    )]
    TooNew { found: u32, known: u32 },
    #[error("{0}")]
    Config(String),
    #[error("could not read the secret {reference}: {reason}")]
    Secret { reference: String, reason: String },
    #[error("the storage service said {status}: {message}")]
    Remote { status: u16, message: String },
    #[error("could not reach the storage service: {0}")]
    Network(String),
    #[error("{0} not found")]
    NotFound(String),
    #[error("{0} already exists; restore and import never overwrite a file")]
    Exists(std::path::PathBuf),
    #[error("no backup destinations are configured")]
    NoDestinations,
    #[error("the backup could not be stored anywhere: {}", .0.join("; "))]
    AllDestinationsFailed(Vec<String>),
    #[error("the backup task stopped unexpectedly: {0}")]
    Task(String),
}

impl BackupError {
    pub(crate) fn io(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> BackupError {
        let what = what.into();
        move |source| BackupError::Io { what, source }
    }
}

impl From<reqwest::Error> for BackupError {
    fn from(e: reqwest::Error) -> Self {
        // reqwest's Display omits the cause chain, which is where the useful part
        // ("connection refused", "certificate expired") lives.
        let mut msg = e.to_string();
        let mut source = std::error::Error::source(&e);
        while let Some(s) = source {
            msg.push_str(": ");
            msg.push_str(&s.to_string());
            source = s.source();
        }
        BackupError::Network(msg)
    }
}

pub type Result<T, E = BackupError> = std::result::Result<T, E>;

/// The version of LibreBooks writing backups (all crates share the workspace
/// version).
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
