//! Running backups on a timetable, for the server.
//!
//! One [`Scheduler`] per open company: a tokio task that sleeps until the next
//! backup is due ([`next_run`]), makes it, and records the outcome for the UI. At
//! start it reads the destinations to learn when the last backup was, so a
//! restart neither skips a backup nor makes an extra one.
//!
//! The task wakes at least once a minute and re-reads the wall clock, so a laptop
//! that slept through 02:00 backs up when it wakes, not a day late. A failed backup
//! is retried after [`RETRY_AFTER`] rather than in a tight loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp, Zoned};
use lb_store::Books;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::config::BackupConfig;
use crate::manifest::slug;
use crate::run::{BackupRecord, backup_now, list_backups};
use crate::schedule::next_run;
use crate::secret::SecretResolver;

/// How long after a failed backup the next attempt is made.
pub const RETRY_AFTER: SignedDuration = SignedDuration::from_mins(30);
/// The longest the task sleeps before looking at the clock again.
const MAX_NAP: Duration = Duration::from_secs(60);

/// What the scheduler is doing, for `GET /backups/status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerStatus {
    /// A backup is being made right now.
    pub running: bool,
    /// When the next scheduled backup is due; `None` when the schedule is off.
    pub next_run: Option<Timestamp>,
    /// The most recent successful backup known, from this run or found at start.
    pub last_success_at: Option<Timestamp>,
    /// The last attempt this scheduler made.
    pub last: Option<LastRun>,
}

/// One attempt and its outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastRun {
    pub started_at: Timestamp,
    pub finished_at: Timestamp,
    /// Present when the backup was made (it may still have failed somewhere; see
    /// each destination).
    pub record: Option<BackupRecord>,
    /// Why it failed, when it did.
    pub error: Option<String>,
}

struct Shared {
    books: Books,
    company_name: String,
    secrets: Arc<dyn SecretResolver>,
    config: Mutex<BackupConfig>,
    status: Mutex<SchedulerStatus>,
    wake: Notify,
    run_now: AtomicBool,
    stop: AtomicBool,
}

impl Shared {
    fn status(&self) -> MutexGuard<'_, SchedulerStatus> {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn config(&self) -> BackupConfig {
        self.config
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// Backs up one company on its schedule. Dropping it stops the task (a backup in
/// progress is abandoned; its working files are cleaned up).
pub struct Scheduler {
    shared: Arc<Shared>,
    task: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("company", &self.shared.company_name)
            .field("status", &*self.shared.status())
            .finish_non_exhaustive()
    }
}

impl Scheduler {
    /// Start backing up `books` per `config`. Must be called inside a tokio
    /// runtime.
    pub fn start(
        books: Books,
        company_name: impl Into<String>,
        config: BackupConfig,
        secrets: Arc<dyn SecretResolver>,
    ) -> Scheduler {
        let shared = Arc::new(Shared {
            books,
            company_name: company_name.into(),
            secrets,
            config: Mutex::new(config),
            status: Mutex::new(SchedulerStatus::default()),
            wake: Notify::new(),
            run_now: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let task = tokio::spawn(run(Arc::clone(&shared)));
        Scheduler {
            shared,
            task: Some(task),
        }
    }

    pub fn status(&self) -> SchedulerStatus {
        self.shared.status().clone()
    }

    /// Make a backup as soon as possible (after one in progress).
    pub fn run_now(&self) {
        self.shared.run_now.store(true, Ordering::SeqCst);
        self.shared.wake.notify_one();
    }

    /// Replace the configuration; the next run is recomputed at once.
    pub fn update(&self, config: BackupConfig) {
        *self
            .shared
            .config
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = config;
        self.shared.wake.notify_one();
    }

    /// Stop after any backup in progress finishes.
    pub async fn shutdown(mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.wake.notify_one();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn run(shared: Arc<Shared>) {
    let company_slug = slug(&shared.company_name);
    let mut last_success = find_last_backup(&shared, &company_slug).await;
    let mut last_failure: Option<Timestamp> = None;
    shared.status().last_success_at = last_success;

    loop {
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }
        let config = shared.config();
        let tz = config.tz().unwrap_or_else(|_| jiff::tz::TimeZone::system());
        let now = Zoned::new(Timestamp::now(), tz);
        let mut next = next_run(&config.schedule, last_success, &now);
        if let (Some(n), Some(failed)) = (next, last_failure) {
            next = Some(n.max(failed.checked_add(RETRY_AFTER).unwrap_or(Timestamp::MAX)));
        }
        shared.status().next_run = next;

        let manual = shared.run_now.swap(false, Ordering::SeqCst);
        let due = next.is_some_and(|n| n <= now.timestamp());
        if manual || due {
            shared.status().running = true;
            let started_at = Timestamp::now();
            let result = backup_now(
                &shared.books,
                &shared.company_name,
                &config,
                shared.secrets.as_ref(),
            )
            .await;
            let finished_at = Timestamp::now();
            let mut status = shared.status();
            status.running = false;
            status.last = Some(match result {
                Ok(record) => {
                    last_success = Some(record.manifest.created_at);
                    last_failure = None;
                    status.last_success_at = last_success;
                    LastRun {
                        started_at,
                        finished_at,
                        record: Some(record),
                        error: None,
                    }
                }
                Err(e) => {
                    tracing::warn!(company = %shared.company_name, error = %e, "scheduled backup failed");
                    last_failure = Some(finished_at);
                    LastRun {
                        started_at,
                        finished_at,
                        record: None,
                        error: Some(e.to_string()),
                    }
                }
            });
            continue;
        }

        let nap = next.map_or(MAX_NAP, |n| {
            let left = now.timestamp().duration_until(n);
            Duration::try_from(left)
                .unwrap_or(Duration::ZERO)
                .min(MAX_NAP)
        });
        tokio::select! {
            () = tokio::time::sleep(nap) => {}
            () = shared.wake.notified() => {}
        }
    }
}

/// The newest backup of this company in any destination, so that a restart keeps
/// to the schedule. Unreachable destinations are skipped.
async fn find_last_backup(shared: &Shared, company_slug: &str) -> Option<Timestamp> {
    let config = shared.config();
    let mut newest = None;
    for d in &config.destinations {
        match list_backups(d, shared.secrets.as_ref(), Some(company_slug)).await {
            Ok(list) => {
                if let Some(first) = list.first() {
                    newest = newest.max(Some(first.manifest.created_at));
                }
            }
            Err(e) => {
                tracing::warn!(destination = %d.describe(), error = %e, "could not list backups");
            }
        }
    }
    newest
}
