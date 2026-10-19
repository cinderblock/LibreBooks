//! Saving, voiding, deleting and finding transactions.
//!
//! Saving is the heart of the store. In one SQLite transaction it:
//! 1. checks the edit sequence (someone else may have changed it) and the closing
//!    date (for both the old and the new date);
//! 2. fills in what QBD fills in (due date from terms, the default A/R or A/P);
//! 3. runs the posting rules, creating role accounts (Undeposited Funds, Sales Tax
//!    Payable…) the first time a company needs them;
//! 4. replaces the transaction's postings, applications and deposit links, keeping
//!    the cleared status of postings that did not change;
//! 5. recomputes the open balance of this transaction and every transaction it pays
//!    or used to pay;
//! 6. writes the audit record.

use std::collections::{BTreeSet, HashMap};

use lb_core::{
    AccountId, AccountRole, AccountType, ClearedStatus, EntityId, LineKind, Money, Posting,
    PostingError, Txn, TxnId, TxnType, post,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::codec::{self, SqlDate, doc};
use crate::context::StoreContext;
use crate::{Result, StoreError, Writer, accounts, audit, company, entities};

/// What saving produced, for the caller to show.
#[derive(Clone, Debug, Serialize)]
pub struct SaveOutcome {
    pub txn: Txn,
    pub postings: Vec<Posting>,
    pub total: Money,
    pub sales_tax: Money,
    /// Soft problems a person should see but that do not stop the save.
    pub warnings: Vec<String>,
}

pub fn find(c: &Connection, id: TxnId) -> Result<Option<Txn>> {
    Ok(
        c.query_row("SELECT doc FROM txn WHERE id = ?1", [id.to_string()], |r| {
            doc(r, 0)
        })
        .optional()?,
    )
}

pub fn get(c: &Connection, id: TxnId) -> Result<Txn> {
    find(c, id)?.ok_or_else(|| StoreError::NotFound {
        kind: "transaction",
        id: id.to_string(),
    })
}

pub fn postings(c: &Connection, id: TxnId) -> Result<Vec<Posting>> {
    let mut stmt = c.prepare(
        "SELECT account_id, amount, entity_id, class_id, item_id, line_id, memo, quantity, cleared
         FROM posting WHERE txn_id = ?1 ORDER BY seq",
    )?;
    let rows = stmt.query_map([id.to_string()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, String>(8)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (account, amount, entity, class, item, line, memo, qty, cleared) = row?;
        out.push(Posting {
            account_id: codec::parse_id(&account)?,
            amount: Money(amount),
            entity_id: entity.map(|s| codec::parse_id(&s)).transpose()?,
            class_id: class.map(|s| codec::parse_id(&s)).transpose()?,
            item_id: item.map(|s| codec::parse_id(&s)).transpose()?,
            line_id: line.map(|s| codec::parse_id(&s)).transpose()?,
            memo,
            quantity: qty.and_then(|q| q.parse().ok()),
            cleared: parse_cleared(&cleared),
        });
    }
    Ok(out)
}

pub fn parse_cleared(s: &str) -> ClearedStatus {
    match s {
        "Cleared" => ClearedStatus::Cleared,
        "Pending" => ClearedStatus::Pending,
        _ => ClearedStatus::Uncleared,
    }
}

pub fn cleared_str(c: ClearedStatus) -> &'static str {
    match c {
        ClearedStatus::Uncleared => "Uncleared",
        ClearedStatus::Pending => "Pending",
        ClearedStatus::Cleared => "Cleared",
    }
}

/// Create or update a transaction. `txn.edit_seq` must match what is stored (0
/// for a new one).
pub fn save(w: &mut Writer<'_>, mut txn: Txn) -> Result<SaveOutcome> {
    let existing = find(&w.tx, txn.id)?;
    match &existing {
        Some(x) => {
            if x.edit_seq != txn.edit_seq {
                return Err(StoreError::Conflict {
                    kind: "transaction",
                    id: txn.id.to_string(),
                    expected: txn.edit_seq,
                    actual: x.edit_seq,
                });
            }
            if x.txn_type != txn.txn_type {
                return Err(StoreError::Invalid(
                    "a transaction can't change type".into(),
                ));
            }
            company::check_open_period(w, x.date)?;
        }
        None if txn.edit_seq != 0 => {
            return Err(StoreError::NotFound {
                kind: "transaction",
                id: txn.id.to_string(),
            });
        }
        None => {}
    }
    company::check_open_period(w, txn.date)?;
    let mut warnings = Vec::new();
    fill_defaults(w, &mut txn, &mut warnings)?;
    check_applications(&w.tx, &txn)?;

    // A voided transaction pays nothing.
    if txn.is_void {
        txn.applications.clear();
    }

    // Post, creating role accounts on first use.
    let posted = loop {
        match post(&txn, &StoreContext::new(&w.tx)) {
            Ok(p) => break p,
            Err(PostingError::MissingRoleAccount(role)) => {
                accounts::ensure_role(w, role)?;
            }
            Err(e) => return Err(e.into()),
        }
    };

    // Resolve the A/R or A/P account the posting rules defaulted to, so the
    // stored transaction says exactly where it sits.
    if uses_ar_ap(txn.txn_type) && txn.ar_ap_account_id.is_none() {
        let role = if is_sales_side(txn.txn_type) {
            AccountRole::AccountsReceivable
        } else {
            AccountRole::AccountsPayable
        };
        txn.ar_ap_account_id = Some(accounts::ensure_role(w, role)?);
    }
    if txn.txn_type.is_posting() || txn.amount.is_zero() {
        txn.amount = posted.total;
    }

    let old_targets: BTreeSet<TxnId> = existing.as_ref().map(touched_txns).unwrap_or_default();
    let old_cleared = existing_cleared(&w.tx, txn.id)?;

    let now = crate::now();
    let user = w.actor.user_id.clone();
    let created_at = existing
        .as_ref()
        .and_then(|x| x.created_at)
        .or(txn.created_at)
        .unwrap_or_else(jiff::Timestamp::now);
    txn.created_at = Some(created_at);
    if existing.is_none() {
        txn.created_by
            .clone_from(&user.as_ref().and_then(|u| u.parse().ok()));
    }
    txn.modified_at = Some(jiff::Timestamp::now());
    txn.modified_by = user.as_ref().and_then(|u| u.parse().ok());
    txn.edit_seq += 1;

    let id = txn.id.to_string();
    w.tx.execute(
        "INSERT INTO txn (id, txn_type, date, number, entity_id, account_id, ar_ap_account_id, amount, sales_tax,
                          open_balance, due_date, is_void, is_pending, memo, edit_seq, created_at, modified_at,
                          created_by, modified_by, doc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
         ON CONFLICT(id) DO UPDATE SET date = excluded.date, number = excluded.number,
           entity_id = excluded.entity_id, account_id = excluded.account_id,
           ar_ap_account_id = excluded.ar_ap_account_id, amount = excluded.amount, sales_tax = excluded.sales_tax,
           due_date = excluded.due_date, is_void = excluded.is_void, is_pending = excluded.is_pending,
           memo = excluded.memo, edit_seq = excluded.edit_seq, modified_at = excluded.modified_at,
           modified_by = excluded.modified_by, doc = excluded.doc",
        params![
            id,
            txn.txn_type.as_str(),
            SqlDate(txn.date),
            txn.number,
            txn.entity_id.map(|e| e.to_string()),
            txn.account_id.map(|a| a.to_string()),
            txn.ar_ap_account_id.map(|a| a.to_string()),
            txn.amount.minor(),
            posted.sales_tax.minor(),
            codec::opt_date(txn.due_date),
            txn.is_void,
            txn.is_pending,
            txn.memo,
            txn.edit_seq,
            created_at.to_string(),
            now,
            txn.created_by.map(|u| u.to_string()),
            txn.modified_by.map(|u| u.to_string()),
            codec::to_doc(&txn)?,
        ],
    )?;

    // Postings, keeping cleared status where the same account/line kept its amount.
    w.tx.execute("DELETE FROM posting WHERE txn_id = ?1", [&id])?;
    let mut postings = posted.postings.clone();
    for (seq, p) in postings.iter_mut().enumerate() {
        if let Some(prev) = old_cleared.get(&(
            p.account_id,
            p.line_id.map(|l| l.to_string()),
            p.amount.minor(),
        )) {
            p.cleared = *prev;
        }
        w.tx.execute(
            "INSERT INTO posting (txn_id, seq, date, txn_type, account_id, amount, entity_id, class_id, item_id,
                                  line_id, memo, quantity, cleared)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id,
                seq as i64,
                SqlDate(txn.date),
                txn.txn_type.as_str(),
                p.account_id.to_string(),
                p.amount.minor(),
                p.entity_id.map(|e| e.to_string()),
                p.class_id.map(|c| c.to_string()),
                p.item_id.map(|i| i.to_string()),
                p.line_id.map(|l| l.to_string()),
                p.memo,
                p.quantity.map(|q| q.to_string()),
                cleared_str(p.cleared),
            ],
        )?;
    }

    // Applications and deposit links.
    w.tx.execute("DELETE FROM application WHERE txn_id = ?1", [&id])?;
    for (seq, a) in txn.applications.iter().enumerate() {
        w.tx.execute(
            "INSERT INTO application (txn_id, seq, target_txn_id, date, amount, discount, discount_account_id,
                                      credit_txn_id, credit_amount)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                seq as i64,
                a.target_txn_id.to_string(),
                SqlDate(txn.date),
                a.amount.minor(),
                a.discount.minor(),
                a.discount_account_id.map(|x| x.to_string()),
                a.credit_txn_id.map(|x| x.to_string()),
                a.credit_amount.minor(),
            ],
        )?;
    }
    w.tx.execute("DELETE FROM deposit_link WHERE deposit_txn_id = ?1", [&id])?;
    if txn.txn_type == TxnType::Deposit && !txn.is_void {
        for line in &txn.lines {
            if let (LineKind::DepositLine, Some(pmt)) = (line.kind, line.linked_txn_id) {
                let other: Option<String> = w
                    .tx
                    .query_row(
                        "SELECT deposit_txn_id FROM deposit_link WHERE payment_txn_id = ?1 AND deposit_txn_id <> ?2",
                        params![pmt.to_string(), id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if other.is_some() {
                    return Err(StoreError::Invalid(
                        "that payment is already in another deposit".into(),
                    ));
                }
                w.tx.execute(
                    "INSERT INTO deposit_link (deposit_txn_id, line_id, payment_txn_id) VALUES (?1, ?2, ?3)",
                    params![id, line.id.to_string(), pmt.to_string()],
                )?;
            }
        }
    }

    // Open balances: this transaction, and everything it pays now or paid before.
    let mut affected = touched_txns(&txn);
    affected.extend(old_targets);
    affected.insert(txn.id);
    for t in affected {
        refresh_open_balance(&w.tx, t)?;
    }

    let action = if existing.is_some() {
        "update"
    } else {
        "create"
    };
    audit::record(w, "txn", &id, action, existing.as_ref(), Some(&txn), None)?;
    w.changed("txn", id, action);

    Ok(SaveOutcome {
        txn,
        postings,
        total: posted.total,
        sales_tax: posted.sales_tax,
        warnings,
    })
}

/// Void: keep the transaction, zero its amounts, unapply it, note it in the memo.
pub fn void(w: &mut Writer<'_>, id: TxnId, edit_seq: i64) -> Result<SaveOutcome> {
    let mut txn = get(&w.tx, id)?;
    if txn.edit_seq != edit_seq {
        return Err(StoreError::Conflict {
            kind: "transaction",
            id: id.to_string(),
            expected: edit_seq,
            actual: txn.edit_seq,
        });
    }
    if txn.is_void {
        return Err(StoreError::Invalid(
            "this transaction is already void".into(),
        ));
    }
    let paid_by: bool = w.tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM application WHERE target_txn_id = ?1 OR credit_txn_id = ?1)",
        [id.to_string()],
        |r| r.get(0),
    )?;
    if paid_by {
        return Err(StoreError::Invalid(
            "payments or credits are applied to this transaction; remove them first".into(),
        ));
    }
    txn.is_void = true;
    txn.memo = Some(match txn.memo.take() {
        Some(m) if !m.starts_with("VOID:") => format!("VOID: {m}"),
        Some(m) => m,
        None => "VOID:".to_owned(),
    });
    let out = save(w, txn)?;
    audit::record::<Txn, Txn>(w, "txn", &id.to_string(), "void", None, None, None)?;
    Ok(out)
}

