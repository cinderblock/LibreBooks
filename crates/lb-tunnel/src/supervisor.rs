//! Running cloudflared and keeping it running.
//!
//! [`Tunnel::start`] spawns `cloudflared tunnel --no-autoupdate run` with the
//! token in `TUNNEL_TOKEN` (see [`crate::proc`] for why, and for how the child is
//! tied to this process's life), reads its log, and publishes a [`TunnelStatus`]
//! the UI can show: the state, the last log lines, the version. When cloudflared
//! exits it is started again after a [`Backoff`] delay, until [`Tunnel::stop`].
//!
//! `--no-autoupdate` because cloudflared's own updater would replace a binary that
//! LibreBooks verified (or that a package manager owns) behind everyone's back;
//! LibreBooks updates its managed copy itself, at start.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncBufReadExt as _, AsyncRead, BufReader};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::backoff::Backoff;
use crate::config::TokenInfo;
use crate::logparse::LogParser;
use crate::proc;

/// What the tunnel is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TunnelState {
    Stopped,
    /// cloudflared is starting and has no connection yet.
    Starting,
    /// Serving: `connections` links to Cloudflare are up (four when healthy).
    Connected {
        connections: u32,
    },
    /// It was connected and lost every connection; cloudflared (or the
    /// supervisor) is retrying.
    Reconnecting,
    /// It cannot work until something changes; `reason` says what, in words.
    Failed {
        reason: String,
    },
}

/// Everything the UI shows about the tunnel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TunnelStatus {
    pub state: TunnelState,
    /// cloudflared's most recent output, token removed.
    pub last_log_lines: Vec<String>,
    pub version: Option<String>,
    /// How many times cloudflared has been restarted.
    pub restarts: u32,
    /// The tunnel's id, from the token.
    pub tunnel_id: Option<String>,
    pub public_url: Option<String>,
}

/// How to run cloudflared.
#[derive(Clone)]
pub struct SupervisorOptions {
    pub binary: PathBuf,
    pub token: String,
    /// More environment for the child (e.g. `TUNNEL_METRICS`).
    pub env: Vec<(String, String)>,
    pub backoff: Backoff,
    /// How long a stop waits for a graceful exit before ending the process.
    pub stop_grace: Duration,
    /// How many log lines to keep.
    pub keep_lines: usize,
    /// The version, if already known (from `cloudflared --version`).
    pub version: Option<String>,
    pub public_url: Option<String>,
}

impl std::fmt::Debug for SupervisorOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SupervisorOptions")
            .field("binary", &self.binary)
            .field("backoff", &self.backoff)
            .finish_non_exhaustive()
    }
}

impl SupervisorOptions {
    pub fn new(binary: impl Into<PathBuf>, token: impl Into<String>) -> Self {
        SupervisorOptions {
            binary: binary.into(),
            token: token.into(),
            env: Vec::new(),
            backoff: Backoff::default(),
            stop_grace: Duration::from_secs(10),
            keep_lines: 100,
            version: None,
            public_url: None,
        }
    }
}

/// The arguments cloudflared is run with. The token is not among them.
pub const ARGS: [&str; 3] = ["tunnel", "--no-autoupdate", "run"];

