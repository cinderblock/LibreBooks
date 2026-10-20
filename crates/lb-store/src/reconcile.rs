//! Reconciling an account against a statement, as QBD's Reconcile window does.
//!
//! A reconciliation starts with a statement date and ending balance (and,
//! optionally, a service charge and interest earned, which become transactions).
//! The person ticks off what the statement shows; ticked lines are `Pending` (QBD's
//! asterisk). When the difference is zero (or the person accepts an adjustment to
//! Reconciliation Discrepancies) it finishes: ticked lines become `Cleared` and
//! remember which reconciliation cleared them, so the last one can be undone.
//!
//! Amounts here are in the account's natural sign (a credit card's balance owed is
//! positive), because that is how statements print them.

use lb_core::{
    AccountId, AccountRole, AccountType, ClearedStatus, Money, ReconciliationId, Txn, TxnId,
    TxnLine, TxnType,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::codec::{self, SqlDate, doc};
use crate::{Result, StoreError, Writer, accounts, txns};

/// A reconciliation, in progress or finished.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Reconciliation {
    pub id: ReconciliationId,
    pub account_id: AccountId,
    pub statement_date: jiff::civil::Date,
    /// What the previous reconciliations cleared, natural sign.
    pub beginning_balance: Money,
    /// The statement's ending balance, natural sign.
    pub ending_balance: Money,
    pub service_charge_txn_id: Option<TxnId>,
    pub interest_txn_id: Option<TxnId>,
    pub adjustment_txn_id: Option<TxnId>,
    pub finished_at: Option<jiff::Timestamp>,
    pub finished_by: Option<String>,
    /// Transactions this reconciliation cleared (filled in when it finishes).
    pub cleared_txn_ids: Vec<TxnId>,
}

/// One line of the Reconcile window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReconcileLine {
    pub txn_id: TxnId,
    pub txn_type: TxnType,
    pub date: jiff::civil::Date,
    pub number: Option<String>,
    pub payee: Option<String>,
    pub memo: Option<String>,
    /// Natural sign for the account: for a bank, deposits positive and checks
    /// negative; for a card, charges positive and payments negative.
    pub amount: Money,
    pub marked: bool,
}

/// Where a reconciliation stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReconcileStatus {
    pub beginning_balance: Money,
    pub marked_increases: Money,
    pub marked_decreases: Money,
    pub cleared_balance: Money,
    pub ending_balance: Money,
    /// Ending minus cleared: zero when it balances.
    pub difference: Money,
    pub marked_count: u32,
}

/// An optional charge or interest line entered when starting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StatementCharge {
    pub amount: Money,
    pub date: jiff::civil::Date,
    pub account_id: AccountId,
}

fn account_type(c: &Connection, account: AccountId) -> Result<AccountType> {
    Ok(accounts::get(c, account)?.account_type)
}

/// What everything already reconciled adds up to, natural sign.
pub fn beginning_balance(c: &Connection, account: AccountId) -> Result<Money> {
    let t = account_type(c, account)?;
    let sum: i64 = c.query_row(
        "SELECT COALESCE(SUM(amount), 0) FROM posting WHERE account_id = ?1 AND cleared = 'Cleared'",
        [account.to_string()],
        |r| r.get(0),
    )?;
    Ok(t.natural(Money(sum)))
}

pub fn get(c: &Connection, id: ReconciliationId) -> Result<Reconciliation> {
    c.query_row(
        "SELECT doc FROM reconciliation WHERE id = ?1",
        [id.to_string()],
        |r| doc(r, 0),
    )
    .optional()?
    .ok_or_else(|| StoreError::NotFound {
        kind: "reconciliation",
        id: id.to_string(),
    })
}

/// The unfinished reconciliation of an account, if there is one.
pub fn in_progress(c: &Connection, account: AccountId) -> Result<Option<Reconciliation>> {
    Ok(c.query_row(
        "SELECT doc FROM reconciliation WHERE account_id = ?1 AND finished_at IS NULL",
        [account.to_string()],
        |r| doc(r, 0),
    )
    .optional()?)
}

