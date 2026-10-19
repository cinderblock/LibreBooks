//! Names: customers and jobs, vendors, employees, other names.

use lb_core::{Entity, EntityId, EntityKind};
use rusqlite::{Connection, OptionalExtension, params};

use crate::codec::{self, doc};
use crate::{Result, StoreError, Writer, audit, hier};

/// Names of one kind (or all), in full-name order.
pub fn list(
    c: &Connection,
    kind: Option<EntityKind>,
    include_inactive: bool,
) -> Result<Vec<Entity>> {
    let mut stmt = c.prepare(
        "SELECT doc FROM entity WHERE (?1 IS NULL OR kind = ?1) AND (?2 OR is_active) ORDER BY full_name",
    )?;
    let rows = stmt.query_map(
        params![kind.map(EntityKind::as_str), include_inactive],
        |r| doc::<Entity>(r, 0),
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(c: &Connection, id: EntityId) -> Result<Entity> {
    find(c, id)?.ok_or_else(|| StoreError::NotFound {
        kind: "name",
        id: id.to_string(),
    })
}

pub fn find(c: &Connection, id: EntityId) -> Result<Option<Entity>> {
    Ok(c.query_row(
        "SELECT doc FROM entity WHERE id = ?1",
        [id.to_string()],
        |r| doc(r, 0),
    )
    .optional()?)
}

pub fn by_full_name(c: &Connection, full_name: &str) -> Result<Option<Entity>> {
    Ok(c.query_row(
        "SELECT doc FROM entity WHERE full_name = ?1",
        [full_name],
        |r| doc(r, 0),
    )
    .optional()?)
}

/// `id` and every job below it (for "customer and its jobs" filters and payments).
pub fn with_descendants(c: &Connection, id: EntityId) -> Result<Vec<EntityId>> {
    let mut stmt = c.prepare(
        "WITH RECURSIVE t(id) AS (SELECT ?1 UNION ALL SELECT e.id FROM entity e JOIN t ON e.parent_id = t.id)
         SELECT id FROM t",
    )?;
    let ids = stmt.query_map([id.to_string()], |r| r.get::<_, String>(0))?;
    ids.map(|s| codec::parse_id(&s?)).collect()
}

/// The top-level customer of a job (itself if it is one).
pub fn root_of(c: &Connection, id: EntityId) -> Result<EntityId> {
    let mut cur = id;
    for _ in 0..1000 {
        let parent: Option<String> = c.query_row(
            "SELECT parent_id FROM entity WHERE id = ?1",
            [cur.to_string()],
            |r| r.get(0),
        )?;
        match parent {
            Some(p) => cur = codec::parse_id(&p)?,
            None => return Ok(cur),
        }
    }
    Err(StoreError::Corrupt("name hierarchy loops".into()))
}

pub fn save(w: &mut Writer<'_>, mut e: Entity) -> Result<Entity> {
    let existing = find(&w.tx, e.id)?;
    if let Some(x) = &existing {
        if x.edit_seq != e.edit_seq {
            return Err(StoreError::Conflict {
                kind: "name",
                id: e.id.to_string(),
                expected: e.edit_seq,
                actual: x.edit_seq,
            });
        }
        if x.kind != e.kind && hier::is_referenced(&w.tx, Some("entity_id"), &e.id.to_string())? {
            return Err(StoreError::Invalid(
                "a name with transactions can't move to another list".into(),
            ));
        }
    } else if e.edit_seq != 0 {
        return Err(StoreError::NotFound {
            kind: "name",
            id: e.id.to_string(),
        });
    }
    if e.parent_id.is_some() && e.kind != EntityKind::Customer {
        return Err(StoreError::Invalid("only customers have jobs".into()));
    }
    let id = e.id.to_string();
    let parent = e.parent_id.map(|p| p.to_string());
    hier::check_no_cycle(&w.tx, "entity", &id, parent.as_deref())?;
    if let Some(p) = e.parent_id {
        let parent = get(&w.tx, p)?;
        if parent.kind != EntityKind::Customer {
            return Err(StoreError::Invalid("a job must be under a customer".into()));
        }
    }
    let (full, level) = hier::full_name(&w.tx, "entity", parent.as_deref(), e.name.trim())?;
    e.name = e.name.trim().to_owned();
    e.full_name = full;
    e.sublevel = level;
    e.edit_seq += 1;
    let renamed = existing
        .as_ref()
        .is_some_and(|x| x.full_name != e.full_name);
    w.tx.execute(
        "INSERT INTO entity (id, kind, parent_id, name, full_name, is_active, edit_seq, doc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET kind = excluded.kind, parent_id = excluded.parent_id,
           name = excluded.name, full_name = excluded.full_name, is_active = excluded.is_active,
           edit_seq = excluded.edit_seq, doc = excluded.doc",
        params![
            id,
            e.kind.as_str(),
            parent,
            e.name,
            e.full_name,
            e.is_active,
            e.edit_seq,
            codec::to_doc(&e)?
        ],
    )
    .map_err(|err| hier::map_duplicate(err, "name", &e.full_name))?;
    if renamed {
        for child in hier::refresh_descendants(&w.tx, "entity", &id)? {
            w.changed("entity", child, "update");
        }
    }
    let action = if existing.is_some() {
        "update"
    } else {
        "create"
    };
    audit::record(w, "entity", &id, action, existing.as_ref(), Some(&e), None)?;
    w.changed("entity", id, action);
    Ok(e)
}

pub fn delete(w: &mut Writer<'_>, id: EntityId, edit_seq: i64) -> Result<()> {
    let e = get(&w.tx, id)?;
    if e.edit_seq != edit_seq {
        return Err(StoreError::Conflict {
            kind: "name",
            id: id.to_string(),
            expected: edit_seq,
            actual: e.edit_seq,
        });
    }
    let sid = id.to_string();
    let has_jobs: bool = w.tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM entity WHERE parent_id = ?1)",
        [&sid],
        |r| r.get(0),
    )?;
    if has_jobs || hier::is_referenced(&w.tx, Some("entity_id"), &sid)? {
        return Err(StoreError::InUse { kind: "name" });
    }
    w.tx.execute("DELETE FROM entity WHERE id = ?1", [&sid])?;
    audit::record::<Entity, Entity>(w, "entity", &sid, "delete", Some(&e), None, None)?;
    w.changed("entity", sid, "delete");
    Ok(())
}
