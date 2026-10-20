//! Exporting a finished report to a file. CSV now; the [`ExportFormat`] enum is
//! where XLSX and PDF go, working from the same [`Report`] so every report exports
//! without knowing how.
//!
//! CSV follows RFC 4180 (CRLF line ends, fields quoted when they contain a comma,
//! quote or line break). Amounts are plain decimals (`-1234.50`, no symbol or
//! grouping) so spreadsheets read them as numbers; indentation is kept as leading
//! spaces in the first column. Text that a spreadsheet would run as a formula
//! (`=`, `+`, `-`, `@` first) is prefixed with `'`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Report, RowKind, Value};
use crate::request::Basis;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ExportFormat {
    Csv,
}

impl ExportFormat {
    pub const fn mime(self) -> &'static str {
        match self {
            ExportFormat::Csv => "text/csv; charset=utf-8",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
        }
    }
}

/// CSV options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CsvOptions {
    /// Lines with the title, company, date line and basis before the table.
    pub header_block: bool,
}

impl Default for CsvOptions {
    fn default() -> Self {
        CsvOptions { header_block: true }
    }
}

/// The report as a file in `format`.
pub fn export(report: &Report, format: ExportFormat) -> Vec<u8> {
    match format {
        ExportFormat::Csv => to_csv(report, CsvOptions::default()).into_bytes(),
    }
}

/// A suggested file name: "Profit & Loss" → `Profit_and_Loss.csv`.
pub fn file_name(report: &Report, format: ExportFormat) -> String {
    let stem: String = report
        .title
        .replace('&', "and")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let mut stem = stem
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if stem.is_empty() {
        stem = "report".to_owned();
    }
    format!("{stem}.{}", format.extension())
}

/// The report as CSV.
pub fn to_csv(report: &Report, opts: CsvOptions) -> String {
    let mut out = String::new();
    if opts.header_block {
        line(&mut out, std::slice::from_ref(&report.title));
        line(&mut out, std::slice::from_ref(&report.company_name));
        line(&mut out, std::slice::from_ref(&report.subtitle));
        if let Some(b) = report.basis {
            line(
                &mut out,
                &[match b {
                    Basis::Accrual => "Accrual Basis".to_owned(),
                    Basis::Cash => "Cash Basis".to_owned(),
                }],
            );
        }
        out.push_str("\r\n");
    }
    let has_groups = report.columns.iter().any(|c| c.group.is_some());
    if has_groups {
        let groups: Vec<String> = report
            .columns
            .iter()
            .map(|c| c.group.clone().unwrap_or_default())
            .collect();
        line(&mut out, &groups);
    }
    let labels: Vec<String> = report.columns.iter().map(|c| c.label.clone()).collect();
    line(&mut out, &labels);
    for row in &report.rows {
        if row.kind == RowKind::Spacer {
            out.push_str("\r\n");
            continue;
        }
        let fields: Vec<String> = row
            .cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let text = value_text(&c.value);
                if i == 0 && row.depth > 0 && !text.is_empty() {
                    format!("{}{text}", "    ".repeat(row.depth as usize))
                } else {
                    text
                }
            })
            .collect();
        line(&mut out, &fields);
    }
    for n in &report.notes {
        line(&mut out, std::slice::from_ref(n));
    }
    out
}

fn value_text(v: &Value) -> String {
    match v {
        Value::Empty => String::new(),
        Value::Text(t) => defuse(t),
        Value::Money(m) => m.to_string(),
        Value::Date(d) => format!("{:02}/{:02}/{}", d.month(), d.day(), d.year()),
        Value::Number(n) => n.clone(),
        Value::Percent(p) => format!("{p}%"),
    }
}

/// Keep spreadsheets from running text as a formula.
fn defuse(t: &str) -> String {
    match t.trim_start().chars().next() {
        Some('=' | '+' | '-' | '@') => format!("'{t}"),
        _ => t.to_owned(),
    }
}

fn line(out: &mut String, fields: &[String]) {
    let quoted: Vec<String> = fields.iter().map(|f| quote(f)).collect();
    out.push_str(&quoted.join(","));
    out.push_str("\r\n");
}

fn quote(f: &str) -> String {
    if f.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", f.replace('"', "\"\""))
    } else {
        f.to_owned()
    }
}

/// The date format CSV uses (`MM/DD/YYYY`, as QBD exports).
pub fn csv_date(d: jiff::civil::Date) -> String {
    value_text(&Value::Date(d))
}
