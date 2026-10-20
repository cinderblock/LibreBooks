//! Building qbXML `…QueryRq` documents: what the Web Connector and SDK
//! transports send to QuickBooks instead of the bridge's GETs.
//!
//! The parameters are the bridge client's ([`ListQuery`], [`ReportQuery`]), so
//! the sync engine and parity check don't know which transport they're on. The
//! semantics follow the bridge's too: `ActiveStatus All` unless asked otherwise,
//! `FromModifiedDate` is QBD's own `TimeModified` string passed back verbatim,
//! transaction lists take a date range, and iterators page the big lists. Two
//! things qbXML can do that the bridge can't: `IncludeLineItems` and
//! `IncludeLinkedTxns` put a transaction's lines in the list response (one page
//! instead of one GET per transaction), and a list without `MaxReturned` comes
//! back whole (no silent 100-row default).
//!
//! **Only queries.** This module can build `…QueryRq` elements and nothing else,
//! and [`check_read_only`] refuses any document whose message isn't one, so a
//! transport that passes every document through it cannot write to QuickBooks
//! even if some other code built the document. Element order inside a request
//! follows the qbXML 13.0 schema (a sequence; QuickBooks rejects elements out
//! of order with a parse error).

use jiff::civil::Date;
use quick_xml::escape::escape;

use crate::client::{ListQuery, QbdError, ReportQuery};
use crate::entity::QbdEntity;
use crate::qbxml::xml;

/// The qbXML version every document asks for. 13.0 is what current QuickBooks
/// Desktop editions speak; everything the import uses exists since 7.0.
pub const QBXML_VERSION: &str = "13.0";

/// The qbXML `…QueryRq` element for an entity.
pub const fn query_name(entity: QbdEntity) -> &'static str {
    match entity {
        QbdEntity::Account => "AccountQueryRq",
        QbdEntity::Class => "ClassQueryRq",
        QbdEntity::Terms => "TermsQueryRq",
        QbdEntity::PaymentMethod => "PaymentMethodQueryRq",
        QbdEntity::Vendor => "VendorQueryRq",
        QbdEntity::Employee => "EmployeeQueryRq",
        QbdEntity::OtherName => "OtherNameQueryRq",
        QbdEntity::SalesRep => "SalesRepQueryRq",
        QbdEntity::Customer => "CustomerQueryRq",
        QbdEntity::Item => "ItemQueryRq",
        QbdEntity::ItemService => "ItemServiceQueryRq",
        QbdEntity::ItemNonInventory => "ItemNonInventoryQueryRq",
        QbdEntity::ItemInventory => "ItemInventoryQueryRq",
        QbdEntity::PayrollItemWage => "PayrollItemWageQueryRq",
        QbdEntity::JournalEntry => "JournalEntryQueryRq",
        QbdEntity::Invoice => "InvoiceQueryRq",
        QbdEntity::CreditMemo => "CreditMemoQueryRq",
        QbdEntity::SalesReceipt => "SalesReceiptQueryRq",
        QbdEntity::Estimate => "EstimateQueryRq",
        QbdEntity::PurchaseOrder => "PurchaseOrderQueryRq",
        QbdEntity::Bill => "BillQueryRq",
        QbdEntity::Check => "CheckQueryRq",
        QbdEntity::TimeTracking => "TimeTrackingQueryRq",
        QbdEntity::ReceivePayment => "ReceivePaymentQueryRq",
        QbdEntity::BillPaymentCheck => "BillPaymentCheckQueryRq",
        QbdEntity::Deposit => "DepositQueryRq",
    }
}

/// The entity a `…QueryRs` element answers for.
pub fn entity_for_response(rs_name: &str) -> Option<QbdEntity> {
    let rq = rs_name.strip_suffix("Rs").map(|s| format!("{s}Rq"))?;
    QbdEntity::ALL.into_iter().find(|e| query_name(*e) == rq)
}

/// What a query may carry, per the qbXML 13.0 schema. Sending an element the
/// schema doesn't allow fails the whole request with a parse error, so these are
/// conservative: every `true` here is in the OSR for that query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// `iterator="Start"` paging.
    pub iterator: bool,
    /// `IncludeLineItems` (lines, applied-to transactions, deposit lines).
    pub include_line_items: bool,
    /// `IncludeLinkedTxns`.
    pub include_linked_txns: bool,
    /// `OwnerID 0` (custom fields in `DataExtRet`).
    pub owner_id: bool,
}

