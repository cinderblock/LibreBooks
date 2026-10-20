//! The request queue between a sync (which pulls: "give me the checks") and the
//! Web Connector (which polls: "any work?").
//!
//! [`Queue`] is the sync's side: a [`QbxmlTransport`] whose `exchange` parks the
//! document and waits for its answer. [`QueueHandle`] is the Web Connector's
//! side, driven by the SOAP handler: take the next document in `sendRequestXML`,
//! deliver the answer in `receiveResponseXML`. One document is outstanding at a
//! time, because the sync engine itself is sequential.
//!
//! Waiting politely: the Web Connector polls on its own schedule or when the
//! user clicks *Update Selected*, so the first request may wait a long while
//! (`first_poll_wait`). Once a session is open, every request should be collected
//! and answered within moments; one that isn't (`in_session_wait`) means the
//! Web Connector went away, and the queue closes so the sync ends with a clear
//! `aborted` instead of hanging. Closing fails every waiting request with
//! [`QbdError::Session`], which the engine treats as "stop the run".

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::{Notify, oneshot};

use crate::client::QbdError;
use crate::entity::QbdEntity;
use crate::qbxml::request::{self, Request};
use crate::source::{QbxmlSource, QbxmlTransport};
use crate::sync::SyncProgress;

/// How long requests may wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueConfig {
    /// Before a session is open: how long the first request waits for the Web
    /// Connector to poll at all.
    pub first_poll_wait: Duration,
    /// Once a session is open (or a request has been handed out): how long a
    /// request may wait for the Web Connector's next call.
    pub in_session_wait: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        QueueConfig {
            first_poll_wait: Duration::from_secs(2 * 60 * 60),
            in_session_wait: Duration::from_secs(10 * 60),
        }
    }
}

struct Pending {
    request: Request,
    reply: oneshot::Sender<Result<String, QbdError>>,
}

#[derive(Default)]
struct State {
    pending: VecDeque<Pending>,
    /// Handed to the Web Connector, awaiting its `receiveResponseXML`.
    in_flight: Option<(Pending, Instant)>,
    /// A `HostQueryRq` keep-alive is out; its answer is to be ignored.
    heartbeat_out: bool,
    session_open: bool,
    /// The engine is done with the queue: no more requests will come.
    finished: bool,
    /// The session is over for this reason; every request fails at once.
    closed: Option<String>,
    last_error: Option<String>,
    answered: u64,
}

struct Shared {
    config: QueueConfig,
    state: Mutex<State>,
    /// Anything changed: a request arrived, was taken, was answered, the queue
    /// closed or finished. `notify_waiters`, so every waiter re-checks.
    changed: Notify,
    percent: AtomicU8,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn close(&self, reason: String) {
        let mut st = self.lock();
        if st.closed.is_some() {
            return;
        }
        tracing::info!(reason = %reason, "Web Connector queue closed");
        st.closed = Some(reason.clone());
        st.last_error = Some(reason.clone());
        let failed = QbdError::Session { message: reason };
        if let Some((p, _)) = st.in_flight.take() {
            let _ = p.reply.send(Err(failed.clone()));
        }
        for p in st.pending.drain(..) {
            let _ = p.reply.send(Err(failed.clone()));
        }
        drop(st);
        self.changed.notify_waiters();
    }
}

/// The sync's side. Dropping it marks the queue finished (the engine is done).
pub struct Queue {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Queue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Queue").finish_non_exhaustive()
    }
}

/// The Web Connector's side. Cheap to clone.
#[derive(Clone)]
pub struct QueueHandle {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for QueueHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = self.shared.lock();
        f.debug_struct("QueueHandle")
            .field("pending", &st.pending.len())
            .field("in_flight", &st.in_flight.is_some())
            .field("finished", &st.finished)
            .field("closed", &st.closed)
            .finish()
    }
}

/// A new queue: the transport for a [`QbxmlSource`] and the handle the SOAP
/// handler drives.
pub fn queue(config: QueueConfig) -> (Queue, QueueHandle) {
    let shared = Arc::new(Shared {
        config,
        state: Mutex::new(State::default()),
        changed: Notify::new(),
        percent: AtomicU8::new(0),
    });
    (
        Queue {
            shared: shared.clone(),
        },
        QueueHandle { shared },
    )
}