/// Delete a transaction outright (QBD's Ctrl+D). Refused while something depends
/// on it (a payment applied to it, a deposit holding it).
pub fn delete(w: &mut Writer<'_>, id: TxnId, edit_seq: i64) -> Result<()> {
    let txn = get(&w.tx, id)?;
    if txn.edit_seq != edit_seq {
        return Err(StoreError::Conflict {
            kind: "transaction",
            id: id.to_string(),
            expected: edit_seq,
            actual: txn.edit_seq,
        });
    }
    company::check_open_period(w, txn.date)?;
    let sid = id.to_string();
    let depended_on: bool = w.tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM application WHERE target_txn_id = ?1 OR credit_txn_id = ?1)
             OR EXISTS (SELECT 1 FROM deposit_link WHERE payment_txn_id = ?1)",
        [&sid],
        |r| r.get(0),
    )?;
    if depended_on {
        return Err(StoreError::Invalid(
            "other transactions pay, use or deposit this one; remove them from it first".into(),
        ));
    }
    let targets = touched_txns(&txn);
    w.tx.execute("DELETE FROM txn WHERE id = ?1", [&sid])?;
    w.tx.execute(
        "DELETE FROM external_ref WHERE local_kind = 'txn' AND local_id = ?1",
        [&sid],
    )?;
    for t in targets {
        refresh_open_balance(&w.tx, t)?;
    }
    audit::record::<Txn, Txn>(w, "txn", &sid, "delete", Some(&txn), None, None)?;
    w.changed("txn", sid, "delete");
    Ok(())
}

