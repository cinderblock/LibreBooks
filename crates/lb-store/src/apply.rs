//! Applying credits to what is owed, as QBD's "Apply Credits" and "Set Credits"
//! do: a customer's credit memos and overpayments against their open invoices, a
//! vendor's credits against their open bills.
//!
//! A credit applied without cash changing hands is carried by a payment of
//! zero, which is also how QBD records it (a "$0.00 payment" appears in the
//! customer's history). The payment's applications say which credit paid which
//! charge; the ledger does not move, because the credit memo already credited
//! A/R when it was entered. Open balances on both sides fall accordingly.
//!
//! Oldest charges are paid first with the oldest credits, as QBD's automatic
//! application does.

use lb_core::{AccountId, Application, EntityId, Money, Txn, TxnId, TxnType};
use rusqlite::{Connection, params};

use crate::codec::{self, SqlDate};
use crate::{Result, StoreError, Writer, entities, txns};

/// An open item of a name: what is owed (positive) or a credit (positive, on
/// the other side).
#[derive(Clone, Debug)]
struct Open {
    id: TxnId,
    txn_type: TxnType,
    date: jiff::civil::Date,
    balance: Money,
}

fn open_items(c: &Connection, root: EntityId, types: &[TxnType]) -> Result<Vec<Open>> {
    let ids = entities::with_descendants(c, root)?;
    let names: Vec<String> = ids.iter().map(ToString::to_string).collect();
    let name_ph = (0..names.len())
        .map(|i| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(",");
    let type_list = types
        .iter()
        .map(|t| format!("'{}'", t.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT id, txn_type, date, open_balance FROM txn
         WHERE entity_id IN ({name_ph}) AND txn_type IN ({type_list})
           AND open_balance IS NOT NULL AND open_balance > 0 AND is_void = 0
         ORDER BY date, rowid"
    );
    let mut stmt = c.prepare(&sql)?;
    let args: Vec<&dyn rusqlite::ToSql> = names.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let rows = stmt.query_map(args.as_slice(), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, SqlDate>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, t, d, open) = row?;
        out.push(Open {
            id: codec::parse_id(&id)?,
            txn_type: TxnType::parse(&t).ok_or_else(|| StoreError::Corrupt(t.clone()))?,
            date: d.0,
            balance: Money(open),
        });
    }
    Ok(out)
}

/// Charges a credit can settle, and credits that can settle them, per side.
const AR_CHARGES: [TxnType; 4] = [
    TxnType::Invoice,
    TxnType::StatementCharge,
    TxnType::JournalEntry,
    TxnType::Check,
];
const AR_CREDITS: [TxnType; 2] = [TxnType::CreditMemo, TxnType::ReceivePayment];
const AP_CHARGES: [TxnType; 3] = [TxnType::Bill, TxnType::JournalEntry, TxnType::Check];
const AP_CREDITS: [TxnType; 1] = [TxnType::VendorCredit];

/// Apply a customer's open credits to their open charges, oldest first. Returns
/// the zero payments created (one per run, when anything applied). A journal
/// entry or check is only a charge here when it is owed *to* us, which its
/// positive open balance in A/R already says.
pub fn apply_customer_credits(w: &mut Writer<'_>, customer: EntityId) -> Result<Option<TxnId>> {
    let root = entities::root_of(&w.tx, customer)?;
    let charges = open_items(&w.tx, root, &AR_CHARGES)?;
    let credits = open_items(&w.tx, root, &AR_CREDITS)?;
    let plan = allocate(&charges, &credits);
    if plan.is_empty() {
        return Ok(None);
    }
    let date = plan
        .iter()
        .map(|(ch, cr, _)| ch.date.max(cr.date))
        .max()
        .unwrap_or_else(|| jiff::Zoned::now().date());
    let mut pmt = Txn::new(TxnType::ReceivePayment, date);
    pmt.entity_id = Some(root);
    pmt.memo = Some("Credits applied".into());
    pmt.amount = Money::ZERO;
    for (charge, credit, amount) in plan {
        pmt.applications.push(Application {
            target_txn_id: charge.id,
            amount: Money::ZERO,
            discount: Money::ZERO,
            discount_account_id: None,
            discount_class_id: None,
            credit_txn_id: Some(credit.id),
            credit_amount: amount,
        });
    }
    Ok(Some(txns::save(w, pmt)?.txn.id))
}

/// Apply a customer's open credits to one charge only, at most `cap` of it:
/// what an import does when QuickBooks shows the charge settled by a credit
/// the bridge never named. Credit memos go first, overpayments after.
pub fn apply_credits_to(w: &mut Writer<'_>, charge: TxnId, cap: Money) -> Result<Option<TxnId>> {
    let t = txns::get(&w.tx, charge)?;
    let Some(entity) = t.entity_id else {
        return Ok(None);
    };
    let root = entities::root_of(&w.tx, entity)?;
    let open: Option<i64> = w.tx.query_row(
        "SELECT open_balance FROM txn WHERE id = ?1",
        [charge.to_string()],
        |r| r.get(0),
    )?;
    let owed = Money(open.unwrap_or(0)).min(cap);
    if !owed.is_positive() {
        return Ok(None);
    }
    let payable = AP_CHARGES.contains(&t.txn_type) && !AR_CHARGES.contains(&t.txn_type);
    if payable {
        return Err(StoreError::Invalid(
            "apply vendor credits with apply_vendor_credits".into(),
        ));
    }
    let mut credits = open_items(&w.tx, root, &AR_CREDITS)?;
    credits.sort_by_key(|c| (c.txn_type != TxnType::CreditMemo, c.date));
    let target = Open {
        id: charge,
        txn_type: t.txn_type,
        date: t.date,
        balance: owed,
    };
    let plan = allocate(std::slice::from_ref(&target), &credits);
    if plan.is_empty() {
        return Ok(None);
    }
    let date = plan
        .iter()
        .map(|(ch, cr, _)| ch.date.max(cr.date))
        .max()
        .unwrap_or(t.date);
    let mut pmt = Txn::new(TxnType::ReceivePayment, date);
    pmt.entity_id = Some(root);
    pmt.memo = Some("Credits applied".into());
    for (ch, credit, amount) in plan {
        pmt.applications.push(Application {
            target_txn_id: ch.id,
            amount: Money::ZERO,
            discount: Money::ZERO,
            discount_account_id: None,
            discount_class_id: None,
            credit_txn_id: Some(credit.id),
            credit_amount: amount,
        });
    }
    Ok(Some(txns::save(w, pmt)?.txn.id))
}

/// Apply a vendor's open credits to their open bills, oldest first. A bill
/// payment needs the bank account it would draw on even when it draws nothing.
pub fn apply_vendor_credits(
    w: &mut Writer<'_>,
    vendor: EntityId,
    bank: AccountId,
) -> Result<Option<TxnId>> {
    let charges = open_items(&w.tx, vendor, &AP_CHARGES)?;
    let credits = open_items(&w.tx, vendor, &AP_CREDITS)?;
    let plan = allocate(&charges, &credits);
    if plan.is_empty() {
        return Ok(None);
    }
    let date = plan
        .iter()
        .map(|(ch, cr, _)| ch.date.max(cr.date))
        .max()
        .unwrap_or_else(|| jiff::Zoned::now().date());
    let mut pmt = Txn::new(TxnType::BillPaymentCheck, date);
    pmt.entity_id = Some(vendor);
    pmt.account_id = Some(bank);
    pmt.memo = Some("Credits applied".into());
    pmt.amount = Money::ZERO;
    for (charge, credit, amount) in plan {
        pmt.applications.push(Application {
            target_txn_id: charge.id,
            amount: Money::ZERO,
            discount: Money::ZERO,
            discount_account_id: None,
            discount_class_id: None,
            credit_txn_id: Some(credit.id),
            credit_amount: amount,
        });
    }
    Ok(Some(txns::save(w, pmt)?.txn.id))
}

/// Oldest charge, oldest credit, as much as both allow; then on to whichever
/// ran out.
fn allocate(charges: &[Open], credits: &[Open]) -> Vec<(Open, Open, Money)> {
    let mut out = Vec::new();
    let mut ci = 0;
    let mut credit_left = credits.first().map(|c| c.balance);
    for charge in charges {
        let mut owed = charge.balance;
        while owed.is_positive() {
            let Some(left) = credit_left else { break };
            let credit = &credits[ci];
            let take = owed.min(left);
            if take.is_positive() {
                out.push((charge.clone(), credit.clone(), take));
                owed -= take;
            }
            let remaining = left - take;
            if remaining.is_zero() {
                ci += 1;
                credit_left = credits.get(ci).map(|c| c.balance);
            } else {
                credit_left = Some(remaining);
            }
        }
        if credit_left.is_none() {
            break;
        }
    }
    out
}

/// Names (customers, or vendors) with both open credits and open charges: the
/// candidates for applying. A job's invoices and its customer's credit memo
/// belong to the same name, so the comparison is by top-level name.
pub fn names_with_unapplied_credits(c: &Connection, receivable: bool) -> Result<Vec<EntityId>> {
    let (charges, credits): (&[TxnType], &[TxnType]) = if receivable {
        (&AR_CHARGES, &AR_CREDITS)
    } else {
        (&AP_CHARGES, &AP_CREDITS)
    };
    let roots_with = |types: &[TxnType]| -> Result<Vec<EntityId>> {
        let list = types
            .iter()
            .map(|t| format!("'{}'", t.as_str()))
            .collect::<Vec<_>>()
            .join(",");
        let mut stmt = c.prepare(&format!(
            "SELECT DISTINCT entity_id FROM txn
             WHERE entity_id IS NOT NULL AND txn_type IN ({list}) AND open_balance > 0 AND is_void = 0"
        ))?;
        let rows = stmt.query_map(params![], |r| r.get::<_, String>(0))?;
        let mut roots = Vec::new();
        for row in rows {
            let id: EntityId = codec::parse_id(&row?)?;
            let root = entities::root_of(c, id)?;
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        Ok(roots)
    };
    let with_credits = roots_with(credits)?;
    let with_charges = roots_with(charges)?;
    Ok(with_credits
        .into_iter()
        .filter(|r| with_charges.contains(r))
        .collect())
}
