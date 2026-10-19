//! The chart of accounts.

use lb_core::{Account, AccountId, AccountRole, AccountType, CashFlowClass};
use rusqlite::{Connection, OptionalExtension, params};

use crate::codec::{self, doc};
use crate::{Result, StoreError, Writer, audit, hier};

const COLS: &str = "doc";

/// All accounts in chart order (type, then number, then full name).
pub fn list(c: &Connection, include_inactive: bool) -> Result<Vec<Account>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {COLS} FROM account WHERE (?1 OR is_active) ORDER BY sort_key"
    ))?;
    let rows = stmt.query_map([include_inactive], |r| doc::<Account>(r, 0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(c: &Connection, id: AccountId) -> Result<Account> {
    find(c, id)?.ok_or_else(|| StoreError::NotFound {
        kind: "account",
        id: id.to_string(),
    })
}

pub fn find(c: &Connection, id: AccountId) -> Result<Option<Account>> {
    Ok(c.query_row(
        &format!("SELECT {COLS} FROM account WHERE id = ?1"),
        [id.to_string()],
        |r| doc(r, 0),
    )
    .optional()?)
}

pub fn by_full_name(c: &Connection, full_name: &str) -> Result<Option<Account>> {
    Ok(c.query_row(
        &format!("SELECT {COLS} FROM account WHERE full_name = ?1"),
        [full_name],
        |r| doc(r, 0),
    )
    .optional()?)
}

pub fn by_role(c: &Connection, role: AccountRole) -> Result<Option<Account>> {
    Ok(c.query_row(
        &format!("SELECT {COLS} FROM account WHERE role = ?1"),
        [role.as_str()],
        |r| doc(r, 0),
    )
    .optional()?)
}

/// The account playing `role`, created with QBD's usual name if there is none.
/// An existing account with that name and type is adopted rather than duplicated.
pub fn ensure_role(w: &mut Writer<'_>, role: AccountRole) -> Result<AccountId> {
    if let Some(a) = by_role(&w.tx, role)? {
        return Ok(a.id);
    }
    let (account_type, name) = role.default_shape();
    if let Some(mut existing) = by_full_name(&w.tx, name)?
        && existing.account_type == account_type
        && existing.role.is_none()
    {
        existing.role = Some(role);
        let id = existing.id;
        save(w, existing)?;
        return Ok(id);
    }
    let mut a = new_account(name, account_type);
    a.role = Some(role);
    let id = a.id;
    save(w, a)?;
    Ok(id)
}

/// A new top-level account with defaults.
pub fn new_account(name: &str, account_type: AccountType) -> Account {
    Account {
        id: AccountId::new(),
        name: name.to_owned(),
        full_name: name.to_owned(),
        parent_id: None,
        sublevel: 0,
        account_type,
        number: None,
        description: None,
        bank_account_number: None,
        tax_line: None,
        role: None,
        is_active: true,
        currency: None,
        cash_flow_class: Some(CashFlowClass::default_for(account_type)),
        edit_seq: 0,
    }
}

fn sort_key(a: &Account) -> String {
    format!(
        "{:02}|{}|{}",
        a.account_type.sort_order(),
        a.number.as_deref().unwrap_or("~"),
        a.full_name.to_lowercase()
    )
}

/// Create or update an account. `edit_seq` must match what is stored (0 for new).
pub fn save(w: &mut Writer<'_>, mut a: Account) -> Result<Account> {
    let existing = find(&w.tx, a.id)?;
    if let Some(e) = &existing {
        if e.edit_seq != a.edit_seq {
            return Err(StoreError::Conflict {
                kind: "account",
                id: a.id.to_string(),
                expected: a.edit_seq,
                actual: e.edit_seq,
            });
        }
        if e.account_type != a.account_type
            && hier::is_referenced(&w.tx, Some("account_id"), &a.id.to_string())?
        {
            // QBD allows some type changes; changing between balance-sheet and
            // income-statement classes rewrites history, so it is refused.
            if e.account_type.is_balance_sheet() != a.account_type.is_balance_sheet()
                || matches!(
                    e.account_type,
                    AccountType::AccountsReceivable | AccountType::AccountsPayable
                )
                || matches!(
                    a.account_type,
                    AccountType::AccountsReceivable | AccountType::AccountsPayable
                )
            {
                return Err(StoreError::Invalid(format!(
                    "an account with transactions can't change from {} to {}",
                    e.account_type.label(),
                    a.account_type.label()
                )));
            }
        }
    } else if a.edit_seq != 0 {
        return Err(StoreError::NotFound {
            kind: "account",
            id: a.id.to_string(),
        });
    }
    let id = a.id.to_string();
    let parent = a.parent_id.map(|p| p.to_string());
    hier::check_no_cycle(&w.tx, "account", &id, parent.as_deref())?;
    if let Some(p) = a.parent_id {
        let parent = get(&w.tx, p)?;
        if parent.account_type != a.account_type {
            return Err(StoreError::Invalid(format!(
                "a subaccount must be the same type as its parent ({})",
                parent.account_type.label()
            )));
        }
    }
    let (full, level) = hier::full_name(&w.tx, "account", parent.as_deref(), a.name.trim())?;
    a.name = a.name.trim().to_owned();
    a.full_name = full;
    a.sublevel = level;
    a.edit_seq += 1;
    let doc_text = codec::to_doc(&a)?;
    let renamed = existing
        .as_ref()
        .is_some_and(|e| e.full_name != a.full_name);
    w.tx.execute(
        "INSERT INTO account (id, parent_id, name, full_name, account_type, number, role, is_active, sort_key, edit_seq, doc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(id) DO UPDATE SET parent_id = excluded.parent_id, name = excluded.name,
           full_name = excluded.full_name, account_type = excluded.account_type, number = excluded.number,
           role = excluded.role, is_active = excluded.is_active, sort_key = excluded.sort_key,
           edit_seq = excluded.edit_seq, doc = excluded.doc",
        params![
            id,
            parent,
            a.name,
            a.full_name,
            a.account_type.as_str(),
            a.number,
            a.role.map(AccountRole::as_str),
            a.is_active,
            sort_key(&a),
            a.edit_seq,
            doc_text
        ],
    )
    .map_err(|e| hier::map_duplicate(e, "account", &a.full_name))?;
    if renamed {
        for child in hier::refresh_descendants(&w.tx, "account", &id)? {
            // Keep sort keys in step with the new full names.
            let acct: Account = c_get(&w.tx, &child)?;
            w.tx.execute(
                "UPDATE account SET sort_key = ?2 WHERE id = ?1",
                params![child, sort_key(&acct)],
            )?;
            w.changed("account", child, "update");
        }
    }
    let action = if existing.is_some() {
        "update"
    } else {
        "create"
    };
    audit::record(w, "account", &id, action, existing.as_ref(), Some(&a), None)?;
    w.changed("account", id, action);
    Ok(a)
}

fn c_get(c: &Connection, id: &str) -> Result<Account> {
    Ok(c.query_row("SELECT doc FROM account WHERE id = ?1", [id], |r| doc(r, 0))?)
}

/// Delete an account nothing uses. Accounts in use can only be made inactive.
pub fn delete(w: &mut Writer<'_>, id: AccountId, edit_seq: i64) -> Result<()> {
    let a = get(&w.tx, id)?;
    if a.edit_seq != edit_seq {
        return Err(StoreError::Conflict {
            kind: "account",
            id: id.to_string(),
            expected: edit_seq,
            actual: a.edit_seq,
        });
    }
    let sid = id.to_string();
    let has_children: bool = w.tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM account WHERE parent_id = ?1)",
        [&sid],
        |r| r.get(0),
    )?;
    if has_children || a.role.is_some() || hier::is_referenced(&w.tx, Some("account_id"), &sid)? {
        return Err(StoreError::InUse { kind: "account" });
    }
    let referenced_by_item: bool = w.tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM item WHERE doc LIKE ?1)",
        [format!("%{sid}%")],
        |r| r.get(0),
    )?;
    if referenced_by_item {
        return Err(StoreError::InUse { kind: "account" });
    }
    w.tx.execute("DELETE FROM account WHERE id = ?1", [&sid])?;
    audit::record::<Account, Account>(w, "account", &sid, "delete", Some(&a), None, None)?;
    w.changed("account", sid, "delete");
    Ok(())
}