pub const fn capabilities(entity: QbdEntity) -> Capabilities {
    use QbdEntity as E;
    Capabilities {
        iterator: entity.supports_iterator(),
        include_line_items: entity.is_transaction() && !matches!(entity, E::TimeTracking),
        include_linked_txns: matches!(
            entity,
            E::Invoice | E::CreditMemo | E::Estimate | E::PurchaseOrder | E::Bill | E::Check
        ),
        owner_id: matches!(
            entity,
            E::Customer
                | E::Vendor
                | E::Employee
                | E::OtherName
                | E::Item
                | E::ItemService
                | E::ItemNonInventory
                | E::ItemInventory
        ) || (entity.is_transaction() && !matches!(entity, E::TimeTracking)),
    }
}

/// How to page a list request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Paging {
    /// Everything in one response (no `MaxReturned`), or at most `limit`.
    Whole { limit: Option<usize> },
    /// Open an iterator with pages of `page` rows.
    Start { page: usize },
    /// The next page of `iterator_id`.
    Continue { iterator_id: String, page: usize },
}

/// A built request document and what it asks for, so the response can be
/// matched and errors named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The `…QueryRq` element name.
    pub name: &'static str,
    pub request_id: u64,
    pub xml: String,
}

fn text(out: &mut String, name: &str, value: &str) {
    out.push('<');
    out.push_str(name);
    out.push('>');
    out.push_str(&escape(value));
    out.push_str("</");
    out.push_str(name);
    out.push('>');
}

fn date(d: Date) -> String {
    d.to_string()
}

/// A whole document around one request element.
fn document(name: &str, request_id: u64, attrs: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <?qbxml version=\"{QBXML_VERSION}\"?>\
         <QBXML><QBXMLMsgsRq onError=\"stopOnError\">\
         <{name} requestID=\"{request_id}\"{attrs}>{body}</{name}>\
         </QBXMLMsgsRq></QBXML>"
    )
}

/// A list page: the same filters the bridge client sends, in schema order.
pub fn list(entity: QbdEntity, query: &ListQuery, paging: &Paging, request_id: u64) -> Request {
    let name = query_name(entity);
    let caps = capabilities(entity);
    let mut body = String::new();
    let mut attrs = String::new();
    match paging {
        Paging::Whole { limit: Some(n) } => text(&mut body, "MaxReturned", &n.to_string()),
        Paging::Whole { limit: None } => {}
        Paging::Start { page } => {
            attrs.push_str(" iterator=\"Start\"");
            text(&mut body, "MaxReturned", &page.to_string());
        }
        Paging::Continue { iterator_id, page } => {
            attrs.push_str(&format!(
                " iterator=\"Continue\" iteratorID=\"{}\"",
                escape(iterator_id.as_str())
            ));
            text(&mut body, "MaxReturned", &page.to_string());
        }
    }
    if entity.is_transaction() {
        if let Some(m) = &query.modified_after {
            body.push_str("<ModifiedDateRangeFilter>");
            text(&mut body, "FromModifiedDate", m);
            body.push_str("</ModifiedDateRangeFilter>");
        }
        if query.from_date.is_some() || query.to_date.is_some() {
            body.push_str("<TxnDateRangeFilter>");
            if let Some(d) = query.from_date {
                text(&mut body, "FromTxnDate", &date(d));
            }
            if let Some(d) = query.to_date {
                text(&mut body, "ToTxnDate", &date(d));
            }
            body.push_str("</TxnDateRangeFilter>");
        }
        if let Some(n) = &query.entity_name {
            body.push_str("<EntityFilter>");
            text(&mut body, "FullName", n);
            body.push_str("</EntityFilter>");
        }
        if caps.include_line_items {
            text(&mut body, "IncludeLineItems", "true");
        }
        if caps.include_linked_txns {
            text(&mut body, "IncludeLinkedTxns", "true");
        }
    } else {
        // Lists: ActiveStatus, FromModifiedDate, NameFilter, in that order. The
        // bridge defaults to active only; an import wants everything, and
        // `Active::default()` is `All`.
        text(&mut body, "ActiveStatus", query.active.as_str());
        if let Some(m) = &query.modified_after {
            text(&mut body, "FromModifiedDate", m);
        }
        if let Some(n) = &query.name {
            body.push_str("<NameFilter>");
            text(&mut body, "MatchCriterion", "Contains");
            text(&mut body, "Name", n);
            body.push_str("</NameFilter>");
        }
    }
    if caps.owner_id {
        text(&mut body, "OwnerID", "0");
    }
    Request {
        name,
        request_id,
        xml: document(name, request_id, &attrs, &body),
    }
}

