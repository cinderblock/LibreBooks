//! Order fulfilment: what has been received or billed against a purchase order,
//! invoiced against a sales order, or progress-invoiced from an estimate, and
//! whether the order is still open.
//!
//! Orders don't post. What makes them live is the lines of later transactions
//! that link back to them ([`TxnLine::linked_txn_id`] / `linked_line_id`): an
//! item receipt's lines against a PO's, an invoice's against a sales order's.
//! Those links are kept relationally in `txn_link` (one row per linked line of
//! a non-void transaction, indexed by target), so an order's status is one
//! indexed query rather than a scan of every transaction's JSON. They are
//! checked when the linking transaction is saved ([`check_links`]): the target
//! must exist and be a type this form may link to ([`TxnType::link_targets`]),
//! belong to the same name (a job rolls up to its customer), the target line
//! must exist and carry the same item, and neither the line nor the order may
//! be closed. Quantities are not capped: QBD allows over-receipt, and the
//! numbers here show it.
//!
//! An order's `open_balance` column is its remaining amount (zero once closed),
//! so `TxnFilter { open_only: true }` lists open orders. `txns::save`, `void`
//! and `delete` refresh it for every order the saved transaction links to.

use std::collections::{BTreeSet, HashMap, HashSet};

use lb_core::{ItemId, ItemType, LineId, LineKind, Money, Txn, TxnId, TxnLine, TxnType};
use rusqlite::{Connection, params};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::codec::{self, doc};
use crate::{Result, StoreError, entities, items, txns};

/// How far an order has been fulfilled, line by line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Fulfilment {
    pub order_id: TxnId,
    /// Closed by hand (`Txn::is_manually_closed`), or every line closed or
    /// fully fulfilled. A closed order has nothing remaining.
    pub is_closed: bool,
    /// What is still expected, in money: the order's open balance.
    pub remaining_amount: Money,
    /// The order's fulfillable lines, in form order. Text lines, subtotals and
    /// zero lines are left out.
    pub lines: Vec<LineFulfilment>,
}

/// One order line against what has been received or invoiced on it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LineFulfilment {
    pub line_id: LineId,
    pub item_id: Option<ItemId>,
    /// The line's quantity; `None` on an expense line or an amount-only line.
    #[ts(type = "string | null")]
    pub ordered_quantity: Option<Decimal>,
    /// Quantity on the linked lines of saved, non-void transactions. May exceed
    /// `ordered_quantity`.
    #[ts(type = "string")]
    pub fulfilled_quantity: Decimal,
    pub ordered_amount: Money,
    /// Amount on the linked lines. For an estimate this is the progress.
    pub fulfilled_amount: Money,
    /// Closed by hand (`TxnLine::is_closed`), fully fulfilled, or the whole
    /// order closed by hand.
    pub is_closed: bool,
    /// `ordered_quantity` less `fulfilled_quantity`, never below zero; zero when
    /// closed; `None` when the line has no quantity.
    #[ts(type = "string | null")]
    pub remaining_quantity: Option<Decimal>,
    /// What is left at the order's own price: the remaining quantity at the
    /// line's rate for a quantity line, the unfulfilled amount otherwise (and
    /// always for an estimate, whose progress is by amount). Zero when closed.
    pub remaining_amount: Money,
}

/// An order's fulfilment. `NotFound` for a transaction that isn't a purchase
/// order, sales order or estimate.
pub fn fulfilment(c: &Connection, order_id: TxnId) -> Result<Fulfilment> {
    let order = txns::get(c, order_id)?;
    if !order.txn_type.is_order() {
        return Err(StoreError::NotFound {
            kind: "order",
            id: order_id.to_string(),
        });
    }
    compute(c, &order)
}

/// What is still expected on an order, for its `open_balance`.
pub(crate) fn remaining(c: &Connection, order: &Txn) -> Result<Money> {
    Ok(compute(c, order)?.remaining_amount)
}

