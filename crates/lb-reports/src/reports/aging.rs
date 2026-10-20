//! What customers owe and what is owed to vendors: aging summaries and details,
//! open invoices, unpaid bills, and customer and vendor balances.
//!
//! "Open as of D" is computed from the ledger and the `application` rows dated
//! on or before D, never from the stored `txn.open_balance` (which is open as of
//! now). For each transaction X with postings in A/R (or A/P), in debit-positive
//! terms, with s = +1 for A/R and −1 for A/P:
//!
//! ```text
//! open(X) = Σ X's A/R postings dated ≤ D
//!         − s · Σ (amount + discount + credit) of applications to X dated ≤ D
//!         + s · Σ (amount + discount) of X's own applications (X is a payment, dated ≤ D)
//!         + s · Σ credit of applications using X as a credit, dated ≤ D
//! ```
//!
//! Every application relieves its target, its payer and its credit by the same
//! amounts, so the open amounts add up to the A/R (A/P) balance as of D exactly.
//! Aging counts days past the due date (the transaction date for credits and
//! payments): Current, 1–30, 31–60, 61–90, over 90.

use std::collections::{HashMap, HashSet};

use jiff::civil::Date;
use lb_core::{AccountId, AccountType, DateRange, EntityId, Money, TxnId, TxnType};
use lb_store::codec::{self, SqlDate};
use rusqlite::params;

use super::detail::{DCol, Splits, Totals, columns, fact_cells, total_row};
use crate::dates::{days_between, short_date};
use crate::filter::Sql;
use crate::ledger::{Dims, Fact, Gran};
use crate::model::{Cell, Column, ColumnKind, Drill, Report, Row, RowKind};
use crate::request::ReportId;
use crate::summary::{Line, entity_tree};
use crate::{Result, Run};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Receivable,
    Payable,
}

impl Side {
    fn account_type(self) -> AccountType {
        match self {
            Side::Receivable => AccountType::AccountsReceivable,
            Side::Payable => AccountType::AccountsPayable,
        }
    }

    fn sign(self) -> i64 {
        match self {
            Side::Receivable => 1,
            Side::Payable => -1,
        }
    }

    /// Owed amounts show positive: A/R debit-normal, A/P credit-normal.
    fn natural(self, debit_positive: Money) -> Money {
        self.account_type().natural(debit_positive)
    }

    fn detail_report(self) -> ReportId {
        match self {
            Side::Receivable => ReportId::ArAgingDetail,
            Side::Payable => ReportId::ApAgingDetail,
        }
    }

    fn balance_detail(self) -> ReportId {
        match self {
            Side::Receivable => ReportId::CustomerBalanceDetail,
            Side::Payable => ReportId::VendorBalanceDetail,
        }
    }
}

/// One open transaction (or open part of one) as of a date.
#[derive(Clone, Debug)]
pub(crate) struct OpenItem {
    pub txn_id: TxnId,
    pub txn_type: TxnType,
    pub date: Date,
    pub number: Option<String>,
    pub due: Date,
    pub entity: Option<EntityId>,
    pub account: AccountId,
    /// Owed, in the side's natural sign (a credit is negative).
    pub open: Money,
    pub memo: Option<String>,
    pub terms: Option<String>,
    pub po_number: Option<String>,
}

impl OpenItem {
    fn days_past_due(&self, as_of: Date) -> i64 {
        days_between(self.due, as_of)
    }
}

const BUCKETS: [&str; 5] = ["Current", "1 - 30", "31 - 60", "61 - 90", "> 90"];

fn bucket(days: i64) -> usize {
    match days {
        i64::MIN..=0 => 0,
        1..=30 => 1,
        31..=60 => 2,
        61..=90 => 3,
        _ => 4,
    }
}

