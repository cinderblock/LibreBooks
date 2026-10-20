//! Sales Tax Liability and the 1099 Summary.
//!
//! **Sales Tax Liability**, by agency and tax item: the sales the tax was charged
//! on (total, non-taxable, taxable, read from the forms the way the posting
//! rules compute the taxable base), the rate, the tax collected in the period,
//! and what is owed as of the period's end (collections less payments). Its
//! basis defaults to the company's sales-tax preference ("owe sales tax as of
//! invoice date / upon receipt of payment"), not the reports preference.
//!
//! **1099 Summary**: payments to vendors marked as 1099 vendors, by the expense
//! account they paid for, always on the cash basis (a 1099 reports what was
//! paid). Payments by credit card are left out, as the IRS has the card
//! processor report those (QBD does the same).

use std::collections::{BTreeMap, HashMap, HashSet};

use lb_core::lists::SalesTaxCode;
use lb_core::{
    AccountClass, AccountId, AccountRole, DateRange, EntityId, ItemId, ItemType, LineKind, Money,
    SalesTaxCodeId, Txn, TxnType,
};

use super::sales::SALES_TYPES;
use crate::filter::Sql;
use crate::ledger::{Dims, Gran, Ledger};
use crate::model::{Cell, Column, ColumnKind, Report, Row, RowKind};
use crate::request::{Basis, ReportId};
use crate::{Result, Run};

#[derive(Clone, Copy, Debug, Default)]
struct TaxRow {
    total_sales: Money,
    taxable: Money,
    collected: Money,
    payable: Money,
}

