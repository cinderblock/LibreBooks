//! What the main window talks to: always a server on `127.0.0.1:<port>` inside
//! this process, either
//! - **local**: lb-server itself, holding the company files, authenticated by
//!   a per-start bearer token (`AuthMode::LocalToken`); or
//! - **remote**: the thin-client proxy (`crate::proxy`) in front of a remote
//!   LibreBooks server.
//!
//! Either way the webview loads the SPA from that origin, so API calls are
//! same-origin and the SPA has a single code path.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use lb_server::{AppState, AuthMode, Mode, ServerConfig};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use url::Url;

use crate::device::{self, DeviceError};
use crate::proxy::{self, Proxy};
use crate::settings::{BackendMode, Settings, SettingsError, parse_server_url};

/// How long a graceful stop may take before connections are cut (open
/// Server-Sent Event streams never finish on their own).
const STOP_GRACE: Duration = Duration::from_secs(3);

/// Development aids, from the environment.
#[derive(Clone, Debug, Default)]
pub struct DevOverrides {
    /// `LIBREBOOKS_DESKTOP_PORT`: a fixed loopback port instead of a random one
    /// (lets the Vite dev server proxy `/api` to it).
    pub port: Option<u16>,
    /// `LIBREBOOKS_WEB_DEV_URL` (debug builds only): load the SPA from the Vite
    /// dev server, e.g. `http://localhost:5740`, for hot reload.
    pub web_dev_url: Option<Url>,
}

impl DevOverrides {
    pub fn from_env() -> DevOverrides {
        let port = std::env::var("LIBREBOOKS_DESKTOP_PORT")
            .ok()
            .and_then(|p| p.parse().ok());
        let web_dev_url = if cfg!(debug_assertions) {
            std::env::var("LIBREBOOKS_WEB_DEV_URL")
                .ok()
                .and_then(|u| Url::parse(&u).ok())
        } else {
            None
        };
        DevOverrides { port, web_dev_url }
    }

    /// The dev server's `Host` header value, if any.
    fn web_dev_host(&self) -> Option<String> {
        let u = self.web_dev_url.as_ref()?;
        Some(format!("{}:{}", u.host_str()?, u.port_or_known_default()?))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("couldn't open a loopback port: {0}")]
    Bind(std::io::Error),
    #[error("couldn't open the company folder {path}: {source}")]
    DataDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Refused(#[from] lb_server::StartError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
}

/// Which backend is running.
#[derive(Clone, Debug)]
pub enum BackendKind {
    Local { data_dir: PathBuf },
    Remote { server: Url },
}

/// A running backend.
#[derive(Debug)]
pub struct Backend {
    pub kind: BackendKind,
    /// `http://127.0.0.1:<port>`: the only origin given the token.
    pub origin: String,
    /// Per-start secret the webview presents (`Authorization: Bearer`).
    pub token: Arc<str>,
    /// Where the books are also served to the network, if they are.
    pub network: Option<SocketAddr>,
    /// Why serving to the network was asked for but isn't happening.
    pub network_error: Option<String>,
    /// Why a thin client can't sign in (e.g. not enrolled), if it can't.
    pub remote_problem: Option<String>,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    /// The in-process server's state (local mode), held to share with the
    /// network listener and to close company files deliberately on stop.
    state: Option<AppState>,
}

/// A fresh 256-bit secret, hex.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    hex::encode(bytes)
}

fn spawn_serve(
    listener: TcpListener,
    app: Router,
    mut shutdown: watch::Receiver<bool>,
    what: &'static str,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let stop = async move {
            let _ = shutdown.wait_for(|stop| *stop).await;
        };
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(stop)
            .await
        {
            log::error!("{what} stopped with an error: {e}");
        }
    })
}