fn compute(c: &Connection, order: &Txn) -> Result<Fulfilment> {
    // Quantity and amount linked to each of the order's lines.
    let mut linked: HashMap<LineId, (Decimal, Money)> = HashMap::new();
    let mut stmt = c.prepare(
        "SELECT k.line_id, k.target_line_id, t.doc FROM txn_link k JOIN txn t ON t.id = k.txn_id
         WHERE k.target_txn_id = ?1 AND t.is_void = 0",
    )?;
    let rows = stmt.query_map([order.id.to_string()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            doc::<Txn>(r, 2)?,
        ))
    })?;
    for row in rows {
        let (line_id, target_line, txn) = row?;
        let line_id: LineId = codec::parse_id(&line_id)?;
        let Some(line) = txn.lines.iter().find(|l| l.id == line_id) else {
            continue;
        };
        let target = match target_line {
            Some(t) => Some(codec::parse_id::<LineId>(&t)?),
            None => matching_line(order, line).map(|l| l.id),
        };
        let Some(target) = target else {
            continue;
        };
        let e = linked.entry(target).or_insert((Decimal::ZERO, Money::ZERO));
        e.0 += line.quantity.unwrap_or(Decimal::ZERO);
        e.1 += line.amount;
    }

    // Progress on an estimate is by amount, however the quantities read.
    let by_amount_only = order.txn_type == TxnType::Estimate;
    let mut lines = Vec::new();
    for l in &order.lines {
        if !fulfillable(c, l)? {
            continue;
        }
        let (fulfilled_quantity, fulfilled_amount) = linked
            .get(&l.id)
            .copied()
            .unwrap_or((Decimal::ZERO, Money::ZERO));
        let quantity = l.quantity.filter(|q| !q.is_zero() && !by_amount_only);
        let fully = match quantity {
            Some(q) => fulfilled_quantity >= q,
            None => toward(fulfilled_amount, l.amount) >= l.amount.abs(),
        };
        let is_closed = order.is_manually_closed || l.is_closed || fully;
        let remaining_quantity = l.quantity.map(|q| {
            if is_closed {
                Decimal::ZERO
            } else {
                (q - fulfilled_quantity).max(Decimal::ZERO)
            }
        });
        let remaining_amount = if is_closed {
            Money::ZERO
        } else if let Some(q) = quantity {
            let left = remaining_quantity.unwrap_or(Decimal::ZERO);
            prorate(l.amount, left, q)
        } else {
            let left = l.amount.abs() - toward(fulfilled_amount, l.amount);
            let left = left.max(Money::ZERO);
            if l.amount.is_negative() { -left } else { left }
        };
        lines.push(LineFulfilment {
            line_id: l.id,
            item_id: l.item_id,
            ordered_quantity: l.quantity,
            fulfilled_quantity,
            ordered_amount: l.amount,
            fulfilled_amount,
            is_closed,
            remaining_quantity,
            remaining_amount,
        });
    }
    let is_closed = order.is_manually_closed || lines.iter().all(|l| l.is_closed);
    let remaining_amount = if is_closed {
        Money::ZERO
    } else {
        lines.iter().map(|l| l.remaining_amount).sum()
    };
    Ok(Fulfilment {
        order_id: order.id,
        is_closed,
        remaining_amount,
        lines,
    })
}

/// How much of `fulfilled` counts toward an ordered amount of `ordered`'s sign:
/// a discount line (negative) is fulfilled by negative amounts.
fn toward(fulfilled: Money, ordered: Money) -> Money {
    if ordered.is_negative() {
        -fulfilled
    } else {
        fulfilled
    }
}

/// `amount × part / whole`, rounded to money: the ordered amount of what is
/// still to come, at the order's own rate.
fn prorate(amount: Money, part: Decimal, whole: Decimal) -> Money {
    if whole.is_zero() {
        return Money::ZERO;
    }
    amount
        .to_decimal()
        .checked_mul(part)
        .and_then(|v| v.checked_div(whole))
        .and_then(Money::from_decimal)
        .unwrap_or(Money::ZERO)
}

/// Can anything be received or invoiced against this line: not text, not a
/// subtotal or group header, not an empty line.
fn fulfillable(c: &Connection, l: &TxnLine) -> Result<bool> {
    if l.kind == LineKind::Text {
        return Ok(false);
    }
    if l.amount.is_zero() && l.quantity.is_none_or(|q| q.is_zero()) {
        return Ok(false);
    }
    if let Some(item) = l.item_id
        && let Some(item) = items::find(c, item)?
        && matches!(item.item_type, ItemType::Subtotal | ItemType::Group)
    {
        return Ok(false);
    }
    Ok(true)
}

/// The order line a link without a line id lands on: the first with the same
/// item.
fn matching_line<'t>(target: &'t Txn, line: &TxnLine) -> Option<&'t TxnLine> {
    let item = line.item_id?;
    target
        .lines
        .iter()
        .find(|l| l.kind == LineKind::Item && l.item_id == Some(item))
}

