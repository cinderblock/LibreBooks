//! Banking: Deposit Detail, Check Detail, Missing Checks.
//!
//! These read transactions as forms (a deposit and what went into it, a check
//! and what it paid for), so they read the `txn` and `posting` tables directly
//! rather than through a basis: a deposit is the same deposit on either basis.

use std::collections::HashMap;

use jiff::civil::Date;
use lb_core::{AccountId, AccountType, EntityId, ItemId, Money, TxnId, TxnType};
use lb_store::codec::{self, SqlDate};
use rusqlite::params;

use crate::filter::Sql;
use crate::model::{Cell, Column, ColumnKind, Report, Row, RowKind};
use crate::{Result, Run};

/// A transaction header as these reports need it.
struct Head {
    id: TxnId,
    txn_type: TxnType,
    date: Date,
    number: Option<String>,
    entity: Option<EntityId>,
    account: Option<AccountId>,
    amount: Money,
    memo: Option<String>,
    is_void: bool,
}

/// One posting.
struct Post {
    account: AccountId,
    amount: Money,
    entity: Option<EntityId>,
    class: Option<String>,
    item: Option<ItemId>,
    line: Option<String>,
}

fn heads(r: &Run<'_>, types: &[TxnType]) -> Result<Vec<Head>> {
    let mut q = Sql::default();
    let list = q.bind_list(types.iter().map(|t| t.as_str().to_owned()));
    let mut sql = format!(
        "SELECT id, txn_type, date, number, entity_id, account_id, amount, memo, is_void
         FROM txn WHERE txn_type IN {list}"
    );
    if let Some(f) = r.range.from {
        let p = q.bind(f.to_string());
        sql.push_str(&format!(" AND date >= {p}"));
    }
    if let Some(t) = r.range.to {
        let p = q.bind(t.to_string());
        sql.push_str(&format!(" AND date <= {p}"));
    }
    if let Some(a) = &r.filter.accounts {
        let l = q.bind_list(a.iter().map(ToString::to_string));
        sql.push_str(&format!(" AND account_id IN {l}"));
    }
    if let Some(n) = &r.filter.names {
        let l = q.bind_list(n.iter().map(ToString::to_string));
        sql.push_str(&format!(" AND entity_id IN {l}"));
    }
    sql.push_str(" ORDER BY date, rowid");
    q.text = sql;
    let mut stmt = r.c.prepare(&q.text)?;
    let mut rows = stmt.query(q.params())?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let t: String = row.get(1)?;
        let date: SqlDate = row.get(2)?;
        let entity: Option<String> = row.get(4)?;
        let account: Option<String> = row.get(5)?;
        out.push(Head {
            id: codec::parse_id(&row.get::<_, String>(0)?)?,
            txn_type: TxnType::parse(&t).unwrap_or(TxnType::Check),
            date: date.0,
            number: row.get(3)?,
            entity: entity.as_deref().map(codec::parse_id).transpose()?,
            account: account.as_deref().map(codec::parse_id).transpose()?,
            amount: Money(row.get(6)?),
            memo: row.get(7)?,
            is_void: row.get(8)?,
        });
    }
    Ok(out)
}

fn posts(r: &Run<'_>, txn: TxnId) -> Result<Vec<Post>> {
    let mut stmt = r.c.prepare_cached(
        "SELECT account_id, amount, entity_id, class_id, item_id, line_id
         FROM posting WHERE txn_id = ?1 ORDER BY seq",
    )?;
    let mut rows = stmt.query([txn.to_string()])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let entity: Option<String> = row.get(2)?;
        let item: Option<String> = row.get(4)?;
        let class: Option<String> = row.get(3)?;
        out.push(Post {
            account: codec::parse_id(&row.get::<_, String>(0)?)?,
            amount: Money(row.get(1)?),
            entity: entity.as_deref().map(codec::parse_id).transpose()?,
            class: class
                .and_then(|c| c.parse().ok())
                .and_then(|c| r.ctx.class_name(Some(c))),
            item: item.as_deref().map(codec::parse_id).transpose()?,
            line: row.get(5)?,
        });
    }
    Ok(out)
}

fn cols(spec: &[(&str, &str, ColumnKind)]) -> Vec<Column> {
    let mut c = vec![Column::new("label", "", ColumnKind::Text)];
    c.extend(spec.iter().map(|(id, l, k)| Column::new(*id, *l, *k)));
    c
}

fn total(label: &str, width: usize, at: usize, amount: Money) -> Row {
    let mut cells = vec![Cell::text(label)];
    cells.extend(std::iter::repeat_n(Cell::EMPTY, width - 1));
    cells[at] = Cell::money(amount);
    Row::new(RowKind::Total, 0, cells)
}

