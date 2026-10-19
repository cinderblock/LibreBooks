//! Items.

use lb_core::{Item, ItemId, ItemType};
use rusqlite::{Connection, OptionalExtension, params};

use crate::codec::{self, doc};
use crate::{Result, StoreError, Writer, audit, hier};

pub fn list(
    c: &Connection,
    item_type: Option<ItemType>,
    include_inactive: bool,
) -> Result<Vec<Item>> {
    let mut stmt = c.prepare(
        "SELECT doc FROM item WHERE (?1 IS NULL OR item_type = ?1) AND (?2 OR is_active) ORDER BY full_name",
    )?;
    let rows = stmt.query_map(
        params![item_type.map(ItemType::as_str), include_inactive],
        |r| doc::<Item>(r, 0),
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(c: &Connection, id: ItemId) -> Result<Item> {
    find(c, id)?.ok_or_else(|| StoreError::NotFound {
        kind: "item",
        id: id.to_string(),
    })
}

pub fn find(c: &Connection, id: ItemId) -> Result<Option<Item>> {
    Ok(c.query_row(
        "SELECT doc FROM item WHERE id = ?1",
        [id.to_string()],
        |r| doc(r, 0),
    )
    .optional()?)
}

pub fn by_full_name(c: &Connection, full_name: &str) -> Result<Option<Item>> {
    Ok(c.query_row(
        "SELECT doc FROM item WHERE full_name = ?1",
        [full_name],
        |r| doc(r, 0),
    )
    .optional()?)
}

pub fn save(w: &mut Writer<'_>, mut item: Item) -> Result<Item> {
    let existing = find(&w.tx, item.id)?;
    if let Some(x) = &existing {
        if x.edit_seq != item.edit_seq {
            return Err(StoreError::Conflict {
                kind: "item",
                id: item.id.to_string(),
                expected: item.edit_seq,
                actual: x.edit_seq,
            });
        }
        if x.item_type != item.item_type
            && hier::is_referenced(&w.tx, Some("item_id"), &item.id.to_string())?
        {
            return Err(StoreError::Invalid(
                "an item with transactions can't change type".into(),
            ));
        }
        // On-hand and average cost belong to the ledger, not the form.
        item.quantity_on_hand = x.quantity_on_hand;
        item.average_cost = x.average_cost;
    } else if item.edit_seq != 0 {
        return Err(StoreError::NotFound {
            kind: "item",
            id: item.id.to_string(),
        });
    }
    validate(&item)?;
    let id = item.id.to_string();
    let parent = item.parent_id.map(|p| p.to_string());
    hier::check_no_cycle(&w.tx, "item", &id, parent.as_deref())?;
    let (full, level) = hier::full_name(&w.tx, "item", parent.as_deref(), item.name.trim())?;
    item.name = item.name.trim().to_owned();
    item.full_name = full;
    item.sublevel = level;
    item.edit_seq += 1;
    let renamed = existing
        .as_ref()
        .is_some_and(|x| x.full_name != item.full_name);
    w.tx.execute(
        "INSERT INTO item (id, item_type, parent_id, name, full_name, is_active, edit_seq, doc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET item_type = excluded.item_type, parent_id = excluded.parent_id,
           name = excluded.name, full_name = excluded.full_name, is_active = excluded.is_active,
           edit_seq = excluded.edit_seq, doc = excluded.doc",
        params![id, item.item_type.as_str(), parent, item.name, item.full_name, item.is_active, item.edit_seq, codec::to_doc(&item)?],
    )
    .map_err(|e| hier::map_duplicate(e, "item", &item.full_name))?;
    if renamed {
        for child in hier::refresh_descendants(&w.tx, "item", &id)? {
            w.changed("item", child, "update");
        }
    }
    let action = if existing.is_some() {
        "update"
    } else {
        "create"
    };
    audit::record(w, "item", &id, action, existing.as_ref(), Some(&item), None)?;
    w.changed("item", id, action);
    Ok(item)
}

/// The accounts each type needs, as QBD's New Item window insists.
fn validate(item: &Item) -> Result<()> {
    let need = |ok: bool, what: &str| {
        if ok {
            Ok(())
        } else {
            Err(StoreError::Invalid(format!(
                "{} items need {what}",
                item.item_type.label()
            )))
        }
    };
    match item.item_type {
        ItemType::Service | ItemType::NonInventory | ItemType::OtherCharge => {
            if item.has_purchase_info {
                need(item.income_account_id.is_some(), "an income account")?;
                need(item.expense_account_id.is_some(), "an expense account")
            } else {
                need(item.income_account_id.is_some(), "an account")
            }
        }
        ItemType::Inventory | ItemType::InventoryAssembly => {
            need(item.income_account_id.is_some(), "an income account")?;
            need(item.expense_account_id.is_some(), "a COGS account")?;
            need(item.asset_account_id.is_some(), "an asset account")
        }
        ItemType::Discount => need(item.income_account_id.is_some(), "an account"),
        ItemType::SalesTax => need(item.tax_agency_id.is_some(), "a tax agency"),
        ItemType::Group | ItemType::SalesTaxGroup => need(!item.components.is_empty(), "members"),
        ItemType::Subtotal | ItemType::Payment | ItemType::FixedAsset => Ok(()),
    }
}

pub fn delete(w: &mut Writer<'_>, id: ItemId, edit_seq: i64) -> Result<()> {
    let item = get(&w.tx, id)?;
    if item.edit_seq != edit_seq {
        return Err(StoreError::Conflict {
            kind: "item",
            id: id.to_string(),
            expected: edit_seq,
            actual: item.edit_seq,
        });
    }
    let sid = id.to_string();
    let has_children: bool = w.tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM item WHERE parent_id = ?1)",
        [&sid],
        |r| r.get(0),
    )?;
    let in_group: bool = w.tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM item WHERE id <> ?1 AND doc LIKE ?2)",
        params![sid, format!("%{sid}%")],
        |r| r.get(0),
    )?;
    if has_children || in_group || hier::is_referenced(&w.tx, Some("item_id"), &sid)? {
        return Err(StoreError::InUse { kind: "item" });
    }
    w.tx.execute("DELETE FROM item WHERE id = ?1", [&sid])?;
    audit::record::<Item, Item>(w, "item", &sid, "delete", Some(&item), None, None)?;
    w.changed("item", sid, "delete");
    Ok(())
}
