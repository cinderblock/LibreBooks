//! Balance Sheet: Standard, Summary, by period, and Previous Year comparison.
//!
//! Each column is a balance *as of* a date E. Income-statement accounts are not
//! listed; their effect shows in equity the way QBD shows it:
//!
//! - **Net Income**: income less expenses from the start of E's fiscal year
//!   through E (the company's fiscal-year start month).
//! - **Retained Earnings**: the Retained Earnings account's own postings plus all
//!   income less expenses before that fiscal year (QBD closes each year into it
//!   virtually; nothing is ever posted to close a year).
//!
//! Assets therefore always equal liabilities plus equity: every transaction's
//! postings sum to zero, and every posting dated on or before E lands in exactly
//! one line.

use std::collections::HashMap;

use lb_core::{AccountId, AccountRole, AccountType, DateRange, Money};

use super::accounts_of;
use crate::dates::{buckets, fiscal_year_start, short_date};
use crate::model::{Drill, Report, RowKind};
use crate::request::{ColumnLayout, ReportId};
use crate::summary::{BaseCol, Line, Plan, PlanOpts, Slot, account_tree, add, compute_slots};
use crate::{Result, Run};

/// The balance sheet's groups: (heading, types). Assets, then liabilities.
const CURRENT_ASSETS: [(&str, AccountType); 3] = [
    ("Checking/Savings", AccountType::Bank),
    ("Accounts Receivable", AccountType::AccountsReceivable),
    ("Other Current Assets", AccountType::OtherCurrentAsset),
];
const CURRENT_LIABILITIES: [(&str, AccountType); 3] = [
    ("Accounts Payable", AccountType::AccountsPayable),
    ("Credit Cards", AccountType::CreditCard),
    (
        "Other Current Liabilities",
        AccountType::OtherCurrentLiability,
    ),
];

