# Desktop app (Tauri 2)

## Goal

`desktop/`: the LibreBooks desktop app. User requirements: a Tauri desktop app
that holds the books locally, can also serve them to the network, can instead be
a thin client to a remote LibreBooks server, updates itself (opt-in automatic
install), and runs on Windows and Linux (macOS eventually). See
`plans/librebooks.md` decisions 1, 8, 9, 13 and `plans/api.md`.

## Environment / context

- Crate `desktop/src-tauri`, package `librebooks-desktop`, binary `LibreBooks`.
  Workspace member, **not** a default member (headless builds never compile a
  webview). Tauri **2** stable (2.12; 3.0 is alpha, not used).
- Plugins: updater, single-instance, window-state, dialog, opener, log, process.
- Shell UI (first-run chooser + settings): `desktop/ui/` — plain HTML + CSS +
  JS (`// @ts-check`), no build step; it is Tauri's `frontendDist`.
- Main window: loads the SPA from a loopback HTTP server inside the process
  (lb-server in local mode, the thin-client proxy in remote mode).
- Settings file: `<app config dir>/settings.json`
  (Windows `%APPDATA%\org.librebooks.desktop\settings.json`,
  Linux `~/.config/org.librebooks.desktop/settings.json`).
- Default company folder: `directories::ProjectDirs("org","LibreBooks","LibreBooks").data_dir()`
  (Windows `%APPDATA%\LibreBooks\LibreBooks\data`, Linux `~/.local/share/librebooks`),
  the same as `librebooks serve`.
- Logs: `<app log dir>/LibreBooks.log` (Windows `%LOCALAPPDATA%\org.librebooks.desktop\logs`)
  via tauri-plugin-log; lb-server's `tracing` events reach it through
  `tracing`'s `log` feature.
- Keychain: service `org.librebooks.desktop`, account `device-key <origin>`
  (`keyring` 4, default `v1` feature: Windows Credential Manager, macOS
  Keychain, Linux Secret Service over zbus — no libdbus needed).

## Module map (`desktop/src-tauri/src`)

| File | What |
| --- | --- |
| `lib.rs` | `Shell` (managed state), `activate` (start backend + main window), run loop, exit/shutdown, macOS menu |
| `settings.rs` | `Settings` JSON model, load (damaged file set aside) / atomic save, validation, server URL parsing |
| `backend.rs` | `Backend::start/stop`: local lb-server or thin-client proxy on `127.0.0.1:<port>`, per-start token, graceful stop |
| `proxy.rs` | Thin-client reverse proxy (`/api/*` → remote, embedded SPA otherwise) |
| `device.rs` | Device key (Ed25519, keychain), enrolment, login handshake (marked unverified) |
| `network.rs` | "Serve to the network" plumbing; refuses until passkey sessions exist |
| `window.rs` | Main window (init script, navigation lock, WebView2 accelerator keys), settings window, debug self-test |
| `tray.rs` | Tray icon and menu |
| `updater.rs` | Update checks, prompt, background download, install on quit |
| `commands.rs` | Tauri commands for the settings page (+ `open_settings` for the SPA) |

## Contract with the web UI (`window.__LIBREBOOKS__`)

In the desktop app, before any page script runs, the main window defines a
frozen, non-enumerable global — **only** when `location.origin` is the app's own
loopback origin (or the Vite dev server in debug builds):

```ts
interface LibreBooksHost {
  mode: 'desktop';
  backend: 'local' | 'remote';   // books here, or thin client
  server: string | null;         // remote server base URL (thin client), else null
  token: string;                 // per-start secret (64 hex chars)
  apiBase: '/api/v1';            // same origin; always relative
  keyboard: 'native';            // Ctrl+W/T/N, Ctrl+R/F/P, F-keys reach the page: use QBD's real keys
  shell: {
    version: string;             // desktop app version
    os: 'windows' | 'linux' | 'macos';
    openSettings(): Promise<void>; // open the desktop settings window
  };
}
declare global { interface Window { __LIBREBOOKS__?: LibreBooksHost } }
```

What the SPA must do when `window.__LIBREBOOKS__` exists:

- Send `Authorization: Bearer <token>` on every API request (fetch). The
  server's CSRF guard accepts bearer requests without `X-LibreBooks`, but
  sending it anyway is harmless.
- `EventSource` can't set headers: append `&token=<token>` to
  `/api/v1/events?company=…`. (Both lb-server's `LocalToken` mode and the proxy
  accept `token=` in the query.)
