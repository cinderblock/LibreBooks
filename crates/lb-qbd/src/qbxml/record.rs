//! Reading qbXML-shaped JSON safely.
//!
//! qbXML is XML; the bridge (and [`super::xml`]) turn it into JSON with two
//! habits every reader must cope with:
//!
//! - **One or many.** A repeated element is an array when there are several and a
//!   bare object when there is one (a journal entry's single `JournalCreditLine`
//!   arrives as an object, its three `JournalDebitLine`s as an array).
//!   [`Record::children`] accepts both.
//! - **Everything is a string.** `"true"`, `"12345.67"`, `"2026-09-01"`,
//!   `"2026-09-14T10:43:05-08:00"`. The typed getters parse them and say which
//!   field was bad when one doesn't parse.
//!
//! A [`Record`] also remembers which fields were read. Whatever is left over is
//! what LibreBooks has no field for: transactions keep it verbatim in `extra`,
//! and list records report it, so nothing QuickBooks sent disappears silently.

use std::collections::BTreeSet;
use std::str::FromStr;

use lb_core::Money;
use rust_decimal::Decimal;
use serde_json::{Map, Value};

/// A field that is present but doesn't parse.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{field} is {value:?}, which isn't {expected}")]
pub struct FieldError {
    pub field: String,
    pub value: String,
    pub expected: &'static str,
}

/// A reference to another QBD record: `{"ListID": "…", "FullName": "Parent:Child"}`
/// (or `TxnID` for transactions).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QbRef {
    pub list_id: Option<String>,
    pub full_name: Option<String>,
}

impl QbRef {
    pub fn is_empty(&self) -> bool {
        self.list_id.is_none() && self.full_name.is_none()
    }
}

impl std::fmt::Display for QbRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.full_name, &self.list_id) {
            (Some(n), Some(id)) => write!(f, "\"{n}\" ({id})"),
            (Some(n), None) => write!(f, "\"{n}\""),
            (None, Some(id)) => write!(f, "{id}"),
            (None, None) => f.write_str("(nothing)"),
        }
    }
}

/// The items of a one-or-many value: an array's elements, or the value itself.
pub fn one_or_many(v: Option<&Value>) -> Vec<&Value> {
    match v {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.iter().collect(),
        Some(other) => vec![other],
    }
}

/// A scalar as text: strings as they are, numbers and booleans spelled out (the
/// XML transport only produces strings; be lenient with other JSON producers).
fn scalar_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

pub fn parse_bool(s: &str) -> Option<bool> {
    if s.eq_ignore_ascii_case("true") || s == "1" {
        Some(true)
    } else if s.eq_ignore_ascii_case("false") || s == "0" {
        Some(false)
    } else {
        None
    }
}

/// A qbXML record being read. See the module docs.
#[derive(Debug)]
pub struct Record<'a> {
    /// Where this record sits, for messages: `Invoice`, `Invoice.InvoiceLineRet`.
    pub path: String,
    map: &'a Map<String, Value>,
    used: BTreeSet<&'a str>,
}

impl<'a> Record<'a> {
    /// Read `value`, which must be a JSON object.
    pub fn new(path: impl Into<String>, value: &'a Value) -> Result<Record<'a>, FieldError> {
        let path = path.into();
        match value {
            Value::Object(map) => Ok(Record {
                path,
                map,
                used: BTreeSet::new(),
            }),
            other => Err(FieldError {
                field: path,
                value: other.to_string(),
                expected: "a record",
            }),
        }
    }

    fn field_name(&self, key: &str) -> String {
        format!("{}.{key}", self.path)
    }

    fn take(&mut self, key: &str) -> Option<&'a Value> {
        let (k, v) = self.map.get_key_value(key)?;
        self.used.insert(k.as_str());
        Some(v)
    }

    /// Mark fields as handled without reading them (derived or deliberately
    /// dropped data, like `BillAddressBlock`, which QBD derives from
    /// `BillAddress`).
    pub fn ignore(&mut self, keys: &[&str]) {
        for k in keys {
            self.take(k);
        }
    }

