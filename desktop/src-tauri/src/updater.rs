//! Self-update from GitHub releases with `tauri-plugin-updater`.
//!
//! - Checking is on by default (`updates.check_automatically`): shortly after
//!   start, then every `check_interval_hours`.
//! - By default a found update is offered in a dialog ("Install and restart" /
//!   "Later"); "Later" leaves it in the tray menu.
//! - With `updates.install_automatically` (opt-in) it downloads in the
//!   background and installs when the app quits (or right away from the tray's
//!   "Restart to update"), so nobody loses a half-typed bill to a restart.
//! - Releases are signed (minisign, the Tauri updater key); the plugin refuses
//!   anything whose signature doesn't verify against the public key in
//!   `tauri.conf.json` and, with `requireSignedVersion`, anything whose signed
//!   version differs from the one announced (no downgrade by a crafted
//!   `latest.json`).
//!
//! The headless server updates itself separately (`lb-update`, minisign over
//! `SHA256SUMS`); see `plans/desktop.md` for how the two relate.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::Shell;

/// The placeholder in `tauri.conf.json` until a release key is generated.
const PUBKEY_PLACEHOLDER: &str = "REPLACE_WITH_TAURI_UPDATER_PUBLIC_KEY";

/// First automatic check this long after start (let the app settle first).
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);

/// What the UI and tray show about an available update.
#[derive(Clone, Debug, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
    /// Downloaded, verified and waiting to be installed.
    pub downloaded: bool,
}

struct Pending {
    update: Update,
    bytes: Option<Vec<u8>>,
}

/// Update state shared by the scheduler, tray and settings page.
#[derive(Default)]
pub struct Updates {
    pending: Mutex<Option<Pending>>,
    /// Versions already offered in a dialog this session (don't nag).
    prompted: Mutex<Vec<String>>,
    /// One check or install at a time.
    busy: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for Updates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Updates")
            .field("pending", &self.pending_info().map(|i| i.version))
            .finish_non_exhaustive()
    }
}

impl Updates {
    pub fn pending_info(&self) -> Option<UpdateInfo> {
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.as_ref().map(|p| info(&p.update, p.bytes.is_some()))
    }

    fn set(&self, update: Option<Update>) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match update {
            // Same version already known: keep any download we have.
            Some(u)
                if pending
                    .as_ref()
                    .is_some_and(|p| p.update.version == u.version) => {}
            Some(u) => {
                *pending = Some(Pending {
                    update: u,
                    bytes: None,
                });
            }
            None => *pending = None,
        }
    }

    fn take(&self) -> Option<Pending> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    fn first_prompt(&self, version: &str) -> bool {
        let mut prompted = self
            .prompted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if prompted.iter().any(|v| v == version) {
            false
        } else {
            prompted.push(version.to_owned());
            true
        }
    }
}

fn info(update: &Update, downloaded: bool) -> UpdateInfo {
    UpdateInfo {
        current: update.current_version.clone(),
        version: update.version.clone(),
        notes: update.body.clone(),
        date: update.date.map(|d| d.to_string()),
        downloaded,
    }
}

/// Why this build can't update itself, if it can't.
pub fn unavailable_reason(app: &AppHandle) -> Option<String> {
    if cfg!(debug_assertions) && std::env::var_os("LIBREBOOKS_UPDATER_IN_DEBUG").is_none() {
        return Some("development builds don't update themselves".into());
    }
    let pubkey = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("pubkey"))
        .and_then(|k| k.as_str())
        .unwrap_or_default();
    if pubkey.is_empty() || pubkey == PUBKEY_PLACEHOLDER {
        return Some("this build has no update signing key".into());
    }
    None
}

/// Who asked for a check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// The schedule: honour the install-automatically setting, prompt once.
    Scheduled,
    /// The user (tray or settings): always report the result.
    User,
}

/// Look for an update. Returns what was found.
pub async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    if let Some(why) = unavailable_reason(app) {
        return Err(format!("Updates are off: {why}."));
    }
    let shell = app.state::<Shell>();
    let _busy = shell.updates.busy.lock().await;
    let updater = app.updater().map_err(|e| e.to_string())?;
    let found = updater.check().await.map_err(|e| e.to_string())?;
    let any = found.is_some();
    if let Some(u) = &found {
        log::info!("update available: {} → {}", u.current_version, u.version);
    }
    shell.updates.set(found);
    crate::tray::refresh(app);
    // What's pending now (it may already be downloaded from an earlier check).
    Ok(if any {
        shell.updates.pending_info()
    } else {
        None
    })
}

