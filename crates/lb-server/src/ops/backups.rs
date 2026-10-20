//! Backups of every company: a [`lb_backup::Scheduler`] each, and the one-off
//! operations (run now, list, verify, restore, export) the API and the CLI share.
//!
//! The server keeps one backup configuration; each company backs up into its
//! own folder of every destination ([`super::settings::company_backup_config`]).
//! A reconcile loop, every minute and whenever woken, starts a scheduler for each
//! company that has none, stops those of companies that are gone (or when backups
//! are turned off), restarts one whose company was renamed (the file names follow
//! the name), and hands changed settings to the rest.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use age::secrecy::SecretString;
use lb_backup::{
    BackupConfig, BackupEntry, BackupError, BackupRecord, Manifest, PortableManifest, Scheduler,
    SchedulerStatus, SecretResolver, Unlock, VerifyReport,
};
use lb_core::CompanyId;
use lb_store::{Actor, Books};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use super::settings::company_backup_config;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// How often the reconcile loop looks for new or removed companies.
const RECONCILE_EVERY: Duration = Duration::from_secs(60);

struct Running {
    name: String,
    config: BackupConfig,
    scheduler: Scheduler,
}

/// The schedulers, and a lock per company so manual backups don't overlap.
#[derive(Default)]
pub(crate) struct BackupSlots {
    running: Mutex<HashMap<CompanyId, Running>>,
    manual: Mutex<HashMap<CompanyId, Arc<tokio::sync::Mutex<()>>>>,
}

impl BackupSlots {
    fn running(&self) -> MutexGuard<'_, HashMap<CompanyId, Running>> {
        self.running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn manual_lock(&self, company: CompanyId) -> Arc<tokio::sync::Mutex<()>> {
        self.manual
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(company)
            .or_default()
            .clone()
    }
}

/// One company's backup configuration.
pub fn company_config(state: &AppState, company: CompanyId) -> BackupConfig {
    company_backup_config(&state.ops.settings().backups, company)
}

fn resolver(state: &AppState) -> Arc<dyn SecretResolver> {
    Arc::new(state.ops.secrets.clone())
}

/// The reconcile loop (see the module docs).
pub(crate) async fn run(state: AppState, mut stop: watch::Receiver<bool>) {
    loop {
        reconcile(&state).await;
        tokio::select! {
            stopping = super::nap(RECONCILE_EVERY, &mut stop) => if stopping { break },
            () = state.ops.wake_backups.notified() => {}
        }
        if *stop.borrow() {
            break;
        }
    }
}

/// Bring the schedulers in line with the companies and the settings.
pub(crate) async fn reconcile(state: &AppState) {
    let settings = state.ops.settings();
    let wanted = settings.backups_enabled() && settings.backups.schedule.enabled;
    let companies = if wanted {
        let s = state.clone();
        match tokio::task::spawn_blocking(move || open_all(&s)).await {
            Ok(list) => list,
            Err(e) => {
                tracing::warn!(error = %e, "could not list companies for backups");
                return;
            }
        }
    } else {
        Vec::new()
    };
    let mut stopping = Vec::new();
    {
        let mut running = state.ops.backups.running();
        // Gone, or renamed (its backups' file names follow the name): stop, and
        // a renamed one starts again below under its new name.
        let ids: Vec<CompanyId> = running.keys().copied().collect();
        for id in ids {
            let stale = companies
                .iter()
                .find(|(c, _, _)| *c == id)
                .is_none_or(|(_, _, name)| *name != running[&id].name);
            if stale && let Some(r) = running.remove(&id) {
                stopping.push(r.scheduler);
            }
        }
        for (id, books, name) in &companies {
            let config = company_backup_config(&settings.backups, *id);
            match running.get_mut(id) {
                Some(r) if r.config != config => {
                    r.scheduler.update(config.clone());
                    r.config = config;
                }
                Some(_) => {}
                None => {
                    tracing::info!(company = %name, "backups scheduled");
                    running.insert(
                        *id,
                        Running {
                            name: name.clone(),
                            scheduler: Scheduler::start(
                                books.clone(),
                                name.clone(),
                                config.clone(),
                                resolver(state),
                            ),
                            config,
                        },
                    );
                }
            }
        }
    }
    for s in stopping {
        s.shutdown().await;
    }
}

/// Every company with its books and name. Unreadable ones are skipped (and
/// logged by the company list).
fn open_all(state: &AppState) -> Vec<(CompanyId, Books, String)> {
    let Ok(list) = state.companies.list() else {
        return Vec::new();
    };
    list.into_iter()
        .filter_map(|c| state.companies.get(c.id).ok().map(|b| (c.id, b, c.name)))
        .collect()
}

/// Stop every scheduler, after any backup in progress.
pub(crate) async fn shutdown(state: &AppState) {
    let all: Vec<Running> = state
        .ops
        .backups
        .running()
        .drain()
        .map(|(_, r)| r)
        .collect();
    for r in all {
        r.scheduler.shutdown().await;
    }
}

