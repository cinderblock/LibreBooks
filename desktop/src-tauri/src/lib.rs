//! The LibreBooks desktop app (Tauri 2).
//!
//! One process holds:
//! - a **backend** on `127.0.0.1:<random port>`: lb-server itself (books on this
//!   computer) or a thin-client proxy to a remote server (`backend`, `proxy`);
//! - the **main window**, loading the web UI from that backend, with a
//!   per-start token handed over by an initialization script (`window`);
//! - the **settings window** (`desktop/ui`, Tauri's own assets) for the
//!   first-run choice and settings (`commands`, `settings`);
//! - a tray icon, single-instance handling, window-state memory and the
//!   self-updater (`tray`, `updater`).
//!
//! Design and decisions: `plans/desktop.md`.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

pub mod backend;
mod commands;
pub mod device;
pub mod network;
pub mod proxy;
pub mod settings;
mod tray;
mod updater;
mod window;

use backend::{Backend, DevOverrides};
use settings::{Settings, SettingsError};

/// What's running, for the tray and the settings page.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Status {
    /// One line: what the main window is connected to.
    pub running: Option<String>,
    /// Why the backend isn't running.
    pub error: Option<String>,
    /// Where the books are shared on the network.
    pub network: Option<String>,
    /// Why sharing to the network was asked for but isn't happening.
    pub network_error: Option<String>,
    /// Why the thin client can't sign in.
    pub remote_problem: Option<String>,
}

/// App-wide state, managed by Tauri.
#[derive(Debug)]
pub struct Shell {
    settings_path: PathBuf,
    settings: Mutex<Settings>,
    backend: tokio::sync::Mutex<Option<Backend>>,
    status: Mutex<Status>,
    dev: DevOverrides,
    /// HTTP client for the thin client, enrolment and server checks.
    pub client: reqwest::Client,
    /// The main window is being replaced; closing it must not quit the app.
    restarting: AtomicBool,
    pub updates: updater::Updates,
}