/// A [`QbdSource`](crate::client::QbdSource) fed by the Web Connector.
pub type QueuedSource = QbxmlSource<Queue>;

/// A queued source and its handle.
pub fn queued_source(config: QueueConfig) -> (QueuedSource, QueueHandle) {
    let (q, h) = queue(config);
    (QbxmlSource::new(q), h)
}

impl Drop for Queue {
    fn drop(&mut self) {
        self.shared.lock().finished = true;
        self.shared.changed.notify_waiters();
    }
}

impl Queue {
    /// Say the engine is done (also happens when the queue is dropped).
    pub fn finish(&self) {
        self.shared.lock().finished = true;
        self.shared.changed.notify_waiters();
    }

    fn deadline(&self, request_id: u64, enqueued: Instant) -> Instant {
        let st = self.shared.lock();
        let c = &self.shared.config;
        match &st.in_flight {
            Some((p, taken)) if p.request.request_id == request_id => *taken + c.in_session_wait,
            _ if st.session_open => enqueued + c.in_session_wait,
            _ => enqueued + c.first_poll_wait,
        }
    }
}

impl QbxmlTransport for Queue {
    async fn exchange(&self, request: Request) -> Result<String, QbdError> {
        let (tx, mut rx) = oneshot::channel();
        let id = request.request_id;
        let name = request.name;
        {
            let mut st = self.shared.lock();
            if let Some(reason) = &st.closed {
                return Err(QbdError::Session {
                    message: reason.clone(),
                });
            }
            st.pending.push_back(Pending { request, reply: tx });
        }
        self.shared.changed.notify_waiters();
        let enqueued = Instant::now();
        loop {
            let changed = self.shared.changed.notified();
            let deadline = self.deadline(id, enqueued);
            tokio::select! {
                r = &mut rx => {
                    return r.unwrap_or_else(|_| Err(QbdError::Session {
                        message: "the queue was dropped".into(),
                    }));
                }
                () = changed => {}
                () = tokio::time::sleep_until(deadline.into()) => {
                    let waited = enqueued.elapsed();
                    let reason = format!(
                        "the QuickBooks Web Connector didn't collect or answer {name} within {}; is it running on the QuickBooks machine, and is this connection set to run?",
                        humanise(waited)
                    );
                    self.shared.close(reason);
                    return Err(QbdError::Timeout { path: name.to_owned() });
                }
            }
        }
    }
}

fn humanise(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{} h {} min", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{} min", s / 60)
    } else {
        format!("{s} s")
    }
}

/// What `sendRequestXML` should do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// Send this document.
    Request(String),
    /// The engine is busy locally; keep the session alive with this harmless
    /// document.
    Heartbeat(String),
    /// The engine finished: no more work.
    Finished,
    /// The session was closed for this reason.
    Closed(String),
}

/// What happened after an answer was delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settled {
    /// The engine finished: answer 100.
    Finished,
    /// The engine has already queued its next request.
    Pending,
    /// The engine is still working on what it got; the next `sendRequestXML`
    /// will wait for it.
    Working,
}

impl QueueHandle {
    /// A Web Connector session authenticated: requests now wait only the
    /// in-session time.
    pub fn session_opened(&self) {
        self.shared.lock().session_open = true;
        self.shared.changed.notify_waiters();
    }

    pub fn is_finished(&self) -> bool {
        self.shared.lock().finished
    }

    pub fn is_closed(&self) -> bool {
        self.shared.lock().closed.is_some()
    }

    /// Anything waiting to be sent, or in flight?
    pub fn has_work(&self) -> bool {
        let st = self.shared.lock();
        st.closed.is_none() && (!st.pending.is_empty() || st.in_flight.is_some())
    }

    /// End the session for this reason: every waiting request fails and the
    /// sync stops.
    pub fn close(&self, reason: impl Into<String>) {
        self.shared.close(reason.into());
    }

