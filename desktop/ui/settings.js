// @ts-check
// The desktop shell's settings page: first-run chooser and settings. Plain JS
// (no build step), served by Tauri from desktop/ui. Talks to the Rust side
// only through the commands in desktop/src-tauri/src/commands.rs.

/**
 * @typedef {{ origin: string, key_id: string, name: string, public_key: string }} DeviceEnrollment
 * @typedef {{
 *   version: number,
 *   configured: boolean,
 *   mode: 'local' | 'remote',
 *   local: { data_dir: string | null },
 *   remote: { url: string | null, allow_insecure_http: boolean, device: DeviceEnrollment | null },
 *   network: { enabled: boolean, bind: string, port: number },
 *   updates: { check_automatically: boolean, install_automatically: boolean, check_interval_hours: number },
 * }} Settings
 * @typedef {{ current: string, version: string, notes: string | null, date: string | null, downloaded: boolean }} UpdateInfo
 * @typedef {{
 *   running: string | null, error: string | null, network: string | null,
 *   network_error: string | null, remote_problem: string | null,
 * }} Status
 * @typedef {{
 *   settings: Settings,
 *   default_data_dir: string,
 *   data_dir: string,
 *   status: Status,
 *   version: string,
 *   os: string,
 *   network_unavailable: string | null,
 *   keychain_unavailable: string | null,
 *   updates_unavailable: string | null,
 *   update: UpdateInfo | null,
 *   computer_name: string,
 *   enrolled: boolean,
 * }} ShellView
 */

/** @type {(cmd: string, args?: Record<string, unknown>) => Promise<any>} */
// @ts-ignore -- injected by Tauri (app.withGlobalTauri)
const invoke = window.__TAURI__.core.invoke;

/** @param {string} id */
function el(id) {
  const e = document.getElementById(id);
  if (!e) throw new Error(`missing #${id}`);
  return e;
}
/** @param {string} id */
const input = (id) => /** @type {HTMLInputElement} */ (el(id));
/** @param {string} id */
const button = (id) => /** @type {HTMLButtonElement} */ (el(id));

/** @type {ShellView} */
let view;
/** The folder chosen on this page (null = default). */
let dataDir = /** @type {string | null} */ (null);

/** @param {string | null} text @param {'error' | 'info'} [kind] */
function banner(text, kind = 'error') {
  const b = el('banner');
  b.hidden = !text;
  b.textContent = text ?? '';
  b.className = kind === 'info' ? 'banner info' : 'banner';
}

/** @param {unknown} e */
const message = (e) => (typeof e === 'string' ? e : e instanceof Error ? e.message : JSON.stringify(e));

function mode() {
  const checked = /** @type {HTMLInputElement | null} */ (document.querySelector('input[name=mode]:checked'));
  return checked?.value === 'remote' ? 'remote' : 'local';
}

function showMode() {
  const m = mode();
  for (const e of document.querySelectorAll('[data-mode]')) {
    /** @type {HTMLElement} */ (e).hidden = /** @type {HTMLElement} */ (e).dataset.mode !== m;
  }
}

function showEnrolment() {
  const enrolled = view.enrolled && sameServer();
  el('enrolled').hidden = !enrolled;
  el('enrol').hidden = enrolled;
  el('enrolled-name').textContent = view.settings.remote.device?.name ?? '';
}

/** Whether the address on the page is the one the enrolment is for. */
function sameServer() {
  return (input('server-url').value.trim() || null) === (view.settings.remote.url ?? null);
}

function showDataDir() {
  input('data-dir').value = dataDir ?? view.default_data_dir;
}

/** @param {ShellView} v */
function render(v) {
  view = v;
  const s = v.settings;
  const welcome = !s.configured;
  document.title = welcome ? 'Welcome to LibreBooks' : 'LibreBooks Settings';
  el('title').textContent = document.title;
  el('version').textContent = v.version;
  button('save').textContent = welcome ? 'Start LibreBooks' : 'Save';
  button('cancel').hidden = welcome;

  for (const r of /** @type {NodeListOf<HTMLInputElement>} */ (document.querySelectorAll('input[name=mode]'))) {
    r.checked = r.value === s.mode;
  }
  dataDir = s.local.data_dir;
  el('default-dir').textContent = v.default_data_dir;
  showDataDir();

  input('server-url').value = s.remote.url ?? '';
  input('allow-insecure').checked = s.remote.allow_insecure_http;
  input('device-name').value = v.computer_name;
  const kc = el('keychain-problem');
  kc.hidden = !v.keychain_unavailable;
  kc.textContent = v.keychain_unavailable ? `The system keychain isn't available: ${v.keychain_unavailable}` : '';
  button('enrol-device').disabled = !!v.keychain_unavailable;
  showEnrolment();

  input('network-enabled').checked = s.network.enabled;
  input('network-enabled').disabled = !!v.network_unavailable && !s.network.enabled;
  input('network-bind').value = s.network.bind;
  input('network-port').value = String(s.network.port);
  const nu = el('network-unavailable');
  nu.hidden = !v.network_unavailable;
  nu.textContent = v.network_unavailable ?? '';

  input('check-updates').checked = s.updates.check_automatically;
  input('install-updates').checked = s.updates.install_automatically;
  button('check-now').disabled = !!v.updates_unavailable;
  el('update-result').textContent = v.updates_unavailable
    ? `Updates are off: ${v.updates_unavailable}.`
    : v.update
      ? `LibreBooks ${v.update.version} is available${v.update.downloaded ? ' (downloaded)' : ''}.`
      : '';
  button('install-now').hidden = !v.update;

  const st = v.status;
  el('footer-status').textContent = st.running ?? '';
  banner(
    st.error ?? st.remote_problem ?? st.network_error ?? null,
    st.error ? 'error' : 'info',
  );
  if (welcome) {
    banner('Choose where your books are. You can change this later in Settings (tray icon → Settings…).', 'info');
  }
  showMode();
}

