//! Hierarchical names (`Parent:Child:Grandchild`) shared by accounts, names, items
//! and the small lists: computing full names, refusing cycles, and renaming a
//! subtree when a parent's name changes.
//!
//! `table` is always one of this crate's own table names, never user input.

use rusqlite::{Connection, OptionalExtension, params};

use crate::{Result, StoreError};

pub const SEPARATOR: char = ':';

/// The full name and sublevel a record named `name` gets under `parent`.
pub fn full_name(
    c: &Connection,
    table: &str,
    parent: Option<&str>,
    name: &str,
) -> Result<(String, u32)> {
    if name.trim().is_empty() {
        return Err(StoreError::Invalid("a name is required".into()));
    }
    if name.contains(SEPARATOR) {
        return Err(StoreError::Invalid(format!(
            "a name can't contain \"{SEPARATOR}\""
        )));
    }
    let Some(parent) = parent else {
        return Ok((name.to_owned(), 0));
    };
    let sql =
        format!("SELECT full_name, json_extract(doc, '$.sublevel') FROM {table} WHERE id = ?1");
    let row: Option<(String, i64)> = c
        .query_row(&sql, [parent], |r| {
            Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0)))
        })
        .optional()?;
    let (parent_full, parent_level) =
        row.ok_or_else(|| StoreError::Invalid("the parent doesn't exist".into()))?;
    Ok((
        format!("{parent_full}{SEPARATOR}{name}"),
        parent_level as u32 + 1,
    ))
}

/// Refuse to make `id` a child of one of its own descendants (or itself).
pub fn check_no_cycle(
    c: &Connection,
    table: &str,
    id: &str,
    new_parent: Option<&str>,
) -> Result<()> {
    let mut cursor = new_parent.map(str::to_owned);
    let sql = format!("SELECT parent_id FROM {table} WHERE id = ?1");
    let mut steps = 0;
    while let Some(p) = cursor {
        if p == id {
            return Err(StoreError::Invalid(
                "something can't be its own parent".into(),
            ));
        }
        steps += 1;
        if steps > 1000 {
            return Err(StoreError::Corrupt(format!("{table} hierarchy loops")));
        }
        cursor = c
            .query_row(&sql, [&p], |r| r.get::<_, Option<String>>(0))
            .optional()?
            .flatten();
    }
    Ok(())
}

/// After `id`'s full name changed, rewrite every descendant's full name and
/// sublevel (in their columns and their docs). Returns the ids touched.
pub fn refresh_descendants(c: &Connection, table: &str, id: &str) -> Result<Vec<String>> {
    let mut touched = Vec::new();
    let mut queue = vec![id.to_owned()];
    let select = format!(
        "SELECT p.full_name, json_extract(p.doc, '$.sublevel'), ch.id, ch.name
         FROM {table} ch JOIN {table} p ON p.id = ch.parent_id WHERE ch.parent_id = ?1"
    );
    let update = format!(
        "UPDATE {table} SET full_name = ?2, doc = json_set(doc, '$.full_name', ?2, '$.sublevel', ?3) WHERE id = ?1"
    );
    while let Some(parent) = queue.pop() {
        let children: Vec<(String, i64, String, String)> = {
            let mut stmt = c.prepare(&select)?;
            stmt.query_map([&parent], |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    r.get(2)?,
                    r.get(3)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?
        };
        for (parent_full, parent_level, child, name) in children {
            let full = format!("{parent_full}{SEPARATOR}{name}");
            c.execute(&update, params![child, full, parent_level + 1])?;
            touched.push(child.clone());
            queue.push(child);
        }
    }
    Ok(touched)
}

/// Map a unique-constraint failure on `full_name` to a friendly duplicate error.
pub fn map_duplicate(e: rusqlite::Error, kind: &'static str, name: &str) -> StoreError {
    if let rusqlite::Error::SqliteFailure(f, Some(msg)) = &e
        && f.code == rusqlite::ErrorCode::ConstraintViolation
        && msg.contains("UNIQUE")
    {
        return StoreError::Duplicate {
            kind,
            name: name.to_owned(),
        };
    }
    StoreError::Sqlite(e)
}

/// Is a record referenced by any posting or transaction? Ids are UUIDs, so a text
/// search of transaction documents cannot match by accident.
pub fn is_referenced(c: &Connection, posting_column: Option<&str>, id: &str) -> Result<bool> {
    if let Some(col) = posting_column {
        let sql = format!("SELECT EXISTS (SELECT 1 FROM posting WHERE {col} = ?1)");
        if c.query_row(&sql, [id], |r| r.get::<_, bool>(0))? {
            return Ok(true);
        }
    }
    let pattern = format!("%{id}%");
    Ok(c.query_row(
        "SELECT EXISTS (SELECT 1 FROM txn WHERE doc LIKE ?1)",
        [pattern],
        |r| r.get::<_, bool>(0),
    )?)
}