    /// The next document for the Web Connector, waiting up to `wait` for the
    /// engine to produce one. A document already in flight (the Web Connector
    /// asked again without answering) is handed out again.
    pub async fn next_request(&self, wait: Duration) -> Next {
        let deadline = Instant::now() + wait;
        loop {
            let changed = self.shared.changed.notified();
            {
                let mut st = self.shared.lock();
                if let Some(reason) = &st.closed {
                    return Next::Closed(reason.clone());
                }
                if let Some((p, _)) = &st.in_flight {
                    return Next::Request(p.request.xml.clone());
                }
                if let Some(p) = st.pending.pop_front() {
                    let xml = p.request.xml.clone();
                    st.in_flight = Some((p, Instant::now()));
                    drop(st);
                    self.shared.changed.notify_waiters();
                    return Next::Request(xml);
                }
                if st.finished {
                    return Next::Finished;
                }
            }
            let now = Instant::now();
            if now >= deadline {
                let mut st = self.shared.lock();
                st.heartbeat_out = true;
                return Next::Heartbeat(request::host(0).xml);
            }
            tokio::select! {
                () = changed => {}
                () = tokio::time::sleep(deadline - now) => {}
            }
        }
    }

    /// Deliver QuickBooks' answer to the document in flight. `Err` when nothing
    /// was in flight (and it wasn't a heartbeat's answer).
    pub fn complete(&self, response_xml: String) -> Result<(), String> {
        let mut st = self.shared.lock();
        if let Some((p, _)) = st.in_flight.take() {
            st.answered += 1;
            let _ = p.reply.send(Ok(response_xml));
            drop(st);
            self.shared.changed.notify_waiters();
            return Ok(());
        }
        if st.heartbeat_out {
            st.heartbeat_out = false;
            return Ok(());
        }
        let msg = "QuickBooks answered, but LibreBooks had asked nothing".to_owned();
        st.last_error = Some(msg.clone());
        Err(msg)
    }

    /// QuickBooks (or the Web Connector) failed the document in flight:
    /// `receiveResponseXML` with an `hresult`, or `connectionError`. Ends the
    /// session.
    pub fn fail(&self, message: impl Into<String>) {
        self.shared.close(message.into());
    }

