//! Profit & loss, balance sheet, trial balance and cash flows against books whose
//! every number is worked out by hand in the comments.

mod common;

use common::{Co, amt, company, d, req, same_amounts};
use lb_core::{AccountType, DateMacro, Money};
use lb_reports::{Basis, ColumnLayout, Drill, ReportId, RowKind};
use pretty_assertions::assert_eq;

/// Q1 2026:
/// - Jan 10 invoice Acme: Labor $1,000 (Consulting) + Parts $500 (Materials) = $1,500
/// - Jan 20 check #101 rent $400
/// - Jan 25 bill Landlord: supplies $300, due Feb 24
/// - Feb 5 Acme pays $600 of the invoice (straight to the bank)
/// - Feb 10 bill payment #102 $100 of the bill
fn partly_paid() -> (Co, lb_core::Txn) {
    let co = company(1);
    let inv = co.invoice(
        d(2026, 1, 10),
        co.acme,
        &[(co.labor, 100_000), (co.parts, 50_000)],
    );
    co.check(d(2026, 1, 20), co.landlord, "101", &[(co.rent, 40_000)]);
    let bill = co.bill(
        d(2026, 1, 25),
        d(2026, 2, 24),
        co.landlord,
        &[(co.supplies, 30_000)],
    );
    co.payment(d(2026, 2, 5), co.acme, 60_000, &[(inv.id, 60_000)]);
    co.bill_payment(d(2026, 2, 10), co.landlord, "102", &[(bill.id, 10_000)]);
    (co, inv)
}

#[test]
fn profit_and_loss_accrual_vs_cash_for_a_partly_paid_invoice() {
    let (co, _) = partly_paid();
    let q1 = req(ReportId::ProfitAndLoss, d(2026, 1, 1), d(2026, 3, 31));

    // Accrual: income 1,500; expenses 400 + 300; net 800.
    let acc = co.run(&q1.clone().basis(Basis::Accrual));
    assert_eq!(acc.basis, Some(Basis::Accrual));
    assert_eq!(acc.subtitle, "January through March 2026");
    assert_eq!(amt(&acc, "Consulting Income", "c0"), 100_000);
    assert_eq!(amt(&acc, "Materials Income", "c0"), 50_000);
    assert_eq!(amt(&acc, "Total Income", "c0"), 150_000);
    assert_eq!(amt(&acc, "Rent", "c0"), 40_000);
    assert_eq!(amt(&acc, "Supplies", "c0"), 30_000);
    assert_eq!(amt(&acc, "Total Expense", "c0"), 70_000);
    assert_eq!(amt(&acc, "Net Ordinary Income", "c0"), 80_000);
    assert_eq!(amt(&acc, "Net Income", "c0"), 80_000);

    // Cash: $600 of $1,500 paid recognises 600 × 1000/1500 = 400 consulting and
    // 600 × 500/1500 = 200 materials; rent 400 paid; 100 of the 300 bill paid.
    let cash = co.run(&q1.clone().basis(Basis::Cash));
    assert_eq!(cash.basis, Some(Basis::Cash));
    assert_eq!(amt(&cash, "Consulting Income", "c0"), 40_000);
    assert_eq!(amt(&cash, "Materials Income", "c0"), 20_000);
    assert_eq!(amt(&cash, "Total Income", "c0"), 60_000);
    assert_eq!(amt(&cash, "Supplies", "c0"), 10_000);
    assert_eq!(amt(&cash, "Total Expense", "c0"), 50_000);
    assert_eq!(amt(&cash, "Net Income", "c0"), 10_000);

    // January alone on cash: nothing was collected yet; only the check went out.
    let jan =
        co.run(&req(ReportId::ProfitAndLoss, d(2026, 1, 1), d(2026, 1, 31)).basis(Basis::Cash));
    assert!(jan.row("Consulting Income").is_none());
    assert_eq!(amt(&jan, "Net Income", "c0"), -40_000);
}

