//! A [`QbdSource`] over any transport that can send one qbXML document and
//! return QuickBooks' answer: the Web Connector queue ([`crate::qbwc`]) and the
//! SDK's COM session ([`crate::sdk`]).
//!
//! The documents come from [`crate::qbxml::request`] with the same parameters
//! the bridge client sends, the responses go through [`crate::qbxml::xml`] into
//! the bridge's JSON shape, so the sync engine, the normaliser and the parity
//! check don't know which transport they're on. Two differences from the bridge
//! are hidden here:
//!
//! - Transaction lists ask for `IncludeLineItems` (and `IncludeLinkedTxns`), so
//!   every record arrives complete. [`QbxmlSource::get`] answers from that cache
//!   and only asks QuickBooks for a record it hasn't just listed. A full import
//!   is a few dozen round trips instead of one per transaction.
//! - `ItemQueryRq` names each item's type (`ItemOtherChargeRet`…), which the
//!   bridge's generic list drops. Each item row gets that element name under
//!   [`RET_ELEMENT_KEY`], and the sync engine uses it instead of guessing.
//!
//! **Read-only.** Every document passes [`request::check_read_only`] in
//! [`QbxmlSource::send`], the one function that hands documents to a transport.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use crate::client::{ListQuery, QbdError, QbdSource, ReportQuery};
use crate::entity::QbdEntity;
use crate::qbxml::request::{self, Paging, Request, capabilities};
use crate::qbxml::xml::{self, QueryResponse};
use crate::qbxml::{record::Meta, report};

/// The key a list row keeps its qbXML element name under (`ItemServiceRet`),
/// when the transport knows it. Stripped before the record is normalised.
pub const RET_ELEMENT_KEY: &str = "__ret_element";

/// Something that carries a qbXML document to QuickBooks and brings back the
/// response document.
pub trait QbxmlTransport: Send + Sync {
    fn exchange(&self, request: Request) -> impl Future<Output = Result<String, QbdError>> + Send;
}

/// A [`QbdSource`] speaking qbXML through `T`.
pub struct QbxmlSource<T> {
    transport: T,
    page_size: usize,
    next_id: AtomicU64,
    requests: AtomicU64,
    /// Complete transaction records from the last list, by (entity, id).
    cache: Mutex<HashMap<(QbdEntity, String), Value>>,
}

