//! A read-only session with the QuickBooks SDK (`QBXMLRP2.RequestProcessor`)
//! over COM, usable from two places:
//!
//! - in-process, through `lb_qbd::sdk`, which is tried first;
//! - in `librebooks-qbsdk`, a small 32-bit helper this crate builds, for when
//!   the 64-bit app can't reach QuickBooks itself.
//!
//! **Why a helper exists.** A 32-bit DLL can't load into a 64-bit process.
//! QuickBooks went 64-bit with 2022, and the 64-bit SDK only works from then on:
//! against QuickBooks 2021 (`QBW32.EXE`), a 64-bit process creates the request
//! processor but `BeginSession` fails with *Could not start QuickBooks*
//! (`0x80040408`), because the `QBInstanceFinder` classes it needs only load in
//! 32-bit. QuickBooks' own log (`qbsdklog.txt`) says so; the error doesn't.
//!
//! The safety of a session lives here rather than in either caller, so the two
//! routes can't differ: see [`session`]. The crate keeps its dependencies few
//! because it's built for `i686-pc-windows-msvc`.

#[cfg(all(windows, feature = "com"))]
pub mod installs;
pub mod read_only;
#[cfg(all(windows, feature = "com"))]
pub mod session;
pub mod wire;

/// How to open the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// Intuit's application id; empty for an application without one.
    pub app_id: String,
    /// Shown in QuickBooks' authorisation dialog and preferences.
    pub app_name: String,
    /// The `.QBW` to open; empty for the company QuickBooks has open.
    pub company_file: String,
    pub file_mode: FileMode,
}

/// A QuickBooks installed on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    /// QuickBooks' major version: 24 is 2014 (Enterprise 14.0), 31 is 2021.
    pub major: u32,
    /// What QuickBooks calls itself ("QuickBooks Pro 2021").
    pub product: String,
    /// Its program (`qbw32.exe`, or `QBW.EXE` from 2022).
    pub exe: std::path::PathBuf,
}

/// A QuickBooks program running in this Windows session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub pid: u32,
    /// The install it was started from, when it matches one.
    pub major: Option<u32>,
    pub exe: std::path::PathBuf,
}

/// How the SDK opens the company file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileMode {
    DontCare,
    SingleUser,
    MultiUser,
}

/// Why a session didn't open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenError {
    pub message: String,
    /// The SDK couldn't reach QuickBooks from this process, as opposed to
    /// QuickBooks answering and refusing. Only this is worth another route: a
    /// refused authorisation or a read-only grant QuickBooks didn't honour would
    /// be the same through the helper.
    pub unreachable: bool,
}

impl OpenError {
    pub fn new(message: impl Into<String>) -> OpenError {
        OpenError {
            message: message.into(),
            unreachable: false,
        }
    }
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[cfg(test)]
mod tests {
    /// A session asks for read-only access between connecting and opening, checks
    /// QuickBooks honoured it, and checks each document before sending it. COM
    /// can't run here without QuickBooks, so pin the order in the source.
    #[test]
    fn a_session_is_read_only_before_any_request() {
        let src = include_str!("session.rs");
        let body = &src[src.find("fn open(config").expect("open fn")..];
        let at = |needle: &str| {
            body.find(needle)
                .unwrap_or_else(|| panic!("{needle} missing"))
        };
        let steps = [
            at("\"OpenConnection2\""),
            at("rp.request_read_only()"),
            at("\"BeginSession\""),
            at("rp.read_only_obeyed(&ticket)"),
            at("read_only::check(xml)"),
            at("\"ProcessRequest\""),
        ];
        assert!(steps.is_sorted(), "out of order: {steps:?}");
    }
}
