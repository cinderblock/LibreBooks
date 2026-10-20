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
//! the state must be shared.
//!
//! lb-server on master (after the lb-auth merge) already lets one state accept
//! both: `AuthMode::LocalToken` plus `ServerConfig::sessions: Some(Sessions)`.
//! What's still missing, described in `plans/desktop.md` ("Needed from
//! lb-server"): the local token must only be accepted from **loopback peers**
//! (serve with `into_make_service_with_connect_info` and check `ConnectInfo`
//! in the `Principal` extractor), so the network listener can safely share the
//! router. Then this module only needs the desktop's own `Sessions` (an
//! `AuthStore` in the data folder, and the address people use as
//! `public_origin`).
//!
//! Until then [`available`] refuses, the settings page shows why, and nothing
//! listens beyond loopback. `lb_server::check_bind` is a second line of
//! defence: it refuses unauthenticated serving on a non-loopback address.

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

/// How network clients authenticate: passkey/device-key sessions (lb-auth).
/// Never `Open`, and never the local token (which must not leave this
/// computer).
fn network_auth() -> Result<AuthMode, NetworkError> {
    Err(NetworkError::NeedsPasskeys)
}

/// The router for the network listener: the shared state, network auth.
fn network_router(_state: &AppState, _auth: AuthMode) -> Result<Router, NetworkError> {
    // Becomes `Ok(lb_server::router(state.clone()))` once the state carries
    // `Sessions` and lb-server accepts the local token only from loopback
    // peers. Before that, sharing the router would let network clients use
    // the local token's auth mode, so it stays refused.
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
