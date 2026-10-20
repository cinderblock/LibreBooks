//! Periodic work the server does on its own: entering memorized transactions
//! that are due, and pruning expired sign-in records.

use std::time::Duration;

use crate::state::AppState;

/// Run housekeeping now and then hourly, until the runtime shuts down.
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60 * 60));
        loop {
            tick.tick().await;
            let s = state.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || run_once(&s)).await {
                tracing::warn!(error = %e, "housekeeping failed");
            }
        }
    })
}

fn run_once(state: &AppState) {
    let today = jiff::Zoned::now().date();
    let ids = match state.companies.ids() {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(error = %e, "couldn't list companies");
            return;
        }
    };
    for id in ids {
        let Ok(books) = state.companies.get(id) else {
            continue;
        };
        match books.write(&lb_store::Actor::system(), |w| {
            lb_store::saved::enter_due(w, today)
        }) {
            Ok(entered) if !entered.is_empty() => {
                tracing::info!(company = %id, count = entered.len(), "entered memorized transactions");
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(company = %id, error = %e, "couldn't enter memorized transactions");
            }
        }
    }
    if let Some(s) = &state.sessions
        && let Err(e) = s.store.prune()
    {
        tracing::warn!(error = %e, "couldn't prune sign-in records");
    }
    state.imports.prune();
}