#[test]
fn cash_basis_balance_sheet_drops_ar_and_ap_and_still_balances() {
    let (co, _) = partly_paid();
    let bs =
        |basis| co.run(&req(ReportId::BalanceSheet, d(2026, 1, 1), d(2026, 3, 31)).basis(basis));
    // Accrual: bank 600 − 400 − 100 = 100; A/R 1,500 − 600 = 900; A/P 200;
    // net income 800.
    let acc = bs(Basis::Accrual);
    assert_eq!(acc.subtitle, "As of March 31, 2026");
    assert_eq!(amt(&acc, "Checking", "c0"), 10_000);
    assert_eq!(amt(&acc, "Accounts Receivable", "c0"), 90_000);
    assert_eq!(amt(&acc, "TOTAL ASSETS", "c0"), 100_000);
    assert_eq!(amt(&acc, "Accounts Payable", "c0"), 20_000);
    assert_eq!(amt(&acc, "Net Income", "c0"), 80_000);
    same_amounts(&acc, "TOTAL ASSETS", "TOTAL LIABILITIES & EQUITY");

    // Cash: A/R and A/P come to nothing (everything unpaid was income or
    // expense), so assets are the bank's 100 and equity the cash net income 100.
    let cash = bs(Basis::Cash);
    assert!(
        cash.row("Accounts Receivable").is_none(),
        "{:#?}",
        cash.rows
    );
    assert!(cash.row("Accounts Payable").is_none());
    assert_eq!(amt(&cash, "TOTAL ASSETS", "c0"), 10_000);
    assert_eq!(amt(&cash, "Net Income", "c0"), 10_000);
    same_amounts(&cash, "TOTAL ASSETS", "TOTAL LIABILITIES & EQUITY");
}

#[test]
fn cash_basis_keeps_accrual_sales_tax_in_ar_as_qbd_does() {
    let co = company(1);
    let (tax_item, taxable, _agency) = co.sales_tax();
    let mut inv = lb_core::Txn::new(lb_core::TxnType::Invoice, d(2026, 3, 1));
    inv.entity_id = Some(co.globex);
    inv.sales_tax_item_id = Some(tax_item);
    let mut line = lb_core::TxnLine::item_amount(co.labor, Money(100_000));
    line.sales_tax_code_id = Some(taxable);
    inv.lines.push(line);
    let inv = co.save(inv);
    assert_eq!(inv.amount, Money(110_000));
    co.payment(d(2026, 3, 15), co.globex, 55_000, &[(inv.id, 55_000)]);

    // Half paid: income recognised 500 of 1,000. Sales tax is owed when invoiced
    // (the default), so Sales Tax Payable is the whole 100 and the unpaid half of
    // the tax (50) stays in A/R even on cash basis.
    let bs = co.run(&req(ReportId::BalanceSheet, d(2026, 1, 1), d(2026, 3, 31)).basis(Basis::Cash));
    assert_eq!(amt(&bs, "Checking", "c0"), 55_000);
    assert_eq!(amt(&bs, "Accounts Receivable", "c0"), 5_000);
    assert_eq!(amt(&bs, "Sales Tax Payable", "c0"), 10_000);
    assert_eq!(amt(&bs, "Net Income", "c0"), 50_000);
    same_amounts(&bs, "TOTAL ASSETS", "TOTAL LIABILITIES & EQUITY");
}

#[test]
fn cash_basis_allocation_is_exact_over_several_payments() {
    let co = company(1);
    // $1,000.00 in three lines that don't divide evenly, paid in three payments.
    let inv = co.invoice(
        d(2026, 4, 1),
        co.globex,
        &[(co.labor, 33_333), (co.parts, 33_333), (co.labor, 33_334)],
    );
    for (i, p) in [33_333, 33_333, 33_334].into_iter().enumerate() {
        co.payment(d(2026, 4, 10 + i as i8), co.globex, p, &[(inv.id, p)]);
    }
    let pl = |to| co.run(&req(ReportId::ProfitAndLoss, d(2026, 4, 1), to).basis(Basis::Cash));
    // After the first payment, a third: 1/3 of each line, to the cent and summing
    // to exactly what was paid.
    let first = pl(d(2026, 4, 10));
    assert_eq!(amt(&first, "Total Income", "c0"), 33_333);
    // Fully paid: every account has exactly its lines' amounts.
    let all = pl(d(2026, 4, 30));
    assert_eq!(amt(&all, "Consulting Income", "c0"), 66_667);
    assert_eq!(amt(&all, "Materials Income", "c0"), 33_333);
    assert_eq!(amt(&all, "Total Income", "c0"), 100_000);
}