/// A supervised cloudflared. Dropping it ends the process at once; [`Tunnel::stop`]
/// lets it close its connections first.
#[derive(Debug)]
pub struct Tunnel {
    status: watch::Receiver<TunnelStatus>,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl Tunnel {
    /// Start supervising. Must be called inside a tokio runtime.
    pub fn start(options: SupervisorOptions) -> Tunnel {
        let tunnel_id = TokenInfo::parse(&options.token).ok().map(|t| t.tunnel_id);
        let initial = TunnelStatus {
            state: TunnelState::Starting,
            last_log_lines: Vec::new(),
            version: options.version.clone(),
            restarts: 0,
            tunnel_id,
            public_url: options.public_url.clone(),
        };
        let (status_tx, status) = watch::channel(initial);
        let (stop, stop_rx) = watch::channel(false);
        let task = tokio::spawn(supervise(options, status_tx, stop_rx));
        Tunnel {
            status,
            stop,
            task: Some(task),
        }
    }

    pub fn status(&self) -> TunnelStatus {
        self.status.borrow().clone()
    }

    /// Changes as they happen (for server-sent events).
    pub fn subscribe(&self) -> watch::Receiver<TunnelStatus> {
        self.status.clone()
    }

    /// Ask cloudflared to close its connections and exit, end it if it takes
    /// longer than the grace period, and stop supervising. Returns the final status.
    pub async fn stop(mut self) -> TunnelStatus {
        let _ = self.stop.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
        self.status.borrow().clone()
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        // Aborting drops the child, which KillOnDrop (and the job object) end.
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn supervise(
    options: SupervisorOptions,
    status: watch::Sender<TunnelStatus>,
    mut stop: watch::Receiver<bool>,
) {
    let mut attempt = 0u32;
    let mut lines: VecDeque<String> = VecDeque::new();
    let mut env = options.env.clone();
    env.push(("TUNNEL_TOKEN".into(), options.token.clone()));

    loop {
        if *stop.borrow() {
            break;
        }
        let run = run_once(&options, &env, &status, &mut stop, &mut lines).await;
        let (healthy_for, stopping, end_state) = match run {
            Run::Stopped => (Duration::ZERO, true, TunnelState::Stopped),
            Run::Exited { healthy_for, state } => (healthy_for, false, state),
        };
        if stopping {
            status.send_modify(|s| s.state = TunnelState::Stopped);
            break;
        }
        let wait = options.backoff.after_run(&mut attempt, healthy_for);
        tracing::warn!(?wait, state = ?end_state, "cloudflared stopped; restarting");
        status.send_modify(|s| {
            s.state = end_state;
            s.restarts += 1;
        });
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            _ = stop.wait_for(|stop| *stop) => {
                status.send_modify(|s| s.state = TunnelState::Stopped);
                break;
            }
        }
    }
}

enum Run {
    Stopped,
    Exited {
        /// How long it was connected (zero if never).
        healthy_for: Duration,
        /// What to show until the next start.
        state: TunnelState,
    },
}

async fn run_once(
    options: &SupervisorOptions,
    env: &[(String, String)],
    status: &watch::Sender<TunnelStatus>,
    stop: &mut watch::Receiver<bool>,
    lines: &mut VecDeque<String>,
) -> Run {
    let mut child = match proc::spawn(&options.binary, &ARGS, env) {
        Ok(c) => c,
        Err(e) => {
            return Run::Exited {
                healthy_for: Duration::ZERO,
                state: TunnelState::Failed {
                    reason: format!(
                        "could not start cloudflared ({}): {e}",
                        options.binary.display()
                    ),
                },
            };
        }
    };
    // A restart after a lost connection keeps saying "reconnecting".
    status.send_modify(|s| {
        if !matches!(s.state, TunnelState::Reconnecting) {
            s.state = TunnelState::Starting;
        }
    });

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    if let Some(out) = child.stdout().take() {
        tokio::spawn(forward_lines(out, tx.clone()));
    }
    if let Some(err) = child.stderr().take() {
        tokio::spawn(forward_lines(err, tx.clone()));
    }
    drop(tx);

    let mut parser = LogParser::new();
    let mut connected_since: Option<Instant> = None;
    let mut healthy_for = Duration::ZERO;
    let mut stopping = false;
    loop {
        tokio::select! {
            line = rx.recv() => {
                let Some(line) = line else { break };
                let line = if options.token.is_empty() {
                    line
                } else {
                    line.replace(&options.token, "[token]")
                };
                parser.feed(&line);
                lines.push_back(line);
                while lines.len() > options.keep_lines {
                    lines.pop_front();
                }
                let n = parser.connections();
                match (n > 0, connected_since) {
                    (true, None) => connected_since = Some(Instant::now()),
                    (false, Some(t)) => {
                        healthy_for += t.elapsed();
                        connected_since = None;
                    }
                    _ => {}
                }
                status.send_modify(|s| {
                    s.last_log_lines = lines.iter().cloned().collect();
                    if s.version.is_none() {
                        s.version = parser.version().map(str::to_owned);
                    }
                    if n > 0 {
                        s.state = TunnelState::Connected { connections: n };
                    } else if let Some(reason) = parser.fatal() {
                        s.state = TunnelState::Failed { reason: reason.to_owned() };
                    } else if parser.ever_connected() {
                        s.state = TunnelState::Reconnecting;
                    }
                });
            }
            _ = stop.wait_for(|stop| *stop), if !stopping => {
                stopping = true;
                proc::request_stop(child.as_mut());
                break;
            }
        }
    }
    if let Some(t) = connected_since {
        healthy_for += t.elapsed();
    }

    let exit = if stopping {
        if let Ok(r) = tokio::time::timeout(options.stop_grace, child.wait()).await {
            r
        } else {
            let _ = child.start_kill();
            child.wait().await
        }
    } else {
        child.wait().await
    };
    if stopping {
        return Run::Stopped;
    }
    let exit = match exit {
        Ok(s) => format!("cloudflared exited ({s})"),
        Err(e) => format!("cloudflared stopped: {e}"),
    };
    let state = match parser.fatal() {
        Some(reason) => TunnelState::Failed {
            reason: reason.to_owned(),
        },
        None if parser.ever_connected() => TunnelState::Reconnecting,
        None => TunnelState::Failed {
            reason: parser
                .last_error()
                .map_or_else(|| exit.clone(), |e| format!("{exit}: {e}")),
        },
    };
    Run::Exited { healthy_for, state }
}

/// Send each line of `stream` to `tx` (invalid UTF-8 replaced, not fatal).
async fn forward_lines(stream: impl AsyncRead + Unpin, tx: mpsc::UnboundedSender<String>) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf).trim_end().to_owned();
                if !line.is_empty() && tx.send(line).is_err() {
                    break;
                }
            }
        }
    }
}
