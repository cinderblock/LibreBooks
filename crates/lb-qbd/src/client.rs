//! The QB Bridge client: read-only by construction.
//!
//! The bridge (`github.com/cinderblock/quickbooks-desktop-sdk-bridge`) is a REST
//! front end to a running QuickBooks Desktop. It accepts writes too; this client
//! cannot make one. Every request goes through [`BridgeClient::get_once`], which
//! is hard-wired to HTTP GET, and no other function in this crate talks HTTP (a
//! test scans the sources to keep it that way). LibreBooks reads the user's live
//! books for months before QuickBooks is retired, so "can't" matters more than
//! "doesn't".
//!
//! Every request also reaches the real QuickBooks on the user's desk, which is in
//! use while we sync. So requests are strictly one at a time by default, with a
//! small pause between them, generous timeouts (a cold QuickBooks connection took
//! 19 s to answer `/company`), and retries with backoff on the failures a busy
//! QuickBooks produces.
//!
//! Paging: the bridge's lists return 100 rows unless asked for more and can page
//! only with a QuickBooks iterator, which several lists don't support (the bridge
//! says `ITERATOR_NOT_SUPPORTED`). [`BridgeClient::list`] hides all of that: it
//! iterates where it can, otherwise asks for the maximum (5000) in one call, and
//! for a transaction list that comes back full it splits the date range until
//! every piece fits.

use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use jiff::civil::Date;
use serde_json::Value;
use url::Url;

use crate::entity::QbdEntity;

/// The most rows the bridge returns in one list call.
pub const MAX_RETURNED: usize = 5000;

/// The earliest and latest dates QuickBooks accepts, for splitting a date range.
const FIRST_DATE: Date = jiff::civil::date(1901, 1, 1);
const LAST_DATE: Date = jiff::civil::date(2099, 12, 31);

/// How to reach the bridge and how gently to treat it.
#[derive(Clone)]
pub struct BridgeConfig {
    /// e.g. `http://10.255.0.18:8743`. Use an IPv4 literal: the bridge refuses
    /// requests that arrive over (public) IPv6 with `FORBIDDEN_IP`.
    pub base_url: Url,
    /// Sent as `X-API-Key`. Never logged.
    pub api_key: String,
    /// Per request, start to finish.
    pub timeout: Duration,
    pub connect_timeout: Duration,
    /// Pause between one request finishing and the next starting.
    pub politeness_delay: Duration,
    /// Requests allowed in flight at once. 1 (strictly sequential) unless the
    /// user's QuickBooks is known to cope with more.
    pub max_in_flight: usize,
    /// Retries after a transient failure, with exponential backoff from
    /// `retry_delay`.
    pub retries: u32,
    pub retry_delay: Duration,
    /// Rows per iterator page.
    pub page_size: usize,
}

impl fmt::Debug for BridgeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgeConfig")
            .field("base_url", &self.base_url.as_str())
            .field("api_key", &"<redacted>")
            .field("timeout", &self.timeout)
            .field("connect_timeout", &self.connect_timeout)
            .field("politeness_delay", &self.politeness_delay)
            .field("max_in_flight", &self.max_in_flight)
            .field("retries", &self.retries)
            .field("retry_delay", &self.retry_delay)
            .field("page_size", &self.page_size)
            .finish()
    }
}

impl BridgeConfig {
    /// Defaults suited to a QuickBooks that someone is using at the same time.
    pub fn new(base_url: &str, api_key: impl Into<String>) -> Result<BridgeConfig, QbdError> {
        let base_url = Url::parse(base_url).map_err(|_| QbdError::BadUrl(base_url.to_owned()))?;
        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(QbdError::BadUrl(base_url.to_string()));
        }
        Ok(BridgeConfig {
            base_url,
            api_key: api_key.into(),
            timeout: Duration::from_secs(120),
            connect_timeout: Duration::from_secs(10),
            politeness_delay: Duration::from_millis(100),
            max_in_flight: 1,
            retries: 3,
            retry_delay: Duration::from_millis(500),
            page_size: 500,
        })
    }
}