/// One record by `ListID` / `TxnID`, complete.
pub fn get(entity: QbdEntity, id: &str, request_id: u64) -> Request {
    let name = query_name(entity);
    let caps = capabilities(entity);
    let mut body = String::new();
    text(&mut body, entity.id_field(), id);
    if caps.include_line_items {
        text(&mut body, "IncludeLineItems", "true");
    }
    if caps.include_linked_txns {
        text(&mut body, "IncludeLinkedTxns", "true");
    }
    if caps.owner_id {
        text(&mut body, "OwnerID", "0");
    }
    Request {
        name,
        request_id,
        xml: document(name, request_id, "", &body),
    }
}

/// `CompanyQueryRq`.
pub fn company(request_id: u64) -> Request {
    Request {
        name: "CompanyQueryRq",
        request_id,
        xml: document("CompanyQueryRq", request_id, "", ""),
    }
}

/// `PreferencesQueryRq`.
pub fn preferences(request_id: u64) -> Request {
    Request {
        name: "PreferencesQueryRq",
        request_id,
        xml: document("PreferencesQueryRq", request_id, "", ""),
    }
}

/// `HostQueryRq`: which QuickBooks this is. Also the cheapest possible request,
/// used as a keep-alive when the Web Connector asks for work before the engine
/// has its next request ready.
pub fn host(request_id: u64) -> Request {
    Request {
        name: "HostQueryRq",
        request_id,
        xml: document("HostQueryRq", request_id, "", ""),
    }
}

/// Which qbXML report query a report belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportKind {
    Summary,
    Detail,
    Aging,
}

const SUMMARY_REPORTS: &[&str] = &[
    "BalanceSheetPrevYearComp",
    "BalanceSheetStandard",
    "BalanceSheetSummary",
    "CustomerBalanceSummary",
    "ExpenseByVendorSummary",
    "IncomeByCustomerSummary",
    "IncomeTaxSummary",
    "InventoryStockStatusByItem",
    "InventoryStockStatusByVendor",
    "InventoryValuationSummary",
    "PhysicalInventoryWorksheet",
    "ProfitAndLossByClass",
    "ProfitAndLossByJob",
    "ProfitAndLossPrevYearComp",
    "ProfitAndLossStandard",
    "ProfitAndLossYTDComp",
    "PurchaseByItemSummary",
    "PurchaseByVendorSummary",
    "SalesByCustomerSummary",
    "SalesByItemSummary",
    "SalesByRepSummary",
    "SalesTaxLiability",
    "SalesTaxRevenueSummary",
    "TrialBalance",
    "VendorBalanceSummary",
];

const DETAIL_REPORTS: &[&str] = &[
    "AuditTrail",
    "BalanceSheetDetail",
    "CheckDetail",
    "CustomerBalanceDetail",
    "DepositDetail",
    "EstimatesByJob",
    "ExpenseByVendorDetail",
    "GeneralLedger",
    "IncomeByCustomerDetail",
    "IncomeTaxDetail",
    "InventoryValuationDetail",
    "JobProgressInvoicesVsEstimates",
    "Journal",
    "MissingChecks",
    "OpenInvoices",
    "OpenPOs",
    "OpenPOsByJob",
    "OpenSalesOrderByCustomer",
    "OpenSalesOrderByItem",
    "PendingSales",
    "ProfitAndLossDetail",
    "PurchaseByItemDetail",
    "PurchaseByVendorDetail",
    "SalesByCustomerDetail",
    "SalesByItemDetail",
    "SalesByRepDetail",
    "TxnDetailByAccount",
    "TxnListByCustomer",
    "TxnListByDate",
    "TxnListByVendor",
    "UnbilledCostsByJob",
    "UnpaidBillsDetail",
    "VendorBalanceDetail",
];

