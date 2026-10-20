//! Server administration: settings, secrets, self-update, the tunnel, and the
//! backup schedulers' status. Everything here needs a server admin.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use lb_tunnel::TokenInfo;
use serde::{Deserialize, Serialize};

use crate::auth::{Principal, Role};
use crate::error::{ApiError, ApiResult};
use crate::ops::backups::{self, CompanyBackupStatus};
use crate::ops::tunnel::{self, TOKEN_SECRET, TunnelView};
use crate::ops::update::{self, UpdateView};
use crate::ops::{self, ServerSettings};
use crate::secrets::{SCHEME, SecretError, SecretInfo};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/server", get(get_server).put(put_server))
        .route("/server/secrets", get(list_secrets))
        .route(
            "/server/secrets/{name}",
            put(put_secret).delete(delete_secret),
        )
        .route("/server/update", get(get_update))
        .route("/server/update/check", post(check_update))
        .route("/server/update/install", post(install_update))
        .route("/server/tunnel", get(get_tunnel).put(put_tunnel))
        .route("/server/backups/status", get(backup_status))
}

impl From<SecretError> for ApiError {
    fn from(e: SecretError) -> Self {
        match e {
            SecretError::BadName(_) | SecretError::Reference(_) => ApiError::invalid(e.to_string()),
            SecretError::Missing(_) => {
                ApiError::new(StatusCode::NOT_FOUND, "not_found", e.to_string())
            }
            _ => ApiError::internal(e.to_string()),
        }
    }
}

/// Everything about the server, for its settings page.
#[derive(Serialize)]
pub struct ServerView {
    pub version: &'static str,
    pub data_dir: String,
    pub settings: ServerSettings,
    /// Secret names (never values).
    pub secrets: Vec<SecretInfo>,
    pub update: UpdateView,
    pub tunnel: TunnelView,
    pub backups: Vec<CompanyBackupStatus>,
    /// Settings that take effect only after a restart.
    pub restart_required: Vec<String>,
    /// The background work is running (false for a CLI or test instance).
    pub running: bool,
}

async fn server_view(s: &AppState) -> ApiResult<ServerView> {
    let secrets = {
        let store = s.ops.secrets.clone();
        tokio::task::spawn_blocking(move || store.list()).await??
    };
    Ok(ServerView {
        version: s.version,
        data_dir: s.ops.data_dir().display().to_string(),
        settings: s.ops.settings(),
        secrets,
        update: update::view(s),
        tunnel: tunnel::view(s).await,
        backups: backups::statuses(s),
        restart_required: ops::restart_required(s),
        running: s.ops.is_running(),
    })
}

async fn get_server(State(s): State<AppState>, p: Principal) -> ApiResult<Json<ServerView>> {
    p.require(Role::Admin)?;
    Ok(Json(server_view(&s).await?))
}

async fn put_server(
    State(s): State<AppState>,
    p: Principal,
    Json(settings): Json<ServerSettings>,
) -> ApiResult<Json<ServerView>> {
    p.require(Role::Admin)?;
    ops::put_settings(&s, settings).await?;
    Ok(Json(server_view(&s).await?))
}

async fn list_secrets(State(s): State<AppState>, p: Principal) -> ApiResult<Json<Vec<SecretInfo>>> {
    p.require(Role::Admin)?;
    let store = s.ops.secrets.clone();
    Ok(Json(
        tokio::task::spawn_blocking(move || store.list()).await??,
    ))
}

#[derive(Deserialize)]
struct SecretBody {
    value: String,
}

impl std::fmt::Debug for SecretBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretBody([redacted])")
    }
}

/// `PUT /server/secrets/:name` `{ value }` → 204. Write-only: nothing ever
/// returns the value. Refer to it in settings as `secret:<name>`.
async fn put_secret(
    State(s): State<AppState>,
    p: Principal,
    Path(name): Path<String>,
    Json(body): Json<SecretBody>,
) -> ApiResult<StatusCode> {
    p.require(Role::Admin)?;
    if body.value.is_empty() {
        return Err(ApiError::invalid(
            "a secret can't be empty; delete it instead",
        ));
    }
    let store = s.ops.secrets.clone();
    tokio::task::spawn_blocking(move || store.set(&name, &body.value)).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_secret(
    State(s): State<AppState>,
    p: Principal,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    p.require(Role::Admin)?;
    let reference = format!("{SCHEME}{name}");
    let settings = s.ops.settings();
    let used = serde_json::to_string(&settings)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .contains(&format!("\"{reference}\""));
    if used {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "in_use",
            format!("the server settings still use {reference}"),
        ));
    }
    let store = s.ops.secrets.clone();
    if tokio::task::spawn_blocking(move || store.delete(&name)).await?? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("secret"))
    }
}

