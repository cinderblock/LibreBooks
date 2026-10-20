//! qbXML as XML → the JSON shape the QB Bridge produces, so the QuickBooks Web
//! Connector transport (qbXML over SOAP; QBWC ships free with QBD) can reuse the
//! whole normaliser.
//!
//! The conversion is the bridge's: an element holding only text becomes a
//! string (every scalar stays a string, as qbXML's are); an element with child
//! elements becomes an object; a child element that repeats becomes an array,
//! and one that doesn't stays a bare object or string. Attributes (qbXML puts
//! them only on the `…Rs` envelopes: `statusCode`, `iteratorID`…) become `@name`
//! keys. Text is kept exactly, trailing spaces included (QBD stores names and
//! address lines with them).
//!
//! One exception to the grouping: a report's `ReportData` holds `DataRow`,
//! `TextRow`, `SubtotalRow` and `TotalRow` elements whose order *is* the report,
//! so it becomes an ordered array of rows (`{"#row": "DataRow", …}`) instead;
//! [`super::report`] turns that into the bridge's report shape.
//!
//! The other direction, [`to_xml`], renders the same JSON back into qbXML. The
//! tests' fake QuickBooks answers with it, so the fixtures stay the single source
//! of truth for both transports.

use std::collections::BTreeMap;

use quick_xml::events::Event;
use quick_xml::{Reader, escape::resolve_predefined_entity};
use serde_json::{Map, Value};

/// Why a qbXML document couldn't be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum XmlError {
    #[error("the qbXML isn't well-formed: {0}")]
    Malformed(String),
    #[error("the qbXML uses an entity it doesn't define: &{0};")]
    UnknownEntity(String),
    #[error("the qbXML has no {0} element")]
    Missing(&'static str),
}

/// The key an ordered `ReportData` row keeps its element name under.
pub const ROW_KEY: &str = "#row";

/// A parsed element. `pub(crate)` so the Web Connector's SOAP reader and the
/// read-only check can walk a document without converting it.
#[derive(Debug, Default)]
pub(crate) struct Node {
    pub(crate) name: String,
    pub(crate) attrs: Vec<(String, String)>,
    pub(crate) children: Vec<Node>,
    pub(crate) text: String,
}

impl Node {
    /// The element name without its namespace prefix (`soap:Body` → `Body`).
    pub(crate) fn local_name(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }

    /// The first child with this local name.
    pub(crate) fn child(&self, local: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.local_name() == local)
    }

    pub(crate) fn into_value(self) -> Value {
        if self.children.is_empty() && self.attrs.is_empty() {
            return Value::String(self.text);
        }
        let mut map = Map::new();
        for (k, v) in self.attrs {
            map.insert(format!("@{k}"), Value::String(v));
        }
        if self.children.is_empty() {
            if !self.text.is_empty() {
                map.insert("#text".into(), Value::String(self.text));
            }
            return Value::Object(map);
        }
        if self.name == "ReportData" {
            let rows: Vec<Value> = self
                .children
                .into_iter()
                .map(|c| {
                    let name = c.name.clone();
                    let mut v = c.into_value();
                    if let Value::Object(m) = &mut v {
                        m.insert(ROW_KEY.into(), Value::String(name));
                    }
                    v
                })
                .collect();
            return Value::Array(rows);
        }
        // Group repeated children; qbXML has no mixed content, so text between
        // child elements is only indentation.
        let mut grouped: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for c in self.children {
            let name = c.name.clone();
            grouped.entry(name).or_default().push(c.into_value());
        }
        for (k, mut vs) in grouped {
            let v = if vs.len() == 1 {
                vs.pop().unwrap_or(Value::Null)
            } else {
                Value::Array(vs)
            };
            map.insert(k, v);
        }
        Value::Object(map)
    }
}

