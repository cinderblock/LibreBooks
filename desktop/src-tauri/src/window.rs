//! The windows: `main` (the SPA, from the loopback backend) and `settings`
//! (the desktop shell's own page, from Tauri's assets).

use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;
use url::Url;

use crate::backend::{Backend, BackendKind, DevOverrides};

pub const MAIN: &str = "main";
pub const SETTINGS: &str = "settings";

/// What the SPA learns about its host, as `window.__LIBREBOOKS__` (see
/// `plans/desktop.md`, "Contract with the web UI").
fn host_info(backend: &Backend) -> serde_json::Value {
    let (kind, server) = match &backend.kind {
        BackendKind::Local { .. } => ("local", None),
        BackendKind::Remote { server } => ("remote", Some(server.as_str())),
    };
    serde_json::json!({
        "mode": "desktop",
        "backend": kind,
        "server": server,
        "token": &*backend.token,
        "apiBase": "/api/v1",
        "keyboard": "native",
        "shell": {
            "version": env!("CARGO_PKG_VERSION"),
            "os": std::env::consts::OS,
        },
    })
}

/// The main window's initialization script. It runs before any page script on
/// every load of the top frame, and only hands over the token when the page is
/// on a trusted (loopback) origin.
fn init_script(backend: &Backend, trusted: &[String]) -> String {
    let info = host_info(backend);
    let trusted = serde_json::to_string(trusted).unwrap_or_else(|_| "[]".into());
    let release = !cfg!(debug_assertions);
    format!(
        r#"(() => {{
  if (!{trusted}.includes(window.location.origin)) return;
  const info = {info};
  info.shell.openSettings = () => window.__TAURI_INTERNALS__.invoke('open_settings');
  Object.freeze(info.shell);
  Object.defineProperty(window, '__LIBREBOOKS__', {{
    value: Object.freeze(info), writable: false, configurable: false, enumerable: false,
  }});
  if ({release}) {{
    // No browser context menu (Reload, Back, Inspect…) except where it helps:
    // editable fields and selected text. The SPA may show its own menus.
    window.addEventListener('contextmenu', (e) => {{
      if (e.defaultPrevented) return;
      const t = e.target;
      if (t instanceof Element && t.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])')) return;
      if (String(window.getSelection() ?? '') !== '') return;
      e.preventDefault();
    }});
  }}
}})();"#
    )
}

/// Whether the main window may show `url` itself.
fn trusted_url(url: &Url, trusted: &[String]) -> bool {
    if url.as_str() == "about:blank" {
        return true;
    }
    // `origin()` sees through `blob:` URLs to the page that made them.
    let origin = url.origin().ascii_serialization();
    trusted.contains(&origin)
}

/// Open links that leave the app in the system browser.
fn open_outside(app: &AppHandle, url: &Url) {
    if matches!(url.scheme(), "http" | "https" | "mailto")
        && let Err(e) = app.opener().open_url(url.as_str(), None::<&str>)
    {
        log::warn!("couldn't open {url}: {e}");
    }
}

/// Create the main window for `backend`. Any previous one must be closed.
pub fn open_main(
    app: &AppHandle,
    backend: &Backend,
    dev: &DevOverrides,
) -> tauri::Result<WebviewWindow> {
    let home = format!("{}/", backend.origin);
    let start = match dev.web_dev_url.clone() {
        Some(u) => u,
        None => Url::parse(&home).expect("a loopback origin is a valid URL"),
    };
    let mut trusted = vec![backend.origin.clone()];
    if let Some(u) = &dev.web_dev_url {
        trusted.push(u.origin().ascii_serialization());
    }
    let script = init_script(backend, &trusted);

    let nav_app = app.clone();
    let nav_trusted = trusted.clone();
    let popup_app = app.clone();
    let mut builder = WebviewWindowBuilder::new(
        app,
        MAIN,
        // Blank first, so the webview's settings are in place before the SPA loads.
        WebviewUrl::External(Url::parse("about:blank").expect("valid")),
    )
    .title("LibreBooks")
    .inner_size(1280.0, 820.0)
    .min_inner_size(720.0, 480.0)
    .initialization_script(script)
    .on_navigation(move |url| {
        if trusted_url(url, &nav_trusted) {
            true
        } else {
            open_outside(&nav_app, url);
            false
        }
    })
    .on_new_window(move |url, _features| {
        open_outside(&popup_app, &url);
        NewWindowResponse::Deny
    })
    // Let the SPA receive HTML5 drag and drop (e.g. a receipt onto a bill);
    // otherwise Tauri intercepts file drops on Windows.
    .disable_drag_drop_handler()
    .devtools(cfg!(debug_assertions));
    if cfg!(debug_assertions) && std::env::var_os("LIBREBOOKS_SELFTEST").is_some() {
        builder = self_test(builder);
    }
    let window = builder.build()?;

    keep_keys_for_the_app(&window);
    window.navigate(start)?;
    Ok(window)
}

