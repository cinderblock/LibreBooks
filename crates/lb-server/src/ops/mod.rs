//! Running the server itself: its settings, its secrets, and the background work
//! a headless server does on its own (backups, self-update, the Cloudflare
//! tunnel, bank-feed downloads).
//!
//! [`Ops`] lives in the shared state. Creating it ([`Ops::new`]) only reads files;
//! nothing runs until [`start`] is called inside a tokio runtime, which returns the
//! [`Background`] handle that `librebooks serve` shuts down at exit. Tests build
//! the router without starting anything, then drive the same functions the
//! background tasks call.
//!
//! Decisions and the reasons for them: `plans/server-ops.md`.

pub mod access;
pub mod backups;
pub mod feeds;
pub mod settings;
pub mod tunnel;
pub mod update;

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use lb_update::{Restart, UpdaterOptions};
use serde_json::json;
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;

pub use settings::{FeedSyncSettings, ServerSettings};

use crate::error::{ApiError, ApiResult};
use crate::secrets::SecretStore;
use crate::state::AppState;

/// How to set up [`Ops`], beyond what `server.json` says.
#[derive(Debug, Default)]
pub struct OpsOptions {
    /// Where the secret store's key lives (default `<data dir>/secret.key`).
    pub secret_key: Option<PathBuf>,
    pub updater: UpdaterChoice,
    pub cloudflared: CloudflaredOptions,
    /// Fetch Cloudflare Access's signing keys from here instead of the team's
    /// own URL (tests).
    pub access_certs_url: Option<String>,
}

/// Which updater the server runs.
#[derive(Debug, Default)]
pub enum UpdaterChoice {
    /// LibreBooks' own signed releases (needs a release key compiled in).
    #[default]
    Releases,
    /// Exactly these options (tests, or a fork's own releases).
    Custom(Box<UpdaterOptions>),
    /// None, for this reason (the desktop app updates through Tauri).
    Off(String),
}

/// Where cloudflared may come from.
#[derive(Clone, Debug)]
pub struct CloudflaredOptions {
    /// Use a cloudflared installed on the system (`PATH`, Cloudflare's MSI).
    pub search_system: bool,
    /// The GitHub API to fetch releases from (tests); `None` for GitHub.
    pub api_base: Option<String>,
}

impl Default for CloudflaredOptions {
    fn default() -> Self {
        CloudflaredOptions {
            search_system: true,
            api_base: None,
        }
    }
}

/// The server's settings, secrets and background work.
pub struct Ops {
    data_dir: PathBuf,
    settings_path: PathBuf,
    settings: RwLock<ServerSettings>,
    /// One settings change at a time (save, then apply).
    change: tokio::sync::Mutex<()>,
    pub secrets: SecretStore,
    pub(crate) update: update::UpdateSlot,
    pub(crate) backups: backups::BackupSlots,
    pub(crate) tunnel: tunnel::TunnelSlot,
    pub(crate) access: access::AccessSlot,
    pub(crate) feeds: feeds::FeedSlots,
    cloudflared: CloudflaredOptions,
    access_certs_url: Option<String>,
    restart: watch::Sender<Option<Restart>>,
    /// Wakes the backup reconcile loop (settings changed, a company appeared).
    wake_backups: Notify,
    running: AtomicBool,
}