impl Backend {
    /// Start the backend the settings ask for.
    pub async fn start(
        settings: &Settings,
        client: &reqwest::Client,
        dev: &DevOverrides,
    ) -> Result<Backend, BackendError> {
        let token: Arc<str> = Arc::from(new_token());
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, dev.port.unwrap_or(0)))
            .await
            .map_err(BackendError::Bind)?;
        let addr = listener.local_addr().map_err(BackendError::Bind)?;
        let origin = format!("http://{addr}");
        let (shutdown, rx) = watch::channel(false);
        let mut tasks = Vec::new();
        let mut network = None;
        let mut network_error = None;
        let mut remote_problem = None;
        let mut state = None;

        let kind = match settings.mode {
            BackendMode::Local => {
                let data_dir = settings.data_dir();
                let config = ServerConfig {
                    data_dir: data_dir.clone(),
                    mode: Mode::Desktop,
                    auth: AuthMode::LocalToken(token.to_string()),
                };
                lb_server::check_bind(&config, addr)?;
                let s = lb_server::state(&config).map_err(|source| BackendError::DataDir {
                    path: config.companies_dir(),
                    source,
                })?;
                tasks.push(spawn_serve(
                    listener,
                    lb_server::router(s.clone()),
                    rx.clone(),
                    "local server",
                ));
                log::info!(
                    "books in {} served to this app on {origin}",
                    config.companies_dir().display()
                );
                if !lb_server::assets::has_ui() {
                    log::warn!(
                        "this build has no web UI embedded; build web/ first (bun run build)"
                    );
                }
                if settings.network.enabled {
                    match crate::network::start(&s, data_dir.clone(), &settings.network, rx.clone())
                        .await
                    {
                        Ok(n) => {
                            network = Some(n.addr);
                            tasks.push(n.task);
                        }
                        Err(e) => {
                            log::warn!("not serving to the network: {e}");
                            network_error = Some(e.to_string());
                        }
                    }
                }
                state = Some(s);
                BackendKind::Local { data_dir }
            }
            BackendMode::Remote => {
                let server = parse_server_url(
                    settings.remote.url.as_deref().unwrap_or_default(),
                    settings.remote.allow_insecure_http,
                )?;
                let identity = match settings.device_for_remote() {
                    Some(e) => device::load_identity(e.clone()).await,
                    None => Err(DeviceError::NotEnrolled),
                };
                if let Err(e) = &identity {
                    remote_problem = Some(e.to_string());
                }
                let mut hosts = vec![addr.to_string(), format!("localhost:{}", addr.port())];
                hosts.extend(dev.web_dev_host());
                let proxy = Arc::new(Proxy::new(
                    server.clone(),
                    token.clone(),
                    hosts,
                    client.clone(),
                    identity,
                ));
                tasks.push(spawn_serve(
                    listener,
                    proxy::router(proxy),
                    rx.clone(),
                    "thin-client proxy",
                ));
                log::info!("thin client for {server} on {origin}");
                BackendKind::Remote { server }
            }
        };
        Ok(Backend {
            kind,
            origin,
            token,
            network,
            network_error,
            remote_problem,
            shutdown,
            tasks,
            state,
        })
    }

    /// Stop serving and close company files. Waits briefly for requests in
    /// flight, then cuts what's left (long-lived event streams).
    pub async fn stop(self) {
        let Backend {
            shutdown,
            tasks,
            state,
            ..
        } = self;
        let _ = shutdown.send(true);
        for mut task in tasks {
            if tokio::time::timeout(STOP_GRACE, &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
        }
        if let Some(state) = state {
            // Every router and handler has finished, so this should be the last
            // reference; dropping it closes each company file (SQLite
            // checkpoints the WAL on the last close).
            let others = Arc::strong_count(&state.inner) - 1;
            if others > 0 {
                log::warn!(
                    "{others} server task(s) still hold the books; they close when those finish"
                );
            }
            drop(state);
            log::info!("company files closed");
        }
    }

    /// One line for the tray and the settings page.
    pub fn describe(&self) -> String {
        match &self.kind {
            BackendKind::Local { .. } => match self.network {
                Some(addr) => format!("Books on this computer, shared on port {}", addr.port()),
                None => "Books on this computer".to_owned(),
            },
            BackendKind::Remote { server } => format!(
                "Connected to {}",
                server.host_str().unwrap_or(server.as_str())
            ),
        }
    }
}