    pub fn has(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    /// Text, trimmed of nothing (QBD keeps trailing spaces in names; so do we).
    /// Empty text counts as absent.
    pub fn text(&mut self, key: &str) -> Option<String> {
        let v = self.take(key)?;
        scalar_text(v).filter(|s| !s.is_empty())
    }

    /// Text that must be there.
    pub fn required_text(&mut self, key: &str) -> Result<String, FieldError> {
        self.text(key).ok_or_else(|| FieldError {
            field: self.field_name(key),
            value: String::new(),
            expected: "present",
        })
    }

    fn parsed<T>(
        &mut self,
        key: &str,
        expected: &'static str,
        parse: impl FnOnce(&str) -> Option<T>,
    ) -> Result<Option<T>, FieldError> {
        let Some(text) = self.text(key) else {
            return Ok(None);
        };
        match parse(text.trim()) {
            Some(v) => Ok(Some(v)),
            None => Err(FieldError {
                field: self.field_name(key),
                value: text,
                expected,
            }),
        }
    }

    pub fn bool(&mut self, key: &str) -> Result<Option<bool>, FieldError> {
        self.parsed(key, "true or false", parse_bool)
    }

    /// An amount (`AMTTYPE`): two decimal places.
    pub fn money(&mut self, key: &str) -> Result<Option<Money>, FieldError> {
        self.parsed(key, "an amount", |s| Money::from_str(s).ok())
    }

    /// A quantity, rate, price or percent.
    pub fn decimal(&mut self, key: &str) -> Result<Option<Decimal>, FieldError> {
        self.parsed(key, "a number", |s| Decimal::from_str(s).ok())
    }

    pub fn integer(&mut self, key: &str) -> Result<Option<i64>, FieldError> {
        self.parsed(key, "a whole number", |s| s.parse().ok())
    }

    pub fn date(&mut self, key: &str) -> Result<Option<jiff::civil::Date>, FieldError> {
        self.parsed(key, "a date", |s| s.parse().ok())
    }

    pub fn required_date(&mut self, key: &str) -> Result<jiff::civil::Date, FieldError> {
        let field = self.field_name(key);
        self.date(key)?.ok_or(FieldError {
            field,
            value: String::new(),
            expected: "present",
        })
    }

    /// A date and time with an offset (`2026-09-14T10:43:05-08:00`).
    pub fn timestamp(&mut self, key: &str) -> Result<Option<jiff::Timestamp>, FieldError> {
        self.parsed(key, "a date and time", |s| s.parse().ok())
    }

    /// A reference (`…Ref`). Absent or empty → `None`.
    pub fn reference(&mut self, key: &str) -> Option<QbRef> {
        let v = self.take(key)?;
        let r = QbRef {
            list_id: v
                .get("ListID")
                .or_else(|| v.get("TxnID"))
                .and_then(scalar_text)
                .filter(|s| !s.is_empty()),
            full_name: v
                .get("FullName")
                .and_then(scalar_text)
                .filter(|s| !s.is_empty()),
        };
        (!r.is_empty()).then_some(r)
    }

    /// Several references under one key (one or many).
    pub fn references(&mut self, key: &str) -> Vec<QbRef> {
        let Some(v) = self.take(key) else {
            return Vec::new();
        };
        one_or_many(Some(v))
            .into_iter()
            .map(|v| QbRef {
                list_id: v.get("ListID").and_then(scalar_text),
                full_name: v.get("FullName").and_then(scalar_text),
            })
            .filter(|r| !r.is_empty())
            .collect()
    }

    /// Child records under `key`, one or many. Children that aren't objects are
    /// reported as errors.
    pub fn children(&mut self, key: &str) -> Result<Vec<Record<'a>>, FieldError> {
        let path = self.field_name(key);
        let Some(v) = self.take(key) else {
            return Ok(Vec::new());
        };
        one_or_many(Some(v))
            .into_iter()
            .map(|c| Record::new(path.clone(), c))
            .collect()
    }

