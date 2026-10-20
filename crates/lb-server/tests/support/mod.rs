//! A server in a temporary folder, driven through the router without a socket.
#![allow(dead_code)]

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signer as _, SigningKey};
use http_body_util::BodyExt;
use lb_auth::{AuthConfig, AuthStore, EnrolmentPurpose, NewEnrolment, Role};
use lb_server::{AppState, AuthMode, Mode, OpsOptions, ServerConfig, Sessions, router, state_with};
use serde_json::{Value, json};
use tower::ServiceExt;

pub const ORIGIN: &str = "http://localhost:8740";

pub struct Server {
    pub app: axum::Router,
    pub state: AppState,
    pub store: Option<AuthStore>,
    pub dir: tempfile::TempDir,
    /// Sent with every request (a session cookie).
    pub cookie: Option<String>,
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub bytes: Vec<u8>,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).unwrap_or(Value::Null)
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

impl Server {
    /// No sign-in (loopback development mode): every caller is an admin.
    pub fn open(options: OpsOptions) -> Server {
        Server::build(None, options)
    }

    /// Passkey/device sign-in; call [`Server::sign_in`] for a cookie.
    pub fn with_sessions(options: OpsOptions) -> Server {
        let store = AuthStore::in_memory(AuthConfig::default()).unwrap();
        Server::build(Some(store), options)
    }

    fn build(store: Option<AuthStore>, options: OpsOptions) -> Server {
        let dir = tempfile::tempdir().unwrap();
        Server::in_dir(dir, store, options)
    }

    pub fn in_dir(dir: tempfile::TempDir, store: Option<AuthStore>, options: OpsOptions) -> Server {
        let config = ServerConfig {
            data_dir: dir.path().to_owned(),
            mode: Mode::Server,
            auth: AuthMode::Open,
            sessions: store.clone().map(|store| Sessions {
                store,
                public_origin: Some(ORIGIN.into()),
                client_ip_header: None,
            }),
        };
        let state = state_with(&config, options).unwrap();
        Server {
            app: router(state.clone()),
            state,
            store,
            dir,
            cookie: None,
        }
    }

    pub async fn send(&self, req: axum::http::request::Builder, body: Body) -> Reply {
        let mut req = req
            .header("x-librebooks", "1")
            .header(header::ORIGIN, ORIGIN);
        if let Some(c) = &self.cookie {
            req = req.header(header::COOKIE, c);
        }
        let res = self
            .app
            .clone()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
        Reply {
            status,
            headers,
            bytes,
        }
    }

    pub async fn raw(&self, method: &str, uri: &str, body: Option<Value>) -> Reply {
        let req = Request::builder().method(method).uri(uri);
        match body {
            Some(b) => {
                self.send(
                    req.header("content-type", "application/json"),
                    Body::from(b.to_string()),
                )
                .await
            }
            None => self.send(req, Body::empty()).await,
        }
    }

    pub async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let r = self.raw(method, uri, body).await;
        (r.status, r.json())
    }

    /// A multipart form: `file` (if any) and text fields.
    pub async fn upload(
        &self,
        uri: &str,
        file: Option<(&str, &[u8])>,
        fields: &[(&str, &str)],
    ) -> (StatusCode, Value) {
        const B: &str = "lbtestboundary7f3a";
        let mut body = Vec::new();
        for (k, v) in fields {
            body.extend_from_slice(
                format!("--{B}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n")
                    .as_bytes(),
            );
        }
        if let Some((name, bytes)) = file {
            body.extend_from_slice(
                format!(
                    "--{B}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
                )
                .as_bytes(),
            );
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{B}--\r\n").as_bytes());
        let req = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", format!("multipart/form-data; boundary={B}"));
        let r = self.send(req, Body::from(body)).await;
        (r.status, r.json())
    }

    /// A new user of `role`, signed in with a device key; returns the cookie.
    pub async fn sign_in(&self, name: &str, role: Role) -> String {
        let store = self.store.as_ref().expect("a server with sessions");
        let minted = store
            .operator_enrolment(&NewEnrolment {
                purpose: EnrolmentPurpose::NewUser,
                role: Some(role),
                user_id: None,
                company_id: None,
                display_name: Some(name.into()),
                ttl_minutes: None,
            })
            .unwrap();
        let key = SigningKey::from_bytes(&rand::random());
        let public = URL_SAFE_NO_PAD.encode(key.verifying_key().as_bytes());
        let (s, enrolled) = self
            .call(
                "POST",
                "/api/v1/auth/device/enroll",
                Some(json!({ "token": minted.token, "public_key": public, "name": "Test laptop", "display_name": name })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{enrolled}");
        let key_id = enrolled["key"]["id"].as_str().unwrap().to_owned();
        let (s, ch) = self
            .call("POST", "/api/v1/auth/device/challenge", Some(json!({})))
            .await;
        assert_eq!(s, StatusCode::OK, "{ch}");
        let challenge: [u8; 32] = URL_SAFE_NO_PAD
            .decode(ch["challenge"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let sig = key.sign(&lb_auth::device_login_message(&challenge, ORIGIN));
        let r = self
            .raw(
                "POST",
                "/api/v1/auth/device/login",
                Some(json!({
                    "challenge_id": ch["challenge_id"],
                    "key_id": key_id,
                    "signature": URL_SAFE_NO_PAD.encode(sig.to_bytes()),
                })),
            )
            .await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text());
        let set = r.headers[header::SET_COOKIE].to_str().unwrap();
        set.split(';').next().unwrap().to_owned()
    }

    /// A new company; its id.
    pub async fn company(&self, name: &str) -> String {
        let (s, v) = self
            .call(
                "POST",
                "/api/v1/companies",
                Some(json!({ "info": { "name": name } })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["id"].as_str().unwrap().to_owned()
    }

    /// A new account in `company`; its id.
    pub async fn account(&self, company: &str, name: &str, kind: &str) -> String {
        let (s, v) = self
            .call(
                "POST",
                &format!("/api/v1/c/{company}/accounts"),
                Some(account(name, kind)),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        v["id"].as_str().unwrap().to_owned()
    }
}

pub fn account(name: &str, t: &str) -> Value {
    json!({
        "id": uuid::Uuid::now_v7().to_string(), "name": name, "full_name": name, "parent_id": null, "sublevel": 0,
        "account_type": t, "number": null, "description": null, "bank_account_number": null,
        "tax_line": null, "role": null, "is_active": true, "currency": null,
        "cash_flow_class": null, "edit_seq": 0
    })
}

/// Serve `app` on a loopback port; its base URL.
pub async fn serve(app: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

/// Wait (up to 20 s) until `f` holds.
pub async fn eventually<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if f().await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}
