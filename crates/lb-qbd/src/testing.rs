//! An in-memory QuickBooks for tests: holds complete qbXML-shaped records and
//! serves them the way the QB Bridge does (lists without lines, `modified_after`
//! inclusive, `active` filtering, the typed item lists), so the sync engine and
//! parity check run end to end without a real QuickBooks. Used by this crate's
//! tests and by the crates above it. [`FakeQuickBooks`] serves the same fixtures
//! as qbXML, for the Web Connector and SDK transports.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use crate::client::{Active, ListQuery, QbdError, QbdSource, ReportQuery};
use crate::entity::QbdEntity;

pub mod fake;

pub use fake::{FakeQuickBooks, item_ret_element, record_xml};

/// Fixture-only marker on `Item` records naming their qbXML type
/// (`Service`, `NonInventory`, `Inventory`), so the typed item lists can be
/// served. Stripped from everything served.
pub const ITEM_TYPE_KEY: &str = "__item_type";

/// Keys the bridge's list rows leave out (lines and what payments apply to).
pub fn is_detail_key(k: &str) -> bool {
    k.ends_with("LineRet")
        || k.ends_with("LineGroupRet")
        || matches!(
            k,
            "JournalDebitLine"
                | "JournalCreditLine"
                | "AppliedToTxnRet"
                | "DepositLineRet"
                | "CashBackInfoRet"
                | "LinkedTxn"
        )
}

#[derive(Debug, Default)]
pub struct FixtureSource {
    records: Mutex<BTreeMap<QbdEntity, Vec<Value>>>,
    company: Mutex<Value>,
    preferences: Mutex<Value>,
    reports: Mutex<HashMap<String, Value>>,
    requests: AtomicU64,
    gets: Mutex<Vec<(QbdEntity, String)>>,
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn id_of(v: &Value) -> Option<&str> {
    v.get("TxnID")
        .or_else(|| v.get("ListID"))
        .and_then(Value::as_str)
}

fn strip(entity: QbdEntity, v: &Value, list_row: bool) -> Value {
    let mut v = v.clone();
    if let Value::Object(m) = &mut v {
        m.remove(ITEM_TYPE_KEY);
        if list_row && entity.needs_detail() {
            m.retain(|k, _| !is_detail_key(k));
        }
    }
    v
}

impl FixtureSource {
    pub fn new(company: Value, preferences: Value) -> FixtureSource {
        FixtureSource {
            company: Mutex::new(company),
            preferences: Mutex::new(preferences),
            ..FixtureSource::default()
        }
    }

