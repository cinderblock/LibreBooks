//! Feed connections: where one LibreBooks account's transactions come from.
//!
//! A connection ties a register (a bank or card account) to a source: files the
//! person imports, a SimpleFIN account, an OFX Direct Connect login. It keeps
//! the source's settings (never its secrets: see [`crate::secrets`]), how to
//! read it (a CSV mapping, reversed signs), and how the last download went.
//! Stored in `feed_connection`, the whole record as JSON in `doc` with the
//! queried fields beside it.

use jiff::civil::Date;
use lb_core::{AccountId, AccountType};
use lb_store::{Writer, accounts, audit, codec};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::csvfile::CsvMapping;
use crate::error::{FeedError, Result};
use crate::ids::FeedConnectionId;
use crate::providers::ofx_direct::OfxDirectSettings;
use crate::secrets::SecretRef;

/// A source and its settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "provider")]
pub enum ProviderSettings {
    /// Files the person downloads and imports: OFX/QFX/QBO, or CSV with this
    /// saved mapping.
    File { csv_mapping: Option<CsvMapping> },
    /// A SimpleFIN Bridge account. `access` names the access URL secret (it
    /// carries credentials); `remote_account_id` picks this account out of the
    /// bridge's list.
    SimpleFin {
        access: SecretRef,
        remote_account_id: String,
    },
    /// The bank's own OFX server, as QuickBooks' Direct Connect uses.
    OfxDirect(OfxDirectSettings),
    /// Plaid with the user's own keys. Not implemented yet (`plans/feeds.md`).
    Plaid {
        access_token: SecretRef,
        remote_account_id: String,
    },
    /// Teller with the user's own certificate. Not implemented yet.
    Teller {
        access_token: SecretRef,
        remote_account_id: String,
    },
}

impl ProviderSettings {
    /// The `provider` column's value.
    pub const fn kind(&self) -> &'static str {
        match self {
            ProviderSettings::File { .. } => "file",
            ProviderSettings::SimpleFin { .. } => "simplefin",
            ProviderSettings::OfxDirect(_) => "ofx-direct",
            ProviderSettings::Plaid { .. } => "plaid",
            ProviderSettings::Teller { .. } => "teller",
        }
    }

    /// Does this source download by itself (as opposed to files)?
    pub const fn is_online(&self) -> bool {
        !matches!(self, ProviderSettings::File { .. })
    }
}

fn default_match_days() -> u32 {
    10
}

/// A connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FeedConnection {
    pub id: FeedConnectionId,
    pub name: String,
    /// The register it feeds. `None` while the person hasn't chosen yet (a new
    /// SimpleFIN account, say).
    pub account_id: Option<AccountId>,
    pub settings: ProviderSettings,
    /// The statement fingerprint ([`crate::StatementAccount::account_key`]) of
    /// the bank account, so the next file from it lands here without asking.
    pub account_key: Option<String>,
    /// The source writes money in as negative: flip every amount.
    #[serde(default)]
    pub reverse_signs: bool,
    /// How many days either side of the bank's date a register line may be to
    /// match. QBD uses a similar window; 10 suits most banks.
    #[serde(default = "default_match_days")]
    pub match_days: u32,
    /// Ignore downloaded transactions before this date (the day the books start,
    /// or the day the account was last reconciled).
    pub not_before: Option<Date>,
    pub last_sync_at: Option<jiff::Timestamp>,
    pub last_error: Option<String>,
    /// Edit sequence (optimistic concurrency); 0 for a new connection.
    #[serde(default)]
    #[ts(type = "number")]
    pub edit_seq: i64,
}

impl FeedConnection {
    /// A new connection with defaults.
    pub fn new(name: &str, account_id: Option<AccountId>, settings: ProviderSettings) -> Self {
        FeedConnection {
            id: FeedConnectionId::new(),
            name: name.to_owned(),
            account_id,
            settings,
            account_key: None,
            reverse_signs: false,
            match_days: default_match_days(),
            not_before: None,
            last_sync_at: None,
            last_error: None,
            edit_seq: 0,
        }
    }
}

