//! The QuickBooks SDK on this same Windows machine: no bridge, no Web Connector;
//! the desktop app or a server running beside QuickBooks imports straight from
//! it. The COM session and its read-only guarantees are in [`lb_qbsdk`]; this
//! is the route to it and the bridge to the async [`QbxmlTransport`].
//!
//! **Two routes.** The session is tried directly, in this process. If that can't
//! reach QuickBooks at all and this is a 64-bit build, which is what QuickBooks
//! 2021 and older answer (see [`lb_qbsdk`]), it's tried again through
//! `librebooks-qbsdk.exe`, the 32-bit helper beside this binary. A refusal is not
//! retried: QuickBooks would say the same to the helper. Either way a session
//! lives on one dedicated thread fed by a channel, which is what
//! [`SdkSession`] holds.
//!
//! **What QuickBooks asks the user.** The first time an application connects,
//! QuickBooks (running, with the company open) shows its *Application with no
//! certificate is requesting access* dialog naming the application
//! (`app_name`, "LibreBooks"); the user picks *Yes, always*. The choice is kept
//! under *Edit › Preferences › Integrated Applications*, where it can be
//! revoked. A headless service with no desktop can't answer the dialog:
//! authorise once from an interactive session (the desktop app, or `librebooks
//! import qbd --transport sdk` in a terminal) and the service inherits the
//! approval for the same application name.
//!
//! Only compiled on Windows with the `qbsdk` feature (on by default); the
//! settings type ([`crate::config::QbdTransport::Sdk`]) exists everywhere so a
//! Linux server can still show "not available here".

use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread::JoinHandle;

use lb_qbsdk::{OpenError, wire};
use tokio::sync::oneshot;

use crate::client::QbdError;
use crate::config::FileMode;
use crate::qbxml::request::Request;
use crate::source::{QbxmlSource, QbxmlTransport};

/// `CREATE_NO_WINDOW`: the helper is a console program, and the desktop app
/// has no console to share.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How to open the SDK session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdkConfig {
    /// Intuit's application id; empty for an application without one.
    pub app_id: String,
    /// Shown in QuickBooks' authorisation dialog and preferences.
    pub app_name: String,
    /// The `.QBW` to open; empty for the company QuickBooks has open.
    pub company_file: String,
    pub file_mode: FileMode,
}

impl Default for SdkConfig {
    fn default() -> Self {
        SdkConfig {
            app_id: String::new(),
            app_name: "LibreBooks".into(),
            company_file: String::new(),
            file_mode: FileMode::DontCare,
        }
    }
}

impl From<&SdkConfig> for lb_qbsdk::Config {
    fn from(c: &SdkConfig) -> Self {
        lb_qbsdk::Config {
            app_id: c.app_id.clone(),
            app_name: c.app_name.clone(),
            company_file: c.company_file.clone(),
            file_mode: match c.file_mode {
                FileMode::DontCare => lb_qbsdk::FileMode::DontCare,
                FileMode::SingleUser => lb_qbsdk::FileMode::SingleUser,
                FileMode::MultiUser => lb_qbsdk::FileMode::MultiUser,
            },
        }
    }
}

struct Job {
    xml: String,
    reply: oneshot::Sender<Result<String, QbdError>>,
}

/// An open SDK session on its own thread. Dropping it ends the session.
pub struct SdkSession {
    jobs: Mutex<Option<mpsc::Sender<Job>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    /// The company file QuickBooks reported open, once known.
    company_file: String,
    through_helper: bool,
}

impl std::fmt::Debug for SdkSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SdkSession")
            .field("company_file", &self.company_file)
            .field("through_helper", &self.through_helper)
            .finish_non_exhaustive()
    }
}

/// A [`QbdSource`](crate::client::QbdSource) over the SDK.
pub type SdkSource = QbxmlSource<SdkSession>;