async fn get_update(State(s): State<AppState>, p: Principal) -> ApiResult<Json<UpdateView>> {
    p.require(Role::Admin)?;
    Ok(Json(update::view(&s)))
}

async fn check_update(State(s): State<AppState>, p: Principal) -> ApiResult<Json<UpdateView>> {
    p.require(Role::Admin)?;
    Ok(Json(update::check(&s).await?))
}

#[derive(Deserialize)]
#[serde(default)]
struct InstallBody {
    /// Restart into the new version straight away (default). `false` installs
    /// it for the next restart.
    restart: bool,
}

impl Default for InstallBody {
    fn default() -> Self {
        InstallBody { restart: true }
    }
}

async fn install_update(
    State(s): State<AppState>,
    p: Principal,
    body: Option<Json<InstallBody>>,
) -> ApiResult<Json<serde_json::Value>> {
    p.require(Role::Admin)?;
    let restart = body.is_none_or(|b| b.restart);
    let installed = update::install(&s, restart).await?;
    Ok(Json(serde_json::json!({
        "installed": installed,
        "restarting": restart,
    })))
}

async fn get_tunnel(State(s): State<AppState>, p: Principal) -> ApiResult<Json<TunnelView>> {
    p.require(Role::Admin)?;
    Ok(Json(tunnel::view(&s).await))
}

/// `PUT /server/tunnel`: any of these; what is left out stays as it is, and an
/// empty string clears a text field.
#[derive(Default, Deserialize)]
#[serde(default)]
struct TunnelBody {
    enabled: Option<bool>,
    /// `books.example.com`.
    public_hostname: Option<String>,
    /// The token from the Cloudflare dashboard (or the whole install command).
    /// Saved in the secret store, never in the settings.
    token: Option<String>,
    /// A cloudflared to run instead of finding or downloading one.
    cloudflared_path: Option<String>,
    auto_download: Option<bool>,
}

impl std::fmt::Debug for TunnelBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelBody")
            .field("enabled", &self.enabled)
            .field("public_hostname", &self.public_hostname)
            .field("token", &self.token.as_ref().map(|_| "[redacted]"))
            .finish_non_exhaustive()
    }
}

/// `Some("")` → `None`, trimmed otherwise.
fn non_empty(s: &str) -> Option<String> {
    Some(s.trim().to_owned()).filter(|s| !s.is_empty())
}

async fn put_tunnel(
    State(s): State<AppState>,
    p: Principal,
    Json(body): Json<TunnelBody>,
) -> ApiResult<Json<TunnelView>> {
    p.require(Role::Admin)?;
    let mut token_ref = None;
    if let Some(token) = &body.token {
        let token = token.trim();
        // People paste the whole `cloudflared service install <token>` command.
        let token = token.rsplit(' ').next().unwrap_or(token).to_owned();
        TokenInfo::parse(&token).map_err(|e| ApiError::invalid(e.to_string()))?;
        let store = s.ops.secrets.clone();
        tokio::task::spawn_blocking(move || store.set(TOKEN_SECRET, &token)).await??;
        token_ref = Some(format!("{SCHEME}{TOKEN_SECRET}"));
    }
    ops::edit_settings(&s, |settings| {
        let t = &mut settings.tunnel;
        if let Some(e) = body.enabled {
            t.enabled = e;
        }
        if let Some(h) = &body.public_hostname {
            t.public_hostname = non_empty(h);
        }
        if let Some(r) = token_ref {
            t.token_ref = r;
        }
        if let Some(path) = &body.cloudflared_path {
            t.cloudflared_path = non_empty(path).map(std::path::PathBuf::from);
        }
        if let Some(a) = body.auto_download {
            t.auto_download = a;
        }
    })
    .await?;
    Ok(Json(tunnel::view(&s).await))
}

async fn backup_status(
    State(s): State<AppState>,
    p: Principal,
) -> ApiResult<Json<serde_json::Value>> {
    p.require(Role::Admin)?;
    let settings = s.ops.settings();
    Ok(Json(serde_json::json!({
        "enabled": settings.backups_enabled() && settings.backups.schedule.enabled,
        "running": s.ops.is_running(),
        "companies": backups::statuses(&s),
    })))
}
