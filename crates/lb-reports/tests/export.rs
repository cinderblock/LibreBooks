//! The export files: the shared layout's grid for a P&L and a balance sheet with
//! comparison columns, an XLSX read back cell by cell, PDFs parsed back.

mod common;

use std::io::Cursor;

use calamine::{Data, DataType as _, Reader, Xlsx};
use common::{Co, company, d, req};
use lb_reports::{
    ColumnLayout, ExportFormat, ExportOptions, Paper, Report, ReportError, ReportId, ReportRequest,
    render,
};
use pretty_assertions::assert_eq;

/// Two months of 2026 and one of 2025, so comparisons have something to compare:
/// $5,000 opening balance; Globex invoiced $80 (Feb 2025) and $100 (Jan 2026);
/// Acme invoiced $200 labor + $50 parts (Feb 2026); check #5 for $50 rent and
/// $12.50 internet (Feb 2026).
fn books() -> Co {
    let co = company(1);
    co.journal(d(2025, 1, 2), &[(co.bank, 500_000), (co.obe, -500_000)]);
    co.invoice(d(2025, 2, 5), co.globex, &[(co.labor, 8_000)]);
    co.invoice(d(2026, 1, 5), co.globex, &[(co.labor, 10_000)]);
    co.invoice(
        d(2026, 2, 5),
        co.acme,
        &[(co.labor, 20_000), (co.parts, 5_000)],
    );
    co.check(
        d(2026, 2, 1),
        co.landlord,
        "5",
        &[(co.rent, 5_000), (co.internet, 1_250)],
    );
    co
}

fn fixed_time(mut r: Report) -> Report {
    r.generated_at = "2026-10-19T21:04:00Z".parse().unwrap();
    r
}

fn pl_by_month_vs_prev_year(co: &Co) -> Report {
    let mut request =
        req(ReportId::ProfitAndLoss, d(2026, 1, 1), d(2026, 2, 28)).columns(ColumnLayout::Month);
    request.previous_year = true;
    fixed_time(co.run(&request))
}

fn balance_sheet_vs_prev_year(co: &Co) -> Report {
    fixed_time(co.run(&req(
        ReportId::BalanceSheetPrevYearComparison,
        d(2026, 1, 1),
        d(2026, 2, 28),
    )))
}

