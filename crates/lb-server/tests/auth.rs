//! Sign-in over the API: an enrolment token, a device key, a session cookie,
//! and per-company access.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signer as _, SigningKey};
use http_body_util::BodyExt;
use lb_auth::{AuthConfig, AuthStore, EnrolmentPurpose, NewEnrolment, Role};
use lb_server::{AuthMode, Mode, ServerConfig, Sessions, router, state};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use tower::ServiceExt;

const ORIGIN: &str = "http://localhost:8740";

struct Server {
    app: axum::Router,
    store: AuthStore,
    _dir: tempfile::TempDir,
}

impl Server {
    fn new() -> Server {
        let dir = tempfile::tempdir().unwrap();
        let store = AuthStore::in_memory(AuthConfig::default()).unwrap();
        let config = ServerConfig {
            data_dir: dir.path().to_owned(),
            mode: Mode::Server,
            auth: AuthMode::Open,
            sessions: Some(Sessions {
                store: store.clone(),
                public_origin: Some(ORIGIN.into()),
                client_ip_header: None,
            }),
        };
        Server {
            app: router(state(&config).unwrap()),
            store,
            _dir: dir,
        }
    }

    async fn call(
        &self,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value, Option<String>) {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-librebooks", "1")
            .header(header::ORIGIN, ORIGIN);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        let req = match body {
            Some(b) => req
                .header("content-type", "application/json")
                .body(Body::from(b.to_string())),
            None => req.body(Body::empty()),
        }
        .unwrap();
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let set_cookie = res
            .headers()
            .get(header::SET_COOKIE)
            .map(|v| v.to_str().unwrap().to_owned());
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, v, set_cookie)
    }

    /// Enrol a device key for a new user of `role`; returns the cookie.
    async fn sign_in_new_user(&self, name: &str, role: Role) -> String {
        let minted = self
            .store
            .operator_enrolment(&NewEnrolment {
                purpose: EnrolmentPurpose::NewUser,
                role: Some(role),
                user_id: None,
                company_id: None,
                display_name: Some(name.into()),
                ttl_minutes: None,
            })
            .unwrap();
        let key = SigningKey::from_bytes(&rand_bytes());
        let public = URL_SAFE_NO_PAD.encode(key.verifying_key().as_bytes());
        let (s, enrolled, _) = self
            .call(
                "POST",
                "/api/v1/auth/device/enroll",
                None,
                Some(json!({ "token": minted.token, "public_key": public, "name": "Test laptop", "display_name": name })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{enrolled}");
        let key_id = enrolled["key"]["id"].as_str().unwrap().to_owned();

        let (s, ch, _) = self
            .call(
                "POST",
                "/api/v1/auth/device/challenge",
                None,
                Some(json!({})),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{ch}");
        let challenge: [u8; 32] = URL_SAFE_NO_PAD
            .decode(ch["challenge"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let sig = key.sign(&lb_auth::device_login_message(&challenge, ORIGIN));
        let (s, login, cookie) = self
            .call(
                "POST",
                "/api/v1/auth/device/login",
                None,
                Some(json!({ "challenge_id": ch["challenge_id"], "key_id": key_id, "signature": URL_SAFE_NO_PAD.encode(sig.to_bytes()) })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{login}");
        let cookie = cookie.expect("a session cookie");
        assert!(
            cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"),
            "{cookie}"
        );
        cookie.split(';').next().unwrap().to_owned()
    }
}

fn rand_bytes() -> [u8; 32] {
    rand::random()
}

#[tokio::test]
async fn nobody_gets_in_without_signing_in() {
    let s = Server::new();
    let (st, _, _) = s.call("GET", "/api/v1/companies", None, None).await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "open mode never applies once sign-in is configured"
    );
    let (st, _, _) = s
        .call(
            "GET",
            "/api/v1/companies",
            Some("lb_session=lbs_nonsense"),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_device_signs_in_and_sees_only_its_companies() {
    let s = Server::new();
    let admin = s.sign_in_new_user("Ada", Role::Admin).await;
    let (st, session, _) = s.call("GET", "/api/v1/session", Some(&admin), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(session["user"]["display_name"], "Ada");
    assert_eq!(session["sign_in"], true);

    let (st, created, _) = s
        .call(
            "POST",
            "/api/v1/companies",
            Some(&admin),
            Some(json!({ "info": { "name": "Ada Co" } })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{created}");
    let company = created["id"].as_str().unwrap().to_owned();

    // A bookkeeper with no access sees nothing, and the company "doesn't exist".
    let bob = s.sign_in_new_user("Bob", Role::Bookkeeper).await;
    let (_, list, _) = s.call("GET", "/api/v1/companies", Some(&bob), None).await;
    assert_eq!(list.as_array().unwrap().len(), 0);
    let (st, _, _) = s
        .call(
            "GET",
            &format!("/api/v1/c/{company}/accounts"),
            Some(&bob),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _, _) = s
        .call(
            "POST",
            "/api/v1/companies",
            Some(&bob),
            Some(json!({ "info": { "name": "Bob Co" } })),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Granted viewer access, Bob can read but not write.
    let (_, users, _) = s.call("GET", "/api/v1/users", Some(&admin), None).await;
    let bob_id = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["display_name"] == "Bob")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (st, _, _) = s
        .call(
            "PUT",
            &format!("/api/v1/users/{bob_id}/companies/{company}"),
            Some(&admin),
            Some(json!({ "role": "viewer" })),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _, _) = s
        .call(
            "GET",
            &format!("/api/v1/c/{company}/accounts"),
            Some(&bob),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) = s
        .call(
            "PUT",
            &format!("/api/v1/c/{company}/closing-date"),
            Some(&bob),
            Some(json!({ "closing_date": "2026-01-01" })),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Signing out ends the session.
    let (st, _, cleared) = s
        .call("POST", "/api/v1/auth/logout", Some(&bob), None)
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(cleared.unwrap().contains("Max-Age=0"));
    let (st, _, _) = s.call("GET", "/api/v1/session", Some(&bob), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admins_mint_enrolment_links() {
    let s = Server::new();
    let admin = s.sign_in_new_user("Ada", Role::Admin).await;
    let (st, minted, _) = s
        .call(
            "POST",
            "/api/v1/auth/enrollments",
            Some(&admin),
            Some(json!({ "purpose": "new_user", "role": "bookkeeper" })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{minted}");
    let url = minted["url"].as_str().unwrap();
    assert!(
        url.starts_with("http://localhost:8740/enroll#token=lbe_"),
        "{url}"
    );
    let token = minted["token"].as_str().unwrap();
    let (st, info, _) = s
        .call(
            "GET",
            &format!("/api/v1/auth/enroll?token={token}"),
            None,
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{info}");
    assert_eq!(info["purpose"], "new_user");
}
