//! Who is calling, and whether they may.
//!
//! Ways in, by deployment:
//! - **Sessions**: people sign in with passkeys, thin clients with device keys
//!   (`lb-auth`); the session token travels as the `lb_session` cookie (or, for
//!   API clients, `Authorization: Bearer lbs_…`). Each company checks the
//!   person's access to it.
//! - **Local token**: the desktop app serves its own UI in-process and hands the
//!   webview a per-launch secret (`Authorization: Bearer …`). Nobody else on the
//!   machine can read it.
//! - **Open**: no authentication, for tests and development only. The server
//!   refuses to listen beyond loopback in this mode, and it never applies once
//!   sessions are configured.
//!
//! State-changing requests must also carry `X-LibreBooks: 1` (a header a browser
//! will not add to a cross-site request without a CORS preflight the server never
//! approves), which closes cross-site request forgery for cookie sessions.

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, header};
use axum::middleware::Next;
use axum::response::Response;
use lb_auth::{AuthStore, ClientInfo};
use subtle::ConstantTimeEq;

pub use lb_auth::Role;

use crate::error::ApiError;
use crate::state::AppState;

/// The cookie holding a session token.
pub const SESSION_COOKIE: &str = "lb_session";

/// How the server authenticates requests without a session.
#[derive(Clone, Debug)]
pub enum AuthMode {
    Open,
    LocalToken(String),
}

/// Passkey and device-key sign-in.
#[derive(Clone, Debug)]
pub struct Sessions {
    pub store: AuthStore,
    /// The address people use (`https://books.example.com`): enrolment links
    /// point here, and device logins from clients that send no `Origin` are
    /// bound to it.
    pub public_origin: Option<String>,
    /// A header carrying the real client address when a trusted proxy on this
    /// machine (cloudflared, Caddy) forwards requests: `cf-connecting-ip`,
    /// `x-forwarded-for`. Trusted only from loopback peers.
    pub client_ip_header: Option<String>,
}

/// The authenticated caller.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Principal {
    pub user_id: Option<String>,
    pub display_name: String,
    pub role: Role,
    /// The signed-in session, for people (not the desktop's local token).
    #[serde(skip)]
    pub session: Option<lb_auth::Principal>,
}

impl Principal {
    /// The caller's server-wide role must be at least `role`.
    pub fn require(&self, role: Role) -> Result<(), ApiError> {
        if self.role >= role {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }

    /// The store's notion of who is acting.
    pub fn actor(&self) -> lb_store::Actor {
        lb_store::Actor {
            user_id: self.user_id.clone(),
            may_override_closing_date: self.role == Role::Admin,
        }
    }

    fn local(display_name: &str) -> Principal {
        Principal {
            user_id: None,
            display_name: display_name.into(),
            role: Role::Admin,
            session: None,
        }
    }

    fn from_session(p: lb_auth::Principal) -> Principal {
        Principal {
            user_id: Some(p.user_id.to_string()),
            display_name: p.display_name.clone(),
            role: p.role,
            session: Some(p),
        }
    }

    /// The signed-in person (for endpoints that only make sense for one).
    pub fn person(&self) -> Result<&lb_auth::Principal, ApiError> {
        self.session.as_ref().ok_or_else(|| {
            ApiError::invalid(
                "this is for people signed in with a passkey or device, not this computer's own session",
            )
        })
    }
}

/// A token from the `Authorization` header or the session cookie.
pub fn token_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(b) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        return Some(b.trim().to_owned());
    }
    cookie(headers, SESSION_COOKIE)
}

/// A token from the headers, or (for `EventSource`, which can't set headers)
/// the `token` query parameter.
pub fn presented_token(parts: &Parts) -> Option<String> {
    token_from_headers(&parts.headers).or_else(|| {
        parts
            .uri
            .query()
            .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")))
            .map(str::to_owned)
    })
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|v| {
            v.split(';').find_map(|kv| {
                let (k, val) = kv.trim().split_once('=')?;
                (k == name && !val.is_empty()).then(|| val.to_owned())
            })
        })
}

impl FromRequestParts<AppState> for Principal {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(t) = presented_token(parts) {
            if let AuthMode::LocalToken(expected) = &state.auth
                && bool::from(t.as_bytes().ct_eq(expected.as_bytes()))
            {
                return Ok(Principal::local("This computer"));
            }
            if let Some(s) = &state.sessions
                && t.starts_with("lbs_")
            {
                let store = s.store.clone();
                let p = tokio::task::spawn_blocking(move || store.validate_session(&t)).await??;
                return Ok(Principal::from_session(p));
            }
            return Err(ApiError::unauthenticated());
        }
        if matches!(state.auth, AuthMode::Open) && state.sessions.is_none() {
            return Ok(Principal::local("Local"));
        }
        Err(ApiError::unauthenticated())
    }
}

/// The client's address and browser, for rate limiting and the session list.
#[derive(Clone, Debug)]
pub struct Client(pub ClientInfo);

impl FromRequestParts<AppState> for Client {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip());
        let forwarded = state
            .sessions
            .as_ref()
            .and_then(|s| s.client_ip_header.as_deref())
            // While the tunnel is on, cloudflared on this machine forwards the
            // real address in `cf-connecting-ip`.
            .or_else(|| state.ops.forwarded_ip_header())
            .filter(|_| peer.is_some_and(|p| p.is_loopback()))
            .and_then(|h| parts.headers.get(h))
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .and_then(|v| v.trim().parse::<IpAddr>().ok());
        let user_agent = parts
            .headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.chars().take(300).collect());
        Ok(Client(ClientInfo {
            ip: forwarded.or(peer),
            user_agent,
        }))
    }
}

/// The origin the client is talking to: the browser's `Origin` header, else the
/// configured public origin (non-browser clients).
#[derive(Clone, Debug)]
pub struct RequestOrigin(pub String);

impl FromRequestParts<AppState> for RequestOrigin {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(o) = parts
            .headers
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
        {
            return Ok(RequestOrigin(o.to_owned()));
        }
        state
            .sessions
            .as_ref()
            .and_then(|s| s.public_origin.clone())
            .map(RequestOrigin)
            .ok_or_else(|| {
                ApiError::invalid("the request has no Origin and the server has no public URL set")
            })
    }
}

/// Middleware: refuse state-changing API requests without `X-LibreBooks`.
pub async fn csrf_guard(req: Request, next: Next) -> Result<Response, ApiError> {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    let bearer = req.headers().get(header::AUTHORIZATION).is_some();
    if !safe && !bearer && req.headers().get("x-librebooks").is_none() {
        return Err(ApiError::forbidden()
            .with_detail(serde_json::json!({ "reason": "missing X-LibreBooks header" })));
    }
    Ok(next.run(req).await)
}

/// `Set-Cookie` for a new session (or, with `None`, clearing it). `Secure`
/// unless the page is plain-HTTP localhost, where browsers would drop it.
pub fn session_cookie(token: Option<&str>, origin: &str, max_age_secs: u64) -> String {
    let secure = if origin.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    match token {
        Some(t) => format!(
            "{SESSION_COOKIE}={t}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age_secs}{secure}"
        ),
        None => format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{secure}"),
    }
}
