//! The report as an Excel workbook, the way QBD's "Export to Excel" lays one out:
//! one sheet named for the report; the title block; the heading rows (group
//! headings merged over their columns); the rows indented by level with bold
//! totals; then the notes and a footer. Amounts are real numbers in an accounting
//! format (two decimals, negatives in parentheses) so the user's own formulas
//! work; percents are fractions with a percent format; dates are Excel dates.
//! Heading rows are frozen and repeat on every printed page.

use std::collections::HashMap;

use rust_xlsxwriter::{
    ColNum, DocProperties, ExcelDateTime, Format, FormatAlign, FormatBorder, RowNum, Workbook,
    Worksheet, XlsxError,
};

use super::ExportError;
use crate::model::{Align, Report, Value};
use crate::render::{self, Layout, RowStyle, Style};

/// Excel's own number formats for the report's values.
const MONEY: &str = "#,##0.00_);(#,##0.00)";
const PERCENT: &str = "0.0%";
const DATE: &str = "mm/dd/yyyy";

/// Columns beyond this many print landscape.
const PORTRAIT_COLUMNS: usize = 6;

pub(super) fn to_xlsx(report: &Report) -> Result<Vec<u8>, ExportError> {
    write(report).map_err(|e| ExportError {
        format: "xlsx",
        message: e.to_string(),
    })
}

fn write(report: &Report) -> Result<Vec<u8>, XlsxError> {
    let lay = render::layout(report);
    let mut wb = Workbook::new();
    wb.set_properties(
        &DocProperties::new()
            .set_title(report.title.as_str())
            .set_subject(report.subtitle.as_str())
            .set_company(report.company_name.as_str())
            .set_author("LibreBooks"),
    );
    let ws = wb.add_worksheet();
    ws.set_name(sheet_name(&report.title))?;
    let mut sheet = Sheet {
        ws,
        last_col: lay.columns.len().saturating_sub(1) as ColNum,
        row: 0,
        formats: HashMap::new(),
    };
    sheet.title_block(&lay)?;
    sheet.headings(&lay)?;
    sheet.rows(&lay)?;
    sheet.notes_and_footer(&lay)?;
    sheet.widths(&lay)?;
    sheet.print_setup(&lay);
    wb.save_to_buffer()
}

/// A worksheet name Excel accepts: at most 31 characters, none of `[]:*?/\`,
/// no leading or trailing apostrophe, never empty.
pub(super) fn sheet_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '[' | ']' | ':' | '*' | '?' | '/' | '\\' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let mut name: String = cleaned.trim().chars().take(31).collect();
    while name.ends_with('\'') || name.ends_with(' ') {
        name.pop();
    }
    while name.starts_with('\'') {
        name.remove(0);
    }
    let name = name.trim().to_owned();
    if name.is_empty() || name.eq_ignore_ascii_case("history") {
        "Report".to_owned()
    } else {
        name
    }
}

/// What distinguishes one cell format from another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Spec {
    kind: Kind,
    align: Align,
    bold: bool,
    rule: Rule,
    indent: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Text,
    Money,
    Percent,
    Number,
    Date,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Rule {
    None,
    Single,
    Double,
}

impl Rule {
    const fn of(style: RowStyle) -> Rule {
        if style.double_rule {
            Rule::Double
        } else if style.rule_above {
            Rule::Single
        } else {
            Rule::None
        }
    }
}

fn build(spec: Spec) -> Format {
    let mut f = Format::new().set_align(match spec.align {
        Align::Left => FormatAlign::Left,
        Align::Right => FormatAlign::Right,
        Align::Center => FormatAlign::Center,
    });
    f = match spec.kind {
        Kind::Text => f,
        Kind::Money => f.set_num_format(MONEY),
        Kind::Percent => f.set_num_format(PERCENT),
        Kind::Number => f.set_num_format("General"),
        Kind::Date => f.set_num_format(DATE),
    };
    if spec.bold {
        f = f.set_bold();
    }
    f = match spec.rule {
        Rule::None => f,
        Rule::Single => f.set_border_top(FormatBorder::Thin),
        Rule::Double => f.set_border_top(FormatBorder::Double),
    };
    if spec.indent > 0 {
        f = f.set_indent(spec.indent);
    }
    f
}

