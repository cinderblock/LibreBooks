//! The windows: `main` (the SPA, from the loopback backend) and `settings`
//! (the desktop shell's own page, from Tauri's assets).

use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;
use url::Url;

use crate::backend::{Backend, BackendKind, DevOverrides};

pub const MAIN: &str = "main";
pub const SETTINGS: &str = "settings";
/// Other company windows (opened from a page): `company-1`, `company-2`…
const EXTRA_PREFIX: &str = "company-";

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

/// What only local mode offers the page: the server is on this computer, so a
/// file's path here means something to it. `pickCompanyFile` is the native
/// file dialog; `pathOf(file)` resolves a dropped `File` to its full path,
/// which a page can't see, through WebView2's
/// `postMessageWithAdditionalObjects` (answered by `webview2::answer_file_paths`).
const LOCAL_SHELL: &str = r"
  info.shell.pickCompanyFile = (current) =>
    window.__TAURI_INTERNALS__.invoke('pick_company_file', { current: current || null });
  info.shell.pathOf = (file) => new Promise((resolve) => {
    const wv = window.chrome && window.chrome.webview;
    if (!wv || typeof wv.postMessageWithAdditionalObjects !== 'function') { resolve(null); return; }
    const id = String(Math.random()).slice(2);
    const done = (path) => { window.removeEventListener('librebooks-file-path', on); clearTimeout(timer); resolve(path); };
    const on = (e) => { if (e.detail && e.detail.id === id) done(e.detail.path || null); };
    const timer = setTimeout(() => done(null), 5000);
    window.addEventListener('librebooks-file-path', on);
    wv.postMessageWithAdditionalObjects(JSON.stringify({ librebooks: 'file-path', id }), [file]);
  });
";

