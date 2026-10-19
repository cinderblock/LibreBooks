//! Cash basis: income and expenses count when they are paid.
//!
//! QuickBooks Desktop keeps one set of books (accrual) and derives cash-basis
//! reports from it. LibreBooks does the same, as two adjustments to the accrual
//! ledger, each of which sums to zero on its own date:
//!
//! **Deferral.** On an *open item* (invoice, statement charge, credit memo, bill,
//! vendor credit, item receipt), every posting to an income-statement account
//! (and to Sales Tax Payable when sales tax is owed on the cash basis) is moved
//! into the transaction's A/R or A/P account, on the transaction's own date. An
//! unpaid invoice therefore shows no income, and its A/R comes to nothing except
//! what was not deferred. Done in SQL ([`deferred_sql`]) by re-pointing the
//! posting's account, so summary reports still aggregate in the database.
//!
//! **Recognition.** Each `application` row dated D relieves its target by
//! `amount + discount + credit_amount`, and relieves the credit it uses (a credit
//! memo or vendor credit) by `credit_amount`. The relief is spread over the open
//! item's postings in proportion to their amounts; the deferred postings' shares
//! move out of A/R or A/P and into their own accounts, dated D, against the
//! payment (QuickZoom opens the payment; `origin` remembers the invoice).
//!
//! Allocation is *cumulative*: after each payment the amount recognised so far on
//! each posting is `allocate(paid so far)`, and a payment recognises the
//! difference from the previous one. So once an invoice is paid in full every
//! posting has been recognised to exactly its own amount, whatever rounding the
//! partial payments needed. Shares are computed with [`Money::allocate`] (largest
//! remainder) when every weight has the same sign, which is the usual case; an
//! invoice mixing charges and discount lines uses the same largest-remainder
//! method on signed weights ([`allocate_signed`]).
//!
//! What stays as posted on the cash basis: everything not on an open item
//! (checks, deposits, sales receipts, card charges, transfers, journal entries,
//! paychecks), the A/R or A/P side of payments not applied to an open item
//! (unapplied credits, payments of journal entries), balance-sheet postings on
//! open items (inventory bought on a bill, fixed assets, sales tax on the accrual
//! basis), and the inventory cost of a sale (COGS and the asset it relieves,
//! which balance each other), which QBD counts when sold. Any A/R or A/P they
//! leave behind shows on the cash-basis balance sheet, as it does in QBD.

use std::collections::HashMap;

use jiff::civil::Date;
use lb_core::{
    AccountClass, AccountId, AccountRole, ClassId, ClearedStatus, EntityId, ItemId, LineId, Money,
    TxnId, TxnType,
};
use lb_store::codec::{self, SqlDate};
use rusqlite::Connection;

use crate::Result;
use crate::ctx::Ctx;
use crate::filter::Sql;
use crate::ledger::Fact;

/// Transaction types that sit open in A/R or A/P until paid.
pub const OPEN_ITEM_TYPES: [TxnType; 6] = [
    TxnType::Invoice,
    TxnType::StatementCharge,
    TxnType::CreditMemo,
    TxnType::Bill,
    TxnType::VendorCredit,
    TxnType::ItemReceipt,
];

/// The sales-side open items, whose inventory cost counts when sold.
const SALES_OPEN_ITEM_TYPES: [TxnType; 3] = [
    TxnType::Invoice,
    TxnType::StatementCharge,
    TxnType::CreditMemo,
];

fn sql_list(types: &[TxnType]) -> String {
    types
        .iter()
        .map(|t| format!("'{}'", t.as_str()))
        .collect::<Vec<_>>()
        .join(",")
}

/// SQL (over `posting p`, `txn t`, `account a`, `item i`) that is true for a
/// posting the cash basis defers. Must agree with [`role_of`].
pub fn deferred_sql(tax_on_cash: bool) -> String {
    let tax = if tax_on_cash {
        format!(" OR a.role = '{}'", AccountRole::SalesTaxPayable.as_str())
    } else {
        String::new()
    };
    format!(
        "(t.txn_type IN ({open}) AND t.ar_ap_account_id IS NOT NULL AND (
            (a.account_type IN ('Income','CostOfGoodsSold','Expense','OtherIncome','OtherExpense')
             AND NOT (t.txn_type IN ({sales})
                      AND a.account_type IN ('CostOfGoodsSold','Expense','OtherExpense')
                      AND i.item_type IN ('Inventory','InventoryAssembly'))){tax}))",
        open = sql_list(&OPEN_ITEM_TYPES),
        sales = sql_list(&SALES_OPEN_ITEM_TYPES),
    )
}