/// Check, then act per the trigger and settings: download for install-on-quit,
/// or offer it in a dialog.
pub async fn check_and_prompt(app: &AppHandle, trigger: Trigger) {
    let found = match check(app).await {
        Ok(f) => f,
        Err(e) => {
            log::warn!("update check failed: {e}");
            if trigger == Trigger::User {
                crate::show_error(app, "Couldn't check for updates", &e);
            }
            return;
        }
    };
    let Some(found) = found else {
        if trigger == Trigger::User {
            app.dialog()
                .message(format!(
                    "LibreBooks {} is the latest version.",
                    env!("CARGO_PKG_VERSION")
                ))
                .title("No updates")
                .kind(MessageDialogKind::Info)
                .show(|_| {});
        }
        return;
    };
    let shell = app.state::<Shell>();
    let auto = shell.settings().updates.install_automatically;
    if trigger == Trigger::Scheduled && auto {
        if let Err(e) = download(app).await {
            log::warn!("downloading the update failed: {e}");
        }
        return;
    }
    if trigger == Trigger::Scheduled && !shell.updates.first_prompt(&found.version) {
        return;
    }
    let mut text = format!(
        "LibreBooks {} is available (you have {}).",
        found.version, found.current
    );
    if let Some(notes) = found.notes.as_deref().filter(|n| !n.trim().is_empty()) {
        text.push_str("\n\n");
        text.push_str(notes.trim());
    }
    text.push_str("\n\nInstall it now? LibreBooks will close and reopen.");
    let app2 = app.clone();
    app.dialog()
        .message(text)
        .title("Update available")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Install and restart".into(),
            "Later".into(),
        ))
        .show(move |yes| {
            if yes {
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = install_now(&app2).await {
                        crate::show_error(&app2, "Update failed", &e);
                    }
                });
            }
        });
}

/// Download (and verify) the pending update, keeping the bytes for install.
async fn download(app: &AppHandle) -> Result<(), String> {
    let shell = app.state::<Shell>();
    let _busy = shell.updates.busy.lock().await;
    let Some(mut pending) = shell.updates.take() else {
        return Ok(());
    };
    if pending.bytes.is_none() {
        log::info!("downloading LibreBooks {}", pending.update.version);
        match pending.update.download(|_, _| {}, || {}).await {
            Ok(bytes) => pending.bytes = Some(bytes),
            Err(e) => {
                *shell
                    .updates
                    .pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pending);
                return Err(e.to_string());
            }
        }
    }
    *shell
        .updates
        .pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pending);
    crate::tray::refresh(app);
    Ok(())
}

/// Download if needed, close the books, install and restart.
pub async fn install_now(app: &AppHandle) -> Result<(), String> {
    if app.state::<Shell>().updates.pending_info().is_none() {
        check(app).await?;
    }
    download(app).await?;
    let shell = app.state::<Shell>();
    let Some(pending) = shell.updates.take() else {
        return Err("no update to install".into());
    };
    let Some(bytes) = pending.bytes else {
        return Err("the update didn't download".into());
    };
    log::info!("installing LibreBooks {}", pending.update.version);
    // Close the company files before the installer replaces the binary.
    shell.stop_backend().await;
    // On Windows this starts the installer and exits the process (the
    // installer relaunches the app); elsewhere we relaunch ourselves.
    pending.update.install(bytes).map_err(|e| e.to_string())?;
    app.restart();
}

/// At quit: install a downloaded update if the user opted into automatic
/// installs. The backend is already stopped.
pub fn install_on_exit(app: &AppHandle) {
    let shell = app.state::<Shell>();
    if !shell.settings().updates.install_automatically {
        return;
    }
    let Some(Pending {
        update,
        bytes: Some(bytes),
    }) = shell.updates.take()
    else {
        return;
    };
    log::info!("installing LibreBooks {} on the way out", update.version);
    // They quit on purpose: don't reopen the app afterwards.
    let update = update.restart_after_install(false);
    if let Err(e) = update.install(bytes) {
        log::error!("installing the update failed: {e}");
    }
}

/// Check now and then, per the settings.
pub fn spawn_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        loop {
            let settings = app.state::<Shell>().settings().updates;
            if settings.check_automatically && unavailable_reason(&app).is_none() {
                check_and_prompt(&app, Trigger::Scheduled).await;
            }
            let hours = u64::from(settings.check_interval_hours.max(1));
            tokio::time::sleep(Duration::from_secs(hours * 3600)).await;
        }
    });
}