    /// After [`QueueHandle::complete`]: wait up to `wait` for the engine to
    /// either queue its next request or finish.
    pub async fn settle(&self, wait: Duration) -> Settled {
        let deadline = Instant::now() + wait;
        loop {
            let changed = self.shared.changed.notified();
            {
                let st = self.shared.lock();
                if !st.pending.is_empty() || st.in_flight.is_some() {
                    return Settled::Pending;
                }
                if st.finished || st.closed.is_some() {
                    return Settled::Finished;
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return Settled::Working;
            }
            tokio::select! {
                () = changed => {}
                () = tokio::time::sleep(deadline - now) => {}
            }
        }
    }

    /// The last thing that went wrong, for `getLastError`.
    pub fn last_error(&self) -> Option<String> {
        self.shared.lock().last_error.clone()
    }

    pub fn answered(&self) -> u64 {
        self.shared.lock().answered
    }

    /// Percent done to report to the Web Connector (1–99 while running).
    pub fn percent(&self) -> u8 {
        self.shared.percent.load(Ordering::Relaxed)
    }

    /// Set the percent from the sync engine's progress callback.
    pub fn set_progress(&self, p: &SyncProgress) {
        self.shared.percent.store(percent_for(p), Ordering::Relaxed);
    }

    pub fn set_percent(&self, percent: u8) {
        self.shared
            .percent
            .store(percent.min(99), Ordering::Relaxed);
    }
}

/// A sync's progress as a single percentage: how far along the entity order it
/// is, plus how far through the current entity. Never 0 (the Web Connector
/// treats 0 oddly) and never 100 (that ends the session).
pub fn percent_for(p: &SyncProgress) -> u8 {
    let n = QbdEntity::SYNC_ORDER.len() as f64;
    let pos = QbdEntity::SYNC_ORDER
        .iter()
        .position(|e| *e == p.entity)
        .unwrap_or(0) as f64;
    let within = if p.total == 0 {
        0.0
    } else {
        (p.done as f64 / p.total as f64).clamp(0.0, 1.0)
    };
    let pct = ((pos + within) / n * 100.0).round() as u8;
    pct.clamp(1, 99)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::SyncPhase;

    fn fast() -> QueueConfig {
        QueueConfig {
            first_poll_wait: Duration::from_millis(200),
            in_session_wait: Duration::from_millis(200),
        }
    }

    #[tokio::test]
    async fn requests_are_handed_out_and_answered_in_order() {
        let (q, h) = queue(fast());
        let q = Arc::new(q);
        let worker = tokio::spawn({
            let q = q.clone();
            async move {
                let a = q.exchange(request::company(1)).await.unwrap();
                let b = q.exchange(request::host(2)).await.unwrap();
                (a, b)
            }
        });
        h.session_opened();
        let Next::Request(first) = h.next_request(Duration::from_secs(1)).await else {
            panic!("expected a request")
        };
        assert!(first.contains("CompanyQueryRq"));
        // Asked again before answering: the same document.
        assert_eq!(
            h.next_request(Duration::from_millis(10)).await,
            Next::Request(first.clone())
        );
        h.complete("<QBXML>1</QBXML>".into()).unwrap();
        assert_eq!(h.settle(Duration::from_secs(1)).await, Settled::Pending);
        let Next::Request(second) = h.next_request(Duration::from_secs(1)).await else {
            panic!("expected a second request")
        };
        assert!(second.contains("HostQueryRq"));
        h.complete("<QBXML>2</QBXML>".into()).unwrap();
        let (a, b) = worker.await.unwrap();
        assert_eq!(a, "<QBXML>1</QBXML>");
        assert_eq!(b, "<QBXML>2</QBXML>");
        drop(q);
        assert_eq!(h.settle(Duration::from_secs(1)).await, Settled::Finished);
        assert_eq!(
            h.next_request(Duration::from_millis(10)).await,
            Next::Finished
        );
        assert_eq!(h.answered(), 2);
    }

    #[tokio::test]
    async fn a_busy_engine_gets_a_heartbeat_and_an_unanswered_request_times_out() {
        let (q, h) = queue(fast());
        // Nothing queued yet and the engine isn't finished: heartbeat.
        match h.next_request(Duration::from_millis(20)).await {
            Next::Heartbeat(xml) => assert!(xml.contains("HostQueryRq")),
            other => panic!("{other:?}"),
        }
        assert!(h.complete("<QBXML/>".into()).is_ok());
        assert!(h.complete("<QBXML/>".into()).is_err());
        // A request nobody collects within the wait fails and closes the queue.
        let err = q.exchange(request::company(1)).await.unwrap_err();
        assert!(matches!(err, QbdError::Timeout { .. }), "{err}");
        assert!(h.is_closed());
        let err = q.exchange(request::company(2)).await.unwrap_err();
        assert!(matches!(err, QbdError::Session { .. }), "{err}");
        assert!(err.is_transient());
        assert!(matches!(
            h.next_request(Duration::from_millis(10)).await,
            Next::Closed(_)
        ));
        assert!(h.last_error().unwrap().contains("Web Connector"));
    }

    #[tokio::test]
    async fn a_connection_error_fails_the_request_in_flight() {
        let (q, h) = queue(fast());
        let worker = tokio::spawn(async move { q.exchange(request::company(1)).await });
        h.session_opened();
        assert!(matches!(
            h.next_request(Duration::from_secs(1)).await,
            Next::Request(_)
        ));
        h.fail("QuickBooks has no company open");
        let err = worker.await.unwrap().unwrap_err();
        assert_eq!(
            err.to_string(),
            "the QuickBooks session is gone: QuickBooks has no company open"
        );
    }

    #[test]
    fn percent_moves_through_the_sync_order_and_never_ends_the_session() {
        let p = |entity, done, total| SyncProgress {
            entity,
            phase: SyncPhase::Importing,
            done,
            total,
        };
        assert_eq!(percent_for(&p(QbdEntity::Account, 0, 0)), 1);
        let mid = percent_for(&p(QbdEntity::Invoice, 50, 100));
        assert!((40..70).contains(&mid), "{mid}");
        assert_eq!(percent_for(&p(QbdEntity::Deposit, 10, 10)), 99);
    }
}
