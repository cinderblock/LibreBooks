//! The Bank Feeds queue: downloaded transactions waiting to be matched, added
//! or ignored.
//!
//! Every downloaded transaction becomes one `feed_txn` row, once: the table is
//! unique on `(account_id, fitid)`, so importing the same file twice, or an
//! overlapping download, adds only what is new. Each row has a status:
//!
//! - `new`: waiting for a person (or a rule) to decide;
//! - `matched`: linked to a transaction that was already in the register;
//! - `added`: a transaction was created from it;
//! - `ignored`: the person said it doesn't belong in the books (a duplicate, a
//!   transfer already entered from the other side, a pending hold).
//!
//! `matched` and `added` rows point at their register transaction
//! (`matched_txn_id`). [`undo`] reverses any of the three decisions.

use jiff::civil::Date;
use lb_core::{AccountId, ClassId, EntityId, Money, TxnId};
use lb_store::codec::{self, SqlDate};
use lb_store::{Writer, balances, txns};
use rusqlite::{Connection, OptionalExtension, ToSql, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::connections;
use crate::error::{FeedError, Result};
use crate::ids::{FeedConnectionId, FeedRuleId, FeedTxnId};
use crate::statement::{Statement, TrnType};

/// Where a downloaded transaction stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum FeedStatus {
    New,
    Matched,
    Added,
    Ignored,
}

impl FeedStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            FeedStatus::New => "new",
            FeedStatus::Matched => "matched",
            FeedStatus::Added => "added",
            FeedStatus::Ignored => "ignored",
        }
    }

    pub fn parse(s: &str) -> Option<FeedStatus> {
        Some(match s {
            "new" => FeedStatus::New,
            "matched" => FeedStatus::Matched,
            "added" => FeedStatus::Added,
            "ignored" => FeedStatus::Ignored,
            _ => return None,
        })
    }
}

/// What to fill in when the transaction is added: from a rule, or from how the
/// same payee was added last time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Suggestion {
    /// The rule that made it, if one did.
    pub rule_id: Option<FeedRuleId>,
    /// True when it came from the payee's history rather than a rule.
    pub from_history: bool,
    /// The payee as it should read (a rule's "rename to").
    pub payee: Option<String>,
    /// The name (vendor, customer…) to put on the transaction.
    pub entity_id: Option<EntityId>,
    /// The account on the other side: the expense, the income.
    pub account_id: Option<AccountId>,
    pub class_id: Option<ClassId>,
    pub memo: Option<String>,
    /// Add it without asking (when nothing in the register matches it).
    pub auto_add: bool,
}

/// One downloaded transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FeedTxn {
    pub id: FeedTxnId,
    pub connection_id: Option<FeedConnectionId>,
    /// The register it belongs to.
    pub account_id: AccountId,
    pub fitid: String,
    pub fitid_synthesized: bool,
    pub posted: Date,
    /// Positive when money went into the account.
    pub amount: Money,
    /// As the bank wrote it.
    pub payee: Option<String>,
    pub memo: Option<String>,
    pub check_number: Option<String>,
    pub trntype: TrnType,
    pub status: FeedStatus,
    /// The register transaction it was matched to or added as.
    pub matched_txn_id: Option<TxnId>,
    pub suggestion: Option<Suggestion>,
    /// Where it came from: `file`, `simplefin`, `ofx-direct`…
    pub source: String,
    pub imported_at: jiff::Timestamp,
    /// Everything the source sent, verbatim.
    #[ts(type = "unknown")]
    pub raw: serde_json::Value,
}

/// The parts of a feed transaction kept in its `doc` column.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Doc {
    trntype: TrnType,
    fitid_synthesized: bool,
    source: String,
    suggestion: Option<Suggestion>,
    raw: serde_json::Value,
}

const COLS: &str = "id, connection_id, account_id, fitid, posted_date, amount, payee, memo, check_number, status, matched_txn_id, doc, imported_at";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<FeedTxn>> {
    let id: String = r.get(0)?;
    let conn: Option<String> = r.get(1)?;
    let account: String = r.get(2)?;
    let posted: SqlDate = r.get(4)?;
    let status: String = r.get(9)?;
    let matched: Option<String> = r.get(10)?;
    let doc: String = r.get(11)?;
    let imported: String = r.get(12)?;
    let build = || -> Result<FeedTxn> {
        let d: Doc = codec::from_doc(&doc)?;
        let corrupt = |what: &str| lb_store::StoreError::Corrupt(what.to_owned());
        Ok(FeedTxn {
            id: id.parse().map_err(|_| corrupt("feed transaction id"))?,
            connection_id: conn
                .as_deref()
                .map(str::parse)
                .transpose()
                .map_err(|_| corrupt("feed connection id"))?,
            account_id: codec::parse_id(&account)?,
            fitid: r.get(3)?,
            fitid_synthesized: d.fitid_synthesized,
            posted: posted.0,
            amount: Money(r.get(5)?),
            payee: r.get(6)?,
            memo: r.get(7)?,
            check_number: r.get(8)?,
            trntype: d.trntype,
            status: FeedStatus::parse(&status).ok_or_else(|| corrupt("feed status"))?,
            matched_txn_id: matched.as_deref().map(codec::parse_id).transpose()?,
            suggestion: d.suggestion,
            source: d.source,
            imported_at: imported.parse().map_err(|_| corrupt("import time"))?,
            raw: d.raw,
        })
    };
    Ok(build())
}

