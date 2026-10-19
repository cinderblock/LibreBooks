//! Configuration: relying parties, lifetimes, limits.
//!
//! **Relying parties and origins.** A passkey is bound to an RP ID, which is a
//! domain (`books.example.com`), and the browser only offers it on pages whose
//! origin is that domain or a subdomain of it. A LibreBooks server is often
//! reachable several ways (a public hostname through a tunnel,
//! `http://localhost:8740` on the machine itself, a LAN hostname), so the
//! configuration lists several [`RelyingParty`]s, each with the exact origins
//! that may use it, and every WebAuthn call picks one by the request's `Origin`.
//! Consequences people will meet:
//!
//! - A passkey made at `https://books.example.com` does not work at
//!   `http://localhost:8740` (different RP ID). Enrol once per hostname you use.
//! - Browsers refuse IP addresses as RP IDs and only allow WebAuthn in a secure
//!   context (HTTPS, or plain HTTP on `localhost`). A bare LAN IP
//!   (`http://192.168.1.10:8740`) cannot use passkeys at all: give the server a
//!   hostname with HTTPS, or sign in from that address with a device key
//!   ([`AuthConfig::extra_device_origins`]).

use std::net::IpAddr;
use std::time::Duration;

use webauthn_rp::request::{AsciiDomain, RpId};

use crate::clock::Clock;
use crate::error::{AuthError, Result};

/// One WebAuthn relying party: the domain passkeys are bound to, the name
/// authenticators show, and the page origins allowed to use it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelyingParty {
    /// The RP ID: a domain such as `books.example.com` or `localhost`. Never an
    /// IP address, never a scheme or port.
    pub id: String,
    /// What the authenticator shows the person ("LibreBooks").
    pub name: String,
    /// Exact origins (`scheme://host[:port]`, no path) that may use this RP.
    /// Each host must be the RP ID or a subdomain of it.
    pub origins: Vec<String>,
}

impl RelyingParty {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        origins: impl IntoIterator<Item = impl Into<String>>,
    ) -> RelyingParty {
        RelyingParty {
            id: id.into(),
            name: name.into(),
            origins: origins.into_iter().map(Into::into).collect(),
        }
    }

    /// The usual case: one origin, whose host is the RP ID.
    /// `RelyingParty::for_origin("https://books.example.com", "LibreBooks")`.
    pub fn for_origin(origin: &str, name: impl Into<String>) -> Result<RelyingParty> {
        let url = url::Url::parse(origin)
            .map_err(|e| AuthError::Config(format!("{origin:?} is not a URL: {e}")))?;
        let host = match url.host() {
            Some(url::Host::Domain(d)) => d.to_owned(),
            Some(_) => {
                return Err(AuthError::Config(format!(
                    "{origin}: passkeys need a hostname, not an IP address"
                )));
            }
            None => return Err(AuthError::Config(format!("{origin} has no host"))),
        };
        Ok(RelyingParty::new(
            host,
            name,
            [url.origin().ascii_serialization()],
        ))
    }
}

/// How many failed sign-in or enrolment attempts one client (IP address) may
/// make in a window before it has to wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimit {
    pub max_failures: u32,
    pub window: Duration,
}

impl Default for RateLimit {
    fn default() -> Self {
        RateLimit {
            max_failures: 20,
            window: Duration::from_secs(10 * 60),
        }
    }
}

/// Everything [`crate::AuthStore`] needs to know. `Default` is a development
/// setup reachable at `http://localhost:8740`; the server replaces
/// `relying_parties` from its own configuration.
#[derive(Clone, Debug)]
pub struct AuthConfig {
    /// Passkey relying parties; the request's origin picks one.
    pub relying_parties: Vec<RelyingParty>,
    /// Origins that may sign in with device keys but not passkeys (a LAN IP,
    /// say). Every relying party's origins are allowed for device keys too.
    pub extra_device_origins: Vec<String>,
    /// Default lifetime of an enrolment token (15 minutes).
    pub enrolment_ttl: Duration,
    /// The longest lifetime an admin may give an enrolment token (7 days).
    pub max_enrolment_ttl: Duration,
    /// Lifetime of the first-admin token printed at startup (1 hour: long enough
    /// to read a log and open a browser).
    pub first_admin_ttl: Duration,
    /// How long a WebAuthn ceremony may take, start to finish (5 minutes; this
    /// is also the `timeout` the browser is given).
    pub ceremony_ttl: Duration,
    /// How long a device-login challenge stays valid (2 minutes).
    pub device_challenge_ttl: Duration,
    /// Passkey and device sessions end after this long unused (14 days)…
    pub session_idle: Duration,
    /// …and this long after sign-in regardless (90 days).
    pub session_absolute: Duration,
    /// Local (desktop, in-process) sessions: idle timeout (1 day)…
    pub local_session_idle: Duration,
    /// …and absolute lifetime (30 days).
    pub local_session_absolute: Duration,
    /// A session's `last_seen_at` is written at most this often (1 minute), so
    /// a busy page doesn't write to the database on every request.
    pub touch_interval: Duration,
    /// Failed-attempt limits per client.
    pub rate_limit: RateLimit,
    /// Most ceremonies (WebAuthn and device challenges) in flight at once; more
    /// are refused as "busy", so unauthenticated callers can't exhaust memory.
    pub max_pending_ceremonies: usize,
    /// Where "now" comes from.
    pub clock: Clock,
}

