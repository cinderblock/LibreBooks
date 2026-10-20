//! QuickBooks Desktop import on the server: each company's import settings
//! (which transport, how to reach it), one background job at a time per
//! company (a sync or a parity check), and the Web Connector sessions that
//! feed a QBWC job.
//!
//! Three transports share the engine (`lb_qbd::sync`, `lb_qbd::parity`):
//!
//! - **Bridge**: the QB Bridge's REST API; the key lives in the secret store as
//!   `qbd.<company>.bridge-key`.
//! - **Web Connector**: the job parks its requests in a queue; the Web
//!   Connector, polling `/qbwc` with the company's user name and the password
//!   kept as `qbd.<company>.qbwc-password`, collects and answers them.
//! - **SDK**: a COM session with the QuickBooks on this machine (Windows, feature
//!   `qbsdk`); the desktop app on the QuickBooks PC uses this.
//!
//! The job runs on a blocking thread (the engine writes SQLite as it goes), with
//! progress in a watch channel and the finished report both in memory and under
//! `<data dir>/imports/`, as the CLI writes it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use age::secrecy::ExposeSecret;
use axum::http::StatusCode;
use jiff::Timestamp;
use lb_core::CompanyId;
use lb_qbd::client::{ListQuery, QbdSource, ReportQuery};
use lb_qbd::config::{ImportSettings, QbdTransport, SETTING_KEY};
use lb_qbd::qbwc::{
    AuthOutcome, QueueConfig, QueueHandle, QueuedSource, QwcFile, Session, SessionConfig,
    queued_source,
};
use lb_qbd::{
    BridgeClient, BridgeConfig, ParityOptions, ParityReport, QbdEntity, QbdError, SyncOptions,
    SyncProgress, SyncReport,
};
use lb_store::Books;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use subtle::ConstantTimeEq;
use tokio::sync::Notify;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// The folder under the data dir where reports are kept.
pub const REPORTS_DIR: &str = "imports";

fn secret_name(company: CompanyId, what: &str) -> String {
    format!("qbd.{company}.{what}")
}

/// The secret the bridge key is kept under.
pub fn bridge_key_name(company: CompanyId) -> String {
    secret_name(company, "bridge-key")
}

/// The secret the Web Connector password is kept under.
pub fn qbwc_password_name(company: CompanyId) -> String {
    secret_name(company, "qbwc-password")
}

/// Is the SDK transport compiled into this build (Windows with `qbsdk`)?
pub const fn sdk_available() -> bool {
    cfg!(all(windows, feature = "qbsdk"))
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// A company's settings, read from its file. A company that has none yet gets
/// defaults, saved at once so the `.qwc` ids they mint stay the same.
pub fn settings_of(books: &Books, company: CompanyId) -> ApiResult<ImportSettings> {
    let stored = books.read(|c| lb_store::company::setting(c, SETTING_KEY))?;
    if let Some(json) = stored {
        return Ok(serde_json::from_str(&json).unwrap_or_else(|e| {
            tracing::warn!(%company, error = %e, "unreadable import settings; using defaults");
            ImportSettings::new_for(&company.to_string())
        }));
    }
    let fresh = ImportSettings::new_for(&company.to_string());
    save_settings(books, &fresh)?;
    Ok(fresh)
}

/// Save a company's settings.
pub fn save_settings(books: &Books, settings: &ImportSettings) -> ApiResult<()> {
    let json = serde_json::to_string(settings).map_err(|e| ApiError::internal(e.to_string()))?;
    books.write(&lb_store::Actor::system(), |w| {
        lb_store::company::set_setting(w, SETTING_KEY, &json)
    })?;
    Ok(())
}

/// Which other company already uses this Web Connector user name, if any.
pub fn user_name_taken(state: &AppState, company: CompanyId, user_name: &str) -> ApiResult<bool> {
    for other in state.companies.list()? {
        if other.id == company {
            continue;
        }
        let books = state.companies.get(other.id)?;
        if settings_of(&books, other.id)?.qbwc.user_name == user_name {
            return Ok(true);
        }
    }
    Ok(false)
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

/// What a job does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobKind {
    Sync,
    Parity,
}

/// A job in progress, as the status shows it.
#[derive(Clone, Debug, Serialize)]
pub struct RunningJob {
    pub kind: JobKind,
    pub transport: QbdTransport,
    pub started_at: Timestamp,
    pub progress: Option<SyncProgress>,
    /// 1–99, what the Web Connector is told.
    pub percent: u8,
    /// A Web Connector job that is waiting for the Web Connector to poll.
    pub waiting_for_web_connector: bool,
    /// A Web Connector session is open right now.
    pub web_connector_connected: bool,
}

/// Everything `GET …/import/qbd` shows.
#[derive(Clone, Debug, Serialize)]
pub struct ImportStatus {
    pub settings: ImportSettings,
    /// The bridge key and Web Connector password are set (never their values).
    pub bridge_key_set: bool,
    pub qbwc_password_set: bool,
    pub sdk_available: bool,
    pub running: Option<RunningJob>,
    pub last_sync: Option<SyncReport>,
    pub last_parity: Option<ParityReport>,
    /// Why the last job failed outright (a report has its own issues).
    pub last_error: Option<String>,
}

struct Running {
    kind: JobKind,
    transport: QbdTransport,
    started_at: Timestamp,
    /// The Web Connector's side of a QBWC job.
    qbwc: Option<QueueHandle>,
}

/// The open Web Connector session of a QBWC job.
struct Connected {
    ticket: String,
    session: Session,
}

#[derive(Default)]
struct Slot {
    running: Mutex<Option<Running>>,
    connected: Mutex<Option<Connected>>,
    progress: Mutex<Option<SyncProgress>>,
    last_sync: Mutex<Option<SyncReport>>,
    last_parity: Mutex<Option<ParityReport>>,
    last_error: Mutex<Option<String>>,
    loaded_from_disk: Mutex<bool>,
    changed: Notify,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Per-company job state. Lives in the server state.
#[derive(Default)]
pub struct ImportSlots {
    slots: Mutex<HashMap<CompanyId, Arc<Slot>>>,
    /// How long requests wait for the Web Connector (shorter in tests).
    queue: Mutex<QueueConfig>,
    session: Mutex<SessionConfig>,
    /// Rows per qbXML iterator page (smaller in tests, to exercise paging).
    page_size: Mutex<Option<usize>>,
}

impl std::fmt::Debug for ImportSlots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportSlots")
            .field("companies", &lock(&self.slots).len())
            .finish_non_exhaustive()
    }
}