pub fn find(c: &Connection, id: FeedTxnId) -> Result<Option<FeedTxn>> {
    c.query_row(
        &format!("SELECT {COLS} FROM feed_txn WHERE id = ?1"),
        [id.to_string()],
        row,
    )
    .optional()?
    .transpose()
}

pub fn get(c: &Connection, id: FeedTxnId) -> Result<FeedTxn> {
    find(c, id)?.ok_or_else(|| FeedError::NotFound {
        kind: "downloaded transaction",
        id: id.to_string(),
    })
}

/// How to land a statement in the queue.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct ImportOptions {
    pub connection_id: Option<FeedConnectionId>,
    /// `file`, `simplefin`… (kept on each row).
    pub source: String,
    /// Flip every amount (a source with backwards signs).
    pub reverse_signs: bool,
    /// Skip transactions dated before this.
    pub not_before: Option<Date>,
}

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ImportSummary {
    /// New rows, in statement order.
    pub imported: Vec<FeedTxnId>,
    /// Already in the queue from an earlier import.
    pub duplicates: u32,
    /// Dated before the connection's start date.
    pub before_start: u32,
    /// The parser's warnings, passed on.
    pub warnings: Vec<String>,
}

/// Land a statement's transactions in `account`'s queue, skipping ones already
/// there.
pub fn import_statement(
    w: &mut Writer<'_>,
    account: AccountId,
    statement: &Statement,
    opts: &ImportOptions,
) -> Result<ImportSummary> {
    connections::check_feedable(&w.tx, account)?;
    let mut statement = statement.clone();
    if opts.reverse_signs {
        statement.reverse_signs();
    }
    // Ids must be settled before they're used as the dedupe key.
    statement.finish();
    let mut out = ImportSummary {
        warnings: statement.warnings.clone(),
        ..ImportSummary::default()
    };
    let now = jiff::Timestamp::now().to_string();
    let source = if opts.source.is_empty() {
        "file".to_owned()
    } else {
        opts.source.clone()
    };
    for t in &statement.transactions {
        if opts.not_before.is_some_and(|d| t.posted < d) {
            out.before_start += 1;
            continue;
        }
        let id = FeedTxnId::new();
        let doc = Doc {
            trntype: t.trntype,
            fitid_synthesized: t.fitid_synthesized,
            source: source.clone(),
            suggestion: None,
            raw: t.raw.clone(),
        };
        let n = w.tx.execute(
            "INSERT INTO feed_txn (id, connection_id, account_id, fitid, posted_date, amount, payee, memo,
                                   check_number, status, matched_txn_id, doc, imported_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'new', NULL, ?10, ?11)
             ON CONFLICT(account_id, fitid) DO NOTHING",
            params![
                id.to_string(),
                opts.connection_id.map(|c| c.to_string()),
                account.to_string(),
                t.fitid,
                SqlDate(t.posted),
                t.amount.minor(),
                t.payee,
                t.memo,
                t.check_number,
                codec::to_doc(&doc)?,
                now,
            ],
        )?;
        if n == 0 {
            out.duplicates += 1;
        } else {
            out.imported.push(id);
        }
    }
    if !out.imported.is_empty() {
        w.changed("feed", account.to_string(), "import");
    }
    Ok(out)
}

/// Filters for the queue (the Bank Feeds window's lists).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct FeedFilter {
    pub account_id: Option<AccountId>,
    pub connection_id: Option<FeedConnectionId>,
    /// Any of these; empty for all.
    pub statuses: Vec<FeedStatus>,
    pub from: Option<Date>,
    pub to: Option<Date>,
    /// Matches payee, memo, check number or the bank's id.
    pub text: Option<String>,
    pub amount: Option<Money>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    /// Newest first when true.
    pub descending: bool,
}