/// Read `key` from a `.env`-style file (`KEY=value` lines, optional quotes), so
/// the API key can come from where the user already keeps it without ever
/// being printed.
pub fn read_env_value(path: &Path, key: &str) -> std::io::Result<Option<String>> {
    let text = std::fs::read_to_string(path)?;
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        if let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                .unwrap_or(v);
            return Ok(Some(v.to_owned()));
        }
    }
    Ok(None)
}

/// Which records a list includes by active status. The bridge defaults to
/// active only; an import wants everything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Active {
    #[default]
    All,
    ActiveOnly,
    InactiveOnly,
}

impl Active {
    pub const fn as_str(self) -> &'static str {
        match self {
            Active::All => "All",
            Active::ActiveOnly => "ActiveOnly",
            Active::InactiveOnly => "InactiveOnly",
        }
    }
}

/// Filters for a list. Empty means everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListQuery {
    /// QBD `TimeModified` at or after this (inclusive). QBD's own `TimeModified`
    /// strings work as they are.
    pub modified_after: Option<String>,
    /// Transactions dated on or after / on or before.
    pub from_date: Option<Date>,
    pub to_date: Option<Date>,
    /// Name contains (lists).
    pub name: Option<String>,
    /// Customer/vendor name contains (transactions).
    pub entity_name: Option<String>,
    pub active: Active,
    /// Stop after this many rows (for light, partial runs).
    pub limit: Option<usize>,
}

impl ListQuery {
    fn params(&self, entity: QbdEntity) -> Vec<(&'static str, String)> {
        let mut p = Vec::new();
        if !entity.is_transaction() {
            p.push(("active", self.active.as_str().to_owned()));
        }
        if let Some(m) = &self.modified_after {
            p.push(("modified_after", m.clone()));
        }
        if let Some(d) = self.from_date {
            p.push(("from_date", d.to_string()));
        }
        if let Some(d) = self.to_date {
            p.push(("to_date", d.to_string()));
        }
        if let Some(n) = &self.name {
            p.push(("name", n.clone()));
        }
        if let Some(n) = &self.entity_name {
            p.push(("entity_name", n.clone()));
        }
        p
    }
}

/// Report basis. Always say which: QBD's default comes from a company preference
/// (the user's is Cash).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    Accrual,
    Cash,
}

impl Basis {
    pub const fn as_str(self) -> &'static str {
        match self {
            Basis::Accrual => "Accrual",
            Basis::Cash => "Cash",
        }
    }
}

/// Parameters for `GET /api/v1/reports/{slug}`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReportQuery {
    pub from_date: Option<Date>,
    pub to_date: Option<Date>,
    pub date_macro: Option<String>,
    pub basis: Option<Basis>,
    pub summarize_by: Option<String>,
    /// Customer/job/vendor full name.
    pub entity: Option<String>,
}

impl ReportQuery {
    fn params(&self) -> Vec<(&'static str, String)> {
        let mut p = Vec::new();
        if let Some(d) = self.from_date {
            p.push(("from_date", d.to_string()));
        }
        if let Some(d) = self.to_date {
            p.push(("to_date", d.to_string()));
        }
        if let Some(m) = &self.date_macro {
            p.push(("date_macro", m.clone()));
        }
        if let Some(b) = self.basis {
            p.push(("basis", b.as_str().to_owned()));
        }
        if let Some(s) = &self.summarize_by {
            p.push(("summarize_by", s.clone()));
        }
        if let Some(e) = &self.entity {
            p.push(("entity", e.clone()));
        }
        p
    }
}

