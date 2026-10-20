//! Keeping inventory cost right when history changes.
//!
//! A sale posts its cost of goods at the item's average cost on its date. So a
//! purchase entered (or changed, or deleted) back in time changes the cost of
//! every later sale of that item, as QBD recalculates. After any save that moves
//! inventory, later sales of the same items are posted again in date order, but
//! only those whose postings actually change. Their edit sequence moves on and
//! the audit trail shows the re-posting, so nobody's open form saves over it.
//!
//! Each item's quantity on hand and average cost (shown on the item list) are
//! refreshed from the ledger at the same time.

use std::collections::BTreeSet;

use lb_core::{ItemId, Money, TxnId, post};
use rusqlite::{Connection, params};
use rust_decimal::Decimal;

use crate::codec::SqlDate;
use crate::context::{StoreContext, on_hand};
use crate::{Result, Writer, txns};

/// Items whose quantity a transaction's stored postings move.
pub fn items_in_postings(c: &Connection, txn: TxnId) -> Result<BTreeSet<ItemId>> {
    let mut stmt = c.prepare(
        "SELECT DISTINCT item_id FROM posting WHERE txn_id = ?1 AND item_id IS NOT NULL AND quantity IS NOT NULL",
    )?;
    let ids = stmt.query_map([txn.to_string()], |r| r.get::<_, String>(0))?;
    ids.map(|s| crate::codec::parse_id(&s?)).collect()
}

/// Re-post later sales of `items` dated on or after `from`, other than `origin`.
pub fn recost(
    w: &mut Writer<'_>,
    items: &BTreeSet<ItemId>,
    from: jiff::civil::Date,
    origin: TxnId,
) -> Result<usize> {
    if items.is_empty() {
        return Ok(0);
    }
    let list: Vec<String> = items.iter().map(ToString::to_string).collect();
    let placeholders = (0..list.len())
        .map(|i| format!("?{}", i + 3))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT DISTINCT t.id FROM txn t JOIN posting p ON p.txn_id = t.id
         WHERE t.txn_type IN ('Invoice', 'SalesReceipt', 'CreditMemo', 'StatementCharge')
           AND p.quantity IS NOT NULL AND p.item_id IN ({placeholders})
           AND t.date >= ?1 AND t.id <> ?2
         ORDER BY t.date, t.rowid"
    );
    let ids: Vec<String> = {
        let mut stmt = w.tx.prepare(&sql)?;
        let mut args: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(SqlDate(from)), Box::new(origin.to_string())];
        for i in &list {
            args.push(Box::new(i.clone()));
        }
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(AsRef::as_ref).collect();
        stmt.query_map(refs.as_slice(), |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    let mut changed = 0;
    for id in ids {
        let id: TxnId = crate::codec::parse_id(&id)?;
        let txn = txns::get(&w.tx, id)?;
        let fresh = post(&txn, &StoreContext::new(&w.tx))?;
        let stored = txns::postings(&w.tx, id)?;
        if same_amounts(&fresh.postings, &stored) {
            continue;
        }
        // Re-costing is the books' own correction, so it may reach into a
        // closed period (QBD does too).
        let may = w.actor.may_override_closing_date;
        w.actor.may_override_closing_date = true;
        let r = txns::repost(w, txn);
        w.actor.may_override_closing_date = may;
        r?;
        changed += 1;
    }
    Ok(changed)
}

fn same_amounts(a: &[lb_core::Posting], b: &[lb_core::Posting]) -> bool {
    let key = |p: &lb_core::Posting| (p.account_id, p.line_id, p.amount, p.quantity);
    let mut x: Vec<_> = a
        .iter()
        .filter(|p| !p.amount.is_zero() || p.quantity.is_some())
        .map(key)
        .collect();
    let mut y: Vec<_> = b
        .iter()
        .filter(|p| !p.amount.is_zero() || p.quantity.is_some())
        .map(key)
        .collect();
    x.sort();
    y.sort();
    x == y
}

/// Write each item's current quantity on hand and average cost into its record.
pub fn refresh_items(w: &mut Writer<'_>, items: &BTreeSet<ItemId>) -> Result<()> {
    let far = jiff::civil::date(9999, 12, 31);
    for item in items {
        let (qty, value) = on_hand(&w.tx, *item, far, None)?;
        let avg = if qty > Decimal::ZERO {
            (value / qty).round_dp(5)
        } else {
            Decimal::ZERO
        };
        w.tx.execute(
            "UPDATE item SET doc = json_set(doc, '$.quantity_on_hand', ?2, '$.average_cost', ?3) WHERE id = ?1",
            params![item.to_string(), qty.normalize().to_string(), avg.normalize().to_string()],
        )?;
        w.changed("item", item.to_string(), "update");
    }
    Ok(())
}

/// Value on hand of an item as of a date (for inventory valuation checks).
pub fn value_on_hand(
    c: &Connection,
    item: ItemId,
    date: jiff::civil::Date,
) -> Result<(Decimal, Money)> {
    let (qty, value) = on_hand(c, item, date, None)?;
    Ok((qty, Money::from_decimal(value).unwrap_or_default()))
}