    /// A single child record (an address, `CashBackInfoRet`).
    pub fn child(&mut self, key: &str) -> Result<Option<Record<'a>>, FieldError> {
        let mut all = self.children(key)?;
        Ok(if all.is_empty() {
            None
        } else {
            Some(all.swap_remove(0))
        })
    }

    /// The raw value under `key`, marked as handled.
    pub fn raw(&mut self, key: &str) -> Option<&'a Value> {
        self.take(key)
    }

    /// The raw value under `key` without marking it handled: for a field that
    /// informs a decision but should still travel in the record's `extra`.
    pub fn peek(&self, key: &str) -> Option<&'a Value> {
        self.map.get(key)
    }

    /// Fields nobody read, verbatim.
    pub fn leftovers(&self) -> Map<String, Value> {
        self.map
            .iter()
            .filter(|(k, _)| !self.used.contains(k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// The names of fields nobody read.
    pub fn leftover_keys(&self) -> Vec<String> {
        self.map
            .keys()
            .filter(|k| !self.used.contains(k.as_str()))
            .cloned()
            .collect()
    }
}

/// The bookkeeping fields every QBD record has, read without consuming a
/// [`Record`]: the sync engine needs them before deciding whether to fetch or
/// normalise anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    /// `ListID` or `TxnID`.
    pub id: String,
    pub edit_sequence: Option<String>,
    /// As QBD wrote it, so it can be sent back as `modified_after` unchanged.
    pub time_modified: Option<String>,
    pub time_modified_at: Option<jiff::Timestamp>,
}

impl Meta {
    pub fn of(v: &Value) -> Option<Meta> {
        let id = v
            .get("TxnID")
            .or_else(|| v.get("ListID"))
            .and_then(scalar_text)
            .filter(|s| !s.is_empty())?;
        let time_modified = v.get("TimeModified").and_then(scalar_text);
        Some(Meta {
            id,
            edit_sequence: v.get("EditSequence").and_then(scalar_text),
            time_modified_at: time_modified.as_deref().and_then(|s| s.parse().ok()),
            time_modified,
        })
    }
}

/// A short description of a QBD record for people: `Check 8090 (2026-09-01)`,
/// `Account "Utilities:Internet"`.
pub fn describe(kind: &str, v: &Value) -> String {
    let s = |k: &str| v.get(k).and_then(scalar_text).filter(|s| !s.is_empty());
    if let Some(date) = s("TxnDate") {
        match s("RefNumber") {
            Some(n) => format!("{kind} {n} ({date})"),
            None => format!("{kind} dated {date}"),
        }
    } else if let Some(name) = s("FullName").or_else(|| s("Name")) {
        format!("{kind} \"{name}\"")
    } else {
        kind.to_owned()
    }
}

/// The sort key a `TxnLineID` gives: its leading hex counter. QBD numbers lines
/// from a company-wide counter (`5011-1700006100`), so sorting by it restores the
/// order lines were entered in, even across separately keyed groups
/// (`JournalDebitLine` vs `JournalCreditLine`). Unparseable ids sort last.
pub fn line_order(txn_line_id: Option<&str>) -> u64 {
    txn_line_id
        .and_then(|id| id.split('-').next())
        .and_then(|hex| u64::from_str_radix(hex, 16).ok())
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn one_or_many_accepts_both() {
        let one = json!({"A": 1});
        let many = json!([{"A": 1}, {"A": 2}]);
        assert_eq!(one_or_many(Some(&one)).len(), 1);
        assert_eq!(one_or_many(Some(&many)).len(), 2);
        assert_eq!(one_or_many(None).len(), 0);
    }

    #[test]
    fn typed_getters_parse_qbxml_strings() {
        let v = json!({
            "IsActive": "true",
            "Amount": "12345.67",
            "Quantity": "1.33333",
            "TxnDate": "2026-09-01",
            "TimeModified": "2026-09-14T10:43:05-08:00",
            "Empty": "",
            "AccountRef": {"ListID": "80000010-1700000000", "FullName": "Checking"},
            "Bad": "12.345",
            "Unread": "x"
        });
        let mut r = Record::new("Check", &v).unwrap();
        assert_eq!(r.bool("IsActive").unwrap(), Some(true));
        assert_eq!(r.money("Amount").unwrap(), Some(Money(1_234_567)));
        assert_eq!(
            r.decimal("Quantity").unwrap(),
            Some(Decimal::new(133_333, 5))
        );
        assert_eq!(
            r.date("TxnDate").unwrap(),
            Some(jiff::civil::date(2026, 9, 1))
        );
        let ts = r.timestamp("TimeModified").unwrap().unwrap();
        assert_eq!(ts.to_string(), "2026-09-14T18:43:05Z");
        assert_eq!(r.text("Empty"), None);
        assert_eq!(r.text("Missing"), None);
        let a = r.reference("AccountRef").unwrap();
        assert_eq!(a.full_name.as_deref(), Some("Checking"));
        let err = r.money("Bad").unwrap_err();
        assert_eq!(err.field, "Check.Bad");
        assert_eq!(r.leftover_keys(), vec!["Unread".to_owned()]);
    }

    #[test]
    fn children_are_one_or_many() {
        let v = json!({
            "JournalDebitLine": [{"Amount": "1.00"}, {"Amount": "2.00"}],
            "JournalCreditLine": {"Amount": "3.00"}
        });
        let mut r = Record::new("JournalEntry", &v).unwrap();
        assert_eq!(r.children("JournalDebitLine").unwrap().len(), 2);
        let credit = r.children("JournalCreditLine").unwrap();
        assert_eq!(credit.len(), 1);
        assert_eq!(credit[0].path, "JournalEntry.JournalCreditLine");
        assert!(r.leftovers().is_empty());
    }

    #[test]
    fn meta_and_descriptions() {
        let v = json!({"TxnID": "5010-1700006100", "EditSequence": "1700006100",
            "TimeModified": "2026-09-14T10:43:05-08:00", "TxnDate": "2026-09-01", "RefNumber": "8090"});
        let m = Meta::of(&v).unwrap();
        assert_eq!(m.id, "5010-1700006100");
        assert_eq!(m.edit_sequence.as_deref(), Some("1700006100"));
        assert!(m.time_modified_at.is_some());
        assert_eq!(describe("Check", &v), "Check 8090 (2026-09-01)");
        assert_eq!(
            describe("Account", &json!({"FullName": "Utilities:Internet"})),
            "Account \"Utilities:Internet\""
        );
    }

    #[test]
    fn line_ids_sort_by_their_hex_counter() {
        let mut ids = vec![
            "1013-1700002000",
            "1011-1700002000",
            "1014-1",
            "zz",
            "1012-1",
        ];
        ids.sort_by_key(|i| line_order(Some(i)));
        assert_eq!(
            ids,
            vec![
                "1011-1700002000",
                "1012-1",
                "1013-1700002000",
                "1014-1",
                "zz"
            ]
        );
    }
}
