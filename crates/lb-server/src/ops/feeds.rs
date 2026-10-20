//! Downloading bank feeds by themselves: every few minutes, each online
//! connection (SimpleFIN, Direct Connect) whose last download is older than the
//! configured interval is synced. The same [`sync_connection`] serves the "Sync
//! now" button.
//!
//! Banks and bridges limit how often they may be asked, so attempts are spaced
//! the same way whether they worked or not, and a connection whose credentials
//! were refused waits until someone edits it (a new password, a reconnect)
//! rather than being refused again every few hours.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use lb_core::CompanyId;
use lb_feeds::connections::{self, FeedConnection};
use lb_feeds::ingest::IngestReport;
use lb_feeds::{FeedConnectionId, FeedError, providers};
use lb_store::{Actor, Books};
use tokio::sync::watch;

use crate::state::AppState;

/// How often the loop looks for connections that are due.
const LOOK_EVERY: Duration = Duration::from_secs(5 * 60);
/// The first look after start.
const FIRST_LOOK: Duration = Duration::from_secs(90);

/// The last automatic or manual attempt per connection (in memory: a restart
/// forgets it, and the stored last success takes over).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Attempt {
    pub at: Timestamp,
    /// The connection's edit sequence then: an edit since means "try again".
    pub edit_seq: i64,
    /// The credentials were refused.
    pub refused: bool,
}

#[derive(Debug, Default)]
pub(crate) struct FeedSlots {
    attempts: Mutex<HashMap<FeedConnectionId, Attempt>>,
    locks: Mutex<HashMap<FeedConnectionId, Arc<tokio::sync::Mutex<()>>>>,
}

impl FeedSlots {
    fn attempts(&self) -> MutexGuard<'_, HashMap<FeedConnectionId, Attempt>> {
        self.attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_for(&self, id: FeedConnectionId) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(id)
            .or_default()
            .clone()
    }
}

/// Is `conn` due for an automatic download at `now`?
pub(crate) fn due(
    conn: &FeedConnection,
    attempt: Option<Attempt>,
    every: SignedDuration,
    now: Timestamp,
) -> bool {
    if !conn.settings.is_online() || conn.account_id.is_none() {
        return false;
    }
    if let Some(a) = attempt {
        if a.refused && a.edit_seq == conn.edit_seq {
            return false;
        }
        if a.edit_seq == conn.edit_seq && a.at.duration_until(now) < every {
            return false;
        }
    }
    conn.last_sync_at
        .is_none_or(|last| last.duration_until(now) >= every)
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> lb_feeds::Result<T> + Send + 'static,
) -> lb_feeds::Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| FeedError::Invalid(format!("a worker failed: {e}")))?
}

/// Download one connection now and ingest what came: fetch (async), then the
/// import, rules, matching and auto-add in one write. The outcome (the time, or
/// the error) is recorded on the connection either way.
pub async fn sync_connection(
    state: &AppState,
    company: CompanyId,
    books: &Books,
    id: FeedConnectionId,
    actor: &Actor,
) -> lb_feeds::Result<IngestReport> {
    let lock = state.ops.feeds.lock_for(id);
    let _one = lock.lock().await;
    let b = books.clone();
    let conn = blocking(move || lb_feeds::read(&b, |c| connections::get(c, id))).await?;
    let result = fetch(state, company, &conn).await;
    let refused = matches!(result, Err(FeedError::Unauthorized { .. }));
    state.ops.feeds.attempts().insert(
        id,
        Attempt {
            at: Timestamp::now(),
            edit_seq: conn.edit_seq,
            refused,
        },
    );
    let b = books.clone();
    let actor = actor.clone();
    match result {
        Err(e) => {
            let message = e.to_string();
            blocking(move || {
                lb_feeds::write(&b, &actor, |w| {
                    connections::record_sync(w, id, Timestamp::now(), Some(&message))
                })
            })
            .await?;
            Err(e)
        }
        Ok(statements) => {
            blocking(move || {
                lb_feeds::write(&b, &actor, |w| {
                    let report = lb_feeds::ingest::ingest_for_connection(w, &conn, &statements)?;
                    connections::record_sync(w, id, Timestamp::now(), None)?;
                    Ok(report)
                })
            })
            .await
        }
    }
}