/// Debug builds, `LIBREBOOKS_SELFTEST=1`: once the SPA has loaded, call
/// `/api/v1/session` from inside the page exactly as the SPA would (bearer
/// token from `window.__LIBREBOOKS__`) and log the outcome. Proves the token
/// injection and the backend end to end without any UI to click.
fn self_test(
    builder: WebviewWindowBuilder<'_, tauri::Wry, AppHandle>,
) -> WebviewWindowBuilder<'_, tauri::Wry, AppHandle> {
    const PROBE: &str = r"(async () => {
  const lb = window.__LIBREBOOKS__;
  if (!lb) { document.title = 'selftest: no __LIBREBOOKS__ on ' + location.origin; return; }
  const r = await fetch(lb.apiBase + '/session', { headers: { Authorization: 'Bearer ' + lb.token } });
  const bare = await fetch(lb.apiBase + '/session');
  document.title = 'selftest: backend=' + lb.backend + ' session=' + r.status + ' without-token=' + bare.status + ' body=' + (await r.text()).slice(0, 200);
})().catch((e) => { document.title = 'selftest: error ' + e; });";
    builder
        .on_page_load(|w, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished
                && payload.url().scheme() == "http"
            {
                let _ = w.eval(PROBE);
            }
        })
        .on_document_title_changed(|_, title| {
            if title.starts_with("selftest:") {
                log::info!("{title}");
            }
        })
}

/// Make sure the webview passes QuickBooks' shortcuts (Ctrl+R register, Ctrl+F
/// find, Ctrl+P print, F-keys…) to the page instead of acting on them itself.
///
/// - WebView2 (Windows) has "browser accelerator keys" (reload, find, print,
///   devtools, caret browsing…); they're switched off here.
/// - WebKitGTK (Linux) binds none of these.
/// - Tauri adds no Ctrl+W/close shortcut on Windows or Linux; macOS's menu is
///   built in `macos_menu` (lib.rs) without one.
fn keep_keys_for_the_app(window: &WebviewWindow) {
    #[cfg(windows)]
    {
        let result = window.with_webview(|wv| {
            if let Err(e) = webview2::disable_browser_accelerator_keys(&wv.controller()) {
                log::warn!("couldn't turn off WebView2 browser shortcuts: {e}");
            }
        });
        if let Err(e) = result {
            log::warn!("couldn't reach the webview: {e}");
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod webview2 {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Controller, ICoreWebView2Settings3,
    };
    use windows_core::Interface;

    /// `ICoreWebView2Settings3::put_AreBrowserAcceleratorKeysEnabled(FALSE)`.
    /// Tauri doesn't expose this setting, so it goes through COM directly.
    pub fn disable_browser_accelerator_keys(
        controller: &ICoreWebView2Controller,
    ) -> windows_core::Result<()> {
        // SAFETY: plain COM method calls on live interfaces owned by Tauri's
        // webview, made on the UI thread (`with_webview` runs us there); no raw
        // pointers are created or kept.
        unsafe {
            let settings = controller.CoreWebView2()?.Settings()?;
            settings
                .cast::<ICoreWebView2Settings3>()?
                .SetAreBrowserAcceleratorKeysEnabled(false)
        }
    }
}

/// Bring a window to the front.
pub fn focus(window: &WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

/// Open (or focus) the settings window. On first run it is the welcome page.
pub fn open_settings(app: &AppHandle, welcome: bool) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window(SETTINGS) {
        focus(&w);
        return Ok(());
    }
    let mut builder =
        WebviewWindowBuilder::new(app, SETTINGS, WebviewUrl::App("index.html".into()))
            .title(if welcome {
                "Welcome to LibreBooks"
            } else {
                "LibreBooks Settings"
            })
            .inner_size(680.0, 760.0)
            .min_inner_size(520.0, 480.0)
            .center()
            .devtools(cfg!(debug_assertions));
    if let Some(main) = app.get_webview_window(MAIN) {
        builder = builder.parent(&main)?;
    }
    let w = builder.build()?;
    keep_keys_for_the_app(&w);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_trusted_origins_stay_in_the_window() {
        let trusted = vec!["http://127.0.0.1:4321".to_owned()];
        let ok = |s: &str| trusted_url(&Url::parse(s).unwrap(), &trusted);
        assert!(ok("http://127.0.0.1:4321/c/abc/registers"));
        assert!(ok("blob:http://127.0.0.1:4321/5d1f…"));
        assert!(ok("about:blank"));
        assert!(!ok("http://127.0.0.1:9999/"));
        assert!(!ok("http://localhost:4321/"));
        assert!(!ok("https://example.com/"));
        assert!(!ok("data:text/html,hi"));
    }
}
