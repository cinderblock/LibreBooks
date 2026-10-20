//! Thin-client mode: a loopback server that serves the web UI built into this
//! app and forwards `/api/*` to a remote LibreBooks server.
//!
//! The webview talks only to `http://127.0.0.1:<port>`, exactly as in local mode
//! (same SPA, same `window.__LIBREBOOKS__` contract, same per-launch token). This
//! proxy checks that token, strips it, and attaches the remote session it got by
//! device-key login (`crate::device`). Remote credentials never reach the
//! webview, and the SPA needs no special thin-client code path.
//!
//! Hardening:
//! - Requests must carry the per-launch token (any local process can connect to
//!   a loopback port; only this app's webview knows the token).
//! - `Host` must be the loopback address we bound (defeats DNS rebinding).
//! - Remote `Set-Cookie` never reaches the webview; the session stays here.
//! - Remote responses that a browser could run as a page (HTML, SVG, XML) get
//!   `Content-Security-Policy: sandbox`, so even if the webview were navigated to
//!   one it would run in an opaque origin, away from the token.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use futures::TryStreamExt;
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;
use url::Url;

use crate::device::{self, DeviceError, Identity, SESSION_COOKIE, Session};

/// Largest request body forwarded (the server's own limit).
const MAX_BODY: usize = 64 * 1024 * 1024;

/// Request headers never forwarded: hop-by-hop, the local credential, and
/// browser context that describes the loopback page rather than this client.
const DROP_REQUEST: &[&str] = &[
    "host",
    "authorization",
    "cookie",
    "origin",
    "referer",
    "connection",
    "keep-alive",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
    "accept-encoding",
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
];

/// Response headers never passed back to the webview.
const DROP_RESPONSE: &[&str] = &[
    "set-cookie",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "strict-transport-security",
    "alt-svc",
];

/// The thin-client proxy.
#[derive(Debug)]
pub struct Proxy {
    /// The remote server's base URL (ends in `/`).
    base: Url,
    token: Arc<str>,
    /// `Host` values accepted (`127.0.0.1:<port>`, `localhost:<port>`, and the
    /// web dev server's in debug builds).
    hosts: Vec<String>,
    client: reqwest::Client,
    /// This computer's device key for the server, or why there isn't one.
    identity: Result<Identity, DeviceError>,
    session: Mutex<Option<Session>>,
}

impl Proxy {
    pub fn new(
        base: Url,
        token: Arc<str>,
        hosts: Vec<String>,
        client: reqwest::Client,
        identity: Result<Identity, DeviceError>,
    ) -> Proxy {
        Proxy {
            base,
            token,
            hosts,
            client,
            identity,
            session: Mutex::new(None),
        }
    }

    /// Whether the request carries the per-launch token: `Authorization:
    /// Bearer`, or `?token=` (EventSource can't set headers).
    fn authorized(&self, headers: &HeaderMap, query: Option<&str>) -> bool {
        let given = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(ToOwned::to_owned)
            .or_else(|| query.and_then(query_token));
        given.is_some_and(|g| bool::from(g.as_bytes().ct_eq(self.token.as_bytes())))
    }

    /// The upstream URL for a local `/api/…` path and query, minus our token.
    fn upstream(&self, path: &str, query: Option<&str>) -> Result<Url, String> {
        let mut url = self
            .base
            .join(path.trim_start_matches('/'))
            .map_err(|e| e.to_string())?;
        url.set_query(None);
        if let Some(q) = query {
            let kept: Vec<(String, String)> = url::form_urlencoded::parse(q.as_bytes())
                .filter(|(k, _)| k != "token")
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            if !kept.is_empty() {
                url.query_pairs_mut().extend_pairs(kept);
            }
        }
        Ok(url)
    }

    /// The current session, signing in first if there is none. Concurrent
    /// requests wait for one sign-in rather than each starting their own.
    async fn session(&self, identity: &Identity) -> Result<Session, DeviceError> {
        let mut guard = self.session.lock().await;
        if let Some(s) = guard.as_ref() {
            return Ok(s.clone());
        }
        log::info!(
            "signing in to {} with this computer's device key",
            self.base
        );
        let s = device::login(&self.client, &self.base, identity).await?;
        *guard = Some(s.clone());
        Ok(s)
    }

    /// Drop `stale` if it is still the current session.
    async fn invalidate(&self, stale: &Session) {
        let mut guard = self.session.lock().await;
        if guard.as_ref().is_some_and(|s| s.cookie == stale.cookie) {
            *guard = None;
        }
    }

    /// Remember a session the server refreshed via `Set-Cookie`.
    async fn refresh(&self, headers: &HeaderMap) {
        if let Some(cookie) = device::session_from_headers(headers) {
            *self.session.lock().await = Some(Session { cookie });
        }
    }
}