/// Every transaction open in A/R (or A/P) as of `as_of`, through the request's
/// name, account and transaction-type filters.
pub(crate) fn open_items_as_of(
    r: &Run<'_>,
    side: Side,
    as_of: Option<Date>,
) -> Result<Vec<OpenItem>> {
    let conn = r.c;
    let as_of_sql = as_of.map(SqlDate);
    // (txn, account, name) → debit-positive sum, and the order of first posting.
    let mut parts: HashMap<TxnId, Vec<(AccountId, Option<EntityId>, i64)>> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT p.txn_id, p.account_id, p.entity_id, SUM(p.amount), MIN(p.seq)
             FROM posting p JOIN account a ON a.id = p.account_id
             WHERE a.account_type = ?1 AND (?2 IS NULL OR p.date <= ?2)
             GROUP BY p.txn_id, p.account_id, p.entity_id
             ORDER BY p.txn_id, MIN(p.seq)",
        )?;
        let mut rows = stmt.query(params![side.account_type().as_str(), as_of_sql])?;
        while let Some(row) = rows.next()? {
            let txn: TxnId = codec::parse_id(&row.get::<_, String>(0)?)?;
            let account: AccountId = codec::parse_id(&row.get::<_, String>(1)?)?;
            let entity: Option<String> = row.get(2)?;
            parts.entry(txn).or_default().push((
                account,
                entity.as_deref().map(codec::parse_id).transpose()?,
                row.get(3)?,
            ));
        }
    }
    let relief = |sql: &str| -> Result<HashMap<TxnId, i64>> {
        let mut out = HashMap::new();
        let mut stmt = conn.prepare(sql)?;
        let mut rows = stmt.query(params![as_of_sql])?;
        while let Some(row) = rows.next()? {
            let id: TxnId = codec::parse_id(&row.get::<_, String>(0)?)?;
            out.insert(id, row.get::<_, i64>(1)?);
        }
        Ok(out)
    };
    let as_target = relief(
        "SELECT target_txn_id, SUM(amount + discount + credit_amount) FROM application
         WHERE (?1 IS NULL OR date <= ?1) GROUP BY target_txn_id",
    )?;
    let as_payer = relief(
        "SELECT txn_id, SUM(amount + discount) FROM application
         WHERE (?1 IS NULL OR date <= ?1) GROUP BY txn_id",
    )?;
    let as_credit = relief(
        "SELECT credit_txn_id, SUM(credit_amount) FROM application
         WHERE credit_txn_id IS NOT NULL AND (?1 IS NULL OR date <= ?1) GROUP BY credit_txn_id",
    )?;
    let sign = side.sign();
    let mut open: Vec<(TxnId, AccountId, Option<EntityId>, Money)> = Vec::new();
    for (txn, rows) in parts {
        let adj = -sign * as_target.get(&txn).copied().unwrap_or(0)
            + sign * as_payer.get(&txn).copied().unwrap_or(0)
            + sign * as_credit.get(&txn).copied().unwrap_or(0);
        for (i, (account, entity, amount)) in rows.into_iter().enumerate() {
            // Payments relieve the transaction as a whole; with several names in
            // one (a journal entry) they relieve the first.
            let dp = amount + if i == 0 { adj } else { 0 };
            if dp != 0 {
                open.push((txn, account, entity, side.natural(Money(dp))));
            }
        }
    }
    // Headers of what is open.
    let ids: Vec<TxnId> = open
        .iter()
        .map(|o| o.0)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    struct Header {
        txn_type: TxnType,
        date: Date,
        number: Option<String>,
        due: Option<Date>,
        memo: Option<String>,
        terms: Option<String>,
        po: Option<String>,
    }
    let mut headers: HashMap<TxnId, Header> = HashMap::new();
    for chunk in ids.chunks(500) {
        let mut q = Sql::default();
        let list = q.bind_list(chunk.iter().map(ToString::to_string));
        q.text = format!(
            "SELECT id, txn_type, date, number, due_date, memo,
                    json_extract(doc, '$.terms_id'), json_extract(doc, '$.po_number')
             FROM txn WHERE id IN {list}"
        );
        let mut stmt = conn.prepare(&q.text)?;
        let mut rows = stmt.query(q.params())?;
        while let Some(row) = rows.next()? {
            let t: String = row.get(1)?;
            let date: SqlDate = row.get(2)?;
            let due: Option<SqlDate> = row.get(4)?;
            let terms: Option<String> = row.get(6)?;
            headers.insert(
                codec::parse_id(&row.get::<_, String>(0)?)?,
                Header {
                    txn_type: TxnType::parse(&t).unwrap_or(TxnType::JournalEntry),
                    date: date.0,
                    number: row.get(3)?,
                    due: due.map(|d| d.0),
                    memo: row.get(5)?,
                    terms: terms
                        .and_then(|t| t.parse().ok())
                        .and_then(|id| r.ctx.terms.get(&id))
                        .map(|t| t.name.clone()),
                    po: row.get(7)?,
                },
            );
        }
    }
    let filter = &r.filter;
    let mut out: Vec<OpenItem> = open
        .into_iter()
        .filter_map(|(txn, account, entity, amount)| {
            let h = headers.get(&txn)?;
            let is_credit = matches!(
                h.txn_type,
                TxnType::CreditMemo
                    | TxnType::VendorCredit
                    | TxnType::ReceivePayment
                    | TxnType::BillPaymentCheck
                    | TxnType::BillPaymentCreditCard
            );
            Some(OpenItem {
                txn_id: txn,
                txn_type: h.txn_type,
                date: h.date,
                number: h.number.clone(),
                due: if is_credit {
                    h.date
                } else {
                    h.due.unwrap_or(h.date)
                },
                entity,
                account,
                open: amount,
                memo: h.memo.clone(),
                terms: h.terms.clone(),
                po_number: h.po.clone(),
            })
        })
        .filter(|o| {
            filter
                .accounts
                .as_ref()
                .is_none_or(|a| a.contains(&o.account))
        })
        .filter(|o| {
            filter
                .names
                .as_ref()
                .is_none_or(|n| o.entity.is_some_and(|e| n.contains(&e)))
        })
        .filter(|o| {
            filter
                .txn_types
                .as_ref()
                .is_none_or(|t| t.contains(&o.txn_type))
        })
        .collect();
    out.sort_by(|a, b| (a.date, a.number.as_deref()).cmp(&(b.date, b.number.as_deref())));
    Ok(out)
}

