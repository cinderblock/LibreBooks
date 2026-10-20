//! Things a company keeps by name: memorized transactions (QBD's Ctrl+M and the
//! Memorized Transaction List, Ctrl+T), memorized reports, saved views.
//!
//! All live in the `saved` table as JSON documents keyed by kind and name. A
//! memorized transaction may repeat: on a schedule it either reminds ("these are
//! due") or enters itself automatically, as QBD's "Remind Me" / "Automatically
//! Enter" do.

use lb_core::{Txn, TxnId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use uuid::Uuid;

use crate::{Result, StoreError, Writer, codec, now, txns};

/// A saved document of any kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Saved {
    #[ts(type = "string")]
    pub id: Uuid,
    pub kind: String,
    pub name: String,
    #[ts(type = "unknown")]
    pub doc: Value,
    pub created_at: String,
    pub modified_at: String,
}

pub fn list(c: &Connection, kind: &str) -> Result<Vec<Saved>> {
    let mut stmt = c.prepare(
        "SELECT id, kind, name, doc, created_at, modified_at FROM saved WHERE kind = ?1 ORDER BY name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([kind], row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(c: &Connection, id: Uuid) -> Result<Saved> {
    c.query_row(
        "SELECT id, kind, name, doc, created_at, modified_at FROM saved WHERE id = ?1",
        [id.to_string()],
        row,
    )
    .optional()?
    .ok_or_else(|| StoreError::NotFound {
        kind: "saved item",
        id: id.to_string(),
    })
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Saved> {
    let id: String = r.get(0)?;
    Ok(Saved {
        id: id.parse().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?,
        kind: r.get(1)?,
        name: r.get(2)?,
        doc: codec::doc(r, 3)?,
        created_at: r.get(4)?,
        modified_at: r.get(5)?,
    })
}

/// Create or replace by id. Names are unique within a kind.
pub fn put(
    w: &mut Writer<'_>,
    id: Option<Uuid>,
    kind: &str,
    name: &str,
    doc: &Value,
) -> Result<Saved> {
    let name = name.trim();
    if name.is_empty() {
        return Err(StoreError::Invalid("a name is required".into()));
    }
    let id = id.unwrap_or_else(Uuid::now_v7);
    let at = now();
    w.tx.execute(
        "INSERT INTO saved (id, kind, name, doc, created_at, modified_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, doc = excluded.doc, modified_at = excluded.modified_at",
        params![id.to_string(), kind, name, codec::to_doc(doc)?, at],
    )
    .map_err(|e| crate::hier::map_duplicate(e, "saved item", name))?;
    w.changed("saved", id.to_string(), "update");
    get(&w.tx, id)
}

pub fn delete(w: &mut Writer<'_>, id: Uuid) -> Result<()> {
    let n =
        w.tx.execute("DELETE FROM saved WHERE id = ?1", [id.to_string()])?;
    if n == 0 {
        return Err(StoreError::NotFound {
            kind: "saved item",
            id: id.to_string(),
        });
    }
    w.changed("saved", id.to_string(), "delete");
    Ok(())
}

/// The `kind` of memorized transactions in `saved`.
pub const MEMORIZED_TXN: &str = "memorized_txn";
/// The `kind` of memorized reports (their `doc` is a report request).
pub const MEMORIZED_REPORT: &str = "memorized_report";

/// How a memorized transaction repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Repeat {
    /// Only entered when someone uses it.
    Never,
    /// Listed as due ("Remind Me").
    Remind,
    /// Entered on its own when due ("Automatically Enter").
    Automatic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Interval {
    Daily,
    Weekly,
    BiWeekly,
    SemiMonthly,
    Monthly,
    Quarterly,
    SemiAnnually,
    Annually,
}

impl Interval {
    /// The next date after `d`. Semi-monthly alternates the 1st/15th-ish halves.
    pub fn after(self, d: jiff::civil::Date) -> jiff::civil::Date {
        use jiff::ToSpan;
        let r = match self {
            Interval::Daily => d.checked_add(1.day()),
            Interval::Weekly => d.checked_add(1.week()),
            Interval::BiWeekly => d.checked_add(2.weeks()),
            Interval::SemiMonthly => {
                if d.day() < 15 {
                    d.checked_add(15.days())
                } else {
                    d.checked_add(1.month())
                        .and_then(|m| m.checked_sub(15.days()))
                }
            }
            Interval::Monthly => d.checked_add(1.month()),
            Interval::Quarterly => d.checked_add(3.months()),
            Interval::SemiAnnually => d.checked_add(6.months()),
            Interval::Annually => d.checked_add(1.year()),
        };
        r.unwrap_or(d)
    }
}

/// A memorized transaction: the template and its schedule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Memorized {
    /// The transaction to copy. Its id, number and edit sequence are replaced on
    /// each use; its date becomes the use's date.
    pub template: Txn,
    pub repeat: Repeat,
    pub interval: Option<Interval>,
    pub next_date: Option<jiff::civil::Date>,
    /// Enter (or remind) this many days before it's due.
    pub days_in_advance: u32,
    /// Stop after this many more; `None` repeats forever.
    pub remaining: Option<u32>,
    /// Groups, as QBD's memorized transaction groups.
    pub group: Option<String>,
}

pub fn list_memorized(c: &Connection) -> Result<Vec<(Saved, Memorized)>> {
    list(c, MEMORIZED_TXN)?
        .into_iter()
        .map(|s| {
            let m: Memorized = serde_json::from_value(s.doc.clone())?;
            Ok((s, m))
        })
        .collect()
}

/// Memorize a transaction under a name.
pub fn memorize(w: &mut Writer<'_>, id: Option<Uuid>, name: &str, m: &Memorized) -> Result<Saved> {
    if m.repeat != Repeat::Never && (m.interval.is_none() || m.next_date.is_none()) {
        return Err(StoreError::Invalid(
            "a repeating transaction needs how often and when next".into(),
        ));
    }
    put(w, id, MEMORIZED_TXN, name, &serde_json::to_value(m)?)
}

/// A fresh transaction from a template, dated `date`, ready to save (or to
/// open in a form).
pub fn instantiate(template: &Txn, date: jiff::civil::Date) -> Txn {
    let mut t = template.clone();
    t.id = TxnId::new();
    t.date = date;
    t.number = None;
    t.edit_seq = 0;
    t.created_at = None;
    t.modified_at = None;
    t.created_by = None;
    t.modified_by = None;
    t.is_void = false;
    t.due_date = None;
    t.discount_date = None;
    for l in &mut t.lines {
        l.id = lb_core::LineId::new();
    }
    t
}

/// Memorized transactions due by `today` (counting days in advance), with the
/// date each is due.
pub fn due(
    c: &Connection,
    today: jiff::civil::Date,
) -> Result<Vec<(Saved, Memorized, jiff::civil::Date)>> {
    let mut out = Vec::new();
    for (s, m) in list_memorized(c)? {
        if m.repeat == Repeat::Never || m.remaining == Some(0) {
            continue;
        }
        let Some(next) = m.next_date else { continue };
        let horizon = today
            .checked_add(jiff::Span::new().days(i64::from(m.days_in_advance)))
            .unwrap_or(today);
        if next <= horizon {
            out.push((s, m, next));
        }
    }
    Ok(out)
}

/// Enter every automatic memorized transaction that is due, catching up on
/// missed dates (a server that was off for a month enters each one). Returns
/// the transactions entered.
pub fn enter_due(w: &mut Writer<'_>, today: jiff::civil::Date) -> Result<Vec<TxnId>> {
    let mut entered = Vec::new();
    for (s, mut m, _) in due(&w.tx, today)? {
        if m.repeat != Repeat::Automatic {
            continue;
        }
        let interval = m
            .interval
            .ok_or_else(|| StoreError::Invalid(format!("{} has no interval", s.name)))?;
        let horizon = today
            .checked_add(jiff::Span::new().days(i64::from(m.days_in_advance)))
            .unwrap_or(today);
        while let Some(next) = m.next_date {
            if next > horizon || m.remaining == Some(0) {
                break;
            }
            let txn = instantiate(&m.template, next);
            entered.push(txns::save(w, txn)?.txn.id);
            m.next_date = Some(interval.after(next));
            m.remaining = m.remaining.map(|r| r - 1);
        }
        memorize(w, Some(s.id), &s.name, &m)?;
    }
    Ok(entered)
}

#[cfg(test)]
mod tests {
    use super::Interval;
    use jiff::civil::date;

    #[test]
    fn intervals() {
        assert_eq!(
            Interval::Monthly.after(date(2026, 1, 31)),
            date(2026, 2, 28)
        );
        assert_eq!(
            Interval::SemiMonthly.after(date(2026, 3, 1)),
            date(2026, 3, 16)
        );
        assert_eq!(
            Interval::SemiMonthly.after(date(2026, 3, 16)),
            date(2026, 4, 1)
        );
        assert_eq!(
            Interval::BiWeekly.after(date(2026, 3, 1)),
            date(2026, 3, 15)
        );
        assert_eq!(
            Interval::Quarterly.after(date(2026, 11, 30)),
            date(2027, 2, 28)
        );
    }
}
