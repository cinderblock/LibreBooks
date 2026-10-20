//! The sync engine: QuickBooks Desktop → a LibreBooks company file, read-only,
//! incremental and idempotent.
//!
//! One sync:
//! 1. Binds the books to the QBD company (`qbd.source_id`, derived from the EIN)
//!    and refuses to mix in a different one.
//! 2. Copies company information and preferences.
//! 3. Walks [`QbdEntity::SYNC_ORDER`]: lists first (parents before children),
//!    then transactions, payments after what they pay, deposits after the
//!    payments they bank. Each entity is listed with `modified_after` set to its
//!    watermark; a row whose `EditSequence` matches what was imported last time
//!    is skipped without fetching it again.
//! 4. Each changed or new record is normalised and saved in its own SQLite
//!    transaction, with its QBD id and `EditSequence` in the store's external
//!    references. A record that fails is reported and retried next time; it never
//!    stops the rest.
//! 5. Compares what LibreBooks computed with what QBD stored (totals, sales tax,
//!    open balances) and reports differences instead of failing on them.
//! 6. Optionally sweeps: lists every id QBD still has and deletes (or, if still
//!    used, deactivates) what QBD deleted.
//!
//! Watermarks are QBD's own `TimeModified` strings. One never moves past a record
//! that failed, or that was imported with a link still missing, so those are
//! listed again next time. A run limited by count or date leaves watermarks and
//! deletions alone.

pub mod import;
pub mod report;
pub mod resolver;

use std::collections::{BTreeMap, HashMap, HashSet};

use jiff::civil::Date;
use lb_core::{ItemType, Money, TxnId};
use lb_store::{Books, StoreError, accounts, company, entities, extref, items, lists, txns};
use serde_json::Value;

use crate::client::{BridgeError, ListQuery, QbdSource};
use crate::entity::{NOT_EXPOSED_TXN_TYPES, QbdEntity};
use crate::qbxml::record::{Meta, describe};
use crate::qbxml::{self};
use import::{ImportError, Outcome, PARTIAL_PREFIX, import_list, import_txn, write_record};
pub use report::{
    EntityReport, Gap, Mismatch, MismatchKind, Severity, SyncIssue, SyncPhase, SyncProgress,
    SyncReport,
};
use resolver::ResolverState;

/// Settings keys in the company file.
pub const SETTING_SOURCE: &str = "qbd.source_id";
pub const SETTING_LAST_SYNC: &str = "qbd.last_sync";

pub fn watermark_key(entity: QbdEntity) -> String {
    format!("qbd.watermark.{}", entity.name())
}

/// How to sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncOptions {
    /// Only these (in sync order). `None`: everything.
    pub entities: Option<Vec<QbdEntity>>,
    /// Ignore watermarks: list everything again (unchanged records are still
    /// skipped by `EditSequence`).
    pub full: bool,
    /// Re-import even unchanged records (after the importer itself improves).
    pub reimport: bool,
    /// Find records deleted in QBD and delete them here.
    pub sweep: bool,
    /// At most this many of each transaction type (light runs against a live
    /// company). Leaves watermarks alone and disables the sweep.
    pub txn_limit: Option<usize>,
    /// Only transactions dated within this range. Leaves watermarks alone and
    /// disables the sweep.
    pub txn_from: Option<Date>,
    pub txn_to: Option<Date>,
    /// Bind to this source id instead of deriving one. Must match the books'
    /// existing binding, if any.
    pub source_id: Option<String>,
    /// Copy QBD's company information and preferences.
    pub import_company_info: bool,
    /// After the import, apply customers' open credits to their open invoices
    /// wherever QBD shows the invoice settled and we don't. The bridge never
    /// says which credit memo QBD applied to which invoice, so this is how
    /// those settlements are reproduced (oldest first, as QBD applies them).
    pub apply_credits: bool,
    /// Abort after this many consecutive failures to reach the bridge.
    pub max_consecutive_bridge_failures: u32,
}

impl Default for SyncOptions {
    fn default() -> Self {
        SyncOptions {
            entities: None,
            full: false,
            reimport: false,
            sweep: false,
            txn_limit: None,
            txn_from: None,
            txn_to: None,
            source_id: None,
            import_company_info: true,
            apply_credits: true,
            max_consecutive_bridge_failures: 5,
        }
    }
}