pub(crate) fn deposit_detail(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let columns = cols(&[
        ("type", "Type", ColumnKind::Text),
        ("num", "Num", ColumnKind::Text),
        ("date", "Date", ColumnKind::Date),
        ("name", "Name", ColumnKind::Text),
        ("account", "Account", ColumnKind::Text),
        ("class", "Class", ColumnKind::Text),
        ("amount", "Amount", ColumnKind::Money),
    ]);
    let width = columns.len();
    let mut rows = Vec::new();
    let mut sum = Money::ZERO;
    for h in heads(r, &[TxnType::Deposit])? {
        // Which lines deposited which payments.
        let mut links: HashMap<String, TxnId> = HashMap::new();
        {
            let mut stmt = r.c.prepare_cached(
                "SELECT line_id, payment_txn_id FROM deposit_link WHERE deposit_txn_id = ?1",
            )?;
            let mut lr = stmt.query([h.id.to_string()])?;
            while let Some(row) = lr.next()? {
                links.insert(row.get(0)?, codec::parse_id(&row.get::<_, String>(1)?)?);
            }
        }
        let ps = posts(r, h.id)?;
        let bank_amount: Money = ps
            .iter()
            .filter(|p| Some(p.account) == h.account && p.line.is_none())
            .map(|p| p.amount)
            .sum();
        sum += bank_amount;
        let drill = Run::txn_drill(h.id);
        rows.push(
            Row::new(
                RowKind::Data,
                0,
                vec![
                    Cell::EMPTY,
                    Cell::text(h.txn_type.label()),
                    Cell::opt_text(h.number.clone()),
                    Cell::date(h.date),
                    Cell::opt_text(ctx.entity_name(h.entity)),
                    Cell::opt_text(h.account.map(|a| ctx.account_full_label(a))),
                    Cell::EMPTY,
                    Cell::money(bank_amount),
                ],
            )
            .drill(drill),
        );
        for p in ps
            .iter()
            .filter(|p| !(Some(p.account) == h.account && p.line.is_none()))
        {
            let payment = p.line.as_ref().and_then(|l| links.get(l)).copied();
            let (t, num, date, name) = match payment {
                Some(pid) => {
                    let pt = lb_store::txns::get(r.c, pid)?;
                    (
                        Cell::text(pt.txn_type.label()),
                        Cell::opt_text(pt.number),
                        Cell::date(pt.date),
                        Cell::opt_text(ctx.entity_name(pt.entity_id)),
                    )
                }
                None => (
                    Cell::EMPTY,
                    Cell::EMPTY,
                    Cell::EMPTY,
                    Cell::opt_text(ctx.entity_name(p.entity)),
                ),
            };
            rows.push(
                Row::new(
                    RowKind::Data,
                    1,
                    vec![
                        Cell::EMPTY,
                        t,
                        num,
                        date,
                        name,
                        Cell::text(ctx.account_full_label(p.account)),
                        Cell::opt_text(p.class.clone()),
                        Cell::money(p.amount),
                    ],
                )
                .drill(Run::txn_drill(payment.unwrap_or(h.id))),
            );
        }
    }
    rows.push(total("TOTAL", width, width - 1, sum));
    Ok(r.finish(r.range_line(), columns, rows, Vec::new()))
}

/// Forms that pay money out of a bank or card account.
const CHECK_TYPES: [TxnType; 7] = [
    TxnType::Check,
    TxnType::BillPaymentCheck,
    TxnType::BillPaymentCreditCard,
    TxnType::CreditCardCharge,
    TxnType::Paycheck,
    TxnType::LiabilityCheck,
    TxnType::SalesTaxPaymentCheck,
];