/// What went wrong talking to QuickBooks, through whichever transport (the
/// bridge, the Web Connector queue, the SDK). Messages are for people.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QbdError {
    #[error("{0:?} isn't a usable bridge address")]
    BadUrl(String),
    #[error("can't reach the QuickBooks bridge for {path}: {message}")]
    Unreachable { path: String, message: String },
    #[error("the QuickBooks bridge didn't answer {path} within the time allowed")]
    Timeout { path: String },
    #[error("the QuickBooks bridge refused {path} (HTTP {status}{}): {message}", code_suffix(.code.as_deref()))]
    Refused {
        path: String,
        status: u16,
        code: Option<String>,
        message: String,
    },
    #[error("the QuickBooks bridge sent something unreadable for {path}: {message}")]
    Garbled { path: String, message: String },
    #[error(
        "QuickBooks has {max} or more {entity} records and the bridge can't page that list, so some would be missed"
    )]
    Truncated { entity: &'static str, max: usize },
    #[error(
        "an iterator over {entity} returned {got} records where QuickBooks promised {expected}"
    )]
    IteratorInconsistent {
        entity: &'static str,
        expected: usize,
        got: usize,
    },
    /// QuickBooks answered a qbXML request with an error status (the Web
    /// Connector and SDK transports). `code` is QuickBooks' `statusCode`
    /// (3120 is "not found"; 1 is "no match", which isn't an error).
    #[error("QuickBooks refused {request} (status {code}, {severity}): {message}")]
    Qbxml {
        request: String,
        code: i64,
        severity: String,
        message: String,
    },
    /// The transport's session with QuickBooks ended or never started (the Web
    /// Connector reported a connection error, the SDK couldn't open the company,
    /// the queue was closed). The whole sync should stop, not one record.
    #[error("the QuickBooks session is gone: {message}")]
    Session { message: String },
    /// The transport can't do this (a report the qbXML builder doesn't know).
    #[error("{0}")]
    Unsupported(String),
}

fn code_suffix(code: Option<&str>) -> String {
    code.map(|c| format!(", {c}")).unwrap_or_default()
}

impl QbdError {
    /// qbXML's "object not found" status.
    pub const QBXML_NOT_FOUND: i64 = 3120;
    /// qbXML's "QuickBooks is busy / another application is modal" statuses,
    /// worth retrying.
    const QBXML_BUSY: [i64; 2] = [0x8004_0408, 0x8004_041A];

    /// The bridge's error code (`NOT_FOUND`, `ITERATOR_NOT_SUPPORTED`…), if any.
    pub fn code(&self) -> Option<&str> {
        match self {
            QbdError::Refused { code, .. } => code.as_deref(),
            _ => None,
        }
    }

    /// Worth trying again, or a reason to stop the whole run rather than mark
    /// one record failed: the network, a timeout, a server-side failure that a
    /// busy QuickBooks produces (5xx, 429), a QuickBooks "busy" status, or the
    /// transport's session going away.
    pub fn is_transient(&self) -> bool {
        match self {
            QbdError::Unreachable { .. } | QbdError::Timeout { .. } | QbdError::Session { .. } => {
                true
            }
            QbdError::Refused { status, .. } => *status == 429 || *status >= 500,
            QbdError::Qbxml { code, .. } => Self::QBXML_BUSY.contains(code),
            _ => false,
        }
    }

    pub fn is_not_found(&self) -> bool {
        match self {
            QbdError::Refused { status, .. } => *status == 404,
            QbdError::Qbxml { code, .. } => *code == Self::QBXML_NOT_FOUND,
            _ => false,
        }
    }
}

/// Where QuickBooks records come from. [`BridgeClient`] reads through the QB
/// Bridge; [`crate::source::QbxmlSource`] speaks qbXML through the Web Connector
/// queue ([`crate::qbwc`]) or the SDK ([`crate::sdk`], Windows); tests use
/// fixtures ([`crate::testing::FixtureSource`]). The sync engine and parity check
/// take any of them.
pub trait QbdSource: Send + Sync {
    /// Every record of `entity` matching `query`, all pages. Transactions come
    /// without their lines (as the bridge's lists do); use [`QbdSource::get`].
    fn list(
        &self,
        entity: QbdEntity,
        query: &ListQuery,
    ) -> impl Future<Output = Result<Vec<Value>, QbdError>> + Send;

    /// One record, complete (lines, applied-to transactions and all).
    fn get(
        &self,
        entity: QbdEntity,
        id: &str,
    ) -> impl Future<Output = Result<Value, QbdError>> + Send;

    /// `CompanyRet`.
    fn company(&self) -> impl Future<Output = Result<Value, QbdError>> + Send;

    /// `PreferencesRet`.
    fn preferences(&self) -> impl Future<Output = Result<Value, QbdError>> + Send;

    /// A QBD report (`trial-balance`, `general-ledger`, …).
    fn report(
        &self,
        slug: &str,
        query: &ReportQuery,
    ) -> impl Future<Output = Result<Value, QbdError>> + Send;