impl SyncOptions {
    fn restricted(&self) -> bool {
        self.txn_limit.is_some() || self.txn_from.is_some() || self.txn_to.is_some()
    }

    fn includes(&self, e: QbdEntity) -> bool {
        self.entities.as_ref().is_none_or(|l| l.contains(&e))
    }
}

/// Why a sync couldn't run at all. Per-record problems are in the report.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Bridge(#[from] BridgeError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(
        "these books hold QuickBooks company {bound}, but the bridge is serving {found}; refusing to mix two companies"
    )]
    DifferentCompany { bound: String, found: String },
    #[error(
        "QuickBooks didn't say which company this is (no EIN or name), so the books can't be bound to it"
    )]
    NoCompanyId,
}

/// A source id for a QBD company: `qbd:ein-<digits>`, else a slug of its legal
/// (or trading) name. QBD exposes no file GUID; the EIN is the most stable field.
pub fn derive_source_id(company: &Value) -> Option<String> {
    let text = |k: &str| company.get(k).and_then(Value::as_str).unwrap_or("");
    let ein: String = text("EIN").chars().filter(char::is_ascii_digit).collect();
    if ein.len() >= 9 {
        return Some(format!("qbd:ein-{ein}"));
    }
    let name = if text("LegalCompanyName").is_empty() {
        text("CompanyName")
    } else {
        text("LegalCompanyName")
    };
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    (!slug.is_empty()).then(|| format!("qbd:name-{slug}"))
}

/// The source these books are bound to, binding them on first use.
fn bind_source(books: &Books, company: &Value, options: &SyncOptions) -> Result<String, SyncError> {
    let stored = books.read(|c| company::setting(c, SETTING_SOURCE))?;
    let id = match (&options.source_id, stored, derive_source_id(company)) {
        (Some(wanted), Some(bound), _) if *wanted != bound => {
            return Err(SyncError::DifferentCompany {
                bound,
                found: wanted.clone(),
            });
        }
        (Some(wanted), _, _) => wanted.clone(),
        (None, Some(bound), Some(found)) if bound != found => {
            return Err(SyncError::DifferentCompany { bound, found });
        }
        (None, Some(bound), _) => bound,
        (None, None, Some(found)) => found,
        (None, None, None) => return Err(SyncError::NoCompanyId),
    };
    books.write(&lb_store::Actor::system(), |w| {
        if company::setting(&w.tx, SETTING_SOURCE)?.is_none() {
            company::set_setting(w, SETTING_SOURCE, &id)?;
        }
        Ok(())
    })?;
    Ok(id)
}

/// The source these books are bound to, if they've been synced.
pub fn bound_source(books: &Books) -> Result<Option<String>, StoreError> {
    books.read(|c| company::setting(c, SETTING_SOURCE))
}

type Progress<'a> = &'a (dyn Fn(&SyncProgress) + Send + Sync);

/// A transaction imported this run, to compare its open balance with QBD's once
/// everything that pays it has been imported too.
#[derive(Clone)]
struct OpenCheck {
    entity: QbdEntity,
    qbd_id: String,
    record: String,
    txn_id: TxnId,
    qbd: Money,
}

struct Run<'a, S: QbdSource> {
    books: &'a Books,
    source: &'a S,
    options: &'a SyncOptions,
    progress: Progress<'a>,
    source_id: String,
    state: ResolverState,
    report: SyncReport,
    open_checks: Vec<OpenCheck>,
    /// Ids QBD listed in full (unfiltered) this run, for the sweep.
    listed_all: HashMap<QbdEntity, HashSet<String>>,
    gap_links: BTreeMap<String, u32>,
    consecutive_bridge_failures: u32,
}