pub(crate) fn check_detail(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let columns = cols(&[
        ("type", "Type", ColumnKind::Text),
        ("num", "Num", ColumnKind::Text),
        ("date", "Date", ColumnKind::Date),
        ("name", "Name", ColumnKind::Text),
        ("item", "Item", ColumnKind::Text),
        ("account", "Account", ColumnKind::Text),
        ("paid", "Paid Amount", ColumnKind::Money),
        ("original", "Original Amount", ColumnKind::Money),
    ]);
    let width = columns.len();
    let mut rows = Vec::new();
    let mut sum = Money::ZERO;
    for h in heads(r, &CHECK_TYPES)? {
        let paid = if h.is_void { Money::ZERO } else { -h.amount };
        sum += paid;
        rows.push(
            Row::new(
                RowKind::Data,
                0,
                vec![
                    Cell::EMPTY,
                    Cell::text(h.txn_type.label()),
                    Cell::opt_text(h.number.clone()),
                    Cell::date(h.date),
                    Cell::opt_text(ctx.entity_name(h.entity)),
                    Cell::EMPTY,
                    Cell::opt_text(h.account.map(|a| ctx.account_full_label(a))),
                    Cell::money(paid),
                    Cell::EMPTY,
                ],
            )
            .drill(Run::txn_drill(h.id)),
        );
        if matches!(
            h.txn_type,
            TxnType::BillPaymentCheck | TxnType::BillPaymentCreditCard
        ) {
            // The bills it paid.
            let mut stmt = r.c.prepare_cached(
                "SELECT a.target_txn_id, a.amount + a.discount, t.txn_type, t.number, t.date,
                        t.amount, t.ar_ap_account_id
                 FROM application a JOIN txn t ON t.id = a.target_txn_id
                 WHERE a.txn_id = ?1 ORDER BY a.seq",
            )?;
            let mut ar = stmt.query(params![h.id.to_string()])?;
            while let Some(row) = ar.next()? {
                let target: TxnId = codec::parse_id(&row.get::<_, String>(0)?)?;
                let t: String = row.get(2)?;
                let date: SqlDate = row.get(4)?;
                let acct: Option<String> = row.get(6)?;
                rows.push(
                    Row::new(
                        RowKind::Data,
                        1,
                        vec![
                            Cell::EMPTY,
                            Cell::text(TxnType::parse(&t).map_or("", TxnType::label)),
                            Cell::opt_text(row.get::<_, Option<String>>(3)?),
                            Cell::date(date.0),
                            Cell::EMPTY,
                            Cell::EMPTY,
                            Cell::opt_text(
                                acct.and_then(|a| a.parse().ok())
                                    .map(|a| ctx.account_full_label(a)),
                            ),
                            Cell::money(Money(row.get(1)?)),
                            Cell::money(Money(row.get(5)?)),
                        ],
                    )
                    .drill(Run::txn_drill(target)),
                );
            }
            continue;
        }
        for p in posts(r, h.id)?
            .iter()
            .filter(|p| !(Some(p.account) == h.account && p.line.is_none()))
        {
            rows.push(
                Row::new(
                    RowKind::Data,
                    1,
                    vec![
                        Cell::EMPTY,
                        Cell::EMPTY,
                        Cell::EMPTY,
                        Cell::EMPTY,
                        Cell::opt_text(ctx.entity_name(p.entity)),
                        Cell::opt_text(
                            p.item
                                .and_then(|i| ctx.item(i))
                                .map(|i| i.full_name.clone()),
                        ),
                        Cell::text(ctx.account_full_label(p.account)),
                        Cell::money(p.amount),
                        Cell::money(p.amount),
                    ],
                )
                .drill(Run::txn_drill(h.id)),
            );
        }
    }
    rows.push(total("TOTAL", width, width - 2, sum));
    Ok(r.finish(r.range_line(), columns, rows, Vec::new()))
}

/// Forms that use a bank account's check numbers.
const NUMBERED_TYPES: [TxnType; 5] = [
    TxnType::Check,
    TxnType::BillPaymentCheck,
    TxnType::Paycheck,
    TxnType::LiabilityCheck,
    TxnType::SalesTaxPaymentCheck,
];

pub(crate) fn missing_checks(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let columns = cols(&[
        ("type", "Type", ColumnKind::Text),
        ("date", "Date", ColumnKind::Date),
        ("num", "Num", ColumnKind::Text),
        ("name", "Name", ColumnKind::Text),
        ("memo", "Memo", ColumnKind::Text),
        ("account", "Account", ColumnKind::Text),
        ("amount", "Amount", ColumnKind::Money),
    ]);
    let width = columns.len();
    let all = heads(r, &NUMBERED_TYPES)?;
    let mut banks: Vec<AccountId> = all
        .iter()
        .filter_map(|h| h.account)
        .filter(|a| ctx.account_type(*a) == Some(AccountType::Bank))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    banks.sort_by_key(|a| ctx.account_order(*a));
    let note_row = |text: &str| {
        let mut cells = vec![Cell::EMPTY; width];
        cells[1] = Cell::text(text);
        Row::new(RowKind::Data, 1, cells)
    };
    let mut rows = Vec::new();
    for bank in banks {
        let mut checks: Vec<(u64, &Head)> = all
            .iter()
            .filter(|h| h.account == Some(bank))
            .filter_map(|h| {
                let n = h.number.as_deref()?.trim();
                (!n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
                    .then(|| n.parse::<u64>().ok())
                    .flatten()
                    .map(|n| (n, h))
            })
            .collect();
        if checks.is_empty() {
            continue;
        }
        checks.sort_by_key(|(n, h)| (*n, h.date));
        let mut head = vec![Cell::EMPTY; width];
        head[0] = Cell::text(ctx.account_full_label(bank));
        rows.push(Row::new(RowKind::Header, 0, head));
        let mut prev: Option<u64> = None;
        for (n, h) in checks {
            if let Some(p) = prev {
                if n == p {
                    rows.push(note_row("*** Duplicate document numbers ***"));
                } else if n > p + 1 {
                    rows.push(note_row("*** Missing numbers here ***"));
                }
            }
            prev = Some(n);
            rows.push(
                Row::new(
                    RowKind::Data,
                    1,
                    vec![
                        Cell::EMPTY,
                        Cell::text(h.txn_type.label()),
                        Cell::date(h.date),
                        Cell::opt_text(h.number.clone()),
                        Cell::opt_text(ctx.entity_name(h.entity)),
                        Cell::opt_text(h.memo.clone()),
                        Cell::text(ctx.account_full_label(bank)),
                        Cell::money(if h.is_void { Money::ZERO } else { -h.amount }),
                    ],
                )
                .drill(Run::txn_drill(h.id)),
            );
        }
    }
    Ok(r.finish(r.range_line(), columns, rows, Vec::new()))
}
