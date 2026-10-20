//! Cloudflare Access JWT verification against a local key server, with keys made
//! for these tests only.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::response::IntoResponse;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use lb_tunnel::{AccessError, AccessVerifier};
use rsa::RsaPrivateKey;
use rsa::pkcs1v15::Pkcs1v15Sign;
use rsa::pkcs8::DecodePrivateKey as _;
use rsa::sha2::{Digest as _, Sha256};
use rsa::traits::PublicKeyParts as _;
use serde_json::json;

const ISSUER: &str = "https://testteam.cloudflareaccess.com";
const AUD: &str = "4714c1358e65fe4b408ad6d432a5f878f08194bdb4752441fd56faefa9b2b6f2";
const NOW: i64 = 1_792_400_000;

fn key(name: &str) -> RsaPrivateKey {
    let pem =
        std::fs::read_to_string(format!("{}/testdata/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    RsaPrivateKey::from_pkcs8_pem(&pem).unwrap()
}

fn jwk(key: &RsaPrivateKey, kid: &str) -> serde_json::Value {
    let public = key.to_public_key();
    json!({
        "kid": kid,
        "kty": "RSA",
        "alg": "RS256",
        "use": "sig",
        "n": URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
        "e": URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
    })
}

fn b64(v: &serde_json::Value) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(v).unwrap())
}

fn sign(key: &RsaPrivateKey, header: &serde_json::Value, claims: &serde_json::Value) -> String {
    let signing_input = format!("{}.{}", b64(header), b64(claims));
    let digest = Sha256::digest(signing_input.as_bytes());
    let sig = key.sign(Pkcs1v15Sign::new::<Sha256>(), &digest).unwrap();
    format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(sig))
}

fn claims() -> serde_json::Value {
    json!({
        "aud": [AUD],
        "email": "cameron@example.com",
        "exp": NOW + 3600,
        "iat": NOW,
        "nbf": NOW,
        "iss": ISSUER,
        "type": "app",
        "identity_nonce": "6ei69kawdKzMIAPF",
        "sub": "7335d417-61da-459d-899c-0a01c76a2f94",
        "country": "US"
    })
}

fn header(kid: &str) -> serde_json::Value {
    json!({ "alg": "RS256", "kid": kid, "typ": "JWT" })
}

/// Serves `/cdn-cgi/access/certs` from a swappable key set; counts fetches.
struct Certs {
    body: Mutex<serde_json::Value>,
    fetches: Mutex<u32>,
}

async fn certs(State(c): State<Arc<Certs>>) -> impl IntoResponse {
    *c.fetches.lock().unwrap() += 1;
    axum::Json(c.body.lock().unwrap().clone())
}

async fn start(keys: serde_json::Value) -> (String, Arc<Certs>) {
    let c = Arc::new(Certs {
        body: Mutex::new(json!({ "keys": keys, "public_cert": {}, "public_certs": [] })),
        fetches: Mutex::new(0),
    });
    let app = axum::Router::new()
        .route("/cdn-cgi/access/certs", axum::routing::get(certs))
        .with_state(Arc::clone(&c));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/cdn-cgi/access/certs",
        listener.local_addr().unwrap()
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, c)
}

fn verifier(url: &str) -> AccessVerifier {
    AccessVerifier::with_urls(ISSUER, url, vec![AUD.into()]).unwrap()
}

#[tokio::test]
async fn a_valid_assertion_verifies_and_keys_are_cached() {
    let k = key("access-test-key.pem");
    let (url, server) = start(json!([jwk(&k, "k1")])).await;
    let v = verifier(&url);
    let token = sign(&k, &header("k1"), &claims());
    let c = v.verify_at(&token, NOW + 10).await.unwrap();
    assert_eq!(c.email.as_deref(), Some("cameron@example.com"));
    assert_eq!(c.kind.as_deref(), Some("app"));
    assert_eq!(c.aud, [AUD]);
    // A string audience works too.
    let mut one = claims();
    one["aud"] = json!(AUD);
    v.verify_at(&sign(&k, &header("k1"), &one), NOW)
        .await
        .unwrap();
    assert_eq!(*server.fetches.lock().unwrap(), 1, "keys fetched once");
}

