//! The layout every export draws from, so CSV, XLSX and PDF agree on what a report
//! looks like on paper: the title block, the column headings (with group headings
//! spanning their columns), the rows with their indentation and emphasis, the
//! notes and the footer.
//!
//! Nothing here knows about files. A writer walks a [`Layout`] and draws each
//! piece in its own medium; the only per-medium choices are how a value is
//! spelled ([`Style`]) and how wide a column is ([`Layout::widths`] is a
//! character count the writers scale). Money stays integer cents until a writer
//! needs a real number for a spreadsheet cell, which is [`money_major`], the one
//! place a float is made from an amount.

use jiff::civil::Date;
use lb_core::Money;

use crate::model::{Cell, Column, ColumnKind, Report, RowKind, Value};
use crate::request::Basis;

/// The lines above the table, in QBD's order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleBlock {
    pub title: String,
    pub company: String,
    /// The date line ("January through March 2026", "As of October 19, 2026").
    pub subtitle: String,
    /// "Accrual Basis" / "Cash Basis", for reports that have one.
    pub basis: Option<&'static str>,
}

impl TitleBlock {
    /// The lines in print order, skipping an absent basis.
    pub fn lines(&self) -> Vec<&str> {
        let mut v = vec![
            self.title.as_str(),
            self.company.as_str(),
            self.subtitle.as_str(),
        ];
        v.extend(self.basis);
        v
    }
}

/// A heading over a run of columns (`first..=last`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub first: usize,
    pub last: usize,
    /// Empty for columns that have no group heading.
    pub label: String,
}

impl Span {
    pub fn width(&self) -> usize {
        self.last - self.first + 1
    }
}

/// How a row is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowStyle {
    pub bold: bool,
    /// A rule above the amounts (subtotals and totals).
    pub rule_above: bool,
    /// A heavier rule: the report's bottom line.
    pub double_rule: bool,
}

impl RowStyle {
    pub const fn of(kind: RowKind) -> RowStyle {
        match kind {
            RowKind::Header => RowStyle {
                bold: true,
                rule_above: false,
                double_rule: false,
            },
            RowKind::Data | RowKind::Spacer => RowStyle {
                bold: false,
                rule_above: false,
                double_rule: false,
            },
            RowKind::Subtotal => RowStyle {
                bold: true,
                rule_above: true,
                double_rule: false,
            },
            RowKind::Total => RowStyle {
                bold: true,
                rule_above: true,
                double_rule: true,
            },
        }
    }
}

/// One row ready to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlatRow<'a> {
    pub kind: RowKind,
    /// Indent level of the first column.
    pub depth: u32,
    pub style: RowStyle,
    /// One per column.
    pub cells: &'a [Cell],
}

impl FlatRow<'_> {
    pub fn is_spacer(&self) -> bool {
        self.kind == RowKind::Spacer
    }
}

/// A report laid out for paper or a sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout<'a> {
    pub title: TitleBlock,
    pub columns: &'a [Column],
    /// The group heading row, when any column has a group: one span per run of
    /// equal headings, with empty spans over columns that have none.
    pub groups: Option<Vec<Span>>,
    pub rows: Vec<FlatRow<'a>>,
    pub notes: &'a [String],
    /// "Generated 2026-10-19 21:04 UTC by LibreBooks".
    pub footer: String,
    /// A width per column in characters, from the widest heading or value
    /// (accounting style, with the first column's indentation), clamped to
    /// something a page can hold. Writers scale it to their units.
    pub widths: Vec<usize>,
}

/// The widths a column may have, in characters.
pub const MIN_WIDTH: usize = 6;
pub const MAX_WIDTH: usize = 60;
/// Characters of indentation per level in text media.
pub const INDENT: usize = 4;

/// Lay a report out.
pub fn layout(report: &Report) -> Layout<'_> {
    let rows: Vec<FlatRow<'_>> = report
        .rows
        .iter()
        .map(|r| FlatRow {
            kind: r.kind,
            depth: r.depth,
            style: RowStyle::of(r.kind),
            cells: &r.cells,
        })
        .collect();
    let groups = report
        .columns
        .iter()
        .any(|c| c.group.is_some())
        .then(|| spans(&report.columns));
    let widths = widths(&report.columns, &rows);
    Layout {
        title: TitleBlock {
            title: report.title.clone(),
            company: report.company_name.clone(),
            subtitle: report.subtitle.clone(),
            basis: report.basis.map(basis_line),
        },
        columns: &report.columns,
        groups,
        rows,
        notes: &report.notes,
        footer: footer_line(report.generated_at),
        widths,
    }
}

