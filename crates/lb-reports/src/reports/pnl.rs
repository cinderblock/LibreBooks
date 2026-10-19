//! Profit & Loss: Standard, by period, by job, by class, YTD comparison, previous
//! year comparison. One engine; the variants only choose columns.
//!
//! QBD's layout: Ordinary Income/Expense (Income, Cost of Goods Sold → Gross
//! Profit, Expense → Net Ordinary Income), Other Income/Expense (→ Net Other
//! Income), Net Income. Income and expenses both show positive; net income is
//! income − COGS − expense + other income − other expense.

use std::collections::{HashMap, HashSet};

use lb_core::{AccountId, AccountType, DateRange, Money};

use super::{PL_SECTIONS, accounts_of};
use crate::dates::{fiscal_year_start, short_range};
use crate::model::{Drill, RowKind};
use crate::request::{ReportFilters, ReportId};
use crate::summary::{BaseCol, DimSel, Line, Plan, PlanOpts, account_tree, base_columns, sub};
use crate::{Result, Run, model::Report};

/// Narrow drill-down filters to a column's dimension.
pub(crate) fn apply_dim(f: &mut ReportFilters, dim: DimSel) {
    match dim {
        DimSel::Job(Some(e)) | DimSel::Vendor(Some(e)) => f.names = vec![e],
        DimSel::Class(Some(c)) => f.classes = vec![c],
        DimSel::Item(Some(i)) => f.items = vec![i],
        _ => {}
    }
}

pub(crate) fn summary(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let ledger = r.ledger();
    let id = r.req.report;
    let pl_types: Vec<AccountType> = PL_SECTIONS.iter().map(|(_, t)| *t).collect();
    let pl_accounts = accounts_of(ctx, &pl_types);
    let pl_set: HashSet<AccountId> = pl_accounts.iter().copied().collect();
    let mut opts = PlanOpts::from_request(r.req, "% of Income", r.req.percent_of_income);
    let mut notes = Vec::new();
    let bases = if id == ReportId::ProfitAndLossYtdComparison {
        // The period beside its fiscal year to date.
        let to = r.range.to.unwrap_or(ctx.today);
        let ytd = DateRange::new(fiscal_year_start(to, ctx.fiscal_start()), to);
        opts.previous_period = false;
        opts.previous_year = false;
        vec![
            BaseCol::total(short_range(r.range), r.range),
            BaseCol::total(short_range(ytd), ytd),
        ]
    } else {
        if id == ReportId::ProfitAndLossPrevYearComparison {
            opts.previous_year = true;
        }
        base_columns(&ledger, r.range, r.layout(), &pl_set)?
    };
    if bases.len() > 60 {
        notes.push(format!(
            "{} columns; consider a longer period per column.",
            bases.len()
        ));
    }
    let mut plan = Plan::new(&bases, opts);
    let values = plan.compute(&ledger)?;
    let width = plan.values.len();

    let own = |a: AccountId| -> Vec<Money> {
        let t = ctx.account_type(a).unwrap_or(AccountType::Expense);
        values
            .iter()
            .map(|m| t.natural(m.get(&a).copied().unwrap_or_default()))
            .collect()
    };
    let col_info: Vec<(DateRange, DimSel)> = plan.values.iter().map(|v| (v.range, v.dim)).collect();
    let drill = |a: AccountId, exact: bool, col: Option<usize>| -> Option<Drill> {
        let (range, dim) = col.map_or((r.range, DimSel::Any), |v| col_info[v]);
        r.drill_to(ReportId::TransactionDetailByAccount, range, |f| {
            f.accounts = vec![a];
            f.exact = exact;
            apply_dim(f, dim);
        })
    };

    let mut sections: HashMap<AccountType, (Vec<Line>, Vec<Money>)> = HashMap::new();
    for (_, t) in PL_SECTIONS {
        let members: Vec<AccountId> = accounts_of(ctx, &[t]);
        sections.insert(
            t,
            account_tree(ctx, &members, &own, width, r.req.show_zero_rows, 2, &drill),
        );
    }
    let zero = vec![Money::ZERO; width];
    let total_of = |t: AccountType| sections.get(&t).map_or(zero.clone(), |s| s.1.clone());
    let present = |t: AccountType| sections.get(&t).is_some_and(|s| !s.0.is_empty());
    let income = total_of(AccountType::Income);
    let cogs = total_of(AccountType::CostOfGoodsSold);
    let expense = total_of(AccountType::Expense);
    let other_income = total_of(AccountType::OtherIncome);
    let other_expense = total_of(AccountType::OtherExpense);
    let gross = sub(&income, &cogs);
    let net_ordinary = sub(&gross, &expense);
    let net_other = sub(&other_income, &other_expense);
    let net: Vec<Money> = net_ordinary
        .iter()
        .zip(&net_other)
        .map(|(a, b)| *a + *b)
        .collect();
    plan.bases.clone_from(&income);
    let ordinary = present(AccountType::Income)
        || present(AccountType::CostOfGoodsSold)
        || present(AccountType::Expense);
    let other = present(AccountType::OtherIncome) || present(AccountType::OtherExpense);
    let has_cogs = present(AccountType::CostOfGoodsSold);

    let mut lines = Vec::new();
    let mut section = |lines: &mut Vec<Line>, label: &str, total_label: &str, t: AccountType| {
        if let Some((l, tot)) = sections.remove(&t)
            && !l.is_empty()
        {
            lines.push(Line::header(1, label));
            lines.extend(l);
            lines.push(Line::amounts(RowKind::Subtotal, 1, total_label, tot));
        }
    };
    if ordinary {
        lines.push(Line::header(0, "Ordinary Income/Expense"));
        section(&mut lines, "Income", "Total Income", AccountType::Income);
        section(
            &mut lines,
            "Cost of Goods Sold",
            "Total COGS",
            AccountType::CostOfGoodsSold,
        );
        if has_cogs {
            lines.push(Line::amounts(RowKind::Subtotal, 1, "Gross Profit", gross));
        }
        section(&mut lines, "Expense", "Total Expense", AccountType::Expense);
        lines.push(Line::amounts(
            RowKind::Subtotal,
            0,
            "Net Ordinary Income",
            net_ordinary,
        ));
    }
    if other {
        lines.push(Line::header(0, "Other Income/Expense"));
        section(
            &mut lines,
            "Other Income",
            "Total Other Income",
            AccountType::OtherIncome,
        );
        section(
            &mut lines,
            "Other Expense",
            "Total Other Expense",
            AccountType::OtherExpense,
        );
        lines.push(Line::amounts(
            RowKind::Subtotal,
            0,
            "Net Other Income",
            net_other,
        ));
    }
    let mut net_line = Line::amounts(RowKind::Total, 0, "Net Income", net);
    net_line.drill = r.drill_to(ReportId::ProfitAndLossDetail, r.range, |_| {});
    lines.push(net_line);

    let columns = plan.columns();
    let rows = plan.rows(lines);
    Ok(r.finish(r.range_line(), columns, rows, notes))
}
