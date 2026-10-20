//! Sales, purchases, jobs and inventory: Sales by Customer and by Item (summary
//! and detail), Expenses by Vendor (summary and detail), Job Profitability
//! (summary and detail) and Inventory Valuation.
//!
//! *Sales* are amounts in income accounts (Income and Other Income) on sales
//! forms (invoices, sales receipts, credit memos, statement charges): items and
//! discounts, not sales tax (a liability) nor inventory cost (COGS). On cash
//! basis they count when paid, attributed to the invoice's customer and item.
//!
//! The customer or job a line belongs to is the line's own name when it is a
//! customer (an expense passed on to a job), else the transaction's (an
//! invoice's customer); sub-jobs roll up into their customer's total.

use std::collections::{HashMap, HashSet};

use jiff::civil::Date;
use lb_core::{
    AccountClass, AccountId, AccountType, DateRange, EntityId, ItemId, ItemType, LineId, Money,
    Txn, TxnId, TxnType,
};
use lb_store::codec::SqlDate;
use rust_decimal::{Decimal, RoundingStrategy};

use super::detail::{DCol, Splits, fact_cells, sort_facts};
use crate::filter::{Filter, Sql};
use crate::ledger::{Dims, Fact, Gran};
use crate::model::{Cell, Column, ColumnKind, Drill, Report, Row, RowKind};
use crate::request::{ColumnLayout, ReportId};
use crate::summary::{BaseCol, Line, add, base_columns, entity_tree, percent};
use crate::{Result, Run};

/// Sales forms.
pub(crate) const SALES_TYPES: [TxnType; 4] = [
    TxnType::Invoice,
    TxnType::SalesReceipt,
    TxnType::CreditMemo,
    TxnType::StatementCharge,
];

fn accounts_where(r: &Run<'_>, pred: impl Fn(AccountType) -> bool) -> HashSet<AccountId> {
    r.ctx
        .accounts
        .iter()
        .filter(|a| pred(a.account_type))
        .map(|a| a.id)
        .collect()
}

fn income_accounts(r: &Run<'_>) -> HashSet<AccountId> {
    accounts_where(r, |t| t.class() == AccountClass::Income)
}

fn expense_accounts(r: &Run<'_>) -> HashSet<AccountId> {
    accounts_where(r, |t| t.class() == AccountClass::Expense)
}

/// Period columns for a name summary (names are the rows, so the columns can
/// only be periods).
fn period_columns(r: &Run<'_>, notes: &mut Vec<String>) -> Result<Vec<BaseCol>> {
    let mut layout = r.layout();
    if !(layout == ColumnLayout::TotalOnly || layout.is_period()) {
        notes.push("This report's columns can only be periods; showing one column.".into());
        layout = ColumnLayout::TotalOnly;
    }
    base_columns(&r.ledger(), r.range, layout, &HashSet::new())
}

fn label_row(label: String, width: usize, kind: RowKind, depth: u32) -> Row {
    Row::new(
        kind,
        depth,
        std::iter::once(Cell::text(label))
            .chain(std::iter::repeat_n(Cell::EMPTY, width - 1))
            .collect(),
    )
}

/// Lines (from a tree) laid out as label + one money column per value.
fn money_rows(lines: Vec<Line>, width: usize) -> Vec<Row> {
    lines
        .into_iter()
        .map(|l| {
            let mut cells = vec![Cell::text(l.label)];
            match l.values {
                Some(v) => {
                    cells.extend(v.iter().enumerate().map(|(i, m)| {
                        Cell::money(*m).drill(l.value_drills.get(i).cloned().flatten())
                    }));
                }
                None => cells.extend(std::iter::repeat_n(Cell::EMPTY, width)),
            }
            Row::new(l.kind, l.depth, cells).drill(l.drill)
        })
        .collect()
}

