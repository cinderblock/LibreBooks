//! Checking Cloudflare Access's identity assertion.
//!
//! When the tunnel's hostname sits behind a Cloudflare Access application, every
//! request reaching LibreBooks carries `Cf-Access-Jwt-Assertion`: a JWT signed by
//! the team's Access keys, saying who signed in. Verifying it is a second lock
//! behind passkeys (master plan, decision 8): a request that did not come through
//! Access (someone who found the origin, or a misrouted tunnel) is refused.
//!
//! What is checked, all of it, every time:
//!
//! - the header names `alg: RS256` (exactly; `none` and HMAC algorithms are
//!   refused, so a token cannot choose a weaker check) and a `kid`;
//! - the RS256 signature, with the key of that `kid` from
//!   `https://<team>.cloudflareaccess.com/cdn-cgi/access/certs`;
//! - `iss` is that team's URL, `aud` includes one of the configured application
//!   audience tags, `exp` is in the future and `nbf` (if present) in the past, each
//!   with a minute of leeway for clock drift.
//!
//! Keys are cached and refetched every hour, or when a token names a `kid` not in
//! the cache (Access rotates keys every six weeks), but at most every 30 seconds,
//! so a stream of tokens with made-up `kid`s cannot turn into a stream of requests
//! to Cloudflare.
//!
//! The signature check is the `rsa` crate's PKCS#1 v1.5 verification (public-key
//! operations only; rsa's timing advisory concerns private-key operations).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rsa::pkcs1v15::Pkcs1v15Sign;
use rsa::sha2::{Digest as _, Sha256};
use rsa::{BigUint, RsaPublicKey};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// The request header Access adds.
pub const HEADER: &str = "Cf-Access-Jwt-Assertion";

/// Allowed clock drift for `exp` and `nbf`.
const LEEWAY: i64 = 60;
/// Keys are refreshed this often even when nothing asks.
const REFRESH_EVERY: Duration = Duration::from_secs(3600);
/// Never fetch keys more often than this.
const MIN_REFETCH: Duration = Duration::from_secs(30);

/// Which Access application to accept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessConfig {
    /// The team: `myteam`, or `myteam.cloudflareaccess.com`.
    pub team_domain: String,
    /// The application's audience tag(s) (Access → Applications → the app →
    /// Overview → "Application Audience (AUD) Tag").
    pub audience: Vec<String>,
}

/// Why a token was refused. The message is safe to log; it never includes the
/// token.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccessError {
    #[error("no Cloudflare Access assertion")]
    Missing,
    #[error("the Access assertion is malformed")]
    Malformed,
    #[error("the Access assertion uses {0:?}; only RS256 is accepted")]
    Algorithm(String),
    #[error("the Access assertion is signed with an unknown key")]
    UnknownKey,
    #[error("the Access assertion's signature is wrong")]
    Signature,
    #[error("the Access assertion is for another team ({0})")]
    Issuer(String),
    #[error("the Access assertion is for another application")]
    Audience,
    #[error("the Access assertion has expired")]
    Expired,
    #[error("the Access assertion is not valid yet")]
    NotYet,
    #[error("could not fetch the Access signing keys: {0}")]
    Keys(String),
}

/// Who Access says the request is from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessClaims {
    /// Access's user id (empty for service tokens).
    #[serde(default)]
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    /// `app` for a person, absent or other for service tokens.
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    /// For a service token, its client id.
    #[serde(default)]
    pub common_name: Option<String>,
    pub iss: String,
    #[serde(deserialize_with = "one_or_many")]
    pub aud: Vec<String>,
    pub exp: i64,
    #[serde(default)]
    pub nbf: Option<i64>,
    #[serde(default)]
    pub iat: Option<i64>,
    #[serde(default)]
    pub country: Option<String>,
}

fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

#[derive(Deserialize)]
struct Header {
    alg: String,
    kid: Option<String>,
}

#[derive(Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
struct Jwk {
    kid: Option<String>,
    kty: String,
    #[serde(default)]
    n: String,
    #[serde(default)]
    e: String,
}

struct Cache {
    keys: HashMap<String, RsaPublicKey>,
    fetched: Option<Instant>,
}

/// Verifies `Cf-Access-Jwt-Assertion` for one Access application.
pub struct AccessVerifier {
    issuer: String,
    certs_url: String,
    audience: Vec<String>,
    http: reqwest::Client,
    min_refetch: Duration,
    cache: Mutex<Cache>,
}

impl std::fmt::Debug for AccessVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessVerifier")
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .finish_non_exhaustive()
    }
}

impl AccessVerifier {
    /// For the team and application in `config`.
    ///
    /// # Errors
    /// [`AccessError::Keys`] if the HTTP client cannot be built.
    pub fn new(config: &AccessConfig) -> Result<AccessVerifier, AccessError> {
        let team = config.team_domain.trim().trim_start_matches("https://");
        let team = team.trim_end_matches('/');
        let host = if team.contains('.') {
            team.to_owned()
        } else {
            format!("{team}.cloudflareaccess.com")
        };
        let issuer = format!("https://{host}");
        let certs_url = format!("{issuer}/cdn-cgi/access/certs");
        AccessVerifier::with_urls(issuer, certs_url, config.audience.clone())
    }

    /// With an explicit issuer and key URL (tests).
    ///
    /// # Errors
    /// [`AccessError::Keys`] if the HTTP client cannot be built.
    pub fn with_urls(
        issuer: impl Into<String>,
        certs_url: impl Into<String>,
        audience: Vec<String>,
    ) -> Result<AccessVerifier, AccessError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("LibreBooks/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| AccessError::Keys(e.to_string()))?;
        Ok(AccessVerifier {
            issuer: issuer.into(),
            certs_url: certs_url.into(),
            audience,
            http,
            min_refetch: MIN_REFETCH,
            cache: Mutex::new(Cache {
                keys: HashMap::new(),
                fetched: None,
            }),
        })
    }