/// The value of `token` in a query string.
fn query_token(query: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == "token")
        .map(|(_, v)| v.into_owned())
}

/// An error in the API's own shape (`plans/api.md`).
fn api_error(status: StatusCode, code: &str, message: &str, reason: Option<&str>) -> Response {
    let mut error = serde_json::json!({ "code": code, "message": message });
    if let Some(reason) = reason {
        error["detail"] = serde_json::json!({ "reason": reason });
    }
    (status, axum::Json(serde_json::json!({ "error": error }))).into_response()
}

/// Device-key and connectivity problems, as API errors the SPA can act on
/// (`detail.reason`, documented in `plans/desktop.md`).
fn device_error(server: &Url, e: &DeviceError) -> Response {
    match e {
        DeviceError::Unreachable { message, .. } => api_error(
            StatusCode::BAD_GATEWAY,
            "unreachable",
            &format!("Couldn't reach the LibreBooks server at {server}: {message}"),
            Some("remote_unreachable"),
        ),
        DeviceError::NotEnrolled | DeviceError::KeyMissing(_) => api_error(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            &format!("{e} (Settings → Server)"),
            Some("device_not_enrolled"),
        ),
        other => api_error(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            &format!("Couldn't sign in to {server}: {other}"),
            Some("device_login_failed"),
        ),
    }
}

/// The loopback app: API forwarded, everything else the embedded web UI.
pub fn router(proxy: Arc<Proxy>) -> Router {
    Router::new()
        .route("/api", any(forward))
        .route("/api/{*rest}", any(forward))
        .fallback(lb_server::assets::serve)
        .layer(middleware::from_fn_with_state(proxy.clone(), host_guard))
        .layer(middleware::map_response(security_headers))
        .with_state(proxy)
}

/// Refuse requests whose `Host` isn't our loopback address (DNS rebinding).
async fn host_guard(State(p): State<Arc<Proxy>>, req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| p.hosts.iter().any(|a| a.eq_ignore_ascii_case(h)));
    if ok {
        next.run(req).await
    } else {
        (StatusCode::MISDIRECTED_REQUEST, "unexpected Host").into_response()
    }
}

async fn security_headers(mut resp: Response) -> Response {
    let h = resp.headers_mut();
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    resp
}

/// Whether a browser could run this content type as a document with script.
fn scriptable(content_type: Option<&HeaderValue>) -> bool {
    let Some(ct) = content_type.and_then(|v| v.to_str().ok()) else {
        // Unknown content may be sniffed; nosniff covers most, sandbox the rest.
        return true;
    };
    let ct = ct
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    ct == "text/html"
        || ct == "application/xhtml+xml"
        || ct == "image/svg+xml"
        || ct == "text/xml"
        || ct == "application/xml"
        || ct.ends_with("+xml")
}

async fn forward(State(p): State<Arc<Proxy>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let path = parts.uri.path().to_owned();
    let query = parts.uri.query().map(ToOwned::to_owned);
    // Health is public on the server too; everything else needs our token and
    // a device identity for the server.
    let identity = if path == "/api/v1/health" {
        None
    } else {
        if !p.authorized(&parts.headers, query.as_deref()) {
            return api_error(
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "missing or wrong desktop token",
                None,
            );
        }
        match &p.identity {
            Ok(id) => Some(id),
            Err(e) => return device_error(&p.base, e),
        }
    };
    let url = match p.upstream(&path, query.as_deref()) {
        Ok(u) => u,
        Err(e) => return api_error(StatusCode::BAD_REQUEST, "invalid", &e, None),
    };
    let Ok(body) = axum::body::to_bytes(body, MAX_BODY).await else {
        return api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "invalid",
            "request body too large",
            None,
        );
    };
    let mut headers = HeaderMap::new();
    for (name, value) in &parts.headers {
        if !DROP_REQUEST.contains(&name.as_str()) {
            headers.append(name.clone(), value.clone());
        }
    }
    // We authenticated the caller with a bearer token, which no cross-site page
    // can send, so the CSRF guard's header is honest here.
    headers.insert(
        HeaderName::from_static("x-librebooks"),
        HeaderValue::from_static("1"),
    );

    let mut retried = false;
    loop {
        let session = match identity {
            None => None,
            Some(id) => match p.session(id).await {
                Ok(s) => Some(s),
                Err(e) => return device_error(&p.base, &e),
            },
        };
        let mut h = headers.clone();
        if let Some(s) = &session
            && let Ok(v) = HeaderValue::from_str(&format!("{SESSION_COOKIE}={}", s.cookie))
        {
            h.insert(header::COOKIE, v);
        }
        let resp = p
            .client
            .request(parts.method.clone(), url.clone())
            .headers(h)
            .body(body.clone())
            .send()
            .await;
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                return device_error(
                    &p.base,
                    &DeviceError::Unreachable {
                        url: url.to_string(),
                        message: e.to_string(),
                    },
                );
            }
        };
        if resp.status() == StatusCode::UNAUTHORIZED
            && let Some(stale) = &session
            && !retried
        {
            // Session expired or revoked: sign in again once, then retry.
            p.invalidate(stale).await;
            retried = true;
            continue;
        }
        p.refresh(resp.headers()).await;
        return respond(&p.base, resp);
    }
}