/// What a company's scheduler is doing.
#[derive(Clone, Debug, Serialize)]
pub struct CompanyBackupStatus {
    pub company_id: CompanyId,
    pub company_name: String,
    pub status: SchedulerStatus,
}

/// Every scheduler's status, by company name.
pub fn statuses(state: &AppState) -> Vec<CompanyBackupStatus> {
    let mut out: Vec<CompanyBackupStatus> = state
        .ops
        .backups
        .running()
        .iter()
        .map(|(id, r)| CompanyBackupStatus {
            company_id: *id,
            company_name: r.name.clone(),
            status: r.scheduler.status(),
        })
        .collect();
    out.sort_by_key(|s| s.company_name.to_lowercase());
    out
}

/// One company's scheduler status, if it has a scheduler.
pub fn status(state: &AppState, company: CompanyId) -> Option<SchedulerStatus> {
    state
        .ops
        .backups
        .running()
        .get(&company)
        .map(|r| r.scheduler.status())
}

/// Back `books` up now (one manual backup per company at a time).
pub async fn run_now(
    state: &AppState,
    company: CompanyId,
    books: &Books,
    company_name: &str,
) -> Result<BackupRecord, BackupError> {
    let lock = state.ops.backups.manual_lock(company);
    let _one = lock.lock().await;
    let config = company_config(state, company);
    lb_backup::backup_now(books, company_name, &config, &state.ops.secrets).await
}

/// A backup as listed: which destination (by position in the settings) holds it.
#[derive(Clone, Debug, Serialize)]
pub struct Listed {
    pub destination: usize,
    /// The destination in words (`local folder D:\Backups`).
    pub destination_name: String,
    pub manifest: Manifest,
}

/// Every backup of a company, newest first, and the destinations that could not
/// be read.
#[derive(Clone, Debug, Default, Serialize)]
pub struct BackupList {
    pub backups: Vec<Listed>,
    pub errors: Vec<String>,
}

/// A company's backups in every destination.
pub async fn list(state: &AppState, company: CompanyId) -> BackupList {
    let config = company_config(state, company);
    let mut out = BackupList::default();
    for (i, d) in config.destinations.iter().enumerate() {
        match lb_backup::list_backups(d, &state.ops.secrets, None).await {
            Ok(entries) => out.backups.extend(entries.into_iter().map(|e| Listed {
                destination: i,
                destination_name: d.describe(),
                manifest: e.manifest,
            })),
            Err(e) => out.errors.push(format!("{}: {e}", d.describe())),
        }
    }
    out.backups.sort_by(|a, b| {
        b.manifest
            .created_at
            .cmp(&a.manifest.created_at)
            .then(a.destination.cmp(&b.destination))
    });
    out
}

/// The backup with id `id` (from the first destination that has it, or from
/// `destination`).
pub async fn find(
    state: &AppState,
    company: CompanyId,
    id: &str,
    destination: Option<usize>,
) -> Result<BackupEntry, BackupError> {
    let config = company_config(state, company);
    for (i, d) in config.destinations.iter().enumerate() {
        if destination.is_some_and(|want| want != i) {
            continue;
        }
        match lb_backup::list_backups(d, &state.ops.secrets, None).await {
            Ok(entries) => {
                if let Some(e) = entries.into_iter().find(|e| e.manifest.id == id) {
                    return Ok(e);
                }
            }
            Err(e) => {
                tracing::warn!(destination = %d.describe(), error = %e, "could not list backups");
            }
        }
    }
    Err(BackupError::NotFound(format!("backup {id}")))
}

/// How to open an encrypted backup, as a request gives it.
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct UnlockRequest {
    /// age identities (`AGE-SECRET-KEY-1…`), for backups encrypted to keys.
    pub identities: Vec<String>,
    /// The passphrase, when it isn't the one in the settings.
    pub passphrase: Option<String>,
}

impl std::fmt::Debug for UnlockRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnlockRequest")
            .field("identities", &self.identities.len())
            .field("passphrase", &self.passphrase.is_some())
            .finish()
    }
}

impl UnlockRequest {
    /// What was given, else what the settings themselves can unlock (the
    /// passphrase of passphrase encryption).
    pub fn unlock(self, state: &AppState) -> Result<Unlock, BackupError> {
        if !self.identities.is_empty() {
            return Ok(Unlock::Identities(self.identities));
        }
        if let Some(p) = self.passphrase {
            return Ok(Unlock::Passphrase(SecretString::from(p)));
        }
        Unlock::from_config(&state.ops.settings().backups.encryption, &state.ops.secrets)
    }
}

/// Check a backup without restoring it.
pub async fn verify(
    state: &AppState,
    entry: &BackupEntry,
    unlock: &Unlock,
) -> Result<VerifyReport, BackupError> {
    lb_backup::verify(entry, &state.ops.secrets, unlock).await
}

