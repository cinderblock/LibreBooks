//! Test helpers: a store with a hand-wound clock, and a software passkey.
//!
//! The authenticator is built by hand rather than with a FIDO emulator crate,
//! so tests control every byte (the origin in clientDataJSON, the RP ID hash,
//! the flags, the counter) and can break each one on purpose. It produces the
//! JSON `@simplewebauthn/browser` v14 produces: `RegistrationResponseJSON` and
//! `AuthenticationResponseJSON`, with the Level 3 extras (`publicKey`,
//! `publicKeyAlgorithm`, `authenticatorData`, `transports`).

#![allow(dead_code)]

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use lb_auth::{AuthConfig, AuthStore, ClientInfo, Clock, RelyingParty};
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

pub const ORIGIN: &str = "https://books.example.com";
pub const RP_ID: &str = "books.example.com";
pub const LOCAL_ORIGIN: &str = "http://localhost:8740";
pub const LAN_ORIGIN: &str = "http://192.168.1.10:8740";
pub const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36";

pub fn config(clock: &Clock) -> AuthConfig {
    AuthConfig {
        relying_parties: vec![
            RelyingParty::new(RP_ID, "LibreBooks", [ORIGIN]),
            RelyingParty::new("localhost", "LibreBooks (this computer)", [LOCAL_ORIGIN]),
        ],
        extra_device_origins: vec![LAN_ORIGIN.to_owned()],
        clock: clock.clone(),
        ..AuthConfig::default()
    }
}

pub fn start_time() -> jiff::Timestamp {
    "2026-10-19T12:00:00Z".parse().unwrap()
}

/// A fresh in-memory store and the clock that drives it.
pub fn store() -> (AuthStore, Clock) {
    let clock = Clock::manual(start_time());
    (AuthStore::in_memory(config(&clock)).unwrap(), clock)
}

pub fn client() -> ClientInfo {
    client_at("203.0.113.7")
}

pub fn client_at(ip: &str) -> ClientInfo {
    ClientInfo {
        ip: Some(ip.parse().unwrap()),
        user_agent: Some(UA.to_owned()),
    }
}

/// The body of `POST /auth/enroll/start` with just a token.
pub fn begin(token: &str) -> lb_auth::EnrolStartRequest {
    lb_auth::EnrolStartRequest {
        token: token.to_owned(),
        display_name: None,
    }
}

pub fn minutes(n: u64) -> Duration {
    Duration::from_secs(n * 60)
}

pub fn days(n: u64) -> Duration {
    Duration::from_secs(n * 24 * 60 * 60)
}

pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn unb64(s: &str) -> Vec<u8> {
    URL_SAFE_NO_PAD.decode(s).unwrap()
}

fn random<const N: usize>() -> [u8; N] {
    rand::random()
}

// --- CBOR, by hand, in CTAP2 canonical form --------------------------------

/// A CBOR item head: major type and argument (additional info 24/25/26 =
/// 1/2/4-byte argument follows).
fn head(major: u8, n: u64) -> Vec<u8> {
    let m = major << 5;
    match n {
        0..=23 => vec![m | n as u8],
        24..=0xff => vec![m | 0x18, n as u8],
        0x100..=0xffff => {
            let mut v = vec![m | 0x19];
            v.extend_from_slice(&(n as u16).to_be_bytes());
            v
        }
        _ => {
            let mut v = vec![m | 0x1a];
            v.extend_from_slice(&(n as u32).to_be_bytes());
            v
        }
    }
}

fn int(i: i64) -> Vec<u8> {
    if i >= 0 {
        head(0, i as u64)
    } else {
        head(1, (-1 - i) as u64)
    }
}

fn bytes(b: &[u8]) -> Vec<u8> {
    let mut v = head(2, b.len() as u64);
    v.extend_from_slice(b);
    v
}

fn text(s: &str) -> Vec<u8> {
    let mut v = head(3, s.len() as u64);
    v.extend_from_slice(s.as_bytes());
    v
}

fn map(entries: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut v = head(5, entries.len() as u64);
    for (k, val) in entries {
        v.extend_from_slice(k);
        v.extend_from_slice(val);
    }
    v
}

// --- The authenticator -------------------------------------------------------

const UP: u8 = 0x01;
const UV: u8 = 0x04;
const BE: u8 = 0x08;
const BS: u8 = 0x10;
const AT: u8 = 0x40;

/// Ways to make a response wrong (or unusual) on purpose.
#[derive(Clone, Debug)]
pub struct Client {
    /// The origin the "browser" puts in clientDataJSON.
    pub origin: String,
    /// The RP ID whose hash goes into authenticatorData (default: the options').
    pub rp_id: Option<String>,
    /// Replace the challenge in clientDataJSON.
    pub challenge: Option<String>,
    /// Replace the ceremony type in clientDataJSON.
    pub typ: Option<&'static str>,
    /// Set the user-verified flag.
    pub uv: bool,
}

impl Client {
    pub fn at(origin: &str) -> Client {
        Client {
            origin: origin.to_owned(),
            rp_id: None,
            challenge: None,
            typ: None,
            uv: true,
        }
    }
}

/// One software passkey.
#[derive(Clone, Debug)]
pub struct Authenticator {
    key: SigningKey,
    pub credential_id: Vec<u8>,
    pub user_handle: Option<Vec<u8>>,
    /// The signature counter; `None` for authenticators that always send 0
    /// (synced passkeys).
    pub counter: Option<u32>,
    pub backup_eligible: bool,
}