pub(crate) fn parse_tree(xml: &str) -> Result<Node, XmlError> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    let start = |e: &quick_xml::events::BytesStart<'_>| -> Result<Node, XmlError> {
        let mut attrs = Vec::new();
        for a in e.attributes() {
            let a = a.map_err(|e| XmlError::Malformed(e.to_string()))?;
            let value = a
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|e| XmlError::Malformed(e.to_string()))?;
            attrs.push((a.key.as_ref().to_owned(), value.into_owned()));
        }
        Ok(Node {
            name: e.name().as_ref().to_owned(),
            attrs,
            ..Node::default()
        })
    };
    let mut finish = |node: Node, stack: &mut Vec<Node>| -> Result<(), XmlError> {
        match stack.last_mut() {
            Some(parent) => parent.children.push(node),
            None if root.is_none() => root = Some(node),
            None => return Err(XmlError::Malformed("more than one root element".into())),
        }
        Ok(())
    };
    loop {
        match reader
            .read_event()
            .map_err(|e| XmlError::Malformed(e.to_string()))?
        {
            Event::Start(e) => stack.push(start(&e)?),
            Event::Empty(e) => {
                let node = start(&e)?;
                finish(node, &mut stack)?;
            }
            Event::End(_) => {
                let node = stack
                    .pop()
                    .ok_or_else(|| XmlError::Malformed("a closing tag with no opening".into()))?;
                finish(node, &mut stack)?;
            }
            Event::Text(t) => {
                if let Some(n) = stack.last_mut() {
                    n.text.push_str(&t.xml10_content());
                }
            }
            Event::CData(c) => {
                if let Some(n) = stack.last_mut() {
                    n.text.push_str(&c.into_inner());
                }
            }
            Event::GeneralRef(r) => {
                let resolved = match r.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    Ok(None) => {
                        let name = r.xml10_content();
                        resolve_predefined_entity(&name)
                            .ok_or_else(|| XmlError::UnknownEntity(name.to_string()))?
                            .to_owned()
                    }
                    Err(e) => return Err(XmlError::Malformed(e.to_string())),
                };
                if let Some(n) = stack.last_mut() {
                    n.text.push_str(&resolved);
                }
            }
            Event::Eof => break,
            // The XML declaration, `<?qbxml version="…"?>`, comments, DOCTYPE.
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err(XmlError::Malformed(
            "the document ends inside an element".into(),
        ));
    }
    root.ok_or(XmlError::Missing("root"))
}

/// A whole qbXML document as JSON: `{"QBXML": {"QBXMLMsgsRs": {…}}}`.
pub fn to_json(xml: &str) -> Result<Value, XmlError> {
    let root = parse_tree(xml)?;
    let mut m = Map::new();
    let name = root.name.clone();
    m.insert(name, root.into_value());
    Ok(Value::Object(m))
}

/// One `…Rs` element of a qbXML response.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryResponse {
    /// `CheckQueryRs`, `AccountQueryRs`…
    pub name: String,
    pub request_id: Option<String>,
    /// 0 is success; 1 is "no records matched" (not an error); ≥ 500 are errors
    /// or warnings by `status_severity`.
    pub status_code: i64,
    pub status_severity: String,
    pub status_message: String,
    pub iterator_remaining_count: Option<u64>,
    pub iterator_id: Option<String>,
    /// Each returned record with its element name (`CheckRet`, `ItemServiceRet`…),
    /// in the bridge's JSON shape.
    pub records: Vec<(String, Value)>,
}

impl QueryResponse {
    /// Success, or "no records matched" (status 1), which isn't an error.
    pub fn is_ok(&self) -> bool {
        self.status_code == 0 || self.status_code == 1 || self.status_severity == "Info"
    }

    /// QuickBooks' warning statuses (`statusSeverity="Warn"`), which still
    /// carry records (e.g. 500 "nothing more to return", 530 on an iterator).
    pub fn is_warning(&self) -> bool {
        self.status_severity == "Warn"
    }
}

/// Read a qbXML response (`<QBXML><QBXMLMsgsRs>…`) into its `…Rs` elements.
pub fn parse_response(xml: &str) -> Result<Vec<QueryResponse>, XmlError> {
    let root = parse_tree(xml)?;
    if root.name != "QBXML" {
        return Err(XmlError::Missing("QBXML"));
    }
    let msgs = root
        .children
        .into_iter()
        .find(|c| c.name == "QBXMLMsgsRs")
        .ok_or(XmlError::Missing("QBXMLMsgsRs"))?;
    let mut out = Vec::new();
    for rs in msgs.children {
        let attr = |k: &str| {
            rs.attrs
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone())
        };
        out.push(QueryResponse {
            request_id: attr("requestID"),
            status_code: attr("statusCode").and_then(|s| s.parse().ok()).unwrap_or(0),
            status_severity: attr("statusSeverity").unwrap_or_default(),
            status_message: attr("statusMessage").unwrap_or_default(),
            iterator_remaining_count: attr("iteratorRemainingCount").and_then(|s| s.parse().ok()),
            iterator_id: attr("iteratorID"),
            name: rs.name,
            records: rs
                .children
                .into_iter()
                .map(|c| (c.name.clone(), c.into_value()))
                .collect(),
        });
    }
    Ok(out)
}

