//! Statement of Cash Flows, by the indirect method, as QBD builds it.
//!
//! Start from net income for the period, then add back the change in every
//! balance-sheet account, grouped by the account's cash-flow class (QBD's
//! "Classify Cash" preference, `Account.cash_flow_class`, defaulting by type):
//! an asset that grew used cash (negative), a liability or equity account that
//! grew provided it (positive). Accounts classed *not applicable* (bank accounts
//! by default) are the cash itself. Because every transaction balances, net
//! income plus the three sections equals the change in cash exactly.

use std::collections::HashMap;

use lb_core::{AccountId, CashFlowClass, DateRange, Money};

use crate::dates::short_range;
use crate::model::{Cell, Column, ColumnKind, Report, Row, RowKind};
use crate::request::ReportId;
use crate::{Result, Run};

pub(crate) fn statement(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let ledger = r.ledger();
    let to = r.range.to.unwrap_or(ctx.today);
    let period = DateRange {
        from: r.range.from,
        to: Some(to),
    };
    let changes = ledger.sums(period)?;
    let begin: HashMap<AccountId, Money> = match r.range.from.and_then(|f| f.yesterday().ok()) {
        Some(b) => ledger.sums(DateRange {
            from: None,
            to: Some(b),
        })?,
        None => HashMap::new(),
    };
    let mut net_income = Money::ZERO;
    let mut sections: HashMap<CashFlowClass, Vec<(AccountId, Money)>> = HashMap::new();
    let mut cash_begin = Money::ZERO;
    let mut cash_change = Money::ZERO;
    for a in &ctx.accounts {
        let t = a.account_type;
        let change = changes.get(&a.id).copied().unwrap_or_default();
        if t.is_income_statement() {
            net_income -= change;
            continue;
        }
        if !t.is_balance_sheet() {
            continue;
        }
        let class = a
            .cash_flow_class
            .unwrap_or_else(|| CashFlowClass::default_for(t));
        if class == CashFlowClass::NotApplicable {
            cash_begin += begin.get(&a.id).copied().unwrap_or_default();
            cash_change += change;
        } else if !change.is_zero() {
            sections.entry(class).or_default().push((a.id, -change));
        }
    }
    let label = short_range(period);
    let columns = vec![
        Column::new("label", "", ColumnKind::Text),
        Column::new("amount", label, ColumnKind::Money),
    ];
    let row = |kind, depth, label: &str, amount: Option<Money>| {
        Row::new(
            kind,
            depth,
            vec![Cell::text(label), amount.map_or(Cell::EMPTY, Cell::money)],
        )
    };
    let account_row = |a: AccountId, amount: Money, depth: u32| {
        let drill = r.drill_to(ReportId::TransactionDetailByAccount, period, |f| {
            f.accounts = vec![a];
            f.exact = true;
        });
        Row::new(
            RowKind::Data,
            depth,
            vec![
                Cell::text(ctx.account_full_label(a)),
                Cell::money(amount).drill(drill.clone()),
            ],
        )
        .drill(drill)
    };
    let mut rows = vec![row(RowKind::Header, 0, "OPERATING ACTIVITIES", None)];
    rows.push(
        row(RowKind::Data, 1, "Net Income", Some(net_income)).drill(r.drill_to(
            ReportId::ProfitAndLoss,
            period,
            |_| {},
        )),
    );
    let mut operating = net_income;
    if let Some(adj) = sections.get(&CashFlowClass::Operating) {
        rows.push(row(
            RowKind::Header,
            1,
            "Adjustments to reconcile Net Income to net cash provided by operations:",
            None,
        ));
        for (a, m) in adj {
            operating += *m;
            rows.push(account_row(*a, *m, 2));
        }
    }
    rows.push(row(
        RowKind::Subtotal,
        0,
        "Net cash provided by Operating Activities",
        Some(operating),
    ));
    let mut increase = operating;
    for (class, head, total) in [
        (
            CashFlowClass::Investing,
            "INVESTING ACTIVITIES",
            "Net cash provided by Investing Activities",
        ),
        (
            CashFlowClass::Financing,
            "FINANCING ACTIVITIES",
            "Net cash provided by Financing Activities",
        ),
    ] {
        if let Some(items) = sections.get(&class) {
            rows.push(row(RowKind::Header, 0, head, None));
            let mut sum = Money::ZERO;
            for (a, m) in items {
                sum += *m;
                rows.push(account_row(*a, *m, 1));
            }
            rows.push(row(RowKind::Subtotal, 0, total, Some(sum)));
            increase += sum;
        }
    }
    let mut notes = Vec::new();
    if increase != cash_change {
        // Only possible when filters leave transactions half-counted.
        notes.push(format!(
            "The filters leave cash changes unexplained by {}.",
            cash_change - increase
        ));
    }
    rows.push(row(
        RowKind::Total,
        0,
        "Net cash increase for period",
        Some(increase),
    ));
    rows.push(row(
        RowKind::Data,
        0,
        "Cash at beginning of period",
        Some(cash_begin),
    ));
    rows.push(row(
        RowKind::Total,
        0,
        "Cash at end of period",
        Some(cash_begin + cash_change),
    ));
    Ok(r.finish(r.range_line(), columns, rows, notes))
}