/// Every transaction this one pays or uses as a credit.
fn touched_txns(txn: &Txn) -> BTreeSet<TxnId> {
    let mut s = BTreeSet::new();
    for a in &txn.applications {
        s.insert(a.target_txn_id);
        if let Some(c) = a.credit_txn_id {
            s.insert(c);
        }
    }
    s
}

fn existing_cleared(
    c: &Connection,
    id: TxnId,
) -> Result<HashMap<(AccountId, Option<String>, i64), ClearedStatus>> {
    let mut stmt =
        c.prepare("SELECT account_id, line_id, amount, cleared FROM posting WHERE txn_id = ?1")?;
    let rows = stmt.query_map([id.to_string()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (a, l, amt, cl) = row?;
        let status = parse_cleared(&cl);
        if status != ClearedStatus::Uncleared {
            out.insert((codec::parse_id(&a)?, l, amt), status);
        }
    }
    Ok(out)
}

fn uses_ar_ap(t: TxnType) -> bool {
    matches!(
        t,
        TxnType::Invoice
            | TxnType::CreditMemo
            | TxnType::StatementCharge
            | TxnType::ReceivePayment
            | TxnType::Bill
            | TxnType::VendorCredit
            | TxnType::ItemReceipt
            | TxnType::BillPaymentCheck
            | TxnType::BillPaymentCreditCard
    )
}

fn is_sales_side(t: TxnType) -> bool {
    matches!(
        t,
        TxnType::Invoice | TxnType::CreditMemo | TxnType::StatementCharge | TxnType::ReceivePayment
    )
}

/// What QBD fills in on its own.
fn fill_defaults(w: &Writer<'_>, txn: &mut Txn, warnings: &mut Vec<String>) -> Result<()> {
    if let (None, Some(terms_id)) = (txn.due_date, txn.terms_id)
        && let Some(terms) =
            crate::lists::find::<lb_core::lists::Terms>(&w.tx, &terms_id.to_string())?
    {
        txn.due_date = Some(terms.due_date(txn.date));
        if txn.discount_date.is_none() {
            txn.discount_date = terms.discount_date(txn.date);
        }
    }
    if txn.due_date.is_none()
        && matches!(
            txn.txn_type,
            TxnType::Invoice | TxnType::Bill | TxnType::StatementCharge
        )
    {
        txn.due_date = Some(txn.date);
    }
    // Date warnings, as QBD's preferences give them.
    let info = company::info(&w.tx)?;
    let today = jiff::Zoned::now().date();
    if let Some(days) = info.preferences.warn_days_past
        && let Ok(span) = txn.date.until(today)
        && span.get_days() > days as i32
    {
        warnings.push(format!("the date is more than {days} days in the past"));
    }
    if let Some(days) = info.preferences.warn_days_future
        && let Ok(span) = today.until(txn.date)
        && span.get_days() > days as i32
    {
        warnings.push(format!("the date is more than {days} days in the future"));
    }
    // Lines in the same register as the header would post to themselves.
    if let Some(acct) = txn.account_id
        && txn
            .lines
            .iter()
            .any(|l| l.account_id == Some(acct) && l.kind != LineKind::CashBack)
        && matches!(
            txn.txn_type,
            TxnType::Check | TxnType::Deposit | TxnType::CreditCardCharge
        )
    {
        warnings.push("a line posts to the same account the transaction is in".into());
    }
    Ok(())
}

/// The store's half of payment checks: the names must match (a customer may pay
/// its own jobs' invoices, and a job's payment may pay its parent's), and the
/// targets must be the right kind of transaction.
fn check_applications(c: &Connection, txn: &Txn) -> Result<()> {
    if txn.applications.is_empty() {
        return Ok(());
    }
    let allowed: &[TxnType] = match txn.txn_type {
        TxnType::ReceivePayment => &[
            TxnType::Invoice,
            TxnType::StatementCharge,
            TxnType::JournalEntry,
        ],
        TxnType::BillPaymentCheck | TxnType::BillPaymentCreditCard => {
            &[TxnType::Bill, TxnType::JournalEntry]
        }
        _ => {
            return Err(StoreError::Invalid(format!(
                "a {} can't pay other transactions",
                txn.txn_type.label()
            )));
        }
    };
    let payer = txn
        .entity_id
        .ok_or_else(|| StoreError::Invalid("choose who the payment is from or to".into()))?;
    let root = entities::root_of(c, payer)?;
    for a in &txn.applications {
        let target = get(c, a.target_txn_id)?;
        if !allowed.contains(&target.txn_type) {
            return Err(StoreError::Invalid(format!(
                "a {} can't pay a {}",
                txn.txn_type.label(),
                target.txn_type.label()
            )));
        }
        if let Some(e) = target.entity_id
            && entities::root_of(c, e)? != root
        {
            return Err(PostingError::TargetForOtherName(a.target_txn_id).into());
        }
        if let Some(credit) = a.credit_txn_id {
            let ct = get(c, credit)?;
            let ok = matches!(
                (txn.txn_type, ct.txn_type),
                (
                    TxnType::ReceivePayment,
                    TxnType::CreditMemo | TxnType::ReceivePayment
                ) | (
                    TxnType::BillPaymentCheck | TxnType::BillPaymentCreditCard,
                    TxnType::VendorCredit
                )
            );
            if !ok {
                return Err(StoreError::Invalid(format!(
                    "a {} can't be used as a credit here",
                    ct.txn_type.label()
                )));
            }
        }
    }
    Ok(())
}

/// Recompute what is still open on `id` (owed on an invoice or bill, unused on a
/// credit or an overpayment). Types that are never open get NULL.
pub fn refresh_open_balance(c: &Connection, id: TxnId) -> Result<()> {
    let Some(txn) = find(c, id)? else {
        return Ok(());
    };
    let sid = id.to_string();
    let sum = |sql: &str| -> Result<i64> {
        Ok(c.query_row(sql, [&sid], |r| r.get::<_, Option<i64>>(0))?
            .unwrap_or(0))
    };
    let open: Option<i64> = if txn.is_void || txn.is_pending {
        if txn.txn_type.is_posting() {
            Some(0)
        } else {
            None
        }
    } else {
        match txn.txn_type {
            TxnType::Invoice | TxnType::StatementCharge | TxnType::Bill => {
                let paid = sum(
                    "SELECT SUM(amount + discount + credit_amount) FROM application WHERE target_txn_id = ?1",
                )?;
                Some(txn.amount.minor() - paid)
            }
            TxnType::CreditMemo | TxnType::VendorCredit => {
                let used =
                    sum("SELECT SUM(credit_amount) FROM application WHERE credit_txn_id = ?1")?;
                Some(txn.amount.minor() - used)
            }
            TxnType::ReceivePayment
            | TxnType::BillPaymentCheck
            | TxnType::BillPaymentCreditCard => {
                let applied = sum("SELECT SUM(amount) FROM application WHERE txn_id = ?1")?;
                let used =
                    sum("SELECT SUM(credit_amount) FROM application WHERE credit_txn_id = ?1")?;
                Some(txn.amount.minor() - applied - used)
            }
            _ => None,
        }
    };
    c.execute(
        "UPDATE txn SET open_balance = ?2 WHERE id = ?1",
        params![sid, open],
    )?;
    Ok(())
}

/// A row in a transaction list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TxnSummary {
    pub id: TxnId,
    pub txn_type: TxnType,
    pub date: jiff::civil::Date,
    pub number: Option<String>,
    pub entity_id: Option<EntityId>,
    pub entity_name: Option<String>,
    pub account_id: Option<AccountId>,
    pub account_name: Option<String>,
    pub amount: Money,
    pub open_balance: Option<Money>,
    pub due_date: Option<jiff::civil::Date>,
    pub memo: Option<String>,
    pub is_void: bool,
    pub is_pending: bool,
    pub edit_seq: i64,
}

