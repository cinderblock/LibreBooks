//! Receivables and payables, jobs, sales, inventory, banking, tax, CSV export,
//! and every report run end to end.

mod common;

use common::{Co, amt, company, company_named, d, req, same_amounts};
use lb_core::{
    AccountRole, AccountType, Application, ClassId, DateMacro, Item, ItemType, LineKind, Money,
    Txn, TxnLine, TxnType,
};
use lb_reports::{
    Basis, CsvOptions, Drill, ExportFormat, Report, ReportId, ReportRequest, RowKind, Value,
};
use lb_store::{Actor, accounts, items, txns};
use pretty_assertions::assert_eq;
use rust_decimal::Decimal;

/// Acme owes, as of April 5 2026 (before a later payment):
/// - invoice A, Jan 1, due Jan 31, $1,000 — 64 days past due on Apr 5
/// - credit memo, Feb 15, $100 — unapplied, 49 days old
/// - invoice B, Mar 1, due Mar 31, $500 — 5 days past due
/// - invoice to job Phase 1, Mar 20, due Apr 19, $800, of which Acme paid $300
///   on Mar 25 — not yet due
/// - Apr 10: Acme pays invoice A in full (after the aging date)
fn receivables() -> Co {
    let co = company(1);
    let a = co.invoice_due(d(2026, 1, 1), d(2026, 1, 31), co.acme, 100_000);
    co.credit_memo(d(2026, 2, 15), co.acme, 10_000);
    co.invoice_due(d(2026, 3, 1), d(2026, 3, 31), co.acme, 50_000);
    let p1 = co.invoice_due(d(2026, 3, 20), d(2026, 4, 19), co.phase1, 80_000);
    co.payment(d(2026, 3, 25), co.acme, 30_000, &[(p1.id, 30_000)]);
    co.payment(d(2026, 4, 10), co.acme, 100_000, &[(a.id, 100_000)]);
    co
}

fn as_of(id: ReportId, day: jiff::civil::Date) -> ReportRequest {
    req(id, day, day)
}

#[test]
fn ar_aging_as_of_a_past_date_ignores_a_later_payment() {
    let co = receivables();
    // Today the invoice is paid, so its stored open balance is zero...
    let stored =
        co.b.read(|c| {
            txns::query(
                c,
                &txns::TxnFilter {
                    types: vec![TxnType::Invoice],
                    open_only: true,
                    ..Default::default()
                },
            )
        })
        .unwrap();
    assert_eq!(stored.len(), 2, "only B and the job's invoice are open now");

    // ...but on April 5 it was owed in full, 64 days past due.
    let aging = co.run(&as_of(ReportId::ArAgingSummary, d(2026, 4, 5)));
    let labels: Vec<&str> = aging.columns.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        vec![
            "", "Current", "1 - 30", "31 - 60", "61 - 90", "> 90", "TOTAL"
        ]
    );
    // Acme's own: B 500 (1–30), the credit −100 (31–60), A 1,000 (61–90).
    assert_eq!(amt(&aging, "Acme - Other", "b1"), 50_000);
    assert_eq!(amt(&aging, "Acme - Other", "b2"), -10_000);
    assert_eq!(amt(&aging, "Acme - Other", "b3"), 100_000);
    assert_eq!(amt(&aging, "Acme - Other", "total"), 140_000);
    // The job's invoice, less the $300 its customer paid on it, is not yet due.
    assert_eq!(amt(&aging, "Phase 1", "b0"), 50_000);
    assert_eq!(amt(&aging, "Total Acme", "total"), 190_000);
    assert_eq!(amt(&aging, "TOTAL", "total"), 190_000);
    let phase1 = aging.row("Phase 1").unwrap();
    assert_eq!(phase1.depth, aging.row("Acme").unwrap().depth + 1);

    // After the payment: A is gone; the credit is now 59 days old.
    let later = co.run(&as_of(ReportId::ArAgingSummary, d(2026, 4, 15)));
    assert_eq!(amt(&later, "Acme - Other", "b3"), 0);
    assert_eq!(amt(&later, "Acme - Other", "b2"), -10_000);
    assert_eq!(amt(&later, "TOTAL", "total"), 90_000);
}