#[test]
fn cash_basis_with_a_discount_line_splits_signed_amounts() {
    let co = company(1);
    // Labor $1,000 less a $100 discount: $900 owed; half paid.
    let inv = co.invoice(
        d(2026, 5, 1),
        co.globex,
        &[(co.labor, 100_000), (co.discount, -10_000)],
    );
    co.payment(d(2026, 5, 20), co.globex, 45_000, &[(inv.id, 45_000)]);
    let pl =
        co.run(&req(ReportId::ProfitAndLoss, d(2026, 5, 1), d(2026, 5, 31)).basis(Basis::Cash));
    assert_eq!(amt(&pl, "Consulting Income", "c0"), 50_000);
    assert_eq!(amt(&pl, "Discounts Given", "c0"), -5_000);
    assert_eq!(amt(&pl, "Total Income", "c0"), 45_000);
}

/// A July fiscal year. FY2025 is Jul 2024–Jun 2025; FY2026 is Jul 2025–Jun 2026.
/// - 2025-01-15 opening balance: bank 10,000 against Opening Balance Equity
/// - 2025-03-01 invoice Globex $2,000 (FY2025 income)
/// - 2025-05-01 check rent $500 (FY2025 expense)
/// - 2025-08-15 Globex pays $2,000
/// - 2025-09-01 invoice Globex $1,200 (FY2026)
/// - 2025-10-01 check rent $300 (FY2026)
/// - 2025-11-01 owner draw $100 posted straight to Retained Earnings
fn july_books() -> Co {
    let co = company(7);
    co.journal(
        d(2025, 1, 15),
        &[(co.bank, 1_000_000), (co.obe, -1_000_000)],
    );
    let inv = co.invoice(d(2025, 3, 1), co.globex, &[(co.labor, 200_000)]);
    co.check(d(2025, 5, 1), co.landlord, "1", &[(co.rent, 50_000)]);
    co.payment(d(2025, 8, 15), co.globex, 200_000, &[(inv.id, 200_000)]);
    co.invoice(d(2025, 9, 1), co.globex, &[(co.labor, 120_000)]);
    co.check(d(2025, 10, 1), co.landlord, "2", &[(co.rent, 30_000)]);
    co.journal(d(2025, 11, 1), &[(co.re, 10_000), (co.bank, -10_000)]);
    co
}

#[test]
fn balance_sheet_carries_retained_earnings_across_a_july_fiscal_year() {
    let co = july_books();
    let bs = |to| co.run(&req(ReportId::BalanceSheet, d(2024, 7, 1), to).basis(Basis::Accrual));
    // End of FY2025: bank 10,000 − 500; A/R 2,000; FY2025 net income 1,500,
    // nothing yet retained.
    let jun = bs(d(2025, 6, 30));
    assert_eq!(amt(&jun, "Checking", "c0"), 950_000);
    assert_eq!(amt(&jun, "Accounts Receivable", "c0"), 200_000);
    assert_eq!(amt(&jun, "Opening Balance Equity", "c0"), 1_000_000);
    assert_eq!(amt(&jun, "Retained Earnings", "c0"), 0);
    assert_eq!(amt(&jun, "Net Income", "c0"), 150_000);
    same_amounts(&jun, "TOTAL ASSETS", "TOTAL LIABILITIES & EQUITY");

    // Mid FY2026: bank 9,500 + 2,000 − 300 − 100 = 11,100; A/R 1,200. Retained
    // earnings = FY2025's 1,500 less the 100 draw = 1,400; net income since
    // July 1 = 1,200 − 300 = 900. 10,000 + 1,400 + 900 = 12,300 = assets.
    let dec = bs(d(2025, 12, 31));
    assert_eq!(amt(&dec, "Checking", "c0"), 1_110_000);
    assert_eq!(amt(&dec, "Accounts Receivable", "c0"), 120_000);
    assert_eq!(amt(&dec, "TOTAL ASSETS", "c0"), 1_230_000);
    assert_eq!(amt(&dec, "Retained Earnings", "c0"), 140_000);
    assert_eq!(amt(&dec, "Net Income", "c0"), 90_000);
    assert_eq!(amt(&dec, "Total Equity", "c0"), 1_230_000);
    same_amounts(&dec, "TOTAL ASSETS", "TOTAL LIABILITIES & EQUITY");

    // The same on cash basis: the $1,200 invoice is unpaid, so A/R is gone and
    // FY2026 net income is −300; FY2025 had no cash income (paid in August), so
    // retained earnings are −500 − 100 = −600; FY2026 adds 2,000 − 300 = 1,700.
    let cash =
        co.run(&req(ReportId::BalanceSheet, d(2024, 7, 1), d(2025, 12, 31)).basis(Basis::Cash));
    assert!(cash.row("Accounts Receivable").is_none());
    assert_eq!(amt(&cash, "Retained Earnings", "c0"), -60_000);
    assert_eq!(amt(&cash, "Net Income", "c0"), 170_000);
    same_amounts(&cash, "TOTAL ASSETS", "TOTAL LIABILITIES & EQUITY");
}