/// Filters for finding transactions (QBD's Find window).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct TxnFilter {
    pub types: Vec<TxnType>,
    pub from: Option<jiff::civil::Date>,
    pub to: Option<jiff::civil::Date>,
    /// This name, and with `include_jobs`, every job under it.
    pub entity_id: Option<EntityId>,
    pub include_jobs: bool,
    /// Transactions with a posting in this account.
    pub account_id: Option<AccountId>,
    pub number: Option<String>,
    /// Matches memo, number or the payee's name.
    pub text: Option<String>,
    pub amount: Option<Money>,
    pub open_only: bool,
    pub include_void: bool,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    /// Newest first when true.
    pub descending: bool,
}

pub fn query(c: &Connection, f: &TxnFilter) -> Result<Vec<TxnSummary>> {
    let mut q = Query::new(
        "SELECT t.id, t.txn_type, t.date, t.number, t.entity_id, e.full_name, t.account_id, a.full_name,
                t.amount, t.open_balance, t.due_date, t.memo, t.is_void, t.is_pending, t.edit_seq
         FROM txn t LEFT JOIN entity e ON e.id = t.entity_id
                    LEFT JOIN account a ON a.id = COALESCE(t.account_id, t.ar_ap_account_id)
         WHERE 1 = 1",
    );
    if !f.types.is_empty() {
        let names: Vec<String> = f.types.iter().map(|t| t.as_str().to_owned()).collect();
        q.push_in(" AND t.txn_type IN ", names);
    }
    if let Some(d) = f.from {
        let n = q.bind(SqlDate(d));
        q.sql.push_str(&format!(" AND t.date >= ?{n}"));
    }
    if let Some(d) = f.to {
        let n = q.bind(SqlDate(d));
        q.sql.push_str(&format!(" AND t.date <= ?{n}"));
    }
    if let Some(e) = f.entity_id {
        let ids: Vec<String> = if f.include_jobs {
            entities::with_descendants(c, e)?
                .iter()
                .map(ToString::to_string)
                .collect()
        } else {
            vec![e.to_string()]
        };
        let list = q.bind_list(ids);
        q.sql.push_str(&format!(
            " AND (t.entity_id IN {list} OR EXISTS (SELECT 1 FROM posting p WHERE p.txn_id = t.id AND p.entity_id IN {list}))"
        ));
    }
    if let Some(a) = f.account_id {
        let n = q.bind(a.to_string());
        q.sql.push_str(&format!(
            " AND EXISTS (SELECT 1 FROM posting p WHERE p.txn_id = t.id AND p.account_id = ?{n})"
        ));
    }
    if let Some(num) = &f.number {
        let n = q.bind(num.clone());
        q.sql.push_str(&format!(" AND t.number = ?{n}"));
    }
    if let Some(text) = &f.text {
        // `!` escapes LIKE's wildcards in what the person typed.
        let like = format!(
            "%{}%",
            text.replace('!', "!!")
                .replace('%', "!%")
                .replace('_', "!_")
        );
        let n = q.bind(like);
        q.sql.push_str(&format!(
            " AND (t.memo LIKE ?{n} ESCAPE '!' OR t.number LIKE ?{n} ESCAPE '!' OR e.full_name LIKE ?{n} ESCAPE '!')"
        ));
    }
    if let Some(m) = f.amount {
        let n = q.bind(m.minor().abs());
        q.sql.push_str(&format!(" AND abs(t.amount) = ?{n}"));
    }
    if f.open_only {
        q.sql
            .push_str(" AND t.open_balance IS NOT NULL AND t.open_balance <> 0");
    }
    if !f.include_void {
        q.sql.push_str(" AND t.is_void = 0");
    }
    q.sql.push_str(if f.descending {
        " ORDER BY t.date DESC, t.rowid DESC"
    } else {
        " ORDER BY t.date, t.rowid"
    });
    q.sql.push_str(&format!(
        " LIMIT {} OFFSET {}",
        f.limit.unwrap_or(500).min(10_000),
        f.offset.unwrap_or(0)
    ));
    let mut stmt = c.prepare(&q.sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = q.args.iter().map(AsRef::as_ref).collect();
    let rows = stmt.query_map(params.as_slice(), summary_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r??);
    }
    Ok(out)
}