async fn fetch(
    state: &AppState,
    company: CompanyId,
    conn: &FeedConnection,
) -> lb_feeds::Result<Vec<lb_feeds::Statement>> {
    let secrets = state.ops.secrets.for_company(company);
    let provider = providers::for_connection(conn, &secrets, providers::http_client()?)?;
    let today = jiff::Zoned::now().date();
    let since = providers::since_for(conn, today);
    provider.fetch(Some(since)).await
}

/// The loop.
pub(crate) async fn run(state: AppState, mut stop: watch::Receiver<bool>) {
    let mut wait = FIRST_LOOK;
    loop {
        if super::nap(wait, &mut stop).await {
            break;
        }
        wait = LOOK_EVERY;
        let settings = state.ops.settings().feeds;
        if !settings.auto_sync {
            continue;
        }
        let s = state.clone();
        let Ok(companies) = tokio::task::spawn_blocking(move || {
            s.companies
                .ids()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|id| s.companies.get(id).ok().map(|b| (id, b)))
                .collect::<Vec<_>>()
        })
        .await
        else {
            continue;
        };
        for (company, books) in companies {
            let b = books.clone();
            let conns = match blocking(move || lb_feeds::read(&b, connections::list)).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(%company, error = %e, "could not read feed connections");
                    continue;
                }
            };
            for conn in conns {
                let attempt = state.ops.feeds.attempts().get(&conn.id).copied();
                if !due(&conn, attempt, settings.every, Timestamp::now()) {
                    continue;
                }
                if *stop.borrow() {
                    return;
                }
                let actor = Actor {
                    user_id: None,
                    may_override_closing_date: false,
                };
                match sync_connection(&state, company, &books, conn.id, &actor).await {
                    Ok(r) => tracing::info!(
                        connection = %conn.name,
                        new = r.import.imported.len(),
                        matched = r.matched.len(),
                        added = r.auto_added.len(),
                        "bank feed downloaded"
                    ),
                    Err(e) => {
                        tracing::warn!(connection = %conn.name, error = %e, "bank feed download failed");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lb_feeds::SecretRef;
    use lb_feeds::connections::ProviderSettings;

    fn conn() -> FeedConnection {
        FeedConnection::new(
            "Checking",
            Some(lb_core::AccountId::new()),
            ProviderSettings::SimpleFin {
                access: SecretRef::new("secret:x"),
                remote_account_id: "A".into(),
            },
        )
    }

    #[test]
    fn connections_are_due_by_their_last_success_and_attempt() {
        let every = SignedDuration::from_hours(6);
        let now: Timestamp = "2026-10-19T12:00:00Z".parse().unwrap();
        let hours_ago = |h: i64| now.checked_sub(SignedDuration::from_hours(h)).unwrap();
        let mut c = conn();
        assert!(due(&c, None, every, now), "never synced");
        c.last_sync_at = Some(hours_ago(2));
        assert!(!due(&c, None, every, now));
        c.last_sync_at = Some(hours_ago(7));
        assert!(due(&c, None, every, now));
        // A failed attempt an hour ago waits too.
        let failed = Attempt {
            at: hours_ago(1),
            edit_seq: c.edit_seq,
            refused: false,
        };
        assert!(!due(&c, Some(failed), every, now));
        // Refused credentials wait for an edit, however long ago.
        let refused = Attempt {
            at: hours_ago(100),
            edit_seq: c.edit_seq,
            refused: true,
        };
        assert!(!due(&c, Some(refused), every, now));
        c.edit_seq += 1;
        assert!(due(&c, Some(refused), every, now));
        // Files and unassigned connections never download.
        c.account_id = None;
        assert!(!due(&c, None, every, now));
        let file = FeedConnection::new(
            "File",
            Some(lb_core::AccountId::new()),
            ProviderSettings::File { csv_mapping: None },
        );
        assert!(!due(&file, None, every, now));
    }
}