- Skip the sign-in screens (no passkeys in the desktop app). A `401` means the
  thin client can't sign in; the error body's `error.detail.reason` says why:
  - `device_not_enrolled`: this computer isn't connected to the server yet;
  - `device_login_failed`: the server refused the device key;
  - and `502` with `error.code = "unreachable"`, `detail.reason = "remote_unreachable"`.
  In each case show the message and offer `__LIBREBOOKS__.shell.openSettings()`.
- Use QBD's real shortcuts (`keyboard === 'native'`), not the browser
  alternates (`plans/librebooks.md` decision 13). The webview does not act on
  them; `preventDefault` is still good practice.
- The native context menu is suppressed in release builds except over
  editable fields and selected text; the SPA may show its own.
- External links (`https:` to other origins, `mailto:`) open in the system
  browser automatically; `target=_blank`/`window.open` never opens a webview
  window.
- HTML5 drag-and-drop of files works (Tauri's own drop handler is off).

Without `__LIBREBOOKS__` the SPA is in a browser and uses cookie sessions.

## Decisions already made (don't re-ask)

1. **The SPA loads from a loopback server in this process, not from Tauri's
   asset protocol.** Same-origin API calls, no CORS, the same SPA code path as
   the headless server and the browser. lb-server already embeds `web/dist`
   (rust-embed), so the SPA is not embedded a second time.
2. **Tauri's `frontendDist` is the desktop shell's own UI (`desktop/ui`)**, not
   `web/dist`, and there is no `devUrl`. Deviation from the brief, for a
   concrete reason: the SPA never loads from the Tauri origin (decision 1), so
   pointing `frontendDist` at `web/dist` would embed a dead second copy; the
   Tauri origin is better used for the chooser/settings page that needs IPC.
   Web-UI hot reload inside the desktop shell still works (see "Dev").
3. **Local mode**: lb-server in-process on Tauri's tokio runtime, bound to
   `127.0.0.1:0`, `Mode::Desktop`, `AuthMode::LocalToken(<256-bit random,
   new each backend start>)`.
4. **Token reaches the SPA by an initialization script** (top frame only) that
   only acts when `location.origin` is trusted. The main window refuses to
   navigate anywhere else (`on_navigation`; external URLs go to the system
   browser), and `window.open` is always denied, so remote content never runs
   in it.
5. **Thin client = loopback reverse proxy.** The webview loads the SPA embedded
   in the desktop binary from a loopback server; `/api/*` is proxied to the
   remote with the session the desktop obtained by device-key login. The
   webview never sees remote credentials; the proxy requires the same
   per-start token and checks `Host` (DNS rebinding). Remote responses with
   script-capable content types get `CSP: sandbox`. Trade-off accepted: the
   SPA is the desktop's version, not the server's (`/api/v1` must stay
   additive; the SPA can compare `/health`'s version with
   `shell.version`). Proxying the server's own UI instead is a possible later
   option.
6. **Device key**: Ed25519; the 32-byte seed lives in the OS keychain; the
   server-assigned `key_id` and the origin are in settings (not secret). The
   key is stored in the keychain *before* enrolling and removed again if the
   server refuses. Login message per `plans/auth.md` decision 9:
   `b"librebooks device login v1\0" || challenge(32) || origin`.
7. **Mode / folder / server changes restart the backend in-process** and
   rebuild the main window (new origin, new token, new init script). No app
   relaunch.
8. **Updates**: check automatically is on by default; install automatically is
   **off** by default. When a scheduled check finds an update: if auto-install
   is off, a dialog offers "Install and restart" / "Later" (once per version
   per session; the tray keeps an "Update to X…" item); if on, it downloads and
   verifies in the background and installs when the app quits (or right away
   from the tray's "Restart to update"), never restarting the app under
   someone mid-entry. `requireSignedVersion: true` (no downgrade via a crafted
   `latest.json`; there are no legacy releases to worry about). Debug builds
   never update (override: `LIBREBOOKS_UPDATER_IN_DEBUG=1`).
9. **Default company folder = the CLI's**, so `librebooks serve` and the
   desktop app see the same books. Deviation from "the app's data dir" in the
   brief: one place for a user's books however they run LibreBooks.
   Changeable in settings.
10. **Closing the main window quits** (as QuickBooks does), unless serving to
    the network, in which case it hides to the tray.
11. **WebView2 browser accelerator keys are off in every build** (QBD's Ctrl+R,
    Ctrl+F, Ctrl+P, F5… must reach the SPA in dev too). Devtools in debug builds:
    tray → "Developer tools". This needs a few lines of COM (`unsafe`, one
    small `#[allow(unsafe_code)]` module) because Tauri doesn't expose the
    setting. Tauri adds no Ctrl+W on Windows/Linux; the macOS menu omits Close
    Window.

## Serve to the network: design (refused until passkeys)

One `AppState` (one set of open company files, one event bus), **two
listeners**, different auth per listener:

| Listener | Who | Auth |
| --- | --- | --- |
| `127.0.0.1:<random>` | this app's webview | `AuthMode::LocalToken` |
| `<bind>:<port>` (default `0.0.0.0:8740`) | phones, other PCs | passkey / device-key sessions |

Two states would open every company file twice and split the event bus. The
settings page shows the toggle disabled with the reason; `Settings::validate`
refuses `network.enabled`; `network::start` refuses too; and
`lb_server::check_bind` refuses `AuthMode::Open` beyond loopback as a last line.

### Needed from lb-server (not changed by this branch)

```rust
/// The application with requests authenticated by `auth` instead of the state's own mode.
pub fn router_with_auth(state: AppState, auth: AuthMode) -> Router;
```

Implementation sketch: a layer inserting `Extension(ListenerAuth(auth))`; the
`Principal` extractor uses `parts.extensions.get::<ListenerAuth>()` when present,
else `state.auth`. Plus an `AuthMode` variant for passkey/device sessions. Then
`network::network_auth` returns that mode and `network::network_router` returns
`Ok(lb_server::router_with_auth(state.clone(), auth))` — both marked in
`network.rs`.

## Thin client: the device-key handshake

Implemented in `device::login` to the contract in `plans/api.md` and
`plans/auth.md`, **not yet verified against a real server** (the endpoints
didn't exist in lb-server when written). Assumptions to re-check when lb-auth is
wired into lb-server:

- `POST /api/v1/auth/device/enroll {token, public_key, name}` → `{key_id}`;
  `public_key` = base64url (no padding) of the 32-byte key.
- `POST /api/v1/auth/device/challenge` (we send `{key_id}`; harmless if
  ignored) → `{challenge_id, challenge}`, challenge = base64url of 32 bytes.
- `POST /api/v1/auth/device/login {challenge_id, key_id, signature}` (base64url
  64-byte signature) → `Set-Cookie: lb_session=…`.
- `origin` in the signed message = `scheme://host[:port]` of the server URL the
  desktop was configured with (no trailing slash).

The proxy signs in lazily on the first API call, shares one sign-in between
concurrent requests, and on a `401` with a session signs in again once and
retries (request bodies are buffered, ≤ 64 MiB, to allow the retry). A
`Set-Cookie` refreshing `lb_session` is adopted. Enrolment links are accepted as
`https://host/…/<token>` or `…?token=<token>` or a bare token; a link also fills
in the server address.

## Self-update vs `lb-update`

- Desktop: `tauri-plugin-updater` against
  `https://github.com/cinderblock/LibreBooks/releases/latest/download/latest.json`,
  signatures by the Tauri updater key (minisign format), installers per
  platform (NSIS on Windows, AppImage on Linux).
- Headless server: `lb-update` (minisign over `SHA256SUMS`, replaces its own
  binary).
- Same releases, two manifests: the release workflow builds desktop bundles
  with `cargo tauri build --config src-tauri/tauri.release.conf.json`
  (`createUpdaterArtifacts: true`, needs `TAURI_SIGNING_PRIVATE_KEY` and
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`), uploads them plus `latest.json`, and
  separately signs `SHA256SUMS` for lb-update. The two keys can be the same
  minisign key or separate ones; separate is simpler to rotate. Desktop
  bundles should not also be listed for lb-update (the desktop never uses it).
- **Before the first release**: `cargo tauri signer generate -w ~/.tauri/librebooks.key`,
  put the public key in `tauri.conf.json` → `plugins.updater.pubkey`
  (replacing `REPLACE_WITH_TAURI_UPDATER_PUBLIC_KEY`), store the private key
  and password as repository secrets. Until then the app reports "this build
  has no update signing key" and never checks.
- deb/rpm installs: the Tauri updater only replaces AppImages on Linux; users
  of the packages update through the package (documented gap).

## Dev and build

Run everything from `desktop/src-tauri` (where `tauri.conf.json` is).

- One-time: `bun install` at the repo root; `bun run build` in `web/` (lb-server
  embeds `web/dist`; debug builds read it from disk at run time, so rebuilding
  the web UI needs no Rust rebuild in dev).
- `cargo tauri dev` — the app with the built SPA from the in-process server.
- Web-UI hot reload inside the app: in one terminal
  `LIBREBOOKS_DESKTOP_PORT=8740 cargo tauri dev` (fixed loopback port, which
  `web/vite.config.ts` already proxies `/api` to), plus
  `LIBREBOOKS_WEB_DEV_URL=http://localhost:5740`; in another, `bun run dev` in
  `web/`. The token is injected for the Vite origin too (debug builds only).
- `LIBREBOOKS_SELFTEST=1` (debug): after the SPA loads, the page itself calls
  `/api/v1/session` with and without the token and the result is logged
  (`selftest: backend=local session=200 without-token=401 …`).
- Release: `cargo tauri build` (all configured bundles for the host), e.g.
  `cargo tauri build --bundles nsis`. `beforeBuildCommand` runs
  `bun run build` in `web/` first.
- All cargo/tauri builds on the shared dev machine go through
  `node ~/.claude/bin/cpu-slots.mjs run --slots 4 --label "librebooks desktop" -- …`.

### Linux build dependencies

Debian/Ubuntu:

```sh
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

Fedora: `webkit2gtk4.1-devel openssl-devel curl wget file libappindicator-gtk3-devel librsvg2-devel libxdo-devel`
plus `rpm-build` for rpm bundles. Runtime: WebKitGTK 4.1, and a Secret Service
(GNOME Keyring or KWallet) for thin-client device keys.

## Plan / steps

1. Scaffold crate, workspace member, tauri.conf.json, capabilities, icons. Done.
2. Settings model + persistence; backend + main window + token injection;
   graceful shutdown. Done.
3. Tray, single instance, window state, keyboard lockdown. Done.
4. Shell UI (chooser + settings) and commands. Done.
5. Thin-client proxy + keychain device key + login handshake (unverified). Done.
6. Network listener plumbing (refused until passkeys). Done.
7. Updater. Done (untested end to end: no signing key/releases yet).
8. ▶ Clippy/tests, launch + screenshot, `cargo tauri build --bundles nsis`.

## Findings / gotchas

- The Bash tool's worktree guard rejects commands containing the worktree path
  (it has `/git/` in it) combined with other commands; use the PowerShell tool
  (cwd is the worktree) for cargo runs.
- `cargo info tauri` shows 3.0.0-alpha as latest; pin `@2`-compatible versions.
- `keyring` 4 is a thin layer over `keyring-core`; its default `v1` feature
  picks the platform store, which is what a simple app wants.
- Tauri 2.12 exposes no WebView2 `AreBrowserAcceleratorKeysEnabled`; wry has
  it internally only. Done through `with_webview` + `webview2-com` 0.39 /
  `windows-core` 0.62 (wry's versions, so the COM types are shared). The main
  window is created on `about:blank` and navigated after the setting is
  applied, so the SPA never runs with the browser keys on.
- Window labels are released only after the event loop processes the destroy;
  `activate` waits for `get_webview_window("main")` to be `None` before
  rebuilding, and the "main window gone → quit" logic lives in
  `RunEvent::WindowEvent(Destroyed)` (after Tauri drops the label), not the
  per-window handler.
- Graceful shutdown: axum's graceful shutdown waits for open SSE streams
  forever, so `Backend::stop` cuts connections after 3 s, then drops the last
  `AppState` (closing company files; warns if anything still holds them).
- Sync Tauri commands that create windows deadlock on Windows; `open_settings`
  is async.

## Progress log

- [x] Scaffold, icons (from `web/public/icon.svg` via resvg + `cargo tauri icon`)
- [x] Settings + tests
- [x] Local backend, main window, token injection, navigation lock
- [x] Thin-client proxy + device keys + tests (fake server end to end)
- [x] Network plumbing (refused)
- [x] Tray, single instance, window state, keys, context menu
- [x] Shell UI
- [x] Updater (needs a signing key to be exercised)
- [ ] Clippy clean, tests pass
- [ ] Launched, self-test, screenshot
- [ ] `cargo tauri build --bundles nsis`

## Open questions for the user

1. **Updater signing key**: generate it (`cargo tauri signer generate`) and add
   the secrets when the release workflow is written? Recommendation: yes, a key
   separate from lb-update's minisign key.
2. **Thin client UI source**: keep serving the desktop's embedded SPA against a
   remote API (current), or proxy the server's own UI to avoid version skew?
   Recommendation: keep embedded for now (trusted code only in the window).

## Things not to do

- Don't add `desktop/src-tauri` to `default-members`.
- Don't edit `web/` (another agent owns it).
- Don't use the Tauri 3.0 alphas.
- Don't point the network listener at `lb_server::router(state)`: that applies
  the local token auth mode to network clients. Wait for `router_with_auth`.
- Don't put the device key or session cookie anywhere the webview can read.
- Don't set `createUpdaterArtifacts: true` in the base config: local builds
  without the signing key would fail. It's in `tauri.release.conf.json`.