impl std::fmt::Debug for Ops {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ops")
            .field("data_dir", &self.data_dir)
            .field("running", &self.running.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Ops {
    /// Read (or create) `server.json` and open the secret store. Starts nothing.
    pub fn new(data_dir: &Path, options: OpsOptions) -> std::io::Result<Ops> {
        let (settings_path, settings) = settings::load_or_create(data_dir)?;
        let secrets = SecretStore::open(data_dir, options.secret_key);
        let update = update::UpdateSlot::new(options.updater, data_dir, &settings.updates);
        let access = access::AccessSlot::default();
        access.set(
            settings.access.as_ref(),
            options.access_certs_url.as_deref(),
        );
        let (restart, _) = watch::channel(None);
        Ok(Ops {
            data_dir: data_dir.to_owned(),
            settings_path,
            settings: RwLock::new(settings),
            change: tokio::sync::Mutex::new(()),
            secrets,
            update,
            backups: backups::BackupSlots::default(),
            tunnel: tunnel::TunnelSlot::default(),
            access,
            feeds: feeds::FeedSlots::default(),
            cloudflared: options.cloudflared,
            access_certs_url: options.access_certs_url,
            restart,
            wake_backups: Notify::new(),
            running: AtomicBool::new(false),
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The settings now.
    pub fn settings(&self) -> ServerSettings {
        self.settings
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Are the background tasks running (`librebooks serve`), or is this a CLI
    /// command or a test?
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// The header holding the real client address, when a proxy on this machine
    /// (cloudflared) forwards requests: `cf-connecting-ip` while the tunnel is on.
    pub fn forwarded_ip_header(&self) -> Option<&'static str> {
        self.settings
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tunnel
            .enabled
            .then_some("cf-connecting-ip")
    }

    /// Ask the process to restart (after an update was installed). `serve`
    /// watches [`Ops::restart_requested`], shuts down gracefully, then starts the
    /// new binary or exits for its supervisor.
    pub fn request_restart(&self, restart: Restart) {
        tracing::info!(?restart, "restart requested");
        self.restart.send_replace(Some(restart));
    }

    pub fn restart_requested(&self) -> watch::Receiver<Option<Restart>> {
        self.restart.subscribe()
    }

    /// Recheck which companies should be backed up, soon.
    pub fn wake_backups(&self) {
        self.wake_backups.notify_one();
    }
}

/// Replace the settings: check them, save them, then apply them to whatever is
/// running (update policy, schedulers, the tunnel, the Access check).
pub async fn put_settings(state: &AppState, new: ServerSettings) -> ApiResult<ServerSettings> {
    let ops = &state.ops;
    let _one_at_a_time = ops.change.lock().await;
    let problems = new.problems(&ops.secrets);
    if !problems.is_empty() {
        return Err(
            ApiError::invalid(problems.join("; ")).with_detail(json!({ "problems": problems }))
        );
    }
    let path = ops.settings_path.clone();
    let to_save = new.clone();
    tokio::task::spawn_blocking(move || settings::save(&path, &to_save))
        .await?
        .map_err(|e| ApiError::internal(format!("could not save the settings: {e}")))?;
    let old = std::mem::replace(
        &mut *ops
            .settings
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        new.clone(),
    );
    if old.updates != new.updates {
        ops.update.set_config(&new.updates);
    }
    if old.access != new.access {
        ops.access
            .set(new.access.as_ref(), ops.access_certs_url.as_deref());
    }
    if old.backups != new.backups {
        // The reconcile loop starts, stops and updates the schedulers.
        ops.wake_backups();
    }
    if old.tunnel != new.tunnel {
        tunnel::apply(state).await;
    }
    Ok(new)
}

/// Change part of the settings with `f`, then [`put_settings`].
pub async fn edit_settings(
    state: &AppState,
    f: impl FnOnce(&mut ServerSettings),
) -> ApiResult<ServerSettings> {
    let mut s = state.ops.settings();
    f(&mut s);
    put_settings(state, s).await
}

/// What the server needs a restart to pick up, in words (passkey domains are
/// fixed when sign-in starts).
pub fn restart_required(state: &AppState) -> Vec<String> {
    let Some(sessions) = &state.sessions else {
        return Vec::new();
    };
    let have: Vec<String> = sessions
        .store
        .config()
        .relying_parties
        .iter()
        .map(|rp| rp.id.to_ascii_lowercase())
        .collect();
    state
        .ops
        .settings()
        .passkey_hosts()
        .into_iter()
        .filter(|h| !have.contains(h))
        .map(|h| format!("restart LibreBooks so passkeys work at {h}"))
        .collect()
}

/// The background tasks, from [`start`].
#[derive(Debug)]
pub struct Background {
    state: AppState,
    stop: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

/// Start the background work: backup schedulers, the update timer, the tunnel
/// (when on) and bank-feed downloads. Must be called inside a tokio runtime.
pub fn start(state: &AppState) -> Background {
    state.ops.running.store(true, Ordering::SeqCst);
    let (stop, _) = watch::channel(false);
    let tasks = vec![
        tokio::spawn(backups::run(state.clone(), stop.subscribe())),
        tokio::spawn(update::run(state.clone(), stop.subscribe())),
        tokio::spawn(feeds::run(state.clone(), stop.subscribe())),
        tokio::spawn({
            let state = state.clone();
            async move { tunnel::apply(&state).await }
        }),
    ];
    Background {
        state: state.clone(),
        stop,
        tasks,
    }
}

impl Background {
    /// Stop everything: the loops at their next pause (work in progress gets 30
    /// seconds to finish), then each scheduler after any backup in progress, then
    /// the tunnel (cloudflared closes its connections first).
    pub async fn shutdown(self) {
        let ops = &self.state.ops;
        ops.running.store(false, Ordering::SeqCst);
        self.stop.send_replace(true);
        for mut task in self.tasks {
            if tokio::time::timeout(Duration::from_secs(30), &mut task)
                .await
                .is_err()
            {
                task.abort();
            }
        }
        backups::shutdown(&self.state).await;
        tunnel::stop(&self.state).await;
        tracing::info!("background work stopped");
    }
}

/// Sleep for `d`, or until `stop` says so. `true` when it is time to stop.
pub(crate) async fn nap(d: Duration, stop: &mut watch::Receiver<bool>) -> bool {
    if *stop.borrow() {
        return true;
    }
    tokio::select! {
        () = tokio::time::sleep(d) => *stop.borrow(),
        r = stop.changed() => r.is_err() || *stop.borrow(),
    }
}
