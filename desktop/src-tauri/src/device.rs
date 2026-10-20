//! Device keys: how a thin client signs in to a remote LibreBooks server.
//!
//! Webviews on Linux (WebKitGTK) have no WebAuthn, so the desktop app does not
//! use passkeys. Instead it holds an Ed25519 key whose 32-byte seed lives in the
//! OS keychain (Windows Credential Manager, macOS Keychain, the Secret Service on
//! Linux), registers the public half once with an enrolment token, and signs a
//! server challenge to get a session (`plans/api.md`, `/auth/device/*`).
//!
//! The session cookie stays inside this process (the thin-client proxy attaches
//! it); the webview never sees it.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ed25519_dalek::{Signer, SigningKey};
use serde::Deserialize;
use url::Url;

use crate::settings::DeviceEnrollment;

/// Keychain service name for every LibreBooks secret.
const KEYCHAIN_SERVICE: &str = "org.librebooks.desktop";

/// Domain separation for the login signature (agreed with lb-auth, see
/// `plans/auth.md` decision 9).
const LOGIN_CONTEXT: &[u8] = b"librebooks device login v1\0";

/// Session cookie name (`plans/api.md`).
pub const SESSION_COOKIE: &str = "lb_session";

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("the system keychain isn't available: {0}")]
    Keychain(String),
    #[error(
        "this computer's key for {0} is missing from the system keychain; connect it again with a new enrolment link"
    )]
    KeyMissing(String),
    #[error(
        "this computer isn't connected to that server yet; get an enrolment link from its administrator"
    )]
    NotEnrolled,
    #[error("couldn't reach {url}: {message}")]
    Unreachable { url: String, message: String },
    #[error("that server doesn't support device sign-in yet (it may need updating)")]
    NotSupported,
    #[error("the server refused: {0}")]
    Refused(String),
    #[error("the server's reply wasn't understood: {0}")]
    Protocol(String),
}

/// A device's signing key. `Debug` never prints the secret.
pub struct DeviceKey(SigningKey);

impl std::fmt::Debug for DeviceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DeviceKey")
            .field(&self.public_key())
            .finish()
    }
}

impl DeviceKey {
    pub fn generate() -> DeviceKey {
        let mut seed = [0u8; 32];
        rand::fill(&mut seed);
        DeviceKey(SigningKey::from_bytes(&seed))
    }

    /// The public key, base64url without padding.
    pub fn public_key(&self) -> String {
        B64.encode(self.0.verifying_key().as_bytes())
    }

    /// Sign a login challenge from the server at `origin`.
    pub fn sign_login(&self, challenge: &[u8], origin: &str) -> String {
        B64.encode(self.0.sign(&login_message(challenge, origin)).to_bytes())
    }
}

/// What the device signs to log in: context, the server's challenge, and the
/// origin we connected to (so a malicious server can't relay another server's
/// challenge and replay our signature there).
pub fn login_message(challenge: &[u8], origin: &str) -> Vec<u8> {
    let mut m = Vec::with_capacity(LOGIN_CONTEXT.len() + challenge.len() + origin.len());
    m.extend_from_slice(LOGIN_CONTEXT);
    m.extend_from_slice(challenge);
    m.extend_from_slice(origin.as_bytes());
    m
}

// ---------------------------------------------------------------------------
// Keychain

fn keychain_entry(origin: &str) -> Result<keyring::Entry, DeviceError> {
    keyring::Entry::new(KEYCHAIN_SERVICE, &format!("device-key {origin}"))
        .map_err(|e| DeviceError::Keychain(e.to_string()))
}

/// Whether the OS keychain can be used here (for the settings page).
pub fn keychain_status() -> Result<(), String> {
    keyring::Entry::store_status()
        .as_ref()
        .copied()
        .map_err(ToString::to_string)
}

/// Store a device key for `origin` in the keychain.
pub fn store_key(origin: &str, key: &DeviceKey) -> Result<(), DeviceError> {
    keychain_entry(origin)?
        .set_secret(key.0.as_bytes())
        .map_err(|e| DeviceError::Keychain(e.to_string()))
}