#[test]
fn ar_aging_detail_open_invoices_and_balances_agree() {
    let co = receivables();
    let day = d(2026, 4, 5);
    let detail = co.run(&as_of(ReportId::ArAgingDetail, day));
    assert_eq!(amt(&detail, "Total 61 - 90", "open"), 100_000);
    assert_eq!(amt(&detail, "Total Current", "open"), 50_000);
    assert_eq!(amt(&detail, "TOTAL", "open"), 190_000);
    let aging_col = detail.column_index("aging").unwrap();
    let a_row = detail
        .rows
        .iter()
        .find(|r| r.kind == RowKind::Data && r.cells[aging_col].value == Value::Number("64".into()))
        .expect("invoice A, 64 days");
    assert!(matches!(a_row.drill, Some(Drill::Txn { .. })));

    let open = co.run(&as_of(ReportId::OpenInvoices, day));
    assert_eq!(amt(&open, "Total Acme", "open"), 140_000);
    assert_eq!(amt(&open, "Total Acme:Phase 1", "open"), 50_000);
    assert_eq!(amt(&open, "TOTAL", "open"), 190_000);

    // Balances follow the A/R postings' names (the customer's payment sits with
    // the customer), so the split differs from aging's but the total agrees.
    let bal = co.run(&as_of(ReportId::CustomerBalanceSummary, day));
    assert_eq!(amt(&bal, "Phase 1", "balance"), 80_000);
    assert_eq!(amt(&bal, "Acme - Other", "balance"), 110_000);
    assert_eq!(amt(&bal, "Total Acme", "balance"), 190_000);
    assert_eq!(amt(&bal, "TOTAL", "balance"), 190_000);

    let mut all = ReportRequest::new(ReportId::CustomerBalanceDetail).today(d(2026, 10, 19));
    all.date_macro = Some(DateMacro::All);
    let det = co.run(&all);
    // 1,000 − 100 + 500 − 300 − 1,000 = 100 for Acme itself; 800 for the job.
    assert_eq!(amt(&det, "Total Acme", "balance"), 10_000);
    assert_eq!(amt(&det, "Total Acme:Phase 1", "balance"), 80_000);
    assert_eq!(amt(&det, "TOTAL", "balance"), 90_000);
}

#[test]
fn ap_aging_and_unpaid_bills() {
    let co = company(1);
    let bill = co.bill(
        d(2026, 2, 1),
        d(2026, 3, 3),
        co.landlord,
        &[(co.rent, 70_000)],
    );
    co.bill_payment(d(2026, 4, 20), co.landlord, "300", &[(bill.id, 70_000)]);
    let aging = co.run(&as_of(ReportId::ApAgingSummary, d(2026, 4, 5)));
    // April 5 is 33 days after March 3.
    assert_eq!(amt(&aging, "Landlord", "b2"), 70_000);
    assert_eq!(amt(&aging, "TOTAL", "total"), 70_000);
    let unpaid = co.run(&as_of(ReportId::UnpaidBillsDetail, d(2026, 4, 5)));
    assert_eq!(amt(&unpaid, "Total Landlord", "open"), 70_000);
    let paid = co.run(&as_of(ReportId::ApAgingSummary, d(2026, 4, 25)));
    assert_eq!(amt(&paid, "TOTAL", "total"), 0);
    let bal = co.run(&as_of(ReportId::VendorBalanceSummary, d(2026, 4, 5)));
    assert_eq!(amt(&bal, "Landlord", "balance"), 70_000);
}

/// Jobs: Phase 1 ($1,000, plus $300 of supplies bought for it), its sub-job
/// Design ($100), Phase 2 ($500), Acme itself ($200), and $50 of rent for no job.
fn jobs() -> Co {
    let co = company(1);
    co.invoice(d(2026, 6, 1), co.phase1, &[(co.labor, 100_000)]);
    co.invoice(d(2026, 6, 2), co.design, &[(co.labor, 10_000)]);
    co.invoice(d(2026, 6, 3), co.phase2, &[(co.parts, 50_000)]);
    co.invoice(d(2026, 6, 4), co.acme, &[(co.labor, 20_000)]);
    let mut chk = Txn::new(TxnType::Check, d(2026, 6, 5));
    chk.account_id = Some(co.bank);
    chk.entity_id = Some(co.landlord);
    chk.lines
        .push(TxnLine::account(co.supplies, Money(30_000)).with_entity(co.phase1));
    chk.lines.push(TxnLine::account(co.rent, Money(5_000)));
    co.save(chk);
    co
}