/** @returns {Settings} */
function collect() {
  const s = structuredClone(view.settings);
  s.mode = mode();
  s.local.data_dir = dataDir;
  s.remote.url = input('server-url').value.trim() || null;
  s.remote.allow_insecure_http = input('allow-insecure').checked;
  s.network.enabled = input('network-enabled').checked;
  s.network.bind = input('network-bind').value.trim();
  s.network.port = Number(input('network-port').value);
  s.updates.check_automatically = input('check-updates').checked;
  s.updates.install_automatically = input('install-updates').checked;
  return s;
}

/**
 * Run an action with its button disabled, reporting failures in the banner.
 * @param {HTMLButtonElement} b @param {() => Promise<void>} action
 */
async function busy(b, action) {
  b.disabled = true;
  try {
    await action();
  } catch (e) {
    banner(message(e));
  } finally {
    b.disabled = false;
  }
}

for (const r of document.querySelectorAll('input[name=mode]')) r.addEventListener('change', showMode);
input('server-url').addEventListener('input', showEnrolment);

button('browse').addEventListener('click', () =>
  busy(button('browse'), async () => {
    const picked = await invoke('pick_folder', { current: dataDir ?? view.default_data_dir });
    if (picked) {
      dataDir = picked === view.default_data_dir ? null : picked;
      showDataDir();
    }
  }),
);
button('default-folder').addEventListener('click', () => {
  dataDir = null;
  showDataDir();
});
button('open-folder').addEventListener('click', () =>
  busy(button('open-folder'), () => invoke('open_data_folder')),
);

button('test-server').addEventListener('click', () =>
  busy(button('test-server'), async () => {
    const out = el('server-result');
    out.className = 'dim';
    out.textContent = 'Checking…';
    try {
      const h = await invoke('test_server', {
        url: input('server-url').value,
        allowInsecureHttp: input('allow-insecure').checked,
      });
      out.textContent = `LibreBooks ${h.version} (${h.mode}) answered.`;
    } catch (e) {
      out.className = 'error';
      out.textContent = message(e);
    }
  }),
);

button('enrol-device').addEventListener('click', () =>
  busy(button('enrol-device'), async () => {
    banner(null);
    const v = await invoke('enroll_device', {
      url: input('server-url').value.trim() || null,
      enrollment: input('enrol-token').value,
      name: input('device-name').value,
      allowInsecureHttp: input('allow-insecure').checked,
    });
    input('enrol-token').value = '';
    // Keep unsaved edits elsewhere on the page; take the enrolment and server.
    const pending = collect();
    render(v);
    pending.remote = v.settings.remote;
    applyUnsaved(pending);
    banner(`Connected as ${v.settings.remote.device?.name}.`, 'info');
  }),
);

button('forget-device').addEventListener('click', () =>
  busy(button('forget-device'), async () => {
    const pending = collect();
    const v = await invoke('forget_device');
    render(v);
    pending.remote = v.settings.remote;
    applyUnsaved(pending);
  }),
);

/** Put back edits the user made but hasn't saved. @param {Settings} s */
function applyUnsaved(s) {
  for (const r of /** @type {NodeListOf<HTMLInputElement>} */ (document.querySelectorAll('input[name=mode]'))) {
    r.checked = r.value === s.mode;
  }
  dataDir = s.local.data_dir;
  showDataDir();
  input('network-enabled').checked = s.network.enabled;
  input('check-updates').checked = s.updates.check_automatically;
  input('install-updates').checked = s.updates.install_automatically;
  showMode();
  showEnrolment();
}

button('check-now').addEventListener('click', () =>
  busy(button('check-now'), async () => {
    const out = el('update-result');
    out.textContent = 'Checking…';
    try {
      /** @type {UpdateInfo | null} */
      const u = await invoke('check_for_updates');
      out.textContent = u ? `LibreBooks ${u.version} is available.` : `LibreBooks ${view.version} is the latest version.`;
      button('install-now').hidden = !u;
    } catch (e) {
      out.textContent = message(e);
    }
  }),
);
button('install-now').addEventListener('click', () =>
  busy(button('install-now'), async () => {
    el('update-result').textContent = 'Downloading and installing… LibreBooks will restart.';
    await invoke('install_update');
  }),
);

button('cancel').addEventListener('click', () => window.close());

el('form').addEventListener('submit', (e) => {
  e.preventDefault();
  void busy(button('save'), async () => {
    banner(null);
    const welcome = !view.settings.configured;
    const v = await invoke('save_settings', { settings: collect() });
    if (welcome) return; // the window closes once the app is up
    render(v);
    if (!v.status.error && !v.status.remote_problem && !v.status.network_error) window.close();
  });
});

invoke('get_shell').then(render, (e) => banner(message(e)));