/// What a posting on an open item is to the cash basis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostingRole {
    /// The A/R or A/P side.
    ArAp,
    /// Recognised when paid.
    Deferred,
    /// Counted when posted; its share of payments stays in A/R or A/P.
    Immediate,
    /// The inventory cost of a sale: counted when posted, and outside the
    /// proportion payments are spread over.
    CostPair,
}

/// Classify a posting of an open item. Must agree with [`deferred_sql`].
pub fn role_of(
    ctx: &Ctx,
    txn_type: TxnType,
    account: AccountId,
    item: Option<ItemId>,
    tax_on_cash: bool,
) -> PostingRole {
    let Some(at) = ctx.account_type(account) else {
        return PostingRole::Immediate;
    };
    if at.requires_name() {
        return PostingRole::ArAp;
    }
    if SALES_OPEN_ITEM_TYPES.contains(&txn_type)
        && ctx.item_is_inventory(item)
        && at.class() != AccountClass::Income
    {
        return PostingRole::CostPair;
    }
    if at.is_income_statement()
        || (tax_on_cash && ctx.role(AccountRole::SalesTaxPayable) == Some(account))
    {
        return PostingRole::Deferred;
    }
    PostingRole::Immediate
}

/// Split `total` over `weights` in proportion, exactly: the parts add up to
/// `total`. Same-sign weights use [`Money::allocate`]; mixed signs (a charge and a
/// discount on one invoice) use the largest-remainder method on the signed
/// proportions `total × wᵢ / Σw`.
pub fn allocate_signed(total: Money, weights: &[Money]) -> Vec<Money> {
    if weights.is_empty() {
        return Vec::new();
    }
    let all_nonneg = weights.iter().all(|w| !w.is_negative());
    let all_nonpos = weights.iter().all(|w| !w.is_positive());
    let sum: i128 = weights.iter().map(|w| i128::from(w.0)).sum();
    if all_nonneg || all_nonpos || sum == 0 {
        // Parts carry `total`'s sign; with same-sign weights that is each
        // weight's own sign times the (positive) fraction paid.
        let raw: Vec<i64> = weights.iter().map(|w| w.0).collect();
        return total.allocate(&raw);
    }
    // partᵢ = total × wᵢ / Σw; flip numerator and divisor together so the
    // divisor is positive and floor division rounds every part down.
    let (flip, den) = if sum < 0 { (-1, -sum) } else { (1, sum) };
    let t = i128::from(total.0) * flip;
    let mut parts: Vec<i128> = Vec::with_capacity(weights.len());
    let mut rems: Vec<(usize, i128)> = Vec::with_capacity(weights.len());
    for (i, w) in weights.iter().enumerate() {
        let n = t * i128::from(w.0);
        parts.push(n.div_euclid(den));
        rems.push((i, n.rem_euclid(den)));
    }
    // The floors fall short of `total` by 0 ≤ left < n units; hand them out
    // largest remainder first (ties to the earliest weight).
    let mut left = i128::from(total.0) - parts.iter().sum::<i128>();
    rems.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (i, _) in rems {
        if left <= 0 {
            break;
        }
        parts[i] += 1;
        left -= 1;
    }
    parts.into_iter().map(|p| Money(p as i64)).collect()
}

/// One payment application, with the paying transaction's header.
struct App {
    payer: TxnId,
    payer_type: TxnType,
    payer_order: i64,
    payer_number: Option<String>,
    payer_amount: Money,
    seq: i64,
    date: Date,
}

/// A posting of an open item.
struct OpenPosting {
    account: AccountId,
    amount: Money,
    entity: Option<EntityId>,
    class: Option<ClassId>,
    item: Option<ItemId>,
    memo: Option<String>,
    line: Option<LineId>,
}