#[test]
fn job_profitability_rolls_sub_jobs_up() {
    let co = jobs();
    let r = co.run(&req(
        ReportId::JobProfitabilitySummary,
        d(2026, 6, 1),
        d(2026, 6, 30),
    ));
    assert_eq!(amt(&r, "Design", "revenue"), 10_000);
    assert_eq!(amt(&r, "Phase 1 - Other", "cost"), 30_000);
    assert_eq!(amt(&r, "Phase 1 - Other", "revenue"), 100_000);
    assert_eq!(amt(&r, "Total Phase 1", "cost"), 30_000);
    assert_eq!(amt(&r, "Total Phase 1", "revenue"), 110_000);
    assert_eq!(amt(&r, "Total Phase 1", "diff"), 80_000);
    assert_eq!(amt(&r, "Phase 2", "revenue"), 50_000);
    assert_eq!(amt(&r, "Acme - Other", "revenue"), 20_000);
    assert_eq!(amt(&r, "Total Acme", "diff"), 150_000);
    assert_eq!(
        amt(&r, "TOTAL", "cost"),
        30_000,
        "rent for no job is left out"
    );

    let detail = co.run(&req(
        ReportId::JobProfitabilityDetail,
        d(2026, 6, 1),
        d(2026, 6, 30),
    ));
    assert_eq!(amt(&detail, "Total Acme:Phase 1", "revenue"), 100_000);
    assert_eq!(amt(&detail, "Total Acme:Phase 1", "cost"), 30_000);
    assert_eq!(amt(&detail, "Total Acme:Phase 1:Design", "revenue"), 10_000);
}

#[test]
fn profit_and_loss_by_job_has_a_column_per_job_and_totals() {
    let co = jobs();
    let r = co.run(&req(
        ReportId::ProfitAndLossByJob,
        d(2026, 6, 1),
        d(2026, 6, 30),
    ));
    let labels: Vec<&str> = r.columns.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        vec![
            "",
            "Design",
            "Phase 1 - Other",
            "Total Phase 1",
            "Phase 2",
            "Acme - Other",
            "Total Acme",
            "No Customer:Job",
            "TOTAL"
        ]
    );
    assert_eq!(amt(&r, "Net Income", "c2"), 80_000);
    assert_eq!(amt(&r, "Net Income", "c5"), 150_000);
    assert_eq!(amt(&r, "Net Income", "c6"), -5_000);
    assert_eq!(amt(&r, "Net Income", "c7"), 145_000);
    // A job column's QuickZoom names the job.
    let cell = &r.row("Consulting Income").unwrap().cells[1];
    let Some(Drill::Report { request }) = &cell.drill else {
        panic!("{cell:?}")
    };
    assert_eq!(request.filters.names, vec![co.design]);
}

#[test]
fn profit_and_loss_by_class() {
    let co = company(1);
    let class =
        co.b.write(&Actor::system(), |w| {
            lb_store::lists::save(
                w,
                lb_core::lists::Class {
                    id: ClassId::new(),
                    name: "East".into(),
                    full_name: "East".into(),
                    parent_id: None,
                    sublevel: 0,
                    is_active: true,
                    edit_seq: 0,
                },
            )
        })
        .unwrap()
        .id;
    let mut inv = Txn::new(TxnType::Invoice, d(2026, 7, 1));
    inv.entity_id = Some(co.globex);
    inv.class_id = Some(class);
    inv.lines
        .push(TxnLine::item_amount(co.labor, Money(40_000)));
    co.save(inv);
    co.invoice(d(2026, 7, 2), co.globex, &[(co.labor, 10_000)]);
    let r = co.run(&req(
        ReportId::ProfitAndLossByClass,
        d(2026, 7, 1),
        d(2026, 7, 31),
    ));
    let labels: Vec<&str> = r.columns.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, vec!["", "East", "Unclassified", "TOTAL"]);
    assert_eq!(amt(&r, "Consulting Income", "c0"), 40_000);
    assert_eq!(amt(&r, "Consulting Income", "c1"), 10_000);
    assert_eq!(amt(&r, "Consulting Income", "c2"), 50_000);
}

fn qty_line(item: lb_core::ItemId, qty: i64, rate: i64) -> TxnLine {
    TxnLine::item(item, Decimal::from(qty), Decimal::from(rate))
}