const AGING_REPORTS: &[&str] = &[
    "APAgingDetail",
    "APAgingSummary",
    "ARAgingDetail",
    "ARAgingSummary",
    "CollectionsReport",
];

/// The bridge's report slug (`trial-balance`, `profit-and-loss-standard`,
/// `ar-aging-summary`) as the qbXML report type, with which query it needs.
pub fn report_type(slug: &str) -> Option<(ReportKind, &'static str)> {
    let wanted: String = slug
        .split(['-', '_'])
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect();
    let find = |list: &[&'static str]| {
        list.iter()
            .copied()
            .find(|n| n.eq_ignore_ascii_case(&wanted) || n.eq_ignore_ascii_case(slug))
    };
    if let Some(n) = find(SUMMARY_REPORTS) {
        return Some((ReportKind::Summary, n));
    }
    if let Some(n) = find(DETAIL_REPORTS) {
        return Some((ReportKind::Detail, n));
    }
    find(AGING_REPORTS).map(|n| (ReportKind::Aging, n))
}

/// A report request for the bridge's `GET /reports/{slug}` parameters.
pub fn report(slug: &str, query: &ReportQuery, request_id: u64) -> Result<Request, QbdError> {
    let (kind, report_type) = report_type(slug).ok_or_else(|| {
        QbdError::Unsupported(format!(
            "{slug:?} isn't a QuickBooks report this transport knows how to ask for"
        ))
    })?;
    let (name, type_element) = match kind {
        ReportKind::Summary => ("GeneralSummaryReportQueryRq", "GeneralSummaryReportType"),
        ReportKind::Detail => ("GeneralDetailReportQueryRq", "GeneralDetailReportType"),
        ReportKind::Aging => ("AgingReportQueryRq", "AgingReportType"),
    };
    let mut body = String::new();
    text(&mut body, type_element, report_type);
    if let Some(m) = &query.date_macro {
        text(&mut body, "ReportDateMacro", m);
    } else if query.from_date.is_some() || query.to_date.is_some() {
        body.push_str("<ReportPeriod>");
        if let Some(d) = query.from_date {
            text(&mut body, "FromReportDate", &date(d));
        }
        if let Some(d) = query.to_date {
            text(&mut body, "ToReportDate", &date(d));
        }
        body.push_str("</ReportPeriod>");
    }
    if let Some(e) = &query.entity {
        body.push_str("<ReportEntityFilter>");
        text(&mut body, "FullName", e);
        body.push_str("</ReportEntityFilter>");
    }
    if kind == ReportKind::Aging
        && let Some(d) = query.to_date
    {
        body.push_str("<ReportAgingAsOf>");
        text(&mut body, "ReportAgingAsOfDate", &date(d));
        body.push_str("</ReportAgingAsOf>");
    }
    match kind {
        ReportKind::Summary | ReportKind::Aging => {
            if let Some(s) = &query.summarize_by {
                text(&mut body, "SummarizeColumnsBy", s);
            }
        }
        ReportKind::Detail => {
            if let Some(s) = &query.summarize_by {
                text(&mut body, "SummarizeRowsBy", s);
            }
        }
    }
    if let Some(b) = query.basis {
        text(&mut body, "ReportBasis", b.as_str());
    }
    Ok(Request {
        name,
        request_id,
        xml: document(name, request_id, "", &body),
    })
}

