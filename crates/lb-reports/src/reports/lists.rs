//! List reports: the chart of accounts, customer and vendor contact lists, and
//! the item list, as tables (active records only).

use lb_core::{AccountType, DateRange, EntityKind, Money};

use crate::model::{Cell, Column, ColumnKind, Drill, Report, Row, RowKind};
use crate::{Result, Run};

fn table(ids: &[(&str, &str, ColumnKind)]) -> Vec<Column> {
    ids.iter()
        .map(|(id, label, kind)| Column::new(*id, *label, *kind))
        .collect()
}

pub(crate) fn account_listing(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let balances = r.accrual_ledger().sums(DateRange {
        from: None,
        to: Some(ctx.today),
    })?;
    let columns = table(&[
        ("account", "Account", ColumnKind::Text),
        ("type", "Type", ColumnKind::Text),
        ("balance", "Balance Total", ColumnKind::Money),
        ("description", "Description", ColumnKind::Text),
        ("tax_line", "Tax Line", ColumnKind::Text),
    ]);
    let mut rows = Vec::new();
    for a in ctx.accounts.iter().filter(|a| a.is_active) {
        // Like QBD, income-statement accounts show no balance here.
        let balance = if a.account_type.is_balance_sheet() {
            // Subaccounts roll into their parent's total.
            let total: Money = ctx
                .account_descendants(&[a.id])
                .iter()
                .filter_map(|d| balances.get(d))
                .copied()
                .sum();
            Cell::money(a.account_type.natural(total))
        } else {
            Cell::EMPTY
        };
        let drill = r.drill_to(
            crate::request::ReportId::AccountQuickReport,
            DateRange::ALL,
            |f| f.accounts = vec![a.id],
        );
        rows.push(
            Row::new(
                RowKind::Data,
                0,
                vec![
                    Cell::text(ctx.account_full_label(a.id)),
                    Cell::text(if a.account_type == AccountType::NonPosting {
                        "Non-Posting"
                    } else {
                        a.account_type.label()
                    }),
                    balance,
                    Cell::opt_text(a.description.clone()),
                    Cell::opt_text(a.tax_line.clone()),
                ],
            )
            .drill(drill),
        );
    }
    Ok(r.finish(
        crate::dates::long_date(ctx.today),
        columns,
        rows,
        Vec::new(),
    ))
}

pub(crate) fn contact_list(r: &Run<'_>, kind: EntityKind) -> Report {
    let ctx = r.ctx;
    let mut names: Vec<&lb_core::Entity> = ctx
        .entities
        .values()
        .filter(|e| e.kind == kind && e.is_active)
        .collect();
    names.sort_by_key(|e| e.full_name.to_lowercase());
    let vendor = kind == EntityKind::Vendor;
    let mut spec = vec![(
        "name",
        if vendor { "Vendor" } else { "Customer" },
        ColumnKind::Text,
    )];
    if vendor {
        spec.push(("account_no", "Account No.", ColumnKind::Text));
    }
    spec.extend([
        (
            "address",
            if vendor { "Bill from" } else { "Bill to" },
            ColumnKind::Text,
        ),
        ("contact", "Contact", ColumnKind::Text),
        ("phone", "Phone", ColumnKind::Text),
        ("fax", "Fax", ColumnKind::Text),
        ("email", "Email", ColumnKind::Text),
    ]);
    if vendor {
        spec.push(("is_1099", "1099", ColumnKind::Text));
    }
    let columns = table(&spec);
    let rows = names
        .into_iter()
        .map(|e| {
            let mut cells = vec![Cell::text(e.full_name.clone())];
            if vendor {
                cells.push(Cell::opt_text(e.account_number.clone()));
            }
            let address = e.bill_address.block().join(", ");
            cells.push(if address.is_empty() {
                Cell::EMPTY
            } else {
                Cell::text(address)
            });
            cells.push(Cell::opt_text(e.contact.clone()));
            cells.push(Cell::opt_text(e.phone.clone()));
            cells.push(Cell::opt_text(e.fax.clone()));
            cells.push(Cell::opt_text(e.email.clone()));
            if vendor {
                cells.push(Cell::text(if e.is_1099 { "Yes" } else { "" }));
            }
            Row::new(RowKind::Data, 0, cells).drill(Some(Drill::Name { entity_id: e.id }))
        })
        .collect();
    r.finish(
        crate::dates::long_date(ctx.today),
        columns,
        rows,
        Vec::new(),
    )
}

pub(crate) fn item_listing(r: &Run<'_>) -> Report {
    let ctx = r.ctx;
    let mut items: Vec<&lb_core::Item> = ctx.items.values().filter(|i| i.is_active).collect();
    items.sort_by_key(|i| i.full_name.to_lowercase());
    let columns = table(&[
        ("item", "Item", ColumnKind::Text),
        ("description", "Description", ColumnKind::Text),
        ("type", "Type", ColumnKind::Text),
        ("account", "Account", ColumnKind::Text),
        ("cogs_account", "COGS Account", ColumnKind::Text),
        ("price", "Price", ColumnKind::Number),
        ("cost", "Cost", ColumnKind::Number),
        ("on_hand", "Quantity On Hand", ColumnKind::Number),
    ]);
    let dec = |d: Option<rust_decimal::Decimal>| {
        d.map_or(Cell::EMPTY, |d| Cell::number(d.normalize().to_string()))
    };
    let rows = items
        .into_iter()
        .map(|i| {
            Row::new(
                RowKind::Data,
                0,
                vec![
                    Cell::text(i.full_name.clone()),
                    Cell::opt_text(i.sales_description.clone()),
                    Cell::text(i.item_type.label()),
                    Cell::opt_text(i.income_account_id.map(|a| ctx.account_full_label(a))),
                    Cell::opt_text(i.expense_account_id.map(|a| ctx.account_full_label(a))),
                    dec(i.sales_price),
                    dec(i.purchase_cost),
                    if i.item_type.is_inventory() {
                        dec(Some(i.quantity_on_hand))
                    } else {
                        Cell::EMPTY
                    },
                ],
            )
        })
        .collect();
    r.finish(
        crate::dates::long_date(ctx.today),
        columns,
        rows,
        Vec::new(),
    )
}
