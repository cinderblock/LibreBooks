//! The company record: information, preferences, closing date, settings.

use lb_core::{AccountRole, CompanyInfo};
use rusqlite::{Connection, OptionalExtension, params};

use crate::codec::{self, SqlDate};
use crate::{Result, StoreError, Writer, accounts, now};

/// Write the company row and the accounts every company starts with.
pub(crate) fn init(w: &mut Writer<'_>, info: &CompanyInfo) -> Result<()> {
    let at = now();
    w.tx.execute(
        "INSERT INTO company (id, name, doc, closing_date, created_at, modified_at) VALUES (1, ?1, ?2, NULL, ?3, ?3)",
        params![info.name, codec::to_doc(info)?, at],
    )?;
    // QBD creates these with every company.
    for role in [
        AccountRole::AccountsReceivable,
        AccountRole::AccountsPayable,
        AccountRole::UndepositedFunds,
        AccountRole::OpeningBalanceEquity,
        AccountRole::RetainedEarnings,
    ] {
        accounts::ensure_role(w, role)?;
    }
    Ok(())
}

pub fn info(c: &Connection) -> Result<CompanyInfo> {
    let doc: String = c.query_row("SELECT doc FROM company WHERE id = 1", [], |r| r.get(0))?;
    codec::from_doc(&doc)
}

pub fn set_info(w: &mut Writer<'_>, info: &CompanyInfo) -> Result<()> {
    let before = self::info(&w.tx)?;
    w.tx.execute(
        "UPDATE company SET name = ?1, doc = ?2, modified_at = ?3 WHERE id = 1",
        params![info.name, codec::to_doc(info)?, now()],
    )?;
    crate::audit::record(w, "company", "1", "update", Some(&before), Some(info), None)?;
    w.changed("company", "1", "update");
    Ok(())
}

pub fn closing_date(c: &Connection) -> Result<Option<jiff::civil::Date>> {
    let d: Option<SqlDate> =
        c.query_row("SELECT closing_date FROM company WHERE id = 1", [], |r| {
            r.get(0)
        })?;
    Ok(d.map(|d| d.0))
}

pub fn set_closing_date(w: &mut Writer<'_>, date: Option<jiff::civil::Date>) -> Result<()> {
    let before = closing_date(&w.tx)?;
    w.tx.execute(
        "UPDATE company SET closing_date = ?1, modified_at = ?2 WHERE id = 1",
        params![codec::opt_date(date), now()],
    )?;
    crate::audit::record(
        w,
        "company",
        "1",
        "closing_date",
        Some(&before),
        Some(&date),
        None,
    )?;
    w.changed("company", "1", "update");
    Ok(())
}

/// Refuse a change dated on or before the closing date unless the actor may.
pub fn check_open_period(w: &Writer<'_>, date: jiff::civil::Date) -> Result<()> {
    if w.actor.may_override_closing_date {
        return Ok(());
    }
    if let Some(closing_date) = closing_date(&w.tx)?
        && date <= closing_date
    {
        return Err(StoreError::ClosedPeriod { closing_date });
    }
    Ok(())
}

pub fn setting(c: &Connection, key: &str) -> Result<Option<String>> {
    Ok(
        c.query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()?,
    )
}

pub fn set_setting(w: &mut Writer<'_>, key: &str, value: &str) -> Result<()> {
    w.tx.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}