pub(crate) fn sales_tax_liability(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let basis = r
        .req
        .basis
        .unwrap_or(if ctx.info.preferences.sales_tax_on_cash_basis {
            Basis::Cash
        } else {
            Basis::Accrual
        });
    let to = r.range.to.unwrap_or(ctx.today);
    let mut notes = Vec::new();
    let Some(stp) = ctx.role(AccountRole::SalesTaxPayable) else {
        let report = r.finish(
            r.range_line(),
            tax_columns(to),
            Vec::new(),
            vec!["No sales tax has been charged yet.".into()],
        );
        return Ok(report);
    };
    let stp_accounts = ctx.account_descendants(&[stp]);
    let ledger = Ledger::new(r.c, ctx, basis, r.filter.clone());
    let filter = r.filter.clone().and_accounts(stp_accounts);
    let dims = Dims {
        names: true,
        item: true,
        origin_type: true,
        ..Dims::default()
    };
    let mut rows_by: BTreeMap<(Option<EntityId>, Option<ItemId>), TaxRow> = BTreeMap::new();
    for (k, v) in ledger.aggregate_with(&filter, r.range, Gran::None, dims)? {
        if k.origin_type.is_some_and(|t| SALES_TYPES.contains(&t)) {
            rows_by
                .entry((k.entity_id, k.item_id))
                .or_default()
                .collected += -v;
        }
    }
    let as_of = DateRange {
        from: None,
        to: Some(to),
    };
    for (k, v) in ledger.aggregate_with(&filter, as_of, Gran::None, dims)? {
        rows_by.entry((k.entity_id, k.item_id)).or_default().payable += -v;
    }
    // The sales the tax was charged on, from the forms.
    let codes: HashMap<SalesTaxCodeId, bool> = lb_store::lists::list::<SalesTaxCode>(r.c, true)?
        .into_iter()
        .map(|c| (c.id, c.is_taxable))
        .collect();
    let taxable_code =
        |c: Option<SalesTaxCodeId>| c.is_some_and(|c| codes.get(&c).copied().unwrap_or(false));
    let mut q = Sql::default();
    let list = q.bind_list(SALES_TYPES.iter().map(|t| t.as_str().to_owned()));
    let mut sql = format!(
        "SELECT doc FROM txn WHERE txn_type IN {list} AND is_void = 0 AND is_pending = 0
           AND json_extract(doc, '$.sales_tax_item_id') IS NOT NULL"
    );
    if let Some(f) = r.range.from {
        let p = q.bind(f.to_string());
        sql.push_str(&format!(" AND date >= {p}"));
    }
    let p = q.bind(to.to_string());
    sql.push_str(&format!(" AND date <= {p}"));
    q.text = sql;
    {
        let mut stmt = r.c.prepare(&q.text)?;
        let mut rows = stmt.query(q.params())?;
        while let Some(row) = rows.next()? {
            let doc: String = row.get(0)?;
            let txn: Txn = lb_store::codec::from_doc(&doc)?;
            let Some(tax_item) = txn.sales_tax_item_id else {
                continue;
            };
            let customer_taxable = txn
                .customer_tax_code_id
                .is_none_or(|c| taxable_code(Some(c)));
            let (mut total, mut base) = (Money::ZERO, Money::ZERO);
            for l in &txn.lines {
                let line_taxable = match l.kind {
                    LineKind::Account => taxable_code(l.sales_tax_code_id),
                    LineKind::Item => {
                        let Some(item) = l.item_id.and_then(|i| ctx.item(i)) else {
                            continue;
                        };
                        if matches!(
                            item.item_type,
                            ItemType::Subtotal
                                | ItemType::Group
                                | ItemType::Payment
                                | ItemType::SalesTax
                                | ItemType::SalesTaxGroup
                        ) {
                            continue;
                        }
                        taxable_code(l.sales_tax_code_id.or(item.sales_tax_code_id))
                    }
                    _ => continue,
                };
                total += l.amount;
                if line_taxable {
                    base += l.amount;
                }
            }
            if !customer_taxable {
                base = Money::ZERO;
            }
            if txn.txn_type == TxnType::CreditMemo {
                total = -total;
                base = -base;
            }
            let components: Vec<ItemId> = match ctx.item(tax_item) {
                Some(i) if i.item_type == ItemType::SalesTaxGroup => {
                    i.components.iter().map(|c| c.item_id).collect()
                }
                Some(_) => vec![tax_item],
                None => continue,
            };
            for comp in components {
                let agency = ctx.item(comp).and_then(|i| i.tax_agency_id);
                let e = rows_by.entry((agency, Some(comp))).or_default();
                e.total_sales += total;
                e.taxable += base;
            }
        }
    }
    if basis == Basis::Cash {
        notes.push("Sales columns are as invoiced; tax collected counts when paid.".to_owned());
    }
    let columns = tax_columns(to);
    let width = columns.len();
    let cells = |label: String, t: &TaxRow, rate: Option<String>| -> Vec<Cell> {
        vec![
            Cell::text(label),
            Cell::money(t.total_sales),
            Cell::money(t.total_sales - t.taxable),
            Cell::money(t.taxable),
            Cell::percent(rate),
            Cell::money(t.collected),
            Cell::money(t.payable),
        ]
    };
    let mut agencies: Vec<Option<EntityId>> = rows_by
        .keys()
        .map(|k| k.0)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    agencies.sort_by_key(|a| {
        (
            a.is_none(),
            ctx.entity_name(*a).unwrap_or_default().to_lowercase(),
        )
    });
    let mut rows = Vec::new();
    let mut grand = TaxRow::default();
    for agency in agencies {
        let name = ctx
            .entity_name(agency)
            .unwrap_or_else(|| "No agency".to_owned());
        let mut head = vec![Cell::EMPTY; width];
        head[0] = Cell::text(name.clone());
        rows.push(Row::new(RowKind::Header, 0, head));
        let mut items: Vec<(Option<ItemId>, TaxRow)> = rows_by
            .iter()
            .filter(|(k, _)| k.0 == agency)
            .map(|(k, v)| (k.1, *v))
            .collect();
        items.sort_by_key(|(i, _)| {
            (
                i.is_none(),
                i.and_then(|i| ctx.item(i))
                    .map(|x| x.full_name.to_lowercase()),
            )
        });
        let mut sum = TaxRow::default();
        for (item, t) in items {
            let it = item.and_then(|i| ctx.item(i));
            let label = it.map_or_else(
                || "Payments and adjustments".to_owned(),
                |i| i.full_name.clone(),
            );
            let rate = it
                .and_then(|i| i.tax_rate)
                .map(|d| d.normalize().to_string());
            sum.total_sales += t.total_sales;
            sum.taxable += t.taxable;
            sum.collected += t.collected;
            sum.payable += t.payable;
            let drill = r.drill_to(ReportId::TransactionDetailByAccount, r.range, |f| {
                f.accounts = vec![stp];
                if let Some(a) = agency {
                    f.names = vec![a];
                }
                if let Some(i) = item {
                    f.items = vec![i];
                }
            });
            rows.push(Row::new(RowKind::Data, 1, cells(label, &t, rate)).drill(drill));
        }
        rows.push(Row::new(
            RowKind::Subtotal,
            0,
            cells(format!("Total {name}"), &sum, None),
        ));
        grand.total_sales += sum.total_sales;
        grand.taxable += sum.taxable;
        grand.collected += sum.collected;
        grand.payable += sum.payable;
    }
    rows.push(Row::new(
        RowKind::Total,
        0,
        cells("TOTAL".into(), &grand, None),
    ));
    let mut report = r.finish(r.range_line(), columns, rows, notes);
    report.basis = Some(basis);
    Ok(report)
}

