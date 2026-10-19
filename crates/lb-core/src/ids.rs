//! Typed identifiers. Every record is a UUIDv7 (time-ordered, so new rows land at
//! the end of indexes), wrapped so an account id cannot be passed where an item id
//! is wanted. Ids imported from QuickBooks (`ListID`, `TxnID`) are kept beside
//! them in the store's external-reference table, never used as ids themselves.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
        #[serde(transparent)]
        #[ts(export, type = "string")]
        pub struct $name(pub Uuid);

        impl $name {
            /// A fresh, time-ordered id.
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    };
}

id_type!(
    /// A company (one set of books, one database file).
    CompanyId
);
id_type!(
    /// An account in the chart of accounts.
    AccountId
);
id_type!(
    /// A name: customer (or job), vendor, employee or other name.
    EntityId
);
id_type!(
    /// An item: service, inventory part, discount, sales tax and so on.
    ItemId
);
id_type!(
    /// A class (QBD's class tracking).
    ClassId
);
id_type!(
    /// Payment terms.
    TermsId
);
id_type!(
    /// A payment method (Cash, Check, Visa, …).
    PaymentMethodId
);
id_type!(
    /// A sales tax code (Tax / Non and friends).
    SalesTaxCodeId
);
id_type!(
    /// A transaction.
    TxnId
);
id_type!(
    /// A line on a transaction's form.
    LineId
);
id_type!(
    /// A user of the books.
    UserId
);
id_type!(
    /// A sales rep.
    SalesRepId
);
id_type!(
    /// A customer type, vendor type or job type.
    TypeId
);
id_type!(
    /// A stored attachment (a receipt photo, a PDF).
    AttachmentId
);
id_type!(
    /// A bank reconciliation.
    ReconciliationId
);