/// The device key for `origin`, if the keychain has one.
pub fn load_key(origin: &str) -> Result<Option<DeviceKey>, DeviceError> {
    match keychain_entry(origin)?.get_secret() {
        Ok(secret) => {
            let seed: [u8; 32] = secret.as_slice().try_into().map_err(|_| {
                DeviceError::Keychain(format!("the stored key for {origin} is damaged"))
            })?;
            Ok(Some(DeviceKey(SigningKey::from_bytes(&seed))))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(DeviceError::Keychain(e.to_string())),
    }
}

/// Remove the device key for `origin` from the keychain (no error if absent).
pub fn forget_key(origin: &str) -> Result<(), DeviceError> {
    match keychain_entry(origin)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(DeviceError::Keychain(e.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Enrolment links

/// Split what the user pasted into (server, token). Accepts a bare token
/// (`lbe_…`), or an enrolment link (`https://host/enroll/<token>` or
/// `https://host/…?token=<token>`), in which case the link also names the server.
pub fn parse_enrollment(input: &str) -> Result<(Option<Url>, String), DeviceError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(DeviceError::Refused(
            "paste the enrolment link or token".into(),
        ));
    }
    if !input.contains("://") {
        return Ok((None, input.to_owned()));
    }
    let url = Url::parse(input).map_err(|e| DeviceError::Refused(format!("not a link: {e}")))?;
    let token = url
        .query_pairs()
        .find(|(k, _)| k == "token")
        .map(|(_, v)| v.into_owned())
        .or_else(|| {
            url.path_segments()
                .and_then(|mut s| s.rfind(|seg| !seg.is_empty()))
                .map(ToOwned::to_owned)
        })
        .ok_or_else(|| DeviceError::Refused("that link has no enrolment token in it".into()))?;
    let server = Url::parse(&format!("{}/", url.origin().ascii_serialization()))
        .map_err(|e| DeviceError::Protocol(e.to_string()))?;
    Ok((Some(server), token))
}

// ---------------------------------------------------------------------------
// Talking to the server

fn api(base: &Url, path: &str) -> Result<Url, DeviceError> {
    base.join(&format!("api/v1/{path}"))
        .map_err(|e| DeviceError::Protocol(e.to_string()))
}

async fn post_json(
    client: &reqwest::Client,
    url: Url,
    body: &serde_json::Value,
) -> Result<reqwest::Response, DeviceError> {
    // The server binds a device login to the request's Origin, and the
    // signature covers the same origin: say it explicitly.
    let origin = url.origin().ascii_serialization();
    let resp = client
        .post(url.clone())
        .header("X-LibreBooks", "1")
        .header(reqwest::header::ORIGIN, origin)
        .json(body)
        .send()
        .await
        .map_err(|e| DeviceError::Unreachable {
            url: url.to_string(),
            message: e.to_string(),
        })?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
    {
        return Err(DeviceError::NotSupported);
    }
    if !status.is_success() {
        let message = resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| status.to_string());
        return Err(DeviceError::Refused(message));
    }
    Ok(resp)
}

/// Register a new device key with the server at `base` using an enrolment
/// token. The private key goes into the keychain before the server hears of
/// it, and is removed again if the server refuses.
pub async fn enroll(
    client: &reqwest::Client,
    base: &Url,
    token: &str,
    name: &str,
) -> Result<DeviceEnrollment, DeviceError> {
    // `{ user, key: DeviceKeyInfo { id, … } }` (lb-auth `enrol_device`).
    #[derive(Deserialize)]
    struct Key {
        id: String,
    }
    #[derive(Deserialize)]
    struct Enrolled {
        key: Key,
    }
    let origin = base.origin().ascii_serialization();
    let key = DeviceKey::generate();
    let public_key = key.public_key();
    {
        let origin = origin.clone();
        tokio::task::spawn_blocking(move || store_key(&origin, &key))
            .await
            .map_err(|e| DeviceError::Keychain(e.to_string()))??;
    }
    let body = serde_json::json!({ "token": token, "public_key": public_key, "name": name });
    let result = async {
        post_json(client, api(base, "auth/device/enroll")?, &body)
            .await?
            .json::<Enrolled>()
            .await
            .map_err(|e| DeviceError::Protocol(e.to_string()))
    }
    .await;
    match result {
        Ok(Enrolled { key }) => Ok(DeviceEnrollment {
            origin,
            key_id: key.id,
            name: name.to_owned(),
            public_key,
        }),
        Err(e) => {
            let _ = tokio::task::spawn_blocking(move || forget_key(&origin)).await;
            Err(e)
        }
    }
}

/// An enrolled device: what the server knows it by, and its key.
#[derive(Debug)]
pub struct Identity {
    pub enrollment: DeviceEnrollment,
    pub key: DeviceKey,
}

/// Load the key for an enrolment from the keychain.
pub async fn load_identity(enrollment: DeviceEnrollment) -> Result<Identity, DeviceError> {
    let origin = enrollment.origin.clone();
    let key = tokio::task::spawn_blocking(move || load_key(&origin))
        .await
        .map_err(|e| DeviceError::Keychain(e.to_string()))??
        .ok_or_else(|| DeviceError::KeyMissing(enrollment.origin.clone()))?;
    Ok(Identity { enrollment, key })
}

/// A signed-in session with a remote server.
#[derive(Clone)]
pub struct Session {
    /// The session token (`lbs_…`), sent upstream as `Authorization: Bearer`.
    pub token: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session(…)")
    }
}

/// The `lb_session` value in a response's `Set-Cookie` headers, if any.
pub fn session_from_headers(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|c| {
            let first = c.split(';').next()?.trim();
            let (name, value) = first.split_once('=')?;
            (name == SESSION_COOKIE && !value.is_empty()).then(|| value.to_owned())
        })
}