/// A restored company: its new id and name.
#[derive(Clone, Debug, Serialize)]
pub struct Restored {
    pub id: CompanyId,
    pub name: String,
    pub from: Manifest,
}

/// The name a restored copy gets unless told otherwise.
pub fn restored_name(original: &str, made: jiff::Timestamp) -> String {
    let day = made.to_zoned(jiff::tz::TimeZone::system()).date();
    format!("{original} (restored from {day})")
}

/// Restore `entry` as a **new** company (fresh id; nothing existing is touched),
/// named `name` or [`restored_name`].
pub async fn restore(
    state: &AppState,
    entry: &BackupEntry,
    unlock: &Unlock,
    name: Option<String>,
    actor: &Actor,
) -> ApiResult<Restored> {
    let work = state.ops.data_dir().join(".restore");
    std::fs::create_dir_all(&work)
        .map_err(|e| ApiError::internal(format!("could not create {}: {e}", work.display())))?;
    let tmp = work.join(format!("{}.lbdb", uuid::Uuid::now_v7()));
    let manifest = lb_backup::restore(entry, &tmp, &state.ops.secrets, unlock)
        .await
        .map_err(ApiError::from)?;
    let name = name
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| restored_name(&manifest.company_name, manifest.created_at));
    let adopted = adopt_renamed(state, tmp.clone(), name.clone(), actor.clone()).await;
    remove_with_sidecars(&tmp);
    let id = adopted?;
    state.ops.wake_backups();
    Ok(Restored {
        id,
        name,
        from: manifest,
    })
}

/// Bring a company file in with a fresh id and give it `name`.
async fn adopt_renamed(
    state: &AppState,
    file: PathBuf,
    name: String,
    actor: Actor,
) -> ApiResult<CompanyId> {
    let s = state.clone();
    tokio::task::spawn_blocking(move || {
        let id = s.companies.adopt(&file)?;
        let books = s.companies.get(id)?;
        books.write(&actor, |w| {
            let mut info = lb_store::company::info(&w.tx)?;
            info.name = name;
            lb_store::company::set_info(w, &info)
        })?;
        Ok(id)
    })
    .await?
}

/// Import a `.lbk` portable company file as a new company.
pub async fn import_lbk(
    state: &AppState,
    lbk: PathBuf,
    name: Option<String>,
    actor: &Actor,
) -> ApiResult<(CompanyId, PortableManifest)> {
    let work = state.ops.data_dir().join(".restore");
    std::fs::create_dir_all(&work)
        .map_err(|e| ApiError::internal(format!("could not create {}: {e}", work.display())))?;
    let tmp = work.join(format!("{}.lbdb", uuid::Uuid::now_v7()));
    let t = tmp.clone();
    let manifest = tokio::task::spawn_blocking(move || lb_backup::import_lbk(&lbk, &t))
        .await?
        .map_err(ApiError::from)?;
    let name = name
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| manifest.company_name.clone());
    let adopted = adopt_renamed(state, tmp.clone(), name, actor.clone()).await;
    remove_with_sidecars(&tmp);
    let id = adopted?;
    state.ops.wake_backups();
    Ok((id, manifest))
}

fn remove_with_sidecars(path: &std::path::Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(p));
    }
}

/// Write `books` as a `.lbk` portable company file at `dest` (must not exist).
pub async fn export_lbk(
    books: &Books,
    company_name: &str,
    dest: PathBuf,
) -> Result<PortableManifest, BackupError> {
    let books = books.clone();
    let name = company_name.to_owned();
    tokio::task::spawn_blocking(move || lb_backup::export_lbk(&books, &name, &dest))
        .await
        .map_err(|e| BackupError::Task(e.to_string()))?
}

impl From<BackupError> for ApiError {
    fn from(e: BackupError) -> Self {
        use axum::http::StatusCode;
        let message = e.to_string();
        match e {
            BackupError::NotFound(_) => ApiError::new(StatusCode::NOT_FOUND, "not_found", message),
            BackupError::Exists(_) => ApiError::new(StatusCode::CONFLICT, "conflict", message),
            BackupError::Config(_)
            | BackupError::NoDestinations
            | BackupError::Secret { .. }
            | BackupError::Decrypt(_)
            | BackupError::Manifest(_)
            | BackupError::TooNew { .. } => ApiError::invalid(message),
            BackupError::Checksum { .. } | BackupError::Integrity(_) => {
                ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "damaged", message)
            }
            BackupError::Remote { .. }
            | BackupError::Network(_)
            | BackupError::AllDestinationsFailed(_) => {
                ApiError::new(StatusCode::BAD_GATEWAY, "storage", message)
            }
            BackupError::Store(s) => s.into(),
            BackupError::Io { .. }
            | BackupError::Sqlite(_)
            | BackupError::Encrypt(_)
            | BackupError::Task(_) => ApiError::internal(message),
        }
    }
}