/// "Accrual Basis" / "Cash Basis".
pub const fn basis_line(b: Basis) -> &'static str {
    match b {
        Basis::Accrual => "Accrual Basis",
        Basis::Cash => "Cash Basis",
    }
}

/// The footer every export ends with.
pub fn footer_line(generated_at: jiff::Timestamp) -> String {
    format!(
        "Generated {} by LibreBooks",
        generated_at.strftime("%Y-%m-%d %H:%M UTC")
    )
}

/// Runs of equal group headings become one span each; a column without a group
/// is a span of its own with an empty label (two ungrouped neighbours are not
/// merged, so an "Account" heading never spans into "Debit").
fn spans(columns: &[Column]) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for (i, c) in columns.iter().enumerate() {
        match (&c.group, out.last_mut()) {
            (Some(g), Some(last)) if last.last + 1 == i && last.label == *g => last.last = i,
            (g, _) => out.push(Span {
                first: i,
                last: i,
                label: g.clone().unwrap_or_default(),
            }),
        }
    }
    out
}

fn widths(columns: &[Column], rows: &[FlatRow<'_>]) -> Vec<usize> {
    let mut w: Vec<usize> = columns.iter().map(|c| c.label.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.cells.iter().enumerate().take(w.len()) {
            let mut n = cell_text(&cell.value, Style::Accounting).chars().count();
            if i == 0 && n > 0 {
                n += row.depth as usize * INDENT;
            }
            w[i] = w[i].max(n);
        }
    }
    w.iter().map(|n| (*n).clamp(MIN_WIDTH, MAX_WIDTH)).collect()
}

/// How numbers are spelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// For a machine to read: `-1234.50`, `12.5%`.
    Plain,
    /// For a person: `1,234.50`, `(1,234.50)`, `12.5%`.
    Accounting,
}

/// A value as text.
pub fn cell_text(v: &Value, style: Style) -> String {
    match v {
        Value::Empty => String::new(),
        Value::Text(t) => t.clone(),
        Value::Money(m) => money_text(*m, style),
        Value::Date(d) => date_text(*d),
        Value::Number(n) => match style {
            Style::Plain => n.clone(),
            Style::Accounting => group_thousands(n),
        },
        Value::Percent(p) => format!("{p}%"),
    }
}

/// An amount as text: `-1234.50` plain, `(1,234.50)` accounting.
pub fn money_text(m: Money, style: Style) -> String {
    match style {
        Style::Plain => m.to_string(),
        Style::Accounting => {
            let abs = Money(m.0.saturating_abs()).to_string();
            let grouped = group_thousands(&abs);
            if m.0 < 0 {
                format!("({grouped})")
            } else {
                grouped
            }
        }
    }
}

/// `MM/DD/YYYY`, as QBD prints and exports dates.
pub fn date_text(d: Date) -> String {
    format!("{:02}/{:02}/{}", d.month(), d.day(), d.year())
}

/// `1234567.5` → `1,234,567.5`. Anything that isn't a plain decimal is returned
/// as it came.
pub fn group_thousands(n: &str) -> String {
    let (sign, rest) = match n.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", n),
    };
    let (int, frac) = rest
        .split_once('.')
        .map_or((rest, None), |(i, f)| (i, Some(f)));
    if int.is_empty() || !int.bytes().all(|b| b.is_ascii_digit()) {
        return n.to_owned();
    }
    let mut grouped = String::with_capacity(int.len() + int.len() / 3);
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

/// Cents as a number of major units, for a spreadsheet cell. The one place an
/// amount becomes a float: every cent value up to 2^53 is exact, and the
/// writer's number format shows two decimals.
pub fn money_major(m: Money) -> f64 {
    m.0 as f64 / 100.0
}

/// A percent string ("12.5") as the fraction a spreadsheet's percent format
/// expects (0.125). `None` when it isn't a decimal.
pub fn percent_fraction(p: &str) -> Option<f64> {
    p.trim().parse::<f64>().ok().map(|v| v / 100.0)
}

/// A quantity or rate string as a number. `None` when it isn't a decimal.
pub fn number_value(n: &str) -> Option<f64> {
    n.trim().parse::<f64>().ok()
}

