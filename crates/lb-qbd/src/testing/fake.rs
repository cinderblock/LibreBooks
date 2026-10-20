//! A QuickBooks that speaks qbXML: the fixtures served the way QuickBooks
//! itself answers `…QueryRq` documents, so the Web Connector and SDK transports
//! can be tested end to end (and the fake Web Connector client in the server's
//! tests has a QuickBooks to run the requests in).
//!
//! It mirrors what matters about QuickBooks: iterators (`Start` / `Continue`,
//! `iteratorRemainingCount`, the same `iteratorID` on every page), `MaxReturned`
//! (and *everything* when it's absent), `IncludeLineItems` (lines are left out
//! without it), `ActiveStatus`, `FromModifiedDate`, `TxnDateRangeFilter`, one
//! record by `ListID`/`TxnID` (3120 when it doesn't exist), status 1 for "no
//! match", typed item elements, and reports as `ReportRet`. Failures can be
//! injected per request type.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;

use lb_core::ItemType;
use quick_xml::escape::escape;
use serde_json::Value;

use super::{FixtureSource, ITEM_TYPE_KEY, is_detail_key, lock};
use crate::client::{Active, ListQuery, QbdError};
use crate::entity::QbdEntity;
use crate::qbxml::lists::classify_item;
use crate::qbxml::request::{self, Request};
use crate::qbxml::xml::{Node, parse_tree, to_xml};
use crate::qbxml::{report, xml};
use crate::source::QbxmlTransport;

/// A failure to inject: this status on every request of one kind.
#[derive(Clone, Debug)]
struct Failure {
    code: i64,
    message: String,
}

/// The rows an iterator hasn't handed out yet.
#[derive(Debug)]
struct OpenIterator {
    rest: Vec<Value>,
}

#[derive(Debug)]
pub struct FakeQuickBooks {
    fixtures: FixtureSource,
    iterators: Mutex<HashMap<String, OpenIterator>>,
    failures: Mutex<HashMap<String, Failure>>,
    requests: Mutex<Vec<String>>,
    next_iterator: Mutex<u32>,
}

/// The qbXML element for an item record: the fixture's type marker, else the
/// shape (as the normaliser classifies it).
pub fn item_ret_element(v: &Value) -> &'static str {
    let hinted = match v.get(ITEM_TYPE_KEY).and_then(Value::as_str) {
        Some("Service") => Some(ItemType::Service),
        Some("NonInventory") => Some(ItemType::NonInventory),
        Some("Inventory") => Some(ItemType::Inventory),
        _ => None,
    };
    match classify_item(v, hinted) {
        ItemType::Service => "ItemServiceRet",
        ItemType::Inventory => "ItemInventoryRet",
        ItemType::InventoryAssembly => "ItemInventoryAssemblyRet",
        ItemType::NonInventory => "ItemNonInventoryRet",
        ItemType::OtherCharge => "ItemOtherChargeRet",
        ItemType::Subtotal => "ItemSubtotalRet",
        ItemType::Group => "ItemGroupRet",
        ItemType::Discount => "ItemDiscountRet",
        ItemType::Payment => "ItemPaymentRet",
        ItemType::SalesTax => "ItemSalesTaxRet",
        ItemType::SalesTaxGroup => "ItemSalesTaxGroupRet",
        ItemType::FixedAsset => "ItemFixedAssetRet",
    }
}

fn ret_element(entity: QbdEntity, v: &Value) -> &'static str {
    match entity {
        QbdEntity::Item => item_ret_element(v),
        QbdEntity::Terms if v.get("DayOfMonthDue").is_some() => "DateDrivenTermsRet",
        other => other.ret_element(),
    }
}