    /// Load `company.json`, `preferences.json`, one `<Entity>.json` array per QBD
    /// entity present, and `reports/<slug>.json`.
    pub fn load_dir(dir: &Path) -> std::io::Result<FixtureSource> {
        let read = |p: &Path| -> std::io::Result<Value> {
            let text = std::fs::read_to_string(p)?;
            serde_json::from_str(&text).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{}: {e}", p.display()),
                )
            })
        };
        let src = FixtureSource::new(
            read(&dir.join("company.json"))?,
            read(&dir.join("preferences.json"))?,
        );
        for e in QbdEntity::ALL {
            let p = dir.join(format!("{}.json", e.name()));
            if p.exists()
                && let Value::Array(rows) = read(&p)?
            {
                lock(&src.records).insert(e, rows);
            }
        }
        let reports = dir.join("reports");
        if reports.is_dir() {
            for entry in std::fs::read_dir(reports)? {
                let p = entry?.path();
                if let Some(slug) = p.file_stem().and_then(|s| s.to_str()) {
                    src.set_report(slug, read(&p)?);
                }
            }
        }
        Ok(src)
    }

    /// Add a record, or replace the one with the same id.
    pub fn put(&self, entity: QbdEntity, record: Value) {
        let mut all = lock(&self.records);
        let rows = all.entry(entity).or_default();
        let id = id_of(&record).map(str::to_owned);
        match rows.iter_mut().find(|r| id_of(r).map(str::to_owned) == id) {
            Some(r) => *r = record,
            None => rows.push(record),
        }
    }

    /// Replace every record of a kind at once.
    pub fn set_records(&self, entity: QbdEntity, rows: Vec<Value>) {
        lock(&self.records).insert(entity, rows);
    }

    /// Change a record in place (bump its `EditSequence` and `TimeModified` in
    /// `f` to look like an edit in QBD).
    pub fn update(&self, entity: QbdEntity, id: &str, f: impl FnOnce(&mut Value)) -> bool {
        let mut all = lock(&self.records);
        match all
            .get_mut(&entity)
            .and_then(|rows| rows.iter_mut().find(|r| id_of(r) == Some(id)))
        {
            Some(r) => {
                f(r);
                true
            }
            None => false,
        }
    }

    pub fn remove(&self, entity: QbdEntity, id: &str) -> bool {
        let mut all = lock(&self.records);
        let Some(rows) = all.get_mut(&entity) else {
            return false;
        };
        let before = rows.len();
        rows.retain(|r| id_of(r) != Some(id));
        rows.len() != before
    }

    pub fn records(&self, entity: QbdEntity) -> Vec<Value> {
        lock(&self.records)
            .get(&entity)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_report(&self, slug: &str, v: Value) {
        lock(&self.reports).insert(slug.to_owned(), v);
    }

    /// Every `get` so far (entity, id), to check what a sync fetched.
    pub fn gets(&self) -> Vec<(QbdEntity, String)> {
        lock(&self.gets).clone()
    }

    pub fn clear_gets(&self) {
        lock(&self.gets).clear();
    }

    fn not_found(path: String) -> QbdError {
        QbdError::Refused {
            path,
            status: 404,
            code: Some("NOT_FOUND".into()),
            message: "not found".into(),
        }
    }

    /// The list a bridge request would return (no lines), synchronously.
    pub fn list_now(&self, entity: QbdEntity, q: &ListQuery) -> Vec<Value> {
        self.list_records(entity, q, false)
    }

    /// One record as `get` serves it, synchronously; `with_detail` false leaves
    /// the lines out (a qbXML query without `IncludeLineItems`).
    pub fn get_now(&self, entity: QbdEntity, id: &str, with_detail: bool) -> Option<Value> {
        let all = lock(&self.records);
        all.get(&entity)
            .and_then(|rows| rows.iter().find(|r| id_of(r) == Some(id)))
            .map(|r| strip(entity, r, !with_detail))
    }

    pub fn company_now(&self) -> Value {
        lock(&self.company).clone()
    }

    pub fn preferences_now(&self) -> Value {
        lock(&self.preferences).clone()
    }

    pub fn report_now(&self, slug: &str) -> Option<Value> {
        lock(&self.reports).get(slug).cloned()
    }

    /// The list, with or without the detail keys the bridge's rows leave out.
    pub fn list_records(&self, entity: QbdEntity, q: &ListQuery, with_detail: bool) -> Vec<Value> {
        self.select(entity, q)
            .iter()
            .map(|r| strip(entity, r, !with_detail))
            .collect()
    }

    /// One record exactly as stored (fixture markers included), for the fake
    /// QuickBooks, which needs the item type marker to name the element.
    pub fn get_raw(&self, entity: QbdEntity, id: &str) -> Option<Value> {
        let all = lock(&self.records);
        all.get(&entity)
            .and_then(|rows| rows.iter().find(|r| id_of(r) == Some(id)))
            .cloned()
    }

    /// The records a query selects, exactly as stored (fixture markers
    /// included), counting one request.
    pub fn select(&self, entity: QbdEntity, q: &ListQuery) -> Vec<Value> {
        self.requests.fetch_add(1, Ordering::Relaxed);
        let typed = match entity {
            QbdEntity::ItemService => Some("Service"),
            QbdEntity::ItemNonInventory => Some("NonInventory"),
            QbdEntity::ItemInventory => Some("Inventory"),
            _ => None,
        };
        let from = if typed.is_some() {
            QbdEntity::Item
        } else {
            entity
        };
        let all = lock(&self.records);
        let rows = all.get(&from).cloned().unwrap_or_default();
        drop(all);
        let after: Option<jiff::Timestamp> =
            q.modified_after.as_deref().and_then(|s| s.parse().ok());
        let mut out = Vec::new();
        for r in rows {
            if let Some(t) = typed
                && r.get(ITEM_TYPE_KEY).and_then(Value::as_str) != Some(t)
            {
                continue;
            }
            if let Some(after) = after {
                let m: Option<jiff::Timestamp> = r
                    .get("TimeModified")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok());
                if m.is_none_or(|m| m < after) {
                    continue;
                }
            }
            let date: Option<jiff::civil::Date> = r
                .get("TxnDate")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok());
            if let (Some(f), Some(d)) = (q.from_date, date)
                && d < f
            {
                continue;
            }
            if let (Some(t), Some(d)) = (q.to_date, date)
                && d > t
            {
                continue;
            }
            if !entity.is_transaction() {
                let active = r.get("IsActive").and_then(Value::as_str) != Some("false");
                match q.active {
                    Active::ActiveOnly if !active => continue,
                    Active::InactiveOnly if active => continue,
                    _ => {}
                }
            }
            out.push(r);
            if q.limit.is_some_and(|l| out.len() >= l) {
                break;
            }
        }
        out
    }
}

// The fixture answers at once: each method does its work and hands back a ready
// future.
impl QbdSource for FixtureSource {
    fn list(
        &self,
        entity: QbdEntity,
        q: &ListQuery,
    ) -> impl Future<Output = Result<Vec<Value>, QbdError>> + Send {
        std::future::ready(Ok(self.list_now(entity, q)))
    }

    fn get(
        &self,
        entity: QbdEntity,
        id: &str,
    ) -> impl Future<Output = Result<Value, QbdError>> + Send {
        self.requests.fetch_add(1, Ordering::Relaxed);
        lock(&self.gets).push((entity, id.to_owned()));
        let all = lock(&self.records);
        std::future::ready(
            all.get(&entity)
                .and_then(|rows| rows.iter().find(|r| id_of(r) == Some(id)))
                .map(|r| strip(entity, r, false))
                .ok_or_else(|| Self::not_found(format!("{}/{id}", entity.path()))),
        )
    }

    fn company(&self) -> impl Future<Output = Result<Value, QbdError>> + Send {
        self.requests.fetch_add(1, Ordering::Relaxed);
        std::future::ready(Ok(lock(&self.company).clone()))
    }

    fn preferences(&self) -> impl Future<Output = Result<Value, QbdError>> + Send {
        self.requests.fetch_add(1, Ordering::Relaxed);
        std::future::ready(Ok(lock(&self.preferences).clone()))
    }

    fn report(
        &self,
        slug: &str,
        _q: &ReportQuery,
    ) -> impl Future<Output = Result<Value, QbdError>> + Send {
        self.requests.fetch_add(1, Ordering::Relaxed);
        std::future::ready(
            lock(&self.reports)
                .get(slug)
                .cloned()
                .ok_or_else(|| Self::not_found(format!("/api/v1/reports/{slug}"))),
        )
    }

    fn requests_made(&self) -> Option<u64> {
        Some(self.requests.load(Ordering::Relaxed))
    }
}
