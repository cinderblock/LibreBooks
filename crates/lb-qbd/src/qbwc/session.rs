//! One Web Connector session over a [`QueueHandle`]: the method-by-method
//! protocol of the Web Connector's update loop, independent of who the user is
//! (the server authenticates and finds the company; this does the rest).
//!
//! The loop the Web Connector runs after `authenticate`:
//!
//! ```text
//! sendRequestXML ──qbXML──▶ QuickBooks ──response──▶ receiveResponseXML ──percent──┐
//!        ▲                                                                           │
//!        └───────────────────────────── < 100 ──────────────────────────────────────┘
//!                                        100 ──▶ closeConnection
//! ```
//!
//! An empty string from `sendRequestXML` ends the loop too; the Web Connector
//! then asks `getLastError`, and `NoOp` there means "nothing was wrong".

use std::time::Duration;

use super::queue::{Next, QueueHandle, Settled};

/// Timing for the SOAP methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    /// How long `sendRequestXML` waits for the engine's next request before
    /// sending a keep-alive.
    pub poll_wait: Duration,
    /// How long `receiveResponseXML` waits for the engine to queue the next
    /// request (or finish) before answering with a percentage.
    pub settle_wait: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            poll_wait: Duration::from_secs(20),
            settle_wait: Duration::from_secs(5),
        }
    }
}

/// The `getLastError` answer that means "no error".
pub const NO_OP: &str = "NoOp";

#[derive(Clone, Debug)]
pub struct Session {
    handle: QueueHandle,
    config: SessionConfig,
}

impl Session {
    pub fn new(handle: QueueHandle, config: SessionConfig) -> Session {
        handle.session_opened();
        Session { handle, config }
    }

    pub fn handle(&self) -> &QueueHandle {
        &self.handle
    }

    /// `sendRequestXML`: the next document, a keep-alive, or `""` when the run
    /// is over.
    pub async fn send_request_xml(&self) -> String {
        match self.handle.next_request(self.config.poll_wait).await {
            Next::Request(xml) | Next::Heartbeat(xml) => xml,
            Next::Finished | Next::Closed(_) => String::new(),
        }
    }

    /// `receiveResponseXML`: hand the answer over, then say how far along the
    /// run is: 100 ends the session, anything lower brings the Web Connector
    /// back for the next request, negative reports an error.
    pub async fn receive_response_xml(
        &self,
        response: String,
        hresult: &str,
        message: &str,
    ) -> i32 {
        if !hresult.trim().is_empty() {
            self.handle.fail(format!(
                "QuickBooks failed the request ({}): {}",
                hresult.trim(),
                message.trim()
            ));
            return -1;
        }
        if let Err(e) = self.handle.complete(response) {
            tracing::warn!(error = %e, "unexpected Web Connector response");
            return -1;
        }
        match self.handle.settle(self.config.settle_wait).await {
            Settled::Finished => 100,
            Settled::Pending | Settled::Working => i32::from(self.handle.percent().clamp(1, 99)),
        }
    }

    /// `getLastError`.
    pub fn get_last_error(&self) -> String {
        if self.handle.is_closed() {
            return self
                .handle
                .last_error()
                .unwrap_or_else(|| "the session was closed".into());
        }
        NO_OP.to_owned()
    }

    /// `connectionError`: the Web Connector couldn't reach QuickBooks. Ends the
    /// run; `done` tells the Web Connector not to retry with another file.
    pub fn connection_error(&self, hresult: &str, message: &str) -> String {
        self.handle.fail(format!(
            "the Web Connector couldn't talk to QuickBooks ({}): {}",
            hresult.trim(),
            message.trim()
        ));
        "done".to_owned()
    }

    /// `closeConnection`: a line for the Web Connector's log.
    pub fn close_connection(&self) -> String {
        let answered = self.handle.answered();
        if self.handle.is_closed() {
            format!(
                "LibreBooks: stopped after {answered} request(s): {}",
                self.handle.last_error().unwrap_or_default()
            )
        } else if self.handle.is_finished() {
            format!("LibreBooks: done, {answered} request(s) answered")
        } else {
            // The Web Connector left while the engine still had work: end it.
            self.handle
                .close("the Web Connector closed the connection before the import finished");
            format!("LibreBooks: connection closed with work left after {answered} request(s)")
        }
    }
}
