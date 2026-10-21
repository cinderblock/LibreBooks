//! The small lists (classes, terms, payment methods, sales tax codes, sales reps,
//! customer/vendor/job types), stored together in `list_entry`.

use lb_core::lists::{Class, PaymentMethod, SalesRep, SalesTaxCode, Terms, TypeEntry};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::codec::{self, doc};
use crate::{Result, StoreError, Writer, audit, hier};

/// A record kept in `list_entry`.
pub trait ListRecord: Serialize + DeserializeOwned + Clone {
    /// The `list` column value.
    const LIST: &'static str;
    /// For messages.
    const NOUN: &'static str;
    fn id_string(&self) -> String;
    fn display_name(&self) -> String;
    fn parent(&self) -> Option<String> {
        None
    }
    fn set_full_name(&mut self, _full: String, _level: u32) {}
    fn is_active(&self) -> bool;
    fn edit_seq(&self) -> i64;
    fn set_edit_seq(&mut self, v: i64);
    /// Rules of the record beyond name uniqueness (QBD's field limits).
    fn validate(&self) -> Result<()> {
        Ok(())
    }
}

macro_rules! simple_list {
    ($t:ty, $list:literal, $noun:literal, $name:ident) => {
        impl ListRecord for $t {
            const LIST: &'static str = $list;
            const NOUN: &'static str = $noun;
            fn id_string(&self) -> String {
                self.id.to_string()
            }
            fn display_name(&self) -> String {
                self.$name.clone()
            }
            fn is_active(&self) -> bool {
                self.is_active
            }
            fn edit_seq(&self) -> i64 {
                self.edit_seq
            }
            fn set_edit_seq(&mut self, v: i64) {
                self.edit_seq = v;
            }
        }
    };
}

simple_list!(Terms, "terms", "terms", name);
simple_list!(PaymentMethod, "payment_method", "payment method", name);

impl ListRecord for SalesTaxCode {
    const LIST: &'static str = "sales_tax_code";
    const NOUN: &'static str = "sales tax code";
    fn id_string(&self) -> String {
        self.id.to_string()
    }
    fn display_name(&self) -> String {
        self.name.clone()
    }
    fn is_active(&self) -> bool {
        self.is_active
    }
    fn edit_seq(&self) -> i64 {
        self.edit_seq
    }
    fn set_edit_seq(&mut self, v: i64) {
        self.edit_seq = v;
    }
    /// QBD's tax codes are at most three characters ("Tax", "Non", "OOS").
    fn validate(&self) -> Result<()> {
        let n = self.name.trim().chars().count();
        if n == 0 || n > 3 {
            return Err(StoreError::Invalid(
                "a sales tax code is one to three characters".into(),
            ));
        }
        Ok(())
    }
}
simple_list!(SalesRep, "sales_rep", "sales rep", initials);

impl ListRecord for Class {
    const LIST: &'static str = "class";
    const NOUN: &'static str = "class";
    fn id_string(&self) -> String {
        self.id.to_string()
    }
    fn display_name(&self) -> String {
        self.name.clone()
    }
    fn parent(&self) -> Option<String> {
        self.parent_id.map(|p| p.to_string())
    }
    fn set_full_name(&mut self, full: String, level: u32) {
        self.full_name = full;
        self.sublevel = level;
    }
    fn is_active(&self) -> bool {
        self.is_active
    }
    fn edit_seq(&self) -> i64 {
        self.edit_seq
    }
    fn set_edit_seq(&mut self, v: i64) {
        self.edit_seq = v;
    }
}

impl ListRecord for TypeEntry {
    const LIST: &'static str = "type";
    const NOUN: &'static str = "type";
    fn id_string(&self) -> String {
        self.id.to_string()
    }
    fn display_name(&self) -> String {
        self.name.clone()
    }
    fn parent(&self) -> Option<String> {
        self.parent_id.map(|p| p.to_string())
    }
    fn set_full_name(&mut self, full: String, _level: u32) {
        self.full_name = full;
    }
    fn is_active(&self) -> bool {
        self.is_active
    }
    fn edit_seq(&self) -> i64 {
        self.edit_seq
    }
    fn set_edit_seq(&mut self, v: i64) {
        self.edit_seq = v;
    }
}