impl<T> std::fmt::Debug for QbxmlSource<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QbxmlSource")
            .field("page_size", &self.page_size)
            .field("requests", &self.requests.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

/// Rows per iterator page. qbXML pages carry whole records, lines included, so
/// pages are smaller than the bridge's.
pub const DEFAULT_PAGE_SIZE: usize = 200;

impl<T: QbxmlTransport> QbxmlSource<T> {
    pub fn new(transport: T) -> QbxmlSource<T> {
        QbxmlSource {
            transport,
            page_size: DEFAULT_PAGE_SIZE,
            next_id: AtomicU64::new(1),
            requests: AtomicU64::new(0),
            cache: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn with_page_size(mut self, page_size: usize) -> QbxmlSource<T> {
        self.page_size = page_size.max(1);
        self
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    fn request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, HashMap<(QbdEntity, String), Value>> {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Send one document and read its `…Rs` elements. The only way out to a
    /// transport, and it refuses anything that isn't a query.
    async fn send(&self, req: Request) -> Result<Vec<QueryResponse>, QbdError> {
        request::check_read_only(&req.xml)?;
        let name = req.name;
        self.requests.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(request = name, id = req.request_id, "qbXML");
        let xml_doc = self.transport.exchange(req).await?;
        xml::parse_response(&xml_doc).map_err(|e| QbdError::Garbled {
            path: name.to_owned(),
            message: e.to_string(),
        })
    }

    /// Send one request and take its one response, turning a QuickBooks error
    /// status into [`QbdError::Qbxml`].
    async fn query(&self, req: Request) -> Result<QueryResponse, QbdError> {
        let name = req.name;
        let expected_rs = format!("{}Rs", name.trim_end_matches("Rq"));
        let responses = self.send(req).await?;
        let rs = responses
            .into_iter()
            .find(|r| r.name == expected_rs)
            .ok_or_else(|| QbdError::Garbled {
                path: name.to_owned(),
                message: format!("the response has no {expected_rs}"),
            })?;
        if rs.is_ok() || rs.is_warning() {
            return Ok(rs);
        }
        Err(QbdError::Qbxml {
            request: name.to_owned(),
            code: rs.status_code,
            severity: rs.status_severity,
            message: rs.status_message,
        })
    }

    /// Keep the records of a list page: full transactions for `get`, and the
    /// element name on items.
    fn absorb(
        &self,
        entity: QbdEntity,
        records: Vec<(String, Value)>,
        limit: Option<usize>,
    ) -> Vec<Value> {
        let mut out = Vec::with_capacity(records.len());
        let mut cache = self.cache();
        for (element, mut value) in records {
            if limit.is_some_and(|l| out.len() >= l) {
                break;
            }
            if entity == QbdEntity::Item
                && let Value::Object(m) = &mut value
            {
                m.insert(RET_ELEMENT_KEY.into(), Value::String(element));
            }
            if entity.needs_detail()
                && let Some(meta) = Meta::of(&value)
            {
                cache.insert((entity, meta.id), value.clone());
            }
            out.push(value);
        }
        out
    }
}

impl<T: QbxmlTransport> QbdSource for QbxmlSource<T> {
    async fn list(&self, entity: QbdEntity, query: &ListQuery) -> Result<Vec<Value>, QbdError> {
        let caps = capabilities(entity);
        let limit = query.limit;
        // Lists that can't iterate come back whole (no MaxReturned means
        // everything); a limited run asks for just the limit.
        let paginate = caps.iterator && limit.is_none_or(|l| l > self.page_size);
        if !paginate {
            let rs = self
                .query(request::list(
                    entity,
                    query,
                    &Paging::Whole { limit },
                    self.request_id(),
                ))
                .await?;
            return Ok(self.absorb(entity, rs.records, limit));
        }
        let mut out = Vec::new();
        let mut paging = Paging::Start {
            page: self.page_size,
        };
        let mut expected: Option<usize> = None;
        loop {
            let rs = self
                .query(request::list(entity, query, &paging, self.request_id()))
                .await?;
            let remaining = rs.iterator_remaining_count.unwrap_or(0) as usize;
            let got = rs.records.len();
            if expected.is_none() {
                expected = Some(got + remaining);
            }
            let want = limit.map(|l| l.saturating_sub(out.len()));
            out.extend(self.absorb(entity, rs.records, want));
            let done = limit.is_some_and(|l| out.len() >= l) || remaining == 0 || got == 0;
            match (done, rs.iterator_id) {
                (false, Some(id)) => {
                    paging = Paging::Continue {
                        iterator_id: id,
                        page: self.page_size,
                    };
                }
                _ => break,
            }
        }
        if let Some(expected) = expected
            && limit.is_none()
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

    async fn get(&self, entity: QbdEntity, id: &str) -> Result<Value, QbdError> {
        if let Some(v) = self.cache().remove(&(entity, id.to_owned())) {
            return Ok(v);
        }
        let rs = self
            .query(request::get(entity, id, self.request_id()))
            .await?;
        let name = request::query_name(entity);
        rs.records
            .into_iter()
            .next()
            .map(|(_, v)| v)
            .ok_or_else(|| QbdError::Qbxml {
                request: name.to_owned(),
                code: QbdError::QBXML_NOT_FOUND,
                severity: "Error".into(),
                message: format!("QuickBooks has no {} {id}", entity.name()),
            })
    }

    async fn company(&self) -> Result<Value, QbdError> {
        let rs = self.query(request::company(self.request_id())).await?;
        rs.records
            .into_iter()
            .find(|(n, _)| n == "CompanyRet")
            .map(|(_, v)| v)
            .ok_or_else(|| QbdError::Garbled {
                path: "CompanyQueryRq".into(),
                message: "no CompanyRet in the response".into(),
            })
    }

    async fn preferences(&self) -> Result<Value, QbdError> {
        let rs = self.query(request::preferences(self.request_id())).await?;
        rs.records
            .into_iter()
            .find(|(n, _)| n == "PreferencesRet")
            .map(|(_, v)| v)
            .ok_or_else(|| QbdError::Garbled {
                path: "PreferencesQueryRq".into(),
                message: "no PreferencesRet in the response".into(),
            })
    }

    async fn report(&self, slug: &str, query: &ReportQuery) -> Result<Value, QbdError> {
        let req = request::report(slug, query, self.request_id())?;
        let name = req.name;
        let rs = self.query(req).await?;
        rs.records
            .into_iter()
            .find(|(n, _)| n == "ReportRet")
            .map(|(_, v)| report::to_bridge_report(&v))
            .ok_or_else(|| QbdError::Garbled {
                path: name.to_owned(),
                message: "no ReportRet in the response".into(),
            })
    }

    fn requests_made(&self) -> Option<u64> {
        Some(self.requests.load(Ordering::Relaxed))
    }
}
