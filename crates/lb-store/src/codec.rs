//! Converting between Rust values and SQLite columns.

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{Row, ToSql};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{Result, StoreError};

/// Parse a JSON `doc` column.
pub fn from_doc<T: DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_str(s).map_err(StoreError::from)
}

pub fn to_doc<T: Serialize + ?Sized>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(StoreError::from)
}

/// Read column `idx` as a doc.
pub fn doc<T: DeserializeOwned>(row: &Row<'_>, idx: usize) -> rusqlite::Result<T> {
    let s: String = row.get(idx)?;
    serde_json::from_str(&s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
    })
}

/// A civil date stored as `YYYY-MM-DD` text (sorts correctly).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SqlDate(pub jiff::civil::Date);

impl ToSql for SqlDate {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.0.to_string()))
    }
}

impl FromSql for SqlDate {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let s = value.as_str()?;
        s.parse::<jiff::civil::Date>()
            .map(SqlDate)
            .map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

pub fn date(d: jiff::civil::Date) -> SqlDate {
    SqlDate(d)
}

pub fn opt_date(d: Option<jiff::civil::Date>) -> Option<SqlDate> {
    d.map(SqlDate)
}

/// Any of lb-core's typed ids, as text.
pub fn id<T: ToString>(v: &T) -> String {
    v.to_string()
}

pub fn opt_id<T: ToString>(v: Option<&T>) -> Option<String> {
    v.map(ToString::to_string)
}

/// Parse an id column.
pub fn parse_id<T: std::str::FromStr>(s: &str) -> Result<T> {
    s.parse::<T>()
        .map_err(|_| StoreError::Corrupt(format!("bad id {s:?}")))
}