fn grid_lines(r: &Report) -> Vec<String> {
    render::grid(&render::layout(r))
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn profit_and_loss_with_comparison_columns_lays_out_with_group_spans() {
    let co = books();
    let pl = pl_by_month_vs_prev_year(&co);
    let lay = render::layout(&pl);
    assert_eq!(lay.footer, "Generated 2026-10-19 21:04 UTC by LibreBooks");
    assert_eq!(lay.widths[0], "        Consulting Income".len());
    assert_eq!(
        grid_lines(&pl),
        [
            "= Profit & Loss",
            "= Test Co",
            "= January through February 2026",
            "= Accrual Basis",
            "G [0..0] [1..4]Jan 26 [5..8]Feb 26 [9..12]TOTAL",
            "L  | Jan 26 | Jan 25 | $ Change | % Change | Feb 26 | Feb 25 | $ Change | % Change | TOTAL | Jan - Feb 25 | $ Change | % Change",
            "H0 Ordinary Income/Expense |  |  |  |  |  |  |  |  |  |  |  | ",
            "H1     Income |  |  |  |  |  |  |  |  |  |  |  | ",
            "D2         Consulting Income | 100.00 | 0.00 | 100.00 |  | 200.00 | 80.00 | 120.00 | 150.0% | 300.00 | 80.00 | 220.00 | 275.0%",
            "D2         Materials Income | 0.00 | 0.00 | 0.00 |  | 50.00 | 0.00 | 50.00 |  | 50.00 | 0.00 | 50.00 | ",
            "S1     Total Income | 100.00 | 0.00 | 100.00 |  | 250.00 | 80.00 | 170.00 | 212.5% | 350.00 | 80.00 | 270.00 | 337.5%",
            "H1     Expense |  |  |  |  |  |  |  |  |  |  |  | ",
            "D2         Rent | 0.00 | 0.00 | 0.00 |  | 50.00 | 0.00 | 50.00 |  | 50.00 | 0.00 | 50.00 | ",
            "H2         Utilities |  |  |  |  |  |  |  |  |  |  |  | ",
            "D3             Internet | 0.00 | 0.00 | 0.00 |  | 12.50 | 0.00 | 12.50 |  | 12.50 | 0.00 | 12.50 | ",
            "S2         Total Utilities | 0.00 | 0.00 | 0.00 |  | 12.50 | 0.00 | 12.50 |  | 12.50 | 0.00 | 12.50 | ",
            "S1     Total Expense | 0.00 | 0.00 | 0.00 |  | 62.50 | 0.00 | 62.50 |  | 62.50 | 0.00 | 62.50 | ",
            "S0 Net Ordinary Income | 100.00 | 0.00 | 100.00 |  | 187.50 | 80.00 | 107.50 | 134.4% | 287.50 | 80.00 | 207.50 | 259.4%",
            "T0 Net Income | 100.00 | 0.00 | 100.00 |  | 187.50 | 80.00 | 107.50 | 134.4% | 287.50 | 80.00 | 207.50 | 259.4%",
        ]
    );
}

#[test]
fn balance_sheet_with_comparison_columns_lays_out_negatives_in_parentheses() {
    let co = books();
    let bs = balance_sheet_vs_prev_year(&co);
    assert_eq!(
        grid_lines(&bs),
        [
            "= Balance Sheet Prev Year Comparison",
            "= Test Co",
            "= As of February 28, 2026",
            "= Accrual Basis",
            "L  | Feb 28, 26 | Feb 28, 25 | $ Change | % Change",
            "H0 ASSETS |  |  |  | ",
            "H1     Current Assets |  |  |  | ",
            "H2         Checking/Savings |  |  |  | ",
            "D3             Checking | 4,937.50 | 5,000.00 | (62.50) | -1.3%",
            "S2         Total Checking/Savings | 4,937.50 | 5,000.00 | (62.50) | -1.3%",
            "H2         Accounts Receivable |  |  |  | ",
            "D3             Accounts Receivable | 430.00 | 80.00 | 350.00 | 437.5%",
            "S2         Total Accounts Receivable | 430.00 | 80.00 | 350.00 | 437.5%",
            "S1     Total Current Assets | 5,367.50 | 5,080.00 | 287.50 | 5.7%",
            "T0 TOTAL ASSETS | 5,367.50 | 5,080.00 | 287.50 | 5.7%",
            "H0 LIABILITIES & EQUITY |  |  |  | ",
            "H1     Equity |  |  |  | ",
            "D2         Opening Balance Equity | 5,000.00 | 5,000.00 | 0.00 | 0.0%",
            "D2         Retained Earnings | 80.00 | 0.00 | 80.00 | ",
            "D2         Net Income | 287.50 | 80.00 | 207.50 | 259.4%",
            "S1     Total Equity | 5,367.50 | 5,080.00 | 287.50 | 5.7%",
            "T0 TOTAL LIABILITIES & EQUITY | 5,367.50 | 5,080.00 | 287.50 | 5.7%",
        ]
    );
}

#[test]
fn csv_follows_the_same_layout() {
    let co = books();
    let bs = balance_sheet_vs_prev_year(&co);
    let csv = String::from_utf8(lb_reports::export(&bs, ExportFormat::Csv).unwrap()).unwrap();
    assert!(csv.starts_with("Balance Sheet Prev Year Comparison\r\nTest Co\r\n\"As of February 28, 2026\"\r\nAccrual Basis\r\n\r\n,\"Feb 28, 26\",\"Feb 28, 25\",$ Change,% Change\r\nASSETS,,,,\r\n"), "{csv}");
    assert!(
        csv.contains("            Checking,4937.50,5000.00,-62.50,-1.3%\r\n"),
        "{csv}"
    );
    // The P&L's group headings repeat over every column they span.
    let pl = pl_by_month_vs_prev_year(&co);
    let csv = String::from_utf8(lb_reports::export(&pl, ExportFormat::Csv).unwrap()).unwrap();
    assert!(csv.contains("\r\n,Jan 26,Jan 26,Jan 26,Jan 26,Feb 26,Feb 26,Feb 26,Feb 26,TOTAL,TOTAL,TOTAL,TOTAL\r\n,Jan 26,Jan 25,$ Change,% Change,"), "{csv}");
}

/// The first cell in `range` whose text is `label`: its row.
fn row_of(range: &calamine::Range<Data>, label: &str) -> u32 {
    range
        .cells()
        .find(|(_, c, v)| *c == 0 && v.get_string().map(str::trim) == Some(label))
        .map_or_else(|| panic!("no row labelled {label:?}"), |(r, _, _)| r as u32)
}

#[test]
fn xlsx_round_trips_numbers_as_numbers() {
    let co = books();
    let bs = balance_sheet_vs_prev_year(&co);
    let bytes = lb_reports::export(&bs, ExportFormat::Xlsx).unwrap();
    assert_eq!(&bytes[..2], b"PK", "an xlsx is a zip");
    assert_eq!(
        lb_reports::file_name(&bs, ExportFormat::Xlsx),
        "Balance_Sheet_Prev_Year_Comparison.xlsx"
    );

    let mut wb: Xlsx<_> = Xlsx::new(Cursor::new(bytes)).unwrap();
    // Excel's 31-character limit cuts the name.
    assert_eq!(wb.sheet_names(), vec!["Balance Sheet Prev Year Compari"]);
    let range = wb
        .worksheet_range("Balance Sheet Prev Year Compari")
        .unwrap();
    let cell = |r: u32, c: u32| range.get_value((r, c)).cloned().unwrap_or(Data::Empty);

    // The title block, then a blank row, then the headings.
    assert_eq!(
        cell(0, 0),
        Data::String("Balance Sheet Prev Year Comparison".into())
    );
    assert_eq!(cell(1, 0), Data::String("Test Co".into()));
    assert_eq!(cell(2, 0), Data::String("As of February 28, 2026".into()));
    assert_eq!(cell(3, 0), Data::String("Accrual Basis".into()));
    assert_eq!(cell(4, 0), Data::Empty);
    assert_eq!(cell(5, 1), Data::String("Feb 28, 26".into()));
    assert_eq!(cell(5, 4), Data::String("% Change".into()));

    // Amounts are numbers in dollars; percents are fractions.
    let checking = row_of(&range, "Checking");
    assert_eq!(cell(checking, 1), Data::Float(4937.5));
    assert_eq!(cell(checking, 2), Data::Float(5000.0));
    assert_eq!(cell(checking, 3), Data::Float(-62.5));
    let Data::Float(pct) = cell(checking, 4) else {
        panic!("percent is a number: {:?}", cell(checking, 4));
    };
    assert!((pct + 0.013).abs() < 1e-9, "{pct}");
    let total = row_of(&range, "TOTAL LIABILITIES & EQUITY");
    assert_eq!(cell(total, 1), Data::Float(5367.5));
    // A percent the report left blank stays blank.
    let re = row_of(&range, "Retained Earnings");
    assert_eq!(cell(re, 4), Data::Empty);

    // The footer closes the sheet.
    let last = range.end().unwrap().0;
    assert_eq!(
        cell(last, 0),
        Data::String("Generated 2026-10-19 21:04 UTC by LibreBooks".into())
    );
}

#[test]
fn xlsx_dates_are_excel_dates_and_group_headings_merge() {
    let co = books();
    let list = fixed_time(co.run(&req(
        ReportId::TransactionListByDate,
        d(2026, 2, 1),
        d(2026, 2, 1),
    )));
    let bytes = lb_reports::export(&list, ExportFormat::Xlsx).unwrap();
    let mut wb: Xlsx<_> = Xlsx::new(Cursor::new(bytes)).unwrap();
    let range = wb.worksheet_range("Transaction List by Date").unwrap();
    let date_col = list.column_index("date").expect("a date column") as u32;
    let serial = range
        .cells()
        .find_map(|(_, c, v)| match v {
            Data::DateTime(dt) if c as u32 == date_col => Some(dt.as_f64()),
            _ => None,
        })
        .expect("a date cell");
    // Excel's day 1 is 1900-01-01 (with its 1900 leap-year quirk): 2026-02-01.
    assert!((serial - 46054.0).abs() < 1e-9, "{serial}");

    let pl = pl_by_month_vs_prev_year(&co);
    let bytes = lb_reports::export(&pl, ExportFormat::Xlsx).unwrap();
    let mut wb: Xlsx<_> = Xlsx::new(Cursor::new(bytes)).unwrap();
    let regions: Vec<(u32, u32, u32, u32)> = wb
        .merge_cells_by_sheet_name("Profit & Loss")
        .unwrap()
        .iter()
        .map(|d| (d.start.0, d.start.1, d.end.0, d.end.1))
        .collect();
    // Title lines span the table; "Jan 26" spans its four columns on the group row.
    assert!(regions.contains(&(0, 0, 0, 12)), "{regions:?}");
    assert!(regions.contains(&(5, 1, 5, 4)), "{regions:?}");
    assert!(regions.contains(&(5, 9, 5, 12)), "{regions:?}");
}

fn pdf_text(doc: &lopdf::Document, page: u32) -> String {
    doc.extract_text(&[page]).unwrap()
}

fn media_box(doc: &lopdf::Document, page: u32) -> (f32, f32) {
    let id = doc.get_pages()[&page];
    let page = doc.get_object(id).unwrap().as_dict().unwrap();
    let mb = page.get(b"MediaBox").unwrap().as_array().unwrap();
    let n = |o: &lopdf::Object| o.as_float().unwrap();
    (n(&mb[2]) - n(&mb[0]), n(&mb[3]) - n(&mb[1]))
}

#[test]
fn pdf_is_well_formed_and_carries_the_title_block() {
    let co = books();
    let bs = balance_sheet_vs_prev_year(&co);
    let bytes = lb_reports::export(&bs, ExportFormat::Pdf).unwrap();
    assert!(bytes.starts_with(b"%PDF-"), "header");
    let tail = String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(16)..]);
    assert!(tail.trim_end().ends_with("%%EOF"), "trailer: {tail:?}");
    assert_eq!(
        lb_reports::file_name(&bs, ExportFormat::Pdf),
        "Balance_Sheet_Prev_Year_Comparison.pdf"
    );

    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.get_pages().len(), 1);
    let text = pdf_text(&doc, 1);
    for needle in [
        "Test Co",
        "Balance Sheet Prev Year Comparison",
        "As of February 28, 2026",
        "Accrual Basis",
        "Checking",
        "4,937.50",
        "(62.50)",
        "TOTAL LIABILITIES & EQUITY",
        "Page 1 of 1",
        "Generated 2026-10-19 21:04 UTC by LibreBooks",
    ] {
        assert!(text.contains(needle), "{needle:?} not in:\n{text}");
    }
    // Letter, portrait: five columns fit.
    let (w, h) = media_box(&doc, 1);
    assert!(
        (w - 612.0).abs() < 0.5 && (h - 792.0).abs() < 0.5,
        "{w}x{h}"
    );
}