#[test]
fn balance_sheet_by_month_and_prev_year_balance_in_every_column() {
    let co = july_books();
    let mut request =
        req(ReportId::BalanceSheet, d(2025, 1, 1), d(2025, 12, 31)).columns(ColumnLayout::Month);
    request.percent_of_column = true;
    let bs = co.run(&request);
    // Twelve month-end columns, each with its % of Column.
    let money_cols = bs
        .columns
        .iter()
        .filter(|c| c.kind == lb_reports::ColumnKind::Money)
        .count();
    assert_eq!(money_cols, 12);
    same_amounts(&bs, "TOTAL ASSETS", "TOTAL LIABILITIES & EQUITY");
    assert_eq!(bs.columns[1].label, "Jan 31, 25");
    // % of Column for Checking at Dec 31: 11,100 / 12,300 = 90.2%.
    let row = bs.row("Checking").unwrap();
    let last_pct = row.cells.last().unwrap();
    assert_eq!(last_pct.value, lb_reports::Value::Percent("90.2".into()));

    let py = co.run(&req(
        ReportId::BalanceSheetPrevYearComparison,
        d(2025, 1, 1),
        d(2025, 12, 31),
    ));
    let labels: Vec<&str> = py.columns.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        vec!["", "Dec 31, 25", "Dec 31, 24", "$ Change", "% Change"]
    );
    same_amounts(&py, "TOTAL ASSETS", "TOTAL LIABILITIES & EQUITY");
    assert_eq!(amt(&py, "TOTAL ASSETS", "c1"), 0, "nothing a year earlier");
    assert_eq!(amt(&py, "TOTAL ASSETS", "d0_2"), 1_230_000);
}

#[test]
fn trial_balance_balances_with_retained_earnings_carried() {
    let co = july_books();
    let tb = co.run(&req(ReportId::TrialBalance, d(2025, 7, 1), d(2025, 12, 31)));
    // Debits: bank 11,100, A/R 1,200, rent (FY2026 only) 300 = 12,600.
    // Credits: OBE 10,000, retained earnings 1,400, consulting (FY2026) 1,200.
    assert_eq!(amt(&tb, "Checking", "debit"), 1_110_000);
    assert_eq!(amt(&tb, "Rent", "debit"), 30_000);
    assert_eq!(amt(&tb, "Retained Earnings", "credit"), 140_000);
    assert_eq!(amt(&tb, "Consulting Income", "credit"), 120_000);
    assert_eq!(amt(&tb, "TOTAL", "debit"), 1_260_000);
    assert_eq!(amt(&tb, "TOTAL", "credit"), 1_260_000);
}

