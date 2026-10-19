//! The normaliser: QuickBooks Desktop records, as qbXML-shaped JSON, into
//! LibreBooks' own types.
//!
//! Both transports produce the same JSON: the QB Bridge directly, and the Web
//! Connector's qbXML through [`xml`]. Everything here is pure. References to
//! other records (`AccountRef`, `CustomerRef`, a payment's `AppliedToTxnRet`) go
//! through a [`Resolver`], which the sync engine backs with the store (creating
//! the few lists the bridge doesn't expose, such as sales tax codes, as they turn
//! up) and tests back with maps.
//!
//! Faithfulness rules:
//! - QBD's meaning wins over LibreBooks' defaults: amounts are taken as QBD stored
//!   them, never recomputed from quantity × rate.
//! - Nothing is dropped silently. Transaction fields with no LibreBooks home go
//!   into `extra` verbatim; list fields with no home are reported by name.
//! - What can't be represented is an error or a note that says which record and
//!   why, never a guess.

pub mod lists;
pub mod record;
pub mod txns;
pub mod xml;

use lb_core::lists::TypeListKind;
use lb_core::{
    AccountId, ClassId, EntityId, ItemId, PaymentMethodId, SalesRepId, SalesTaxCodeId, TermsId,
    TxnId, TypeId,
};
use rust_decimal::Decimal;

pub use record::{FieldError, Meta, QbRef, Record};

/// Why a QBD record could not be turned into a LibreBooks one.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NormaliseError {
    #[error(transparent)]
    Field(#[from] FieldError),
    #[error("it refers to {what} {reference}, which hasn't been imported")]
    Unresolved {
        what: &'static str,
        reference: String,
    },
    #[error("{0}")]
    Unsupported(String),
    /// The resolver failed (a store error while creating a missing list entry).
    #[error("{0}")]
    Resolver(String),
}

/// Maps QBD ids to LibreBooks ids. Lookups return `Ok(None)` for a record that
/// isn't imported; the normaliser turns that into a clear error (or, where QBD's
/// data allows, a reported gap).
pub trait Resolver {
    fn account(&mut self, r: &QbRef) -> Result<Option<AccountId>, NormaliseError>;
    fn entity(&mut self, r: &QbRef) -> Result<Option<EntityId>, NormaliseError>;
    fn item(&mut self, r: &QbRef) -> Result<Option<ItemId>, NormaliseError>;
    fn class(&mut self, r: &QbRef) -> Result<Option<ClassId>, NormaliseError>;
    fn terms(&mut self, r: &QbRef) -> Result<Option<TermsId>, NormaliseError>;
    fn payment_method(&mut self, r: &QbRef) -> Result<Option<PaymentMethodId>, NormaliseError>;
    fn sales_rep(&mut self, r: &QbRef) -> Result<Option<SalesRepId>, NormaliseError>;
    /// Sales tax codes aren't listed by the bridge: find or create one.
    fn sales_tax_code(&mut self, r: &QbRef) -> Result<Option<SalesTaxCodeId>, NormaliseError>;
    /// Sales tax items aren't listed by the bridge either: find or create one
    /// charging `rate` percent (from the transaction that names it).
    fn sales_tax_item(
        &mut self,
        r: &QbRef,
        rate: Option<Decimal>,
    ) -> Result<Option<ItemId>, NormaliseError>;
    /// Customer, vendor and job types: find or create.
    fn type_entry(
        &mut self,
        list: TypeListKind,
        r: &QbRef,
    ) -> Result<Option<TypeId>, NormaliseError>;
    /// A transaction by its `TxnID`, and its qbXML `TxnType` when the record says
    /// (`AppliedToTxnRet`, `LinkedTxn` and deposit lines do).
    fn txn(
        &mut self,
        txn_type: Option<&str>,
        txn_id: &str,
    ) -> Result<Option<TxnId>, NormaliseError>;
}

/// Resolve a reference that must resolve.
pub(crate) fn require<T>(
    what: &'static str,
    r: &QbRef,
    found: Result<Option<T>, NormaliseError>,
) -> Result<T, NormaliseError> {
    found?.ok_or_else(|| NormaliseError::Unresolved {
        what,
        reference: r.to_string(),
    })
}

/// A normalised list record, with the QBD fields LibreBooks had nowhere to put.
#[derive(Clone, Debug, PartialEq)]
pub struct ListImport<T> {
    pub record: T,
    /// Field names (`Record.path`-qualified) that were present but not mapped.
    pub unmapped: Vec<String>,
    pub notes: Vec<String>,
}