pub(crate) fn aging_summary(r: &Run<'_>, side: Side) -> Result<Report> {
    let ctx = r.ctx;
    let as_of = r.as_of().unwrap_or(ctx.today);
    let items = open_items_as_of(r, side, Some(as_of))?;
    let mut by_name: HashMap<Option<EntityId>, Vec<Money>> = HashMap::new();
    for it in &items {
        let v = by_name
            .entry(it.entity)
            .or_insert_with(|| vec![Money::ZERO; 6]);
        v[bucket(it.days_past_due(as_of))] += it.open;
        v[5] += it.open;
    }
    let named: HashSet<EntityId> = by_name.keys().flatten().copied().collect();
    let values = |e: EntityId| {
        by_name
            .get(&Some(e))
            .cloned()
            .unwrap_or_else(|| vec![Money::ZERO; 6])
    };
    let drill = |e: EntityId, _exact: bool, _col: Option<usize>| -> Option<Drill> {
        r.drill_to(
            side.detail_report(),
            DateRange {
                from: None,
                to: Some(as_of),
            },
            |f| f.names = vec![e],
        )
    };
    let (mut lines, mut total) = entity_tree(ctx, &named, &values, 6, false, 0, &drill);
    if let Some(v) = by_name.get(&None) {
        lines.push(Line::amounts(RowKind::Data, 0, "No name", v.clone()));
        crate::summary::add(&mut total, v);
    }
    lines.push(Line::amounts(RowKind::Total, 0, "TOTAL", total));
    let mut columns = vec![Column::new("label", "", ColumnKind::Text)];
    for (i, b) in BUCKETS.iter().enumerate() {
        columns.push(Column::new(format!("b{i}"), *b, ColumnKind::Money));
    }
    columns.push(Column::new("total", "TOTAL", ColumnKind::Money));
    let rows =
        lines
            .into_iter()
            .map(|l| {
                let mut cells = vec![Cell::text(l.label)];
                match l.values {
                    Some(v) => cells.extend(v.iter().enumerate().map(|(i, m)| {
                        Cell::money(*m).drill(l.value_drills.get(i).cloned().flatten())
                    })),
                    None => cells.extend(std::iter::repeat_n(Cell::EMPTY, 6)),
                }
                Row::new(l.kind, l.depth, cells).drill(l.drill)
            })
            .collect();
    Ok(r.finish(r.as_of_line(), columns, rows, Vec::new()))
}