#[test]
fn profit_and_loss_by_month_and_comparisons() {
    let co = company(1);
    co.invoice(d(2026, 1, 5), co.globex, &[(co.labor, 10_000)]);
    co.invoice(d(2026, 2, 5), co.globex, &[(co.labor, 20_000)]);
    co.invoice(d(2026, 3, 5), co.globex, &[(co.parts, 30_000)]);
    co.check(d(2026, 2, 1), co.landlord, "5", &[(co.rent, 5_000)]);
    co.invoice(d(2025, 2, 5), co.globex, &[(co.labor, 8_000)]);

    let mut request =
        req(ReportId::ProfitAndLoss, d(2026, 1, 1), d(2026, 3, 31)).columns(ColumnLayout::Month);
    request.percent_of_income = true;
    let pl = co.run(&request);
    let labels: Vec<&str> = pl.columns.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        vec![
            "",
            "Jan 26",
            "% of Income",
            "Feb 26",
            "% of Income",
            "Mar 26",
            "% of Income",
            "TOTAL",
            "% of Income"
        ]
    );
    assert_eq!(amt(&pl, "Consulting Income", "c0"), 10_000);
    assert_eq!(amt(&pl, "Consulting Income", "c1"), 20_000);
    assert_eq!(amt(&pl, "Consulting Income", "c2"), 0);
    assert_eq!(amt(&pl, "Materials Income", "c2"), 30_000);
    assert_eq!(amt(&pl, "Total Income", "c3"), 60_000);
    assert_eq!(amt(&pl, "Net Income", "c1"), 15_000);
    assert_eq!(amt(&pl, "Net Income", "c3"), 55_000);
    // Rent is 5,000 of February's 20,000 income: 25.0%.
    let rent = pl.row("Rent").unwrap();
    assert_eq!(
        rent.cells[4].value,
        lb_reports::Value::Percent("25.0".into())
    );

    // Previous year: February 2026 against February 2025.
    let py = co.run(&req(
        ReportId::ProfitAndLossPrevYearComparison,
        d(2026, 2, 1),
        d(2026, 2, 28),
    ));
    let labels: Vec<&str> = py.columns.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, vec!["", "Feb 26", "Feb 25", "$ Change", "% Change"]);
    assert_eq!(amt(&py, "Consulting Income", "c0"), 20_000);
    assert_eq!(amt(&py, "Consulting Income", "c1"), 8_000);
    assert_eq!(amt(&py, "Consulting Income", "d0_2"), 12_000);
    let row = py.row("Consulting Income").unwrap();
    assert_eq!(
        row.cells[4].value,
        lb_reports::Value::Percent("150.0".into())
    );

    // YTD comparison: March beside January–March.
    let ytd = co.run(&req(
        ReportId::ProfitAndLossYtdComparison,
        d(2026, 3, 1),
        d(2026, 3, 31),
    ));
    assert_eq!(amt(&ytd, "Total Income", "c0"), 30_000);
    assert_eq!(amt(&ytd, "Total Income", "c1"), 60_000);
}

#[test]
fn subaccounts_indent_with_other_and_total_lines() {
    let co = company(1);
    co.check(
        d(2026, 6, 1),
        co.landlord,
        "9",
        &[(co.internet, 7_000), (co.utilities, 3_000)],
    );
    let pl = co.run(&req(ReportId::ProfitAndLoss, d(2026, 6, 1), d(2026, 6, 30)));
    let parent = pl.row("Utilities").unwrap();
    assert_eq!(parent.kind, RowKind::Header);
    let child = pl.row("Internet").unwrap();
    assert_eq!(child.depth, parent.depth + 1);
    assert_eq!(amt(&pl, "Internet", "c0"), 7_000);
    assert_eq!(amt(&pl, "Utilities - Other", "c0"), 3_000);
    assert_eq!(amt(&pl, "Total Utilities", "c0"), 10_000);
    // The "- Other" line's QuickZoom leaves out the subaccount.
    let other = pl.row("Utilities - Other").unwrap();
    match &other.cells[1].drill {
        Some(Drill::Report { request }) => {
            assert!(request.filters.exact);
            assert_eq!(request.filters.accounts, vec![co.utilities]);
            let detail = co.run(request);
            assert_eq!(amt(&detail, "TOTAL", "amount"), 3_000);
        }
        other => panic!("expected a report drill, got {other:?}"),
    }
}

#[test]
fn quickzoom_from_a_summary_amount_reaches_its_transactions() {
    let (co, inv) = partly_paid();
    let pl = co.run(&req(ReportId::ProfitAndLoss, d(2026, 1, 1), d(2026, 3, 31)));
    let row = pl.row("Consulting Income").unwrap();
    let Some(Drill::Report { request }) = &row.cells[1].drill else {
        panic!("no drill on {row:?}");
    };
    assert_eq!(request.report, ReportId::TransactionDetailByAccount);
    assert_eq!(request.from, Some(d(2026, 1, 1)));
    assert_eq!(request.to, Some(d(2026, 3, 31)));
    assert_eq!(request.basis, Some(Basis::Accrual));
    let detail = co.run(request);
    assert_eq!(amt(&detail, "Total Consulting Income", "amount"), 100_000);
    // Each line opens its transaction.
    let line = detail
        .rows
        .iter()
        .find(|r| r.kind == RowKind::Data)
        .unwrap();
    assert_eq!(line.drill, Some(Drill::Txn { txn_id: inv.id }));
    // Net Income zooms to the P&L detail, which ties out.
    let Some(Drill::Report { request }) = &pl.row("Net Income").unwrap().drill else {
        panic!()
    };
    let detail = co.run(request);
    assert_eq!(amt(&detail, "Net Income", "amount"), 80_000);
}

