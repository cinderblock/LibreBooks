//! Account balances and registers, read straight from the ledger.

use std::collections::HashMap;

use lb_core::{AccountId, ClearedStatus, EntityId, Money, TxnId, TxnType};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::codec::{self, SqlDate};
use crate::txns::parse_cleared;
use crate::{Result, StoreError};

/// One account's balance over a period, debit-positive. "Total" (with
/// subaccounts) is for callers to roll up; they hold the hierarchy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AccountBalance {
    pub account_id: AccountId,
    pub debit_positive: Money,
}

/// Debit-positive sums per account for postings dated within `[from, to]` (either
/// end open). For a balance sheet balance, pass `from = None`.
pub fn sums(
    c: &Connection,
    from: Option<jiff::civil::Date>,
    to: Option<jiff::civil::Date>,
) -> Result<HashMap<AccountId, Money>> {
    let mut stmt = c.prepare(
        "SELECT account_id, SUM(amount) FROM posting
         WHERE (?1 IS NULL OR date >= ?1) AND (?2 IS NULL OR date <= ?2)
         GROUP BY account_id",
    )?;
    let rows = stmt.query_map(params![from.map(SqlDate), to.map(SqlDate)], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (a, m) = row?;
        out.insert(codec::parse_id(&a)?, Money(m));
    }
    Ok(out)
}

/// One line of an account register (QBD's Ctrl+R window).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RegisterLine {
    pub txn_id: TxnId,
    pub txn_type: TxnType,
    pub date: jiff::civil::Date,
    pub number: Option<String>,
    pub payee_id: Option<EntityId>,
    pub payee: Option<String>,
    /// The account on the other side, or `None` with `is_split` when several.
    pub other_account_id: Option<AccountId>,
    pub other_account: Option<String>,
    pub is_split: bool,
    pub memo: Option<String>,
    /// This transaction's effect on the account, debit-positive.
    pub amount: Money,
    pub cleared: ClearedStatus,
    /// Running balance, debit-positive (the UI shows it naturally for the type).
    pub balance: Money,
    pub is_void: bool,
    pub edit_seq: i64,
    /// For A/R and A/P registers: due date and what is still open.
    pub due_date: Option<jiff::civil::Date>,
    pub open_balance: Option<Money>,
}