/// A name summary: rows keyed by name (customers with jobs indented), one column
/// per period, amounts in `accounts` from `origin` forms, keyed by `key`.
fn name_summary(
    r: &Run<'_>,
    accounts: HashSet<AccountId>,
    origin: Option<&[TxnType]>,
    natural: impl Fn(Money) -> Money,
    key: impl Fn(Option<EntityId>, Option<EntityId>) -> Option<EntityId>,
    detail: ReportId,
    tree: bool,
) -> Result<Report> {
    let ctx = r.ctx;
    let mut notes = Vec::new();
    let cols = period_columns(r, &mut notes)?;
    let width = cols.len();
    let ledger = r.ledger();
    let filter = r.filter.clone().and_accounts(accounts);
    let mut values: HashMap<Option<EntityId>, Vec<Money>> = HashMap::new();
    for (ci, col) in cols.iter().enumerate() {
        let agg = ledger.aggregate_with(
            &filter,
            col.range,
            Gran::None,
            Dims {
                names: true,
                origin_type: true,
                ..Dims::default()
            },
        )?;
        for (k, v) in agg {
            if origin.is_some_and(|o| !k.origin_type.is_some_and(|t| o.contains(&t))) {
                continue;
            }
            values
                .entry(key(k.entity_id, k.txn_entity_id))
                .or_insert_with(|| vec![Money::ZERO; width])[ci] += natural(v);
        }
    }
    values.retain(|_, v| v.iter().any(|m| !m.is_zero()));
    let ranges: Vec<DateRange> = cols.iter().map(|c| c.range).collect();
    let drill = |e: EntityId, _: bool, col: Option<usize>| -> Option<Drill> {
        let range = col.map_or(r.range, |c| ranges[c]);
        r.drill_to(detail, range, |f| f.names = vec![e])
    };
    let get = |e: EntityId| {
        values
            .get(&Some(e))
            .cloned()
            .unwrap_or_else(|| vec![Money::ZERO; width])
    };
    let named: HashSet<EntityId> = values.keys().flatten().copied().collect();
    let (mut lines, mut total) = if tree {
        entity_tree(ctx, &named, &get, width, r.req.show_zero_rows, 0, &drill)
    } else {
        let mut names: Vec<EntityId> = named.iter().copied().collect();
        names.sort_by_key(|e| ctx.entity_name(Some(*e)).unwrap_or_default().to_lowercase());
        let mut lines = Vec::new();
        let mut total = vec![Money::ZERO; width];
        for e in names {
            let v = get(e);
            add(&mut total, &v);
            let mut l = Line::amounts(
                RowKind::Data,
                0,
                ctx.entity_name(Some(e)).unwrap_or_default(),
                v,
            );
            l.drill = drill(e, false, None);
            l.value_drills = (0..width).map(|c| drill(e, false, Some(c))).collect();
            lines.push(l);
        }
        (lines, total)
    };
    if let Some(v) = values.get(&None) {
        add(&mut total, v);
        lines.push(Line::amounts(RowKind::Data, 0, "No name", v.clone()));
    }
    lines.push(Line::amounts(RowKind::Total, 0, "TOTAL", total));
    let mut columns = vec![Column::new("label", "", ColumnKind::Text)];
    columns.extend(
        cols.iter()
            .enumerate()
            .map(|(i, c)| Column::new(format!("c{i}"), c.label.clone(), ColumnKind::Money)),
    );
    Ok(r.finish(r.range_line(), columns, money_rows(lines, width), notes))
}

pub(crate) fn by_customer_summary(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    name_summary(
        r,
        income_accounts(r),
        Some(&SALES_TYPES),
        |m| -m,
        |e, te| ctx.job_of(e, te),
        ReportId::SalesByCustomerDetail,
        true,
    )
}

pub(crate) fn expenses_by_vendor_summary(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    name_summary(
        r,
        expense_accounts(r),
        None,
        |m| m,
        |e, te| ctx.vendor_of(e, te).or(te),
        ReportId::ExpensesByVendorDetail,
        false,
    )
}