#[test]
fn pdf_paginates_with_repeated_headings_and_a4_landscape_when_asked() {
    let co = books();
    for i in 0..160_i64 {
        co.check(
            d(2026, 3, 1 + (i % 28) as i8),
            co.landlord,
            &format!("{}", 100 + i),
            &[(co.supplies, 1_000 + i)],
        );
    }
    let list = fixed_time(co.run(&req(
        ReportId::TransactionListByDate,
        d(2026, 3, 1),
        d(2026, 3, 31),
    )));
    let bytes = lb_reports::export_with(
        &list,
        ExportFormat::Pdf,
        &ExportOptions { paper: Paper::A4 },
    )
    .unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    let pages = doc.get_pages().len() as u32;
    assert!(pages >= 3, "{pages} pages for 160 lines");
    for p in 1..=pages {
        let text = pdf_text(&doc, p);
        assert!(text.contains("Transaction List by Date"), "page {p} title");
        assert!(
            text.contains("Memo"),
            "page {p} repeats the headings:\n{text}"
        );
        assert!(
            text.contains(&format!("Page {p} of {pages}")),
            "page {p} number:\n{text}"
        );
    }
    // A4; its ten short columns still fit portrait.
    let (w, h) = media_box(&doc, 1);
    assert!(
        (w - 595.28).abs() < 0.5 && (h - 841.89).abs() < 0.5,
        "{w}x{h}"
    );

    // Thirteen columns of amounts don't: landscape, and the columns squeezed to
    // the page.
    let pl = pl_by_month_vs_prev_year(&co);
    let bytes = lb_reports::export(&pl, ExportFormat::Pdf).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    let (w, h) = media_box(&doc, 1);
    assert!(
        (w - 792.0).abs() < 0.5 && (h - 612.0).abs() < 0.5,
        "{w}x{h}"
    );
    let text = pdf_text(&doc, 1);
    assert!(text.contains("Jan - Feb 25"), "{text}");
    assert!(text.contains("259.4%"), "{text}");
}

#[test]
fn every_report_exports_in_every_format() {
    let co = books();
    for entry in lb_reports::catalog() {
        let mut request = ReportRequest::new(entry.id).today(d(2026, 10, 19));
        let rep = match lb_reports::run(&co.b, &request) {
            Ok(r) => r,
            // An Account QuickReport needs its account.
            Err(ReportError::Invalid(_)) => {
                request.filters.accounts = vec![co.bank];
                co.run(&request)
            }
            Err(e) => panic!("{:?}: {e}", entry.id),
        };
        for format in [ExportFormat::Csv, ExportFormat::Xlsx, ExportFormat::Pdf] {
            let bytes = lb_reports::export(&rep, format)
                .unwrap_or_else(|e| panic!("{:?} as {format:?}: {e}", entry.id));
            assert!(!bytes.is_empty(), "{:?} as {format:?}", entry.id);
        }
    }
}