struct Sheet<'w> {
    ws: &'w mut Worksheet,
    last_col: ColNum,
    /// The next row to write.
    row: RowNum,
    formats: HashMap<Spec, Format>,
}

impl Sheet<'_> {
    fn format(&mut self, spec: Spec) -> Format {
        self.formats
            .entry(spec)
            .or_insert_with(|| build(spec))
            .clone()
    }

    /// A line of text across the whole table (merged when there is more than one
    /// column).
    fn banner(&mut self, text: &str, format: &Format) -> Result<(), XlsxError> {
        if self.last_col > 0 {
            self.ws
                .merge_range(self.row, 0, self.row, self.last_col, text, format)?;
        } else {
            self.ws
                .write_string_with_format(self.row, 0, text, format)?;
        }
        self.row += 1;
        Ok(())
    }

    fn title_block(&mut self, lay: &Layout<'_>) -> Result<(), XlsxError> {
        let title = Format::new().set_bold().set_font_size(14);
        let company = Format::new().set_bold().set_font_size(11);
        let plain = Format::new();
        self.banner(&lay.title.title, &title)?;
        self.banner(&lay.title.company, &company)?;
        self.banner(&lay.title.subtitle, &plain)?;
        if let Some(b) = lay.title.basis {
            self.banner(b, &plain)?;
        }
        self.row += 1;
        Ok(())
    }

    fn headings(&mut self, lay: &Layout<'_>) -> Result<(), XlsxError> {
        let first_heading = self.row;
        if let Some(groups) = &lay.groups {
            let group = Format::new()
                .set_bold()
                .set_align(FormatAlign::Center)
                .set_border_bottom(FormatBorder::Thin);
            for span in groups {
                let (first, last) = (span.first as ColNum, span.last as ColNum);
                if last > first {
                    self.ws
                        .merge_range(self.row, first, self.row, last, &span.label, &group)?;
                } else if !span.label.is_empty() {
                    self.ws
                        .write_string_with_format(self.row, first, &span.label, &group)?;
                }
            }
            self.row += 1;
        }
        for (i, c) in lay.columns.iter().enumerate() {
            let f = Format::new()
                .set_bold()
                .set_border_bottom(FormatBorder::Thin)
                .set_align(match c.align {
                    Align::Left => FormatAlign::Left,
                    Align::Right => FormatAlign::Right,
                    Align::Center => FormatAlign::Center,
                });
            self.ws
                .write_string_with_format(self.row, i as ColNum, &c.label, &f)?;
        }
        self.ws.set_repeat_rows(first_heading, self.row)?;
        self.row += 1;
        self.ws.set_freeze_panes(self.row, 0)?;
        Ok(())
    }

    fn rows(&mut self, lay: &Layout<'_>) -> Result<(), XlsxError> {
        for row in &lay.rows {
            if row.is_spacer() {
                self.row += 1;
                continue;
            }
            let rule = Rule::of(row.style);
            for (i, cell) in row.cells.iter().enumerate() {
                let Some(column) = lay.columns.get(i) else {
                    break;
                };
                let col = i as ColNum;
                let indent = if i == 0 { row.depth.min(250) as u8 } else { 0 };
                let spec = |kind: Kind, align: Align| Spec {
                    kind,
                    align,
                    bold: row.style.bold,
                    rule,
                    indent,
                };
                match &cell.value {
                    Value::Empty => {
                        // A rule above an empty amount cell still shows as a
                        // border, as QBD's totals do.
                        if rule != Rule::None && render::is_numeric(column.kind) {
                            let f = self.format(spec(Kind::Text, column.align));
                            self.ws.write_blank(self.row, col, &f)?;
                        }
                    }
                    Value::Text(t) => {
                        let f = self.format(spec(Kind::Text, column.align));
                        self.ws.write_string_with_format(self.row, col, t, &f)?;
                    }
                    Value::Money(m) => {
                        let f = self.format(spec(Kind::Money, Align::Right));
                        self.ws.write_number_with_format(
                            self.row,
                            col,
                            render::money_major(*m),
                            &f,
                        )?;
                    }
                    Value::Percent(p) => {
                        if let Some(v) = render::percent_fraction(p) {
                            let f = self.format(spec(Kind::Percent, Align::Right));
                            self.ws.write_number_with_format(self.row, col, v, &f)?;
                        } else {
                            let f = self.format(spec(Kind::Text, Align::Right));
                            let text = render::cell_text(&cell.value, Style::Accounting);
                            self.ws.write_string_with_format(self.row, col, &text, &f)?;
                        }
                    }
                    Value::Number(n) => {
                        if let Some(v) = render::number_value(n) {
                            let f = self.format(spec(Kind::Number, Align::Right));
                            self.ws.write_number_with_format(self.row, col, v, &f)?;
                        } else {
                            let f = self.format(spec(Kind::Text, Align::Right));
                            self.ws.write_string_with_format(self.row, col, n, &f)?;
                        }
                    }
                    Value::Date(d) => {
                        // Excel's calendar starts in 1900.
                        if let Some(year) = u16::try_from(d.year()).ok().filter(|y| *y >= 1900) {
                            let dt = ExcelDateTime::from_ymd(
                                year,
                                d.month().unsigned_abs(),
                                d.day().unsigned_abs(),
                            )?;
                            let f = self.format(spec(Kind::Date, column.align));
                            self.ws.write_datetime_with_format(self.row, col, dt, &f)?;
                        } else {
                            let f = self.format(spec(Kind::Text, column.align));
                            let text = render::date_text(*d);
                            self.ws.write_string_with_format(self.row, col, &text, &f)?;
                        }
                    }
                }
            }
            self.row += 1;
        }
        Ok(())
    }

    fn notes_and_footer(&mut self, lay: &Layout<'_>) -> Result<(), XlsxError> {
        self.row += 1;
        let note = Format::new().set_italic();
        for n in lay.notes {
            self.banner(n, &note)?;
        }
        if !lay.notes.is_empty() {
            self.row += 1;
        }
        let footer = Format::new().set_italic().set_font_color("#666666");
        self.banner(&lay.footer, &footer)
    }

    /// Character counts become Excel column widths (units of one digit's width,
    /// plus a little room for the cell padding).
    fn widths(&mut self, lay: &Layout<'_>) -> Result<(), XlsxError> {
        for (i, w) in lay.widths.iter().enumerate() {
            let width = (*w as f64).mul_add(1.1, 2.0);
            self.ws.set_column_width(i as ColNum, width)?;
        }
        Ok(())
    }

    fn print_setup(&mut self, lay: &Layout<'_>) {
        if lay.columns.len() > PORTRAIT_COLUMNS {
            self.ws.set_landscape();
        }
        self.ws.set_print_fit_to_pages(1, 0);
        self.ws.set_footer("&L&8LibreBooks&R&8Page &P of &N");
        self.ws
            .set_header(format!("&C&12&\"-,Bold\"{}", lay.title.title));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_names_fit_excel() {
        assert_eq!(sheet_name("Profit & Loss"), "Profit & Loss");
        assert_eq!(
            sheet_name("Transaction Detail by Account: All/Everything?"),
            "Transaction Detail by Account"
        );
        assert_eq!(sheet_name("'quoted'"), "quoted");
        assert_eq!(sheet_name(""), "Report");
        assert_eq!(sheet_name("History"), "Report");
        assert!(sheet_name(&"x".repeat(100)).chars().count() <= 31);
    }
}