/// Render a JSON value in the bridge's shape back into qbXML elements named
/// `name`: a string becomes a text element, an object an element with `@attr`
/// keys as attributes and the rest as children (in key order), an array one
/// element per item, and an array under [`ROW_KEY`]-tagged rows (what
/// `ReportData` parses to) its rows in order. Null and the fixtures' private
/// `__…` keys render nothing.
pub fn to_xml(name: &str, value: &Value, out: &mut String) {
    use quick_xml::escape::escape;
    match value {
        Value::Null => {}
        Value::Array(items) => {
            if name == "ReportData" {
                out.push_str("<ReportData>");
                for row in items {
                    if let Value::Object(m) = row {
                        let row_name = m
                            .get(ROW_KEY)
                            .and_then(Value::as_str)
                            .unwrap_or("DataRow")
                            .to_owned();
                        let mut m = m.clone();
                        m.remove(ROW_KEY);
                        to_xml(&row_name, &Value::Object(m), out);
                    }
                }
                out.push_str("</ReportData>");
            } else {
                for item in items {
                    to_xml(name, item, out);
                }
            }
        }
        Value::Object(m) => {
            out.push('<');
            out.push_str(name);
            for (k, v) in m {
                if let Some(attr) = k.strip_prefix('@') {
                    out.push(' ');
                    out.push_str(attr);
                    out.push_str("=\"");
                    out.push_str(&escape(scalar(v).as_str()));
                    out.push('"');
                }
            }
            out.push('>');
            for (k, v) in m {
                if k.starts_with('@') || k.starts_with("__") {
                    continue;
                }
                if k == "#text" {
                    out.push_str(&escape(scalar(v).as_str()));
                } else {
                    to_xml(k, v, out);
                }
            }
            out.push_str("</");
            out.push_str(name);
            out.push('>');
        }
        other => {
            out.push('<');
            out.push_str(name);
            out.push('>');
            out.push_str(&escape(scalar(other).as_str()));
            out.push_str("</");
            out.push_str(name);
            out.push('>');
        }
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn report_rows_keep_their_order_and_round_trip() {
        let xml = r#"<ReportRet><ReportTitle>Trial Balance</ReportTitle><ReportData>
<TextRow rowNumber="1" value="Header"/>
<DataRow rowNumber="2"><RowData rowType="account" value="Checking"/><ColData colID="1" value="Checking"/><ColData colID="2" value="9685.00"/></DataRow>
<TotalRow rowNumber="3"><ColData colID="1" value="TOTAL"/></TotalRow>
<DataRow rowNumber="4"><RowData rowType="account" value="Rent"/><ColData colID="2" value="1.00"/></DataRow>
</ReportData></ReportRet>"#;
        let v = to_json(xml).unwrap();
        let rows = v["ReportRet"]["ReportData"].as_array().unwrap();
        let kinds: Vec<&str> = rows.iter().map(|r| r[ROW_KEY].as_str().unwrap()).collect();
        assert_eq!(kinds, ["TextRow", "DataRow", "TotalRow", "DataRow"]);
        assert_eq!(rows[1]["ColData"][1]["@value"], "9685.00");
        let mut out = String::new();
        to_xml("ReportRet", &v["ReportRet"], &mut out);
        assert_eq!(to_json(&out).unwrap(), v);
    }

    #[test]
    fn json_renders_back_to_qbxml() {
        let v = json!({
            "TxnID": "1-1",
            "Memo": "Smith & Sons <b>",
            "ExpenseLineRet": [{"TxnLineID": "2-1"}, {"TxnLineID": "3-1"}],
            "__item_type": "Service"
        });
        let mut out = String::new();
        to_xml("CheckRet", &v, &mut out);
        assert_eq!(
            out,
            "<CheckRet><ExpenseLineRet><TxnLineID>2-1</TxnLineID></ExpenseLineRet><ExpenseLineRet><TxnLineID>3-1</TxnLineID></ExpenseLineRet><Memo>Smith &amp; Sons &lt;b&gt;</Memo><TxnID>1-1</TxnID></CheckRet>"
        );
        let back = to_json(&out).unwrap();
        assert_eq!(back["CheckRet"]["Memo"], "Smith & Sons <b>");
        assert_eq!(
            back["CheckRet"]["ExpenseLineRet"].as_array().unwrap().len(),
            2
        );
    }

    #[test]
    fn one_child_is_an_object_and_several_an_array() {
        let xml = r#"<?xml version="1.0"?><?qbxml version="13.0"?>
<JournalEntryRet>
  <TxnID>1-1</TxnID>
  <JournalDebitLine><TxnLineID>2-1</TxnLineID><Amount>10.00</Amount></JournalDebitLine>
  <JournalDebitLine><TxnLineID>3-1</TxnLineID><Amount>5.00</Amount></JournalDebitLine>
  <JournalCreditLine><TxnLineID>4-1</TxnLineID><Amount>15.00</Amount></JournalCreditLine>
</JournalEntryRet>"#;
        let v = to_json(xml).unwrap();
        assert_eq!(
            v,
            json!({"JournalEntryRet": {
                "TxnID": "1-1",
                "JournalDebitLine": [
                    {"TxnLineID": "2-1", "Amount": "10.00"},
                    {"TxnLineID": "3-1", "Amount": "5.00"}
                ],
                "JournalCreditLine": {"TxnLineID": "4-1", "Amount": "15.00"}
            }})
        );
    }

    #[test]
    fn text_is_kept_exactly_and_entities_resolve() {
        let xml = "<A><Name>Smith &amp; Sons </Name><Memo>caf&#233; &lt;b&gt;</Memo><Empty/><C><![CDATA[x<y]]></C></A>";
        let v = to_json(xml).unwrap();
        assert_eq!(v["A"]["Name"], "Smith & Sons ");
        assert_eq!(v["A"]["Memo"], "café <b>");
        assert_eq!(v["A"]["Empty"], "");
        assert_eq!(v["A"]["C"], "x<y");
    }

    #[test]
    fn rejects_broken_documents() {
        assert!(matches!(to_json("<A><B></A>"), Err(XmlError::Malformed(_))));
        assert!(matches!(
            to_json("<A>&nbsp;</A>"),
            Err(XmlError::UnknownEntity(_))
        ));
        assert!(to_json("<A>").is_err());
    }

    #[test]
    fn reads_response_envelopes() {
        let xml = r#"<?xml version="1.0" ?>
<QBXML><QBXMLMsgsRs>
<CheckQueryRs requestID="7" statusCode="0" statusSeverity="Info" statusMessage="Status OK" iteratorRemainingCount="12" iteratorID="{abc}">
  <CheckRet><TxnID>1-1</TxnID></CheckRet>
  <CheckRet><TxnID>2-1</TxnID></CheckRet>
</CheckQueryRs>
<ItemQueryRs requestID="8" statusCode="1" statusSeverity="Info" statusMessage="A query request did not find a matching object in QuickBooks" />
</QBXMLMsgsRs></QBXML>"#;
        let rs = parse_response(xml).unwrap();
        assert_eq!(rs.len(), 2);
        assert_eq!(rs[0].name, "CheckQueryRs");
        assert_eq!(rs[0].request_id.as_deref(), Some("7"));
        assert_eq!(rs[0].iterator_remaining_count, Some(12));
        assert_eq!(rs[0].iterator_id.as_deref(), Some("{abc}"));
        assert_eq!(rs[0].records.len(), 2);
        assert_eq!(
            rs[0].records[1],
            ("CheckRet".to_owned(), json!({"TxnID": "2-1"}))
        );
        assert_eq!(rs[1].status_code, 1);
        assert_eq!(rs[1].records.len(), 0);
    }
}