    /// Requests made so far, where the source counts them.
    fn requests_made(&self) -> Option<u64> {
        None
    }
}

/// The bridge client. Cheap to share by reference; it serialises its own
/// requests.
pub struct BridgeClient {
    http: reqwest::Client,
    config: BridgeConfig,
    gate: tokio::sync::Semaphore,
    last_finished: tokio::sync::Mutex<Option<Instant>>,
    /// Lists the bridge has told us can't iterate (beyond the known ones).
    no_iterator: Mutex<HashSet<QbdEntity>>,
    requests: AtomicU64,
}

impl fmt::Debug for BridgeClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgeClient")
            .field("config", &self.config)
            .field("requests", &self.requests.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl BridgeClient {
    pub fn new(config: BridgeConfig) -> Result<BridgeClient, QbdError> {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .user_agent(concat!("LibreBooks/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| QbdError::Unreachable {
                path: String::new(),
                message: e.to_string(),
            })?;
        Ok(BridgeClient {
            http,
            gate: tokio::sync::Semaphore::new(config.max_in_flight.max(1)),
            config,
            last_finished: tokio::sync::Mutex::new(None),
            no_iterator: Mutex::new(HashSet::new()),
            requests: AtomicU64::new(0),
        })
    }

    pub fn config(&self) -> &BridgeConfig {
        &self.config
    }

    /// `GET /api/v1/health` (no QuickBooks round trip).
    pub async fn health(&self) -> Result<Value, QbdError> {
        self.get_retrying("/api/v1/health", None, &[]).await
    }

    /// `GET /api/v1/status`: the bridge's view of its QuickBooks connection. Its
    /// `qb_desktop_running` has been seen `false` while queries worked, so don't
    /// gate anything on it.
    pub async fn status(&self) -> Result<Value, QbdError> {
        Ok(data(self.get_retrying("/api/v1/status", None, &[]).await?))
    }

    /// `GET /api/v1/entities`.
    pub async fn entities(&self) -> Result<Value, QbdError> {
        Ok(data(
            self.get_retrying("/api/v1/entities", None, &[]).await?,
        ))
    }

    fn knows_no_iterator(&self, entity: QbdEntity) -> bool {
        !entity.supports_iterator()
            || self
                .no_iterator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(&entity)
    }

    /// Page through a list with a QuickBooks iterator. A failed page restarts the
    /// iteration rather than retrying the page: if QuickBooks answered a page the
    /// bridge then failed to deliver, its iterator has already moved on, and a
    /// retried "next page" would silently skip rows.
    async fn list_iterating(
        &self,
        entity: QbdEntity,
        query: &ListQuery,
    ) -> Result<Vec<Value>, QbdError> {
        let mut attempt = 0;
        loop {
            match self.iterate_once(entity, query).await {
                Err(e) if e.is_transient() && attempt < self.config.retries => {
                    tracing::warn!(entity = entity.name(), error = %e, "restarting iterator");
                    tokio::time::sleep(self.backoff(attempt)).await;
                    attempt += 1;
                }
                other => return other,
            }
        }
    }

    async fn iterate_once(
        &self,
        entity: QbdEntity,
        query: &ListQuery,
    ) -> Result<Vec<Value>, QbdError> {
        let base = query.params(entity);
        let mut out: Vec<Value> = Vec::new();
        let mut iterator = "Start".to_owned();
        let mut expected: Option<usize> = None;
        loop {
            let want = query
                .limit
                .map_or(usize::MAX, |l| l.saturating_sub(out.len()));
            if want == 0 {
                break;
            }
            let page = self.config.page_size.clamp(1, MAX_RETURNED).min(want);
            let mut params = base.clone();
            params.push(("max_returned", page.to_string()));
            params.push(("iterator_id", iterator.clone()));
            let body = self.get_once(entity.path(), None, &params).await?;
            let meta = body.get("meta").cloned().unwrap_or(Value::Null);
            let rows = rows(body, entity.path())?;
            let remaining = meta.get("remaining").and_then(Value::as_u64).unwrap_or(0) as usize;
            if expected.is_none() {
                expected = Some(rows.len() + remaining);
            }
            let got_none = rows.is_empty();
            out.extend(rows);
            match meta.get("iterator_id").and_then(Value::as_str) {
                Some(next) if remaining > 0 && !got_none => next.clone_into(&mut iterator),
                _ => break,
            }
        }
        if let Some(expected) = expected
            && query.limit.is_none()
            && out.len() != expected
        {
            return Err(QbdError::IteratorInconsistent {
                entity: entity.name(),
                expected,
                got: out.len(),
            });
        }
        Ok(out)
    }

    /// One call for everything; for transactions, split the date range while a
    /// call comes back full.
    async fn list_whole(
        &self,
        entity: QbdEntity,
        query: &ListQuery,
    ) -> Result<Vec<Value>, QbdError> {
        let cap = query
            .limit
            .map_or(MAX_RETURNED, |l| l.clamp(1, MAX_RETURNED));
        let first = self.list_call(entity, query, cap).await?;
        if first.len() < MAX_RETURNED || query.limit.is_some_and(|l| l <= MAX_RETURNED) {
            return Ok(first);
        }
        if !entity.is_transaction() {
            return Err(QbdError::Truncated {
                entity: entity.name(),
                max: MAX_RETURNED,
            });
        }
        let mut out = Vec::new();
        let mut ranges = vec![(
            query.from_date.unwrap_or(FIRST_DATE),
            query.to_date.unwrap_or(LAST_DATE),
        )];
        while let Some((from, to)) = ranges.pop() {
            let q = ListQuery {
                from_date: Some(from),
                to_date: Some(to),
                ..query.clone()
            };
            let rows = self.list_call(entity, &q, MAX_RETURNED).await?;
            if rows.len() < MAX_RETURNED {
                out.extend(rows);
                continue;
            }
            let days = from.until(to).map_or(0, |s| s.get_days());
            if days < 1 {
                return Err(QbdError::Truncated {
                    entity: entity.name(),
                    max: MAX_RETURNED,
                });
            }
            let mid = from
                .checked_add(jiff::Span::new().days(i64::from(days / 2)))
                .unwrap_or(from);
            let after_mid = mid.tomorrow().unwrap_or(to);
            // Later half pushed first so the earlier half is fetched first.
            ranges.push((after_mid, to));
            ranges.push((from, mid));
        }
        if let Some(limit) = query.limit {
            out.truncate(limit);
        }
        Ok(out)
    }

    async fn list_call(
        &self,
        entity: QbdEntity,
        query: &ListQuery,
        max: usize,
    ) -> Result<Vec<Value>, QbdError> {
        let mut params = query.params(entity);
        params.push(("max_returned", max.to_string()));
        let body = self.get_retrying(entity.path(), None, &params).await?;
        rows(body, entity.path())
    }

    fn backoff(&self, attempt: u32) -> Duration {
        self.config
            .retry_delay
            .saturating_mul(2u32.saturating_pow(attempt))
            .min(Duration::from_secs(30))
    }

    /// GET with retries on transient failures. Only for requests that are safe to
    /// repeat (everything except an iterator's next page).
    async fn get_retrying(
        &self,
        path: &str,
        id: Option<&str>,
        params: &[(&'static str, String)],
    ) -> Result<Value, QbdError> {
        let mut attempt = 0;
        loop {
            match self.get_once(path, id, params).await {
                Err(e) if e.is_transient() && attempt < self.config.retries => {
                    tracing::warn!(path, error = %e, attempt, "retrying bridge request");
                    tokio::time::sleep(self.backoff(attempt)).await;
                    attempt += 1;
                }
                other => return other,
            }
        }
    }

    /// The one function in LibreBooks that sends a request to QuickBooks. It
    /// sends GET and nothing else.
    async fn get_once(
        &self,
        path: &str,
        id: Option<&str>,
        params: &[(&'static str, String)],
    ) -> Result<Value, QbdError> {
        let url = self.url(path, id, params);
        let shown = match id {
            Some(id) => format!("{path}/{id}"),
            None => path.to_owned(),
        };
        let _permit = self
            .gate
            .acquire()
            .await
            .expect("the request gate is never closed");
        {
            let last = self.last_finished.lock().await;
            if let Some(t) = *last {
                let ready = t + self.config.politeness_delay;
                let now = Instant::now();
                if ready > now {
                    tokio::time::sleep(ready - now).await;
                }
            }
        }
        self.requests.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(path = %shown, "GET");
        let result = self
            .http
            .get(url)
            .header("X-API-Key", self.config.api_key.as_str())
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await;
        let outcome = match result {
            Ok(resp) => {
                let status = resp.status().as_u16();
                match resp.bytes().await {
                    Ok(body) => interpret(&shown, status, &body),
                    Err(e) => Err(transport_error(&shown, &e)),
                }
            }
            Err(e) => Err(transport_error(&shown, &e)),
        };
        *self.last_finished.lock().await = Some(Instant::now());
        outcome
    }

    fn url(&self, path: &str, id: Option<&str>, params: &[(&'static str, String)]) -> Url {
        let mut url = self.config.base_url.clone();
        let prefix = url.path().trim_end_matches('/').to_owned();
        url.set_path(&format!("{prefix}{path}"));
        if let Some(id) = id
            && let Ok(mut segments) = url.path_segments_mut()
        {
            segments.push(id);
        }
        url.set_query(None);
        if !params.is_empty() {
            let mut q = url.query_pairs_mut();
            for (k, v) in params {
                q.append_pair(k, v);
            }
        }
        url
    }
}

fn transport_error(path: &str, e: &reqwest::Error) -> QbdError {
    if e.is_timeout() {
        QbdError::Timeout {
            path: path.to_owned(),
        }
    } else {
        // reqwest's Display omits the cause ("error sending request"); include it.
        let mut message = e.to_string();
        let mut source = std::error::Error::source(e);
        while let Some(s) = source {
            message.push_str(": ");
            message.push_str(&s.to_string());
            source = s.source();
        }
        QbdError::Unreachable {
            path: path.to_owned(),
            message,
        }
    }
}

/// Turn a response into the envelope or an error. The bridge wraps errors two
/// ways: FastAPI's `{"detail": {"ok": false, "error": {code, message}}}` (or a
/// bare `{"detail": "Not Found"}`), and its own `{"ok": false, "error": …}`.
fn interpret(path: &str, status: u16, body: &[u8]) -> Result<Value, QbdError> {
    let parsed: Result<Value, _> = serde_json::from_slice(body);
    if !(200..300).contains(&status) {
        let (code, message) = match &parsed {
            Ok(v) => error_detail(v),
            Err(_) => (
                None,
                String::from_utf8_lossy(&body[..body.len().min(300)]).into_owned(),
            ),
        };
        return Err(QbdError::Refused {
            path: path.to_owned(),
            status,
            code,
            message,
        });
    }
    let v = parsed.map_err(|e| QbdError::Garbled {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    if v.get("ok") == Some(&Value::Bool(false)) {
        let (code, message) = error_detail(&v);
        return Err(QbdError::Refused {
            path: path.to_owned(),
            status,
            code,
            message,
        });
    }
    Ok(v)
}

fn error_detail(v: &Value) -> (Option<String>, String) {
    let inner = v.get("detail").unwrap_or(v);
    if let Some(s) = inner.as_str() {
        return (None, s.to_owned());
    }
    let err = inner.get("error").unwrap_or(inner);
    let code = err.get("code").and_then(Value::as_str).map(str::to_owned);
    let message = err
        .get("message")
        .and_then(Value::as_str)
        .map_or_else(|| err.to_string(), str::to_owned);
    (code, message)
}

/// The envelope's `data`.
fn data(v: Value) -> Value {
    match v {
        Value::Object(mut m) => m.remove("data").unwrap_or(Value::Null),
        other => other,
    }
}

/// The envelope's `data` as rows (one-or-many, like everything qbXML).
fn rows(v: Value, path: &str) -> Result<Vec<Value>, QbdError> {
    match data(v) {
        Value::Array(a) => Ok(a),
        Value::Null => Ok(Vec::new()),
        obj @ Value::Object(_) => Ok(vec![obj]),
        other => Err(QbdError::Garbled {
            path: path.to_owned(),
            message: format!("expected a list, got {other}"),
        }),
    }
}

impl QbdSource for BridgeClient {
    async fn list(&self, entity: QbdEntity, query: &ListQuery) -> Result<Vec<Value>, QbdError> {
        if !self.knows_no_iterator(entity) {
            match self.list_iterating(entity, query).await {
                Err(e) if e.code() == Some("ITERATOR_NOT_SUPPORTED") => {
                    self.no_iterator
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(entity);
                }
                other => return other,
            }
        }
        self.list_whole(entity, query).await
    }

    async fn get(&self, entity: QbdEntity, id: &str) -> Result<Value, QbdError> {
        Ok(data(self.get_retrying(entity.path(), Some(id), &[]).await?))
    }

    async fn company(&self) -> Result<Value, QbdError> {
        Ok(data(self.get_retrying("/api/v1/company", None, &[]).await?))
    }

    async fn preferences(&self) -> Result<Value, QbdError> {
        Ok(data(
            self.get_retrying("/api/v1/company/preferences", None, &[])
                .await?,
        ))
    }

    async fn report(&self, slug: &str, query: &ReportQuery) -> Result<Value, QbdError> {
        let path = format!("/api/v1/reports/{slug}");
        Ok(data(self.get_retrying(&path, None, &query.params()).await?))
    }

    fn requests_made(&self) -> Option<u64> {
        Some(self.requests.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interprets_the_bridges_error_shapes() {
        let e = interpret(
            "/x",
            400,
            br#"{"detail":{"ok":false,"error":{"code":"ITERATOR_NOT_SUPPORTED","message":"no"}}}"#,
        )
        .unwrap_err();
        assert_eq!(e.code(), Some("ITERATOR_NOT_SUPPORTED"));
        assert!(!e.is_transient());
        let e = interpret("/x", 404, br#"{"detail":"Not Found"}"#).unwrap_err();
        assert!(e.is_not_found());
        assert_eq!(
            e.to_string(),
            "the QuickBooks bridge refused /x (HTTP 404): Not Found"
        );
        let e = interpret(
            "/x",
            200,
            br#"{"ok":false,"error":{"code":"QB_ERROR","message":"busy"}}"#,
        )
        .unwrap_err();
        assert_eq!(e.code(), Some("QB_ERROR"));
        let e = interpret("/x", 503, b"<html>down</html>").unwrap_err();
        assert!(e.is_transient());
        assert!(interpret("/x", 200, br#"{"ok":true,"data":[]}"#).is_ok());
    }

    #[test]
    fn urls_keep_a_base_path_and_escape_ids() {
        let mut cfg = BridgeConfig::new("http://10.0.0.1:8743/bridge/", "k").unwrap();
        cfg.politeness_delay = Duration::ZERO;
        let c = BridgeClient::new(cfg).unwrap();
        let u = c.url(
            "/api/v1/checks",
            Some("5010-1700006100"),
            &[("max_returned", "5".into())],
        );
        assert_eq!(
            u.as_str(),
            "http://10.0.0.1:8743/bridge/api/v1/checks/5010-1700006100?max_returned=5"
        );
        let u = c.url(
            "/api/v1/accounts",
            None,
            &[("modified_after", "2026-10-19T10:43:19-08:00".into())],
        );
        assert_eq!(
            u.query(),
            Some("modified_after=2026-10-19T10%3A43%3A19-08%3A00")
        );
    }

    #[test]
    fn config_debug_hides_the_key() {
        let cfg = BridgeConfig::new("http://10.0.0.1:8743", "super-secret").unwrap();
        assert!(!format!("{cfg:?}").contains("super-secret"));
        assert!(BridgeConfig::new("ftp://x", "k").is_err());
    }

    #[test]
    fn reads_env_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".env");
        std::fs::write(&p, "A=1\nQB_API_KEY=\"abc=def\"\r\nexport B='2'\n").unwrap();
        assert_eq!(
            read_env_value(&p, "QB_API_KEY").unwrap().as_deref(),
            Some("abc=def")
        );
        assert_eq!(read_env_value(&p, "B").unwrap().as_deref(), Some("2"));
        assert_eq!(read_env_value(&p, "C").unwrap(), None);
    }
}
