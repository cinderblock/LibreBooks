//! Typed ids for feed records, in the same style as `lb_core::ids` (UUIDv7,
//! wrapped so one kind can't be passed for another).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

macro_rules! feed_id {
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

feed_id!(
    /// A feed connection: where an account's transactions come from.
    FeedConnectionId
);
feed_id!(
    /// One downloaded transaction in the Bank Feeds queue.
    FeedTxnId
);
feed_id!(
    /// A renaming / categorising rule.
    FeedRuleId
);