/// Open a session and wrap it as a source.
pub fn sdk_source(config: &SdkConfig) -> Result<SdkSource, QbdError> {
    Ok(QbxmlSource::new(SdkSession::open(config)?))
}

fn session_error(message: impl Into<String>) -> QbdError {
    QbdError::Session {
        message: message.into(),
    }
}

/// Where to reach QuickBooks from.
enum Route {
    Direct,
    Helper(PathBuf),
}

/// Whether a failed direct attempt is worth repeating through the 32-bit helper.
fn helper_could_help(direct: &OpenError) -> bool {
    direct.unreachable && cfg!(target_pointer_width = "64")
}

/// The helper beside `exe`.
fn helper_beside(exe: &Path) -> PathBuf {
    exe.with_file_name(wire::HELPER_EXE)
}

impl SdkSession {
    /// Reach QuickBooks directly or, failing that, through the helper. Blocks
    /// until QuickBooks has accepted (or refused) the connection, which includes
    /// the time the user takes on the authorisation dialog the first time.
    pub fn open(config: &SdkConfig) -> Result<SdkSession, QbdError> {
        let direct = match SdkSession::start(config, Route::Direct) {
            Ok(session) => return Ok(session),
            Err(e) => e,
        };
        if !helper_could_help(&direct) {
            return Err(session_error(direct.message));
        }
        let exe = std::env::current_exe()
            .map_err(|e| session_error(format!("{}; {e}", direct.message)))?;
        let helper = helper_beside(&exe);
        if !helper.is_file() {
            return Err(session_error(format!(
                "{} QuickBooks 2021 and older need the 32-bit helper, and {} isn't there.",
                direct.message,
                helper.display()
            )));
        }
        tracing::info!(
            direct = %direct.message,
            "QuickBooks isn't reachable from this process; trying the 32-bit helper"
        );
        SdkSession::start(config, Route::Helper(helper)).map_err(|e| {
            session_error(format!(
                "{} Through the 32-bit helper: {}",
                direct.message, e.message
            ))
        })
    }

    fn start(config: &SdkConfig, route: Route) -> Result<SdkSession, OpenError> {
        let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<String, OpenError>>();
        let config = lb_qbsdk::Config::from(config);
        let through_helper = matches!(route, Route::Helper(_));
        let thread = std::thread::Builder::new()
            .name("quickbooks-sdk".into())
            .spawn(move || match route {
                Route::Direct => direct(&config, &ready_tx, &jobs_rx),
                Route::Helper(path) => helper(&path, &config, &ready_tx, &jobs_rx),
            })
            .map_err(|e| OpenError::new(format!("couldn't start the SDK thread: {e}")))?;
        let company_file = match ready_rx.recv() {
            Ok(Ok(file)) => file,
            Ok(Err(e)) => {
                let _ = thread.join();
                return Err(e);
            }
            Err(_) => {
                let _ = thread.join();
                return Err(OpenError::new(
                    "the SDK thread ended before the session opened",
                ));
            }
        };
        Ok(SdkSession {
            jobs: Mutex::new(Some(jobs_tx)),
            thread: Mutex::new(Some(thread)),
            company_file,
            through_helper,
        })
    }

    /// The company file QuickBooks has open in this session.
    pub fn company_file(&self) -> &str {
        &self.company_file
    }

    /// Whether this session runs through the 32-bit helper.
    pub fn through_helper(&self) -> bool {
        self.through_helper
    }

    /// End the session and wait for its thread.
    pub fn close(&self) {
        self.jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(t) = self
            .thread
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = t.join();
        }
    }
}

impl Drop for SdkSession {
    fn drop(&mut self) {
        self.close();
    }
}

impl QbxmlTransport for SdkSession {
    async fn exchange(&self, request: Request) -> Result<String, QbdError> {
        let (tx, rx) = oneshot::channel();
        {
            let jobs = self
                .jobs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(sender) = jobs.as_ref() else {
                return Err(session_error("the SDK session is closed"));
            };
            sender
                .send(Job {
                    xml: request.xml,
                    reply: tx,
                })
                .map_err(|_| session_error("the SDK thread has ended"))?;
        }
        rx.await
            .unwrap_or_else(|_| Err(session_error("the SDK thread dropped the request")))
    }
}

