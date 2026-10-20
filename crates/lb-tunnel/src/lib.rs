//! An opt-in Cloudflare tunnel: download, verify and supervise cloudflared.
//!
//! The user's requirement is that publishing LibreBooks to the internet through a
//! Cloudflare zero-trust tunnel takes nothing but the tunnel token (and the public
//! hostname). So this crate does the rest:
//!
//! 1. **Find or fetch cloudflared** ([`cloudflared::ensure`]): the configured path,
//!    the system's copy, or Cloudflare's official release downloaded into the data
//!    folder, checked against the SHA-256 sums Cloudflare publishes (the trust
//!    model, and its limits, are spelled out in [`cloudflared`]). The managed copy
//!    is updated at start when a newer release exists.
//! 2. **Run it** ([`Tunnel`]): `cloudflared tunnel --no-autoupdate run`, the token in
//!    the environment rather than on the command line, tied to this process so it
//!    cannot outlive it, restarted with backoff when it exits.
//! 3. **Say what it is doing** ([`TunnelStatus`]): its log is parsed into
//!    starting / connected (with how many connections) / reconnecting / failed (with
//!    a reason a person can act on, such as a revoked token).
//! 4. **Check Cloudflare Access** ([`access`]): when the hostname is behind an
//!    Access application, verify the `Cf-Access-Jwt-Assertion` on each request.

pub mod access;
pub mod backoff;
pub mod cloudflared;
pub mod config;
pub mod logparse;
mod proc;
pub mod supervisor;

pub use access::{AccessClaims, AccessConfig, AccessError, AccessVerifier};
pub use backoff::Backoff;
pub use cloudflared::{Downloader, Located, Origin, Ready, Sources};
pub use config::{EnvAndFiles, TokenInfo, TokenSource, TunnelConfig};
pub use logparse::{LogEvent, LogParser};
pub use supervisor::{SupervisorOptions, Tunnel, TunnelState, TunnelStatus};

/// Why the tunnel could not be set up. Messages are for people.
#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    #[error("{0}")]
    Token(String),
    #[error("{0}")]
    NotInstalled(String),
    #[error("Cloudflare publishes no cloudflared for this platform")]
    UnsupportedPlatform,
    #[error("{0}")]
    Spawn(String),
    #[error("the download of {0} does not match the checksum Cloudflare published")]
    Checksum(String),
    #[error(
        "Cloudflare's release notes list no checksum for {0}, so it cannot be verified and was not installed"
    )]
    Unverifiable(String),
    #[error("the cloudflared archive is unusable: {0}")]
    Archive(String),
    #[error("could not reach GitHub to fetch cloudflared: {0}")]
    Network(String),
    #[error("could not {what}: {source}")]
    Io {
        what: &'static str,
        #[source]
        source: std::io::Error,
    },
}

impl TunnelError {
    pub(crate) fn io(what: &'static str) -> impl FnOnce(std::io::Error) -> TunnelError {
        move |source| TunnelError::Io { what, source }
    }
}

impl From<reqwest::Error> for TunnelError {
    fn from(e: reqwest::Error) -> Self {
        let mut msg = e.to_string();
        let mut source = std::error::Error::source(&e);
        while let Some(s) = source {
            msg.push_str(": ");
            msg.push_str(&s.to_string());
            source = s.source();
        }
        TunnelError::Network(msg)
    }
}
