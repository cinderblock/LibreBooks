//! Merging two list entries into one, as QBD does when you rename a name,
//! account or item to one that already exists: everything that pointed at the
//! one being merged away points at the survivor, and the merged one is deleted.
//!
//! Records refer to each other by UUID, both in indexed columns and inside the
//! JSON documents, so a merge rewrites the columns and replaces the id's text in
//! every document. A UUID can't occur in a document by accident, which is what
//! makes the textual replacement safe. All of it happens in one write
//! transaction, with an audit entry naming both.

use lb_core::{AccountId, EntityId, ItemId};
use rusqlite::params;

use crate::{Result, StoreError, Writer, accounts, audit, entities, hier, items};

/// Every table whose JSON `doc` may mention another record's id.
const DOC_TABLES: [&str; 8] = [
    "txn",
    "account",
    "entity",
    "item",
    "list_entry",
    "saved",
    "feed_connection",
    "feed_rule",
];

fn replace_in_docs(w: &Writer<'_>, from: &str, into: &str) -> Result<usize> {
    let mut n = 0;
    let pattern = format!("%{from}%");
    for t in DOC_TABLES {
        n += w.tx.execute(
            &format!("UPDATE {t} SET doc = replace(doc, ?1, ?2) WHERE doc LIKE ?3"),
            params![from, into, pattern],
        )?;
    }
    n += w.tx.execute(
        "UPDATE feed_txn SET doc = replace(doc, ?1, ?2) WHERE doc LIKE ?3",
        params![from, into, pattern],
    )?;
    Ok(n)
}

fn repoint(w: &Writer<'_>, sql: &[&str], from: &str, into: &str) -> Result<()> {
    for s in sql {
        w.tx.execute(s, params![from, into])?;
    }
    Ok(())
}

/// Bump the edit sequence of every transaction that mentions `id`, so an open
/// form holding the old version can't save over the merge.
fn bump_txns(w: &Writer<'_>, id: &str) -> Result<()> {
    w.tx.execute(
        "UPDATE txn SET edit_seq = edit_seq + 1,
                        doc = json_set(doc, '$.edit_seq', json_extract(doc, '$.edit_seq') + 1)
         WHERE doc LIKE ?1",
        [format!("%{id}%")],
    )?;
    Ok(())
}

/// Merge the name `from` into `into`. Both must be in the same list; jobs of
/// `from` move under `into`.
pub fn merge_entities(w: &mut Writer<'_>, from: EntityId, into: EntityId) -> Result<()> {
    if from == into {
        return Err(StoreError::Invalid(
            "a name can't be merged into itself".into(),
        ));
    }
    let a = entities::get(&w.tx, from)?;
    let b = entities::get(&w.tx, into)?;
    if a.kind != b.kind {
        return Err(StoreError::Invalid(format!(
            "only names in the same list merge ({} into {})",
            a.kind.as_str(),
            b.kind.as_str()
        )));
    }
    let ancestors = entities::with_descendants(&w.tx, from)?;
    if ancestors.contains(&into) {
        return Err(StoreError::Invalid(
            "a name can't be merged into one of its own jobs".into(),
        ));
    }
    let (f, t) = (from.to_string(), into.to_string());
    bump_txns(w, &f)?;
    repoint(
        w,
        &[
            "UPDATE posting SET entity_id = ?2 WHERE entity_id = ?1",
            "UPDATE txn SET entity_id = ?2 WHERE entity_id = ?1",
            "UPDATE entity SET parent_id = ?2, doc = json_set(doc, '$.parent_id', ?2) WHERE parent_id = ?1",
            "UPDATE external_ref SET local_id = ?2 WHERE local_kind = 'entity' AND local_id = ?1",
            "UPDATE attachment SET record_id = ?2 WHERE record_kind = 'entity' AND record_id = ?1",
        ],
        &f,
        &t,
    )?;
    w.tx.execute("DELETE FROM entity WHERE id = ?1", [&f])?;
    replace_in_docs(w, &f, &t)?;
    for child in hier::refresh_descendants(&w.tx, "entity", &t)? {
        w.changed("entity", child, "update");
    }
    audit::record(
        w,
        "entity",
        &f,
        "merge",
        Some(&a),
        Some(&b),
        Some(&format!("merged into {}", b.full_name)),
    )?;
    w.changed("entity", f, "delete");
    w.changed("entity", t, "update");
    Ok(())
}