/// Every amount recognised on the cash basis from a payment, as facts dated on
/// the payment (in pairs: the income or expense account, and the A/R or A/P it
/// leaves).
pub fn recognitions(c: &Connection, ctx: &Ctx, tax_on_cash: bool) -> Result<Vec<Fact>> {
    // Applications in the order they happened.
    let mut apps: Vec<App> = Vec::new();
    // Open item → (application index, relief).
    let mut events: HashMap<TxnId, Vec<(usize, i64)>> = HashMap::new();
    {
        let mut stmt = c.prepare(
            "SELECT a.txn_id, a.seq, a.target_txn_id, a.date, a.amount, a.discount,
                    a.credit_txn_id, a.credit_amount, t.txn_type, t.rowid, t.number, t.amount
             FROM application a JOIN txn t ON t.id = a.txn_id
             ORDER BY a.date, t.rowid, a.seq",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let i = apps.len();
            let target: TxnId = codec::parse_id(&r.get::<_, String>(2)?)?;
            let amount: i64 = r.get(4)?;
            let discount: i64 = r.get(5)?;
            let credit: Option<String> = r.get(6)?;
            let credit_amount: i64 = r.get(7)?;
            let payer_type: String = r.get(8)?;
            let date: SqlDate = r.get(3)?;
            apps.push(App {
                payer: codec::parse_id(&r.get::<_, String>(0)?)?,
                payer_type: TxnType::parse(&payer_type)
                    .ok_or_else(|| lb_store::StoreError::Corrupt(payer_type.clone()))?,
                payer_order: r.get(9)?,
                payer_number: r.get(10)?,
                payer_amount: Money(r.get(11)?),
                seq: r.get(1)?,
                date: date.0,
            });
            events
                .entry(target)
                .or_default()
                .push((i, amount + discount + credit_amount));
            if let Some(credit) = credit
                && credit_amount != 0
            {
                events
                    .entry(codec::parse_id(&credit)?)
                    .or_default()
                    .push((i, credit_amount));
            }
        }
    }
    if events.is_empty() {
        return Ok(Vec::new());
    }

    // The open items' headers and postings.
    let ids: Vec<TxnId> = events.keys().copied().collect();
    let mut headers: HashMap<TxnId, (TxnType, Option<EntityId>)> = HashMap::new();
    let mut postings: HashMap<TxnId, Vec<OpenPosting>> = HashMap::new();
    for chunk in ids.chunks(500) {
        let mut q = Sql::default();
        let list = q.bind_list(chunk.iter().map(ToString::to_string));
        let open = sql_list(&OPEN_ITEM_TYPES);
        q.text = format!(
            "SELECT id, txn_type, entity_id FROM txn
             WHERE id IN {list} AND txn_type IN ({open}) AND is_void = 0 AND is_pending = 0"
        );
        let mut stmt = c.prepare(&q.text)?;
        let mut rows = stmt.query(q.params())?;
        while let Some(r) = rows.next()? {
            let t: String = r.get(1)?;
            let Some(t) = TxnType::parse(&t) else {
                continue;
            };
            let entity: Option<String> = r.get(2)?;
            headers.insert(
                codec::parse_id(&r.get::<_, String>(0)?)?,
                (t, entity.as_deref().map(codec::parse_id).transpose()?),
            );
        }
        drop(rows);
        drop(stmt);
        q.text = format!(
            "SELECT txn_id, account_id, amount, entity_id, class_id, item_id, memo, line_id
             FROM posting WHERE txn_id IN {list} ORDER BY txn_id, seq"
        );
        let mut stmt = c.prepare(&q.text)?;
        let mut rows = stmt.query(q.params())?;
        while let Some(r) = rows.next()? {
            let txn: TxnId = codec::parse_id(&r.get::<_, String>(0)?)?;
            let opt = |s: Option<String>| -> Result<Option<String>> { Ok(s) };
            postings.entry(txn).or_default().push(OpenPosting {
                account: codec::parse_id(&r.get::<_, String>(1)?)?,
                amount: Money(r.get(2)?),
                entity: opt(r.get(3)?)?
                    .as_deref()
                    .map(codec::parse_id)
                    .transpose()?,
                class: opt(r.get(4)?)?
                    .as_deref()
                    .map(codec::parse_id)
                    .transpose()?,
                item: opt(r.get(5)?)?
                    .as_deref()
                    .map(codec::parse_id)
                    .transpose()?,
                memo: r.get(6)?,
                line: opt(r.get(7)?)?
                    .as_deref()
                    .map(codec::parse_id)
                    .transpose()?,
            });
        }
    }

    let mut out = Vec::new();
    // Deterministic order regardless of hash order.
    let mut open_ids: Vec<TxnId> = headers.keys().copied().collect();
    open_ids.sort();
    for t_id in open_ids {
        let (t_type, t_entity) = headers[&t_id];
        let Some(ps) = postings.get(&t_id) else {
            continue;
        };
        let mut open = Money::ZERO;
        let mut ar_ap: Option<(AccountId, Option<EntityId>)> = None;
        let mut weighted: Vec<(&OpenPosting, bool)> = Vec::new();
        for p in ps {
            match role_of(ctx, t_type, p.account, p.item, tax_on_cash) {
                PostingRole::ArAp => {
                    open += p.amount;
                    ar_ap.get_or_insert((p.account, p.entity));
                }
                PostingRole::Deferred => weighted.push((p, true)),
                PostingRole::Immediate => weighted.push((p, false)),
                PostingRole::CostPair => {}
            }
        }
        let Some((ar_ap_account, ar_ap_entity)) = ar_ap else {
            continue;
        };
        if open.is_zero() || !weighted.iter().any(|(_, d)| *d) {
            continue;
        }
        let weights: Vec<Money> = weighted.iter().map(|(p, _)| p.amount).collect();
        let cap = open.0.abs();
        let sign = -open.0.signum();
        let mut paid: i64 = 0;
        let mut before = vec![Money::ZERO; weights.len()];
        for &(app_i, relief) in &events[&t_id] {
            let app = &apps[app_i];
            paid = (paid + relief).clamp(0, cap);
            let now = allocate_signed(Money(sign * paid), &weights);
            let mut moved = Money::ZERO;
            for (k, ((p, deferred), share)) in weighted.iter().zip(&now).enumerate() {
                let delta = *share - before[k];
                if !*deferred || delta.is_zero() {
                    continue;
                }
                moved += delta;
                out.push(Fact {
                    txn_id: app.payer,
                    txn_type: app.payer_type,
                    txn_order: app.payer_order,
                    seq: app.seq * 10_000 + k as i64,
                    date: app.date,
                    account_id: p.account,
                    amount: delta,
                    entity_id: p.entity,
                    txn_entity_id: t_entity,
                    class_id: p.class,
                    item_id: p.item,
                    memo: p.memo.clone(),
                    cleared: ClearedStatus::Uncleared,
                    number: app.payer_number.clone(),
                    quantity: None,
                    line_id: p.line,
                    is_void: false,
                    txn_amount: app.payer_amount,
                    origin: Some(t_id),
                    origin_type: t_type,
                    repointed: false,
                });
            }
            before = now;
            if !moved.is_zero() {
                out.push(Fact {
                    txn_id: app.payer,
                    txn_type: app.payer_type,
                    txn_order: app.payer_order,
                    seq: app.seq * 10_000 + 9_999,
                    date: app.date,
                    account_id: ar_ap_account,
                    amount: -moved,
                    entity_id: ar_ap_entity,
                    txn_entity_id: t_entity,
                    class_id: None,
                    item_id: None,
                    memo: None,
                    cleared: ClearedStatus::Uncleared,
                    number: app.payer_number.clone(),
                    quantity: None,
                    line_id: None,
                    is_void: false,
                    txn_amount: app.payer_amount,
                    origin: Some(t_id),
                    origin_type: t_type,
                    repointed: true,
                });
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(v: &[i64]) -> Vec<Money> {
        v.iter().copied().map(Money).collect()
    }

    #[test]
    fn same_sign_weights_use_money_allocate() {
        assert_eq!(
            allocate_signed(Money(-100), &m(&[-1, -1, -1])),
            m(&[-34, -33, -33])
        );
        assert_eq!(allocate_signed(Money(50), &m(&[60, 40])), m(&[30, 20]));
    }

    #[test]
    fn mixed_signs_split_exactly() {
        // A $1,000 charge with a $100 discount: $900 owed, half paid.
        let parts = allocate_signed(Money(-450), &m(&[-100_000, 10_000]));
        assert_eq!(parts, m(&[-500, 50]));
        assert_eq!(parts.iter().sum::<Money>(), Money(-450));
        // Rounding still sums exactly.
        let parts = allocate_signed(Money(-1), &m(&[-3, 1, -1]));
        assert_eq!(parts.iter().sum::<Money>(), Money(-1));
        let parts = allocate_signed(Money(-7), &m(&[-1001, 333, -332]));
        assert_eq!(parts.iter().sum::<Money>(), Money(-7));
    }
}
