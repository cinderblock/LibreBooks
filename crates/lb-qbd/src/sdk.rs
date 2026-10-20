//! The QuickBooks SDK (`QBXMLRP2.RequestProcessor2`) over COM: a direct,
//! in-process session with the QuickBooks on this same Windows machine. No
//! bridge, no Web Connector; the desktop app or a server running beside
//! QuickBooks imports straight from it.
//!
//! COM late binding (`IDispatch`), so nothing links against the SDK at build
//! time: `CLSIDFromProgID` finds the request processor QuickBooks installs,
//! `OpenConnection2` / `BeginSession` open the company, `ProcessRequest` sends
//! each qbXML document, `EndSession` / `CloseConnection` tidy up. The request
//! processor is apartment-threaded, so it lives on one dedicated thread fed by a
//! channel; [`SdkSession`] bridges that to the async [`QbxmlTransport`].
//!
//! **What QuickBooks asks the user.** The first time an application connects,
//! QuickBooks (running, with the company open) shows its *Application with no
//! certificate is requesting access* dialog naming the application
//! (`app_name`, "LibreBooks"); the user picks *Yes, always* (and leaves
//! *Allow this application to access personal data* unticked: the import never
//! reads SSNs or the like). The choice is kept under *Edit › Preferences ›
//! Integrated Applications*, where it can be revoked. The SDK has no
//! read-only request at this level; LibreBooks' guarantee is that the documents
//! it sends are queries only ([`crate::qbxml::request::check_read_only`]). A
//! headless service with no desktop can't answer the dialog: authorise once from
//! an interactive session (the desktop app, or `librebooks import qbd
//! --transport sdk` in a terminal) and the service inherits the approval for
//! the same application name.
//!
//! Only compiled on Windows with the `qbsdk` feature (on by default); the
//! settings type ([`crate::config::QbdTransport::Sdk`]) exists everywhere so a
//! Linux server can still show "not available here".

use std::sync::Mutex;
use std::sync::mpsc;
use std::thread::JoinHandle;

use tokio::sync::oneshot;

use crate::client::QbdError;
use crate::config::FileMode;
use crate::qbxml::request::Request;
use crate::source::{QbxmlSource, QbxmlTransport};

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

struct Job {
    xml: String,
    reply: oneshot::Sender<Result<String, QbdError>>,
}

/// An open SDK session on its own COM thread. Dropping it ends the session.
pub struct SdkSession {
    jobs: Mutex<Option<mpsc::Sender<Job>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    /// The company file QuickBooks reported open, once known.
    company_file: String,
}

impl std::fmt::Debug for SdkSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SdkSession")
            .field("company_file", &self.company_file)
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

impl SdkSession {
    /// Start the COM thread, connect and begin the session. Blocks until
    /// QuickBooks has accepted (or refused) the connection, which includes the
    /// time the user takes on the authorisation dialog the first time.
    pub fn open(config: &SdkConfig) -> Result<SdkSession, QbdError> {
        let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<String, String>>();
        let config = config.clone();
        let thread = std::thread::Builder::new()
            .name("quickbooks-sdk".into())
            .spawn(move || com::run(&config, &ready_tx, &jobs_rx))
            .map_err(|e| session_error(format!("couldn't start the SDK thread: {e}")))?;
        let company_file = match ready_rx.recv() {
            Ok(Ok(file)) => file,
            Ok(Err(message)) => {
                let _ = thread.join();
                return Err(session_error(message));
            }
            Err(_) => {
                let _ = thread.join();
                return Err(session_error(
                    "the SDK thread ended before the session opened",
                ));
            }
        };
        Ok(SdkSession {
            jobs: Mutex::new(Some(jobs_tx)),
            thread: Mutex::new(Some(thread)),
            company_file,
        })
    }

    /// The company file QuickBooks has open in this session.
    pub fn company_file(&self) -> &str {
        &self.company_file
    }

    /// End the session and wait for the COM thread.
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

/// The COM layer. `unsafe` is confined to this module: every call is a
/// documented `IDispatch` invocation on an object this thread created.
mod com {
    #![allow(unsafe_code)]

    use std::mem::ManuallyDrop;
    use std::sync::mpsc;