fn tax_columns(to: jiff::civil::Date) -> Vec<Column> {
    vec![
        Column::new("label", "", ColumnKind::Text),
        Column::new("total_sales", "Total Sales", ColumnKind::Money),
        Column::new("non_taxable", "Non-Taxable Sales", ColumnKind::Money),
        Column::new("taxable", "Taxable Sales", ColumnKind::Money),
        Column::new("rate", "Tax Rate", ColumnKind::Percent),
        Column::new("collected", "Tax Collected", ColumnKind::Money),
        Column::new(
            "payable",
            format!("Sales Tax Payable As of {}", crate::dates::short_date(to)),
            ColumnKind::Money,
        ),
    ]
}

/// Card payments: reported by the card processor, not on a 1099.
const CARD_TYPES: [TxnType; 3] = [
    TxnType::CreditCardCharge,
    TxnType::CreditCardCredit,
    TxnType::BillPaymentCreditCard,
];

pub(crate) fn vendor_1099_summary(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let expense: HashSet<AccountId> = ctx
        .accounts
        .iter()
        .filter(|a| a.account_type.class() == AccountClass::Expense)
        .map(|a| a.id)
        .collect();
    let ledger = Ledger::new(r.c, ctx, Basis::Cash, r.filter.clone());
    let filter = r.filter.clone().and_accounts(expense);
    let mut by: HashMap<(EntityId, AccountId), Money> = HashMap::new();
    for f in ledger.facts_with(&filter, r.range)? {
        if CARD_TYPES.contains(&f.txn_type) {
            continue;
        }
        let Some(v) = ctx.vendor_of(f.entity_id, f.txn_entity_id) else {
            continue;
        };
        if !ctx.entity(v).is_some_and(|e| e.is_1099) {
            continue;
        }
        *by.entry((v, f.account_id)).or_default() += f.amount;
    }
    by.retain(|_, m| !m.is_zero());
    let mut accounts: Vec<AccountId> = by
        .keys()
        .map(|k| k.1)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    accounts.sort_by_key(|a| ctx.account_order(*a));
    let mut vendors: Vec<EntityId> = by
        .keys()
        .map(|k| k.0)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    vendors.sort_by_key(|v| ctx.entity_name(Some(*v)).unwrap_or_default().to_lowercase());
    let mut columns = vec![Column::new("label", "", ColumnKind::Text)];
    for (i, a) in accounts.iter().enumerate() {
        columns.push(Column::new(
            format!("a{i}"),
            ctx.account_full_label(*a),
            ColumnKind::Money,
        ));
    }
    columns.push(Column::new("total", "TOTAL", ColumnKind::Money));
    let mut rows = Vec::new();
    let mut totals = vec![Money::ZERO; accounts.len() + 1];
    for v in vendors {
        let mut cells = vec![Cell::text(ctx.entity_name(Some(v)).unwrap_or_default())];
        let mut sum = Money::ZERO;
        for (i, a) in accounts.iter().enumerate() {
            let m = by.get(&(v, *a)).copied().unwrap_or_default();
            sum += m;
            totals[i] += m;
            let drill = r.drill_to(ReportId::TransactionDetailByAccount, r.range, |f| {
                f.accounts = vec![*a];
                f.names = vec![v];
            });
            cells.push(if m.is_zero() {
                Cell::EMPTY
            } else {
                Cell::money(m).drill(drill)
            });
        }
        let last = totals.len() - 1;
        totals[last] += sum;
        cells.push(Cell::money(sum));
        rows.push(
            Row::new(RowKind::Data, 0, cells)
                .drill(Some(crate::model::Drill::Name { entity_id: v })),
        );
    }
    let mut cells = vec![Cell::text("TOTAL")];
    cells.extend(totals.iter().map(|m| Cell::money(*m)));
    rows.push(Row::new(RowKind::Total, 0, cells));
    let mut report = r.finish(r.range_line(), columns, rows, Vec::new());
    report.basis = Some(Basis::Cash);
    Ok(report)
}