/// Turn the upstream response into ours, streaming the body (SSE included).
fn respond(base: &Url, resp: reqwest::Response) -> Response {
    let status = resp.status();
    let mut headers = HeaderMap::new();
    for (name, value) in resp.headers() {
        if !DROP_RESPONSE.contains(&name.as_str()) {
            headers.append(name.clone(), value.clone());
        }
    }
    // A redirect within the server becomes a redirect within the proxy.
    if let Some(loc) = headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|l| l.strip_prefix(base.as_str()))
        .and_then(|rest| HeaderValue::from_str(&format!("/{rest}")).ok())
    {
        headers.insert(header::LOCATION, loc);
    }
    if scriptable(headers.get(header::CONTENT_TYPE)) {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("sandbox; default-src 'none'"),
        );
    }
    let body = Body::from_stream(resp.bytes_stream().map_err(std::io::Error::other));
    let mut out = Response::new(body);
    *out.status_mut() = status;
    *out.headers_mut() = headers;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceKey;
    use crate::settings::DeviceEnrollment;
    use axum::Json;
    use axum::routing::{get, post};
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
    use ed25519_dalek::{Signature, VerifyingKey};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn proxy() -> Proxy {
        Proxy::new(
            Url::parse("https://books.example.com/lb/").unwrap(),
            Arc::from("secret"),
            vec!["127.0.0.1:1234".into()],
            reqwest::Client::new(),
            Err(DeviceError::NotEnrolled),
        )
    }

    #[test]
    fn upstream_keeps_the_prefix_and_drops_the_token() {
        let p = proxy();
        let u = p
            .upstream("/api/v1/events", Some("company=abc&token=secret"))
            .unwrap();
        assert_eq!(
            u.as_str(),
            "https://books.example.com/lb/api/v1/events?company=abc"
        );
        let u = p.upstream("/api/v1/session", Some("token=secret")).unwrap();
        assert_eq!(u.as_str(), "https://books.example.com/lb/api/v1/session");
    }

    #[test]
    fn token_by_header_or_query() {
        let p = proxy();
        let mut h = HeaderMap::new();
        assert!(!p.authorized(&h, None));
        assert!(p.authorized(&h, Some("company=x&token=secret")));
        assert!(!p.authorized(&h, Some("token=wrong")));
        h.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
        assert!(p.authorized(&h, None));
        h.insert(header::AUTHORIZATION, "Bearer secre".parse().unwrap());
        assert!(!p.authorized(&h, None));
    }

    #[test]
    fn scriptable_types_are_sandboxed() {
        let v = |s: &'static str| HeaderValue::from_static(s);
        assert!(scriptable(Some(&v("text/html; charset=utf-8"))));
        assert!(scriptable(Some(&v("image/svg+xml"))));
        assert!(scriptable(None));
        assert!(!scriptable(Some(&v("application/json"))));
        assert!(!scriptable(Some(&v("text/event-stream"))));
        assert!(!scriptable(Some(&v("application/pdf"))));
    }

    /// A fake LibreBooks server implementing the device-key handshake as
    /// `device::login` expects it, plus one authenticated endpoint.
    struct Fake {
        public_key: VerifyingKey,
        origin: String,
        logins: AtomicUsize,
        /// The session the server currently accepts.
        valid: std::sync::Mutex<String>,
    }

    fn fake_router(fake: Arc<Fake>) -> Router {
        Router::new()
            .route(
                "/api/v1/health",
                get(|| async { Json(serde_json::json!({ "ok": true })) }),
            )
            .route(
                "/api/v1/auth/device/challenge",
                post(|| async {
                    Json(serde_json::json!({
                        "challenge_id": "c1",
                        "challenge": B64.encode([9u8; 32]),
                    }))
                }),
            )
            .route(
                "/api/v1/auth/device/login",
                post(
                    |State(f): State<Arc<Fake>>, Json(body): Json<serde_json::Value>| async move {
                        let sig: [u8; 64] = B64
                            .decode(body["signature"].as_str().unwrap())
                            .unwrap()
                            .try_into()
                            .unwrap();
                        let msg = device::login_message(&[9u8; 32], &f.origin);
                        if f.public_key
                            .verify_strict(&msg, &Signature::from_bytes(&sig))
                            .is_err()
                        {
                            return StatusCode::UNAUTHORIZED.into_response();
                        }
                        let n = f.logins.fetch_add(1, Ordering::SeqCst) + 1;
                        let cookie = format!("s{n}");
                        *f.valid.lock().unwrap() = cookie.clone();
                        (
                            [(
                                header::SET_COOKIE,
                                format!("lb_session={cookie}; HttpOnly; SameSite=Strict"),
                            )],
                            Json(serde_json::json!({})),
                        )
                            .into_response()
                    },
                ),
            )
            .route(
                "/api/v1/session",
                get(
                    |State(f): State<Arc<Fake>>, headers: HeaderMap| async move {
                        let cookie = headers
                            .get(header::COOKIE)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_owned();
                        let csrf = headers.contains_key("x-librebooks");
                        let leaked = headers.contains_key(header::AUTHORIZATION);
                        if cookie != format!("lb_session={}", f.valid.lock().unwrap()) {
                            return StatusCode::UNAUTHORIZED.into_response();
                        }
                        Json(
                            serde_json::json!({ "cookie": cookie, "csrf": csrf, "leaked": leaked }),
                        )
                        .into_response()
                    },
                ),
            )
            .with_state(fake)
    }

    #[tokio::test]
    async fn signs_in_forwards_and_signs_in_again_when_the_session_dies() {
        let key = DeviceKey::generate();
        let public: [u8; 32] = B64.decode(key.public_key()).unwrap().try_into().unwrap();
        // Bind the fake first to learn its origin.
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let remote = l.local_addr().unwrap();
        let origin = format!("http://{remote}");
        let fake = Arc::new(Fake {
            public_key: VerifyingKey::from_bytes(&public).unwrap(),
            origin: origin.clone(),
            logins: AtomicUsize::new(0),
            valid: std::sync::Mutex::new(String::new()),
        });
        let app = fake_router(fake.clone());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

        let identity = Identity {
            enrollment: DeviceEnrollment {
                origin: origin.clone(),
                key_id: "k1".into(),
                name: "test".into(),
                public_key: key.public_key(),
            },
            key,
        };
        // The proxy's own address, to know the Host it must accept.
        let pl = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = pl.local_addr().unwrap();
        let proxy = Arc::new(Proxy::new(
            Url::parse(&format!("{origin}/")).unwrap(),
            Arc::from("tok"),
            vec![local.to_string()],
            reqwest::Client::new(),
            Ok(identity),
        ));
        let app = router(proxy);
        tokio::spawn(async move { axum::serve(pl, app).await.unwrap() });
        let base = format!("http://{local}");
        let client = reqwest::Client::new();

        // Without the token: refused locally.
        let r = client
            .get(format!("{base}/api/v1/session"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401);

        // With it: signed in by device key, forwarded with the session cookie
        // and the CSRF header, and without our bearer token.
        let r = client
            .get(format!("{base}/api/v1/session"))
            .bearer_auth("tok")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = r.json().await.unwrap();
        assert_eq!(v["cookie"], "lb_session=s1");
        assert_eq!(v["csrf"], true);
        assert_eq!(v["leaked"], false);
        assert_eq!(fake.logins.load(Ordering::SeqCst), 1);

        // The server forgets the session: the proxy signs in again and retries.
        *fake.valid.lock().unwrap() = "gone".into();
        let r = client
            .get(format!("{base}/api/v1/session?token=tok"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(fake.logins.load(Ordering::SeqCst), 2);

        // A foreign Host is refused (DNS rebinding).
        let r = client
            .get(format!("{base}/api/v1/health"))
            .header(header::HOST, "evil.example.com")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::MISDIRECTED_REQUEST);

        // Health is public and passes straight through.
        let r = client
            .get(format!("{base}/api/v1/health"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
    }

    #[tokio::test]
    async fn without_enrolment_api_calls_say_so() {
        let pl = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = pl.local_addr().unwrap();
        let proxy = Arc::new(Proxy::new(
            Url::parse("https://books.invalid/").unwrap(),
            Arc::from("tok"),
            vec![local.to_string()],
            reqwest::Client::new(),
            Err(DeviceError::NotEnrolled),
        ));
        let app = router(proxy);
        tokio::spawn(async move { axum::serve(pl, app).await.unwrap() });
        let r = reqwest::Client::new()
            .get(format!("http://{local}/api/v1/session"))
            .bearer_auth("tok")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401);
        let v: serde_json::Value = r.json().await.unwrap();
        assert_eq!(v["error"]["detail"]["reason"], "device_not_enrolled");
    }
}
