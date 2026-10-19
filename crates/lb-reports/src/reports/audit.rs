//! Audit Trail: who added, changed, voided and deleted what, and when, from the
//! store's `audit` table (every save writes the record before and after).
//!
//! Grouped by record, oldest change first, as QBD groups it by transaction. The
//! date range selects changes by when they were made (UTC days), not by the
//! transactions' own dates.

use lb_core::{EntityId, Money, TxnId, TxnType};
use serde_json::Value as Json;

use crate::model::{Cell, Column, ColumnKind, Drill, Report, Row, RowKind};
use crate::{Result, Run};

struct Entry {
    at: String,
    user: Option<String>,
    kind: String,
    record: String,
    action: String,
    doc: Option<Json>,
}

fn action_label(a: &str) -> String {
    match a {
        "create" => "Added".to_owned(),
        "update" => "Changed".to_owned(),
        "void" => "Voided".to_owned(),
        "delete" => "Deleted".to_owned(),
        "closing_date" => "Closing date changed".to_owned(),
        other => other.to_owned(),
    }
}

pub(crate) fn audit_trail(r: &Run<'_>) -> Result<Report> {
    let ctx = r.ctx;
    let mut entries: Vec<Entry> = Vec::new();
    {
        let mut stmt = r.c.prepare(
            "SELECT at, user_id, record_kind, record_id, action, COALESCE(after, before)
             FROM audit
             WHERE (?1 IS NULL OR substr(at, 1, 10) >= ?1) AND (?2 IS NULL OR substr(at, 1, 10) <= ?2)
             ORDER BY record_kind, record_id, id",
        )?;
        let mut rows = stmt.query(rusqlite::params![
            r.range.from.map(|d| d.to_string()),
            r.range.to.map(|d| d.to_string())
        ])?;
        while let Some(row) = rows.next()? {
            let doc: Option<String> = row.get(5)?;
            entries.push(Entry {
                at: row.get(0)?,
                user: row.get(1)?,
                kind: row.get(2)?,
                record: row.get(3)?,
                action: row.get(4)?,
                doc: doc.and_then(|d| serde_json::from_str(&d).ok()),
            });
        }
    }
    let columns = vec![
        Column::new("label", "", ColumnKind::Text),
        Column::new("action", "State", ColumnKind::Text),
        Column::new("at", "Entered/Last Modified", ColumnKind::Text),
        Column::new("user", "Last modified by", ColumnKind::Text),
        Column::new("date", "Date", ColumnKind::Date),
        Column::new("name", "Name", ColumnKind::Text),
        Column::new("memo", "Memo", ColumnKind::Text),
        Column::new("amount", "Amount", ColumnKind::Money),
    ];
    let text = |j: &Option<Json>, k: &str| -> Option<String> {
        j.as_ref()?.get(k)?.as_str().map(str::to_owned)
    };
    let mut rows = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        let (kind, record) = (entries[i].kind.clone(), entries[i].record.clone());
        let group: Vec<&Entry> = entries[i..]
            .iter()
            .take_while(|e| e.kind == kind && e.record == record)
            .collect();
        i += group.len();
        let latest = group.iter().rev().find_map(|e| e.doc.clone());
        let txn_type = (kind == "txn")
            .then(|| text(&latest, "txn_type"))
            .flatten()
            .and_then(|t| TxnType::parse(&t));
        if let Some(types) = &r.filter.txn_types
            && !txn_type.is_some_and(|t| types.contains(&t))
        {
            continue;
        }
        let heading = if let Some(t) = txn_type {
            match text(&latest, "number") {
                Some(n) => format!("{} {n}", t.label()),
                None => t.label().to_owned(),
            }
        } else {
            let name = text(&latest, "full_name")
                .or_else(|| text(&latest, "name"))
                .unwrap_or_else(|| record.clone());
            format!("{kind}: {name}")
        };
        let deleted = group.iter().any(|e| e.action == "delete");
        let drill = match (kind.as_str(), record.parse::<TxnId>()) {
            ("txn", Ok(id)) if !deleted => Some(Drill::Txn { txn_id: id }),
            _ => None,
        };
        let mut head = vec![Cell::EMPTY; columns.len()];
        head[0] = Cell::text(heading);
        rows.push(Row::new(RowKind::Header, 0, head).drill(drill.clone()));
        for e in group {
            let d = &e.doc;
            let entity = text(d, "entity_id").and_then(|s| s.parse::<EntityId>().ok());
            let date = text(d, "date").and_then(|s| s.parse().ok());
            let amount = (kind == "txn")
                .then(|| d.as_ref()?.get("amount")?.as_i64())
                .flatten();
            rows.push(
                Row::new(
                    RowKind::Data,
                    1,
                    vec![
                        Cell::EMPTY,
                        Cell::text(action_label(&e.action)),
                        Cell::text(e.at.clone()),
                        Cell::text(e.user.clone().unwrap_or_else(|| "System".to_owned())),
                        Cell::opt_date(date),
                        Cell::opt_text(ctx.entity_name(entity)),
                        Cell::opt_text(text(d, "memo")),
                        amount.map_or(Cell::EMPTY, |a| Cell::money(Money(a))),
                    ],
                )
                .drill(drill.clone()),
            );
        }
    }
    Ok(r.finish(r.range_line(), columns, rows, Vec::new()))
}