/// Give a new balance-sheet account its opening balance as of `date`, the way
/// QBD's "Opening Balance" field does: a journal entry against Opening Balance
/// Equity. `natural` is in the account's own sense (a loan's balance owed is
/// positive). A/R and A/P get their opening balances from invoices and bills.
pub fn set_opening_balance(
    w: &mut Writer<'_>,
    id: AccountId,
    natural: lb_core::Money,
    date: jiff::civil::Date,
) -> Result<lb_core::TxnId> {
    let a = get(&w.tx, id)?;
    if !a.account_type.is_balance_sheet() {
        return Err(StoreError::Invalid(
            "only balance sheet accounts have opening balances".into(),
        ));
    }
    if a.account_type.requires_name() {
        return Err(StoreError::Invalid(
            "enter invoices or bills for what's owed instead of an opening balance".into(),
        ));
    }
    let obe = ensure_role(w, AccountRole::OpeningBalanceEquity)?;
    if obe == id {
        return Err(StoreError::Invalid(
            "Opening Balance Equity can't open against itself".into(),
        ));
    }
    let debit = a.account_type.natural(natural);
    let mut je = lb_core::Txn::new(lb_core::TxnType::JournalEntry, date);
    je.memo = Some("Opening Balance".into());
    je.lines.push(lb_core::TxnLine::journal(id, debit));
    je.lines.push(lb_core::TxnLine::journal(obe, -debit));
    Ok(crate::txns::save(w, je)?.txn.id)
}
