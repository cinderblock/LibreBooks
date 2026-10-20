//! QuickBooks Desktop import, read-only.
//!
//! LibreBooks dogfoods the author's real books by reading them out of a running
//! QuickBooks Desktop (QBD) for as long as QBD stays the system of record. This
//! crate does that, and nothing here can change QBD:
//!
//! - [`client`]: the QB Bridge client (REST in front of QBD). GET only, by
//!   construction; polite to a QuickBooks someone is using; pages every list.
//! - [`qbxml`]: the normaliser, from qbXML-shaped JSON to `lb_core` types, shared
//!   by every transport; and [`qbxml::xml`], which turns qbXML documents (what the
//!   QuickBooks Web Connector speaks) into the same JSON.
//! - [`sync`]: incremental, idempotent import into an `lb_store::Books`, with a
//!   report of everything that didn't fit.
//! - [`parity`]: LibreBooks' balances against QBD's own, account by account and
//!   on the trial balance.
//! - [`testing`]: an in-memory QuickBooks for tests.
//!
//! What the bridge actually does (paging quirks, what an account's `Balance`
//! means, which transaction types it can't supply) is written up in
//! `plans/qbd-import.md`, with the probes that showed it.

pub mod client;
pub mod entity;
pub mod parity;
pub mod qbxml;
pub mod sync;
pub mod testing;

pub use client::{
    Active, Basis, BridgeClient, BridgeConfig, BridgeError, ListQuery, QbdSource, ReportQuery,
};
pub use entity::QbdEntity;
pub use parity::{ParityOptions, ParityReport, ParityRow, parity};
pub use sync::{SyncError, SyncOptions, SyncProgress, SyncReport, sync};
