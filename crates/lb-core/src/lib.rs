//! The LibreBooks accounting core.
//!
//! Pure domain logic with no I/O: money, the chart of accounts, names, items, the
//! smaller lists, transactions as entered, and the rules that turn them into
//! double-entry postings. Storage ([`lb_store`]) and everything above it build on
//! these types; the web UI's TypeScript types are generated from them (`ts-rs`).

pub mod account;
pub mod company;
pub mod entity;
pub mod ids;
pub mod item;
pub mod lists;
pub mod money;
pub mod period;
pub mod posting;
pub mod testing;
pub mod txn;

pub use account::{Account, AccountClass, AccountRole, AccountType, CashFlowClass};
pub use company::{CompanyInfo, Preferences};
pub use entity::{Address, Entity, EntityKind, JobStatus};
pub use ids::*;
pub use item::{Item, ItemComponent, ItemType};
pub use money::{Money, MoneyParseError};
pub use period::{DateMacro, DateRange};
pub use posting::{AccountInfo, OpenTxnInfo, Posted, PostingContext, PostingError, post};
pub use txn::{
    Application, BillableStatus, ClearedStatus, HeaderEntity, LineKind, Posting, Txn, TxnLine,
    TxnType,
};
