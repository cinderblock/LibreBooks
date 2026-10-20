//! The tray icon: open the app, see what it's running, settings, updates, quit.

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::Shell;
use crate::window::{self, MAIN, SETTINGS};

const TRAY_ID: &str = "librebooks";

fn menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let shell = app.state::<Shell>();
    let status = shell.status_snapshot();
    let line = status
        .error
        .as_deref()
        .map(|e| format!("Not running: {e}"))
        .or(status.running.clone())
        .unwrap_or_else(|| "Starting…".to_owned());
    let line = truncate(&line, 60);

    let update_item = match shell.updates.pending_info() {
        Some(info) if info.downloaded => MenuItem::with_id(
            app,
            "install_update",
            format!("Restart to update to {}", info.version),
            true,
            None::<&str>,
        )?,
        Some(info) => MenuItem::with_id(
            app,
            "install_update",
            format!("Update to {}…", info.version),
            true,
            None::<&str>,
        )?,
        None => MenuItem::with_id(
            app,
            "check_updates",
            "Check for updates",
            true,
            None::<&str>,
        )?,
    };

    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", "Open LibreBooks", true, None::<&str>)?,
            &MenuItem::with_id(app, "status", line, false, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?,
            &update_item,
        ],
    )?;
    if cfg!(debug_assertions) {
        menu.append(&MenuItem::with_id(
            app,
            "devtools",
            "Developer tools",
            true,
            None::<&str>,
        )?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        "Quit LibreBooks",
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}

/// Bring up the main window, or the settings window if there's no main one.
pub fn show_app(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        window::focus(&w);
    } else if let Some(w) = app.get_webview_window(SETTINGS) {
        window::focus(&w);
    } else {
        let welcome = !app.state::<Shell>().settings().configured;
        if let Err(e) = window::open_settings(app, welcome) {
            log::error!("couldn't open settings: {e}");
        }
    }
}

fn on_menu(app: &AppHandle, event: &MenuEvent) {
    match event.id().as_ref() {
        "open" => show_app(app),
        "settings" => {
            let welcome = !app.state::<Shell>().settings().configured;
            if let Err(e) = window::open_settings(app, welcome) {
                log::error!("couldn't open settings: {e}");
            }
        }
        "check_updates" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                crate::updater::check_and_prompt(&app, crate::updater::Trigger::User).await;
            });
        }
        "install_update" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::updater::install_now(&app).await {
                    crate::show_error(&app, "Update failed", &e);
                }
            });
        }
        "devtools" => {
            if let Some(w) = app.get_webview_window(MAIN) {
                w.open_devtools();
            }
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

#[allow(clippy::needless_pass_by_value)] // the shape Tauri's tray callback has
fn on_icon(tray: &TrayIcon, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        show_app(tray.app_handle());
    }
}

/// Create the tray icon.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("LibreBooks")
        .menu(&menu(app)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, &event))
        .on_tray_icon_event(on_icon);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Rebuild the menu and tooltip after the status or a pending update changes.
pub fn refresh(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    match menu(app) {
        Ok(m) => {
            let _ = tray.set_menu(Some(m));
        }
        Err(e) => log::warn!("couldn't rebuild the tray menu: {e}"),
    }
    let status = app.state::<Shell>().status_snapshot();
    let tip = status
        .running
        .map_or_else(|| "LibreBooks".to_owned(), |s| format!("LibreBooks — {s}"));
    let _ = tray.set_tooltip(Some(tip));
}
