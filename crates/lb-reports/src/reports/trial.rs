//! Trial Balance: every account's balance as of a date, in debit and credit
//! columns that total the same.
//!
//! As QBD does it: balance-sheet accounts carry their whole history; income and
//! expense accounts show only the current fiscal year (through the as-of date);
//! Retained Earnings carries every earlier year's net income on top of its own
//! postings. So the columns balance without a closing entry ever being posted.

use lb_core::{AccountRole, AccountType, DateRange, Money};

use crate::dates::{fiscal_year_start, short_date};
use crate::model::{Cell, Column, ColumnKind, Report, Row, RowKind};
use crate::request::ReportId;
use crate::summary::{DimSel, Slot, compute_slots};
use crate::{Result, Run};

pub(crate) fn trial_balance(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let as_of = r.as_of().unwrap_or(ctx.today);
    let fys = fiscal_year_start(as_of, ctx.fiscal_start());
    let ledger = r.ledger();
    let to_date = DateRange {
        from: None,
        to: Some(as_of),
    };
    let this_year = DateRange::new(fys, as_of);
    let sums = compute_slots(
        &ledger,
        &[
            Slot {
                range: to_date,
                dim: DimSel::Any,
                set: 0,
            },
            Slot {
                range: this_year,
                dim: DimSel::Any,
                set: 1,
            },
        ],
    )?;
    let (all, cur) = (&sums[0], &sums[1]);
    let is_pl = |a| {
        ctx.account_type(a)
            .is_some_and(AccountType::is_income_statement)
    };
    let prior: Money = all
        .iter()
        .filter(|(a, _)| is_pl(**a))
        .map(|(_, v)| *v)
        .sum::<Money>()
        - cur
            .iter()
            .filter(|(a, _)| is_pl(**a))
            .map(|(_, v)| *v)
            .sum::<Money>();
    let re = ctx.role(AccountRole::RetainedEarnings);

    let group = Some(short_date(as_of));
    let columns = vec![
        Column::new("label", "", ColumnKind::Text),
        Column::new("debit", "Debit", ColumnKind::Money).grouped(group.clone()),
        Column::new("credit", "Credit", ColumnKind::Money).grouped(group),
    ];
    let mut rows = Vec::new();
    let (mut debits, mut credits) = (Money::ZERO, Money::ZERO);
    for a in &ctx.accounts {
        if a.account_type == AccountType::NonPosting {
            continue;
        }
        let pl = a.account_type.is_income_statement();
        let mut dp = if pl {
            cur.get(&a.id).copied().unwrap_or_default()
        } else {
            all.get(&a.id).copied().unwrap_or_default()
        };
        if Some(a.id) == re {
            dp += prior;
        }
        if dp.is_zero() && !(r.req.show_zero_rows && a.is_active) {
            continue;
        }
        let range = if pl { this_year } else { to_date };
        let drill = r.drill_to(ReportId::TransactionDetailByAccount, range, |f| {
            f.accounts = vec![a.id];
            f.exact = true;
        });
        let (d, c) = if dp.is_negative() {
            (Cell::EMPTY, Cell::money(-dp).drill(drill.clone()))
        } else {
            (Cell::money(dp).drill(drill.clone()), Cell::EMPTY)
        };
        if dp.is_negative() {
            credits += -dp;
        } else {
            debits += dp;
        }
        rows.push(
            Row::new(
                RowKind::Data,
                0,
                vec![Cell::text(ctx.account_full_label(a.id)), d, c],
            )
            .drill(drill),
        );
    }
    if re.is_none() && !prior.is_zero() {
        let (d, c) = if prior.is_negative() {
            credits += -prior;
            (Cell::EMPTY, Cell::money(-prior))
        } else {
            debits += prior;
            (Cell::money(prior), Cell::EMPTY)
        };
        rows.push(Row::new(
            RowKind::Data,
            0,
            vec![Cell::text("Retained Earnings"), d, c],
        ));
    }
    rows.push(Row::new(
        RowKind::Total,
        0,
        vec![
            Cell::text("TOTAL"),
            Cell::money(debits),
            Cell::money(credits),
        ],
    ));
    Ok(r.finish(r.as_of_line(), columns, rows, Vec::new()))
}
