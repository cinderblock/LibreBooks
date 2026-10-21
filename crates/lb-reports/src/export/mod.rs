//! Exporting a finished report to a file: CSV, XLSX (what QBD's "Export to Excel"
//! gives) and PDF (its "Print"). All three draw the same [`Layout`] from
//! [`crate::render`], so the title block, headings, indentation, emphasis and
//! footer agree; each writer only chooses how to spell a value and how wide a
//! column is in its own medium.
//!
//! CSV follows RFC 4180 (CRLF line ends, fields quoted when they contain a comma,
//! quote or line break). Amounts are plain decimals (`-1234.50`, no symbol or
//! grouping) so spreadsheets read them as numbers; indentation is kept as leading
//! spaces in the first column. Text that a spreadsheet would run as a formula
//! (`=`, `+`, `-`, `@` first) is prefixed with `'`.

mod pdf;
mod xlsx;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Report, Value};
use crate::render::{self, Layout, Style};

pub use pdf::{Paper, PdfOptions};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ExportFormat {
    Csv,
    Xlsx,
    Pdf,
}

impl ExportFormat {
    pub const fn mime(self) -> &'static str {
        match self {
            ExportFormat::Csv => "text/csv; charset=utf-8",
            ExportFormat::Xlsx => {
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            }
            ExportFormat::Pdf => "application/pdf",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Xlsx => "xlsx",
            ExportFormat::Pdf => "pdf",
        }
    }

    /// The format a query string names (`csv`, `xlsx`, `pdf`; case-insensitive).
    pub fn parse(s: &str) -> Option<ExportFormat> {
        match s.trim().to_ascii_lowercase().as_str() {
            "csv" => Some(ExportFormat::Csv),
            "xlsx" => Some(ExportFormat::Xlsx),
            "pdf" => Some(ExportFormat::Pdf),
            _ => None,
        }
    }
}

/// Choices that apply to some formats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportOptions {
    /// PDF paper size.
    pub paper: Paper,
}

/// Why a file could not be built. CSV never fails; the XLSX and PDF writers can
/// (an impossible sheet name, a font that did not load).
#[derive(Debug, thiserror::Error)]
#[error("could not build the {format} file: {message}")]
pub struct ExportError {
    pub format: &'static str,
    pub message: String,
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

/// The report as a file in `format`, with the default options.
pub fn export(report: &Report, format: ExportFormat) -> Result<Vec<u8>, ExportError> {
    export_with(report, format, &ExportOptions::default())
}

/// The report as a file in `format`.
pub fn export_with(
    report: &Report,
    format: ExportFormat,
    opts: &ExportOptions,
) -> Result<Vec<u8>, ExportError> {
    match format {
        ExportFormat::Csv => Ok(to_csv(report, CsvOptions::default()).into_bytes()),
        ExportFormat::Xlsx => xlsx::to_xlsx(report),
        ExportFormat::Pdf => pdf::to_pdf(report, PdfOptions { paper: opts.paper }),
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
    let lay: Layout<'_> = render::layout(report);
    let mut out = String::new();
    if opts.header_block {
        for l in lay.title.lines() {
            line(&mut out, &[l.to_owned()]);
        }
        out.push_str("\r\n");
    }
    if let Some(groups) = &lay.groups {
        // The heading repeated over every column it spans, so a spreadsheet
        // without merged cells still shows which column belongs to which.
        let mut fields = vec![String::new(); lay.columns.len()];
        for span in groups {
            for f in &mut fields[span.first..=span.last] {
                f.clone_from(&span.label);
            }
        }
        line(&mut out, &fields);
    }
    let labels: Vec<String> = lay.columns.iter().map(|c| c.label.clone()).collect();
    line(&mut out, &labels);
    for row in &lay.rows {
        if row.is_spacer() {
            out.push_str("\r\n");
            continue;
        }
        let fields: Vec<String> = row
            .cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let text = csv_text(&c.value);
                if i == 0 && row.depth > 0 && !text.is_empty() {
                    format!("{}{text}", " ".repeat(row.depth as usize * render::INDENT))
                } else {
                    text
                }
            })
            .collect();
        line(&mut out, &fields);
    }
    for n in lay.notes {
        line(&mut out, std::slice::from_ref(n));
    }
    out
}

fn csv_text(v: &Value) -> String {
    match v {
        Value::Text(t) => defuse(t),
        other => render::cell_text(other, Style::Plain),
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
    render::date_text(d)
}