/// Refuse any document that isn't made only of `…QueryRq` messages. Every
/// qbXML transport runs its documents through this before sending; LibreBooks
/// must never write to the user's QuickBooks.
pub fn check_read_only(xml_doc: &str) -> Result<(), QbdError> {
    let root = xml::parse_tree(xml_doc).map_err(|e| QbdError::Unsupported(e.to_string()))?;
    if root.name != "QBXML" {
        return Err(QbdError::Unsupported(format!(
            "refusing to send a document whose root is {:?}, not QBXML",
            root.name
        )));
    }
    let mut messages = 0;
    for msgs in &root.children {
        if msgs.name != "QBXMLMsgsRq" {
            return Err(QbdError::Unsupported(format!(
                "refusing to send {:?}: only QBXMLMsgsRq is allowed",
                msgs.name
            )));
        }
        for rq in &msgs.children {
            messages += 1;
            if !rq.name.ends_with("QueryRq") {
                return Err(QbdError::Unsupported(format!(
                    "refusing to send {:?} to QuickBooks: LibreBooks only ever queries",
                    rq.name
                )));
            }
        }
    }
    if messages == 0 {
        return Err(QbdError::Unsupported(
            "refusing to send an empty qbXML document".into(),
        ));
    }
    Ok(())
}

/// A qbXML `DATETIMETYPE` for a civil date (midnight, QuickBooks' own local
/// time), for callers that have a date where a watermark is wanted.
pub fn datetime_at_midnight(d: Date) -> String {
    format!("{d}T00:00:00")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Active;
    use pretty_assertions::assert_eq;

    #[test]
    fn lists_follow_the_bridge_semantics_in_schema_order() {
        let q = ListQuery {
            modified_after: Some("2026-10-19T10:43:19-08:00".into()),
            ..ListQuery::default()
        };
        let r = list(QbdEntity::Account, &q, &Paging::Whole { limit: None }, 7);
        assert_eq!(r.name, "AccountQueryRq");
        assert!(
            r.xml.starts_with(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?><?qbxml version=\"13.0\"?>"
            )
        );
        assert!(r.xml.contains(
            "<AccountQueryRq requestID=\"7\"><ActiveStatus>All</ActiveStatus><FromModifiedDate>2026-10-19T10:43:19-08:00</FromModifiedDate></AccountQueryRq>"
        ), "{}", r.xml);
        let r = list(
            QbdEntity::Customer,
            &ListQuery {
                active: Active::ActiveOnly,
                ..ListQuery::default()
            },
            &Paging::Start { page: 500 },
            8,
        );
        assert!(r.xml.contains(
            "<CustomerQueryRq requestID=\"8\" iterator=\"Start\"><MaxReturned>500</MaxReturned><ActiveStatus>ActiveOnly</ActiveStatus><OwnerID>0</OwnerID></CustomerQueryRq>"
        ), "{}", r.xml);
    }

    #[test]
    fn transactions_ask_for_lines_and_links_and_page() {
        let q = ListQuery {
            modified_after: Some("2026-01-01T00:00:00".into()),
            from_date: Some(jiff::civil::date(2026, 1, 1)),
            to_date: Some(jiff::civil::date(2026, 3, 31)),
            ..ListQuery::default()
        };
        let r = list(
            QbdEntity::Check,
            &q,
            &Paging::Continue {
                iterator_id: "{abc}".into(),
                page: 200,
            },
            3,
        );
        assert_eq!(
            r.xml
                .split("<QBXMLMsgsRq onError=\"stopOnError\">")
                .nth(1)
                .unwrap(),
            "<CheckQueryRq requestID=\"3\" iterator=\"Continue\" iteratorID=\"{abc}\"><MaxReturned>200</MaxReturned>\
             <ModifiedDateRangeFilter><FromModifiedDate>2026-01-01T00:00:00</FromModifiedDate></ModifiedDateRangeFilter>\
             <TxnDateRangeFilter><FromTxnDate>2026-01-01</FromTxnDate><ToTxnDate>2026-03-31</ToTxnDate></TxnDateRangeFilter>\
             <IncludeLineItems>true</IncludeLineItems><IncludeLinkedTxns>true</IncludeLinkedTxns><OwnerID>0</OwnerID>\
             </CheckQueryRq></QBXMLMsgsRq></QBXML>"
        );
        // Deposits and payments have lines but no linked transactions; time
        // tracking has neither and no custom fields.
        let d = list(
            QbdEntity::Deposit,
            &ListQuery::default(),
            &Paging::Whole { limit: Some(5) },
            1,
        )
        .xml;
        assert!(d.contains("<MaxReturned>5</MaxReturned><IncludeLineItems>true</IncludeLineItems><OwnerID>0</OwnerID>"));
        assert!(!d.contains("IncludeLinkedTxns"));
        let t = list(
            QbdEntity::TimeTracking,
            &ListQuery::default(),
            &Paging::Whole { limit: None },
            1,
        )
        .xml;
        assert!(
            t.contains("<TimeTrackingQueryRq requestID=\"1\"></TimeTrackingQueryRq>"),
            "{t}"
        );
    }

    #[test]
    fn single_records_and_company_queries() {
        let r = get(QbdEntity::Invoice, "1A-170", 2);
        assert!(r.xml.contains(
            "<InvoiceQueryRq requestID=\"2\"><TxnID>1A-170</TxnID><IncludeLineItems>true</IncludeLineItems><IncludeLinkedTxns>true</IncludeLinkedTxns><OwnerID>0</OwnerID></InvoiceQueryRq>"
        ));
        let r = get(QbdEntity::Terms, "80000003-1", 2);
        assert!(
            r.xml.contains(
                "<TermsQueryRq requestID=\"2\"><ListID>80000003-1</ListID></TermsQueryRq>"
            )
        );
        assert!(
            company(1)
                .xml
                .contains("<CompanyQueryRq requestID=\"1\"></CompanyQueryRq>")
        );
        assert!(host(1).xml.contains("<HostQueryRq"));
        assert_eq!(entity_for_response("CheckQueryRs"), Some(QbdEntity::Check));
        assert_eq!(entity_for_response("HostQueryRs"), None);
    }

    #[test]
    fn reports_map_the_bridges_slugs() {
        let q = ReportQuery {
            to_date: Some(jiff::civil::date(2026, 3, 31)),
            basis: Some(crate::client::Basis::Accrual),
            ..ReportQuery::default()
        };
        let r = report("trial-balance", &q, 4).unwrap();
        assert!(r.xml.contains(
            "<GeneralSummaryReportQueryRq requestID=\"4\"><GeneralSummaryReportType>TrialBalance</GeneralSummaryReportType>\
             <ReportPeriod><ToReportDate>2026-03-31</ToReportDate></ReportPeriod><ReportBasis>Accrual</ReportBasis></GeneralSummaryReportQueryRq>"
        ), "{}", r.xml);
        assert_eq!(
            report_type("general-ledger"),
            Some((ReportKind::Detail, "GeneralLedger"))
        );
        assert_eq!(
            report_type("ar-aging-summary"),
            Some((ReportKind::Aging, "ARAgingSummary"))
        );
        assert_eq!(
            report_type("ARAgingSummary"),
            Some((ReportKind::Aging, "ARAgingSummary"))
        );
        assert!(matches!(
            report("no-such-report", &q, 1),
            Err(QbdError::Unsupported(_))
        ));
    }

    #[test]
    fn only_queries_pass_the_read_only_check() {
        for e in QbdEntity::ALL {
            check_read_only(&list(e, &ListQuery::default(), &Paging::Whole { limit: None }, 1).xml)
                .unwrap();
            check_read_only(&get(e, "x", 1).xml).unwrap();
        }
        check_read_only(&company(1).xml).unwrap();
        check_read_only(&preferences(1).xml).unwrap();
        for forbidden in [
            "CheckAddRq",
            "CustomerModRq",
            "ListDelRq",
            "TxnVoidRq",
            "TxnDelRq",
            "DataExtAddRq",
        ] {
            let doc = format!(
                "<?xml version=\"1.0\"?><QBXML><QBXMLMsgsRq onError=\"stopOnError\"><{forbidden}/></QBXMLMsgsRq></QBXML>"
            );
            let e = check_read_only(&doc).unwrap_err();
            assert!(e.to_string().contains(forbidden), "{e}");
        }
        // A query smuggled in beside a write is still refused.
        let mixed = "<QBXML><QBXMLMsgsRq><CheckQueryRq/><CheckAddRq/></QBXMLMsgsRq></QBXML>";
        assert!(check_read_only(mixed).is_err());
        assert!(check_read_only("<QBXML><QBXMLMsgsRq/></QBXML>").is_err());
        assert!(
            check_read_only("<QBXML><QBXMLMsgsRs><CheckQueryRs/></QBXMLMsgsRs></QBXML>").is_err()
        );
    }
}