/// SQL text with numbered parameters bound as it is built.
pub(crate) struct Query {
    pub sql: String,
    pub args: Vec<Box<dyn rusqlite::ToSql>>,
}

impl Query {
    pub fn new(sql: &str) -> Query {
        Query {
            sql: sql.to_owned(),
            args: Vec::new(),
        }
    }

    /// Bind a value; returns its parameter number for `?N`.
    pub fn bind(&mut self, v: impl rusqlite::ToSql + 'static) -> usize {
        self.args.push(Box::new(v));
        self.args.len()
    }

    /// Bind several values; returns `(?a, ?b, …)` text.
    pub fn bind_list(&mut self, vs: Vec<String>) -> String {
        if vs.is_empty() {
            return "(NULL)".to_owned();
        }
        let nums: Vec<String> = vs
            .into_iter()
            .map(|v| format!("?{}", self.bind(v)))
            .collect();
        format!("({})", nums.join(", "))
    }

    pub fn push_in(&mut self, prefix: &str, vs: Vec<String>) {
        let list = self.bind_list(vs);
        self.sql.push_str(prefix);
        self.sql.push_str(&list);
    }
}

#[allow(clippy::type_complexity)]
fn summary_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<TxnSummary>> {
    let id: String = r.get(0)?;
    let t: String = r.get(1)?;
    let date: SqlDate = r.get(2)?;
    let entity: Option<String> = r.get(4)?;
    let account: Option<String> = r.get(6)?;
    let due: Option<SqlDate> = r.get(10)?;
    let build = || -> Result<TxnSummary> {
        Ok(TxnSummary {
            id: codec::parse_id(&id)?,
            txn_type: TxnType::parse(&t).ok_or_else(|| StoreError::Corrupt(format!("type {t}")))?,
            date: date.0,
            number: r.get(3)?,
            entity_id: entity.as_deref().map(codec::parse_id).transpose()?,
            entity_name: r.get(5)?,
            account_id: account.as_deref().map(codec::parse_id).transpose()?,
            account_name: r.get(7)?,
            amount: Money(r.get(8)?),
            open_balance: r.get::<_, Option<i64>>(9)?.map(Money),
            due_date: due.map(|d| d.0),
            memo: r.get(11)?,
            is_void: r.get(12)?,
            is_pending: r.get(13)?,
            edit_seq: r.get(14)?,
        })
    };
    Ok(build())
}