pub(crate) fn balance_sheet(r: &Run<'_>, summary_only: bool) -> Result<Report> {
    let ctx = r.ctx;
    let fs = ctx.fiscal_start();
    let ledger = r.ledger();
    let mut notes = Vec::new();
    let as_of = r.as_of().unwrap_or(ctx.today);
    let mut layout = r.layout();
    if !(layout == ColumnLayout::TotalOnly || layout.is_period()) {
        notes.push("The balance sheet's columns can only be periods; showing one column.".into());
        layout = ColumnLayout::TotalOnly;
    }
    let bases: Vec<BaseCol> = if layout == ColumnLayout::TotalOnly {
        vec![BaseCol::total(
            short_date(as_of),
            DateRange {
                from: None,
                to: Some(as_of),
            },
        )]
    } else {
        let from = r
            .range
            .from
            .or(ledger.first_date()?)
            .unwrap_or(as_of)
            .min(as_of);
        buckets(from, as_of, layout, fs)
            .into_iter()
            .map(|b| {
                let end = b.to.unwrap_or(as_of);
                BaseCol::total(
                    short_date(end),
                    DateRange {
                        from: None,
                        to: Some(end),
                    },
                )
            })
            .collect()
    };
    let mut opts = PlanOpts::from_request(r.req, "% of Column", r.req.percent_of_column);
    if r.req.report == ReportId::BalanceSheetPrevYearComparison {
        opts.previous_year = true;
    }
    let mut plan = Plan::new(&bases, opts);
    let balances = plan.compute(&ledger)?;
    let width = plan.values.len();

    // Income less expenses for each column's fiscal year to date.
    let ends: Vec<jiff::civil::Date> = plan
        .values
        .iter()
        .map(|v| v.range.to.unwrap_or(as_of))
        .collect();
    let ni_slots: Vec<Slot> = plan
        .values
        .iter()
        .zip(&ends)
        .map(|(v, end)| Slot {
            range: DateRange::new(fiscal_year_start(*end, fs), *end),
            dim: crate::summary::DimSel::Any,
            set: 10 + plan.slots[v.slots[0]].set,
        })
        .collect();
    let ni_sums = compute_slots(&ledger, &ni_slots)?;
    let is_pl = |a: &AccountId| {
        ctx.account_type(*a)
            .is_some_and(AccountType::is_income_statement)
    };
    let pl_sum = |m: &HashMap<AccountId, Money>| -> Money {
        m.iter().filter(|(a, _)| is_pl(a)).map(|(_, v)| *v).sum()
    };
    // Debit-positive: prior years' P&L closes into retained earnings.
    let re_adjust: Vec<Money> = balances
        .iter()
        .zip(&ni_sums)
        .map(|(all, cur)| pl_sum(all) - pl_sum(cur))
        .collect();
    let net_income: Vec<Money> = ni_sums.iter().map(|m| -pl_sum(m)).collect();
    let re_account = ctx.role(AccountRole::RetainedEarnings);

    let own = |a: AccountId| -> Vec<Money> {
        let t = ctx.account_type(a).unwrap_or(AccountType::Bank);
        balances
            .iter()
            .enumerate()
            .map(|(v, m)| {
                let mut dp = m.get(&a).copied().unwrap_or_default();
                if Some(a) == re_account {
                    dp += re_adjust[v];
                }
                t.natural(dp)
            })
            .collect()
    };
    let col_ends = ends.clone();
    let drill = |a: AccountId, exact: bool, col: Option<usize>| -> Option<Drill> {
        let end = col.map_or(as_of, |v| col_ends[v]);
        r.drill_to(
            ReportId::TransactionDetailByAccount,
            DateRange {
                from: None,
                to: Some(end),
            },
            |f| {
                f.accounts = vec![a];
                f.exact = exact;
            },
        )
    };
    let show_zero = r.req.show_zero_rows;

    // One group of account types: its lines (heading, accounts, total) and total.
    let group =
        |label: &str, types: &[AccountType], depth: u32| -> Option<(Vec<Line>, Vec<Money>)> {
            let members = accounts_of(ctx, types);
            let (lines, total) =
                account_tree(ctx, &members, &own, width, show_zero, depth + 1, &drill);
            if lines.is_empty() {
                return None;
            }
            if summary_only {
                return Some((
                    vec![Line::amounts(RowKind::Data, depth, label, total.clone())],
                    total,
                ));
            }
            let mut out = vec![Line::header(depth, label)];
            out.extend(lines);
            out.push(Line::amounts(
                RowKind::Subtotal,
                depth,
                format!("Total {label}"),
                total.clone(),
            ));
            Some((out, total))
        };
    // Several groups under one heading ("Current Assets").
    let block = |label: &str,
                 groups: &[(&str, AccountType)],
                 depth: u32|
     -> Option<(Vec<Line>, Vec<Money>)> {
        let mut lines = Vec::new();
        let mut total = vec![Money::ZERO; width];
        for (l, t) in groups {
            if let Some((gl, gt)) = group(l, &[*t], depth + 1) {
                lines.extend(gl);
                add(&mut total, &gt);
            }
        }
        if lines.is_empty() {
            return None;
        }
        let mut out = vec![Line::header(depth, label)];
        out.extend(lines);
        out.push(Line::amounts(
            RowKind::Subtotal,
            depth,
            format!("Total {label}"),
            total.clone(),
        ));
        Some((out, total))
    };

    let mut lines = vec![Line::header(0, "ASSETS")];
    let mut assets = vec![Money::ZERO; width];
    for part in [
        block("Current Assets", &CURRENT_ASSETS, 1),
        group("Fixed Assets", &[AccountType::FixedAsset], 1),
        group("Other Assets", &[AccountType::OtherAsset], 1),
    ]
    .into_iter()
    .flatten()
    {
        lines.extend(part.0);
        add(&mut assets, &part.1);
    }
    lines.push(Line::amounts(
        RowKind::Total,
        0,
        "TOTAL ASSETS",
        assets.clone(),
    ));
    lines.push(Line::header(0, "LIABILITIES & EQUITY"));

    let mut liabilities = vec![Money::ZERO; width];
    let mut liab_lines = Vec::new();
    for part in [
        block("Current Liabilities", &CURRENT_LIABILITIES, 2),
        group(
            "Long Term Liabilities",
            &[AccountType::LongTermLiability],
            2,
        ),
    ]
    .into_iter()
    .flatten()
    {
        liab_lines.extend(part.0);
        add(&mut liabilities, &part.1);
    }
    if !liab_lines.is_empty() {
        lines.push(Line::header(1, "Liabilities"));
        lines.extend(liab_lines);
        lines.push(Line::amounts(
            RowKind::Subtotal,
            1,
            "Total Liabilities",
            liabilities.clone(),
        ));
    }

    // Equity: the equity accounts (retained earnings carrying prior years), then
    // this year's net income.
    let equity_members = accounts_of(ctx, &[AccountType::Equity]);
    let (mut eq_lines, mut equity) =
        account_tree(ctx, &equity_members, &own, width, show_zero, 2, &drill);
    if re_account.is_none() && re_adjust.iter().any(|m| !m.is_zero()) {
        let re: Vec<Money> = re_adjust.iter().map(|m| -*m).collect();
        add(&mut equity, &re);
        eq_lines.push(Line::amounts(RowKind::Data, 2, "Retained Earnings", re));
    }
    let mut ni = Line::amounts(RowKind::Data, 2, "Net Income", net_income.clone());
    ni.drill = r.drill_to(
        ReportId::ProfitAndLoss,
        DateRange::new(fiscal_year_start(as_of, fs), as_of),
        |_| {},
    );
    ni.value_drills = ends
        .iter()
        .map(|e| {
            r.drill_to(
                ReportId::ProfitAndLoss,
                DateRange::new(fiscal_year_start(*e, fs), *e),
                |_| {},
            )
        })
        .collect();
    if net_income.iter().any(|m| !m.is_zero()) || show_zero {
        eq_lines.push(ni);
    }
    add(&mut equity, &net_income);
    if summary_only {
        lines.push(Line::amounts(RowKind::Data, 1, "Equity", equity.clone()));
    } else {
        lines.push(Line::header(1, "Equity"));
        lines.extend(eq_lines);
        lines.push(Line::amounts(
            RowKind::Subtotal,
            1,
            "Total Equity",
            equity.clone(),
        ));
    }
    let mut le = liabilities;
    add(&mut le, &equity);
    lines.push(Line::amounts(
        RowKind::Total,
        0,
        "TOTAL LIABILITIES & EQUITY",
        le,
    ));
    plan.bases = assets;
    let columns = plan.columns();
    let rows = plan.rows(lines);
    Ok(r.finish(crate::dates::as_of_line(Some(as_of)), columns, rows, notes))
}