#[test]
fn sales_by_customer_and_item_with_quantities() {
    let co = company(1);
    let sale = |date, customer, lines: Vec<TxnLine>, t: TxnType| {
        let mut x = Txn::new(t, date);
        x.entity_id = Some(customer);
        x.lines = lines;
        co.save(x)
    };
    // Globex: 10 labor at $100 and 2 parts at $250; a credit for 1 labor.
    sale(
        d(2026, 8, 1),
        co.globex,
        vec![qty_line(co.labor, 10, 100), qty_line(co.parts, 2, 250)],
        TxnType::Invoice,
    );
    sale(
        d(2026, 8, 2),
        co.acme,
        vec![qty_line(co.labor, 5, 100)],
        TxnType::Invoice,
    );
    sale(
        d(2026, 8, 3),
        co.globex,
        vec![qty_line(co.labor, 1, 100)],
        TxnType::CreditMemo,
    );
    let range = (d(2026, 8, 1), d(2026, 8, 31));

    let by_cust = co.run(&req(ReportId::SalesByCustomerSummary, range.0, range.1));
    assert_eq!(amt(&by_cust, "Acme", "c0"), 50_000);
    assert_eq!(amt(&by_cust, "Globex", "c0"), 140_000);
    assert_eq!(amt(&by_cust, "TOTAL", "c0"), 190_000);

    let by_item = co.run(&req(ReportId::SalesByItemSummary, range.0, range.1));
    let labor = by_item.row("Labor").unwrap();
    let col = |id: &str| by_item.column_index(id).unwrap();
    assert_eq!(labor.cells[col("qty")].value, Value::Number("14".into()));
    assert_eq!(labor.cells[col("amount")].as_money(), Some(Money(140_000)));
    assert_eq!(labor.cells[col("pct")].value, Value::Percent("73.7".into()));
    assert_eq!(
        labor.cells[col("avg_price")].value,
        Value::Number("100.00".into())
    );
    assert_eq!(amt(&by_item, "Total Service", "amount"), 190_000);

    let detail = co.run(&req(ReportId::SalesByCustomerDetail, range.0, range.1));
    assert_eq!(amt(&detail, "Total Globex", "amount"), 140_000);
    let qty_col = detail.column_index("qty").unwrap();
    assert!(
        detail
            .rows
            .iter()
            .any(|r| r.cells[qty_col].value == Value::Number("-1".into())),
        "the credit memo's quantity counts against the sale"
    );
    let by_item_detail = co.run(&req(ReportId::SalesByItemDetail, range.0, range.1));
    assert_eq!(amt(&by_item_detail, "Total Labor", "amount"), 140_000);
}

#[test]
fn expenses_by_vendor_and_the_1099_summary() {
    let co = company(1);
    co.check(
        d(2025, 3, 1),
        co.landlord,
        "1",
        &[(co.rent, 40_000), (co.supplies, 5_000)],
    );
    let bill = co.bill(
        d(2025, 4, 1),
        d(2025, 5, 1),
        co.contractor,
        &[(co.supplies, 100_000)],
    );
    co.bill_payment(d(2025, 5, 1), co.contractor, "2", &[(bill.id, 60_000)]);
    let year = (d(2025, 1, 1), d(2025, 12, 31));

    let ev = co.run(&req(ReportId::ExpensesByVendorSummary, year.0, year.1).basis(Basis::Accrual));
    assert_eq!(amt(&ev, "Contractor", "c0"), 100_000);
    assert_eq!(amt(&ev, "Landlord", "c0"), 45_000);
    assert_eq!(amt(&ev, "TOTAL", "c0"), 145_000);
    let evd = co.run(&req(ReportId::ExpensesByVendorDetail, year.0, year.1));
    assert_eq!(amt(&evd, "Total Landlord", "amount"), 45_000);

    // Only what was paid to the 1099 vendor, by the account it paid for.
    let t = co.run(&req(ReportId::Vendor1099Summary, year.0, year.1));
    assert_eq!(t.basis, Some(Basis::Cash));
    assert_eq!(t.columns[1].label, "Supplies");
    assert_eq!(amt(&t, "Contractor", "a0"), 60_000);
    assert_eq!(amt(&t, "TOTAL", "total"), 60_000);
    assert!(t.row("Landlord").is_none());
}