/// Merge account `from` into `into` (same type). Subaccounts move under `into`.
/// An account the books rely on (a role, such as Undeposited Funds) can't be
/// merged away.
pub fn merge_accounts(w: &mut Writer<'_>, from: AccountId, into: AccountId) -> Result<()> {
    if from == into {
        return Err(StoreError::Invalid(
            "an account can't be merged into itself".into(),
        ));
    }
    let a = accounts::get(&w.tx, from)?;
    let b = accounts::get(&w.tx, into)?;
    if a.account_type != b.account_type {
        return Err(StoreError::Invalid(format!(
            "only accounts of the same type merge ({} into {})",
            a.account_type.label(),
            b.account_type.label()
        )));
    }
    if a.role.is_some() {
        return Err(StoreError::Invalid(format!(
            "{} is one the books rely on and can't be merged away; merge the other into it",
            a.full_name
        )));
    }
    let mut cursor = b.parent_id;
    while let Some(p) = cursor {
        if p == from {
            return Err(StoreError::Invalid(
                "an account can't be merged into its own subaccount".into(),
            ));
        }
        cursor = accounts::get(&w.tx, p)?.parent_id;
    }
    let (f, t) = (from.to_string(), into.to_string());
    bump_txns(w, &f)?;
    repoint(
        w,
        &[
            "UPDATE posting SET account_id = ?2 WHERE account_id = ?1",
            "UPDATE txn SET account_id = ?2 WHERE account_id = ?1",
            "UPDATE txn SET ar_ap_account_id = ?2 WHERE ar_ap_account_id = ?1",
            "UPDATE application SET discount_account_id = ?2 WHERE discount_account_id = ?1",
            "UPDATE account SET parent_id = ?2, doc = json_set(doc, '$.parent_id', ?2) WHERE parent_id = ?1",
            "UPDATE reconciliation SET account_id = ?2 WHERE account_id = ?1",
            "UPDATE feed_connection SET account_id = ?2 WHERE account_id = ?1",
            "UPDATE feed_txn SET account_id = ?2 WHERE account_id = ?1",
            "UPDATE external_ref SET local_id = ?2 WHERE local_kind = 'account' AND local_id = ?1",
            "UPDATE attachment SET record_id = ?2 WHERE record_kind = 'account' AND record_id = ?1",
        ],
        &f,
        &t,
    )?;
    w.tx.execute("DELETE FROM account WHERE id = ?1", [&f])?;
    replace_in_docs(w, &f, &t)?;
    for child in hier::refresh_descendants(&w.tx, "account", &t)? {
        w.changed("account", child, "update");
    }
    audit::record(
        w,
        "account",
        &f,
        "merge",
        Some(&a),
        Some(&b),
        Some(&format!("merged into {}", b.full_name)),
    )?;
    w.changed("account", f, "delete");
    w.changed("account", t, "update");
    Ok(())
}

/// Merge item `from` into `into` (same type).
pub fn merge_items(w: &mut Writer<'_>, from: ItemId, into: ItemId) -> Result<()> {
    if from == into {
        return Err(StoreError::Invalid(
            "an item can't be merged into itself".into(),
        ));
    }
    let a = items::get(&w.tx, from)?;
    let b = items::get(&w.tx, into)?;
    if a.item_type != b.item_type {
        return Err(StoreError::Invalid(format!(
            "only items of the same type merge ({} into {})",
            a.item_type.label(),
            b.item_type.label()
        )));
    }
    let (f, t) = (from.to_string(), into.to_string());
    bump_txns(w, &f)?;
    repoint(
        w,
        &[
            "UPDATE posting SET item_id = ?2 WHERE item_id = ?1",
            "UPDATE item SET parent_id = ?2, doc = json_set(doc, '$.parent_id', ?2) WHERE parent_id = ?1",
            "UPDATE external_ref SET local_id = ?2 WHERE local_kind = 'item' AND local_id = ?1",
            "UPDATE attachment SET record_id = ?2 WHERE record_kind = 'item' AND record_id = ?1",
        ],
        &f,
        &t,
    )?;
    w.tx.execute("DELETE FROM item WHERE id = ?1", [&f])?;
    replace_in_docs(w, &f, &t)?;
    for child in hier::refresh_descendants(&w.tx, "item", &t)? {
        w.changed("item", child, "update");
    }
    audit::record(
        w,
        "item",
        &f,
        "merge",
        Some(&a),
        Some(&b),
        Some(&format!("merged into {}", b.full_name)),
    )?;
    w.changed("item", f, "delete");
    w.changed("item", t, "update");
    Ok(())
}
