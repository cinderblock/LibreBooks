//! Who is calling, and whether they may.
//!
//! Three ways in, by deployment:
//! - **Local token**: the desktop app serves its own UI in-process and hands the
//!   webview a per-launch secret (`Authorization: Bearer …`). Nobody else on the
//!   machine can read it.
//! - **Sessions**: passkeys or device keys (`lb-auth`), cookie `lb_session`.
//!   Wired in once `lb-auth` lands.
//! - **Open**: no authentication, for tests and development only. The server
//!   refuses to listen beyond loopback in this mode.
//!
//! State-changing requests must also carry `X-LibreBooks: 1` (a header a browser
//! will not add to a cross-site request without a CORS preflight the server never
//! approves), which closes cross-site request forgery for cookie sessions.

use axum::extract::{FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{Method, header};
use axum::middleware::Next;
use axum::response::Response;
use subtle::ConstantTimeEq;

use crate::error::ApiError;
use crate::state::AppState;

/// How the server authenticates requests.
#[derive(Clone, Debug)]
pub enum AuthMode {
    Open,
    LocalToken(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Bookkeeper,
    Admin,
}

/// The authenticated caller.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Principal {
    pub user_id: Option<String>,
    pub display_name: String,
    pub role: Role,
}

impl Principal {
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
}

impl FromRequestParts<AppState> for Principal {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match &state.auth {
            AuthMode::Open => Ok(Principal {
                user_id: None,
                display_name: "Local".into(),
                role: Role::Admin,
            }),
            AuthMode::LocalToken(expected) => {
                let given = parts
                    .headers
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .or_else(|| {
                        // EventSource can't set headers; the desktop passes the token
                        // in the query string for /events only.
                        parts
                            .uri
                            .query()
                            .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")))
                    });
                match given {
                    Some(g) if bool::from(g.as_bytes().ct_eq(expected.as_bytes())) => {
                        Ok(Principal {
                            user_id: None,
                            display_name: "This computer".into(),
                            role: Role::Admin,
                        })
                    }
                    _ => Err(ApiError::unauthenticated()),
                }
            }
        }
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