#[test]
fn inventory_cost_counts_when_sold_even_on_cash_basis() {
    let co = company(1);
    let (asset, cogs, sales) =
        co.b.write(&Actor::system(), |w| {
            let asset = accounts::ensure_role(w, AccountRole::InventoryAsset)?;
            let cogs = accounts::ensure_role(w, AccountRole::CostOfGoodsSold)?;
            let sales = accounts::save(
                w,
                accounts::new_account("Product Sales", AccountType::Income),
            )?
            .id;
            Ok((asset, cogs, sales))
        })
        .unwrap();
    let widget =
        co.b.write(&Actor::system(), |w| {
            let mut i = Item::blank(ItemType::Inventory, "Widget");
            i.income_account_id = Some(sales);
            i.expense_account_id = Some(cogs);
            i.asset_account_id = Some(asset);
            i.sales_price = Some(Decimal::from(12));
            Ok(items::save(w, i)?.id)
        })
        .unwrap();
    // Buy 10 at $5 on a bill; sell 4 at $12 on an invoice; neither paid.
    let mut bill = Txn::new(TxnType::Bill, d(2026, 9, 1));
    bill.entity_id = Some(co.landlord);
    bill.lines.push(qty_line(widget, 10, 5));
    co.save(bill);
    let mut inv = Txn::new(TxnType::Invoice, d(2026, 9, 2));
    inv.entity_id = Some(co.globex);
    inv.lines.push(qty_line(widget, 4, 12));
    co.save(inv);

    let val = co.run(&as_of(ReportId::InventoryValuationSummary, d(2026, 9, 30)));
    let row = val.row("Widget").unwrap();
    let col = |id: &str| val.column_index(id).unwrap();
    assert_eq!(row.cells[col("on_hand")].value, Value::Number("6".into()));
    assert_eq!(
        row.cells[col("avg_cost")].value,
        Value::Number("5.00".into())
    );
    assert_eq!(amt(&val, "Widget", "value"), 3_000);
    assert_eq!(amt(&val, "Widget", "retail"), 7_200);

    // Cash basis: no sale yet, but the $20 cost of the four sold counts now (QBD
    // does the same); the inventory bought on an unpaid bill leaves A/P on the
    // cash-basis balance sheet, as QBD's does.
    let range = (d(2026, 9, 1), d(2026, 9, 30));
    let pl = co.run(&req(ReportId::ProfitAndLoss, range.0, range.1).basis(Basis::Cash));
    assert!(pl.row("Product Sales").is_none());
    assert_eq!(amt(&pl, "Cost of Goods Sold", "c0"), 2_000);
    assert_eq!(amt(&pl, "Net Income", "c0"), -2_000);
    let bs = co.run(&req(ReportId::BalanceSheet, range.0, range.1).basis(Basis::Cash));
    assert_eq!(amt(&bs, "Inventory Asset", "c0"), 3_000);
    assert_eq!(amt(&bs, "Accounts Payable", "c0"), 5_000);
    assert!(bs.row("Accounts Receivable").is_none());
    same_amounts(&bs, "TOTAL ASSETS", "TOTAL LIABILITIES & EQUITY");
    let sbi = co.run(&req(ReportId::SalesByItemSummary, range.0, range.1).basis(Basis::Accrual));
    assert_eq!(amt(&sbi, "Widget", "cogs"), 2_000);
    assert_eq!(amt(&sbi, "Widget", "margin"), 2_800);
}

#[test]
fn banking_reports() {
    let co = company(1);
    for n in ["101", "102", "104", "104"] {
        co.check(d(2026, 2, 1), co.landlord, n, &[(co.rent, 1_000)]);
    }
    let mut pmt = Txn::new(TxnType::ReceivePayment, d(2026, 2, 2));
    pmt.entity_id = Some(co.globex);
    pmt.amount = Money(25_000);
    let pmt = co.save(pmt);
    let uf =
        co.b.read(|c| accounts::by_role(c, AccountRole::UndepositedFunds))
            .unwrap()
            .unwrap()
            .id;
    let mut dep = Txn::new(TxnType::Deposit, d(2026, 2, 3));
    dep.account_id = Some(co.bank);
    let mut line = TxnLine::deposit(uf, Money(25_000)).with_entity(co.globex);
    line.linked_txn_id = Some(pmt.id);
    dep.lines.push(line);
    co.save(dep);
    let range = (d(2026, 2, 1), d(2026, 2, 28));

    let missing = co.run(&req(ReportId::MissingChecks, range.0, range.1));
    let notes: Vec<&str> = missing
        .rows
        .iter()
        .filter_map(|r| r.cells.get(1).and_then(|c| c.as_text()))
        .filter(|t| t.starts_with("***"))
        .collect();
    assert_eq!(
        notes,
        vec![
            "*** Missing numbers here ***",
            "*** Duplicate document numbers ***"
        ]
    );

    let deposits = co.run(&req(ReportId::DepositDetail, range.0, range.1));
    assert_eq!(amt(&deposits, "TOTAL", "amount"), 25_000);
    let type_col = deposits.column_index("type").unwrap();
    assert!(
        deposits
            .rows
            .iter()
            .any(|r| r.depth == 1 && r.cells[type_col].as_text() == Some("Payment")),
        "the deposited payment is listed under the deposit"
    );

    let checks = co.run(&req(ReportId::CheckDetail, range.0, range.1));
    assert_eq!(amt(&checks, "TOTAL", "paid"), -4_000);
}

