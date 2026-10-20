//! The LibreBooks HTTP server: the JSON API (`plans/api.md`), live updates over
//! Server-Sent Events, and the web UI embedded in the binary.
//!
//! The same server runs headless (`librebooks serve`) and inside the desktop app,
//! which links this crate and starts it in-process on loopback.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderValue, header};
use axum::middleware;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tower_http::compression::CompressionLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

pub mod assets;
pub mod auth;
pub mod error;
mod routes;
pub mod state;

pub use auth::{AuthMode, Principal, Role, Sessions};
pub use state::{AppState, ChangeEvent, Companies, Mode};

/// How to run a server.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Where company files (and, later, server settings and backups) live.
    pub data_dir: PathBuf,
    pub mode: Mode,
    pub auth: AuthMode,
    /// Passkey and device sign-in; `None` serves only the local token (desktop)
    /// or nobody-signs-in (`AuthMode::Open`, loopback development).
    pub sessions: Option<Sessions>,
}

impl ServerConfig {
    pub fn companies_dir(&self) -> PathBuf {
        self.data_dir.join("companies")
    }
}

/// Build the shared state.
pub fn state(config: &ServerConfig) -> std::io::Result<AppState> {
    let (events, _) = broadcast::channel(1024);
    let companies = Companies::new(config.companies_dir(), events.clone())?;
    Ok(AppState {
        inner: Arc::new(state::Inner {
            mode: config.mode,
            version: env!("CARGO_PKG_VERSION"),
            companies,
            events,
            auth: config.auth.clone(),
            sessions: config.sessions.clone(),
        }),
    })
}

/// The whole application: API, then the web UI for everything else.
pub fn router(state: AppState) -> Router {
    let api = routes::api()
        .layer(middleware::from_fn(auth::csrf_guard))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ));
    Router::new()
        .nest("/api/v1", api)
        .fallback(assets::serve)
        .with_state(state)
        .layer(RequestBodyLimitLayer::new(64 * 1024 * 1024))
        .layer(CompressionLayer::new())
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(TraceLayer::new_for_http())
}

/// Why the server would not start.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(
        "refusing to listen on {0} without authentication; bind to 127.0.0.1 or turn authentication on"
    )]
    OpenBeyondLoopback(SocketAddr),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Refuse configurations that would expose the books.
pub fn check_bind(config: &ServerConfig, addr: SocketAddr) -> Result<(), StartError> {
    if matches!(config.auth, AuthMode::Open)
        && config.sessions.is_none()
        && !addr.ip().is_loopback()
    {
        return Err(StartError::OpenBeyondLoopback(addr));
    }
    Ok(())
}

/// Serve until `shutdown` resolves.
pub async fn serve(
    config: &ServerConfig,
    listener: TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<(), StartError> {
    check_bind(config, listener.local_addr()?)?;
    let app = router(state(config)?);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}