/// The quantity and rate typed on each form line behind `facts` (accrual lines
/// only; amounts recognised from a payment have no quantity of their own).
fn line_info(
    r: &Run<'_>,
    facts: &[Fact],
) -> Result<HashMap<LineId, (Option<Decimal>, Option<Decimal>)>> {
    let ids: Vec<TxnId> = facts
        .iter()
        .filter(|f| f.origin.is_none() && f.line_id.is_some())
        .map(|f| f.txn_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let mut out = HashMap::new();
    for chunk in ids.chunks(500) {
        let mut q = Sql::default();
        let list = q.bind_list(chunk.iter().map(ToString::to_string));
        q.text = format!("SELECT doc FROM txn WHERE id IN {list}");
        let mut stmt = r.c.prepare(&q.text)?;
        let mut rows = stmt.query(q.params())?;
        while let Some(row) = rows.next()? {
            let doc: String = row.get(0)?;
            let txn: Txn = lb_store::codec::from_doc(&doc)?;
            let sign = if txn.txn_type == TxnType::CreditMemo {
                -Decimal::ONE
            } else {
                Decimal::ONE
            };
            for l in txn.lines {
                out.insert(l.id, (l.quantity.map(|q| q * sign), l.rate));
            }
        }
    }
    Ok(out)
}

fn decimal_text(d: Decimal) -> String {
    d.normalize().to_string()
}

/// Detail by name: lines grouped by `key`, with quantity and price for sales.
#[allow(clippy::too_many_arguments)]
fn name_detail(
    r: &Run<'_>,
    accounts: HashSet<AccountId>,
    origin: Option<&[TxnType]>,
    natural: impl Fn(Money) -> Money,
    key: impl Fn(&Fact) -> Option<EntityId>,
    with_qty: bool,
) -> Result<Report> {
    let ctx = r.ctx;
    let base_cols: Vec<DCol> = if with_qty {
        vec![
            DCol::Type,
            DCol::Date,
            DCol::Num,
            DCol::Memo,
            DCol::Name,
            DCol::Item,
        ]
    } else {
        vec![
            DCol::Type,
            DCol::Date,
            DCol::Num,
            DCol::Memo,
            DCol::Account,
            DCol::Clr,
            DCol::Split,
        ]
    };
    let filter = r.filter.clone().and_accounts(accounts);
    let facts: Vec<Fact> = r
        .ledger()
        .facts_with(&filter, r.range)?
        .into_iter()
        .filter(|f| origin.is_none_or(|o| o.contains(&f.origin_type)))
        .collect();
    let splits = Splits::load(r, &facts)?;
    let info = if with_qty {
        line_info(r, &facts)?
    } else {
        HashMap::new()
    };
    let mut columns = super::detail::columns(&base_cols);
    if with_qty {
        columns.push(Column::new("qty", "Qty", ColumnKind::Number));
        columns.push(Column::new("price", "Sales Price", ColumnKind::Number));
    }
    columns.push(Column::new("amount", "Amount", ColumnKind::Money));
    columns.push(Column::new("balance", "Balance", ColumnKind::Money));
    let width = columns.len();
    let mut groups: HashMap<Option<EntityId>, Vec<Fact>> = HashMap::new();
    for f in facts {
        groups.entry(key(&f)).or_default().push(f);
    }
    let mut keys: Vec<Option<EntityId>> = groups.keys().copied().collect();
    keys.sort_by_key(|e| {
        (
            e.is_none(),
            ctx.entity_name(*e).unwrap_or_default().to_lowercase(),
        )
    });
    let amount_row = |kind, depth, label: String, amount: Money, balance: Money| {
        let mut cells = vec![Cell::text(label)];
        cells.extend(std::iter::repeat_n(Cell::EMPTY, width - 3));
        cells.push(Cell::money(amount));
        cells.push(Cell::money(balance));
        Row::new(kind, depth, cells)
    };
    let mut rows = Vec::new();
    let mut grand = Money::ZERO;
    for k in keys {
        let mut fs = groups.remove(&k).unwrap_or_default();
        sort_facts(r, &mut fs);
        let name = ctx.entity_name(k).unwrap_or_else(|| "No name".to_owned());
        rows.push(
            label_row(name.clone(), width, RowKind::Header, 0)
                .drill(k.map(|entity_id| Drill::Name { entity_id })),
        );
        let mut bal = Money::ZERO;
        for f in &fs {
            let a = natural(f.amount);
            bal += a;
            let mut cells = fact_cells(r, &splits, f, &base_cols, a, None, None);
            if with_qty {
                let (qty, rate) = f
                    .line_id
                    .filter(|_| f.origin.is_none())
                    .and_then(|l| info.get(&l).copied())
                    .unwrap_or((None, None));
                cells.push(qty.map_or(Cell::EMPTY, |q| Cell::number(decimal_text(q))));
                cells.push(rate.map_or(Cell::EMPTY, |p| Cell::number(decimal_text(p))));
            }
            cells.push(Cell::money(a));
            cells.push(Cell::money(bal));
            rows.push(Row::new(RowKind::Data, 1, cells).drill(Run::txn_drill(f.txn_id)));
        }
        rows.push(amount_row(
            RowKind::Subtotal,
            0,
            format!("Total {name}"),
            bal,
            bal,
        ));
        grand += bal;
    }
    rows.push(amount_row(RowKind::Total, 0, "TOTAL".into(), grand, grand));
    Ok(r.finish(r.range_line(), columns, rows, Vec::new()))
}

pub(crate) fn by_customer_detail(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    name_detail(
        r,
        income_accounts(r),
        Some(&SALES_TYPES),
        |m| -m,
        |f| ctx.job_of(f.entity_id, f.txn_entity_id),
        true,
    )
}

pub(crate) fn expenses_by_vendor_detail(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    name_detail(
        r,
        expense_accounts(r),
        None,
        |m| m,
        |f| {
            ctx.vendor_of(f.entity_id, f.txn_entity_id)
                .or(f.txn_entity_id)
        },
        false,
    )
}

/// Item type headings in the order QBD lists them.
const ITEM_TYPE_ORDER: [ItemType; 12] = [
    ItemType::Service,
    ItemType::Inventory,
    ItemType::InventoryAssembly,
    ItemType::NonInventory,
    ItemType::OtherCharge,
    ItemType::Discount,
    ItemType::Group,
    ItemType::Subtotal,
    ItemType::Payment,
    ItemType::SalesTax,
    ItemType::SalesTaxGroup,
    ItemType::FixedAsset,
];

/// Items present, grouped by type in QBD's order, each group in name order;
/// `None` (lines without an item) last.
fn items_by_type(
    r: &Run<'_>,
    items: impl Iterator<Item = Option<ItemId>>,
) -> Vec<(Option<ItemType>, Vec<Option<ItemId>>)> {
    let ctx = r.ctx;
    let mut by_type: HashMap<Option<ItemType>, Vec<Option<ItemId>>> = HashMap::new();
    for i in items.collect::<HashSet<_>>() {
        by_type
            .entry(i.and_then(|i| ctx.item_type(i)))
            .or_default()
            .push(i);
    }
    let mut out = Vec::new();
    for t in ITEM_TYPE_ORDER.into_iter().map(Some).chain([None]) {
        if let Some(mut v) = by_type.remove(&t) {
            v.sort_by_key(|i| {
                i.and_then(|i| ctx.item(i))
                    .map(|i| i.full_name.to_lowercase())
                    .unwrap_or_default()
            });
            out.push((t, v));
        }
    }
    out
}

fn item_label(r: &Run<'_>, i: Option<ItemId>) -> String {
    i.and_then(|i| r.ctx.item(i))
        .map_or_else(|| "Other (no item)".to_owned(), |i| i.full_name.clone())
}

fn type_label(t: Option<ItemType>) -> String {
    t.map_or_else(|| "Other".to_owned(), |t| t.label().to_owned())
}

/// `amount / qty` to two places, as text.
fn per_unit(amount: Money, qty: Decimal) -> Option<String> {
    if qty.is_zero() {
        return None;
    }
    let v = (amount.to_decimal() / qty)
        .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    Some(format!("{v:.2}"))
}

pub(crate) fn by_item_summary(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let ledger = r.ledger();
    let income = r.filter.clone().and_accounts(income_accounts(r));
    let dims = Dims {
        item: true,
        origin_type: true,
        ..Dims::default()
    };
    let mut amount: HashMap<Option<ItemId>, Money> = HashMap::new();
    for (k, v) in ledger.aggregate_with(&income, r.range, Gran::None, dims)? {
        if k.origin_type.is_some_and(|t| SALES_TYPES.contains(&t)) {
            *amount.entry(k.item_id).or_default() += -v;
        }
    }
    let cost_filter = r.filter.clone().and_accounts(expense_accounts(r));
    let mut cogs: HashMap<Option<ItemId>, Money> = HashMap::new();
    for (k, v) in ledger.aggregate_with(&cost_filter, r.range, Gran::None, dims)? {
        if k.origin_type.is_some_and(|t| SALES_TYPES.contains(&t))
            && ctx.item_is_inventory(k.item_id)
        {
            *cogs.entry(k.item_id).or_default() += v;
        }
    }
    let qty = ledger.item_quantities(&income, r.range, &SALES_TYPES)?;
    amount.retain(|i, v| !v.is_zero() || cogs.contains_key(i));
    let total_sales: Money = amount.values().copied().sum();
    let columns = vec![
        Column::new("label", "", ColumnKind::Text),
        Column::new("qty", "Qty", ColumnKind::Number),
        Column::new("amount", "Amount", ColumnKind::Money),
        Column::new("pct", "% of Sales", ColumnKind::Percent),
        Column::new("avg_price", "Avg Price", ColumnKind::Number),
        Column::new("cogs", "COGS", ColumnKind::Money),
        Column::new("avg_cogs", "Avg COGS", ColumnKind::Number),
        Column::new("margin", "Gross Margin", ColumnKind::Money),
        Column::new("margin_pct", "Gross Margin %", ColumnKind::Percent),
    ];
    let width = columns.len();
    let cells = |label: String, q: Option<Decimal>, a: Money, c: Money| -> Vec<Cell> {
        vec![
            Cell::text(label),
            q.map_or(Cell::EMPTY, |q| Cell::number(decimal_text(q))),
            Cell::money(a),
            Cell::percent(percent(a, total_sales)),
            q.and_then(|q| per_unit(a, q))
                .map_or(Cell::EMPTY, Cell::number),
            Cell::money(c),
            q.and_then(|q| per_unit(c, q))
                .map_or(Cell::EMPTY, Cell::number),
            Cell::money(a - c),
            Cell::percent(percent(a - c, a)),
        ]
    };
    let mut rows = Vec::new();
    let (mut ta, mut tc) = (Money::ZERO, Money::ZERO);
    for (t, items) in items_by_type(r, amount.keys().copied()) {
        let label = type_label(t);
        rows.push(label_row(label.clone(), width, RowKind::Header, 0));
        let (mut ga, mut gc) = (Money::ZERO, Money::ZERO);
        let mut gq = Decimal::ZERO;
        for i in items {
            let a = amount.get(&i).copied().unwrap_or_default();
            let c = cogs.get(&i).copied().unwrap_or_default();
            let q = qty.get(&i).copied();
            ga += a;
            gc += c;
            gq += q.unwrap_or_default();
            let drill = r.drill_to(ReportId::SalesByItemDetail, r.range, |f| {
                if let Some(i) = i {
                    f.items = vec![i];
                }
            });
            rows.push(Row::new(RowKind::Data, 1, cells(item_label(r, i), q, a, c)).drill(drill));
        }
        rows.push(Row::new(
            RowKind::Subtotal,
            0,
            cells(
                format!("Total {label}"),
                Some(gq).filter(|q| !q.is_zero()),
                ga,
                gc,
            ),
        ));
        ta += ga;
        tc += gc;
    }
    rows.push(Row::new(
        RowKind::Total,
        0,
        cells("TOTAL".into(), None, ta, tc),
    ));
    Ok(r.finish(r.range_line(), columns, rows, Vec::new()))
}

pub(crate) fn by_item_detail(r: &Run<'_>) -> Result<Report> {
    let base_cols = [DCol::Type, DCol::Date, DCol::Num, DCol::Memo, DCol::Name];
    let filter = r.filter.clone().and_accounts(income_accounts(r));
    let facts: Vec<Fact> = r
        .ledger()
        .facts_with(&filter, r.range)?
        .into_iter()
        .filter(|f| SALES_TYPES.contains(&f.origin_type))
        .collect();
    let splits = Splits::load(r, &facts)?;
    let info = line_info(r, &facts)?;
    let mut columns = super::detail::columns(&base_cols);
    columns.push(Column::new("qty", "Qty", ColumnKind::Number));
    columns.push(Column::new("price", "Sales Price", ColumnKind::Number));
    columns.push(Column::new("amount", "Amount", ColumnKind::Money));
    columns.push(Column::new("balance", "Balance", ColumnKind::Money));
    let width = columns.len();
    let mut by_item: HashMap<Option<ItemId>, Vec<Fact>> = HashMap::new();
    for f in facts {
        by_item.entry(f.item_id).or_default().push(f);
    }
    let amount_row = |kind, depth, label: String, amount: Money| {
        let mut cells = vec![Cell::text(label)];
        cells.extend(std::iter::repeat_n(Cell::EMPTY, width - 3));
        cells.push(Cell::money(amount));
        cells.push(Cell::money(amount));
        Row::new(kind, depth, cells)
    };
    let mut rows = Vec::new();
    let mut grand = Money::ZERO;
    for (t, items) in items_by_type(r, by_item.keys().copied()) {
        let tl = type_label(t);
        rows.push(label_row(tl.clone(), width, RowKind::Header, 0));
        let mut type_total = Money::ZERO;
        for i in items {
            let mut fs = by_item.remove(&i).unwrap_or_default();
            sort_facts(r, &mut fs);
            let il = item_label(r, i);
            rows.push(label_row(il.clone(), width, RowKind::Header, 1));
            let mut bal = Money::ZERO;
            for f in &fs {
                let a = -f.amount;
                bal += a;
                let (qty, rate) = f
                    .line_id
                    .filter(|_| f.origin.is_none())
                    .and_then(|l| info.get(&l).copied())
                    .unwrap_or((None, None));
                let mut cells = fact_cells(r, &splits, f, &base_cols, a, None, None);
                cells.push(qty.map_or(Cell::EMPTY, |q| Cell::number(decimal_text(q))));
                cells.push(rate.map_or(Cell::EMPTY, |p| Cell::number(decimal_text(p))));
                cells.push(Cell::money(a));
                cells.push(Cell::money(bal));
                rows.push(Row::new(RowKind::Data, 2, cells).drill(Run::txn_drill(f.txn_id)));
            }
            rows.push(amount_row(RowKind::Subtotal, 1, format!("Total {il}"), bal));
            type_total += bal;
        }
        rows.push(amount_row(
            RowKind::Subtotal,
            0,
            format!("Total {tl}"),
            type_total,
        ));
        grand += type_total;
    }
    rows.push(amount_row(RowKind::Total, 0, "TOTAL".into(), grand));
    Ok(r.finish(r.range_line(), columns, rows, Vec::new()))
}

/// Cost (expenses, debit-positive) and revenue (income, credit-positive) per
/// customer:job, and per item/account within it.
fn job_amounts(
    r: &Run<'_>,
) -> Result<HashMap<EntityId, HashMap<(Option<ItemId>, AccountId), (Money, Money)>>> {
    let ctx = r.ctx;
    let pl: HashSet<AccountId> = accounts_where(r, AccountType::is_income_statement);
    let filter: Filter = r.filter.clone().and_accounts(pl);
    let agg = r.ledger().aggregate_with(
        &filter,
        r.range,
        Gran::None,
        Dims {
            names: true,
            item: true,
            ..Dims::default()
        },
    )?;
    let mut out: HashMap<EntityId, HashMap<(Option<ItemId>, AccountId), (Money, Money)>> =
        HashMap::new();
    for (k, v) in agg {
        let Some(job) = ctx.job_of(k.entity_id, k.txn_entity_id) else {
            continue;
        };
        let income = ctx
            .account_type(k.account_id)
            .is_some_and(|t| t.class() == AccountClass::Income);
        let e = out
            .entry(job)
            .or_default()
            .entry((k.item_id, k.account_id))
            .or_default();
        if income {
            e.1 += -v;
        } else {
            e.0 += v;
        }
    }
    Ok(out)
}

fn profit_columns() -> Vec<Column> {
    vec![
        Column::new("label", "", ColumnKind::Text),
        Column::new("cost", "Act. Cost", ColumnKind::Money),
        Column::new("revenue", "Act. Revenue", ColumnKind::Money),
        Column::new("diff", "($) Diff.", ColumnKind::Money),
    ]
}

pub(crate) fn job_profitability_summary(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let amounts = job_amounts(r)?;
    let mut per_job: HashMap<EntityId, Vec<Money>> = HashMap::new();
    for (job, m) in &amounts {
        let (c, rev) = m.values().fold((Money::ZERO, Money::ZERO), |acc, x| {
            (acc.0 + x.0, acc.1 + x.1)
        });
        per_job.insert(*job, vec![c, rev, rev - c]);
    }
    per_job.retain(|_, v| v.iter().any(|m| !m.is_zero()));
    let named: HashSet<EntityId> = per_job.keys().copied().collect();
    let get = |e: EntityId| {
        per_job
            .get(&e)
            .cloned()
            .unwrap_or_else(|| vec![Money::ZERO; 3])
    };
    let drill = |e: EntityId, _: bool, _: Option<usize>| {
        r.drill_to(ReportId::JobProfitabilityDetail, r.range, |f| {
            f.names = vec![e];
        })
    };
    let (mut lines, total) = entity_tree(ctx, &named, &get, 3, r.req.show_zero_rows, 0, &drill);
    lines.push(Line::amounts(RowKind::Total, 0, "TOTAL", total));
    Ok(r.finish(
        r.range_line(),
        profit_columns(),
        money_rows(lines, 3),
        Vec::new(),
    ))
}

pub(crate) fn job_profitability_detail(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let amounts = job_amounts(r)?;
    let mut jobs: Vec<EntityId> = amounts.keys().copied().collect();
    jobs.sort_by_key(|e| ctx.entity_name(Some(*e)).unwrap_or_default().to_lowercase());
    let row = |kind, depth, label: String, c: Money, rev: Money| {
        Row::new(
            kind,
            depth,
            vec![
                Cell::text(label),
                Cell::money(c),
                Cell::money(rev),
                Cell::money(rev - c),
            ],
        )
    };
    let mut rows = Vec::new();
    let (mut tc, mut tr) = (Money::ZERO, Money::ZERO);
    for job in jobs {
        let m = &amounts[&job];
        let name = ctx.entity_name(Some(job)).unwrap_or_default();
        rows.push(
            label_row(name.clone(), 4, RowKind::Header, 0)
                .drill(Some(Drill::Name { entity_id: job })),
        );
        // Items first (by name), then lines without an item, by account.
        let mut by_item: HashMap<Option<ItemId>, (Money, Money)> = HashMap::new();
        let mut by_account: HashMap<AccountId, (Money, Money)> = HashMap::new();
        for ((item, account), (c, rev)) in m {
            if item.is_some() {
                let e = by_item.entry(*item).or_default();
                e.0 += *c;
                e.1 += *rev;
            } else {
                let e = by_account.entry(*account).or_default();
                e.0 += *c;
                e.1 += *rev;
            }
        }
        let (mut jc, mut jr) = (Money::ZERO, Money::ZERO);
        let mut items: Vec<Option<ItemId>> = by_item.keys().copied().collect();
        items.sort_by_key(|i| item_label(r, *i).to_lowercase());
        for i in items {
            let (c, rev) = by_item[&i];
            jc += c;
            jr += rev;
            rows.push(row(RowKind::Data, 1, item_label(r, i), c, rev));
        }
        let mut accts: Vec<AccountId> = by_account.keys().copied().collect();
        accts.sort_by_key(|a| ctx.account_order(*a));
        for a in accts {
            let (c, rev) = by_account[&a];
            jc += c;
            jr += rev;
            rows.push(
                row(RowKind::Data, 1, ctx.account_full_label(a), c, rev).drill(r.drill_to(
                    ReportId::TransactionDetailByAccount,
                    r.range,
                    |f| {
                        f.accounts = vec![a];
                        f.names = vec![job];
                    },
                )),
            );
        }
        rows.push(row(RowKind::Subtotal, 0, format!("Total {name}"), jc, jr));
        tc += jc;
        tr += jr;
    }
    rows.push(row(RowKind::Total, 0, "TOTAL".into(), tc, tr));
    Ok(r.finish(r.range_line(), profit_columns(), rows, Vec::new()))
}

pub(crate) fn inventory_valuation(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let as_of: Date = r.as_of().unwrap_or(ctx.today);
    let mut qty: HashMap<ItemId, Decimal> = HashMap::new();
    let mut value: HashMap<ItemId, Money> = HashMap::new();
    {
        let mut stmt = r.c.prepare(
            "SELECT p.item_id, p.quantity, COUNT(*), SUM(p.amount)
             FROM posting p JOIN item i ON i.id = p.item_id
             WHERE i.item_type IN ('Inventory', 'InventoryAssembly') AND p.quantity IS NOT NULL
               AND p.date <= ?1
             GROUP BY p.item_id, p.quantity",
        )?;
        let mut rows = stmt.query([SqlDate(as_of)])?;
        while let Some(row) = rows.next()? {
            let item: ItemId = lb_store::codec::parse_id(&row.get::<_, String>(0)?)?;
            let q: String = row.get(1)?;
            let n: i64 = row.get(2)?;
            let v: i64 = row.get(3)?;
            *qty.entry(item).or_default() +=
                q.parse::<Decimal>().unwrap_or_default() * Decimal::from(n);
            *value.entry(item).or_default() += Money(v);
        }
    }
    let filter = &r.filter;
    let mut items: Vec<ItemId> = ctx
        .items
        .values()
        .filter(|i| i.item_type.is_inventory())
        .filter(|i| filter.items.as_ref().is_none_or(|s| s.contains(&i.id)))
        .filter(|i| {
            r.req.show_zero_rows && i.is_active
                || qty.get(&i.id).is_some_and(|q| !q.is_zero())
                || value.get(&i.id).is_some_and(|v| !v.is_zero())
        })
        .map(|i| i.id)
        .collect();
    items.sort_by_key(|i| ctx.item(*i).map(|x| x.full_name.to_lowercase()));
    let total_value: Money = items.iter().filter_map(|i| value.get(i)).copied().sum();
    let retail = |i: ItemId| -> Money {
        let q = qty.get(&i).copied().unwrap_or_default();
        ctx.item(i)
            .and_then(|x| x.sales_price)
            .and_then(|p| Money::extend(q, p))
            .unwrap_or_default()
    };
    let total_retail: Money = items.iter().map(|i| retail(*i)).sum();
    let columns = vec![
        Column::new("label", "", ColumnKind::Text),
        Column::new("on_hand", "On Hand", ColumnKind::Number),
        Column::new("avg_cost", "Avg Cost", ColumnKind::Number),
        Column::new("value", "Asset Value", ColumnKind::Money),
        Column::new("pct_value", "% of Tot Asset", ColumnKind::Percent),
        Column::new("price", "Sales Price", ColumnKind::Number),
        Column::new("retail", "Retail Value", ColumnKind::Money),
        Column::new("pct_retail", "% of Tot Retail", ColumnKind::Percent),
    ];
    let mut rows = Vec::new();
    for i in items {
        let item = ctx.item(i);
        let q = qty.get(&i).copied().unwrap_or_default();
        let v = value.get(&i).copied().unwrap_or_default();
        let rv = retail(i);
        rows.push(
            Row::new(
                RowKind::Data,
                0,
                vec![
                    Cell::text(item.map(|x| x.full_name.clone()).unwrap_or_default()),
                    Cell::number(decimal_text(q)),
                    per_unit(v, q).map_or(Cell::EMPTY, Cell::number),
                    Cell::money(v),
                    Cell::percent(percent(v, total_value)),
                    item.and_then(|x| x.sales_price)
                        .map_or(Cell::EMPTY, |p| Cell::number(decimal_text(p))),
                    Cell::money(rv),
                    Cell::percent(percent(rv, total_retail)),
                ],
            )
            .drill(r.drill_to(
                ReportId::TransactionDetailByAccount,
                DateRange {
                    from: None,
                    to: Some(as_of),
                },
                |f| f.items = vec![i],
            )),
        );
    }
    rows.push(Row::new(
        RowKind::Total,
        0,
        vec![
            Cell::text("TOTAL"),
            Cell::EMPTY,
            Cell::EMPTY,
            Cell::money(total_value),
            Cell::percent(percent(total_value, total_value)),
            Cell::EMPTY,
            Cell::money(total_retail),
            Cell::percent(percent(total_retail, total_retail)),
        ],
    ));
    Ok(r.finish(r.as_of_line(), columns, rows, Vec::new()))
}