#[tokio::test]
async fn claims_are_checked() {
    let k = key("access-test-key.pem");
    let (url, _server) = start(json!([jwk(&k, "k1")])).await;
    let v = verifier(&url);
    let with = |field: &str, value: serde_json::Value| {
        let mut c = claims();
        c[field] = value;
        sign(&k, &header("k1"), &c)
    };
    let cases = [
        (with("aud", json!(["another-app"])), AccessError::Audience),
        (
            with("iss", json!("https://otherteam.cloudflareaccess.com")),
            AccessError::Issuer("https://otherteam.cloudflareaccess.com".into()),
        ),
        (with("exp", json!(NOW - 61)), AccessError::Expired),
        (with("nbf", json!(NOW + 61)), AccessError::NotYet),
    ];
    for (token, want) in cases {
        assert_eq!(v.verify_at(&token, NOW).await.unwrap_err(), want);
    }
    // Within a minute of drift either way is fine.
    v.verify_at(&with("exp", json!(NOW - 30)), NOW)
        .await
        .unwrap();
    v.verify_at(&with("nbf", json!(NOW + 30)), NOW)
        .await
        .unwrap();
}

#[tokio::test]
async fn forgeries_are_refused() {
    let k = key("access-test-key.pem");
    let other = key("access-other-key.pem");
    let (url, _server) = start(json!([jwk(&k, "k1")])).await;
    let v = verifier(&url);

    // Signed by someone else, claiming Access's key id.
    let forged = sign(&other, &header("k1"), &claims());
    assert_eq!(
        v.verify_at(&forged, NOW).await.unwrap_err(),
        AccessError::Signature
    );

    // A genuine token with its claims edited.
    let token = sign(&k, &header("k1"), &claims());
    let parts: Vec<&str> = token.split('.').collect();
    let mut edited = claims();
    edited["email"] = json!("attacker@example.com");
    let tampered = format!("{}.{}.{}", parts[0], b64(&edited), parts[2]);
    assert_eq!(
        v.verify_at(&tampered, NOW).await.unwrap_err(),
        AccessError::Signature
    );

    // Algorithms other than RS256, including none.
    for alg in ["none", "HS256", "RS512"] {
        let h = json!({ "alg": alg, "kid": "k1" });
        let t = format!("{}.{}.", b64(&h), b64(&claims()));
        assert_eq!(
            v.verify_at(&t, NOW).await.unwrap_err(),
            AccessError::Algorithm(alg.into())
        );
    }

    for junk in [
        "",
        "a.b",
        "a.b.c.d",
        "!!.!!.!!",
        &format!("{}.x.y", b64(&json!({"alg":"RS256"}))),
    ] {
        let e = v.verify_at(junk, NOW).await.unwrap_err();
        assert!(
            matches!(e, AccessError::Missing | AccessError::Malformed),
            "{junk:?}: {e:?}"
        );
    }
}

#[tokio::test]
async fn a_rotated_key_is_fetched_when_first_seen() {
    let k1 = key("access-test-key.pem");
    let k2 = key("access-other-key.pem");
    let (url, server) = start(json!([jwk(&k1, "k1")])).await;
    let v = verifier(&url).with_min_refetch(Duration::ZERO);
    v.verify_at(&sign(&k1, &header("k1"), &claims()), NOW)
        .await
        .unwrap();

    // Access rotates: the set now has k2 (and still k1).
    *server.body.lock().unwrap() = json!({ "keys": [jwk(&k2, "k2"), jwk(&k1, "k1")] });
    v.verify_at(&sign(&k2, &header("k2"), &claims()), NOW)
        .await
        .unwrap();
    assert_eq!(*server.fetches.lock().unwrap(), 2);

    // A kid nobody has: refetched, then refused.
    let e = v
        .verify_at(&sign(&k2, &header("k9"), &claims()), NOW)
        .await
        .unwrap_err();
    assert_eq!(e, AccessError::UnknownKey);
}

#[tokio::test]
async fn unknown_key_ids_cannot_make_it_hammer_cloudflare() {
    let k = key("access-test-key.pem");
    let (url, server) = start(json!([jwk(&k, "k1")])).await;
    let v = verifier(&url); // the real 30-second minimum
    v.verify_at(&sign(&k, &header("k1"), &claims()), NOW)
        .await
        .unwrap();
    for i in 0..20 {
        let t = sign(&k, &header(&format!("made-up-{i}")), &claims());
        assert_eq!(
            v.verify_at(&t, NOW).await.unwrap_err(),
            AccessError::UnknownKey
        );
    }
    assert_eq!(*server.fetches.lock().unwrap(), 1);
}

#[tokio::test]
async fn unreachable_keys_are_an_error_in_words() {
    let v =
        AccessVerifier::with_urls(ISSUER, "http://127.0.0.1:1/certs", vec![AUD.into()]).unwrap();
    let k = key("access-test-key.pem");
    let e = v
        .verify_at(&sign(&k, &header("k1"), &claims()), NOW)
        .await
        .unwrap_err();
    assert!(matches!(e, AccessError::Keys(_)), "{e:?}");
}