impl ImportSlots {
    fn slot(&self, company: CompanyId) -> Arc<Slot> {
        lock(&self.slots).entry(company).or_default().clone()
    }

    /// Change the Web Connector timing (tests).
    pub fn set_timing(&self, queue: QueueConfig, session: SessionConfig) {
        *lock(&self.queue) = queue;
        *lock(&self.session) = session;
    }

    /// Rows per qbXML page (tests use a few to exercise iterators).
    pub fn set_page_size(&self, rows: usize) {
        *lock(&self.page_size) = Some(rows.max(1));
    }
}

/// Options for `POST …/sync`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct SyncBody {
    pub full: bool,
    pub sweep: bool,
    pub reimport: bool,
    pub txn_limit: Option<usize>,
    pub txn_from: Option<jiff::civil::Date>,
    pub txn_to: Option<jiff::civil::Date>,
    pub apply_credits: Option<bool>,
    pub entities: Option<Vec<QbdEntity>>,
}

impl SyncBody {
    fn options(self) -> SyncOptions {
        let defaults = SyncOptions::default();
        SyncOptions {
            entities: self.entities,
            full: self.full,
            reimport: self.reimport,
            sweep: self.sweep,
            txn_limit: self.txn_limit,
            txn_from: self.txn_from,
            txn_to: self.txn_to,
            apply_credits: self.apply_credits.unwrap_or(defaults.apply_credits),
            ..defaults
        }
    }
}

/// Options for `POST …/parity`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ParityBody {
    pub as_of: Option<jiff::civil::Date>,
    pub trial_balance: Option<bool>,
}

/// One of the three transports as a [`QbdSource`].
enum Source {
    Bridge(Box<BridgeClient>),
    Queued(QueuedSource),
    #[cfg(all(windows, feature = "qbsdk"))]
    Sdk(lb_qbd::sdk::SdkSource),
}

impl QbdSource for Source {
    async fn list(&self, entity: QbdEntity, query: &ListQuery) -> Result<Vec<Value>, QbdError> {
        match self {
            Source::Bridge(s) => s.list(entity, query).await,
            Source::Queued(s) => s.list(entity, query).await,
            #[cfg(all(windows, feature = "qbsdk"))]
            Source::Sdk(s) => s.list(entity, query).await,
        }
    }

