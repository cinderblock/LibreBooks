//! The report as a PDF, the way QBD's "Print" lays one out: the title block and
//! the column headings on every page, rows indented by level, bold totals with a
//! rule above their amounts, amounts right-aligned in accounting style, and
//! "Page n of m" in the footer. Letter or A4; portrait unless the columns need
//! landscape.
//!
//! Text is set in Helvetica and Helvetica-Bold, two of the fourteen fonts every
//! PDF reader carries, so nothing is embedded and no font file ships with
//! LibreBooks. Those fonts are WinAnsi-encoded; a character outside that set
//! prints as `?`. Widths are measured from printpdf's metrics for the same faces,
//! so right alignment and column fitting are exact.

use std::str::FromStr;

use printpdf::{
    BuiltinFont, Color, Greyscale, Line, LinePoint, Mm, Op, ParsedFont, PdfDocument, PdfFontHandle,
    PdfPage, PdfSaveOptions, Point, Pt, TextItem,
};

use super::ExportError;
use crate::model::{Align, Report, RowKind};
use crate::render::{self, FlatRow, Layout, Span, Style};

/// Paper size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Paper {
    #[default]
    Letter,
    A4,
}

impl Paper {
    /// Portrait width and height in points.
    const fn size_pt(self) -> (f32, f32) {
        match self {
            Paper::Letter => (612.0, 792.0),
            Paper::A4 => (595.276, 841.89),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Paper::Letter => "letter",
            Paper::A4 => "a4",
        }
    }
}

impl FromStr for Paper {
    type Err = String;

    /// `letter` or `a4`, case-insensitive.
    fn from_str(s: &str) -> Result<Paper, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "letter" => Ok(Paper::Letter),
            "a4" => Ok(Paper::A4),
            other => Err(format!("paper is letter or a4, not {other:?}")),
        }
    }
}

/// PDF options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PdfOptions {
    pub paper: Paper,
}

/// Points, all of them.
const MARGIN: f32 = 36.0;
const TITLE_SIZE: f32 = 14.0;
const COMPANY_SIZE: f32 = 11.0;
const SUBTITLE_SIZE: f32 = 10.0;
const BASIS_SIZE: f32 = 9.0;
const HEAD_SIZE: f32 = 8.0;
const BODY_SIZE: f32 = 8.0;
const FOOTER_SIZE: f32 = 7.0;
/// Line height as a multiple of the font size.
const LEADING: f32 = 1.3;
/// Room on each side of a cell's text.
const PAD: f32 = 4.0;
/// Indentation per level in the first column.
const INDENT: f32 = 10.0;
/// Narrower than this and a column is not worth printing; text is cut instead.
const MIN_COLUMN: f32 = 40.0;
const RULE: f32 = 0.5;
const HEAVY_RULE: f32 = 1.0;

pub(super) fn to_pdf(report: &Report, opts: PdfOptions) -> Result<Vec<u8>, ExportError> {
    let fonts = Fonts::load()?;
    let lay = render::layout(report);
    let texts: Vec<Vec<String>> = lay
        .rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| win_ansi(&render::cell_text(&c.value, Style::Accounting)))
                .collect()
        })
        .collect();
    let page = Page::fit(&lay, &texts, &fonts, opts.paper);
    let items = paginate(&lay, &page);
    let count = items.len();
    let mut doc = PdfDocument::new(&report.title);
    doc.metadata.info.author = "LibreBooks".to_owned();
    doc.metadata.info.creator = "LibreBooks".to_owned();
    doc.metadata.info.producer = "LibreBooks".to_owned();
    doc.metadata.info.subject = report.subtitle.clone();
    let pages: Vec<PdfPage> = items
        .into_iter()
        .enumerate()
        .map(|(n, items)| {
            let mut ops = Vec::new();
            ops.push(Op::SetFillColor {
                col: Color::Greyscale(Greyscale::new(0.0, None)),
            });
            ops.push(Op::SetOutlineColor {
                col: Color::Greyscale(Greyscale::new(0.0, None)),
            });
            let mut y = page.height - MARGIN;
            y = page.title_block(&mut ops, &lay, &fonts, y);
            y = page.headings(&mut ops, &lay, &fonts, y);
            for item in items {
                y = match item {
                    Item::Row(i) => page.row(&mut ops, &lay.rows[i], &texts[i], &fonts, y),
                    Item::Note(i) => page.note(&mut ops, &lay.notes[i], &fonts, y),
                    Item::Blank => y - BODY_SIZE * LEADING,
                };
            }
            page.footer(&mut ops, &lay, &fonts, n + 1, count);
            PdfPage::new(Mm::from(Pt(page.width)), Mm::from(Pt(page.height)), ops)
        })
        .collect();
    doc.with_pages(pages);
    let mut warnings = Vec::new();
    let bytes = doc.save(&PdfSaveOptions::default(), &mut warnings);
    if bytes.is_empty() {
        return Err(ExportError {
            format: "pdf",
            message: "the PDF writer produced no bytes".to_owned(),
        });
    }
    Ok(bytes)
}