/// Columns for lists of open items.
fn item_columns(side: Side, with_name: bool) -> Vec<Column> {
    let mut c = vec![
        Column::new("label", "", ColumnKind::Text),
        Column::new("type", "Type", ColumnKind::Text),
        Column::new("date", "Date", ColumnKind::Date),
        Column::new("num", "Num", ColumnKind::Text),
    ];
    if side == Side::Receivable {
        c.push(Column::new("po", "P. O. #", ColumnKind::Text));
    }
    if with_name {
        c.push(Column::new("name", "Name", ColumnKind::Text));
    }
    c.push(Column::new("terms", "Terms", ColumnKind::Text));
    c.push(Column::new("due", "Due Date", ColumnKind::Date));
    c.push(Column::new("aging", "Aging", ColumnKind::Number));
    c.push(Column::new("open", "Open Balance", ColumnKind::Money));
    c
}

fn item_row(
    r: &Run<'_>,
    side: Side,
    it: &OpenItem,
    as_of: Date,
    with_name: bool,
    depth: u32,
) -> Row {
    let days = it.days_past_due(as_of);
    let mut cells = vec![
        Cell::EMPTY,
        Cell::text(it.txn_type.label()),
        Cell::date(it.date),
        Cell::opt_text(it.number.clone()),
    ];
    if side == Side::Receivable {
        cells.push(Cell::opt_text(it.po_number.clone()));
    }
    if with_name {
        cells.push(Cell::opt_text(r.ctx.entity_name(it.entity)));
    }
    cells.push(Cell::opt_text(it.terms.clone()));
    cells.push(Cell::date(it.due));
    cells.push(if days > 0 {
        Cell::number(days.to_string())
    } else {
        Cell::EMPTY
    });
    cells.push(Cell::money(it.open));
    Row::new(RowKind::Data, depth, cells).drill(Run::txn_drill(it.txn_id))
}

fn amount_row(kind: RowKind, depth: u32, label: String, width: usize, amount: Money) -> Row {
    let mut cells = vec![Cell::text(label)];
    cells.extend(std::iter::repeat_n(Cell::EMPTY, width - 2));
    cells.push(Cell::money(amount));
    Row::new(kind, depth, cells)
}

pub(crate) fn aging_detail(r: &Run<'_>, side: Side) -> Result<Report> {
    let as_of = r.as_of().unwrap_or(r.ctx.today);
    let items = open_items_as_of(r, side, Some(as_of))?;
    let columns = item_columns(side, true);
    let width = columns.len();
    let mut rows = Vec::new();
    let mut total = Money::ZERO;
    for (b, label) in BUCKETS.iter().enumerate() {
        let mut in_bucket: Vec<&OpenItem> = items
            .iter()
            .filter(|i| bucket(i.days_past_due(as_of)) == b)
            .collect();
        if in_bucket.is_empty() {
            continue;
        }
        sort_items(r, &mut in_bucket);
        rows.push(Row::new(
            RowKind::Header,
            0,
            std::iter::once(Cell::text(*label))
                .chain(std::iter::repeat_n(Cell::EMPTY, width - 1))
                .collect(),
        ));
        let mut sum = Money::ZERO;
        for it in in_bucket {
            sum += it.open;
            rows.push(item_row(r, side, it, as_of, true, 1));
        }
        rows.push(amount_row(
            RowKind::Subtotal,
            0,
            format!("Total {label}"),
            width,
            sum,
        ));
        total += sum;
    }
    rows.push(amount_row(RowKind::Total, 0, "TOTAL".into(), width, total));
    Ok(r.finish(r.as_of_line(), columns, rows, Vec::new()))
}

fn sort_items(r: &Run<'_>, items: &mut [&OpenItem]) {
    use crate::request::SortBy;
    match r.req.sort_by {
        SortBy::Number => items.sort_by(|a, b| a.number.cmp(&b.number)),
        SortBy::Amount => items.sort_by_key(|i| i.open),
        SortBy::Name => items.sort_by_key(|i| r.ctx.entity_name(i.entity).unwrap_or_default()),
        SortBy::Memo => items.sort_by(|a, b| a.memo.cmp(&b.memo)),
        SortBy::Default | SortBy::Date => items.sort_by_key(|i| i.date),
    }
    if r.req.sort_descending {
        items.reverse();
    }
}

