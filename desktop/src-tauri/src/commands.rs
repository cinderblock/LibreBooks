//! Tauri commands. Which window may call which is set by the capabilities in
//! `capabilities/` (permissions generated in `build.rs`):
//! - the settings window (`desktop/ui`, Tauri's own origin) gets everything here
//!   except `open_settings`;
//! - the main window (the SPA on the loopback origin) gets only
//!   `open_settings`.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::settings::{Settings, default_data_dir, parse_server_url};
use crate::updater::{self, UpdateInfo};
use crate::window::{self, SETTINGS};
use crate::{Shell, Status, device};

/// Everything the settings page shows.
#[derive(Debug, Serialize)]
pub struct ShellView {
    pub settings: Settings,
    pub default_data_dir: String,
    pub data_dir: String,
    pub status: Status,
    pub version: &'static str,
    pub os: &'static str,
    /// Why serving to the network isn't possible, if it isn't.
    pub network_unavailable: Option<String>,
    /// Why the OS keychain can't be used, if it can't.
    pub keychain_unavailable: Option<String>,
    /// Why this build can't update itself, if it can't.
    pub updates_unavailable: Option<String>,
    pub update: Option<UpdateInfo>,
    /// A name for this computer, for device enrolment.
    pub computer_name: String,
    /// Whether this computer is enrolled with the configured server.
    pub enrolled: bool,
}

fn computer_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "This computer".to_owned())
}

fn view(app: &AppHandle) -> ShellView {
    let shell = app.state::<Shell>();
    let settings = shell.settings();
    ShellView {
        default_data_dir: default_data_dir().display().to_string(),
        data_dir: settings.data_dir().display().to_string(),
        enrolled: settings.device_for_remote().is_some(),
        settings,
        status: shell.status_snapshot(),
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        network_unavailable: crate::network::available().err().map(|e| e.to_string()),
        keychain_unavailable: device::keychain_status().err(),
        updates_unavailable: updater::unavailable_reason(app),
        update: shell.updates.pending_info(),
        computer_name: computer_name(),
    }
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri hands commands an owned AppHandle
pub fn get_shell(app: AppHandle) -> ShellView {
    view(&app)
}

/// Save settings from the settings page, restarting the backend if what it
/// runs changed. On first run this also starts the app proper.
#[tauri::command]
pub async fn save_settings(app: AppHandle, settings: Settings) -> Result<ShellView, String> {
    let shell = app.state::<Shell>();
    let old = shell.settings();
    let mut new = settings;
    // Not the page's to change: the enrolment is written only by enroll/forget.
    new.remote.device = old.remote.device.clone();
    new.version = crate::settings::SCHEMA_VERSION;
    new.configured = true;
    new.validate().map_err(|e| e.to_string())?;
    let first_run = !old.configured;
    let restart = first_run
        || old.mode != new.mode
        || old.local != new.local
        || old.remote != new.remote
        || old.network != new.network;
    shell.set_settings(new).map_err(|e| e.to_string())?;
    if restart {
        crate::activate(&app).await;
    }
    if first_run && let Some(w) = app.get_webview_window(SETTINGS) {
        let _ = w.close();
    }
    Ok(view(&app))
}

/// Ask for a folder for company files.
#[tauri::command]
pub async fn pick_folder(app: AppHandle, current: Option<String>) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Folder for company files")
        .set_can_create_directories(true);
    if let Some(dir) = current.filter(|d| std::path::Path::new(d).is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    if let Some(w) = app.get_webview_window(SETTINGS) {
        dialog = dialog.set_parent(&w);
    }
    dialog.pick_folder(move |picked| {
        let _ = tx.send(picked.and_then(|p| p.into_path().ok()));
    });
    rx.await.ok().flatten().map(|p| p.display().to_string())
}

/// Show the company folder in the file manager.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri hands commands an owned AppHandle
pub fn open_data_folder(app: AppHandle) -> Result<(), String> {
    let dir = app.state::<Shell>().settings().data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(dir.display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// What `/api/v1/health` says.
#[derive(Debug, Deserialize, Serialize)]
pub struct Health {
    pub version: String,
    pub mode: String,
}

/// Check a server address answers like LibreBooks.
#[tauri::command]
pub async fn test_server(
    app: AppHandle,
    url: String,
    allow_insecure_http: bool,
) -> Result<Health, String> {
    let base = parse_server_url(&url, allow_insecure_http).map_err(|e| e.to_string())?;
    let health = base.join("api/v1/health").map_err(|e| e.to_string())?;
    let client = app.state::<Shell>().client.clone();
    let resp = client
        .get(health.clone())
        .send()
        .await
        .map_err(|e| format!("couldn't reach {base}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "{base} answered {} — is it a LibreBooks server?",
            resp.status()
        ));
    }
    resp.json::<Health>()
        .await
        .map_err(|_| format!("{base} answered, but not like a LibreBooks server"))
}

/// Connect this computer to a server with an enrolment link or token.
#[tauri::command]
pub async fn enroll_device(
    app: AppHandle,
    url: Option<String>,
    enrollment: String,
    name: String,
    allow_insecure_http: bool,
) -> Result<ShellView, String> {
    let (from_link, token) = device::parse_enrollment(&enrollment).map_err(|e| e.to_string())?;
    let typed = url.filter(|u| !u.trim().is_empty());
    let server = match (typed, from_link) {
        (Some(u), _) => parse_server_url(&u, allow_insecure_http),
        (None, Some(u)) => parse_server_url(u.as_str(), allow_insecure_http),
        (None, None) => {
            return Err("enter the server's address, or paste the whole enrolment link".into());
        }
    }
    .map_err(|e| e.to_string())?;
    let name = if name.trim().is_empty() {
        computer_name()
    } else {
        name.trim().to_owned()
    };
    let shell = app.state::<Shell>();
    let enrolled = device::enroll(&shell.client, &server, &token, &name)
        .await
        .map_err(|e| e.to_string())?;
    log::info!("enrolled this computer with {} as {name}", enrolled.origin);
    let mut settings = shell.settings();
    settings.remote.url = Some(server.to_string());
    settings.remote.allow_insecure_http = allow_insecure_http;
    settings.remote.device = Some(enrolled);
    let running_remote =
        settings.configured && settings.mode == crate::settings::BackendMode::Remote;
    shell.set_settings(settings).map_err(|e| e.to_string())?;
    if running_remote {
        crate::activate(&app).await;
    }
    Ok(view(&app))
}

/// Forget this computer's key for the configured server.
#[tauri::command]
pub async fn forget_device(app: AppHandle) -> Result<ShellView, String> {
    let shell = app.state::<Shell>();
    let mut settings = shell.settings();
    if let Some(d) = settings.remote.device.take() {
        let origin = d.origin.clone();
        tokio::task::spawn_blocking(move || device::forget_key(&origin))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
    }
    let running_remote =
        settings.configured && settings.mode == crate::settings::BackendMode::Remote;
    shell.set_settings(settings).map_err(|e| e.to_string())?;
    if running_remote {
        crate::activate(&app).await;
    }
    Ok(view(&app))
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    updater::check(&app).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    updater::install_now(&app).await
}

/// For the SPA (main window): open the desktop settings. (Async: creating a
/// window from a synchronous command deadlocks on Windows.)
#[tauri::command]
pub async fn open_settings(app: AppHandle) -> Result<(), String> {
    let welcome = !app.state::<Shell>().settings().configured;
    window::open_settings(&app, welcome).map_err(|e| e.to_string())
}