/// The two faces, with their metrics for measuring.
struct Fonts {
    regular: ParsedFont,
    bold: ParsedFont,
}

impl Fonts {
    fn load() -> Result<Fonts, ExportError> {
        let face = |f: BuiltinFont| {
            f.get_parsed_font().ok_or_else(|| ExportError {
                format: "pdf",
                message: format!("the built-in font {f:?} did not load"),
            })
        };
        Ok(Fonts {
            regular: face(BuiltinFont::Helvetica)?,
            bold: face(BuiltinFont::HelveticaBold)?,
        })
    }

    /// The width of `text` at `size` points.
    fn width(&self, text: &str, size: f32, bold: bool) -> f32 {
        let f = if bold { &self.bold } else { &self.regular };
        let upm = f32::from(f.units_per_em).max(1.0);
        let units: f32 = text
            .chars()
            .map(|c| {
                f.lookup_glyph_index(c as u32)
                    .and_then(|g| f.get_glyph_width(g))
                    .map_or(upm * 0.5, f32::from)
            })
            .sum();
        units / upm * size
    }

    /// `text` cut (with an ellipsis) to fit `max` points.
    fn fit(&self, text: &str, max: f32, size: f32, bold: bool) -> String {
        if self.width(text, size, bold) <= max {
            return text.to_owned();
        }
        let mut chars: Vec<char> = text.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let candidate: String = chars.iter().collect::<String>() + "\u{2026}";
            if self.width(&candidate, size, bold) <= max {
                return candidate;
            }
        }
        String::new()
    }

    const fn handle(bold: bool) -> PdfFontHandle {
        PdfFontHandle::Builtin(if bold {
            BuiltinFont::HelveticaBold
        } else {
            BuiltinFont::Helvetica
        })
    }
}

/// Characters WinAnsi (Windows-1252) can encode; anything else prints as `?`.
fn win_ansi(text: &str) -> String {
    text.chars()
        .map(|c| if is_win_ansi(c) { c } else { '?' })
        .collect()
}

const fn is_win_ansi(c: char) -> bool {
    matches!(
        c,
        ' '..='~'
            | '\u{A0}'..='\u{FF}'
            | '\u{20AC}'
            | '\u{201A}'
            | '\u{0192}'
            | '\u{201E}'
            | '\u{2026}'
            | '\u{2020}'
            | '\u{2021}'
            | '\u{02C6}'
            | '\u{2030}'
            | '\u{0160}'
            | '\u{2039}'
            | '\u{0152}'
            | '\u{017D}'
            | '\u{2018}'
            | '\u{2019}'
            | '\u{201C}'
            | '\u{201D}'
            | '\u{2022}'
            | '\u{2013}'
            | '\u{2014}'
            | '\u{02DC}'
            | '\u{2122}'
            | '\u{0161}'
            | '\u{203A}'
            | '\u{0153}'
            | '\u{017E}'
            | '\u{0178}'
    )
}

/// A column's place on the page.
#[derive(Clone, Copy, Debug)]
struct Col {
    x: f32,
    width: f32,
    align: Align,
    numeric: bool,
}

/// The page geometry and the fitted columns.
struct Page {
    width: f32,
    height: f32,
    cols: Vec<Col>,
    title_height: f32,
    headings_height: f32,
    footer_height: f32,
}