/// An account's register for a date range, oldest first, with the balance carried
/// in from before `from`. Several postings of one transaction in the same account
/// show as one line.
pub fn register(
    c: &Connection,
    account: AccountId,
    from: Option<jiff::civil::Date>,
    to: Option<jiff::civil::Date>,
) -> Result<(Money, Vec<RegisterLine>)> {
    let aid = account.to_string();
    let opening: i64 = match from {
        Some(f) => c.query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM posting WHERE account_id = ?1 AND date < ?2",
            params![aid, SqlDate(f)],
            |r| r.get(0),
        )?,
        None => 0,
    };
    let mut stmt = c.prepare(
        "SELECT t.id, t.txn_type, t.date, t.number, t.entity_id, e.full_name, t.memo, t.is_void, t.edit_seq,
                t.due_date, t.open_balance,
                SUM(p.amount), MIN(p.cleared), MIN(p.memo), MIN(p.entity_id),
                (SELECT COUNT(DISTINCT o.account_id) FROM posting o WHERE o.txn_id = t.id AND o.account_id <> ?1),
                (SELECT MIN(o.account_id) FROM posting o WHERE o.txn_id = t.id AND o.account_id <> ?1)
         FROM posting p JOIN txn t ON t.id = p.txn_id LEFT JOIN entity e ON e.id = t.entity_id
         WHERE p.account_id = ?1 AND (?2 IS NULL OR p.date >= ?2) AND (?3 IS NULL OR p.date <= ?3)
         GROUP BY t.id
         ORDER BY t.date, t.rowid",
    )?;
    let names = account_names(c)?;
    let entity_names = entity_names(c)?;
    let rows = stmt.query_map(params![aid, from.map(SqlDate), to.map(SqlDate)], |r| {
        Ok(RawLine {
            id: r.get(0)?,
            t: r.get(1)?,
            date: r.get(2)?,
            number: r.get(3)?,
            entity: r.get(4)?,
            entity_name: r.get(5)?,
            memo: r.get(6)?,
            is_void: r.get(7)?,
            edit_seq: r.get(8)?,
            due: r.get(9)?,
            open: r.get(10)?,
            amount: r.get(11)?,
            cleared: r.get(12)?,
            posting_memo: r.get(13)?,
            posting_entity: r.get(14)?,
            others: r.get(15)?,
            other: r.get(16)?,
        })
    })?;
    let mut balance = Money(opening);
    let mut out = Vec::new();
    for row in rows {
        let r = row?;
        balance += Money(r.amount);
        let other_account_id: Option<AccountId> = if r.others == 1 {
            r.other.as_deref().map(codec::parse_id).transpose()?
        } else {
            None
        };
        // In A/R and A/P the line's name is the posting's (a journal entry's line
        // names the customer even though the header has none).
        let payee_id_text = r.entity.or(r.posting_entity);
        let payee_id: Option<EntityId> =
            payee_id_text.as_deref().map(codec::parse_id).transpose()?;
        let payee = r
            .entity_name
            .or_else(|| payee_id.and_then(|p| entity_names.get(&p).cloned()));
        out.push(RegisterLine {
            txn_id: codec::parse_id(&r.id)?,
            txn_type: TxnType::parse(&r.t).ok_or_else(|| StoreError::Corrupt(r.t.clone()))?,
            date: r.date.0,
            number: r.number,
            payee_id,
            payee,
            other_account: other_account_id.and_then(|a| names.get(&a).cloned()),
            other_account_id,
            is_split: r.others > 1,
            memo: r.memo.or(r.posting_memo),
            amount: Money(r.amount),
            cleared: parse_cleared(&r.cleared),
            balance,
            is_void: r.is_void,
            edit_seq: r.edit_seq,
            due_date: r.due.map(|d| d.0),
            open_balance: r.open.map(Money),
        });
    }
    Ok((Money(opening), out))
}

struct RawLine {
    id: String,
    t: String,
    date: SqlDate,
    number: Option<String>,
    entity: Option<String>,
    entity_name: Option<String>,
    memo: Option<String>,
    is_void: bool,
    edit_seq: i64,
    due: Option<SqlDate>,
    open: Option<i64>,
    amount: i64,
    cleared: String,
    posting_memo: Option<String>,
    posting_entity: Option<String>,
    others: i64,
    other: Option<String>,
}

pub fn account_names(c: &Connection) -> Result<HashMap<AccountId, String>> {
    let mut stmt = c.prepare("SELECT id, full_name FROM account")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = HashMap::new();
    for row in rows {
        let (id, name) = row?;
        out.insert(codec::parse_id(&id)?, name);
    }
    Ok(out)
}

pub fn entity_names(c: &Connection) -> Result<HashMap<EntityId, String>> {
    let mut stmt = c.prepare("SELECT id, full_name FROM entity")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = HashMap::new();
    for row in rows {
        let (id, name) = row?;
        out.insert(codec::parse_id(&id)?, name);
    }
    Ok(out)
}

/// Set the cleared mark on a transaction's postings in one account (the register's
/// check column, and reconciliation).
pub fn set_cleared(
    w: &mut crate::Writer<'_>,
    txn: TxnId,
    account: AccountId,
    status: ClearedStatus,
) -> Result<()> {
    let n = w.tx.execute(
        "UPDATE posting SET cleared = ?3 WHERE txn_id = ?1 AND account_id = ?2",
        params![
            txn.to_string(),
            account.to_string(),
            crate::txns::cleared_str(status)
        ],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound {
            kind: "register line",
            id: txn.to_string(),
        });
    }
    w.changed("txn", txn.to_string(), "cleared");
    Ok(())
}