/// Whether a column holds numbers (for alignment and number formats).
pub const fn is_numeric(kind: ColumnKind) -> bool {
    matches!(
        kind,
        ColumnKind::Money | ColumnKind::Number | ColumnKind::Percent
    )
}

/// The layout as a text grid, for tests and debugging: the title lines prefixed
/// `= `, the group row as `G` spans, the heading row as `L`, then each row as its
/// kind letter and depth followed by the cells in accounting style, separated by
/// ` | `, the first cell indented.
pub fn grid(layout: &Layout<'_>) -> String {
    let mut out = String::new();
    for l in layout.title.lines() {
        out.push_str("= ");
        out.push_str(l);
        out.push('\n');
    }
    if let Some(groups) = &layout.groups {
        out.push_str("G ");
        let parts: Vec<String> = groups
            .iter()
            .map(|s| format!("[{}..{}]{}", s.first, s.last, s.label))
            .collect();
        out.push_str(&parts.join(" "));
        out.push('\n');
    }
    out.push_str("L ");
    let labels: Vec<&str> = layout.columns.iter().map(|c| c.label.as_str()).collect();
    out.push_str(&labels.join(" | "));
    out.push('\n');
    for row in &layout.rows {
        let letter = match row.kind {
            RowKind::Header => 'H',
            RowKind::Data => 'D',
            RowKind::Subtotal => 'S',
            RowKind::Total => 'T',
            RowKind::Spacer => '-',
        };
        out.push(letter);
        out.push_str(&row.depth.to_string());
        if !row.is_spacer() {
            out.push(' ');
            let cells: Vec<String> = row
                .cells
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let text = cell_text(&c.value, Style::Accounting);
                    if i == 0 && !text.is_empty() {
                        format!("{}{text}", " ".repeat(row.depth as usize * INDENT))
                    } else {
                        text
                    }
                })
                .collect();
            out.push_str(&cells.join(" | "));
        }
        out.push('\n');
    }
    for n in layout.notes {
        out.push_str("N ");
        out.push_str(n);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounting_money() {
        assert_eq!(
            money_text(Money(12_345_678), Style::Accounting),
            "123,456.78"
        );
        assert_eq!(money_text(Money(-500), Style::Accounting), "(5.00)");
        assert_eq!(money_text(Money(0), Style::Accounting), "0.00");
        assert_eq!(money_text(Money(-123_450), Style::Plain), "-1234.50");
        assert_eq!(
            money_text(Money(10_000_000_000), Style::Accounting),
            "100,000,000.00"
        );
    }

    #[test]
    fn thousands_grouping_leaves_odd_strings_alone() {
        assert_eq!(group_thousands("1234567.5"), "1,234,567.5");
        assert_eq!(group_thousands("-1000"), "-1,000");
        assert_eq!(group_thousands("12"), "12");
        assert_eq!(group_thousands("n/a"), "n/a");
        assert_eq!(group_thousands(""), "");
    }

    #[test]
    fn spans_merge_only_equal_neighbouring_groups() {
        let cols = vec![
            Column::new("label", "", ColumnKind::Text),
            Column::new("c0", "Actual", ColumnKind::Money).grouped(Some("Feb 26".into())),
            Column::new("p0", "% of Income", ColumnKind::Percent).grouped(Some("Feb 26".into())),
            Column::new("c1", "Actual", ColumnKind::Money).grouped(Some("Feb 25".into())),
            Column::new("x", "Notes", ColumnKind::Text),
            Column::new("y", "More", ColumnKind::Text),
        ];
        let s = spans(&cols);
        let view: Vec<(usize, usize, &str)> = s
            .iter()
            .map(|s| (s.first, s.last, s.label.as_str()))
            .collect();
        assert_eq!(
            view,
            vec![
                (0, 0, ""),
                (1, 2, "Feb 26"),
                (3, 3, "Feb 25"),
                (4, 4, ""),
                (5, 5, "")
            ]
        );
    }

    #[test]
    fn money_major_is_exact_for_cents() {
        assert!((money_major(Money(123_456)) - 1234.56).abs() < 1e-9);
        assert!((money_major(Money(-1)) + 0.01).abs() < 1e-12);
        assert!((percent_fraction("12.5").unwrap() - 0.125).abs() < 1e-12);
        assert!(percent_fraction("abc").is_none());
    }
}