impl Page {
    /// Measure every column, choose the orientation and shrink what must shrink.
    fn fit(lay: &Layout<'_>, texts: &[Vec<String>], fonts: &Fonts, paper: Paper) -> Page {
        let mut natural: Vec<f32> = lay
            .columns
            .iter()
            .map(|c| fonts.width(&win_ansi(&c.label), HEAD_SIZE, true))
            .collect();
        for (row, texts) in lay.rows.iter().zip(texts) {
            for (i, t) in texts.iter().enumerate().take(natural.len()) {
                let mut w = fonts.width(t, BODY_SIZE, row.style.bold);
                if i == 0 && !t.is_empty() {
                    w += row.depth as f32 * INDENT;
                }
                if w > natural[i] {
                    natural[i] = w;
                }
            }
        }
        for w in &mut natural {
            *w += 2.0 * PAD;
        }
        let total: f32 = natural.iter().sum();
        let (pw, ph) = paper.size_pt();
        let landscape = total > pw - 2.0 * MARGIN;
        let (width, height) = if landscape { (ph, pw) } else { (pw, ph) };
        let usable = width - 2.0 * MARGIN;
        let text_col: Vec<bool> = lay
            .columns
            .iter()
            .map(|c| !render::is_numeric(c.kind))
            .collect();
        let widths = shrink(natural, &text_col, usable);
        let mut x = MARGIN;
        let cols = lay
            .columns
            .iter()
            .zip(widths)
            .map(|(c, w)| {
                let col = Col {
                    x,
                    width: w,
                    align: c.align,
                    numeric: render::is_numeric(c.kind),
                };
                x += w;
                col
            })
            .collect();
        let title_height = (TITLE_SIZE + COMPANY_SIZE + SUBTITLE_SIZE) * LEADING
            + lay.title.basis.map_or(0.0, |_| BASIS_SIZE * LEADING)
            + 6.0;
        let headings_height =
            HEAD_SIZE * LEADING * if lay.groups.is_some() { 2.0 } else { 1.0 } + 4.0;
        let footer_height = FOOTER_SIZE * LEADING + 8.0;
        Page {
            width,
            height,
            cols,
            title_height,
            headings_height,
            footer_height,
        }
    }

    fn body_top(&self) -> f32 {
        self.height - MARGIN - self.title_height - self.headings_height
    }

    fn body_bottom(&self) -> f32 {
        MARGIN + self.footer_height
    }

    fn table_right(&self) -> f32 {
        self.cols
            .last()
            .map_or(MARGIN, |c| c.x + c.width)
            .min(self.width - MARGIN)
    }

    fn title_block(&self, ops: &mut Vec<Op>, lay: &Layout<'_>, fonts: &Fonts, top: f32) -> f32 {
        let centre = self.width / 2.0;
        let mut y = top;
        let mut line = |text: &str, size: f32, bold: bool, y: &mut f32| {
            *y -= size * LEADING;
            let text = win_ansi(text);
            let w = fonts.width(&text, size, bold);
            draw_text(ops, &text, centre - w / 2.0, *y, size, bold);
        };
        line(&lay.title.company, COMPANY_SIZE, true, &mut y);
        line(&lay.title.title, TITLE_SIZE, true, &mut y);
        line(&lay.title.subtitle, SUBTITLE_SIZE, false, &mut y);
        if let Some(b) = lay.title.basis {
            line(b, BASIS_SIZE, false, &mut y);
        }
        top - self.title_height
    }

    fn headings(&self, ops: &mut Vec<Op>, lay: &Layout<'_>, fonts: &Fonts, top: f32) -> f32 {
        let mut y = top;
        if let Some(groups) = &lay.groups {
            y -= HEAD_SIZE * LEADING;
            for span in groups {
                self.group_heading(ops, span, fonts, y);
            }
        }
        y -= HEAD_SIZE * LEADING;
        for (col, c) in self.cols.iter().zip(lay.columns) {
            let text = fonts.fit(&win_ansi(&c.label), col.width - 2.0 * PAD, HEAD_SIZE, true);
            cell_text(ops, col, &text, fonts, y, HEAD_SIZE, true, 0.0);
        }
        let rule_y = y - 2.0;
        draw_rule(ops, MARGIN, self.table_right(), rule_y, RULE);
        top - self.headings_height
    }

    fn group_heading(&self, ops: &mut Vec<Op>, span: &Span, fonts: &Fonts, y: f32) {
        if span.label.is_empty() {
            return;
        }
        let (Some(first), Some(last)) = (self.cols.get(span.first), self.cols.get(span.last))
        else {
            return;
        };
        let left = first.x;
        let right = last.x + last.width;
        let text = fonts.fit(
            &win_ansi(&span.label),
            right - left - 2.0 * PAD,
            HEAD_SIZE,
            true,
        );
        let w = fonts.width(&text, HEAD_SIZE, true);
        draw_text(
            ops,
            &text,
            f32::midpoint(left, right) - w / 2.0,
            y,
            HEAD_SIZE,
            true,
        );
        draw_rule(ops, left + PAD, right - PAD, y - 2.0, RULE);
    }