/// Sync QBD into `books`. See the module docs.
pub async fn sync<S: QbdSource>(
    books: &Books,
    source: &S,
    options: &SyncOptions,
    progress: Progress<'_>,
) -> Result<SyncReport, SyncError> {
    let started = jiff::Timestamp::now();
    let company_rec = source.company().await?;
    let source_id = bind_source(books, &company_rec, options)?;
    let mut report = SyncReport {
        source_id: source_id.clone(),
        started_at: Some(started),
        full: options.full,
        partial_run: options.restricted(),
        ..SyncReport::default()
    };
    let prefs = match source.preferences().await {
        Ok(p) => Some(p),
        Err(e) => {
            report.issues.push(SyncIssue {
                severity: Severity::Warning,
                entity: QbdEntity::Account,
                qbd_id: None,
                record: "Company preferences".into(),
                message: format!("couldn't read them: {e}"),
            });
            None
        }
    };
    let (taxable_code, non_taxable_code) = qbxml::lists::default_tax_codes(prefs.as_ref());
    if options.import_company_info {
        let info = books.read(company::info)?;
        match qbxml::lists::company_info(&company_rec, prefs.as_ref(), &info) {
            Ok(new) if new != info => {
                books.write(&lb_store::Actor::system(), |w| company::set_info(w, &new))?;
            }
            Ok(_) => {}
            Err(e) => report.issues.push(SyncIssue {
                severity: Severity::Warning,
                entity: QbdEntity::Account,
                qbd_id: None,
                record: "Company information".into(),
                message: e.to_string(),
            }),
        }
    }
    let mut run = Run {
        books,
        source,
        options,
        progress,
        source_id,
        state: ResolverState {
            taxable_code,
            non_taxable_code,
            ..ResolverState::default()
        },
        report,
        open_checks: Vec::new(),
        listed_all: HashMap::new(),
        gap_links: BTreeMap::new(),
        consecutive_bridge_failures: 0,
    };
    // Files written before checks and deposits in A/R or A/P counted as open
    // items need their links before payments can apply to them.
    let repaired = books.write(
        &lb_store::Actor::system(),
        lb_store::txns::repair_ar_ap_links,
    )?;
    if repaired > 0 {
        tracing::info!(
            repaired,
            "backfilled A/R and A/P links on older transactions"
        );
    }
    for entity in QbdEntity::SYNC_ORDER {
        if !options.includes(entity) {
            continue;
        }
        if run.report.aborted.is_some() {
            break;
        }
        run.entity(entity).await?;
    }
    run.check_open_balances()?;
    if options.sweep && !options.restricted() && run.report.aborted.is_none() {
        run.sweep().await?;
    }
    run.finish()
}