/// The session in this process: answer jobs until the channel closes.
fn direct(
    config: &lb_qbsdk::Config,
    ready: &mpsc::Sender<Result<String, OpenError>>,
    jobs: &mpsc::Receiver<Job>,
) {
    lb_qbsdk::session::run(config, |opened| match opened {
        Err(e) => {
            tracing::warn!(error = %e, "QuickBooks SDK session didn't open");
            let _ = ready.send(Err(e));
        }
        Ok(session) => {
            let _ = ready.send(Ok(session.company_file().to_owned()));
            while let Ok(job) = jobs.recv() {
                let answer = session
                    .exchange(&job.xml)
                    .map_err(|message| QbdError::Session { message });
                let _ = job.reply.send(answer);
            }
        }
    });
}

/// The session in the helper: start it and relay jobs to it.
fn helper(
    path: &Path,
    config: &lb_qbsdk::Config,
    ready: &mpsc::Sender<Result<String, OpenError>>,
    jobs: &mpsc::Receiver<Job>,
) {
    let spawned = Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            let _ = ready.send(Err(OpenError::new(format!(
                "couldn't start {}: {e}",
                path.display()
            ))));
            return;
        }
    };
    let (Some(to_helper), Some(from_helper)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = ready.send(Err(OpenError::new("the helper has no pipes")));
        let _ = child.kill();
        let _ = child.wait();
        return;
    };
    relay(to_helper, from_helper, config, ready, jobs);
    // `relay` dropped the helper's stdin, which ends its session.
    let _ = child.wait();
}