    /// Draw one row; returns the new baseline cursor.
    fn row(
        &self,
        ops: &mut Vec<Op>,
        row: &FlatRow<'_>,
        texts: &[String],
        fonts: &Fonts,
        top: f32,
    ) -> f32 {
        if row.is_spacer() {
            return top - BODY_SIZE * LEADING / 2.0;
        }
        let y = top - BODY_SIZE * LEADING;
        if row.style.rule_above {
            let thickness = if row.style.double_rule {
                HEAVY_RULE
            } else {
                RULE
            };
            for (col, text) in self.cols.iter().zip(texts) {
                if col.numeric && !text.is_empty() {
                    draw_rule(
                        ops,
                        col.x + PAD,
                        col.x + col.width - PAD,
                        y + BODY_SIZE * LEADING - 1.5,
                        thickness,
                    );
                }
            }
        }
        for (i, (col, text)) in self.cols.iter().zip(texts).enumerate() {
            if text.is_empty() {
                continue;
            }
            let indent = if i == 0 {
                row.depth as f32 * INDENT
            } else {
                0.0
            };
            let text = fonts.fit(
                text,
                col.width - 2.0 * PAD - indent,
                BODY_SIZE,
                row.style.bold,
            );
            cell_text(ops, col, &text, fonts, y, BODY_SIZE, row.style.bold, indent);
        }
        if row.kind == RowKind::Total {
            // Breathing room under the bottom line.
            return y - 2.0;
        }
        y
    }

    fn note(&self, ops: &mut Vec<Op>, note: &str, fonts: &Fonts, top: f32) -> f32 {
        let y = top - BODY_SIZE * LEADING;
        let text = fonts.fit(&win_ansi(note), self.width - 2.0 * MARGIN, BODY_SIZE, false);
        draw_text(ops, &text, MARGIN, y, BODY_SIZE, false);
        y
    }

    fn footer(&self, ops: &mut Vec<Op>, lay: &Layout<'_>, fonts: &Fonts, n: usize, of: usize) {
        let y = MARGIN;
        draw_rule(
            ops,
            MARGIN,
            self.width - MARGIN,
            y + FOOTER_SIZE * LEADING,
            RULE,
        );
        draw_text(ops, &win_ansi(&lay.footer), MARGIN, y, FOOTER_SIZE, false);
        let page = format!("Page {n} of {of}");
        let w = fonts.width(&page, FOOTER_SIZE, false);
        draw_text(ops, &page, self.width - MARGIN - w, y, FOOTER_SIZE, false);
    }
}

/// Columns squeezed into `usable` points: text columns give way first, down to
/// [`MIN_COLUMN`]; then everything shrinks alike. Nothing grows.
fn shrink(mut widths: Vec<f32>, text_col: &[bool], usable: f32) -> Vec<f32> {
    let total: f32 = widths.iter().sum();
    if total <= usable || widths.is_empty() {
        return widths;
    }
    let mut excess = total - usable;
    // Text columns, proportionally to what they have above the floor.
    let spare: f32 = widths
        .iter()
        .zip(text_col)
        .filter(|(_, t)| **t)
        .map(|(w, _)| (w - MIN_COLUMN).max(0.0))
        .sum();
    if spare > 0.0 {
        let take = excess.min(spare);
        for (w, t) in widths.iter_mut().zip(text_col) {
            if *t {
                let room = (*w - MIN_COLUMN).max(0.0);
                *w -= take * room / spare;
            }
        }
        excess -= take;
    }
    if excess > 0.0 {
        // Still too wide: scale every column.
        let total: f32 = widths.iter().sum();
        let factor = usable / total;
        for w in &mut widths {
            *w *= factor;
        }
    }
    widths
}

/// What goes on a page, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Row(usize),
    Note(usize),
    Blank,
}

/// Split the rows (then a blank line and the notes) into pages that fit between
/// the headings and the footer.
fn paginate(lay: &Layout<'_>, page: &Page) -> Vec<Vec<Item>> {
    let room = (page.body_top() - page.body_bottom()).max(BODY_SIZE * LEADING);
    let height = |item: Item| match item {
        Item::Row(i) if lay.rows[i].is_spacer() => BODY_SIZE * LEADING / 2.0,
        Item::Row(i) if lay.rows[i].kind == RowKind::Total => BODY_SIZE * LEADING + 2.0,
        Item::Row(_) | Item::Note(_) | Item::Blank => BODY_SIZE * LEADING,
    };
    let mut items: Vec<Item> = (0..lay.rows.len()).map(Item::Row).collect();
    if !lay.notes.is_empty() {
        items.push(Item::Blank);
        items.extend((0..lay.notes.len()).map(Item::Note));
    }
    let mut pages: Vec<Vec<Item>> = vec![Vec::new()];
    let mut used = 0.0;
    for item in items {
        let h = height(item);
        if used + h > room && !pages.last().is_some_and(Vec::is_empty) {
            pages.push(Vec::new());
            used = 0.0;
        }
        // A page never starts with a spacer.
        if matches!(item, Item::Row(i) if lay.rows[i].is_spacer())
            && pages.last().is_some_and(Vec::is_empty)
        {
            continue;
        }
        pages.last_mut().expect("a page").push(item);
        used += h;
    }
    pages
}