    use windows::Win32::Foundation::DISP_E_EXCEPTION;
    use windows::Win32::System::Com::{
        CLSCTX_ALL, CLSIDFromProgID, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize, DISPATCH_METHOD, DISPPARAMS, EXCEPINFO, IDispatch,
    };
    use windows::Win32::System::Variant::{
        VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BSTR, VT_I4, VariantClear,
    };
    use windows::core::{BSTR, GUID, PCWSTR, w};

    use super::{Job, SdkConfig};
    use crate::client::QbdError;
    use crate::config::FileMode;

    const LOCALE_USER_DEFAULT: u32 = 0x0400;
    /// `QBXMLRPConnectionType.localQBD`.
    const LOCAL_QBD: i32 = 1;

    fn file_mode(m: FileMode) -> i32 {
        // QBFileMode: qbFileOpenSingleUser = 0, qbFileOpenMultiUser = 1,
        // qbFileOpenDoNotCare = 2.
        match m {
            FileMode::SingleUser => 0,
            FileMode::MultiUser => 1,
            FileMode::DontCare => 2,
        }
    }

    /// Method arguments, as the `VARIANT`s COM wants, freed after the call.
    struct Args(Vec<VARIANT>);

    impl Args {
        fn string(s: &str) -> VARIANT {
            VARIANT {
                Anonymous: VARIANT_0 {
                    Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                        vt: VT_BSTR,
                        wReserved1: 0,
                        wReserved2: 0,
                        wReserved3: 0,
                        Anonymous: VARIANT_0_0_0 {
                            bstrVal: ManuallyDrop::new(BSTR::from(s)),
                        },
                    }),
                },
            }
        }