/// Every connection, by name.
pub fn list(c: &Connection) -> Result<Vec<FeedConnection>> {
    let mut stmt = c.prepare("SELECT doc FROM feed_connection ORDER BY name COLLATE NOCASE")?;
    let rows = stmt.query_map([], |r| codec::doc::<FeedConnection>(r, 0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn find(c: &Connection, id: FeedConnectionId) -> Result<Option<FeedConnection>> {
    Ok(c.query_row(
        "SELECT doc FROM feed_connection WHERE id = ?1",
        [id.to_string()],
        |r| codec::doc(r, 0),
    )
    .optional()?)
}

pub fn get(c: &Connection, id: FeedConnectionId) -> Result<FeedConnection> {
    find(c, id)?.ok_or_else(|| FeedError::NotFound {
        kind: "feed connection",
        id: id.to_string(),
    })
}

/// The connection a statement with this fingerprint was imported through last
/// time.
pub fn by_account_key(c: &Connection, key: &str) -> Result<Option<FeedConnection>> {
    if key.is_empty() {
        return Ok(None);
    }
    Ok(list(c)?
        .into_iter()
        .find(|f| f.account_key.as_deref() == Some(key)))
}

/// The accounts a feed can feed: those with a register a bank statement can be
/// checked against.
pub fn check_feedable(c: &Connection, account: AccountId) -> Result<AccountType> {
    let a = accounts::find(c, account)?.ok_or_else(|| FeedError::NotFound {
        kind: "account",
        id: account.to_string(),
    })?;
    if !matches!(
        a.account_type,
        AccountType::Bank
            | AccountType::CreditCard
            | AccountType::OtherCurrentAsset
            | AccountType::OtherCurrentLiability
            | AccountType::LongTermLiability
    ) {
        return Err(FeedError::Invalid(format!(
            "{} ({}) can't have a feed; feeds go into bank, credit card and similar accounts",
            a.full_name,
            a.account_type.label()
        )));
    }
    Ok(a.account_type)
}

/// Create or update a connection. `edit_seq` must match what is stored (0 for
/// new).
pub fn save(w: &mut Writer<'_>, mut f: FeedConnection) -> Result<FeedConnection> {
    let existing = find(&w.tx, f.id)?;
    match &existing {
        Some(x) if x.edit_seq != f.edit_seq => {
            return Err(lb_store::StoreError::Conflict {
                kind: "feed connection",
                id: f.id.to_string(),
                expected: f.edit_seq,
                actual: x.edit_seq,
            }
            .into());
        }
        None if f.edit_seq != 0 => {
            return Err(FeedError::NotFound {
                kind: "feed connection",
                id: f.id.to_string(),
            });
        }
        _ => {}
    }
    f.name = f.name.trim().to_owned();
    if f.name.is_empty() {
        return Err(FeedError::Invalid("give the connection a name".into()));
    }
    if let Some(a) = f.account_id {
        check_feedable(&w.tx, a)?;
    }
    if f.match_days > 60 {
        return Err(FeedError::Invalid(
            "the matching window can be at most 60 days".into(),
        ));
    }
    f.edit_seq += 1;
    let id = f.id.to_string();
    w.tx.execute(
        "INSERT INTO feed_connection (id, provider, name, account_id, doc, last_sync_at, last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET provider = excluded.provider, name = excluded.name,
           account_id = excluded.account_id, doc = excluded.doc, last_sync_at = excluded.last_sync_at,
           last_error = excluded.last_error",
        params![
            id,
            f.settings.kind(),
            f.name,
            f.account_id.map(|a| a.to_string()),
            codec::to_doc(&f)?,
            f.last_sync_at.map(|t| t.to_string()),
            f.last_error,
        ],
    )?;
    let action = if existing.is_some() {
        "update"
    } else {
        "create"
    };
    audit::record(
        w,
        "feed_connection",
        &id,
        action,
        existing.as_ref(),
        Some(&f),
        None,
    )?;
    w.changed("feed_connection", id, action);
    Ok(f)
}

/// Delete a connection. Its downloaded transactions stay (detached), so the
/// history of what was matched and added isn't lost.
pub fn delete(w: &mut Writer<'_>, id: FeedConnectionId, edit_seq: i64) -> Result<()> {
    let f = get(&w.tx, id)?;
    if f.edit_seq != edit_seq {
        return Err(lb_store::StoreError::Conflict {
            kind: "feed connection",
            id: id.to_string(),
            expected: edit_seq,
            actual: f.edit_seq,
        }
        .into());
    }
    let sid = id.to_string();
    w.tx.execute(
        "UPDATE feed_txn SET connection_id = NULL WHERE connection_id = ?1",
        [&sid],
    )?;
    w.tx.execute("DELETE FROM feed_connection WHERE id = ?1", [&sid])?;
    audit::record::<FeedConnection, FeedConnection>(
        w,
        "feed_connection",
        &sid,
        "delete",
        Some(&f),
        None,
        None,
    )?;
    w.changed("feed_connection", sid, "delete");
    Ok(())
}

/// Record how a download went: the time, and the error (or none). Not a user
/// edit, so it doesn't bump the edit sequence or write an audit record.
pub fn record_sync(
    w: &mut Writer<'_>,
    id: FeedConnectionId,
    at: jiff::Timestamp,
    error: Option<&str>,
) -> Result<()> {
    let mut f = get(&w.tx, id)?;
    if error.is_none() {
        f.last_sync_at = Some(at);
    }
    f.last_error = error.map(str::to_owned);
    w.tx.execute(
        "UPDATE feed_connection SET doc = ?2, last_sync_at = ?3, last_error = ?4 WHERE id = ?1",
        params![
            id.to_string(),
            codec::to_doc(&f)?,
            f.last_sync_at.map(|t| t.to_string()),
            f.last_error
        ],
    )?;
    w.changed("feed_connection", id.to_string(), "sync");
    Ok(())
}