/// Text in a cell, aligned as the column is, `indent` points in from the left.
#[allow(clippy::too_many_arguments)]
fn cell_text(
    ops: &mut Vec<Op>,
    col: &Col,
    text: &str,
    fonts: &Fonts,
    y: f32,
    size: f32,
    bold: bool,
    indent: f32,
) {
    let w = fonts.width(text, size, bold);
    let x = match col.align {
        Align::Left => col.x + PAD + indent,
        Align::Right => col.x + col.width - PAD - w,
        Align::Center => col.x + (col.width - w) / 2.0,
    };
    draw_text(ops, text, x.max(col.x), y, size, bold);
}

fn draw_text(ops: &mut Vec<Op>, text: &str, x: f32, y: f32, size: f32, bold: bool) {
    if text.is_empty() {
        return;
    }
    ops.extend([
        Op::StartTextSection,
        Op::SetFont {
            font: Fonts::handle(bold),
            size: Pt(size),
        },
        Op::SetTextCursor {
            pos: Point { x: Pt(x), y: Pt(y) },
        },
        Op::ShowText {
            items: vec![TextItem::Text(text.to_owned())],
        },
        Op::EndTextSection,
    ]);
}

fn draw_rule(ops: &mut Vec<Op>, x1: f32, x2: f32, y: f32, thickness: f32) {
    if x2 <= x1 {
        return;
    }
    ops.push(Op::SetOutlineThickness { pt: Pt(thickness) });
    ops.push(Op::DrawLine {
        line: Line {
            points: vec![
                LinePoint {
                    p: Point {
                        x: Pt(x1),
                        y: Pt(y),
                    },
                    bezier: false,
                },
                LinePoint {
                    p: Point {
                        x: Pt(x2),
                        y: Pt(y),
                    },
                    bezier: false,
                },
            ],
            is_closed: false,
        },
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_parses() {
        assert_eq!("letter".parse::<Paper>(), Ok(Paper::Letter));
        assert_eq!(" A4 ".parse::<Paper>(), Ok(Paper::A4));
        assert!("legal".parse::<Paper>().is_err());
    }

    #[test]
    fn win_ansi_keeps_what_helvetica_has() {
        assert_eq!(win_ansi("Café — (1,234.50)"), "Café — (1,234.50)");
        assert_eq!(win_ansi("日本"), "??");
        assert_eq!(win_ansi("a\u{2026}"), "a\u{2026}");
    }

    #[test]
    fn shrinking_takes_from_text_columns_first() {
        let w = shrink(vec![300.0, 100.0, 100.0], &[true, false, false], 400.0);
        assert!((w[0] - 200.0).abs() < 0.01, "{w:?}");
        assert!((w[1] - 100.0).abs() < 0.01);
        // Beyond what text columns can give, everything scales.
        let w = shrink(vec![50.0, 300.0, 300.0], &[true, false, false], 325.0);
        let total: f32 = w.iter().sum();
        assert!((total - 325.0).abs() < 0.01, "{w:?}");
        // Nothing grows.
        let w = shrink(vec![50.0, 50.0], &[true, true], 400.0);
        assert_eq!(w, vec![50.0, 50.0]);
    }

    #[test]
    fn helvetica_metrics_load_and_measure() {
        let fonts = Fonts::load().unwrap();
        let w = fonts.width("1,234.50", BODY_SIZE, false);
        assert!(w > 20.0 && w < 50.0, "{w}");
        assert!(fonts.width("Total", 8.0, true) > fonts.width("Total", 8.0, false));
        let cut = fonts.fit(
            "A very long account name that will not fit",
            60.0,
            8.0,
            false,
        );
        assert!(cut.ends_with('\u{2026}'), "{cut}");
        assert!(fonts.width(&cut, 8.0, false) <= 60.0);
    }
}