pub fn list<T: ListRecord>(c: &Connection, include_inactive: bool) -> Result<Vec<T>> {
    let mut stmt = c.prepare(
        "SELECT doc FROM list_entry WHERE list = ?1 AND (?2 OR is_active) ORDER BY full_name",
    )?;
    let rows = stmt.query_map(params![T::LIST, include_inactive], |r| doc::<T>(r, 0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn find<T: ListRecord>(c: &Connection, id: &str) -> Result<Option<T>> {
    Ok(c.query_row(
        "SELECT doc FROM list_entry WHERE list = ?1 AND id = ?2",
        params![T::LIST, id],
        |r| doc(r, 0),
    )
    .optional()?)
}

pub fn get<T: ListRecord>(c: &Connection, id: &str) -> Result<T> {
    find(c, id)?.ok_or_else(|| StoreError::NotFound {
        kind: T::NOUN,
        id: id.to_owned(),
    })
}

pub fn by_name<T: ListRecord>(c: &Connection, full_name: &str) -> Result<Option<T>> {
    Ok(c.query_row(
        "SELECT doc FROM list_entry WHERE list = ?1 AND full_name = ?2 COLLATE NOCASE",
        params![T::LIST, full_name],
        |r| doc(r, 0),
    )
    .optional()?)
}

pub fn save<T: ListRecord>(w: &mut Writer<'_>, mut rec: T) -> Result<T> {
    let id = rec.id_string();
    let existing: Option<T> = find(&w.tx, &id)?;
    if let Some(x) = &existing {
        if x.edit_seq() != rec.edit_seq() {
            return Err(StoreError::Conflict {
                kind: T::NOUN,
                id,
                expected: rec.edit_seq(),
                actual: x.edit_seq(),
            });
        }
    } else if rec.edit_seq() != 0 {
        return Err(StoreError::NotFound { kind: T::NOUN, id });
    }
    rec.validate()?;
    let parent = rec.parent();
    hier::check_no_cycle(&w.tx, "list_entry", &id, parent.as_deref())?;
    let name = rec.display_name().trim().to_owned();
    let (full, level) = hier::full_name(&w.tx, "list_entry", parent.as_deref(), &name)?;
    rec.set_full_name(full.clone(), level);
    rec.set_edit_seq(rec.edit_seq() + 1);
    w.tx.execute(
        "INSERT INTO list_entry (id, list, name, full_name, parent_id, is_active, edit_seq, doc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, full_name = excluded.full_name,
           parent_id = excluded.parent_id, is_active = excluded.is_active, edit_seq = excluded.edit_seq,
           doc = excluded.doc",
        params![id, T::LIST, name, full, parent, rec.is_active(), rec.edit_seq(), codec::to_doc(&rec)?],
    )
    .map_err(|e| hier::map_duplicate(e, T::NOUN, &full))?;
    if existing.is_some() {
        for child in hier::refresh_descendants(&w.tx, "list_entry", &id)? {
            w.changed("list", child, "update");
        }
    }
    let action = if existing.is_some() {
        "update"
    } else {
        "create"
    };
    audit::record(w, T::LIST, &id, action, existing.as_ref(), Some(&rec), None)?;
    w.changed("list", id, action);
    Ok(rec)
}

pub fn delete<T: ListRecord>(w: &mut Writer<'_>, id: &str, edit_seq: i64) -> Result<()> {
    let rec: T = get(&w.tx, id)?;
    if rec.edit_seq() != edit_seq {
        return Err(StoreError::Conflict {
            kind: T::NOUN,
            id: id.to_owned(),
            expected: edit_seq,
            actual: rec.edit_seq(),
        });
    }
    let posting_col = (T::LIST == "class").then_some("class_id");
    let in_lists: bool = w.tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM entity WHERE doc LIKE ?1) OR EXISTS (SELECT 1 FROM item WHERE doc LIKE ?1)
           OR EXISTS (SELECT 1 FROM list_entry WHERE parent_id = ?2)",
        params![format!("%{id}%"), id],
        |r| r.get(0),
    )?;
    if in_lists || hier::is_referenced(&w.tx, posting_col, id)? {
        return Err(StoreError::InUse { kind: T::NOUN });
    }
    w.tx.execute("DELETE FROM list_entry WHERE id = ?1", [id])?;
    audit::record::<T, T>(w, T::LIST, id, "delete", Some(&rec), None, None)?;
    w.changed("list", id, "delete");
    Ok(())
}
