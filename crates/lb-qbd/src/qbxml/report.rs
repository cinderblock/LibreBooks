//! A qbXML `ReportRet` in the QB Bridge's report shape, so the parity check
//! reads a Web Connector or SDK report exactly as it reads the bridge's:
//!
//! ```json
//! { "title", "subtitle", "basis",
//!   "columns": [{ "id": 1, "type": "STRTYPE", "title": "" }],
//!   "rows": [{ "type": "DataRow", "label": "Checking", "values": { "0": "Checking", "1": "Checking", "2": "9685.00" } }] }
//! ```
//!
//! The bridge is itself a thin layer over qbXML, so the mapping is direct:
//! `ColDesc` → `columns`, each `ReportData` row → `rows` with its `RowData` value
//! as `"0"` and the label, and each `ColData` as `values[colID]`.

use serde_json::{Map, Value, json};

use super::xml::ROW_KEY;

fn one_or_many(v: Option<&Value>) -> Vec<&Value> {
    match v {
        Some(Value::Array(a)) => a.iter().collect(),
        Some(Value::Null) | None => Vec::new(),
        Some(other) => vec![other],
    }
}

fn attr<'a>(v: &'a Value, name: &str) -> Option<&'a str> {
    v.get(format!("@{name}")).and_then(Value::as_str)
}

/// Convert `ReportRet` (as [`super::xml::to_json`] produces it) to the bridge's
/// shape. Anything missing becomes empty rather than failing: a report with no
/// rows is a valid answer.
pub fn to_bridge_report(ret: &Value) -> Value {
    let text = |k: &str| ret.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let columns: Vec<Value> = one_or_many(ret.get("ColDesc"))
        .into_iter()
        .map(|c| {
            let id = attr(c, "colID")
                .and_then(|s| s.parse::<u64>().ok())
                .map_or(Value::Null, Value::from);
            let title = one_or_many(c.get("ColTitle"))
                .into_iter()
                .filter_map(|t| attr(t, "value"))
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            json!({
                "id": id,
                "type": attr(c, "dataType").unwrap_or(""),
                "title": title,
            })
        })
        .collect();
    let rows: Vec<Value> = one_or_many(ret.get("ReportData"))
        .into_iter()
        .map(|r| {
            let kind = r.get(ROW_KEY).and_then(Value::as_str).unwrap_or("DataRow");
            let mut values = Map::new();
            let mut label = String::new();
            // TextRow carries its text as its own `value` attribute.
            if let Some(v) = attr(r, "value") {
                label = v.to_owned();
                values.insert("0".into(), Value::String(v.to_owned()));
            }
            if let Some(v) = r.get("RowData").and_then(|d| attr(d, "value")) {
                label = v.to_owned();
                values.insert("0".into(), Value::String(v.to_owned()));
            }
            for c in one_or_many(r.get("ColData")) {
                if let (Some(id), Some(v)) = (attr(c, "colID"), attr(c, "value")) {
                    values.insert(id.to_owned(), Value::String(v.to_owned()));
                }
            }
            if label.is_empty()
                && let Some(Value::String(v)) = values.get("1")
            {
                label = v.clone();
            }
            json!({ "type": kind, "label": label, "values": Value::Object(values) })
        })
        .collect();
    json!({
        "title": text("ReportTitle"),
        "subtitle": text("ReportSubtitle"),
        "basis": text("ReportBasis"),
        "columns": columns,
        "rows": rows,
    })
}