/// Find downloaded transactions.
pub fn query(c: &Connection, f: &FeedFilter) -> Result<Vec<FeedTxn>> {
    let mut sql = format!("SELECT {COLS} FROM feed_txn WHERE 1 = 1");
    let mut args: Vec<Box<dyn ToSql>> = Vec::new();
    let mut bind = |sql: &mut String, clause: &str, v: Box<dyn ToSql>| {
        args.push(v);
        sql.push_str(&clause.replace('?', &format!("?{}", args.len())));
    };
    if let Some(a) = f.account_id {
        bind(&mut sql, " AND account_id = ?", Box::new(a.to_string()));
    }
    if let Some(cid) = f.connection_id {
        bind(
            &mut sql,
            " AND connection_id = ?",
            Box::new(cid.to_string()),
        );
    }
    if let Some(d) = f.from {
        bind(&mut sql, " AND posted_date >= ?", Box::new(SqlDate(d)));
    }
    if let Some(d) = f.to {
        bind(&mut sql, " AND posted_date <= ?", Box::new(SqlDate(d)));
    }
    if let Some(m) = f.amount {
        bind(&mut sql, " AND abs(amount) = ?", Box::new(m.minor().abs()));
    }
    if let Some(text) = f.text.as_deref().filter(|t| !t.trim().is_empty()) {
        let like = format!(
            "%{}%",
            text.trim()
                .replace('!', "!!")
                .replace('%', "!%")
                .replace('_', "!_")
        );
        bind(
            &mut sql,
            " AND (payee LIKE ? ESCAPE '!' OR memo LIKE ? ESCAPE '!' OR check_number LIKE ? ESCAPE '!' OR fitid LIKE ? ESCAPE '!')",
            Box::new(like),
        );
    }
    if !f.statuses.is_empty() {
        let list = f
            .statuses
            .iter()
            .map(|s| format!("'{}'", s.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" AND status IN ({list})"));
    }
    sql.push_str(if f.descending {
        " ORDER BY posted_date DESC, rowid DESC"
    } else {
        " ORDER BY posted_date, rowid"
    });
    sql.push_str(&format!(
        " LIMIT {} OFFSET {}",
        f.limit.unwrap_or(500).min(10_000),
        f.offset.unwrap_or(0)
    ));
    let mut stmt = c.prepare(&sql)?;
    let params: Vec<&dyn ToSql> = args.iter().map(AsRef::as_ref).collect();
    let rows = stmt.query_map(params.as_slice(), row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r??);
    }
    Ok(out)
}

/// One account's line in the Bank Feeds centre.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FeedAccountSummary {
    pub account_id: AccountId,
    pub new: u32,
    pub matched: u32,
    pub added: u32,
    pub ignored: u32,
    /// The newest bank date in the queue.
    pub latest_posted: Option<Date>,
    pub last_imported_at: Option<jiff::Timestamp>,
}

/// Counts per account, for the Bank Feeds centre.
pub fn summary(c: &Connection) -> Result<Vec<FeedAccountSummary>> {
    let mut stmt = c.prepare(
        "SELECT f.account_id,
                SUM(f.status = 'new'), SUM(f.status = 'matched'), SUM(f.status = 'added'), SUM(f.status = 'ignored'),
                MAX(f.posted_date), MAX(f.imported_at)
         FROM feed_txn f JOIN account a ON a.id = f.account_id
         GROUP BY f.account_id ORDER BY MIN(a.sort_key)",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, Option<SqlDate>>(5)?,
            r.get::<_, Option<String>>(6)?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        let (a, n, m, ad, ig, latest, imported) = r?;
        out.push(FeedAccountSummary {
            account_id: codec::parse_id(&a)?,
            new: n as u32,
            matched: m as u32,
            added: ad as u32,
            ignored: ig as u32,
            latest_posted: latest.map(|d| d.0),
            last_imported_at: imported.and_then(|t| t.parse().ok()),
        });
    }
    Ok(out)
}

fn write_doc_field(w: &mut Writer<'_>, id: FeedTxnId, f: impl FnOnce(&mut Doc)) -> Result<()> {
    let doc: String =
        w.tx.query_row(
            "SELECT doc FROM feed_txn WHERE id = ?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| FeedError::NotFound {
            kind: "downloaded transaction",
            id: id.to_string(),
        })?;
    let mut d: Doc = codec::from_doc(&doc)?;
    f(&mut d);
    w.tx.execute(
        "UPDATE feed_txn SET doc = ?2 WHERE id = ?1",
        params![id.to_string(), codec::to_doc(&d)?],
    )?;
    Ok(())
}