/// Every finished reconciliation of an account, newest first.
pub fn history(c: &Connection, account: AccountId) -> Result<Vec<Reconciliation>> {
    let mut stmt = c.prepare(
        "SELECT doc FROM reconciliation WHERE account_id = ?1 AND finished_at IS NOT NULL
         ORDER BY statement_date DESC, finished_at DESC",
    )?;
    let rows = stmt.query_map([account.to_string()], |r| doc::<Reconciliation>(r, 0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn put(w: &Writer<'_>, r: &Reconciliation) -> Result<()> {
    w.tx.execute(
        "INSERT INTO reconciliation (id, account_id, statement_date, beginning_balance, ending_balance,
                                     finished_at, finished_by, doc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET statement_date = excluded.statement_date,
           beginning_balance = excluded.beginning_balance, ending_balance = excluded.ending_balance,
           finished_at = excluded.finished_at, finished_by = excluded.finished_by, doc = excluded.doc",
        params![
            r.id.to_string(),
            r.account_id.to_string(),
            SqlDate(r.statement_date),
            r.beginning_balance.minor(),
            r.ending_balance.minor(),
            r.finished_at.map(|t| t.to_string()),
            r.finished_by,
            codec::to_doc(r)?,
        ],
    )?;
    Ok(())
}

/// Begin reconciling `account` (or pick up the one in progress, updated with the
/// new statement figures). A service charge becomes a check (or charge, for a
/// card) to its account; interest becomes a deposit. Both are marked.
pub fn start(
    w: &mut Writer<'_>,
    account: AccountId,
    statement_date: jiff::civil::Date,
    ending_balance: Money,
    service_charge: Option<StatementCharge>,
    interest: Option<StatementCharge>,
) -> Result<Reconciliation> {
    let t = account_type(&w.tx, account)?;
    if !t.is_reconcilable() {
        return Err(StoreError::Invalid(format!(
            "{} accounts aren't reconciled",
            t.label()
        )));
    }
    let mut rec = match in_progress(&w.tx, account)? {
        Some(r) => r,
        None => Reconciliation {
            id: ReconciliationId::new(),
            account_id: account,
            statement_date,
            beginning_balance: beginning_balance(&w.tx, account)?,
            ending_balance,
            service_charge_txn_id: None,
            interest_txn_id: None,
            adjustment_txn_id: None,
            finished_at: None,
            finished_by: None,
            cleared_txn_ids: Vec::new(),
        },
    };
    rec.statement_date = statement_date;
    rec.ending_balance = ending_balance;
    if let Some(sc) = service_charge.filter(|s| !s.amount.is_zero()) {
        let txn_type = if t == AccountType::CreditCard {
            TxnType::CreditCardCharge
        } else {
            TxnType::Check
        };
        let mut txn = Txn::new(txn_type, sc.date);
        txn.account_id = Some(account);
        txn.memo = Some("Service Charge".into());
        txn.lines.push(TxnLine::account(sc.account_id, sc.amount));
        let saved = txns::save(w, txn)?;
        mark_txn(w, saved.txn.id, account, true)?;
        rec.service_charge_txn_id = Some(saved.txn.id);
    }
    if let Some(int) = interest.filter(|s| !s.amount.is_zero()) {
        let mut txn = Txn::new(TxnType::Deposit, int.date);
        txn.account_id = Some(account);
        txn.memo = Some("Interest".into());
        txn.lines.push(TxnLine::deposit(int.account_id, int.amount));
        let saved = txns::save(w, txn)?;
        mark_txn(w, saved.txn.id, account, true)?;
        rec.interest_txn_id = Some(saved.txn.id);
    }
    put(w, &rec)?;
    w.changed("reconciliation", rec.id.to_string(), "update");
    Ok(rec)
}

fn mark_txn(w: &mut Writer<'_>, txn: TxnId, account: AccountId, marked: bool) -> Result<()> {
    let n = w.tx.execute(
        "UPDATE posting SET cleared = ?3 WHERE txn_id = ?1 AND account_id = ?2 AND cleared <> 'Cleared'",
        params![txn.to_string(), account.to_string(), if marked { "Pending" } else { "Uncleared" }],
    )?;
    if n == 0 {
        return Err(StoreError::Invalid(
            "that transaction is already reconciled or isn't in this account".into(),
        ));
    }
    w.changed("txn", txn.to_string(), "cleared");
    Ok(())
}

/// Tick (or untick) transactions in the reconciliation in progress.
pub fn mark(
    w: &mut Writer<'_>,
    id: ReconciliationId,
    txn_ids: &[TxnId],
    marked: bool,
) -> Result<()> {
    let rec = get(&w.tx, id)?;
    if rec.finished_at.is_some() {
        return Err(StoreError::Invalid(
            "this reconciliation is finished".into(),
        ));
    }
    for t in txn_ids {
        mark_txn(w, *t, rec.account_id, marked)?;
    }
    Ok(())
}

/// The lines the Reconcile window shows: everything not yet reconciled, dated on
/// or before the statement date (and anything already ticked, whatever its date).
pub fn lines(c: &Connection, id: ReconciliationId) -> Result<Vec<ReconcileLine>> {
    let rec = get(c, id)?;
    let t = account_type(c, rec.account_id)?;
    let mut stmt = c.prepare(
        "SELECT t.id, t.txn_type, t.date, t.number, e.full_name, t.memo, SUM(p.amount), MAX(p.cleared = 'Pending')
         FROM posting p JOIN txn t ON t.id = p.txn_id LEFT JOIN entity e ON e.id = t.entity_id
         WHERE p.account_id = ?1 AND p.cleared <> 'Cleared' AND t.is_void = 0
           AND (p.date <= ?2 OR p.cleared = 'Pending')
         GROUP BY t.id
         HAVING SUM(p.amount) <> 0
         ORDER BY t.date, t.rowid",
    )?;
    let rows = stmt.query_map(
        params![rec.account_id.to_string(), SqlDate(rec.statement_date)],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, SqlDate>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, bool>(7)?,
            ))
        },
    )?;
    let mut out = Vec::new();
    for row in rows {
        let (id, tt, date, number, payee, memo, amount, marked) = row?;
        out.push(ReconcileLine {
            txn_id: codec::parse_id(&id)?,
            txn_type: TxnType::parse(&tt).ok_or_else(|| StoreError::Corrupt(tt.clone()))?,
            date: date.0,
            number,
            payee,
            memo,
            amount: t.natural(Money(amount)),
            marked,
        });
    }
    Ok(out)
}

