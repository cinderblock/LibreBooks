//! Documents attached to the books: receipt photos, PDFs of bills, statements.
//!
//! Kept inside the company file so a backup or a copied file carries them (QBD's
//! Doc Center keeps them beside the file and loses them). An attachment may exist
//! before it belongs to anything: a phone snaps a receipt into the inbox, and it is
//! attached when the transaction is entered.

use lb_core::AttachmentId;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::{Result, StoreError, Writer, now};

/// Largest attachment accepted: phone photos and multi-page PDFs fit comfortably.
pub const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AttachmentMeta {
    pub id: AttachmentId,
    /// What it is attached to (`txn`, `entity`, `account`, `item`), or nothing yet.
    pub record_kind: Option<String>,
    pub record_id: Option<String>,
    pub filename: String,
    pub mime: String,
    #[ts(type = "number")]
    pub size: u64,
    pub sha256: String,
    pub created_at: String,
    pub created_by: Option<String>,
}

const COLS: &str =
    "id, record_kind, record_id, filename, mime, size, sha256, created_at, created_by";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AttachmentMeta> {
    let id: String = r.get(0)?;
    Ok(AttachmentMeta {
        id: id.parse().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?,
        record_kind: r.get(1)?,
        record_id: r.get(2)?,
        filename: r.get(3)?,
        mime: r.get(4)?,
        size: r.get::<_, i64>(5)? as u64,
        sha256: r.get(6)?,
        created_at: r.get(7)?,
        created_by: r.get(8)?,
    })
}

/// Store a document, optionally attached to a record.
pub fn add(
    w: &mut Writer<'_>,
    record: Option<(&str, &str)>,
    filename: &str,
    mime: &str,
    data: &[u8],
) -> Result<AttachmentMeta> {
    if data.is_empty() {
        return Err(StoreError::Invalid("the file is empty".into()));
    }
    if data.len() > MAX_BYTES {
        return Err(StoreError::Invalid(format!(
            "the file is larger than {} MB",
            MAX_BYTES / (1024 * 1024)
        )));
    }
    let filename = sanitize(filename);
    let meta = AttachmentMeta {
        id: AttachmentId::new(),
        record_kind: record.map(|(k, _)| k.to_owned()),
        record_id: record.map(|(_, i)| i.to_owned()),
        filename,
        mime: mime.to_owned(),
        size: data.len() as u64,
        sha256: hex(&Sha256::digest(data)),
        created_at: now(),
        created_by: w.actor.user_id.clone(),
    };
    w.tx.execute(
        &format!(
            "INSERT INTO attachment ({COLS}, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
        ),
        params![
            meta.id.to_string(),
            meta.record_kind,
            meta.record_id,
            meta.filename,
            meta.mime,
            meta.size as i64,
            meta.sha256,
            meta.created_at,
            meta.created_by,
            data
        ],
    )?;
    crate::audit::record(
        w,
        "attachment",
        &meta.id.to_string(),
        "create",
        None::<&AttachmentMeta>,
        Some(&meta),
        None,
    )?;
    w.changed("attachment", meta.id.to_string(), "create");
    Ok(meta)
}

/// Attach (or move, or detach with `None`) an existing document.
pub fn attach(
    w: &mut Writer<'_>,
    id: AttachmentId,
    record: Option<(&str, &str)>,
) -> Result<AttachmentMeta> {
    let before = meta(&w.tx, id)?;
    w.tx.execute(
        "UPDATE attachment SET record_kind = ?2, record_id = ?3 WHERE id = ?1",
        params![
            id.to_string(),
            record.map(|(k, _)| k),
            record.map(|(_, i)| i)
        ],
    )?;
    let after = meta(&w.tx, id)?;
    crate::audit::record(
        w,
        "attachment",
        &id.to_string(),
        "update",
        Some(&before),
        Some(&after),
        None,
    )?;
    w.changed("attachment", id.to_string(), "update");
    Ok(after)
}

pub fn meta(c: &Connection, id: AttachmentId) -> Result<AttachmentMeta> {
    c.query_row(
        &format!("SELECT {COLS} FROM attachment WHERE id = ?1"),
        [id.to_string()],
        row,
    )
    .optional()?
    .ok_or_else(|| StoreError::NotFound {
        kind: "attachment",
        id: id.to_string(),
    })
}

/// The document's bytes and description.
pub fn get(c: &Connection, id: AttachmentId) -> Result<(AttachmentMeta, Vec<u8>)> {
    let m = meta(c, id)?;
    let data: Vec<u8> = c.query_row(
        "SELECT data FROM attachment WHERE id = ?1",
        [id.to_string()],
        |r| r.get(0),
    )?;
    Ok((m, data))
}

/// Documents attached to a record, oldest first.
pub fn for_record(
    c: &Connection,
    record_kind: &str,
    record_id: &str,
) -> Result<Vec<AttachmentMeta>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {COLS} FROM attachment WHERE record_kind = ?1 AND record_id = ?2 ORDER BY created_at"
    ))?;
    let rows = stmt.query_map(params![record_kind, record_id], row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Documents not attached to anything yet (the inbox), newest first.
pub fn inbox(c: &Connection) -> Result<Vec<AttachmentMeta>> {
    let mut stmt = c.prepare(&format!(
        "SELECT {COLS} FROM attachment WHERE record_id IS NULL ORDER BY created_at DESC"
    ))?;
    let rows = stmt.query_map([], row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn delete(w: &mut Writer<'_>, id: AttachmentId) -> Result<()> {
    let before = meta(&w.tx, id)?;
    w.tx.execute("DELETE FROM attachment WHERE id = ?1", [id.to_string()])?;
    crate::audit::record(
        w,
        "attachment",
        &id.to_string(),
        "delete",
        Some(&before),
        None::<&AttachmentMeta>,
        None,
    )?;
    w.changed("attachment", id.to_string(), "delete");
    Ok(())
}

/// Keep a file name printable and path-free.
fn sanitize(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let clean: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
    if clean.trim().is_empty() {
        "attachment".to_owned()
    } else {
        clean
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn file_names_lose_their_paths() {
        assert_eq!(sanitize("C:\\Users\\me\\receipt.jpg"), "receipt.jpg");
        assert_eq!(sanitize("../../etc/passwd"), "passwd");
        assert_eq!(sanitize("\u{0}"), "attachment");
    }
}
