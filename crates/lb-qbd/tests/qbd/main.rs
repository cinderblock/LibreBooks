//! lb-qbd end to end: the normaliser on synthetic qbXML-shaped records, a full and
//! incremental sync into an in-memory company, the parity check, the bridge
//! client against a mock bridge over HTTP, and the read-only guarantee.
//!
//! The fixtures (`testdata/larkspur/`) are a made-up company in the real shapes
//! the QB Bridge returns. Its books were worked out by hand; see `sync.rs` for the
//! figures and where they come from.

mod client;
mod common;
mod live;
mod normalise;
mod parity;
mod read_only;
mod sync;
mod xml;