    /// Change the shortest time between key fetches (30 s), for tests.
    #[must_use]
    pub fn with_min_refetch(mut self, every: Duration) -> Self {
        self.min_refetch = every;
        self
    }

    /// Verify `token` now.
    ///
    /// # Errors
    /// [`AccessError`] saying why it was refused.
    pub async fn verify(&self, token: &str) -> Result<AccessClaims, AccessError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        self.verify_at(token, now).await
    }

    /// Verify `token` as if the time were `now` (Unix seconds).
    ///
    /// # Errors
    /// [`AccessError`] saying why it was refused.
    pub async fn verify_at(&self, token: &str, now: i64) -> Result<AccessClaims, AccessError> {
        let token = token.trim();
        if token.is_empty() {
            return Err(AccessError::Missing);
        }
        let mut parts = token.split('.');
        let (Some(h), Some(p), Some(s), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(AccessError::Malformed);
        };
        let header: Header = decode_json(h)?;
        if header.alg != "RS256" {
            return Err(AccessError::Algorithm(header.alg));
        }
        let kid = header.kid.ok_or(AccessError::Malformed)?;
        let signature = URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|_| AccessError::Malformed)?;
        let key = self.key(&kid).await?;
        let digest = Sha256::digest(&token.as_bytes()[..h.len() + 1 + p.len()]);
        key.verify(Pkcs1v15Sign::new::<Sha256>(), &digest, &signature)
            .map_err(|_| AccessError::Signature)?;

        // Signed by Access; now see what it says.
        let claims: AccessClaims = decode_json(p)?;
        if claims.iss.trim_end_matches('/') != self.issuer {
            return Err(AccessError::Issuer(claims.iss));
        }
        if !claims.aud.iter().any(|a| self.audience.contains(a)) {
            return Err(AccessError::Audience);
        }
        if claims.exp + LEEWAY < now {
            return Err(AccessError::Expired);
        }
        if claims.nbf.is_some_and(|nbf| nbf - LEEWAY > now) {
            return Err(AccessError::NotYet);
        }
        Ok(claims)
    }

    /// The key for `kid`, refetching the key set if it is unknown or stale.
    async fn key(&self, kid: &str) -> Result<RsaPublicKey, AccessError> {
        let mut cache = self.cache.lock().await;
        let stale = cache.fetched.is_none_or(|t| t.elapsed() >= REFRESH_EVERY);
        let unknown = !cache.keys.contains_key(kid);
        let may_fetch = cache
            .fetched
            .is_none_or(|t| t.elapsed() >= self.min_refetch);
        if (stale || unknown) && may_fetch {
            match self.fetch_keys().await {
                Ok(keys) => {
                    cache.keys = keys;
                    cache.fetched = Some(Instant::now());
                }
                // Keep using the keys we have if a refresh fails.
                Err(e) if !cache.keys.is_empty() => {
                    tracing::warn!(error = %e, "could not refresh Access keys");
                    cache.fetched = Some(Instant::now());
                }
                Err(e) => return Err(e),
            }
        }
        cache.keys.get(kid).cloned().ok_or(AccessError::UnknownKey)
    }

    async fn fetch_keys(&self) -> Result<HashMap<String, RsaPublicKey>, AccessError> {
        let fail = |e: String| AccessError::Keys(e);
        let resp = self
            .http
            .get(&self.certs_url)
            .send()
            .await
            .map_err(|e| fail(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(fail(format!(
                "{} answered HTTP {}",
                self.certs_url,
                resp.status().as_u16()
            )));
        }
        let body = resp.bytes().await.map_err(|e| fail(e.to_string()))?;
        let set: Jwks = serde_json::from_slice(&body).map_err(|e| fail(e.to_string()))?;
        let mut keys = HashMap::new();
        for k in set.keys {
            let (Some(kid), "RSA") = (k.kid, k.kty.as_str()) else {
                continue;
            };
            let (Ok(n), Ok(e)) = (URL_SAFE_NO_PAD.decode(&k.n), URL_SAFE_NO_PAD.decode(&k.e))
            else {
                continue;
            };
            if let Ok(key) =
                RsaPublicKey::new(BigUint::from_bytes_be(&n), BigUint::from_bytes_be(&e))
            {
                keys.insert(kid, key);
            }
        }
        Ok(keys)
    }
}

fn decode_json<T: serde::de::DeserializeOwned>(part: &str) -> Result<T, AccessError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(part)
        .map_err(|_| AccessError::Malformed)?;
    serde_json::from_slice(&bytes).map_err(|_| AccessError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_names_become_issuers_and_key_urls() {
        for team in [
            "myteam",
            "myteam.cloudflareaccess.com",
            "https://myteam.cloudflareaccess.com/",
        ] {
            let v = AccessVerifier::new(&AccessConfig {
                team_domain: team.into(),
                audience: vec!["aud".into()],
            })
            .unwrap();
            assert_eq!(v.issuer, "https://myteam.cloudflareaccess.com", "{team}");
            assert_eq!(
                v.certs_url,
                "https://myteam.cloudflareaccess.com/cdn-cgi/access/certs"
            );
        }
    }

    #[test]
    fn audiences_may_be_a_string_or_a_list() {
        let one: AccessClaims = serde_json::from_str(r#"{"iss":"i","aud":"a","exp":1}"#).unwrap();
        assert_eq!(one.aud, ["a"]);
        let many: AccessClaims =
            serde_json::from_str(r#"{"iss":"i","aud":["a","b"],"exp":1}"#).unwrap();
        assert_eq!(many.aud, ["a", "b"]);
    }
}