    async fn get(&self, entity: QbdEntity, id: &str) -> Result<Value, QbdError> {
        match self {
            Source::Bridge(s) => s.get(entity, id).await,
            Source::Queued(s) => s.get(entity, id).await,
            #[cfg(all(windows, feature = "qbsdk"))]
            Source::Sdk(s) => s.get(entity, id).await,
        }
    }

    async fn company(&self) -> Result<Value, QbdError> {
        match self {
            Source::Bridge(s) => s.company().await,
            Source::Queued(s) => s.company().await,
            #[cfg(all(windows, feature = "qbsdk"))]
            Source::Sdk(s) => s.company().await,
        }
    }

    async fn preferences(&self) -> Result<Value, QbdError> {
        match self {
            Source::Bridge(s) => s.preferences().await,
            Source::Queued(s) => s.preferences().await,
            #[cfg(all(windows, feature = "qbsdk"))]
            Source::Sdk(s) => s.preferences().await,
        }
    }

    async fn report(&self, slug: &str, query: &ReportQuery) -> Result<Value, QbdError> {
        match self {
            Source::Bridge(s) => s.report(slug, query).await,
            Source::Queued(s) => s.report(slug, query).await,
            #[cfg(all(windows, feature = "qbsdk"))]
            Source::Sdk(s) => s.report(slug, query).await,
        }
    }

    fn requests_made(&self) -> Option<u64> {
        match self {
            Source::Bridge(s) => s.requests_made(),
            Source::Queued(s) => s.requests_made(),
            #[cfg(all(windows, feature = "qbsdk"))]
            Source::Sdk(s) => s.requests_made(),
        }
    }
}

impl From<QbdError> for ApiError {
    fn from(e: QbdError) -> Self {
        let message = e.to_string();
        match e {
            QbdError::BadUrl(_) | QbdError::Unsupported(_) => ApiError::invalid(message),
            QbdError::Unreachable { .. } | QbdError::Timeout { .. } | QbdError::Session { .. } => {
                ApiError::new(StatusCode::BAD_GATEWAY, "network", message)
            }
            _ => ApiError::new(StatusCode::BAD_GATEWAY, "provider", message),
        }
    }
}

/// Build the source the settings ask for. The Web Connector's handle comes
/// back too, for the SOAP endpoint to drive.
async fn make_source(
    state: &AppState,
    company: CompanyId,
    settings: &ImportSettings,
) -> ApiResult<(Source, Option<QueueHandle>)> {
    let problems = settings.problems();
    if !problems.is_empty() {
        return Err(ApiError::invalid(problems.join("; "))
            .with_detail(serde_json::json!({ "problems": problems })));
    }
    match settings.transport {
        QbdTransport::Bridge => {
            let reference = settings
                .bridge
                .key_ref
                .clone()
                .unwrap_or_else(|| format!("secret:{}", bridge_key_name(company)));
            let store = state.ops.secrets.clone();
            let key = tokio::task::spawn_blocking(move || store.resolve(&reference))
                .await?
                .map_err(|e| ApiError::invalid(format!("the bridge key can't be read: {e}")))?;
            let mut config = BridgeConfig::new(&settings.bridge.url, key.expose_secret())?;
            config.politeness_delay = Duration::from_millis(settings.bridge.delay_ms);
            Ok((Source::Bridge(Box::new(BridgeClient::new(config)?)), None))
        }
        QbdTransport::Qbwc => {
            let config = *lock(&state.imports.queue);
            let (mut source, handle) = queued_source(config);
            if let Some(rows) = *lock(&state.imports.page_size) {
                source = source.with_page_size(rows);
            }
            Ok((Source::Queued(source), Some(handle)))
        }
        QbdTransport::Sdk => {
            #[cfg(all(windows, feature = "qbsdk"))]
            {
                let config = lb_qbd::sdk::SdkConfig {
                    company_file: settings.company_file.clone(),
                    file_mode: settings.sdk.file_mode,
                    ..lb_qbd::sdk::SdkConfig::default()
                };
                // Opening blocks on QuickBooks (and on the user's answer to its
                // authorisation dialog the first time).
                let source =
                    tokio::task::spawn_blocking(move || lb_qbd::sdk::sdk_source(&config)).await??;
                Ok((Source::Sdk(source), None))
            }
            #[cfg(not(all(windows, feature = "qbsdk")))]
            {
                Err(ApiError::new(
                    StatusCode::NOT_IMPLEMENTED,
                    "unsupported",
                    "the QuickBooks SDK transport needs LibreBooks running on the Windows machine that has QuickBooks (this build has no SDK support)",
                ))
            }
        }
    }
}