impl Default for AuthConfig {
    fn default() -> Self {
        AuthConfig {
            relying_parties: vec![RelyingParty::new(
                "localhost",
                "LibreBooks",
                ["http://localhost:8740"],
            )],
            extra_device_origins: Vec::new(),
            enrolment_ttl: Duration::from_secs(15 * 60),
            max_enrolment_ttl: Duration::from_secs(7 * 24 * 60 * 60),
            first_admin_ttl: Duration::from_secs(60 * 60),
            ceremony_ttl: Duration::from_secs(5 * 60),
            device_challenge_ttl: Duration::from_secs(2 * 60),
            session_idle: Duration::from_secs(14 * 24 * 60 * 60),
            session_absolute: Duration::from_secs(90 * 24 * 60 * 60),
            local_session_idle: Duration::from_secs(24 * 60 * 60),
            local_session_absolute: Duration::from_secs(30 * 24 * 60 * 60),
            touch_interval: Duration::from_secs(60),
            rate_limit: RateLimit::default(),
            max_pending_ceremonies: 10_000,
            clock: Clock::system(),
        }
    }
}

/// A validated relying party, ready for webauthn_rp.
#[derive(Debug)]
pub(crate) struct Rp {
    pub webauthn_id: RpId,
    pub id: String,
    pub name: String,
    pub origins: Vec<String>,
}

impl AuthConfig {
    /// Check the configuration and prepare the relying parties. Mistakes here
    /// would otherwise surface as passkeys that silently never work.
    pub(crate) fn validate(&self) -> Result<Vec<Rp>> {
        let mut rps = Vec::with_capacity(self.relying_parties.len());
        let mut seen_origins: Vec<&str> = Vec::new();
        for rp in &self.relying_parties {
            if rp.id.parse::<IpAddr>().is_ok() || rp.id.starts_with('[') {
                return Err(AuthError::Config(format!(
                    "relying party {:?}: browsers don't allow an IP address as a passkey domain; \
                     use a hostname (or device keys for this address)",
                    rp.id
                )));
            }
            let rp_id = AsciiDomain::try_from(rp.id.to_ascii_lowercase())
                .map(RpId::Domain)
                .map_err(|e| {
                    AuthError::Config(format!(
                        "relying party {:?} is not a valid domain: {e}",
                        rp.id
                    ))
                })?;
            if rp.name.trim().is_empty() {
                return Err(AuthError::Config(format!(
                    "relying party {:?} needs a name",
                    rp.id
                )));
            }
            if rp.origins.is_empty() {
                return Err(AuthError::Config(format!(
                    "relying party {:?} needs at least one origin",
                    rp.id
                )));
            }
            for origin in &rp.origins {
                let url = check_origin(origin)?;
                let host = match url.host() {
                    Some(url::Host::Domain(d)) => d.to_owned(),
                    _ => {
                        return Err(AuthError::Config(format!(
                            "origin {origin}: passkeys need a hostname, not an IP address"
                        )));
                    }
                };
                let id = rp_id.as_ref();
                if host != id && !host.ends_with(&format!(".{id}")) {
                    return Err(AuthError::Config(format!(
                        "origin {origin} can't use passkeys for {id}: its host must be {id} \
                         or a subdomain of it"
                    )));
                }
                let local = host == "localhost" || host.ends_with(".localhost");
                if url.scheme() != "https" && !local {
                    return Err(AuthError::Config(format!(
                        "origin {origin}: browsers only allow passkeys over HTTPS \
                         (plain HTTP works only on localhost)"
                    )));
                }
                if seen_origins.contains(&origin.as_str()) {
                    return Err(AuthError::Config(format!(
                        "origin {origin} is listed under two relying parties"
                    )));
                }
                seen_origins.push(origin);
            }
            rps.push(Rp {
                id: rp_id.as_ref().to_owned(),
                webauthn_id: rp_id,
                name: rp.name.clone(),
                origins: rp.origins.clone(),
            });
        }
        for origin in &self.extra_device_origins {
            check_origin(origin)?;
        }
        if self.max_pending_ceremonies == 0 {
            return Err(AuthError::Config(
                "max_pending_ceremonies must be at least 1".to_owned(),
            ));
        }
        if self.ceremony_ttl.is_zero() || self.ceremony_ttl > Duration::from_secs(60 * 60) {
            return Err(AuthError::Config(
                "ceremony_ttl must be between a millisecond and an hour".to_owned(),
            ));
        }
        Ok(rps)
    }
}

/// An origin must be exactly `scheme://host[:port]` as browsers send it.
fn check_origin(origin: &str) -> Result<url::Url> {
    let url = url::Url::parse(origin)
        .map_err(|e| AuthError::Config(format!("origin {origin:?} is not a URL: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AuthError::Config(format!(
            "origin {origin} must be http or https"
        )));
    }
    let normal = url.origin().ascii_serialization();
    if normal != origin {
        return Err(AuthError::Config(format!(
            "origin {origin:?} must be written exactly as browsers send it: {normal:?} \
             (scheme, host and non-default port only; no path or trailing slash)"
        )));
    }
    Ok(url)
}
