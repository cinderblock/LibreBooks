//! "Serve to the network": letting phones and other computers use the books
//! held by this desktop app.
//!
//! ## Design
//!
//! The in-process server has exactly one [`AppState`]: one set of open company
//! files, one live-update bus. Serving to the network adds a **second listener
//! on the same state** (e.g. `0.0.0.0:8740`) with **different authentication**:
//!
//! | Listener | Who | Auth |
//! | --- | --- | --- |
//! | `127.0.0.1:<random>` | this app's webview | `AuthMode::LocalToken` (per-launch bearer token) |
//! | `<bind>:<port>` | phones, other PCs | passkey / device-key sessions (`lb-auth`) |
//!
//! Two states would open every company file twice and split the event bus (a
//! check entered on a phone would not appear in the desktop's open register), so
//! the state must be shared and the auth chosen per listener. lb-server today
//! keeps `AuthMode` inside `AppState`; it needs one small addition, described in
//! `plans/desktop.md` ("Needed from lb-server"):
//!
//! ```ignore
//! /// The application with requests authenticated by `auth` rather than the state's own mode.
//! pub fn router_with_auth(state: AppState, auth: AuthMode) -> Router;
//! ```
//!
//! (implemented by a layer that puts the listener's `AuthMode` in the request
//! extensions, which the `Principal` extractor prefers over `state.auth`).
//!
//! Until passkey sessions land, [`available`] refuses, the settings page shows
//! why, and nothing listens beyond loopback. `lb_server::check_bind` is a second
//! line of defence: it refuses `AuthMode::Open` on a non-loopback address.

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::Router;
use lb_server::{AppState, AuthMode, Mode, ServerConfig};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::settings::NetworkSettings;

/// Why the books can't be served to the network.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error(
        "Serving to the network needs passkey sign-in, which this version of LibreBooks doesn't have yet. Your books stay on this computer for now."
    )]
    NeedsPasskeys,
    #[error("couldn't listen on {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },
    #[error(transparent)]
    Refused(#[from] lb_server::StartError),
}

/// Whether this build can serve to the network at all.
pub fn available() -> Result<(), NetworkError> {
    network_auth().map(|_| ())
}

/// How network clients authenticate. Passkey/device-key sessions from lb-auth,
/// once lb-server exposes them as an `AuthMode`. Never `Open`, never the local
/// token (which must not leave this computer).
fn network_auth() -> Result<AuthMode, NetworkError> {
    Err(NetworkError::NeedsPasskeys)
}

/// The router for the network listener: the shared state, network auth.
fn network_router(_state: &AppState, _auth: AuthMode) -> Result<Router, NetworkError> {
    // Becomes `Ok(lb_server::router_with_auth(state.clone(), auth))` once that
    // exists. Using `lb_server::router(state)` here would apply the *local*
    // auth mode to network clients, so it stays refused instead.
    Err(NetworkError::NeedsPasskeys)
}

/// A running network listener.
#[derive(Debug)]
pub struct NetworkListener {
    pub addr: SocketAddr,
    pub task: JoinHandle<()>,
}

/// Start listening on the configured network address, sharing `state`.
pub async fn start(
    state: &AppState,
    data_dir: PathBuf,
    settings: &NetworkSettings,
    mut shutdown: watch::Receiver<bool>,
) -> Result<NetworkListener, NetworkError> {
    let auth = network_auth()?;
    let addr = SocketAddr::new(settings.bind, settings.port);
    let config = ServerConfig {
        data_dir,
        mode: Mode::Desktop,
        auth: auth.clone(),
    };
    lb_server::check_bind(&config, addr)?;
    let app = network_router(state, auth)?;
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|source| NetworkError::Bind { addr, source })?;
    let addr = listener.local_addr().unwrap_or(addr);
    log::info!("serving the books to the network on {addr}");
    let task = tokio::spawn(async move {
        let stop = async move {
            let _ = shutdown.wait_for(|stop| *stop).await;
        };
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(stop)
            .await
        {
            log::error!("network listener on {addr} failed: {e}");
        }
    });
    Ok(NetworkListener { addr, task })
}
