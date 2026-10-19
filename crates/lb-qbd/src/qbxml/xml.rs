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

#[derive(Debug, Default)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
    text: String,
}

impl Node {
    fn into_value(self) -> Value {
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

fn parse_tree(xml: &str) -> Result<Node, XmlError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
