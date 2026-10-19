//! Where records came from (QuickBooks ids, bank FITIDs…), for idempotent,
//! incremental imports.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::{Result, Writer, now};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExternalRef {
    pub source: String,
    pub kind: String,
    pub external_id: String,
    pub local_kind: String,
    pub local_id: String,
    pub edit_sequence: Option<String>,
    pub hash: Option<String>,
    pub synced_at: String,
}

pub fn get(
    c: &Connection,
    source: &str,
    kind: &str,
    external_id: &str,
) -> Result<Option<ExternalRef>> {
    Ok(c.query_row(
        "SELECT source, kind, external_id, local_kind, local_id, edit_sequence, hash, synced_at
         FROM external_ref WHERE source = ?1 AND kind = ?2 AND external_id = ?3",
        params![source, kind, external_id],
        row,
    )
    .optional()?)
}

/// Look up by external id regardless of the source's record kind. Only safe where
/// the source's ids are unique across kinds (QBD `TxnID`s are). QBD `ListID`s are
/// **not** unique across lists: in a real company file, Terms and Payment Methods
/// share ids of the form `80000003-<same timestamp>`. Look those up with [`get`]
/// and a kind.
pub fn get_any_kind(
    c: &Connection,
    source: &str,
    external_id: &str,
) -> Result<Option<ExternalRef>> {
    Ok(c.query_row(
        "SELECT source, kind, external_id, local_kind, local_id, edit_sequence, hash, synced_at
         FROM external_ref WHERE source = ?1 AND external_id = ?2 LIMIT 1",
        params![source, external_id],
        row,
    )
    .optional()?)
}

pub fn for_local(c: &Connection, local_kind: &str, local_id: &str) -> Result<Vec<ExternalRef>> {
    let mut stmt = c.prepare(
        "SELECT source, kind, external_id, local_kind, local_id, edit_sequence, hash, synced_at
         FROM external_ref WHERE local_kind = ?1 AND local_id = ?2",
    )?;
    let rows = stmt.query_map(params![local_kind, local_id], row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Every external id of a kind from a source (to find records deleted upstream).
pub fn all_of_kind(c: &Connection, source: &str, kind: &str) -> Result<Vec<ExternalRef>> {
    let mut stmt = c.prepare(
        "SELECT source, kind, external_id, local_kind, local_id, edit_sequence, hash, synced_at
         FROM external_ref WHERE source = ?1 AND kind = ?2",
    )?;
    let rows = stmt.query_map(params![source, kind], row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[allow(clippy::too_many_arguments)]
pub fn put(
    w: &mut Writer<'_>,
    source: &str,
    kind: &str,
    external_id: &str,
    local_kind: &str,
    local_id: &str,
    edit_sequence: Option<&str>,
    hash: Option<&str>,
) -> Result<()> {
    w.tx.execute(
        "INSERT INTO external_ref (source, kind, external_id, local_kind, local_id, edit_sequence, hash, synced_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(source, kind, external_id) DO UPDATE SET local_kind = excluded.local_kind,
           local_id = excluded.local_id, edit_sequence = excluded.edit_sequence, hash = excluded.hash,
           synced_at = excluded.synced_at",
        params![source, kind, external_id, local_kind, local_id, edit_sequence, hash, now()],
    )?;
    Ok(())
}

pub fn remove(w: &mut Writer<'_>, source: &str, kind: &str, external_id: &str) -> Result<()> {
    w.tx.execute(
        "DELETE FROM external_ref WHERE source = ?1 AND kind = ?2 AND external_id = ?3",
        params![source, kind, external_id],
    )?;
    Ok(())
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ExternalRef> {
    Ok(ExternalRef {
        source: r.get(0)?,
        kind: r.get(1)?,
        external_id: r.get(2)?,
        local_kind: r.get(3)?,
        local_id: r.get(4)?,
        edit_sequence: r.get(5)?,
        hash: r.get(6)?,
        synced_at: r.get(7)?,
    })
}
