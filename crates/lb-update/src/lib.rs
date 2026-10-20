//! Self-update from signed GitHub releases, for the headless server.
//!
//! (The desktop app updates through Tauri's updater plugin instead; see
//! `plans/self-update.md` for how both share one signing key.)
//!
//! The design is open-task's `ot-update`, generalised from "run the Windows
//! installer" to "replace the binary":
//!
//! 1. **Check** ([`Updater::check`]): ask the GitHub releases API for the newest
//!    release on the channel ([`feed`]), then believe it only once its
//!    `SHA256SUMS` verifies with the minisign key compiled into this binary and the
//!    signature names that very tag ([`trust`]). Versions compare as semver, and a
//!    development build is never offered the release it was built after
//!    ([`version`]).
//! 2. **Download** ([`Updater::download`]): the archive for this build's target,
//!    with a size limit, checked against its signed line in the sums.
//! 3. **Install** ([`Updater::install`]): unpack the binary, keep the current one
//!    as `<name>.old`, replace it ([`install`]), and say how to restart (exit for a
//!    supervisor, or respawn).
//!
//! Checking is on by default and only reports. Installing without asking is
//! opt-in ([`UpdateConfig::auto_install`], decided by [`policy::next_step`]). A copy
//! that cannot sensibly replace itself (a container, a package manager's copy, a
//! development build) is told what to do instead and never touched.

pub mod feed;
pub mod install;
pub mod policy;
pub mod trust;
mod updater;
pub mod version;

pub use feed::{Channel, FeedSource};
pub use install::{InstallKind, RESTART_EXIT_CODE, Restart, Target};
pub use policy::{Step, UpdateConfig};
pub use trust::{TrustError, TrustedKey};
pub use updater::{
    Available, CheckOutcome, DOWNLOAD_LIMIT, Downloaded, Installed, Phase, Stage, UpdateStatus,
    Updater, UpdaterOptions,
};
pub use version::{Build, Version};

/// Why an update step failed. The messages are for people.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error(transparent)]
    Trust(#[from] TrustError),
    #[error("the release list is unreadable: {0}")]
    Feed(String),
    #[error("release {tag} has no {name}")]
    NoAsset { tag: String, name: String },
    #[error("{url} answered HTTP {status}")]
    Http { url: String, status: u16 },
    #[error("{url} is larger than the {limit} bytes allowed")]
    TooLarge { url: String, limit: u64 },
    #[error("could not reach the update server: {0}")]
    Network(String),
    #[error("could not {what}: {source}")]
    Io {
        what: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("the download does not match the release's signed checksum")]
    Checksum,
    #[error("the update archive is unusable: {0}")]
    Archive(String),
    #[error("{0}")]
    NotSelfUpdatable(String),
    #[error("there is no previous version to go back to")]
    NothingToRollBack,
}

impl UpdateError {
    pub(crate) fn io(what: &'static str) -> impl FnOnce(std::io::Error) -> UpdateError {
        move |source| UpdateError::Io { what, source }
    }
}

impl From<reqwest::Error> for UpdateError {
    fn from(e: reqwest::Error) -> Self {
        let mut msg = e.to_string();
        let mut source = std::error::Error::source(&e);
        while let Some(s) = source {
            msg.push_str(": ");
            msg.push_str(&s.to_string());
            source = s.source();
        }
        UpdateError::Network(msg)
    }
}