impl Default for Authenticator {
    fn default() -> Self {
        Self::new()
    }
}

impl Authenticator {
    /// A synced-passkey-like authenticator: backup eligible, counter always 0.
    pub fn new() -> Authenticator {
        Authenticator {
            key: SigningKey::from_slice(&random::<32>()).unwrap(),
            credential_id: random::<16>().to_vec(),
            user_handle: None,
            counter: None,
            backup_eligible: true,
        }
    }

    /// A hardware-key-like authenticator with a real counter.
    pub fn counting() -> Authenticator {
        Authenticator {
            counter: Some(0),
            backup_eligible: false,
            ..Authenticator::new()
        }
    }

    fn flags(&self, uv: bool) -> u8 {
        let mut f = UP;
        if uv {
            f |= UV;
        }
        if self.backup_eligible {
            f |= BE | BS;
        }
        f
    }

    fn next_counter(&mut self) -> u32 {
        match &mut self.counter {
            None => 0,
            Some(c) => {
                *c += 1;
                *c
            }
        }
    }

    fn client_data(typ: &str, options: &Value, client: &Client) -> Vec<u8> {
        let challenge = client
            .challenge
            .clone()
            .unwrap_or_else(|| options["challenge"].as_str().unwrap().to_owned());
        serde_json::to_vec(&json!({
            "type": client.typ.unwrap_or(typ),
            "challenge": challenge,
            "origin": client.origin,
            "crossOrigin": false,
        }))
        .unwrap()
    }

    fn uncompressed_point(&self) -> (Vec<u8>, Vec<u8>) {
        let point = self.key.verifying_key().to_encoded_point(false);
        (point.x().unwrap().to_vec(), point.y().unwrap().to_vec())
    }

    /// Answer `startRegistration({ optionsJSON: options })`.
    pub fn register(&mut self, options: &Value, client: &Client) -> Value {
        let rp_id = client
            .rp_id
            .clone()
            .unwrap_or_else(|| options["rp"]["id"].as_str().unwrap().to_owned());
        self.user_handle = Some(unb64(options["user"]["id"].as_str().unwrap()));
        let client_data = Self::client_data("webauthn.create", options, client);

        let (x, y) = self.uncompressed_point();
        let cose_key = map(&[
            (int(1), int(2)),  // kty: EC2
            (int(3), int(-7)), // alg: ES256
            (int(-1), int(1)), // crv: P-256
            (int(-2), bytes(&x)),
            (int(-3), bytes(&y)),
        ]);
        let counter = self.counter.unwrap_or(0);
        let mut auth_data = Sha256::digest(rp_id.as_bytes()).to_vec();
        auth_data.push(self.flags(client.uv) | AT);
        auth_data.extend_from_slice(&counter.to_be_bytes());
        auth_data.extend_from_slice(&[0u8; 16]); // AAGUID
        auth_data.extend_from_slice(&(self.credential_id.len() as u16).to_be_bytes());
        auth_data.extend_from_slice(&self.credential_id);
        auth_data.extend_from_slice(&cose_key);

        let attestation_object = map(&[
            (text("fmt"), text("none")),
            (text("attStmt"), map(&[])),
            (text("authData"), bytes(&auth_data)),
        ]);
        // SubjectPublicKeyInfo for a P-256 key: fixed prefix + 0x04 || x || y.
        let mut spki = hex_decode("3059301306072a8648ce3d020106082a8648ce3d030107034200");
        spki.push(0x04);
        spki.extend_from_slice(&x);
        spki.extend_from_slice(&y);

        json!({
            "id": b64(&self.credential_id),
            "rawId": b64(&self.credential_id),
            "type": "public-key",
            "response": {
                "clientDataJSON": b64(&client_data),
                "attestationObject": b64(&attestation_object),
                "authenticatorData": b64(&auth_data),
                "transports": ["internal", "hybrid"],
                "publicKeyAlgorithm": -7,
                "publicKey": b64(&spki),
            },
            "authenticatorAttachment": "platform",
            "clientExtensionResults": {},
        })
    }

    /// Answer `startAuthentication({ optionsJSON: options })`.
    pub fn login(&mut self, options: &Value, client: &Client) -> Value {
        let rp_id = client
            .rp_id
            .clone()
            .unwrap_or_else(|| options["rpId"].as_str().unwrap().to_owned());
        let client_data = Self::client_data("webauthn.get", options, client);
        let counter = self.next_counter();
        let mut auth_data = Sha256::digest(rp_id.as_bytes()).to_vec();
        auth_data.push(self.flags(client.uv));
        auth_data.extend_from_slice(&counter.to_be_bytes());
        let mut signed = auth_data.clone();
        signed.extend_from_slice(&Sha256::digest(&client_data));
        let signature: Signature = self.key.sign(&signed);
        json!({
            "id": b64(&self.credential_id),
            "rawId": b64(&self.credential_id),
            "type": "public-key",
            "response": {
                "clientDataJSON": b64(&client_data),
                "authenticatorData": b64(&auth_data),
                "signature": b64(signature.to_der().as_bytes()),
                "userHandle": b64(self.user_handle.as_deref().unwrap()),
            },
            "authenticatorAttachment": "platform",
            "clientExtensionResults": {},
        })
    }
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
