//! The audit trail: every change with who, when, and the record before and after.

use rusqlite::{Connection, params};
use serde::Serialize;

use crate::{Result, Writer, codec, now};

pub fn record<B: Serialize + ?Sized, A: Serialize + ?Sized>(
    w: &mut Writer<'_>,
    record_kind: &str,
    record_id: &str,
    action: &str,
    before: Option<&B>,
    after: Option<&A>,
    note: Option<&str>,
) -> Result<()> {
    let before = before.map(codec::to_doc).transpose()?;
    let after = after.map(codec::to_doc).transpose()?;
    w.tx.execute(
        "INSERT INTO audit (at, user_id, record_kind, record_id, action, before, after, note)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            now(),
            w.actor.user_id,
            record_kind,
            record_id,
            action,
            before,
            after,
            note
        ],
    )?;
    Ok(())
}

/// One audit entry as stored.
#[derive(Clone, Debug, Serialize)]
pub struct AuditEntry {
    pub id: i64,
    pub at: String,
    pub user_id: Option<String>,
    pub record_kind: String,
    pub record_id: String,
    pub action: String,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
    pub note: Option<String>,
}

/// A record's history, oldest first.
pub fn history(c: &Connection, record_kind: &str, record_id: &str) -> Result<Vec<AuditEntry>> {
    let mut stmt = c.prepare(
        "SELECT id, at, user_id, record_kind, record_id, action, before, after, note
         FROM audit WHERE record_kind = ?1 AND record_id = ?2 ORDER BY id",
    )?;
    let rows = stmt.query_map(params![record_kind, record_id], row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The most recent `limit` entries, newest first, optionally since an id.
pub fn recent(c: &Connection, limit: u32, before_id: Option<i64>) -> Result<Vec<AuditEntry>> {
    let mut stmt = c.prepare(
        "SELECT id, at, user_id, record_kind, record_id, action, before, after, note
         FROM audit WHERE (?1 IS NULL OR id < ?1) ORDER BY id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![before_id, limit], row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEntry> {
    let parse = |s: Option<String>| s.and_then(|s| serde_json::from_str(&s).ok());
    Ok(AuditEntry {
        id: r.get(0)?,
        at: r.get(1)?,
        user_id: r.get(2)?,
        record_kind: r.get(3)?,
        record_id: r.get(4)?,
        action: r.get(5)?,
        before: parse(r.get(6)?),
        after: parse(r.get(7)?),
        note: r.get(8)?,
    })
}