/// The main window's initialization script. It runs before any page script on
/// every load of the top frame, and only hands over the token when the page is
/// on a trusted (loopback) origin.
fn init_script(backend: &Backend, trusted: &[String]) -> String {
    let info = host_info(backend);
    let trusted = serde_json::to_string(trusted).unwrap_or_else(|_| "[]".into());
    let release = !cfg!(debug_assertions);
    let local_shell = if matches!(backend.kind, BackendKind::Local { .. }) {
        LOCAL_SHELL
    } else {
        ""
    };
    format!(
        r#"(() => {{
  if (!{trusted}.includes(window.location.origin)) return;
  const info = {info};
  info.shell.openSettings = () => window.__TAURI_INTERNALS__.invoke('open_settings');
{local_shell}
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

/// Create the main window for `backend`. Any previous company windows must be
/// closed.
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
    let recipe = Recipe {
        script: init_script(backend, &trusted),
        trusted,
        local: matches!(backend.kind, BackendKind::Local { .. }),
    };
    build_company_window(app, &recipe, MAIN, start)
}

/// What every company window of one backend is built from.
#[derive(Clone)]
struct Recipe {
    script: String,
    trusted: Vec<String>,
    local: bool,
}

/// Whether `label` is a window showing the web UI (the main window or another
/// company window), as opposed to the settings window.
pub fn is_company_window(label: &str) -> bool {
    label == MAIN || label.starts_with(EXTRA_PREFIX)
}

/// The windows showing the web UI, the main window first.
pub fn company_windows(app: &AppHandle) -> Vec<WebviewWindow> {
    let mut all: Vec<WebviewWindow> = app
        .webview_windows()
        .into_values()
        .filter(|w| is_company_window(w.label()))
        .collect();
    all.sort_by_key(|w| (w.label() != MAIN, w.label().to_owned()));
    all
}

/// The company window to act on: the focused one, else a visible one (the
/// main window first), else any.
pub fn front_company_window(app: &AppHandle) -> Option<WebviewWindow> {
    let all = company_windows(app);
    let pick = all
        .iter()
        .position(|w| w.is_focused().unwrap_or(false))
        .or_else(|| all.iter().position(|w| w.is_visible().unwrap_or(false)))
        .unwrap_or(0);
    all.into_iter().nth(pick)
}

/// The label of the next extra company window: `company-1`, `company-2`…
fn next_extra_label() -> String {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{EXTRA_PREFIX}{n}")
}

/// Whether a page's request for a new window (`window.open`, a Shift+click…)
/// becomes another company window: only for the app's own pages.
fn opens_company_window(url: &Url, trusted: &[String]) -> bool {
    matches!(url.scheme(), "http" | "https") && trusted_url(url, trusted)
}

/// Build a window showing the web UI at `start`: the main window, or another
/// one opened from a page (each with the same token, guards and WebView2 set-up).
fn build_company_window(
    app: &AppHandle,
    recipe: &Recipe,
    label: &str,
    start: Url,
) -> tauri::Result<WebviewWindow> {
    let nav_app = app.clone();
    let nav_trusted = recipe.trusted.clone();
    let popup_app = app.clone();
    let popup_recipe = recipe.clone();
    let selftest = label == MAIN
        && cfg!(debug_assertions)
        && std::env::var_os("LIBREBOOKS_SELFTEST").is_some();
    let mut builder = WebviewWindowBuilder::new(
        app,
        label,
        // Blank first, so the webview's settings are in place before the SPA loads.
        WebviewUrl::External(Url::parse("about:blank").expect("valid")),
    )
    .title("LibreBooks")
    .inner_size(1280.0, 820.0)
    .min_inner_size(720.0, 480.0)
    .initialization_script(recipe.script.clone())
    .on_navigation(move |url| {
        if trusted_url(url, &nav_trusted) {
            true
        } else {
            open_outside(&nav_app, url);
            false
        }
    })
    .on_new_window(move |url, _features| {
        if opens_company_window(&url, &popup_recipe.trusted) {
            // Not from inside WebView2's event: building a webview there can
            // deadlock on Windows.
            let app = popup_app.clone();
            let recipe = popup_recipe.clone();
            tauri::async_runtime::spawn(async move {
                match build_company_window(&app, &recipe, &next_extra_label(), url) {
                    Ok(w) => focus(&w),
                    Err(e) => log::error!("couldn't open another window: {e}"),
                }
            });
        } else {
            open_outside(&popup_app, &url);
        }
        NewWindowResponse::Deny
    })
    // The taskbar and Alt+Tab tell windows apart by the page's title, which
    // names the company ("Invoice · TWILL · LibreBooks").
    .on_document_title_changed(move |w, title| {
        if selftest && title.starts_with("selftest:") {
            log::info!("{title}");
        } else if !title.is_empty() {
            let _ = w.set_title(&title);
        }
    })
    // Let the SPA receive HTML5 drag and drop (e.g. a receipt onto a bill);
    // otherwise Tauri intercepts file drops on Windows.
    .disable_drag_drop_handler()
    .devtools(cfg!(debug_assertions));
    if selftest {
        builder = self_test(builder);
    }
    let window = builder.build()?;

    keep_keys_for_the_app(&window);
    if recipe.local {
        answer_file_paths(&window, recipe.trusted.clone());
    }
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

/// Answer the page's `pathOf(file)` (see [`LOCAL_SHELL`]).
fn answer_file_paths(window: &WebviewWindow, trusted: Vec<String>) {
    #[cfg(windows)]
    {
        let result = window.with_webview(move |wv| {
            if let Err(e) = webview2::answer_file_paths(&wv.controller(), trusted) {
                log::warn!("dropped files won't give their paths: {e}");
            }
        });
        if let Err(e) = result {
            log::warn!("couldn't reach the webview: {e}");
        }
    }
    #[cfg(not(windows))]
    let _ = (window, trusted);
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod webview2 {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2, ICoreWebView2Controller, ICoreWebView2File, ICoreWebView2Settings3,
        ICoreWebView2WebMessageReceivedEventArgs, ICoreWebView2WebMessageReceivedEventArgs2,
    };
    use webview2_com::{ExecuteScriptCompletedHandler, WebMessageReceivedEventHandler, take_pwstr};
    use windows_core::{HSTRING, Interface, PWSTR};

    /// Listen for `{ librebooks: "file-path", id }` messages carrying a `File`,
    /// from a trusted origin only, and answer each with a
    /// `librebooks-file-path` event `{ id, path }`. Tauri's own IPC handler sees
    /// these messages too and logs one console error for each (not IPC).
    pub fn answer_file_paths(
        controller: &ICoreWebView2Controller,
        trusted: Vec<String>,
    ) -> windows_core::Result<()> {
        let handler = WebMessageReceivedEventHandler::create(Box::new(
            move |sender: Option<ICoreWebView2>,
                  args: Option<ICoreWebView2WebMessageReceivedEventArgs>| {
                let (Some(sender), Some(args)) = (sender, args) else {
                    return Ok(());
                };
                if let Some((id, path)) = file_path(&args, &trusted) {
                    let detail = serde_json::json!({ "id": id, "path": path });
                    let js = format!(
                        "window.dispatchEvent(new CustomEvent('librebooks-file-path', {{ detail: {detail} }}))"
                    );
                    // SAFETY: a COM call on the live webview that raised the
                    // event, on its UI thread; the string outlives the call.
                    unsafe {
                        let _ = sender.ExecuteScript(
                            &HSTRING::from(js),
                            &ExecuteScriptCompletedHandler::create(Box::new(|_, _| Ok(()))),
                        );
                    }
                }
                Ok(())
            },
        ));
        let mut token = 0i64;
        // SAFETY: registering a handler on Tauri's live webview, on the UI
        // thread (`with_webview` runs us there). It lives as long as the webview.
        unsafe {
            controller
                .CoreWebView2()?
                .add_WebMessageReceived(&handler, &raw mut token)
        }
    }

    /// The id and the file's path, when this is our message from a trusted page.
    fn file_path(
        args: &ICoreWebView2WebMessageReceivedEventArgs,
        trusted: &[String],
    ) -> Option<(String, Option<String>)> {
        // SAFETY: COM getters on the event's live arguments; each returned
        // string is taken (and freed) by `take_pwstr`.
        unsafe {
            let mut source = PWSTR::null();
            args.Source(&raw mut source).ok()?;
            let source = take_pwstr(source);
            let origin = url::Url::parse(&source).ok()?.origin().ascii_serialization();
            if !trusted.contains(&origin) {
                return None;
            }
            let mut text = PWSTR::null();
            args.TryGetWebMessageAsString(&raw mut text).ok()?;
            let text = take_pwstr(text);
            let msg: serde_json::Value = serde_json::from_str(&text).ok()?;
            if msg.get("librebooks")?.as_str()? != "file-path" {
                return None;
            }
            let id = msg.get("id")?.as_str()?.to_owned();
            let path = (|| {
                let objects = args
                    .cast::<ICoreWebView2WebMessageReceivedEventArgs2>()
                    .ok()?
                    .AdditionalObjects()
                    .ok()?;
                let mut count = 0u32;
                objects.Count(&raw mut count).ok()?;
                if count == 0 {
                    return None;
                }
                let file = objects
                    .GetValueAtIndex(0)
                    .ok()?
                    .cast::<ICoreWebView2File>()
                    .ok()?;
                let mut path = PWSTR::null();
                file.Path(&raw mut path).ok()?;
                Some(take_pwstr(path))
            })();
            Some((id, path))
        }
    }

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
    if let Some(front) = front_company_window(app) {
        builder = builder.parent(&front)?;
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

    #[test]
    fn only_the_apps_own_pages_open_another_window() {
        let trusted = vec!["http://127.0.0.1:4321".to_owned()];
        let new = |s: &str| opens_company_window(&Url::parse(s).unwrap(), &trusted);
        assert!(new("http://127.0.0.1:4321/c/abc/home"));
        assert!(new("http://127.0.0.1:4321/companies?fixtures=1"));
        assert!(!new("blob:http://127.0.0.1:4321/5d1f"));
        assert!(!new("about:blank"));
        assert!(!new("https://example.com/"));
        assert!(!new("http://127.0.0.1:9999/c/abc/home"));
    }

    #[test]
    fn company_windows_are_main_and_the_extra_ones() {
        assert!(is_company_window(MAIN));
        assert!(is_company_window(&next_extra_label()));
        assert!(!is_company_window(SETTINGS));
    }
}