impl<S: QbdSource> Run<'_, S> {
    fn issue(
        &mut self,
        severity: Severity,
        entity: QbdEntity,
        qbd_id: Option<&str>,
        record: String,
        message: String,
    ) {
        self.report.issues.push(SyncIssue {
            severity,
            entity,
            qbd_id: qbd_id.map(str::to_owned),
            record,
            message,
        });
    }

    fn progress(&self, entity: QbdEntity, phase: SyncPhase, done: usize, total: usize) {
        (self.progress)(&SyncProgress {
            entity,
            phase,
            done,
            total,
        });
    }

    /// Note a bridge failure; abort the run after too many in a row.
    fn bridge_failed(&mut self, e: &BridgeError) -> bool {
        if e.is_transient() {
            self.consecutive_bridge_failures += 1;
            if self.consecutive_bridge_failures >= self.options.max_consecutive_bridge_failures {
                self.report.aborted = Some(format!(
                    "the bridge failed {} times in a row; last: {e}",
                    self.consecutive_bridge_failures
                ));
                return true;
            }
        }
        false
    }

    async fn entity(&mut self, entity: QbdEntity) -> Result<(), SyncError> {
        let key = watermark_key(entity);
        let before = self.books.read(|c| company::setting(c, &key))?;
        let watermark = if self.options.full {
            None
        } else {
            before.clone()
        };
        {
            let er = self.report.entity_mut(entity);
            er.watermark_before.clone_from(&before);
        }
        let mut query = ListQuery {
            modified_after: watermark.clone(),
            ..ListQuery::default()
        };
        if entity.is_transaction() {
            query.limit = self.options.txn_limit;
            query.from_date = self.options.txn_from;
            query.to_date = self.options.txn_to;
        }
        self.progress(entity, SyncPhase::Listing, 0, 0);
        let rows = match self.source.list(entity, &query).await {
            Ok(rows) => {
                self.consecutive_bridge_failures = 0;
                rows
            }
            Err(e) => {
                let aborted = e.is_transient();
                self.issue(
                    Severity::Error,
                    entity,
                    None,
                    format!("{} list", entity.name()),
                    format!("couldn't list them: {e}"),
                );
                if aborted {
                    self.report.aborted = Some(format!("listing {}: {e}", entity.name()));
                }
                return Ok(());
            }
        };
        let unfiltered = query.modified_after.is_none()
            && query.limit.is_none()
            && query.from_date.is_none()
            && query.to_date.is_none();
        if unfiltered {
            self.listed_all.insert(
                entity,
                rows.iter().filter_map(Meta::of).map(|m| m.id).collect(),
            );
        }
        self.report.entity_mut(entity).listed = rows.len() as u32;
        let newest = rows
            .iter()
            .filter_map(Meta::of)
            .filter(|m| m.time_modified_at.is_some())
            .max_by_key(|m| m.time_modified_at);

        // Rows that hold the watermark back (failed, or missing a link).
        let mut hold: Vec<Meta> = Vec::new();
        let mut aborted_here = false;
        if entity.is_transaction() {
            self.transactions(entity, rows, &mut hold, &mut aborted_here)
                .await?;
        } else {
            self.list_records(entity, rows, &mut hold).await?;
        }

        // Advance the watermark.
        if !self.options.restricted() && !aborted_here {
            let oldest_held = hold
                .iter()
                .filter(|m| m.time_modified_at.is_some())
                .min_by_key(|m| m.time_modified_at);
            let after = match (oldest_held, newest) {
                (Some(h), _) => h.time_modified.clone(),
                (None, Some(n)) => n.time_modified.clone(),
                (None, None) => before.clone(),
            };
            if let Some(a) = &after
                && Some(a) != before.as_ref()
            {
                self.books.write(&lb_store::Actor::system(), |w| {
                    company::set_setting(w, &key, a)
                })?;
            }
            self.report.entity_mut(entity).watermark_after = after;
        } else {
            self.report.entity_mut(entity).watermark_after = before;
        }
        Ok(())
    }

    /// Whether a list row is unchanged since last time (and its local record still
    /// exists).
    fn unchanged(
        &self,
        entity: QbdEntity,
        meta: &Meta,
        existing: Option<&extref::ExternalRef>,
    ) -> Result<bool, StoreError> {
        if self.options.reimport {
            return Ok(false);
        }
        let Some(e) = existing else {
            return Ok(false);
        };
        if e.edit_sequence.is_none() || e.edit_sequence != meta.edit_sequence {
            return Ok(false);
        }
        // The store forgets a transaction's reference when it's deleted locally,
        // but not a list record's: check it's still there.
        if entity.is_transaction() {
            return Ok(true);
        }
        self.books.read(|c| {
            Ok(match e.local_kind.as_str() {
                "account" => accounts::find(c, lb_store::codec::parse_id(&e.local_id)?)?.is_some(),
                "entity" => entities::find(c, lb_store::codec::parse_id(&e.local_id)?)?.is_some(),
                "item" => items::find(c, lb_store::codec::parse_id(&e.local_id)?)?.is_some(),
                _ => c.query_row(
                    "SELECT EXISTS (SELECT 1 FROM list_entry WHERE id = ?1)",
                    [&e.local_id],
                    |r| r.get::<_, bool>(0),
                )?,
            })
        })
    }

    async fn item_hints(&mut self) -> HashMap<String, ItemType> {
        let mut hints = HashMap::new();
        let watermark = if self.options.full {
            None
        } else {
            self.books
                .read(|c| company::setting(c, &watermark_key(QbdEntity::Item)))
                .ok()
                .flatten()
        };
        for (list, t) in [
            (QbdEntity::ItemService, ItemType::Service),
            (QbdEntity::ItemNonInventory, ItemType::NonInventory),
            (QbdEntity::ItemInventory, ItemType::Inventory),
        ] {
            let q = ListQuery {
                modified_after: watermark.clone(),
                ..ListQuery::default()
            };
            match self.source.list(list, &q).await {
                Ok(rows) => {
                    for m in rows.iter().filter_map(Meta::of) {
                        hints.insert(m.id, t);
                    }
                }
                Err(e) => self.issue(
                    Severity::Warning,
                    QbdEntity::Item,
                    None,
                    format!("{} list", list.name()),
                    format!(
                        "couldn't list them to tell item types apart, so some may import as other charges: {e}"
                    ),
                ),
            }
        }
        hints
    }

    async fn list_records(
        &mut self,
        entity: QbdEntity,
        mut rows: Vec<Value>,
        hold: &mut Vec<Meta>,
    ) -> Result<(), SyncError> {
        let hints = if entity == QbdEntity::Item {
            self.item_hints().await
        } else {
            HashMap::new()
        };
        if entity.is_hierarchical() {
            // Parents first; groups and assemblies after the items they contain.
            let level = |v: &Value| {
                v.get("Sublevel")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0)
            };
            let container = |v: &Value| {
                [
                    "ItemGroupLine",
                    "ItemInventoryAssemblyLine",
                    "ItemSalesTaxRef",
                    "IsPrintItemsInGroup",
                ]
                .iter()
                .any(|k| v.get(*k).is_some())
            };
            rows.sort_by_key(|v| (container(v), level(v)));
        }
        let total = rows.len();
        for (i, row) in rows.iter().enumerate() {
            self.progress(entity, SyncPhase::Importing, i, total);
            let record = describe(entity.name(), row);
            let Some(meta) = Meta::of(row) else {
                self.report.entity_mut(entity).failed += 1;
                self.issue(Severity::Error, entity, None, record, "it has no id".into());
                continue;
            };
            let existing = self
                .books
                .read(|c| extref::get(c, &self.source_id, entity.name(), &meta.id))?;
            if self.unchanged(entity, &meta, existing.as_ref())? {
                self.report.entity_mut(entity).unchanged += 1;
                continue;
            }
            let hint = hints.get(&meta.id).copied();
            let source_id = self.source_id.clone();
            let state = &mut self.state;
            let result = write_record(self.books, |w| {
                import_list(
                    w,
                    state,
                    &source_id,
                    entity,
                    row,
                    &meta.id,
                    meta.edit_sequence.as_deref(),
                    existing.as_ref(),
                    hint,
                )
            });
            match result {
                Ok(done) => {
                    let er = self.report.entity_mut(entity);
                    match done.outcome {
                        Outcome::Created => er.created += 1,
                        Outcome::Updated => er.updated += 1,
                    }
                    for f in done.unmapped {
                        *self.report.unmapped_fields.entry(f).or_insert(0) += 1;
                    }
                    for n in done.notes {
                        self.issue(Severity::Info, entity, Some(&meta.id), record.clone(), n);
                    }
                    self.report.created_lists.extend(done.created_lists);
                }
                Err(e) => self.record_failure(entity, &meta, record, &e, hold),
            }
        }
        self.drain_resolver_notes(entity);
        Ok(())
    }

    fn record_failure(
        &mut self,
        entity: QbdEntity,
        meta: &Meta,
        record: String,
        e: &ImportError,
        hold: &mut Vec<Meta>,
    ) {
        if e.is_skip() {
            self.report.entity_mut(entity).skipped += 1;
            self.issue(
                Severity::Warning,
                entity,
                Some(&meta.id),
                record,
                format!("not imported: {}", e.message()),
            );
        } else {
            self.report.entity_mut(entity).failed += 1;
            self.issue(Severity::Error, entity, Some(&meta.id), record, e.message());
            hold.push(meta.clone());
        }
    }

    fn drain_resolver_notes(&mut self, entity: QbdEntity) {
        for n in std::mem::take(&mut self.state.notes) {
            self.issue(Severity::Warning, entity, None, "Lists".into(), n);
        }
    }

    async fn transactions(
        &mut self,
        entity: QbdEntity,
        mut rows: Vec<Value>,
        hold: &mut Vec<Meta>,
        aborted_here: &mut bool,
    ) -> Result<(), SyncError> {
        // Oldest first: average cost and open balances build up in date order.
        let key = |v: &Value| {
            let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
            let n = s("TxnNumber").parse::<u64>().unwrap_or(u64::MAX);
            (s("TxnDate"), n)
        };
        rows.sort_by_cached_key(key);
        let total = rows.len();
        for (i, row) in rows.iter().enumerate() {
            self.progress(entity, SyncPhase::Importing, i, total);
            let record = describe(entity.name(), row);
            let Some(meta) = Meta::of(row) else {
                self.report.entity_mut(entity).failed += 1;
                self.issue(Severity::Error, entity, None, record, "it has no id".into());
                continue;
            };
            let existing = self
                .books
                .read(|c| extref::get(c, &self.source_id, entity.name(), &meta.id))?;
            if self.unchanged(entity, &meta, existing.as_ref())? {
                self.report.entity_mut(entity).unchanged += 1;
                continue;
            }
            let detail = if entity.needs_detail() {
                match self.source.get(entity, &meta.id).await {
                    Ok(d) => {
                        self.consecutive_bridge_failures = 0;
                        d
                    }
                    Err(e) if e.is_not_found() => {
                        self.issue(
                            Severity::Info,
                            entity,
                            Some(&meta.id),
                            record,
                            "deleted in QuickBooks while the sync ran".into(),
                        );
                        continue;
                    }
                    Err(e) => {
                        self.report.entity_mut(entity).failed += 1;
                        self.issue(
                            Severity::Error,
                            entity,
                            Some(&meta.id),
                            record,
                            format!("couldn't fetch it: {e}"),
                        );
                        hold.push(meta.clone());
                        if self.bridge_failed(&e) {
                            *aborted_here = true;
                            return Ok(());
                        }
                        continue;
                    }
                }
            } else {
                row.clone()
            };
            let source_id = self.source_id.clone();
            let state = &mut self.state;
            let result = write_record(self.books, |w| {
                import_txn(
                    w,
                    state,
                    &source_id,
                    entity,
                    &detail,
                    &meta.id,
                    meta.edit_sequence.as_deref(),
                    existing.as_ref(),
                )
            });
            match result {
                Ok(done) => self.txn_imported(entity, &meta, record, done, hold),
                Err(e) => self.record_failure(entity, &meta, record, &e, hold),
            }
        }
        self.drain_resolver_notes(entity);
        Ok(())
    }

    fn txn_imported(
        &mut self,
        entity: QbdEntity,
        meta: &Meta,
        record: String,
        done: import::TxnOutcome,
        hold: &mut Vec<Meta>,
    ) {
        {
            let er = self.report.entity_mut(entity);
            match done.outcome {
                Outcome::Created => er.created += 1,
                Outcome::Updated => er.updated += 1,
            }
        }
        let mut partial = false;
        for link in &done.missing_links {
            if link.exposed {
                partial = true;
            } else if let Some(t) = &link.txn_type {
                *self.gap_links.entry(t.clone()).or_insert(0) += 1;
            }
            self.issue(
                if link.exposed {
                    Severity::Warning
                } else {
                    Severity::Info
                },
                entity,
                Some(&meta.id),
                record.clone(),
                format!("imported without its link: {link}"),
            );
        }
        if partial {
            self.report.entity_mut(entity).partial += 1;
            hold.push(meta.clone());
        }
        if done.dropped_credits > 0 {
            self.issue(
                Severity::Info,
                entity,
                Some(&meta.id),
                record.clone(),
                format!(
                    "{} credit(s) listed against what it pays were already recorded with another payment",
                    done.dropped_credits
                ),
            );
        }
        for f in done.unmapped {
            *self.report.unmapped_fields.entry(f).or_insert(0) += 1;
        }
        for n in done.notes {
            self.issue(Severity::Info, entity, Some(&meta.id), record.clone(), n);
        }
        self.report.created_lists.extend(done.created_lists);
        let mut mismatch = |kind, qbd: Money, ours: Money| {
            self.report.mismatches.push(Mismatch {
                entity,
                qbd_id: meta.id.clone(),
                record: record.clone(),
                kind,
                qbd,
                ours,
            });
        };
        if done.is_posting && !done.is_void {
            if let Some(q) = done.qbd_total
                && q != done.total
            {
                mismatch(MismatchKind::Total, q, done.total);
            }
            let qbd_tax = done.qbd_sales_tax.unwrap_or(Money::ZERO);
            if qbd_tax != done.sales_tax {
                mismatch(MismatchKind::SalesTax, qbd_tax, done.sales_tax);
            }
        }
        if let Some(q) = done.qbd_open_balance {
            self.open_checks.push(OpenCheck {
                entity,
                qbd_id: meta.id.clone(),
                record,
                txn_id: done.txn_id,
                qbd: q,
            });
        }
    }

    /// Compare open balances once all payments are in. Where QBD shows an
    /// invoice more settled than we do and the customer has unapplied credits,
    /// apply them and compare again.
    fn check_open_balances(&mut self) -> Result<(), SyncError> {
        let checks = std::mem::take(&mut self.open_checks);
        let mut off = self.open_mismatches(&checks)?;
        if self.options.apply_credits && !off.is_empty() {
            // Only charges QBD shows more settled than we do, and only by the
            // difference: a credit QBD left unapplied must stay unapplied here.
            for (c, ours) in &off {
                let short = *ours - c.qbd;
                if !short.is_positive() || !c.entity.is_charge() {
                    continue;
                }
                self.books.write(&lb_store::Actor::system(), |w| {
                    lb_store::apply::apply_credits_to(w, c.txn_id, short)
                })?;
            }
            off = self.open_mismatches(&checks)?;
        }
        for (c, ours) in off {
            self.report.mismatches.push(Mismatch {
                entity: c.entity,
                qbd_id: c.qbd_id,
                record: c.record,
                kind: MismatchKind::OpenBalance,
                qbd: c.qbd,
                ours,
            });
        }
        Ok(())
    }

    fn open_mismatches(&self, checks: &[OpenCheck]) -> Result<Vec<(OpenCheck, Money)>, SyncError> {
        let total = checks.len();
        let mut off = Vec::new();
        for (i, c) in checks.iter().enumerate() {
            self.progress(c.entity, SyncPhase::Checking, i, total);
            let ours = import::open_balance(self.books, c.txn_id)?.unwrap_or(Money::ZERO);
            if ours != c.qbd {
                off.push((c.clone(), ours));
            }
        }
        Ok(off)
    }

    /// Delete what QBD deleted: dependants first (deposits, then payments, then
    /// the rest; lists after transactions, children before parents).
    async fn sweep(&mut self) -> Result<(), SyncError> {
        for entity in QbdEntity::SYNC_ORDER.into_iter().rev() {
            if !self.options.includes(entity) || self.report.aborted.is_some() {
                continue;
            }
            self.progress(entity, SyncPhase::Sweeping, 0, 0);
            let upstream: HashSet<String> = if let Some(ids) = self.listed_all.get(&entity) {
                ids.clone()
            } else {
                match self.source.list(entity, &ListQuery::default()).await {
                    Ok(rows) => rows.iter().filter_map(Meta::of).map(|m| m.id).collect(),
                    Err(e) => {
                        self.issue(
                            Severity::Error,
                            entity,
                            None,
                            format!("{} list", entity.name()),
                            format!("couldn't list them to find deletions: {e}"),
                        );
                        if self.bridge_failed(&e) {
                            return Ok(());
                        }
                        continue;
                    }
                }
            };
            let mut gone: Vec<extref::ExternalRef> = self
                .books
                .read(|c| extref::all_of_kind(c, &self.source_id, entity.name()))?
                .into_iter()
                .filter(|r| !upstream.contains(&r.external_id))
                .collect();
            // Newest first: local ids are time-ordered (UUIDv7) and the import
            // creates parents before children, so children go first.
            gone.sort_by_key(|r| std::cmp::Reverse(r.local_id.clone()));
            for r in gone {
                let record = format!("{} {}", entity.name(), r.external_id);
                match self.delete_local(&r) {
                    Ok(true) => self.report.entity_mut(entity).deleted += 1,
                    Ok(false) => {
                        self.report.entity_mut(entity).deactivated += 1;
                        self.issue(
                            Severity::Warning,
                            entity,
                            Some(&r.external_id),
                            record,
                            "deleted in QuickBooks but still used here, so made inactive".into(),
                        );
                    }
                    Err(e) => {
                        self.issue(
                            Severity::Error,
                            entity,
                            Some(&r.external_id),
                            record,
                            format!("deleted in QuickBooks, but couldn't be deleted here: {e}"),
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Delete one local record QBD no longer has. `Ok(false)`: still in use, made
    /// inactive instead.
    fn delete_local(&self, r: &extref::ExternalRef) -> Result<bool, StoreError> {
        let source = self.source_id.clone();
        self.books.write(&lb_store::Actor::system(), |w| {
            let id = r.local_id.as_str();
            let deleted = match r.local_kind.as_str() {
                "txn" => {
                    if let Some(t) = txns::find(&w.tx, lb_store::codec::parse_id(id)?)? {
                        txns::delete(w, t.id, t.edit_seq)?;
                    }
                    true
                }
                "account" => match accounts::find(&w.tx, lb_store::codec::parse_id(id)?)? {
                    Some(a) => match accounts::delete(w, a.id, a.edit_seq) {
                        Ok(()) => true,
                        Err(StoreError::InUse { .. }) => {
                            let mut a = a;
                            a.is_active = false;
                            accounts::save(w, a)?;
                            false
                        }
                        Err(e) => return Err(e),
                    },
                    None => true,
                },
                "entity" => match entities::find(&w.tx, lb_store::codec::parse_id(id)?)? {
                    Some(e) => match entities::delete(w, e.id, e.edit_seq) {
                        Ok(()) => true,
                        Err(StoreError::InUse { .. }) => {
                            let mut e = e;
                            e.is_active = false;
                            entities::save(w, e)?;
                            false
                        }
                        Err(err) => return Err(err),
                    },
                    None => true,
                },
                "item" => match items::find(&w.tx, lb_store::codec::parse_id(id)?)? {
                    Some(i) => match items::delete(w, i.id, i.edit_seq) {
                        Ok(()) => true,
                        Err(StoreError::InUse { .. }) => {
                            let mut i = i;
                            i.is_active = false;
                            items::save(w, i)?;
                            false
                        }
                        Err(e) => return Err(e),
                    },
                    None => true,
                },
                _ => delete_small_list(w, &r.kind, id)?,
            };
            extref::remove(w, &source, &r.kind, &r.external_id)?;
            Ok(deleted)
        })
    }

    fn finish(mut self) -> Result<SyncReport, SyncError> {
        for (t, links) in std::mem::take(&mut self.gap_links) {
            self.report.gaps.push(Gap {
                note: format!(
                    "imported records link to {links} {t} transaction(s), which the bridge doesn't expose"
                ),
                qbd_type: t,
                links,
            });
        }
        for t in NOT_EXPOSED_TXN_TYPES {
            if !self.report.gaps.iter().any(|g| g.qbd_type == t) {
                self.report.gaps.push(Gap {
                    qbd_type: t.to_owned(),
                    links: 0,
                    note:
                        "not exposed by the bridge; these transactions are missing from the books"
                            .into(),
                });
            }
        }
        self.report.gaps.push(Gap {
            qbd_type: "PayrollItemWage".into(),
            links: 0,
            note: "LibreBooks has no payroll item list yet; time entries keep the payroll item in their extra fields".into(),
        });
        self.report.requests = self.source.requests_made();
        let finished = jiff::Timestamp::now();
        self.report.finished_at = Some(finished);
        if !self.options.restricted() {
            self.books.write(&lb_store::Actor::system(), |w| {
                company::set_setting(w, SETTING_LAST_SYNC, &finished.to_string())
            })?;
        }
        Ok(self.report)
    }
}

fn delete_small_list(
    w: &mut lb_store::Writer<'_>,
    kind: &str,
    id: &str,
) -> Result<bool, StoreError> {
    use lb_core::lists::{Class, PaymentMethod, SalesRep, Terms};
    fn go<T: lists::ListRecord>(
        w: &mut lb_store::Writer<'_>,
        id: &str,
        deactivate: impl FnOnce(&mut T),
    ) -> Result<bool, StoreError> {
        let Some(rec) = lists::find::<T>(&w.tx, id)? else {
            return Ok(true);
        };
        match lists::delete::<T>(w, id, rec.edit_seq()) {
            Ok(()) => Ok(true),
            Err(StoreError::InUse { .. }) => {
                let mut rec = rec;
                deactivate(&mut rec);
                lists::save(w, rec)?;
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }
    match kind {
        "Class" => go::<Class>(w, id, |c| c.is_active = false),
        "Terms" => go::<Terms>(w, id, |t| t.is_active = false),
        "PaymentMethod" => go::<PaymentMethod>(w, id, |p| p.is_active = false),
        "SalesRep" => go::<SalesRep>(w, id, |s| s.is_active = false),
        _ => Ok(true),
    }
}

/// The stored edit sequence for a QBD record, without the partial marker.
pub fn stored_edit_sequence(r: &extref::ExternalRef) -> Option<&str> {
    r.edit_sequence
        .as_deref()
        .map(|s| s.strip_prefix(PARTIAL_PREFIX).unwrap_or(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn source_ids_prefer_the_ein() {
        assert_eq!(
            derive_source_id(&json!({"EIN": "12-3456789", "CompanyName": "X"})).as_deref(),
            Some("qbd:ein-123456789")
        );
        assert_eq!(
            derive_source_id(
                &json!({"LegalCompanyName": "Larkspur Robotics, Inc.", "CompanyName": "Larkspur"})
            )
            .as_deref(),
            Some("qbd:name-larkspur-robotics-inc")
        );
        assert_eq!(derive_source_id(&json!({})), None);
    }
}