/// Say hello, then pass jobs to the helper and its replies back, until the
/// channel closes or the helper goes. Dropping `to` tells it to finish.
fn relay(
    mut to: impl Write,
    mut from: impl Read,
    config: &lb_qbsdk::Config,
    ready: &mpsc::Sender<Result<String, OpenError>>,
    jobs: &mpsc::Receiver<Job>,
) {
    let opened = wire::write_frame(&mut to, &wire::hello(config))
        .map_err(|e| e.to_string())
        .and_then(|()| wire::read_reply(&mut from).map_err(|e| e.to_string()));
    match opened {
        Ok(Some(Ok(file))) => {
            let _ = ready.send(Ok(file));
        }
        Ok(Some(Err(message))) => {
            let _ = ready.send(Err(OpenError::new(message)));
            return;
        }
        Ok(None) => {
            let _ = ready.send(Err(OpenError::new("the helper ended without answering")));
            return;
        }
        Err(e) => {
            let _ = ready.send(Err(OpenError::new(format!(
                "couldn't talk to the helper: {e}"
            ))));
            return;
        }
    }
    while let Ok(job) = jobs.recv() {
        let answer = wire::write_frame(&mut to, job.xml.as_bytes())
            .and_then(|()| wire::read_reply(&mut from));
        let (answer, gone) = match answer {
            Ok(Some(reply)) => (
                reply.map_err(|message| QbdError::Session { message }),
                false,
            ),
            Ok(None) => (Err(session_error("the helper has ended")), true),
            Err(e) => (
                Err(session_error(format!("couldn't talk to the helper: {e}"))),
                true,
            ),
        };
        let _ = job.reply.send(answer);
        if gone {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> lb_qbsdk::Config {
        (&SdkConfig::default()).into()
    }

    #[test]
    fn only_an_unreachable_quickbooks_is_tried_again() {
        let mut e = OpenError::new("Could not start QuickBooks.");
        assert!(!helper_could_help(&e), "a refusal isn't retried");
        e.unreachable = true;
        assert_eq!(helper_could_help(&e), cfg!(target_pointer_width = "64"));
    }

    #[test]
    fn the_helper_lives_beside_the_binary() {
        assert_eq!(
            helper_beside(Path::new(r"C:\Program Files\LibreBooks\librebooks.exe")),
            Path::new(r"C:\Program Files\LibreBooks\librebooks-qbsdk.exe")
        );
    }

    /// A pretend helper on the far end of two real pipes; `script` plays it.
    fn with_helper(
        script: impl FnOnce(&mut std::io::PipeReader, &mut std::io::PipeWriter) + Send + 'static,
        f: impl FnOnce(&mpsc::Receiver<Result<String, OpenError>>, mpsc::Sender<Job>),
    ) {
        let (mut from_app, to_helper) = std::io::pipe().unwrap();
        let (from_helper, mut to_app) = std::io::pipe().unwrap();
        let far = std::thread::spawn(move || script(&mut from_app, &mut to_app));
        let (ready_tx, ready_rx) = mpsc::channel();
        let (jobs_tx, jobs_rx) = mpsc::channel();
        let near = std::thread::spawn(move || {
            relay(to_helper, from_helper, &config(), &ready_tx, &jobs_rx);
        });
        f(&ready_rx, jobs_tx);
        near.join().unwrap();
        far.join().unwrap();
    }

    fn ask(jobs: &mpsc::Sender<Job>, xml: &str) -> Result<String, QbdError> {
        let (reply, answer) = oneshot::channel();
        jobs.send(Job {
            xml: xml.into(),
            reply,
        })
        .unwrap();
        answer.blocking_recv().unwrap()
    }

    #[test]
    fn a_session_through_the_helper_relays_and_ends_when_the_app_is_done() {
        with_helper(
            |from_app, to_app| {
                let hello = wire::read_frame(from_app).unwrap().unwrap();
                let got = wire::parse_hello(&hello).unwrap();
                assert_eq!(got.app_name, "LibreBooks");
                wire::write_reply(to_app, Ok(r"C:\Books\Acme.QBW")).unwrap();
                let doc = wire::read_frame(from_app).unwrap().unwrap();
                assert_eq!(doc, b"<one/>");
                wire::write_reply(to_app, Ok("<answer/>")).unwrap();
                let doc = wire::read_frame(from_app).unwrap().unwrap();
                assert_eq!(doc, b"<two/>");
                wire::write_reply(to_app, Err("refused")).unwrap();
                // The app closing its end is how a session ends.
                assert_eq!(wire::read_frame(from_app).unwrap(), None);
            },
            |ready, jobs| {
                assert_eq!(ready.recv().unwrap().unwrap(), r"C:\Books\Acme.QBW");
                assert_eq!(ask(&jobs, "<one/>").unwrap(), "<answer/>");
                let e = ask(&jobs, "<two/>").unwrap_err();
                assert!(e.to_string().contains("refused"), "{e}");
            },
        );
    }

    #[test]
    fn a_helper_that_cant_open_says_why() {
        with_helper(
            |from_app, to_app| {
                wire::read_frame(from_app).unwrap().unwrap();
                wire::write_reply(to_app, Err("QuickBooks didn't grant read-only access")).unwrap();
            },
            |ready, _jobs| {
                let e = ready.recv().unwrap().unwrap_err();
                assert!(e.message.contains("read-only"), "{e}");
                assert!(!e.unreachable, "there's no third route to try");
            },
        );
    }

    #[test]
    fn a_helper_that_dies_fails_the_request_rather_than_hanging() {
        with_helper(
            |from_app, to_app| {
                wire::read_frame(from_app).unwrap().unwrap();
                wire::write_reply(to_app, Ok("file")).unwrap();
                wire::read_frame(from_app).unwrap().unwrap();
            },
            |ready, jobs| {
                ready.recv().unwrap().unwrap();
                let e = ask(&jobs, "<one/>").unwrap_err();
                assert!(e.to_string().contains("ended"), "{e}");
            },
        );
    }
}