#[test]
fn sales_tax_liability_by_agency_and_item() {
    let co = company(1);
    let (tax_item, taxable, agency) = co.sales_tax();
    let mut inv = Txn::new(TxnType::Invoice, d(2026, 3, 1));
    inv.entity_id = Some(co.globex);
    inv.sales_tax_item_id = Some(tax_item);
    let mut l = TxnLine::item_amount(co.labor, Money(100_000));
    l.sales_tax_code_id = Some(taxable);
    inv.lines.push(l);
    inv.lines
        .push(TxnLine::item_amount(co.parts, Money(20_000)));
    co.save(inv);
    let mut pay = Txn::new(TxnType::SalesTaxPaymentCheck, d(2026, 3, 20));
    pay.account_id = Some(co.bank);
    pay.entity_id = Some(agency);
    pay.amount = Money(6_000);
    co.save(pay);

    let r = co.run(&req(
        ReportId::SalesTaxLiability,
        d(2026, 3, 1),
        d(2026, 3, 31),
    ));
    let row = r.row("State 10%").unwrap();
    let col = |id: &str| r.column_index(id).unwrap();
    assert_eq!(
        row.cells[col("total_sales")].as_money(),
        Some(Money(120_000))
    );
    assert_eq!(
        row.cells[col("non_taxable")].as_money(),
        Some(Money(20_000))
    );
    assert_eq!(row.cells[col("taxable")].as_money(), Some(Money(100_000)));
    assert_eq!(row.cells[col("rate")].value, Value::Percent("10".into()));
    assert_eq!(row.cells[col("collected")].as_money(), Some(Money(10_000)));
    // Owed: 100 collected less the 60 paid.
    assert_eq!(amt(&r, "Total State Tax", "payable"), 4_000);
}

#[test]
fn csv_export_quotes_indents_and_defuses() {
    let co = company_named("Test, Co", 1);
    co.invoice(d(2026, 1, 15), co.globex, &[(co.labor, 100_000)]);
    let pl = co.run(&req(ReportId::ProfitAndLoss, d(2026, 1, 1), d(2026, 1, 31)));
    let csv = lb_reports::to_csv(&pl, CsvOptions::default());
    let expected = [
        "Profit & Loss",
        "\"Test, Co\"",
        "January 2026",
        "Accrual Basis",
        "",
        ",Jan 26",
        "Ordinary Income/Expense,",
        "    Income,",
        "        Consulting Income,1000.00",
        "    Total Income,1000.00",
        "Net Ordinary Income,1000.00",
        "Net Income,1000.00",
        "",
    ]
    .join("\r\n");
    assert_eq!(csv, expected);
    assert_eq!(
        lb_reports::file_name(&pl, ExportFormat::Csv),
        "Profit_and_Loss.csv"
    );
    assert_eq!(
        lb_reports::export(&pl, ExportFormat::Csv),
        csv.clone().into_bytes()
    );

    // A memo a spreadsheet would run as a formula is defused; dates are M/D/Y.
    let mut chk = Txn::new(TxnType::Check, d(2026, 1, 20));
    chk.account_id = Some(co.bank);
    chk.memo = Some("=HYPERLINK(\"x\")".into());
    chk.lines.push(TxnLine::account(co.rent, Money(1_000)));
    co.save(chk);
    let list = co.run(&req(
        ReportId::TransactionListByDate,
        d(2026, 1, 20),
        d(2026, 1, 20),
    ));
    let csv = lb_reports::to_csv(
        &list,
        CsvOptions {
            header_block: false,
        },
    );
    assert!(csv.starts_with(",Type,Date,Num,Name,Memo,Account,Clr,Split,Amount\r\n"));
    assert!(
        csv.contains(",Check,01/20/2026,,,\"'=HYPERLINK(\"\"x\"\")\",Checking,,Rent,-10.00\r\n"),
        "{csv}"
    );
}

