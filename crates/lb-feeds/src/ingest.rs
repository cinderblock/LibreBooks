//! The whole pipeline for a statement, in one write: import (skipping what's
//! already there) → rules → match → auto-add.
//!
//! Order matters. Rules run before matching so a rule's chosen name can count
//! towards a match. Matching runs before auto-add, and auto-add only touches a
//! download nothing in the register resembles, so a rule saying "always add
//! Comcast as Utilities" never duplicates the Comcast check someone already
//! wrote.

use lb_core::AccountId;
use lb_store::Writer;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::add::{self, AddOutcome, AddRequest};
use crate::connections::{self, FeedConnection};
use crate::error::{FeedError, Result};
use crate::matching::{self, MatchOptions, MatchedPair};
use crate::queue::{self, FeedStatus, ImportOptions, ImportSummary};
use crate::rules;
use crate::statement::Statement;

/// How to run the pipeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct IngestOptions {
    pub import: ImportOptions,
    pub matching: MatchOptions,
    /// Let rules marked "add automatically" add.
    pub auto_add: bool,
}

impl Default for IngestOptions {
    fn default() -> Self {
        IngestOptions {
            import: ImportOptions::default(),
            matching: MatchOptions::default(),
            auto_add: true,
        }
    }
}

/// What happened to a statement.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IngestReport {
    pub import: ImportSummary,
    /// Matched to transactions already in the register.
    pub matched: Vec<MatchedPair>,
    /// Added by rules.
    pub auto_added: Vec<AddOutcome>,
    /// Rules that wanted to add but couldn't, and why.
    pub auto_add_failures: Vec<add::AddFailure>,
    /// Still waiting for a person.
    pub waiting: u32,
}

impl IngestReport {
    fn absorb(&mut self, other: IngestReport) {
        self.import.imported.extend(other.import.imported);
        self.import.duplicates += other.import.duplicates;
        self.import.before_start += other.import.before_start;
        self.import.warnings.extend(other.import.warnings);
        self.matched.extend(other.matched);
        self.auto_added.extend(other.auto_added);
        self.auto_add_failures.extend(other.auto_add_failures);
        self.waiting += other.waiting;
    }
}

/// Import a statement into `account`, then rename, match and auto-add.
pub fn ingest(
    w: &mut Writer<'_>,
    account: AccountId,
    statement: &Statement,
    opts: &IngestOptions,
) -> Result<IngestReport> {
    let import = queue::import_statement(w, account, statement, &opts.import)?;
    let ids = import.imported.clone();
    rules::apply_to(w, &ids)?;
    let matched = matching::auto_match(w, &ids, opts.matching)?;

    let mut auto_added = Vec::new();
    let mut auto_add_failures = Vec::new();
    if opts.auto_add {
        for &id in &ids {
            let f = queue::get(&w.tx, id)?;
            let wants =
                f.status == FeedStatus::New && f.suggestion.as_ref().is_some_and(|s| s.auto_add);
            if !wants || matching::has_any_candidate(&w.tx, &f, opts.matching)? {
                continue;
            }
            let batch = add::add_all(w, &[id], Some(&AddRequest::default()))?;
            auto_added.extend(batch.added);
            auto_add_failures.extend(batch.failed);
        }
    }
    let waiting = ids.len() - matched.len() - auto_added.len();
    Ok(IngestReport {
        import,
        matched,
        auto_added,
        auto_add_failures,
        waiting: waiting as u32,
    })
}

/// Ingest a download for a connection: pick out the statement(s) that are this
/// connection's account, apply its settings, and remember the account's
/// fingerprint for next time.
///
/// A statement is this connection's when its provider account id equals the
/// connection's (SimpleFIN and friends), or its fingerprint equals the
/// connection's, or, when the download has exactly one statement and the
/// connection has no fingerprint yet, that one.
pub fn ingest_for_connection(
    w: &mut Writer<'_>,
    conn: &FeedConnection,
    statements: &[Statement],
) -> Result<IngestReport> {
    let account = conn.account_id.ok_or_else(|| {
        FeedError::Invalid(format!(
            "choose which account \"{}\" feeds before downloading",
            conn.name
        ))
    })?;
    let remote = match &conn.settings {
        connections::ProviderSettings::SimpleFin {
            remote_account_id, ..
        }
        | connections::ProviderSettings::Plaid {
            remote_account_id, ..
        }
        | connections::ProviderSettings::Teller {
            remote_account_id, ..
        } => Some(remote_account_id.as_str()),
        _ => None,
    };
    let mine: Vec<&Statement> = statements
        .iter()
        .filter(|s| {
            remote.is_some_and(|r| s.account.remote_id.as_deref() == Some(r))
                || conn
                    .account_key
                    .as_deref()
                    .is_some_and(|k| !k.is_empty() && s.account.account_key == k)
        })
        .collect();
    let mine = if mine.is_empty() && remote.is_none() && conn.account_key.is_none() {
        match statements {
            [only] => vec![only],
            [] => Vec::new(),
            _ => {
                return Err(FeedError::Invalid(
                    "the download has several accounts; choose which one this connection is".into(),
                ));
            }
        }
    } else {
        mine
    };
    let opts = IngestOptions {
        import: ImportOptions {
            connection_id: Some(conn.id),
            source: conn.settings.kind().to_owned(),
            reverse_signs: conn.reverse_signs,
            not_before: conn.not_before,
        },
        matching: MatchOptions {
            days: conn.match_days,
        },
        auto_add: true,
    };
    let mut report = IngestReport::default();
    for s in &mine {
        report.absorb(ingest(w, account, s, &opts)?);
    }
    // Remember the fingerprint so the next file finds this connection.
    if conn.account_key.is_none()
        && let Some(s) = mine.first()
        && !s.account.account_key.is_empty()
    {
        let mut updated = connections::get(&w.tx, conn.id)?;
        updated.account_key = Some(s.account.account_key.clone());
        connections::save(w, updated)?;
    }
    Ok(report)
}