/// `TrialBalance` → `trial-balance`.
fn kebab(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

struct Rs {
    name: String,
    request_id: Option<String>,
    status: (i64, &'static str, String),
    iterator: Option<(usize, String)>,
    body: String,
}

impl Rs {
    fn ok(name: &str, request_id: Option<String>) -> Rs {
        Rs {
            name: name.to_owned(),
            request_id,
            status: (0, "Info", "Status OK".into()),
            iterator: None,
            body: String::new(),
        }
    }

    fn error(name: &str, request_id: Option<String>, code: i64, message: impl Into<String>) -> Rs {
        Rs {
            name: name.to_owned(),
            request_id,
            status: (code, "Error", message.into()),
            iterator: None,
            body: String::new(),
        }
    }

    fn render(&self, out: &mut String) {
        out.push('<');
        out.push_str(&self.name);
        if let Some(id) = &self.request_id {
            out.push_str(&format!(" requestID=\"{}\"", escape(id.as_str())));
        }
        out.push_str(&format!(
            " statusCode=\"{}\" statusSeverity=\"{}\" statusMessage=\"{}\"",
            self.status.0,
            self.status.1,
            escape(self.status.2.as_str())
        ));
        if let Some((remaining, id)) = &self.iterator {
            out.push_str(&format!(
                " iteratorRemainingCount=\"{remaining}\" iteratorID=\"{}\"",
                escape(id.as_str())
            ));
        }
        if self.body.is_empty() {
            out.push_str(" />");
        } else {
            out.push('>');
            out.push_str(&self.body);
            out.push_str("</");
            out.push_str(&self.name);
            out.push('>');
        }
    }
}

impl FakeQuickBooks {
    pub fn new(fixtures: FixtureSource) -> FakeQuickBooks {
        FakeQuickBooks {
            fixtures,
            iterators: Mutex::new(HashMap::new()),
            failures: Mutex::new(HashMap::new()),
            requests: Mutex::new(Vec::new()),
            next_iterator: Mutex::new(0),
        }
    }

    pub fn fixtures(&self) -> &FixtureSource {
        &self.fixtures
    }

    /// Make every `query_name` request (`CheckQueryRq`) fail with this status.
    pub fn fail(&self, query_name: &str, code: i64, message: &str) {
        lock(&self.failures).insert(
            query_name.to_owned(),
            Failure {
                code,
                message: message.to_owned(),
            },
        );
    }

    pub fn clear_failure(&self, query_name: &str) {
        lock(&self.failures).remove(query_name);
    }

    /// Every request element name received, in order.
    pub fn requests(&self) -> Vec<String> {
        lock(&self.requests).clone()
    }

    pub fn open_iterators(&self) -> usize {
        lock(&self.iterators).len()
    }

    /// Answer a whole qbXML document.
    pub fn answer(&self, request_xml: &str) -> String {
        let mut out = String::from("<?xml version=\"1.0\" ?><QBXML><QBXMLMsgsRs>");
        match parse_tree(request_xml) {
            Ok(root) if root.name == "QBXML" => {
                for msgs in &root.children {
                    for rq in &msgs.children {
                        lock(&self.requests).push(rq.name.clone());
                        self.one(rq).render(&mut out);
                    }
                }
            }
            _ => {
                // What QuickBooks does with a document it can't parse.
                Rs::error(
                    "QBXMLMsgsRs",
                    None,
                    0x8004_0400,
                    "QuickBooks found an error when parsing the provided XML text stream.",
                )
                .render(&mut out);
            }
        }
        out.push_str("</QBXMLMsgsRs></QBXML>");
        out
    }

    fn one(&self, rq: &Node) -> Rs {
        let attr = |k: &str| {
            rq.attrs
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone())
        };
        let request_id = attr("requestID");
        let rs_name = format!("{}Rs", rq.name.trim_end_matches("Rq"));
        if let Some(f) = lock(&self.failures).get(&rq.name) {
            return Rs::error(&rs_name, request_id, f.code, f.message.clone());
        }
        let param = |name: &str| rq.child(name).map(|n| n.text.clone());
        match rq.name.as_str() {
            "HostQueryRq" => {
                let mut rs = Rs::ok(&rs_name, request_id);
                rs.body = "<HostRet><ProductName>QuickBooks Desktop (fake)</ProductName><MajorVersion>33</MajorVersion><MinorVersion>0</MinorVersion><Country>US</Country><SupportedQBXMLVersion>13.0</SupportedQBXMLVersion><IsAutomaticLogin>false</IsAutomaticLogin><QBFileMode>SingleUser</QBFileMode></HostRet>".into();
                rs
            }
            "CompanyQueryRq" => {
                let mut rs = Rs::ok(&rs_name, request_id);
                to_xml("CompanyRet", &self.fixtures.company_now(), &mut rs.body);
                rs
            }
            "PreferencesQueryRq" => {
                let mut rs = Rs::ok(&rs_name, request_id);
                to_xml(
                    "PreferencesRet",
                    &self.fixtures.preferences_now(),
                    &mut rs.body,
                );
                rs
            }
            "GeneralSummaryReportQueryRq" | "GeneralDetailReportQueryRq" | "AgingReportQueryRq" => {
                let kind = param("GeneralSummaryReportType")
                    .or_else(|| param("GeneralDetailReportType"))
                    .or_else(|| param("AgingReportType"))
                    .unwrap_or_default();
                match self.fixtures.report_now(&kebab(&kind)) {
                    Some(report) => {
                        let mut rs = Rs::ok(&rs_name, request_id);
                        to_xml(
                            "ReportRet",
                            &report::from_bridge_report(&report),
                            &mut rs.body,
                        );
                        rs
                    }
                    None => Rs::error(
                        &rs_name,
                        request_id,
                        0x8004_0400,
                        format!("the fake QuickBooks has no {kind} report"),
                    ),
                }
            }
            name => match QbdEntity::ALL
                .into_iter()
                .find(|e| request::query_name(*e) == name)
            {
                Some(entity) => self.entity_query(entity, rq, &rs_name, request_id),
                None => Rs::error(
                    &rs_name,
                    request_id,
                    0x8004_0400,
                    format!(
                        "QuickBooks found an error when parsing the provided XML text stream: {name} isn't a request"
                    ),
                ),
            },
        }
    }

    fn entity_query(
        &self,
        entity: QbdEntity,
        rq: &Node,
        rs_name: &str,
        request_id: Option<String>,
    ) -> Rs {
        let attr = |k: &str| {
            rq.attrs
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone())
        };
        let param = |name: &str| rq.child(name).map(|n| n.text.clone());
        let with_detail = param("IncludeLineItems").as_deref() == Some("true");
        // Records come straight from the fixtures (type markers included, which
        // name the element and which `to_xml` leaves out); without
        // IncludeLineItems QuickBooks omits the lines, as the bridge's lists do.
        let render = |rows: &[Value]| {
            let mut body = String::new();
            for r in rows {
                let element = ret_element(entity, r);
                let mut r = r.clone();
                if !with_detail
                    && entity.needs_detail()
                    && let Value::Object(m) = &mut r
                {
                    m.retain(|k, _| !is_detail_key(k));
                }
                to_xml(element, &r, &mut body);
            }
            body
        };
        // One record by id.
        if let Some(id) = param(entity.id_field()) {
            return match self.fixtures.get_raw(entity, &id) {
                Some(r) => {
                    let mut rs = Rs::ok(rs_name, request_id);
                    rs.body = render(&[r]);
                    rs
                }
                None => Rs::error(
                    rs_name,
                    request_id,
                    QbdError::QBXML_NOT_FOUND,
                    format!("Object \"{id}\" specified in the request cannot be found."),
                ),
            };
        }
        let max: Option<usize> = param("MaxReturned").and_then(|s| s.parse().ok());
        let (rows, iterator) = match attr("iterator").as_deref() {
            Some("Continue") => {
                let id = attr("iteratorID").unwrap_or_default();
                let mut open = lock(&self.iterators);
                let Some(it) = open.get_mut(&id) else {
                    return Rs::error(
                        rs_name,
                        request_id,
                        0x8004_0400,
                        format!("iterator {id} isn't open"),
                    );
                };
                let take = max.unwrap_or(it.rest.len()).min(it.rest.len());
                let page: Vec<Value> = it.rest.drain(..take).collect();
                let remaining = it.rest.len();
                if remaining == 0 {
                    open.remove(&id);
                }
                (page, Some((remaining, id)))
            }
            start => {
                let q = Self::list_query(entity, rq);
                let mut rows = self.fixtures.select(entity, &q);
                if start == Some("Start") {
                    let id = {
                        let mut n = lock(&self.next_iterator);
                        *n += 1;
                        format!("{{fake-iterator-{n}}}")
                    };
                    let take = max.unwrap_or(rows.len()).min(rows.len());
                    let rest = rows.split_off(take);
                    let remaining = rest.len();
                    if remaining > 0 {
                        lock(&self.iterators).insert(id.clone(), OpenIterator { rest });
                    }
                    (rows, Some((remaining, id)))
                } else {
                    if let Some(m) = max {
                        rows.truncate(m);
                    }
                    (rows, None)
                }
            }
        };
        let mut rs = if rows.is_empty() && iterator.as_ref().is_none_or(|(r, _)| *r == 0) {
            Rs {
                name: rs_name.to_owned(),
                request_id,
                status: (
                    1,
                    "Info",
                    "A query request did not find a matching object in QuickBooks".into(),
                ),
                iterator: None,
                body: String::new(),
            }
        } else {
            Rs::ok(rs_name, request_id)
        };
        rs.iterator = iterator;
        rs.body = render(&rows);
        rs
    }

    fn list_query(entity: QbdEntity, rq: &Node) -> ListQuery {
        let param = |name: &str| rq.child(name).map(|n| n.text.clone());
        let nested = |outer: &str, inner: &str| {
            rq.child(outer)
                .and_then(|o| o.child(inner))
                .map(|n| n.text.clone())
        };
        let mut q = ListQuery::default();
        if entity.is_transaction() {
            q.modified_after = nested("ModifiedDateRangeFilter", "FromModifiedDate");
            q.from_date = nested("TxnDateRangeFilter", "FromTxnDate").and_then(|s| s.parse().ok());
            q.to_date = nested("TxnDateRangeFilter", "ToTxnDate").and_then(|s| s.parse().ok());
            q.entity_name = nested("EntityFilter", "FullName");
        } else {
            q.modified_after = param("FromModifiedDate");
            q.active = match param("ActiveStatus").as_deref() {
                Some("ActiveOnly") | None => Active::ActiveOnly,
                Some("InactiveOnly") => Active::InactiveOnly,
                _ => Active::All,
            };
            q.name = nested("NameFilter", "Name");
        }
        q
    }
}

impl QbxmlTransport for FakeQuickBooks {
    fn exchange(&self, request: Request) -> impl Future<Output = Result<String, QbdError>> + Send {
        std::future::ready(Ok(self.answer(&request.xml)))
    }
}

/// Convert a fixture record to the qbXML QuickBooks would send for it (for
/// building test documents by hand).
pub fn record_xml(entity: QbdEntity, record: &Value) -> String {
    let mut out = String::new();
    to_xml(ret_element(entity, record), record, &mut out);
    out
}

/// The `…Rs` elements of a fake answer, for assertions.
pub fn responses(xml_doc: &str) -> Vec<xml::QueryResponse> {
    xml::parse_response(xml_doc).expect("the fake writes well-formed qbXML")
}