#[test]
fn filters_narrow_detail_reports() {
    let co = company(1);
    let mut a = Txn::new(TxnType::Check, d(2026, 5, 1));
    a.account_id = Some(co.bank);
    a.memo = Some("printer paper".into());
    a.lines.push(TxnLine::account(co.supplies, Money(2_500)));
    co.save(a);
    co.check(d(2026, 5, 2), co.landlord, "7", &[(co.rent, 90_000)]);
    let mut r = req(
        ReportId::TransactionDetailByAccount,
        d(2026, 5, 1),
        d(2026, 5, 31),
    );
    r.filters.memo = Some("PAPER".into());
    let rep = co.run(&r);
    assert_eq!(amt(&rep, "Total Supplies", "amount"), 2_500);
    assert!(rep.row("Rent").is_none());
    let mut r = req(
        ReportId::TransactionDetailByAccount,
        d(2026, 5, 1),
        d(2026, 5, 31),
    );
    r.filters.amount_min = Some(Money(10_000));
    r.filters.names = vec![co.landlord];
    let rep = co.run(&r);
    assert_eq!(amt(&rep, "Total Rent", "amount"), 90_000);
    assert!(rep.row("Supplies").is_none());
    // A QuickReport needs an account.
    let err = lb_reports::run(
        &co.b,
        &req(ReportId::AccountQuickReport, d(2026, 5, 1), d(2026, 5, 31)),
    );
    assert!(matches!(err, Err(lb_reports::ReportError::Invalid(_))));
}

/// Every transaction type the reports care about, in one company.
fn everything() -> Co {
    let co = jobs();
    let (tax_item, taxable, _) = co.sales_tax();
    let mut inv = Txn::new(TxnType::Invoice, d(2026, 6, 10));
    inv.entity_id = Some(co.globex);
    inv.sales_tax_item_id = Some(tax_item);
    let mut l = TxnLine::item_amount(co.labor, Money(10_000));
    l.sales_tax_code_id = Some(taxable);
    inv.lines.push(l);
    inv.lines
        .push(TxnLine::item_amount(co.discount, Money(-1_000)));
    let inv = co.save(inv);
    let cm = co.credit_memo(d(2026, 6, 11), co.globex, 500);
    let mut pmt = Txn::new(TxnType::ReceivePayment, d(2026, 6, 12));
    pmt.entity_id = Some(co.globex);
    pmt.account_id = Some(co.bank);
    pmt.amount = Money(5_000);
    let mut app = Application::cash(inv.id, Money(5_000));
    app.credit_txn_id = Some(cm.id);
    app.credit_amount = Money(500);
    app.discount = Money(100);
    app.discount_account_id = Some(co.discounts);
    pmt.applications.push(app);
    co.save(pmt);
    let bill = co.bill(
        d(2026, 6, 13),
        d(2026, 7, 13),
        co.contractor,
        &[(co.supplies, 9_000)],
    );
    co.bill_payment(d(2026, 6, 14), co.contractor, "500", &[(bill.id, 4_000)]);
    co.journal(d(2026, 6, 15), &[(co.truck, 50_000), (co.loan, -50_000)]);
    let mut je = Txn::new(TxnType::JournalEntry, d(2026, 6, 16));
    je.lines
        .push(TxnLine::journal(co.ar, Money(700)).with_entity(co.globex));
    je.lines.push(TxnLine::journal(co.consulting, Money(-700)));
    co.save(je);
    let mut text = Txn::new(TxnType::Check, d(2026, 6, 17));
    text.account_id = Some(co.bank);
    text.lines.push(TxnLine::account(co.internet, Money(1_234)));
    let mut note = TxnLine::new(LineKind::Text);
    note.description = Some("a note".into());
    text.lines.push(note);
    co.save(text);
    co
}

fn check_shape(r: &Report) {
    for row in &r.rows {
        assert_eq!(
            row.cells.len(),
            r.columns.len(),
            "{}: row {:?} has {} cells for {} columns",
            r.title,
            row.label(),
            row.cells.len(),
            r.columns.len()
        );
    }
    // The shape survives JSON (what the API sends).
    let json = serde_json::to_string(r).unwrap();
    let back: Report = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, r);
}