        fn int(n: i32) -> VARIANT {
            VARIANT {
                Anonymous: VARIANT_0 {
                    Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                        vt: VT_I4,
                        wReserved1: 0,
                        wReserved2: 0,
                        wReserved3: 0,
                        Anonymous: VARIANT_0_0_0 { lVal: n },
                    }),
                },
            }
        }
    }

    impl Drop for Args {
        fn drop(&mut self) {
            for v in &mut self.0 {
                // SAFETY: every element was built by `Args::string`/`Args::int`
                // and is a valid VARIANT owning its BSTR; VariantClear frees it
                // once.
                unsafe {
                    let _ = VariantClear(&raw mut *v);
                }
            }
        }
    }

    /// The string a method returned (empty for non-string results), freeing the
    /// VARIANT.
    fn take_string(mut v: VARIANT) -> String {
        // SAFETY: `v` was filled in by `IDispatch::Invoke` as `pvarresult`; the
        // discriminant says which union field is live.
        unsafe {
            let inner = &*v.Anonymous.Anonymous;
            let s = if inner.vt == VT_BSTR {
                (*inner.Anonymous.bstrVal).to_string()
            } else {
                String::new()
            };
            let _ = VariantClear(&raw mut v);
            s
        }
    }

    fn describe(e: &windows::core::Error, excep: &EXCEPINFO) -> String {
        // `excep` was filled in by `Invoke` (or is zeroed); a zeroed BSTR is
        // null, which displays as "".
        let description = (*excep.bstrDescription).to_string();
        if e.code() == DISP_E_EXCEPTION && !description.is_empty() {
            format!("{description} (0x{:08X})", excep.scode as u32)
        } else {
            e.to_string()
        }
    }

    struct Processor {
        disp: IDispatch,
    }

    impl Processor {
        fn create() -> Result<Processor, String> {
            // SAFETY: plain COM activation on a thread that called CoInitializeEx.
            unsafe {
                let clsid = CLSIDFromProgID(w!("QBXMLRP2.RequestProcessor2")).map_err(|e| {
                    format!(
                        "the QuickBooks SDK isn't installed on this machine (QBXMLRP2.RequestProcessor2: {e})"
                    )
                })?;
                let disp: IDispatch = CoCreateInstance(&raw const clsid, None, CLSCTX_ALL)
                    .map_err(|e| format!("couldn't start the QuickBooks request processor: {e}"))?;
                Ok(Processor { disp })
            }
        }

        /// `IDispatch::Invoke` of `method` with `args` (in call order).
        fn call(&self, method: &str, args: Vec<VARIANT>) -> Result<String, String> {
            // IDispatch takes arguments in reverse order; `Args` frees them.
            let mut args = Args(args.into_iter().rev().collect());
            let wide: Vec<u16> = method.encode_utf16().chain(std::iter::once(0)).collect();
            let names = [PCWSTR(wide.as_ptr())];
            let mut dispid = 0i32;
            // SAFETY: `names` holds one valid, NUL-terminated wide string and
            // `dispid` receives one id.
            unsafe {
                self.disp
                    .GetIDsOfNames(
                        &GUID::zeroed(),
                        names.as_ptr(),
                        1,
                        LOCALE_USER_DEFAULT,
                        &raw mut dispid,
                    )
                    .map_err(|e| format!("the request processor has no {method}: {e}"))?;
            }
            let params = DISPPARAMS {
                rgvarg: args.0.as_mut_ptr(),
                rgdispidNamedArgs: std::ptr::null_mut(),
                cArgs: args.0.len() as u32,
                cNamedArgs: 0,
            };
            let mut result = VARIANT::default();
            let mut excep = EXCEPINFO::default();
            // SAFETY: `params` points at `args.0`, which outlives the call and
            // is freed exactly once when `args` drops.
            let outcome = unsafe {
                self.disp.Invoke(
                    dispid,
                    &GUID::zeroed(),
                    LOCALE_USER_DEFAULT,
                    DISPATCH_METHOD,
                    &raw const params,
                    Some(&raw mut result),
                    Some(&raw mut excep),
                    None,
                )
            };
            match outcome {
                Ok(()) => Ok(take_string(result)),
                Err(e) => {
                    let message = describe(&e, &excep);
                    // SAFETY: the EXCEPINFO strings are ours to free now.
                    unsafe {
                        ManuallyDrop::drop(&mut excep.bstrSource);
                        ManuallyDrop::drop(&mut excep.bstrDescription);
                        ManuallyDrop::drop(&mut excep.bstrHelpFile);
                    }
                    Err(format!("{method} failed: {message}"))
                }
            }
        }
    }

    /// The COM thread: connect, open the session, answer jobs until the channel
    /// closes, then end the session.
    pub(super) fn run(
        config: &SdkConfig,
        ready: &mpsc::Sender<Result<String, String>>,
        jobs: &mpsc::Receiver<Job>,
    ) {
        // SAFETY: once per thread, paired with CoUninitialize below.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr.is_err() {
            let _ = ready.send(Err(format!("COM couldn't be initialised: {hr}")));
            return;
        }
        let outcome = session(config, ready, jobs);
        if let Err(e) = outcome {
            tracing::warn!(error = %e, "QuickBooks SDK session ended with an error");
        }
        // SAFETY: balances the CoInitializeEx above; every COM object is dropped.
        unsafe { CoUninitialize() };
    }

    fn session(
        config: &SdkConfig,
        ready: &mpsc::Sender<Result<String, String>>,
        jobs: &mpsc::Receiver<Job>,
    ) -> Result<(), String> {
        let rp = match Processor::create() {
            Ok(rp) => rp,
            Err(e) => {
                let _ = ready.send(Err(e.clone()));
                return Err(e);
            }
        };
        if let Err(e) = rp.call(
            "OpenConnection2",
            vec![
                Args::string(&config.app_id),
                Args::string(&config.app_name),
                Args::int(LOCAL_QBD),
            ],
        ) {
            let _ = ready.send(Err(e.clone()));
            return Err(e);
        }
        let ticket = match rp.call(
            "BeginSession",
            vec![
                Args::string(&config.company_file),
                Args::int(file_mode(config.file_mode)),
            ],
        ) {
            Ok(t) => t,
            Err(e) => {
                let _ = rp.call("CloseConnection", vec![]);
                let message = format!(
                    "{e}. Is QuickBooks running with the company open, and has it been allowed to talk to LibreBooks?"
                );
                let _ = ready.send(Err(message.clone()));
                return Err(message);
            }
        };
        let file = rp
            .call("GetCurrentCompanyFileName", vec![Args::string(&ticket)])
            .unwrap_or_default();
        let _ = ready.send(Ok(file));
        while let Ok(job) = jobs.recv() {
            let answer = rp
                .call(
                    "ProcessRequest",
                    vec![Args::string(&ticket), Args::string(&job.xml)],
                )
                .map_err(|message| QbdError::Session { message });
            let _ = job.reply.send(answer);
        }
        let _ = rp.call("EndSession", vec![Args::string(&ticket)]);
        let _ = rp.call("CloseConnection", vec![]);
        Ok(())
    }
}