pub fn status(c: &Connection, id: ReconciliationId) -> Result<ReconcileStatus> {
    let rec = get(c, id)?;
    let mut inc = Money::ZERO;
    let mut dec = Money::ZERO;
    let mut count = 0;
    for l in lines(c, id)?.into_iter().filter(|l| l.marked) {
        count += 1;
        if l.amount.is_negative() {
            dec += l.amount;
        } else {
            inc += l.amount;
        }
    }
    let cleared = rec.beginning_balance + inc + dec;
    Ok(ReconcileStatus {
        beginning_balance: rec.beginning_balance,
        marked_increases: inc,
        marked_decreases: dec,
        cleared_balance: cleared,
        ending_balance: rec.ending_balance,
        difference: rec.ending_balance - cleared,
        marked_count: count,
    })
}

/// Finish: ticked lines become cleared. A remaining difference is refused unless
/// `adjust`, which posts it to Reconciliation Discrepancies (as QBD's "Enter
/// Adjustment" does) dated the statement date.
pub fn finish(w: &mut Writer<'_>, id: ReconciliationId, adjust: bool) -> Result<Reconciliation> {
    let mut rec = get(&w.tx, id)?;
    if rec.finished_at.is_some() {
        return Err(StoreError::Invalid(
            "this reconciliation is already finished".into(),
        ));
    }
    let st = status(&w.tx, id)?;
    if !st.difference.is_zero() {
        if !adjust {
            return Err(StoreError::Invalid(format!(
                "the statement and the cleared balance differ by {}",
                st.difference
            )));
        }
        let t = account_type(&w.tx, rec.account_id)?;
        let discrepancy = accounts::ensure_role(w, AccountRole::ReconciliationDiscrepancies)?;
        // Raise the account's natural balance by `difference`.
        let debit = t.natural(st.difference);
        let mut je = Txn::new(TxnType::JournalEntry, rec.statement_date);
        je.memo = Some("Balance Adjustment".into());
        je.is_adjustment = true;
        je.lines.push(TxnLine::journal(rec.account_id, debit));
        je.lines.push(TxnLine::journal(discrepancy, -debit));
        let saved = txns::save(w, je)?;
        mark_txn(w, saved.txn.id, rec.account_id, true)?;
        rec.adjustment_txn_id = Some(saved.txn.id);
    }
    let rid = rec.id.to_string();
    let mut stmt = w.tx.prepare(
        "SELECT DISTINCT txn_id FROM posting WHERE account_id = ?1 AND cleared = 'Pending'",
    )?;
    let ids: Vec<String> = stmt
        .query_map([rec.account_id.to_string()], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);
    rec.cleared_txn_ids = ids
        .iter()
        .map(|s| codec::parse_id(s))
        .collect::<Result<_>>()?;
    w.tx.execute(
        "UPDATE posting SET cleared = 'Cleared', reconciliation_id = ?2 WHERE account_id = ?1 AND cleared = 'Pending'",
        params![rec.account_id.to_string(), rid],
    )?;
    rec.finished_at = Some(jiff::Timestamp::now());
    rec.finished_by.clone_from(&w.actor.user_id);
    put(w, &rec)?;
    crate::audit::record(
        w,
        "reconciliation",
        &rid,
        "finish",
        None::<&Reconciliation>,
        Some(&rec),
        None,
    )?;
    w.changed("reconciliation", rid, "finish");
    Ok(rec)
}

