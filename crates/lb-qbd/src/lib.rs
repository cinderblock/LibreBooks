//! QuickBooks Desktop import, read-only.
//!
//! LibreBooks dogfoods the author's real books by reading them out of a running
//! QuickBooks Desktop (QBD) for as long as QBD stays the system of record. This
//! crate does that, and nothing here can change QBD:
//!
//! - [`client`]: the QB Bridge client (REST in front of QBD). GET only, by
//!   construction; polite to a QuickBooks someone is using; pages every list.
//! - [`source`]: a [`QbdSource`] over any transport that carries qbXML
//!   documents, built from [`qbxml::request`] (queries only, by construction):
//!   - [`qbwc`]: the QuickBooks Web Connector (free with QBD) polls LibreBooks'
//!     SOAP endpoint and runs the qbXML it hands back; the queue inverts that
//!     into the pull the sync engine wants.
//!   - [`sdk`]: the QuickBooks SDK over COM, when LibreBooks runs on the
//!     QuickBooks machine (Windows, feature `qbsdk`).
//! - [`config`]: a company's import settings (which transport, how to reach it).
//! - [`qbxml`]: the normaliser, from qbXML-shaped JSON to `lb_core` types, shared
//!   by every transport; and [`qbxml::xml`], which turns qbXML documents into the
//!   same JSON (and back, for tests).
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
pub mod config;
pub mod entity;
pub mod parity;
pub mod qbwc;
pub mod qbxml;
#[cfg(all(windows, feature = "qbsdk"))]
pub mod sdk;
pub mod source;
pub mod sync;
pub mod testing;

pub use client::{
    Active, Basis, BridgeClient, BridgeConfig, ListQuery, QbdError, QbdSource, ReportQuery,
};
pub use config::{ImportSettings, QbdTransport};
pub use entity::QbdEntity;
pub use parity::{ParityOptions, ParityReport, ParityRow, parity};
pub use source::{QbxmlSource, QbxmlTransport};
pub use sync::{SyncError, SyncOptions, SyncProgress, SyncReport, sync};