/// Open Invoices and Unpaid Bills Detail: open items grouped by name.
pub(crate) fn open_items(r: &Run<'_>, side: Side) -> Result<Report> {
    let ctx = r.ctx;
    let as_of = r.as_of().unwrap_or(ctx.today);
    let items = open_items_as_of(r, side, Some(as_of))?;
    let columns = item_columns(side, false);
    let width = columns.len();
    let mut groups: HashMap<Option<EntityId>, Vec<&OpenItem>> = HashMap::new();
    for it in &items {
        groups.entry(it.entity).or_default().push(it);
    }
    let mut keys: Vec<Option<EntityId>> = groups.keys().copied().collect();
    keys.sort_by_key(|e| {
        (
            e.is_none(),
            ctx.entity_name(*e).unwrap_or_default().to_lowercase(),
        )
    });
    let mut rows = Vec::new();
    let mut total = Money::ZERO;
    for k in keys {
        let mut g = groups.remove(&k).unwrap_or_default();
        sort_items(r, &mut g);
        let name = ctx.entity_name(k).unwrap_or_else(|| "No name".to_owned());
        let drill = k.map(|entity_id| Drill::Name { entity_id });
        rows.push(
            Row::new(
                RowKind::Header,
                0,
                std::iter::once(Cell::text(name.clone()))
                    .chain(std::iter::repeat_n(Cell::EMPTY, width - 1))
                    .collect(),
            )
            .drill(drill.clone()),
        );
        let mut sum = Money::ZERO;
        for it in g {
            sum += it.open;
            rows.push(item_row(r, side, it, as_of, false, 1));
        }
        rows.push(
            amount_row(RowKind::Subtotal, 0, format!("Total {name}"), width, sum).drill(drill),
        );
        total += sum;
    }
    rows.push(amount_row(RowKind::Total, 0, "TOTAL".into(), width, total));
    Ok(r.finish(r.as_of_line(), columns, rows, Vec::new()))
}

/// The A/R (A/P) accounts.
fn side_accounts(r: &Run<'_>, side: Side) -> HashSet<AccountId> {
    r.ctx
        .accounts
        .iter()
        .filter(|a| a.account_type == side.account_type())
        .map(|a| a.id)
        .collect()
}

/// Customer (Vendor) Balance Summary: each name's A/R (A/P) balance as of a date,
/// jobs under their customers.
pub(crate) fn balance_summary(r: &Run<'_>, side: Side) -> Result<Report> {
    let ctx = r.ctx;
    let as_of = r.as_of().unwrap_or(ctx.today);
    let ledger = r.accrual_ledger();
    let filter = r.filter.clone().and_accounts(side_accounts(r, side));
    let agg = ledger.aggregate_with(
        &filter,
        DateRange {
            from: None,
            to: Some(as_of),
        },
        Gran::None,
        Dims {
            names: true,
            ..Dims::default()
        },
    )?;
    let mut by_name: HashMap<Option<EntityId>, Money> = HashMap::new();
    for (k, v) in agg {
        *by_name.entry(k.entity_id).or_default() += side.natural(v);
    }
    by_name.retain(|_, v| !v.is_zero());
    let named: HashSet<EntityId> = by_name.keys().flatten().copied().collect();
    let values = |e: EntityId| vec![by_name.get(&Some(e)).copied().unwrap_or_default()];
    let drill = |e: EntityId, _: bool, _: Option<usize>| {
        r.drill_to(
            side.balance_detail(),
            DateRange {
                from: None,
                to: Some(as_of),
            },
            |f| f.names = vec![e],
        )
    };
    let (mut lines, mut total) =
        entity_tree(ctx, &named, &values, 1, r.req.show_zero_rows, 0, &drill);
    if let Some(v) = by_name.get(&None) {
        lines.push(Line::amounts(RowKind::Data, 0, "No name", vec![*v]));
        total[0] += *v;
    }
    lines.push(Line::amounts(RowKind::Total, 0, "TOTAL", total));
    let columns = vec![
        Column::new("label", "", ColumnKind::Text),
        Column::new("balance", short_date(as_of), ColumnKind::Money),
    ];
    let rows = lines
        .into_iter()
        .map(|l| {
            let cells = vec![
                Cell::text(l.label),
                l.values.map_or(Cell::EMPTY, |v| {
                    Cell::money(v[0]).drill(l.value_drills.first().cloned().flatten())
                }),
            ];
            Row::new(l.kind, l.depth, cells).drill(l.drill)
        })
        .collect();
    Ok(r.finish(r.as_of_line(), columns, rows, Vec::new()))
}

