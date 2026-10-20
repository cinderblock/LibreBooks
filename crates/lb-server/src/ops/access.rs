//! Optional Cloudflare Access check, as a second layer behind a tunnel.
//!
//! When the public hostname sits behind a Cloudflare Access application, every
//! request Cloudflare forwards carries `Cf-Access-Jwt-Assertion`, signed by the
//! team's keys. Verifying it here means a tunnel route accidentally left without
//! Access (or a request that reaches the tunnel some other way) still can't get
//! in. Passkeys stay the sign-in; this only decides who may knock.
//!
//! Only requests that came **through Cloudflare** are checked: they carry
//! `cf-ray`, which Cloudflare's edge always sets and a client cannot remove.
//! Requests that reach the server directly (localhost, the LAN) are not affected.

use std::sync::{Arc, RwLock};

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use lb_tunnel::{AccessConfig, AccessVerifier};
use serde_json::json;

use crate::error::ApiError;
use crate::state::AppState;

/// The header Cloudflare's edge adds to every request it forwards.
pub const CF_RAY: &str = "cf-ray";
/// The header Cloudflare Access adds.
pub const ASSERTION: &str = "cf-access-jwt-assertion";

#[derive(Debug, Default)]
pub(crate) struct AccessSlot(RwLock<Option<Arc<AccessVerifier>>>);

impl AccessSlot {
    /// Use `config` from now on (`None` turns the check off). `certs_url`
    /// overrides where the team's keys are fetched (tests).
    pub(crate) fn set(&self, config: Option<&AccessConfig>, certs_url: Option<&str>) {
        let verifier = config.and_then(|c| {
            let built = match certs_url {
                None => AccessVerifier::new(c),
                Some(url) => {
                    let team = c.team_domain.trim().trim_start_matches("https://");
                    let team = team.trim_end_matches('/');
                    let host = if team.contains('.') {
                        team.to_owned()
                    } else {
                        format!("{team}.cloudflareaccess.com")
                    };
                    AccessVerifier::with_urls(format!("https://{host}"), url, c.audience.clone())
                }
            };
            match built {
                Ok(v) => Some(Arc::new(v)),
                Err(e) => {
                    tracing::error!(error = %e, "the Cloudflare Access check could not be set up");
                    None
                }
            }
        });
        *self
            .0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = verifier;
    }

    fn get(&self) -> Option<Arc<AccessVerifier>> {
        self.0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

fn refused(message: &str) -> ApiError {
    ApiError::forbidden().with_detail(json!({ "reason": "cloudflare_access", "message": message }))
}

/// Middleware: refuse requests from Cloudflare without a valid Access assertion.
/// The verified claims are left in the request's extensions.
#[allow(clippy::needless_pass_by_value)] // axum's `from_fn_with_state` hands the state over
pub async fn guard(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let Some(verifier) = state.ops.access.get() else {
        return Ok(next.run(req).await);
    };
    if !req.headers().contains_key(CF_RAY) {
        return Ok(next.run(req).await);
    }
    let Some(token) = req
        .headers()
        .get(ASSERTION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
    else {
        return Err(refused(&lb_tunnel::AccessError::Missing.to_string()));
    };
    match verifier.verify(&token).await {
        Ok(claims) => {
            req.extensions_mut().insert(claims);
            Ok(next.run(req).await)
        }
        Err(e) => {
            tracing::warn!(error = %e, "refused a request without a valid Cloudflare Access assertion");
            Err(refused(&e.to_string()))
        }
    }
}