impl Shell {
    fn new(settings_path: PathBuf) -> Shell {
        let settings = Settings::load(&settings_path);
        let client = reqwest::Client::builder()
            .user_agent(concat!("LibreBooks-Desktop/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("an HTTP client with these options always builds");
        Shell {
            settings_path,
            settings: Mutex::new(settings),
            backend: tokio::sync::Mutex::new(None),
            status: Mutex::new(Status::default()),
            dev: DevOverrides::from_env(),
            client,
            restarting: AtomicBool::new(false),
            updates: updater::Updates::default(),
        }
    }

    /// A copy of the current settings.
    pub fn settings(&self) -> Settings {
        self.settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Save and adopt new settings.
    pub fn set_settings(&self, settings: Settings) -> Result<(), SettingsError> {
        settings.save(&self.settings_path)?;
        *self
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = settings;
        Ok(())
    }

    pub fn status_snapshot(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn set_status(&self, status: Status) {
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = status;
    }

    /// Stop the backend (closing company files), if one is running.
    pub async fn stop_backend(&self) {
        let old = self.backend.lock().await.take();
        if let Some(b) = old {
            b.stop().await;
        }
    }
}

/// Show an error dialog.
pub fn show_error(app: &AppHandle, title: &str, message: &str) {
    log::error!("{title}: {message}");
    app.dialog()
        .message(message)
        .title(title)
        .kind(MessageDialogKind::Error)
        .show(|_| {});
}

/// (Re)start the backend the settings ask for and (re)build the main window
/// around it. Used at start, and when the mode, company folder or server
/// changes.
pub async fn activate(app: &AppHandle) {
    let shell = app.state::<Shell>();
    shell.restarting.store(true, Ordering::SeqCst);
    let mut slot = shell.backend.lock().await;
    if let Some(w) = app.get_webview_window(window::MAIN) {
        // destroy, not close: skip the close handler (which might hide it).
        let _ = w.destroy();
        // The label is only released once the event loop has processed it.
        for _ in 0..100 {
            if app.get_webview_window(window::MAIN).is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    if let Some(old) = slot.take() {
        old.stop().await;
    }
    let settings = shell.settings();
    // Whether something needs the user's attention afterwards.
    let problem = match Backend::start(&settings, &shell.client, &shell.dev).await {
        Ok(b) => {
            shell.set_status(Status {
                running: Some(b.describe()),
                error: None,
                network: b.network.map(|a| a.to_string()),
                network_error: b.network_error.clone(),
                remote_problem: b.remote_problem.clone(),
            });
            if let Err(e) = window::open_main(app, &b, &shell.dev) {
                show_error(app, "LibreBooks couldn't open its window", &e.to_string());
            }
            let problem = b.remote_problem.is_some() || b.network_error.is_some();
            *slot = Some(b);
            problem
        }
        Err(e) => {
            shell.set_status(Status {
                error: Some(e.to_string()),
                ..Status::default()
            });
            show_error(app, "LibreBooks couldn't start", &e.to_string());
            true
        }
    };
    drop(slot);
    shell.restarting.store(false, Ordering::SeqCst);
    tray::refresh(app);
    // Something needs the user's attention: put settings in front.
    if problem && let Err(e) = window::open_settings(app, false) {
        log::error!("couldn't open settings: {e}");
    }
}

/// Closing the main window quits, unless the books are being served to the
/// network, in which case it only hides (the tray icon brings it back).
fn on_window_event(window: &tauri::Window, event: &WindowEvent) {
    if window.label() != window::MAIN {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        let serving = window
            .app_handle()
            .state::<Shell>()
            .status_snapshot()
            .network
            .is_some();
        if serving {
            api.prevent_close();
            let _ = window.hide();
        }
    }
}

fn on_run_event(app: &AppHandle, event: RunEvent) {
    match event {
        RunEvent::ExitRequested { code, api, .. } => {
            // The last window closing while the main window is being rebuilt
            // isn't the user quitting.
            if code.is_none() && app.state::<Shell>().restarting.load(Ordering::SeqCst) {
                api.prevent_exit();
            }
        }
        // The main window gone for good (not being rebuilt) quits the app, even
        // if the settings window is still open. (Here rather than in the window
        // handler: by now Tauri has dropped the label.)
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::Destroyed,
            ..
        } if label == window::MAIN
            && !app.state::<Shell>().restarting.load(Ordering::SeqCst)
            && app.get_webview_window(window::MAIN).is_none() =>
        {
            app.exit(0);
        }
        RunEvent::Exit => {
            log::info!("quitting");
            let shell = app.state::<Shell>();
            tauri::async_runtime::block_on(async {
                if tokio::time::timeout(Duration::from_secs(10), shell.stop_backend())
                    .await
                    .is_err()
                {
                    log::error!("the server didn't stop in time");
                }
            });
            updater::install_on_exit(app);
        }
        _ => {}
    }
}

/// macOS needs an app menu for Quit and the Edit shortcuts (copy/paste). It
/// deliberately has no Close Window item, so Cmd+W reaches the web UI.
#[cfg(target_os = "macos")]
fn macos_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, PredefinedMenuItem, Submenu};
    let app_menu = Submenu::with_items(
        app,
        "LibreBooks",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &edit])
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let settings_path = handle.path().app_config_dir()?.join("settings.json");
    app.manage(Shell::new(settings_path));
    log::info!(
        "LibreBooks {} starting ({})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    );

    #[cfg(target_os = "macos")]
    app.set_menu(macos_menu(&handle)?)?;

    tray::create(&handle)?;
    if handle.state::<Shell>().settings().configured {
        tauri::async_runtime::spawn(async move { activate(&handle).await });
    } else {
        window::open_settings(&handle, true)?;
    }
    updater::spawn_scheduler(app.handle().clone());
    Ok(())
}

/// Run the desktop app.
pub fn run() {
    let log = tauri_plugin_log::Builder::new()
        .clear_targets()
        .target(tauri_plugin_log::Target::new(
            tauri_plugin_log::TargetKind::Stdout,
        ))
        .target(tauri_plugin_log::Target::new(
            tauri_plugin_log::TargetKind::LogDir { file_name: None },
        ))
        .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
        .max_file_size(5 * 1024 * 1024)
        .level(log::LevelFilter::Info)
        .level_for("tower_http", log::LevelFilter::Warn)
        .level_for("hyper", log::LevelFilter::Warn)
        .level_for("reqwest", log::LevelFilter::Warn)
        .build();

    let app = tauri::Builder::default()
        // First, so a second launch is caught before anything else starts.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_app(app);
        }))
        .plugin(log)
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .with_denylist(&[window::SETTINGS])
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::get_shell,
            commands::save_settings,
            commands::pick_folder,
            commands::open_data_folder,
            commands::test_server,
            commands::enroll_device,
            commands::forget_device,
            commands::check_for_updates,
            commands::install_update,
            commands::open_settings,
        ])
        .on_window_event(on_window_event)
        .setup(setup)
        .build(tauri::generate_context!())
        .expect("error while building the LibreBooks desktop app");
    app.run(on_run_event);
}