/// Store (or clear) what to fill in when this is added.
pub fn set_suggestion(
    w: &mut Writer<'_>,
    id: FeedTxnId,
    suggestion: Option<Suggestion>,
) -> Result<()> {
    write_doc_field(w, id, |d| d.suggestion = suggestion)?;
    w.changed("feed_txn", id.to_string(), "update");
    Ok(())
}

/// Change the memo a downloaded transaction carries (Amazon order details, say),
/// keeping the bank's original in `raw.original_memo`.
pub fn set_memo(
    w: &mut Writer<'_>,
    id: FeedTxnId,
    memo: &str,
    extra: Option<(&str, serde_json::Value)>,
) -> Result<()> {
    let old: Option<String> = w.tx.query_row(
        "SELECT memo FROM feed_txn WHERE id = ?1",
        [id.to_string()],
        |r| r.get(0),
    )?;
    write_doc_field(w, id, |d| {
        if !d.raw.is_object() {
            let original = std::mem::take(&mut d.raw);
            d.raw = serde_json::json!({ "source": original });
        }
        if let Some(obj) = d.raw.as_object_mut() {
            if !obj.contains_key("original_memo") {
                obj.insert(
                    "original_memo".into(),
                    old.map_or(serde_json::Value::Null, serde_json::Value::String),
                );
            }
            if let Some((k, v)) = extra {
                obj.insert(k.to_owned(), v);
            }
        }
    })?;
    w.tx.execute(
        "UPDATE feed_txn SET memo = ?2 WHERE id = ?1",
        params![id.to_string(), memo],
    )?;
    w.changed("feed_txn", id.to_string(), "update");
    Ok(())
}

/// Set a row's status and link. Only for this crate: matching and adding go
/// through their own functions, which check what they link to.
pub(crate) fn set_status(
    w: &mut Writer<'_>,
    id: FeedTxnId,
    status: FeedStatus,
    txn: Option<TxnId>,
) -> Result<()> {
    let n = w.tx.execute(
        "UPDATE feed_txn SET status = ?2, matched_txn_id = ?3 WHERE id = ?1",
        params![id.to_string(), status.as_str(), txn.map(|t| t.to_string())],
    )?;
    if n == 0 {
        return Err(FeedError::NotFound {
            kind: "downloaded transaction",
            id: id.to_string(),
        });
    }
    w.changed("feed_txn", id.to_string(), status.as_str());
    Ok(())
}

/// Ignore downloaded transactions that are still waiting (QBD's "Ignore").
pub fn ignore(w: &mut Writer<'_>, ids: &[FeedTxnId]) -> Result<u32> {
    let mut n = 0;
    for &id in ids {
        let f = get(&w.tx, id)?;
        if f.status == FeedStatus::New {
            set_status(w, id, FeedStatus::Ignored, None)?;
            n += 1;
        }
    }
    Ok(n)
}

/// Reverse a decision: an ignored row comes back; a matched row is unlinked and
/// its register line uncleared; an added row's transaction is deleted (it was
/// created only from this download). Returns the row as it now stands.
pub fn undo(w: &mut Writer<'_>, id: FeedTxnId) -> Result<FeedTxn> {
    let f = get(&w.tx, id)?;
    match (f.status, f.matched_txn_id) {
        (FeedStatus::New, _) => {}
        (FeedStatus::Ignored, _) | (FeedStatus::Matched | FeedStatus::Added, None) => {
            set_status(w, id, FeedStatus::New, None)?;
        }
        (FeedStatus::Matched, Some(t)) => {
            set_status(w, id, FeedStatus::New, None)?;
            if txns::find(&w.tx, t)?.is_some() {
                balances::set_downloaded(w, t, f.account_id, false)?;
            }
        }
        (FeedStatus::Added, Some(t)) => {
            // Unlink first: the link would otherwise stop the delete.
            set_status(w, id, FeedStatus::New, None)?;
            if let Some(txn) = txns::find(&w.tx, t)? {
                txns::delete(w, t, txn.edit_seq)?;
            }
        }
    }
    get(&w.tx, id)
}

/// The downloaded transaction linked to a register transaction, if any (for the
/// register's "downloaded" mark and to keep two feed rows off one transaction).
pub fn linked_to(c: &Connection, txn: TxnId) -> Result<Option<FeedTxnId>> {
    let id: Option<String> = c
        .query_row(
            "SELECT id FROM feed_txn WHERE matched_txn_id = ?1 AND status IN ('matched', 'added') LIMIT 1",
            [txn.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    id.map(|s| {
        s.parse()
            .map_err(|_| lb_store::StoreError::Corrupt("feed transaction id".into()).into())
    })
    .transpose()
}
