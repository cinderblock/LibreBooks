//! The Cloudflare tunnel: started when the settings turn it on, restarted when
//! they change, stopped at shutdown.
//!
//! Starting has two parts: making sure a cloudflared is there (found, or
//! downloaded and verified, which can take a while), then supervising it
//! ([`lb_tunnel::Tunnel`]). The first runs as its own task so a settings change or
//! a shutdown never waits on a download; a generation counter makes sure a
//! preparation that finishes after its settings were replaced is discarded.

use std::sync::atomic::{AtomicU64, Ordering};

use jiff::Timestamp;
use lb_tunnel::{
    Downloader, Ready, Sources, SupervisorOptions, TokenInfo, TokenSource, Tunnel, TunnelConfig,
    TunnelState, TunnelStatus,
};
use serde::Serialize;
use tokio::task::JoinHandle;

use crate::state::AppState;

/// The secret the tunnel token is saved as.
pub const TOKEN_SECRET: &str = "tunnel-token";

enum Slot {
    Off,
    Preparing {
        since: Timestamp,
        task: JoinHandle<()>,
    },
    Running {
        tunnel: Tunnel,
        ready: Ready,
    },
    Failed {
        reason: String,
    },
}

pub(crate) struct TunnelSlot {
    slot: tokio::sync::Mutex<Slot>,
    generation: AtomicU64,
}

impl Default for TunnelSlot {
    fn default() -> Self {
        TunnelSlot {
            slot: tokio::sync::Mutex::new(Slot::Off),
            generation: AtomicU64::new(0),
        }
    }
}

impl std::fmt::Debug for TunnelSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelSlot").finish_non_exhaustive()
    }
}

async fn stop_slot(slot: &mut Slot) {
    match std::mem::replace(slot, Slot::Off) {
        Slot::Running { tunnel, .. } => {
            tunnel.stop().await;
        }
        Slot::Preparing { task, .. } => task.abort(),
        Slot::Off | Slot::Failed { .. } => {}
    }
}

/// Start, restart or stop the tunnel to match the settings. Does nothing (beyond
/// stopping) unless the background work is running.
pub async fn apply(state: &AppState) {
    let ops = &state.ops;
    let generation = ops.tunnel.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let mut slot = ops.tunnel.slot.lock().await;
    stop_slot(&mut slot).await;
    let config = ops.settings().tunnel;
    if !config.enabled || !ops.is_running() {
        return;
    }
    let problems = config.problems();
    if !problems.is_empty() {
        *slot = Slot::Failed {
            reason: problems.join("; "),
        };
        return;
    }
    let token = match TokenSource::resolve(&ops.secrets, &config.token_ref) {
        Ok(t) => t,
        Err(e) => {
            *slot = Slot::Failed {
                reason: e.to_string(),
            };
            return;
        }
    };
    let task = tokio::spawn(prepare(state.clone(), config, token, generation));
    *slot = Slot::Preparing {
        since: Timestamp::now(),
        task,
    };
}

async fn prepare(state: AppState, config: TunnelConfig, token: String, generation: u64) {
    let ops = &state.ops;
    let downloader = match &ops.cloudflared.api_base {
        Some(base) => Downloader::with_api(base),
        None => Downloader::new(),
    };
    let sources = Sources {
        configured: config.cloudflared_path.as_deref(),
        data_dir: ops.data_dir(),
        search_system: ops.cloudflared.search_system,
    };
    let ready = match downloader {
        Ok(d) => lb_tunnel::cloudflared::ensure(&sources, config.auto_download, &d).await,
        Err(e) => Err(e),
    };
    let mut slot = ops.tunnel.slot.lock().await;
    if ops.tunnel.generation.load(Ordering::SeqCst) != generation {
        return;
    }
    *slot = match ready {
        Ok(ready) => {
            tracing::info!(
                cloudflared = %ready.located.path.display(),
                version = %ready.version,
                "starting the Cloudflare tunnel"
            );
            let mut options = SupervisorOptions::new(&ready.located.path, token);
            options.version = Some(ready.version.clone());
            options.public_url = config.public_url();
            Slot::Running {
                tunnel: Tunnel::start(options),
                ready,
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "the Cloudflare tunnel cannot start");
            Slot::Failed {
                reason: e.to_string(),
            }
        }
    };
}

/// Stop the tunnel (cloudflared closes its connections first).
pub async fn stop(state: &AppState) {
    state.ops.tunnel.generation.fetch_add(1, Ordering::SeqCst);
    let mut slot = state.ops.tunnel.slot.lock().await;
    stop_slot(&mut slot).await;
}

/// What `GET /server/tunnel` shows. Never the token.
#[derive(Clone, Debug, Serialize)]
pub struct TunnelView {
    pub enabled: bool,
    pub public_hostname: Option<String>,
    pub public_url: Option<String>,
    /// Where the token is (`secret:tunnel-token`), not the token.
    pub token_ref: String,
    /// The token can be read.
    pub token_set: bool,
    /// The tunnel's id, read from the token.
    pub tunnel_id: Option<String>,
    pub cloudflared_path: Option<std::path::PathBuf>,
    pub auto_download: bool,
    /// The cloudflared in use, once one is found.
    pub cloudflared: Option<Ready>,
    /// Preparing: since when (finding or downloading cloudflared).
    pub preparing_since: Option<Timestamp>,
    pub status: TunnelStatus,
    pub problems: Vec<String>,
    pub restart_required: Vec<String>,
}

pub async fn view(state: &AppState) -> TunnelView {
    let ops = &state.ops;
    let config = ops.settings().tunnel;
    let token = if config.token_ref.trim().is_empty() {
        None
    } else {
        TokenSource::resolve(&ops.secrets, &config.token_ref).ok()
    };
    let tunnel_id = token
        .as_deref()
        .and_then(|t| TokenInfo::parse(t).ok())
        .map(|i| i.tunnel_id);
    let idle = |state: TunnelState| TunnelStatus {
        state,
        last_log_lines: Vec::new(),
        version: None,
        restarts: 0,
        tunnel_id: tunnel_id.clone(),
        public_url: config.public_url(),
    };
    let (status, cloudflared, preparing_since) = {
        let slot = ops.tunnel.slot.lock().await;
        match &*slot {
            Slot::Off => (idle(TunnelState::Stopped), None, None),
            Slot::Preparing { since, .. } => (idle(TunnelState::Starting), None, Some(*since)),
            Slot::Running { tunnel, ready } => (tunnel.status(), Some(ready.clone()), None),
            Slot::Failed { reason } => (
                idle(TunnelState::Failed {
                    reason: reason.clone(),
                }),
                None,
                None,
            ),
        }
    };
    let mut problems = config.problems();
    if config.enabled && !ops.is_running() {
        problems.push("the tunnel runs only while `librebooks serve` does".into());
    }
    TunnelView {
        enabled: config.enabled,
        public_url: config.public_url(),
        public_hostname: config.public_hostname.clone(),
        token_set: token.is_some(),
        token_ref: config.token_ref.clone(),
        tunnel_id,
        cloudflared_path: config.cloudflared_path.clone(),
        auto_download: config.auto_download,
        cloudflared,
        preparing_since,
        status,
        problems,
        restart_required: super::restart_required(state),
    }
}