/// The store's half of line links, before a transaction is saved. `existing`
/// is the saved version of the same transaction, if any: a link it already had
/// stays legal even if the order or line has since been closed, so an edit
/// doesn't fail for what it didn't change.
pub(crate) fn check_links(c: &Connection, txn: &Txn, existing: Option<&Txn>) -> Result<()> {
    // Deposit lines link to payments, checked with the deposit itself.
    if txn.txn_type == TxnType::Deposit {
        return Ok(());
    }
    let linked: Vec<&TxnLine> = txn
        .lines
        .iter()
        .filter(|l| l.linked_txn_id.is_some())
        .collect();
    if linked.is_empty() {
        return Ok(());
    }
    let targets = txn.txn_type.link_targets();
    if targets.is_empty() {
        return Err(StoreError::Invalid(format!(
            "lines on a {} can't be entered against another transaction",
            txn.txn_type.label()
        )));
    }
    let own = txn.entity_id.ok_or_else(|| {
        StoreError::Invalid("choose a name before entering lines against an order".into())
    })?;
    let root = entities::root_of(c, own)?;
    let previous: HashSet<(TxnId, Option<LineId>)> = existing
        .map(|x| {
            x.lines
                .iter()
                .filter_map(|l| l.linked_txn_id.map(|t| (t, l.linked_line_id)))
                .collect()
        })
        .unwrap_or_default();
    let mut cache: HashMap<TxnId, Txn> = HashMap::new();
    for line in linked {
        let target_id = line.linked_txn_id.expect("filtered");
        if target_id == txn.id {
            return Err(StoreError::Invalid(
                "a line can't be entered against its own transaction".into(),
            ));
        }
        let target = match cache.entry(target_id) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let t = txns::find(c, target_id)?.ok_or_else(|| {
                    StoreError::Invalid(
                        "the transaction this line is entered against doesn't exist".into(),
                    )
                })?;
                e.insert(t)
            }
        };
        let label = target.txn_type.label();
        if !targets.contains(&target.txn_type) {
            return Err(StoreError::Invalid(format!(
                "a {} line can't be entered against a {label}",
                txn.txn_type.label()
            )));
        }
        let theirs = target.entity_id.ok_or_else(|| {
            StoreError::Invalid(format!("that {label} has no name to match against"))
        })?;
        if entities::root_of(c, theirs)? != root {
            let who = if target.txn_type.header_entity() == lb_core::HeaderEntity::Customer {
                "customer"
            } else {
                "vendor"
            };
            return Err(StoreError::Invalid(format!(
                "that {label} belongs to another {who}"
            )));
        }
        let is_new = !previous.contains(&(target_id, line.linked_line_id));
        if is_new && target.is_void {
            return Err(StoreError::Invalid(format!("that {label} is void")));
        }
        if is_new && target.txn_type.is_order() && target.is_manually_closed {
            return Err(StoreError::Invalid(format!("that {label} is closed")));
        }
        let target_line = match line.linked_line_id {
            Some(lid) => Some(target.lines.iter().find(|l| l.id == lid).ok_or_else(|| {
                StoreError::Invalid(format!(
                    "the line this one is entered against isn't on that {label}"
                ))
            })?),
            None => matching_line(target, line),
        };
        match target_line {
            Some(tl) => {
                if line.kind == LineKind::Item && tl.item_id != line.item_id {
                    return Err(StoreError::Invalid(format!(
                        "that line of the {label} is for a different item"
                    )));
                }
                if is_new && target.txn_type.is_order() && tl.is_closed {
                    return Err(StoreError::Invalid(format!(
                        "that line of the {label} is closed"
                    )));
                }
            }
            None if line.kind == LineKind::Item => {
                return Err(StoreError::Invalid(format!(
                    "that {label} has no line for this item"
                )));
            }
            None => {}
        }
    }
    Ok(())
}

/// Replace the `txn_link` rows of a transaction: one per linked line, none
/// when it is void (a voided receipt reopens its PO). Call after
/// [`check_links`], which made sure every target exists.
pub(crate) fn write_links(c: &Connection, txn: &Txn) -> Result<()> {
    let id = txn.id.to_string();
    c.execute("DELETE FROM txn_link WHERE txn_id = ?1", [&id])?;
    if txn.is_void || txn.txn_type.link_targets().is_empty() {
        return Ok(());
    }
    for line in &txn.lines {
        if let Some(target) = line.linked_txn_id {
            c.execute(
                "INSERT INTO txn_link (txn_id, line_id, target_txn_id, target_line_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    id,
                    line.id.to_string(),
                    target.to_string(),
                    line.linked_line_id.map(|l| l.to_string()),
                ],
            )?;
        }
    }
    Ok(())
}

/// Every transaction this one's lines are entered against (orders, and the
/// receipt a bill was made from), whose open balance depends on it.
pub(crate) fn linked_targets(txn: &Txn) -> BTreeSet<TxnId> {
    if txn.txn_type.link_targets().is_empty() {
        return BTreeSet::new();
    }
    txn.lines.iter().filter_map(|l| l.linked_txn_id).collect()
}

/// Recompute every live order's open balance. Run once when a file from
/// before order fulfilment is opened (its orders had none); idempotent.
pub fn refresh_all(c: &Connection) -> Result<usize> {
    let ids: Vec<String> = {
        let mut stmt = c.prepare(
            "SELECT id FROM txn WHERE txn_type IN ('PurchaseOrder', 'SalesOrder', 'Estimate')
             AND is_void = 0",
        )?;
        stmt.query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    for id in &ids {
        txns::refresh_open_balance(c, codec::parse_id(id)?)?;
    }
    Ok(ids.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prorates_at_the_orders_rate() {
        // 10 @ $5 = $50; 6 left → $30.
        assert_eq!(
            prorate(Money(5_000), Decimal::from(6), Decimal::from(10)),
            Money(3_000)
        );
        // Thirds round half away from zero.
        assert_eq!(
            prorate(Money(1_000), Decimal::from(1), Decimal::from(3)),
            Money(333)
        );
        assert_eq!(
            prorate(Money(1_000), Decimal::ONE, Decimal::ZERO),
            Money::ZERO
        );
    }

    #[test]
    fn discount_lines_fulfil_toward_their_own_sign() {
        assert_eq!(toward(Money(-500), Money(-1_000)), Money(500));
        assert_eq!(toward(Money(500), Money(1_000)), Money(500));
    }
}
