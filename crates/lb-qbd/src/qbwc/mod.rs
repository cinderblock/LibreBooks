//! The QuickBooks Web Connector transport.
//!
//! The Web Connector ships free with QuickBooks Desktop. Given a `.qwc` file
//! ([`qwc`]) it polls a SOAP web service ([`soap`]) on a schedule or when the
//! user clicks *Update Selected*, runs the qbXML the service hands back inside
//! QuickBooks, and posts the answers. LibreBooks is that service, so a server on
//! another machine imports from QuickBooks with nothing installed beside
//! QuickBooks itself.
//!
//! That inverts the sync engine's pull: [`queue`] turns each `list`/`get` into a
//! parked request the Web Connector collects, and [`session`] is the protocol of
//! one update run. The server (`lb-server`) owns authentication, the company
//! lookup and the HTTP endpoint; everything here is testable without it.
//!
//! Read-only twice over: the documents come from
//! [`crate::qbxml::request`] and pass its [`check_read_only`](crate::qbxml::request::check_read_only),
//! and the `.qwc` sets `IsReadOnly`, so QuickBooks grants the connection
//! read-only access when the user first approves it.

pub mod queue;
pub mod qwc;
pub mod session;
pub mod soap;

pub use queue::{
    Next, Queue, QueueConfig, QueueHandle, QueuedSource, Settled, queue, queued_source,
};
pub use qwc::{QwcError, QwcFile};
pub use session::{NO_OP, Session, SessionConfig};
pub use soap::{AuthOutcome, Call, SoapError};
