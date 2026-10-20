//! Adding a downloaded transaction to the books (QBD's "Add" and "Add All").
//!
//! The form follows the account and the direction, as QBD does:
//!
//! | Account | Money out | Money in |
//! | --- | --- | --- |
//! | bank (and other asset registers) | Check | Deposit |
//! | credit card | Credit Card Charge | Credit Card Credit |
//!
//! The transaction gets one line: the account on the other side (an expense, an
//! income account), with the name and class the person or a rule chose. It is
//! saved through `lb_store::txns::save` like any other transaction (so it posts,
//! balances, respects the closing date and is audited), then marked cleared in
//! the bank's register and linked to the download.

use lb_core::{AccountId, AccountType, ClassId, EntityId, Txn, TxnId, TxnLine, TxnType};
use lb_store::{Writer, accounts, balances, txns};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{FeedError, Result};
use crate::ids::FeedTxnId;
use crate::queue::{self, FeedStatus, FeedTxn};

/// What the person chose on the "Add" row. Anything left out comes from the
/// transaction's suggestion (rule or history).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct AddRequest {
    pub entity_id: Option<EntityId>,
    /// The account on the other side.
    pub account_id: Option<AccountId>,
    pub class_id: Option<ClassId>,
    pub memo: Option<String>,
    /// The check number (checks only); defaults to the bank's.
    pub number: Option<String>,
}

/// What adding one produced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AddOutcome {
    pub feed_txn_id: FeedTxnId,
    pub txn_id: TxnId,
    pub txn_type: TxnType,
    /// Soft problems from saving (a date far in the past, say).
    pub warnings: Vec<String>,
}

/// One that couldn't be added, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AddFailure {
    pub feed_txn_id: FeedTxnId,
    pub message: String,
}

/// What "Add All" did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BatchOutcome {
    pub added: Vec<AddOutcome>,
    pub failed: Vec<AddFailure>,
}

/// The form a download becomes in an account of this type.
pub fn txn_type_for(account_type: AccountType, money_in: bool) -> TxnType {
    match (account_type, money_in) {
        (AccountType::CreditCard, false) => TxnType::CreditCardCharge,
        (AccountType::CreditCard, true) => TxnType::CreditCardCredit,
        (_, false) => TxnType::Check,
        (_, true) => TxnType::Deposit,
    }
}

/// The transaction a download would become, without saving it (for a preview,
/// and the first half of [`add`]).
pub fn plan(c: &Connection, f: &FeedTxn, req: &AddRequest) -> Result<Txn> {
    if f.amount.is_zero() {
        return Err(FeedError::Invalid(
            "a zero-amount transaction can't be added; ignore it instead".into(),
        ));
    }
    let account = accounts::get(c, f.account_id)?;
    if !matches!(
        account.account_type,
        AccountType::Bank | AccountType::OtherCurrentAsset | AccountType::CreditCard
    ) {
        return Err(FeedError::Unsupported(format!(
            "adding downloaded transactions to a {} account; match them to transactions entered by hand",
            account.account_type.label()
        )));
    }
    let s = f.suggestion.clone().unwrap_or_default();
    let offset = req.account_id.or(s.account_id).ok_or_else(|| {
        FeedError::Invalid(format!(
            "choose an account for {} on {}",
            f.payee.as_deref().unwrap_or("this transaction"),
            f.posted
        ))
    })?;
    if offset == f.account_id {
        return Err(FeedError::Invalid(
            "the other account can't be the account the transaction is in".into(),
        ));
    }
    let entity = req.entity_id.or(s.entity_id);
    let class = req.class_id.or(s.class_id);
    let memo = req
        .memo
        .clone()
        .or(s.memo)
        .or_else(|| default_memo(f, entity.is_some()));
    let txn_type = txn_type_for(account.account_type, f.amount.is_positive());

    let mut txn = Txn::new(txn_type, f.posted);
    txn.account_id = Some(f.account_id);
    txn.memo.clone_from(&memo);
    let amount = f.amount.abs();
    let mut line = if txn_type == TxnType::Deposit {
        TxnLine::deposit(offset, amount)
    } else {
        TxnLine::account(offset, amount)
    };
    line.class_id = class;
    line.description.clone_from(&memo);
    match txn_type {
        // A deposit's header has no name; "Received From" is on the line.
        TxnType::Deposit => line.entity_id = entity,
        _ => txn.entity_id = entity,
    }
    if txn_type == TxnType::Check {
        txn.number = req
            .number
            .clone()
            .or_else(|| f.check_number.clone())
            .filter(|n| !n.trim().is_empty());
    }
    txn.lines.push(line);
    txn.amount = amount;
    txn.extra = Some(serde_json::json!({
        "feed": { "fitid": f.fitid, "source": f.source, "payee": f.payee },
    }));
    Ok(txn)
}

/// The bank's own words, when the person hasn't chosen a memo: the payee text
/// (if no name was chosen, so it isn't lost) and the bank's memo.
fn default_memo(f: &FeedTxn, has_name: bool) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    if !has_name && let Some(p) = f.payee.as_deref() {
        parts.push(p.trim());
    }
    if let Some(m) = f.memo.as_deref() {
        let m = m.trim();
        if !m.is_empty() && !parts.contains(&m) {
            parts.push(m);
        }
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Add one download as a new transaction, clear it, and link it.
pub fn add(w: &mut Writer<'_>, feed: FeedTxnId, req: &AddRequest) -> Result<AddOutcome> {
    let f = queue::get(&w.tx, feed)?;
    if f.status != FeedStatus::New {
        return Err(FeedError::Invalid(format!(
            "this downloaded transaction is already {}",
            f.status.as_str()
        )));
    }
    let txn = plan(&w.tx, &f, req)?;
    let txn_type = txn.txn_type;
    let saved = txns::save(w, txn)?;
    let id = saved.txn.id;
    balances::set_downloaded(w, id, f.account_id, true)?;
    queue::set_status(w, feed, FeedStatus::Added, Some(id))?;
    Ok(AddOutcome {
        feed_txn_id: feed,
        txn_id: id,
        txn_type,
        warnings: saved.warnings,
    })
}

/// "Add All": add each with its suggestion (and `req` on top, when given), one
/// at a time. One that fails (no account chosen, a closed period) is reported
/// and left waiting; the rest are still added.
pub fn add_all(
    w: &mut Writer<'_>,
    ids: &[FeedTxnId],
    req: Option<&AddRequest>,
) -> Result<BatchOutcome> {
    let empty = AddRequest::default();
    let req = req.unwrap_or(&empty);
    let mut out = BatchOutcome::default();
    for &id in ids {
        // A savepoint per transaction: a failure part-way through one save
        // leaves nothing of it behind, and doesn't undo the others.
        w.tx.execute_batch("SAVEPOINT lb_feed_add")?;
        match add(w, id, req) {
            Ok(o) => {
                w.tx.execute_batch("RELEASE lb_feed_add")?;
                out.added.push(o);
            }
            Err(e) => {
                w.tx.execute_batch("ROLLBACK TO lb_feed_add; RELEASE lb_feed_add")?;
                if matches!(e, FeedError::Store(lb_store::StoreError::Sqlite(_))) {
                    return Err(e);
                }
                out.failed.push(AddFailure {
                    feed_txn_id: id,
                    message: e.to_string(),
                });
            }
        }
    }
    Ok(out)
}