fn reports_dir(state: &AppState) -> PathBuf {
    state.ops.data_dir().join(REPORTS_DIR)
}

fn write_report(dir: &PathBuf, company: CompanyId, kind: JobKind, report: &impl Serialize) {
    let stamp = Timestamp::now().strftime("%Y%m%dT%H%M%SZ").to_string();
    let suffix = match kind {
        JobKind::Sync => "",
        JobKind::Parity => "-parity",
    };
    let path = dir.join(format!("{company}{suffix}-{stamp}.json"));
    let result = std::fs::create_dir_all(dir).and_then(|()| {
        let bytes = serde_json::to_vec_pretty(report)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&path, bytes)
    });
    if let Err(e) = result {
        tracing::warn!(%company, error = %e, "couldn't write the import report");
    }
}

/// The newest report of `kind` for `company` on disk.
fn latest_report<T: for<'de> Deserialize<'de>>(
    dir: &PathBuf,
    company: CompanyId,
    kind: JobKind,
) -> Option<T> {
    let prefix = match kind {
        JobKind::Sync => format!("{company}-"),
        JobKind::Parity => format!("{company}-parity-"),
    };
    let mut newest: Option<PathBuf> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_sync_of_parity = kind == JobKind::Sync && name.contains("-parity-");
        let is_json = std::path::Path::new(&name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"));
        if name.starts_with(&prefix) && is_json && !is_sync_of_parity {
            let path = entry.path();
            if newest.as_ref().is_none_or(|n| path > *n) {
                newest = Some(path);
            }
        }
    }
    let text = std::fs::read_to_string(newest?).ok()?;
    serde_json::from_str(&text).ok()
}

impl Slot {
    fn load_from_disk(&self, dir: &PathBuf, company: CompanyId) {
        let mut loaded = lock(&self.loaded_from_disk);
        if *loaded {
            return;
        }
        *loaded = true;
        if lock(&self.last_sync).is_none() {
            *lock(&self.last_sync) = latest_report(dir, company, JobKind::Sync);
        }
        if lock(&self.last_parity).is_none() {
            *lock(&self.last_parity) = latest_report(dir, company, JobKind::Parity);
        }
    }

    fn running_view(&self) -> Option<RunningJob> {
        let running = lock(&self.running);
        let r = running.as_ref()?;
        let connected = lock(&self.connected).is_some();
        Some(RunningJob {
            kind: r.kind,
            transport: r.transport,
            started_at: r.started_at,
            progress: lock(&self.progress).clone(),
            percent: r.qbwc.as_ref().map_or(0, QueueHandle::percent),
            waiting_for_web_connector: r.qbwc.as_ref().is_some_and(|h| !connected && h.has_work()),
            web_connector_connected: connected,
        })
    }

    fn finish(
        &self,
        kind: JobKind,
        outcome: Result<Value, String>,
        dir: &PathBuf,
        company: CompanyId,
    ) {
        match outcome {
            Ok(report) => {
                match kind {
                    JobKind::Sync => {
                        if let Ok(r) = serde_json::from_value::<SyncReport>(report.clone()) {
                            write_report(dir, company, kind, &r);
                            *lock(&self.last_sync) = Some(r);
                        }
                    }
                    JobKind::Parity => {
                        if let Ok(r) = serde_json::from_value::<ParityReport>(report.clone()) {
                            write_report(dir, company, kind, &r);
                            *lock(&self.last_parity) = Some(r);
                        }
                    }
                }
                *lock(&self.last_error) = None;
            }
            Err(message) => {
                tracing::warn!(%company, ?kind, error = %message, "import job failed");
                *lock(&self.last_error) = Some(message);
            }
        }
        lock(&self.running).take();
        lock(&self.connected).take();
        lock(&self.progress).take();
        self.changed.notify_waiters();
    }
}