#[test]
fn every_report_runs_on_both_bases_with_a_consistent_shape() {
    let co = everything();
    for id in ReportId::ALL {
        for basis in [Basis::Accrual, Basis::Cash] {
            let mut r = ReportRequest::new(id).today(d(2026, 6, 30)).basis(basis);
            r.date_macro = Some(DateMacro::ThisFiscalYearToDate);
            if id == ReportId::AccountQuickReport {
                r.filters.accounts = vec![co.bank];
            }
            let rep = co.run(&r);
            check_shape(&rep);
            assert!(!rep.columns.is_empty(), "{id:?}");
            let _ = lb_reports::to_csv(&rep, CsvOptions::default());
        }
    }
    // Balance and ledger reports tie out on both bases with everything in.
    for basis in [Basis::Accrual, Basis::Cash] {
        let bs = co.run(&req(ReportId::BalanceSheet, d(2026, 1, 1), d(2026, 6, 30)).basis(basis));
        same_amounts(&bs, "TOTAL ASSETS", "TOTAL LIABILITIES & EQUITY");
        let tb = co.run(&req(ReportId::TrialBalance, d(2026, 1, 1), d(2026, 6, 30)).basis(basis));
        assert_eq!(
            amt(&tb, "TOTAL", "debit"),
            amt(&tb, "TOTAL", "credit"),
            "{basis:?}"
        );
        let bsd =
            co.run(&req(ReportId::BalanceSheetDetail, d(2026, 1, 1), d(2026, 6, 30)).basis(basis));
        assert_eq!(
            amt(&bsd, "TOTAL ASSETS", "balance"),
            amt(&bs, "TOTAL ASSETS", "c0"),
            "{basis:?}"
        );
        assert_eq!(
            amt(&bsd, "TOTAL LIABILITIES & EQUITY", "balance"),
            amt(&bs, "TOTAL LIABILITIES & EQUITY", "c0"),
            "{basis:?}"
        );
        let pl = co.run(&req(ReportId::ProfitAndLoss, d(2026, 1, 1), d(2026, 6, 30)).basis(basis));
        let pld =
            co.run(&req(ReportId::ProfitAndLossDetail, d(2026, 1, 1), d(2026, 6, 30)).basis(basis));
        assert_eq!(
            amt(&pl, "Net Income", "c0"),
            amt(&pld, "Net Income", "amount"),
            "{basis:?}"
        );
        let scf = co.run(
            &req(
                ReportId::StatementOfCashFlows,
                d(2026, 1, 1),
                d(2026, 6, 30),
            )
            .basis(basis),
        );
        assert!(scf.notes.is_empty(), "{basis:?}: {:?}", scf.notes);
        assert_eq!(
            amt(&scf, "Cash at end of period", "amount"),
            amt(&bs, "Checking", "c0") + amt(&bs, "Undeposited Funds", "c0"),
            "{basis:?}"
        );
    }
    // The audit trail saw every save.
    let mut audit = ReportRequest::new(ReportId::AuditTrail);
    audit.date_macro = Some(DateMacro::All);
    let a = co.run(&audit);
    assert!(
        a.rows
            .iter()
            .any(|r| r.kind == RowKind::Header && r.label() == Some("Invoice"))
    );
    assert!(
        a.rows
            .iter()
            .any(|r| r.label() == Some("Check 500") || r.label() == Some("Bill Pmt -Check 500"))
    );
}

#[test]
fn list_reports() {
    let co = jobs();
    let mut r = ReportRequest::new(ReportId::AccountListing).today(d(2026, 6, 30));
    r.date_macro = Some(DateMacro::All);
    let accounts = co.run(&r);
    let col = accounts.column_index("balance").unwrap();
    let checking = accounts.row("Checking").unwrap();
    assert_eq!(checking.cells[col].as_money(), Some(Money(-35_000)));
    assert_eq!(accounts.row("Rent").unwrap().cells[col].value, Value::Empty);
    let vendors = co.run(&ReportRequest::new(ReportId::VendorContactList));
    assert!(vendors.row("Contractor").is_some());
    let one099 = vendors.column_index("is_1099").unwrap();
    assert_eq!(
        vendors.row("Contractor").unwrap().cells[one099].as_text(),
        Some("Yes")
    );
    let customers = co.run(&ReportRequest::new(ReportId::CustomerContactList));
    assert!(customers.row("Acme:Phase 1:Design").is_some());
    let items = co.run(&ReportRequest::new(ReportId::ItemListing));
    assert!(items.row("Labor").is_some());
}