/// Undo the most recent finished reconciliation of an account: what it cleared
/// goes back to uncleared, and the reconciliation is forgotten. Transactions it
/// created (service charge, interest, adjustment) stay; the person deletes them if
/// wanted, as in QBD.
pub fn undo_last(w: &mut Writer<'_>, account: AccountId) -> Result<Option<Reconciliation>> {
    if in_progress(&w.tx, account)?.is_some() {
        return Err(StoreError::Invalid(
            "finish or discard the reconciliation in progress first".into(),
        ));
    }
    let Some(last) = history(&w.tx, account)?.into_iter().next() else {
        return Ok(None);
    };
    let rid = last.id.to_string();
    w.tx.execute(
        "UPDATE posting SET cleared = 'Uncleared', reconciliation_id = NULL WHERE reconciliation_id = ?1",
        [&rid],
    )?;
    w.tx.execute("DELETE FROM reconciliation WHERE id = ?1", [&rid])?;
    crate::audit::record(
        w,
        "reconciliation",
        &rid,
        "undo",
        Some(&last),
        None::<&Reconciliation>,
        None,
    )?;
    w.changed("reconciliation", rid, "delete");
    Ok(Some(last))
}

/// Abandon the reconciliation in progress, unticking everything ticked.
pub fn discard(w: &mut Writer<'_>, id: ReconciliationId) -> Result<()> {
    let rec = get(&w.tx, id)?;
    if rec.finished_at.is_some() {
        return Err(StoreError::Invalid(
            "a finished reconciliation is undone, not discarded".into(),
        ));
    }
    w.tx.execute(
        "UPDATE posting SET cleared = 'Uncleared' WHERE account_id = ?1 AND cleared = 'Pending'",
        [rec.account_id.to_string()],
    )?;
    w.tx.execute("DELETE FROM reconciliation WHERE id = ?1", [id.to_string()])?;
    w.changed("reconciliation", id.to_string(), "delete");
    Ok(())
}

/// Shorthand for tests and callers: the cleared mark of one transaction in one
/// account.
pub fn cleared_status(c: &Connection, txn: TxnId, account: AccountId) -> Result<ClearedStatus> {
    let s: String = c.query_row(
        "SELECT cleared FROM posting WHERE txn_id = ?1 AND account_id = ?2 LIMIT 1",
        params![txn.to_string(), account.to_string()],
        |r| r.get(0),
    )?;
    Ok(txns::parse_cleared(&s))
}