/// Start a sync or parity job. 409 while one runs.
async fn start(
    state: &AppState,
    company: CompanyId,
    books: Books,
    kind: JobKind,
    sync_options: SyncOptions,
    parity_options: ParityOptions,
) -> ApiResult<()> {
    let settings = settings_of(&books, company)?;
    let slot = state.imports.slot(company);
    {
        let running = lock(&slot.running);
        if let Some(r) = running.as_ref() {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "running",
                format!("a {:?} job is already running for this company", r.kind),
            ));
        }
    }
    let (source, qbwc) = make_source(state, company, &settings).await?;
    {
        let mut running = lock(&slot.running);
        if running.is_some() {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "running",
                "a job is already running for this company",
            ));
        }
        *running = Some(Running {
            kind,
            transport: settings.transport,
            started_at: Timestamp::now(),
            qbwc: qbwc.clone(),
        });
        lock(&slot.progress).take();
        *lock(&slot.last_error) = None;
    }
    let dir = reports_dir(state);
    let handle = tokio::runtime::Handle::current();
    let slot_for_job = slot.clone();
    // The engine writes SQLite as it imports: keep it off the request threads.
    tokio::task::spawn_blocking(move || {
        let progress_slot = slot_for_job.clone();
        let progress = move |p: &SyncProgress| {
            *lock(&progress_slot.progress) = Some(p.clone());
            if let Some(h) = &qbwc {
                h.set_progress(p);
            }
        };
        let outcome = handle.block_on(async {
            match kind {
                JobKind::Sync => lb_qbd::sync(&books, &source, &sync_options, &progress)
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
                JobKind::Parity => lb_qbd::parity(&books, &source, &parity_options)
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
            }
        });
        // Dropping the source finishes a Web Connector queue (no more requests).
        drop(source);
        slot_for_job.finish(kind, outcome, &dir, company);
    });
    Ok(())
}

/// `POST …/sync`.
pub async fn start_sync(
    state: &AppState,
    company: CompanyId,
    books: Books,
    body: SyncBody,
) -> ApiResult<()> {
    start(
        state,
        company,
        books,
        JobKind::Sync,
        body.options(),
        ParityOptions::default(),
    )
    .await
}

/// `POST …/parity`.
pub async fn start_parity(
    state: &AppState,
    company: CompanyId,
    books: Books,
    body: ParityBody,
) -> ApiResult<()> {
    start(
        state,
        company,
        books,
        JobKind::Parity,
        SyncOptions::default(),
        ParityOptions {
            as_of: body.as_of,
            trial_balance: body.trial_balance.unwrap_or(true),
        },
    )
    .await
}

/// Wait up to `wait` for the running job (if any) to finish.
pub async fn wait_for_idle(state: &AppState, company: CompanyId, wait: Duration) {
    let slot = state.imports.slot(company);
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let changed = slot.changed.notified();
        if lock(&slot.running).is_none() {
            return;
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return;
        }
        tokio::select! {
            () = changed => {}
            () = tokio::time::sleep_until(deadline) => return,
        }
    }
}

/// The status view.
pub async fn status(
    state: &AppState,
    company: CompanyId,
    books: &Books,
) -> ApiResult<ImportStatus> {
    let settings = settings_of(books, company)?;
    let slot = state.imports.slot(company);
    let dir = reports_dir(state);
    let store = state.ops.secrets.clone();
    let for_check = settings.clone();
    let (key_set, password_set) = tokio::task::spawn_blocking(move || {
        let key = settings_key_set(&store, company, &for_check);
        let pw = store
            .contains(&qbwc_password_name(company))
            .unwrap_or(false);
        (key, pw)
    })
    .await?;
    {
        let slot = slot.clone();
        tokio::task::spawn_blocking(move || slot.load_from_disk(&dir, company)).await?;
    }
    Ok(ImportStatus {
        settings,
        bridge_key_set: key_set,
        qbwc_password_set: password_set,
        sdk_available: sdk_available(),
        running: slot.running_view(),
        last_sync: lock(&slot.last_sync).clone(),
        last_parity: lock(&slot.last_parity).clone(),
        last_error: lock(&slot.last_error).clone(),
    })
}

fn settings_key_set(
    store: &crate::secrets::SecretStore,
    company: CompanyId,
    s: &ImportSettings,
) -> bool {
    match &s.bridge.key_ref {
        Some(r) => store.check_reference(r).is_ok(),
        None => store.contains(&bridge_key_name(company)).unwrap_or(false),
    }
}

/// The company's `.qwc`, for the Web Connector.
pub fn qwc_file(
    state: &AppState,
    company: CompanyId,
    company_name: &str,
    settings: &ImportSettings,
    request_origin: Option<&str>,
) -> Result<QwcFile, lb_qbd::qbwc::QwcError> {
    let server = state.ops.settings();
    let base = settings
        .qbwc
        .app_url
        .clone()
        .or_else(|| server.public_urls.first().cloned())
        .or_else(|| server.tunnel.public_url())
        .or_else(|| request_origin.map(str::to_owned))
        .unwrap_or_default();
    let base = base.trim_end_matches('/');
    let app_url = if base.ends_with("/qbwc") {
        base.to_owned()
    } else {
        format!("{base}/qbwc")
    };
    let file = QwcFile {
        app_name: format!("LibreBooks ({company_name})"),
        app_id: String::new(),
        app_support: QwcFile::support_url_for(&app_url),
        app_url,
        app_description: format!(
            "Read-only import of {company_name} into LibreBooks (company {company})"
        ),
        user_name: settings.qbwc.user_name.clone(),
        owner_id: settings.qbwc.owner_id,
        file_id: settings.qbwc.file_id,
        run_every_minutes: settings.qbwc.run_every_minutes,
        read_only: true,
    };
    file.validate()?;
    Ok(file)
}