#[test]
fn cash_basis_detail_ties_to_the_summary() {
    let (co, _) = partly_paid();
    let q1 = req(ReportId::ProfitAndLossDetail, d(2026, 1, 1), d(2026, 3, 31)).basis(Basis::Cash);
    let detail = co.run(&q1);
    assert_eq!(amt(&detail, "Total Consulting Income", "amount"), 40_000);
    assert_eq!(amt(&detail, "Net Income", "amount"), 10_000);
    // The recognised income is dated on the payment and opens it.
    let line = detail
        .rows
        .iter()
        .find(|r| r.kind == RowKind::Data && r.cells[1].as_text() == Some("Payment"))
        .expect("a payment line");
    let date_col = detail.column_index("date").unwrap();
    assert_eq!(
        line.cells[date_col].value,
        lb_reports::Value::Date(d(2026, 2, 5))
    );
}

#[test]
fn general_ledger_and_journal_balance() {
    let co = july_books();
    let gl = co.run(&req(
        ReportId::GeneralLedger,
        d(2025, 7, 1),
        d(2025, 12, 31),
    ));
    assert_eq!(amt(&gl, "TOTAL", "debit"), amt(&gl, "TOTAL", "credit"));
    // Checking brought forward 9,500 at July 1, ends at 11,100.
    let check = gl.row("Checking").unwrap();
    let bal = gl.column_index("balance").unwrap();
    assert_eq!(check.cells[bal].as_money(), Some(Money(950_000)));
    assert_eq!(amt(&gl, "Total Checking", "balance"), 1_110_000);

    let j = co.run(&req(ReportId::Journal, d(2025, 1, 1), d(2025, 12, 31)));
    assert_eq!(amt(&j, "TOTAL", "debit"), amt(&j, "TOTAL", "credit"));
    assert!(amt(&j, "TOTAL", "debit") > 0);
}

#[test]
fn statement_of_cash_flows_reconciles_to_the_bank() {
    let co = july_books();
    co.journal(
        d(2025, 9, 15),
        &[(co.truck, 2_000_000), (co.loan, -2_000_000)],
    );
    let scf = co.run(&req(
        ReportId::StatementOfCashFlows,
        d(2025, 7, 1),
        d(2025, 12, 31),
    ));
    // Net income 900; A/R went from 2,000 to 1,200 (+800); the truck bought on
    // a loan is −20,000 investing and +20,000 financing; the draw −100.
    assert_eq!(amt(&scf, "Net Income", "amount"), 90_000);
    assert_eq!(amt(&scf, "Accounts Receivable", "amount"), 80_000);
    assert_eq!(
        amt(&scf, "Net cash provided by Operating Activities", "amount"),
        170_000
    );
    assert_eq!(amt(&scf, "Truck", "amount"), -2_000_000);
    assert_eq!(amt(&scf, "Truck Loan", "amount"), 2_000_000);
    assert_eq!(amt(&scf, "Retained Earnings", "amount"), -10_000);
    assert_eq!(amt(&scf, "Net cash increase for period", "amount"), 160_000);
    assert_eq!(amt(&scf, "Cash at beginning of period", "amount"), 950_000);
    assert_eq!(amt(&scf, "Cash at end of period", "amount"), 1_110_000);
    assert!(scf.notes.is_empty(), "{:?}", scf.notes);
}

#[test]
fn date_macros_resolve_against_the_supplied_today() {
    let co = july_books();
    let mut request = lb_reports::ReportRequest::new(ReportId::BalanceSheet).today(d(2025, 12, 31));
    request.date_macro = Some(DateMacro::ThisFiscalYearToDate);
    let bs = co.run(&request);
    assert_eq!(bs.subtitle, "As of December 31, 2025");
    assert_eq!(amt(&bs, "Net Income", "c0"), 90_000);
    // No dates at all: the report's default (P&L: this month to date).
    let pl =
        co.run(&lb_reports::ReportRequest::new(ReportId::ProfitAndLoss).today(d(2025, 10, 15)));
    assert_eq!(pl.subtitle, "October 1 - 15, 2025");
    assert_eq!(amt(&pl, "Rent", "c0"), 30_000);
    let _ = AccountType::Bank;
}
