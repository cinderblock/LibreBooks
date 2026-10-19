//! Account feeds: getting bank, card and other transactions into the books as
//! automatically as possible, the way QuickBooks Desktop's Bank Feeds do.
//!
//! The pipeline, and the module for each step:
//!
//! 1. **Sources** turn into [`Statement`]s: OFX/QFX/QBO files ([`ofx`]), CSV
//!    files with a saved or detected mapping ([`csvfile`]), and online
//!    [`providers`] (SimpleFIN Bridge, OFX Direct Connect).
//! 2. **Import** ([`queue`]) lands each statement's transactions in the
//!    `feed_txn` table, once each: re-importing is harmless because the bank's id
//!    (or a stable synthesised one) is unique per account.
//! 3. **Rules** ([`rules`]) rename and categorise what came in.
//! 4. **Matching** ([`matching`]) finds what is already in the register and, when
//!    there is exactly one strong candidate, links it and marks it cleared.
//! 5. **Adding** ([`add`]) turns what is left into checks, deposits and card
//!    charges and credits, one at a time or all at once.
//!
//! [`ingest`] runs steps 2–5 together for a statement. [`amazon`] reads
//! Amazon's order history to explain card charges.
//!
//! Plans, decisions and the provider comparison: `plans/feeds.md`.

pub mod add;
pub mod amazon;
pub mod connections;
pub mod csvfile;
pub mod error;
pub mod ids;
pub mod ingest;
pub mod matching;
pub mod ofx;
pub mod providers;
pub mod queue;
pub mod rules;
pub mod secrets;
pub mod statement;
pub mod text;

pub use error::{FeedError, Result};
pub use ids::{FeedConnectionId, FeedRuleId, FeedTxnId};
pub use secrets::{Secret, SecretRef, Secrets};
pub use statement::{AccountKind, Statement, StatementAccount, StatementTxn, TrnType};

/// Run feed functions in a write transaction on `books`, keeping their own
/// error (rather than flattening it into a store error, as `Books::write`
/// would). Nothing is committed if `f` fails.
pub fn write<T>(
    books: &lb_store::Books,
    actor: &lb_store::Actor,
    f: impl FnOnce(&mut lb_store::Writer<'_>) -> Result<T>,
) -> Result<T> {
    let mut failure: Option<FeedError> = None;
    let out = books.write(actor, |w| match f(w) {
        Ok(v) => Ok(v),
        Err(e) => {
            failure = Some(e);
            // Any error rolls the transaction back; the real one is kept above.
            Err(lb_store::StoreError::Invalid(String::new()))
        }
    });
    match (out, failure) {
        (_, Some(e)) => Err(e),
        (Ok(v), None) => Ok(v),
        (Err(e), None) => Err(e.into()),
    }
}

/// Run feed functions in a read transaction on `books`.
pub fn read<T>(
    books: &lb_store::Books,
    f: impl FnOnce(&rusqlite::Connection) -> Result<T>,
) -> Result<T> {
    let mut failure: Option<FeedError> = None;
    let out = books.read(|c| match f(c) {
        Ok(v) => Ok(v),
        Err(e) => {
            failure = Some(e);
            Err(lb_store::StoreError::Invalid(String::new()))
        }
    });
    match (out, failure) {
        (_, Some(e)) => Err(e),
        (Ok(v), None) => Ok(v),
        (Err(e), None) => Err(e.into()),
    }
}

/// What kind of file some bytes are, for an import dialog that takes anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export)]
pub enum FileKind {
    /// OFX, QFX (Quicken) or QBO (QuickBooks Web Connect).
    Ofx,
    Csv,
}

/// Tell OFX from CSV by content, not by file name (banks name QBO files `.ofx`,
/// `.qfx` and `.txt` interchangeably).
pub fn sniff(bytes: &[u8]) -> FileKind {
    if ofx::looks_like_ofx(bytes) {
        FileKind::Ofx
    } else {
        FileKind::Csv
    }
}