/// The next number to suggest: one more than the highest numeric number used by
/// this type (for checks and bill payments, from this bank account).
pub fn next_number(
    c: &Connection,
    txn_type: TxnType,
    account: Option<AccountId>,
) -> Result<Option<String>> {
    let types: &[TxnType] = match txn_type {
        TxnType::Check
        | TxnType::BillPaymentCheck
        | TxnType::Paycheck
        | TxnType::LiabilityCheck
        | TxnType::SalesTaxPaymentCheck => &[
            TxnType::Check,
            TxnType::BillPaymentCheck,
            TxnType::Paycheck,
            TxnType::LiabilityCheck,
            TxnType::SalesTaxPaymentCheck,
        ],
        TxnType::Invoice
        | TxnType::CreditMemo
        | TxnType::SalesReceipt
        | TxnType::StatementCharge => {
            &[TxnType::Invoice, TxnType::CreditMemo, TxnType::SalesReceipt]
        }
        _ => &[],
    };
    let types: Vec<TxnType> = if types.is_empty() {
        vec![txn_type]
    } else {
        types.to_vec()
    };
    let list = types
        .iter()
        .map(|t| format!("'{}'", t.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let by_account = matches!(
        txn_type,
        TxnType::Check
            | TxnType::BillPaymentCheck
            | TxnType::Paycheck
            | TxnType::LiabilityCheck
            | TxnType::SalesTaxPaymentCheck
    );
    let sql = format!(
        "SELECT number FROM txn WHERE txn_type IN ({list}) AND number GLOB '[0-9]*' AND (?1 IS NULL OR account_id = ?1)
         ORDER BY CAST(number AS INTEGER) DESC, length(number) DESC LIMIT 1"
    );
    let last: Option<String> = c
        .query_row(
            &sql,
            [if by_account {
                account.map(|a| a.to_string())
            } else {
                None
            }],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match last {
        Some(n) => increment(&n),
        None if txn_type == TxnType::JournalEntry => Some("1".into()),
        None => None,
    })
}

/// "1041" → "1042", "INV-0099" → "INV-0100", keeping leading zeros' width.
fn increment(n: &str) -> Option<String> {
    let digits_start = n.rfind(|c: char| !c.is_ascii_digit()).map_or(0, |i| i + 1);
    let (prefix, digits) = n.split_at(digits_start);
    if digits.is_empty() {
        return None;
    }
    let value: u128 = digits.parse().ok()?;
    Some(format!(
        "{prefix}{:0width$}",
        value + 1,
        width = digits.len()
    ))
}

/// Accounts of a type, for pickers (e.g. bank accounts for Write Checks).
pub fn accounts_of_types(c: &Connection, types: &[AccountType]) -> Result<Vec<AccountId>> {
    let list = types
        .iter()
        .map(|t| format!("'{}'", t.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let mut stmt = c.prepare(&format!(
        "SELECT id FROM account WHERE is_active AND account_type IN ({list}) ORDER BY sort_key"
    ))?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.map(|s| codec::parse_id(&s?)).collect()
}

#[cfg(test)]
mod tests {
    use super::increment;

    #[test]
    fn increments_numbers() {
        assert_eq!(increment("1041").as_deref(), Some("1042"));
        assert_eq!(increment("INV-0099").as_deref(), Some("INV-0100"));
        assert_eq!(increment("0009").as_deref(), Some("0010"));
        assert_eq!(increment("999").as_deref(), Some("1000"));
        assert_eq!(increment("ABC"), None);
    }
}