/// Customer (Vendor) Balance Detail: every A/R (A/P) line by name, with a running
/// balance.
pub(crate) fn balance_detail(r: &Run<'_>, side: Side) -> Result<Report> {
    let ctx = r.ctx;
    let cols = [
        DCol::Type,
        DCol::Date,
        DCol::Num,
        DCol::Memo,
        DCol::Account,
        DCol::Amount,
        DCol::Balance,
    ];
    let ledger = r.accrual_ledger();
    let filter = r.filter.clone().and_accounts(side_accounts(r, side));
    let facts = ledger.facts_with(&filter, r.range)?;
    let opening: HashMap<Option<EntityId>, Money> =
        match r.range.from.and_then(|f| f.yesterday().ok()) {
            Some(before) => {
                let agg = ledger.aggregate_with(
                    &filter,
                    DateRange {
                        from: None,
                        to: Some(before),
                    },
                    Gran::None,
                    Dims {
                        names: true,
                        ..Dims::default()
                    },
                )?;
                let mut m = HashMap::new();
                for (k, v) in agg {
                    *m.entry(k.entity_id).or_default() += side.natural(v);
                }
                m
            }
            None => HashMap::new(),
        };
    let splits = Splits::load(r, &facts)?;
    let mut groups: HashMap<Option<EntityId>, Vec<Fact>> = HashMap::new();
    for f in facts {
        groups.entry(f.entity_id).or_default().push(f);
    }
    for k in opening.keys() {
        if !opening[k].is_zero() {
            groups.entry(*k).or_default();
        }
    }
    let mut keys: Vec<Option<EntityId>> = groups.keys().copied().collect();
    keys.sort_by_key(|e| {
        (
            e.is_none(),
            ctx.entity_name(*e).unwrap_or_default().to_lowercase(),
        )
    });
    let mut rows = Vec::new();
    let mut grand = Money::ZERO;
    let mut grand_amount = Money::ZERO;
    for k in keys {
        let mut fs = groups.remove(&k).unwrap_or_default();
        super::detail::sort_facts(r, &mut fs);
        let name = ctx.entity_name(k).unwrap_or_else(|| "No name".to_owned());
        let open = opening.get(&k).copied().unwrap_or_default();
        let mut head = vec![Cell::text(name.clone())];
        for c in &cols {
            head.push(if *c == DCol::Balance && !open.is_zero() {
                Cell::money(open)
            } else {
                Cell::EMPTY
            });
        }
        rows.push(
            Row::new(RowKind::Header, 0, head).drill(k.map(|entity_id| Drill::Name { entity_id })),
        );
        let mut bal = open;
        let mut amount = Money::ZERO;
        for f in &fs {
            let a = side.natural(f.amount);
            bal += a;
            amount += a;
            rows.push(
                Row::new(
                    RowKind::Data,
                    1,
                    fact_cells(r, &splits, f, &cols, a, Some(bal), None),
                )
                .drill(Run::txn_drill(f.txn_id)),
            );
        }
        rows.push(total_row(
            RowKind::Subtotal,
            0,
            format!("Total {name}"),
            &cols,
            Totals {
                debit: Money::ZERO,
                credit: Money::ZERO,
                amount,
                balance: Some(bal),
            },
        ));
        grand += bal;
        grand_amount += amount;
    }
    rows.push(total_row(
        RowKind::Total,
        0,
        "TOTAL".into(),
        &cols,
        Totals {
            debit: Money::ZERO,
            credit: Money::ZERO,
            amount: grand_amount,
            balance: Some(grand),
        },
    ));
    Ok(r.finish(r.range_line(), columns(&cols), rows, Vec::new()))
}