// ---------------------------------------------------------------------------
// The Web Connector's side
// ---------------------------------------------------------------------------

fn ticket() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}

/// A fresh Web Connector password: 32 hex characters.
pub fn new_password() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}

/// `authenticate`: find the company by user name, check the password in
/// constant time, and say whether there's work. A ticket is always returned
/// (the Web Connector expects one even for "none" and "nvu").
pub async fn authenticate(
    state: &AppState,
    user_name: &str,
    password: &str,
) -> (String, AuthOutcome) {
    let ticket = ticket();
    let found = match find_company_by_user(state, user_name) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e.message, "Web Connector authenticate: couldn't read companies");
            None
        }
    };
    let Some((company, settings)) = found else {
        tracing::info!(user_name, "Web Connector sign-in refused: unknown user");
        return (ticket, AuthOutcome::Invalid);
    };
    let store = state.ops.secrets.clone();
    let name = qbwc_password_name(company);
    let stored = tokio::task::spawn_blocking(move || store.get(&name))
        .await
        .ok()
        .and_then(Result::ok)
        .flatten();
    let Some(stored) = stored else {
        tracing::warn!(%company, "Web Connector sign-in refused: no password set for this company");
        return (ticket, AuthOutcome::Invalid);
    };
    let ok: bool = stored
        .expose_secret()
        .as_bytes()
        .ct_eq(password.as_bytes())
        .into();
    if !ok {
        tracing::warn!(%company, "Web Connector sign-in refused: wrong password");
        return (ticket, AuthOutcome::Invalid);
    }
    let slot = state.imports.slot(company);
    let handle = lock(&slot.running).as_ref().and_then(|r| r.qbwc.clone());
    let Some(handle) = handle else {
        return (ticket, AuthOutcome::NothingToDo);
    };
    if handle.is_finished() || handle.is_closed() {
        return (ticket, AuthOutcome::NothingToDo);
    }
    let mut connected = lock(&slot.connected);
    if connected.is_some() {
        // Another Web Connector run is already in this session; let it finish.
        return (ticket, AuthOutcome::Busy);
    }
    let session = Session::new(handle, *lock(&state.imports.session));
    *connected = Some(Connected {
        ticket: ticket.clone(),
        session,
    });
    tracing::info!(%company, "Web Connector session opened");
    (
        ticket,
        AuthOutcome::Work {
            company_file: settings.company_file.clone(),
        },
    )
}

fn find_company_by_user(
    state: &AppState,
    user_name: &str,
) -> ApiResult<Option<(CompanyId, ImportSettings)>> {
    if user_name.trim().is_empty() {
        return Ok(None);
    }
    for c in state.companies.list()? {
        let books = state.companies.get(c.id)?;
        let settings = settings_of(&books, c.id)?;
        if settings.transport == QbdTransport::Qbwc && settings.qbwc.user_name == user_name {
            return Ok(Some((c.id, settings)));
        }
    }
    Ok(None)
}

/// The session a ticket belongs to.
pub fn session_for(state: &AppState, ticket: &str) -> Option<Session> {
    if ticket.is_empty() {
        return None;
    }
    let slots: Vec<Arc<Slot>> = lock(&state.imports.slots).values().cloned().collect();
    for slot in slots {
        let connected = lock(&slot.connected);
        if let Some(c) = connected.as_ref()
            && c.ticket.as_bytes().ct_eq(ticket.as_bytes()).into()
        {
            return Some(c.session.clone());
        }
    }
    None
}

/// `closeConnection`: forget the session (the job itself finishes on its own).
pub fn close_session(state: &AppState, ticket: &str) {
    let slots: Vec<Arc<Slot>> = lock(&state.imports.slots).values().cloned().collect();
    for slot in slots {
        let mut connected = lock(&slot.connected);
        if connected.as_ref().is_some_and(|c| c.ticket == ticket) {
            connected.take();
        }
    }
}