/// Sign in to `base` with this computer's device key.
///
/// ┌────────────────────────────────────────────────────────────────────────┐
/// │ DEVICE-KEY LOGIN HANDSHAKE: matches lb-server/lb-auth on master as of   │
/// │ d15f35d, checked by reading that code; this branch predates the merge,  │
/// │ so it has only run against the fake server in `proxy`'s tests. Shape:   │
/// │  - `POST auth/device/challenge` → `{challenge_id, challenge}`, the      │
/// │    challenge 32 bytes base64url;                                        │
/// │  - signature (base64url) over `login_message(challenge, origin)`, with  │
/// │    `Origin: <origin>` sent so the server binds to the same origin (it   │
/// │    must be one of its relying-party or `extra_device_origins`);         │
/// │  - `POST auth/device/login` → `{user, token, expires_at}` + cookie.     │
/// └────────────────────────────────────────────────────────────────────────┘
pub async fn login(
    client: &reqwest::Client,
    base: &Url,
    identity: &Identity,
) -> Result<Session, DeviceError> {
    #[derive(Deserialize)]
    struct Challenge {
        challenge_id: String,
        challenge: String,
    }
    let origin = base.origin().ascii_serialization();
    let Identity { enrollment, key } = identity;
    if enrollment.origin != origin {
        return Err(DeviceError::NotEnrolled);
    }
    #[derive(Deserialize)]
    struct LoggedIn {
        token: Option<String>,
    }
    let challenge: Challenge = post_json(
        client,
        api(base, "auth/device/challenge")?,
        &serde_json::json!({}),
    )
    .await?
    .json()
    .await
    .map_err(|e| DeviceError::Protocol(e.to_string()))?;
    let bytes = B64
        .decode(challenge.challenge.trim_end_matches('='))
        .map_err(|e| DeviceError::Protocol(format!("challenge: {e}")))?;
    let signature = key.sign_login(&bytes, &origin);
    let resp = post_json(
        client,
        api(base, "auth/device/login")?,
        &serde_json::json!({
            "challenge_id": challenge.challenge_id,
            "key_id": enrollment.key_id,
            "signature": signature,
        }),
    )
    .await?;
    // The token is in the body for thin clients; the cookie is the fallback.
    let from_cookie = session_from_headers(resp.headers());
    let from_body = resp.json::<LoggedIn>().await.ok().and_then(|b| b.token);
    let token = from_body.or(from_cookie).ok_or_else(|| {
        DeviceError::Protocol("signed in, but the server sent no session token".into())
    })?;
    Ok(Session { token })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    #[test]
    fn login_signature_verifies_over_the_agreed_message() {
        let key = DeviceKey::generate();
        let challenge = [7u8; 32];
        let origin = "https://books.example.com";
        let sig = key.sign_login(&challenge, origin);

        let public: [u8; 32] = B64.decode(key.public_key()).unwrap().try_into().unwrap();
        let public = VerifyingKey::from_bytes(&public).unwrap();
        let sig: [u8; 64] = B64.decode(sig).unwrap().try_into().unwrap();
        let sig = Signature::from_bytes(&sig);

        let mut expected = b"librebooks device login v1\0".to_vec();
        expected.extend_from_slice(&challenge);
        expected.extend_from_slice(origin.as_bytes());
        public.verify_strict(&expected, &sig).unwrap();
        // Bound to the origin: the same signature is no good for another server.
        let mut elsewhere = b"librebooks device login v1\0".to_vec();
        elsewhere.extend_from_slice(&challenge);
        elsewhere.extend_from_slice(b"https://evil.example.com");
        assert!(public.verify(&elsewhere, &sig).is_err());
    }

    #[test]
    fn debug_never_shows_the_secret() {
        let key = DeviceKey::generate();
        let shown = format!("{key:?}");
        assert!(shown.contains(&key.public_key()));
        assert!(!shown.contains(&hex::encode(key.0.as_bytes())));
    }

    #[test]
    fn enrolment_input() {
        let (server, token) = parse_enrollment("  lbe_abc  ").unwrap();
        assert!(server.is_none());
        assert_eq!(token, "lbe_abc");

        let (server, token) = parse_enrollment("https://books.example.com/enroll/lbe_xyz").unwrap();
        assert_eq!(server.unwrap().as_str(), "https://books.example.com/");
        assert_eq!(token, "lbe_xyz");

        let (server, token) =
            parse_enrollment("https://books.example.com:8443/join?token=lbe_q&x=1").unwrap();
        assert_eq!(server.unwrap().as_str(), "https://books.example.com:8443/");
        assert_eq!(token, "lbe_q");

        assert!(parse_enrollment("").is_err());
    }

    #[test]
    fn session_cookie_from_set_cookie() {
        let mut h = reqwest::header::HeaderMap::new();
        h.append(
            reqwest::header::SET_COOKIE,
            "other=1; Path=/".parse().unwrap(),
        );
        h.append(
            reqwest::header::SET_COOKIE,
            "lb_session=lbs_abc; HttpOnly; Secure; SameSite=Strict"
                .parse()
                .unwrap(),
        );
        assert_eq!(session_from_headers(&h).as_deref(), Some("lbs_abc"));
    }
}