/// The other way, for tests and fixtures: a bridge-shaped report as the JSON a
/// `ReportRet` parses to (so [`super::xml::to_xml`] can render it).
pub fn from_bridge_report(report: &Value) -> Value {
    let text = |k: &str| report.get(k).and_then(Value::as_str).unwrap_or("");
    let columns: Vec<Value> = one_or_many(report.get("columns"))
        .into_iter()
        .map(|c| {
            json!({
                "@colID": c.get("id").map(ToString::to_string).unwrap_or_default(),
                "@dataType": c.get("type").and_then(Value::as_str).unwrap_or(""),
                "ColTitle": { "@titleRow": "1", "@value": c.get("title").and_then(Value::as_str).unwrap_or("") },
            })
        })
        .collect();
    let rows: Vec<Value> = one_or_many(report.get("rows"))
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let kind = r.get("type").and_then(Value::as_str).unwrap_or("DataRow");
            let mut row = Map::new();
            row.insert(ROW_KEY.into(), Value::String(kind.to_owned()));
            row.insert("@rowNumber".into(), Value::String((i + 1).to_string()));
            let values = r.get("values").and_then(Value::as_object);
            let label = r.get("label").and_then(Value::as_str).unwrap_or("");
            if kind == "TextRow" {
                row.insert("@value".into(), Value::String(label.to_owned()));
            } else {
                if kind == "DataRow" {
                    row.insert(
                        "RowData".into(),
                        json!({ "@rowType": "account", "@value": label }),
                    );
                }
                let mut cols = Vec::new();
                if let Some(values) = values {
                    for (id, v) in values {
                        if id == "0" {
                            continue;
                        }
                        cols.push(json!({ "@colID": id, "@value": v }));
                    }
                }
                row.insert("ColData".into(), Value::Array(cols));
            }
            Value::Object(row)
        })
        .collect();
    json!({
        "ReportTitle": text("title"),
        "ReportSubtitle": text("subtitle"),
        "ReportBasis": text("basis"),
        "NumRows": rows.len().to_string(),
        "NumColumns": columns.len().to_string(),
        "NumColTitleRows": "1",
        "ColDesc": columns,
        "ReportData": rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qbxml::xml::{to_json, to_xml};
    use pretty_assertions::assert_eq;

    #[test]
    fn report_ret_becomes_the_bridges_shape() {
        let xml = r#"<ReportRet>
<ReportTitle>Trial Balance</ReportTitle><ReportSubtitle>As of March 31, 2026</ReportSubtitle>
<ReportBasis>Accrual</ReportBasis><NumRows>3</NumRows><NumColumns>3</NumColumns><NumColTitleRows>1</NumColTitleRows>
<ColDesc colID="1" dataType="STRTYPE"><ColTitle titleRow="1" value=""/></ColDesc>
<ColDesc colID="2" dataType="AMTTYPE"><ColTitle titleRow="1" value="Debit"/></ColDesc>
<ColDesc colID="3" dataType="AMTTYPE"><ColTitle titleRow="1" value="Credit"/></ColDesc>
<ReportData>
<DataRow rowNumber="1"><RowData rowType="account" value="Checking"/><ColData colID="1" value="Checking"/><ColData colID="2" value="9685.00"/></DataRow>
<DataRow rowNumber="2"><RowData rowType="account" value="Visa x1234"/><ColData colID="1" value="Visa x1234"/><ColData colID="3" value="89.99"/></DataRow>
<TotalRow rowNumber="3"><ColData colID="1" value="TOTAL"/><ColData colID="2" value="9685.00"/><ColData colID="3" value="89.99"/></TotalRow>
</ReportData></ReportRet>"#;
        let ret = to_json(xml).unwrap();
        let report = to_bridge_report(&ret["ReportRet"]);
        assert_eq!(report["title"], "Trial Balance");
        assert_eq!(report["basis"], "Accrual");
        assert_eq!(
            report["columns"][1],
            json!({"id": 2, "type": "AMTTYPE", "title": "Debit"})
        );
        assert_eq!(
            report["rows"][0],
            json!({"type": "DataRow", "label": "Checking", "values": {"0": "Checking", "1": "Checking", "2": "9685.00"}})
        );
        assert_eq!(report["rows"][1]["values"]["3"], "89.99");
        assert_eq!(report["rows"][2]["type"], "TotalRow");
        assert_eq!(report["rows"][2]["label"], "TOTAL");

        // And back: the bridge fixture renders to a ReportRet that converts to itself.
        let again = from_bridge_report(&report);
        let mut out = String::new();
        to_xml("ReportRet", &again, &mut out);
        let round = to_bridge_report(&to_json(&out).unwrap()["ReportRet"]);
        assert_eq!(round, report);
    }
}
