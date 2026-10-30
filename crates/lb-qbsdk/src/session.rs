//! The session itself. Both routes run this code, so a session is the same
//! whichever one reached QuickBooks:
//!
//! - **Read-only, three times over.** Before the session opens,
//!   `AuthPreferences` gets `PutIsReadOnly(true)`, so QuickBooks' dialog offers
//!   read-only access and QuickBooks itself refuses any write, and
//!   `PutPersonalDataPref(pdpNotNeeded)`, since the import never reads SSNs or
//!   the like. Once it's open, `WasAuthPreferencesObeyed` must say QuickBooks
//!   honoured them (it doesn't when the application was already authorised with
//!   more access); if not, the session closes and nothing is sent. And
//!   [`Session::exchange`] refuses any document that isn't a query.
//! - COM late binding (`IDispatch`), so nothing links against the SDK at build
//!   time: `CLSIDFromProgID` finds the request processor, `OpenConnection2` and
//!   `BeginSession` open the company, `ProcessRequest` sends each document,
//!   `EndSession` and `CloseConnection` tidy up.
//! - The request processor is apartment-threaded: [`run`] holds it on the
//!   calling thread for the length of its callback.
//!
//! `unsafe` is confined to this module: every call is a documented `IDispatch`
//! invocation on an object this thread created.

#![allow(unsafe_code)]

use std::mem::ManuallyDrop;

use windows::Win32::Foundation::{DISP_E_EXCEPTION, VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Com::{
    CLSCTX_ALL, CLSIDFromProgID, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPPARAMS, EXCEPINFO,
    IDispatch,
};
use windows::Win32::System::Variant::{
    VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BOOL, VT_BSTR, VT_DISPATCH, VT_I4,
    VariantClear,
};
use windows::core::{BSTR, GUID, Interface, PCWSTR, w};

use crate::{Config, FileMode, OpenError, read_only};

const LOCALE_USER_DEFAULT: u32 = 0x0400;
/// `QBXMLRPConnectionType.localQBD`.
const LOCAL_QBD: i32 = 1;
/// `QBXMLRPPersonalDataPrefType.pdpNotNeeded` (`pdpRequired` is 1,
/// `pdpOptional` 2), from the type library in `QBXMLRP2.dll`.
const PDP_NOT_NEEDED: i32 = 3;

/// `IAuthPreferences`, from the type library in `QBXMLRP2.dll`. The object
/// `AuthPreferences` returns has the request processor's COM identity, so
/// it must be asked for this interface: the processor's own `IDispatch`
/// knows none of these names, and its DISPID 3 is `CloseConnection`.
const IID_IAUTH_PREFERENCES: GUID = GUID::from_u128(0x4277578a_9687_4ebc_8586_1ca4759b17c3);

/// `Could not start QuickBooks`: what `BeginSession` answers when it can't reach
/// one, which includes a 64-bit process and a 32-bit QuickBooks.
const QB_COULD_NOT_START: u32 = 0x8004_0408;
/// `REGDB_E_CLASSNOTREG`.
const CLASS_NOT_REGISTERED: u32 = 0x8004_0154;

/// A failed call: the text for the user, and the HRESULT to decide on.
struct ComError {
    message: String,
    code: u32,
}

impl ComError {
    fn new(message: String, e: &windows::core::Error) -> ComError {
        ComError {
            message,
            code: e.code().0 as u32,
        }
    }

    fn other(message: impl Into<String>) -> ComError {
        ComError {
            message: message.into(),
            code: 0,
        }
    }

    /// Whether this says the SDK can't reach QuickBooks from this process.
    fn unreachable(&self) -> bool {
        matches!(self.code, QB_COULD_NOT_START | CLASS_NOT_REGISTERED)
    }

    fn into_open_error(self, message: String) -> OpenError {
        OpenError {
            message,
            unreachable: self.unreachable(),
        }
    }
}

impl From<ComError> for OpenError {
    fn from(e: ComError) -> OpenError {
        let message = e.message.clone();
        e.into_open_error(message)
    }
}

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

    fn boolean(b: bool) -> VARIANT {
        VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_BOOL,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: VARIANT_0_0_0 {
                        boolVal: if b { VARIANT_TRUE } else { VARIANT_FALSE },
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

/// The boolean a method returned (`false` for non-boolean results), freeing
/// the VARIANT.
fn take_bool(mut v: VARIANT) -> bool {
    // SAFETY: as in `take_string`; the discriminant says which field is live.
    unsafe {
        let inner = &*v.Anonymous.Anonymous;
        let b = inner.vt == VT_BOOL && inner.Anonymous.boolVal != VARIANT_FALSE;
        let _ = VariantClear(&raw mut v);
        b
    }
}

/// The object a property returned, freeing the VARIANT.
fn take_dispatch(mut v: VARIANT) -> Option<IDispatch> {
    // SAFETY: as in `take_string`; the clone is an AddRef'd reference that
    // outlives the VariantClear releasing the VARIANT's own.
    unsafe {
        let inner = &*v.Anonymous.Anonymous;
        let d = if inner.vt == VT_DISPATCH {
            (*inner.Anonymous.pdispVal).clone()
        } else {
            None
        };
        let _ = VariantClear(&raw mut v);
        d
    }
}

/// `IDispatch::Invoke` of `disp`'s member `name` with `args` (in call
/// order), as `flags` says (method or property get).
fn invoke(
    disp: &IDispatch,
    name: &str,
    flags: DISPATCH_FLAGS,
    args: Vec<VARIANT>,
) -> Result<VARIANT, ComError> {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let names = [PCWSTR(wide.as_ptr())];
    let mut dispid = 0i32;
    // SAFETY: `names` holds one valid, NUL-terminated wide string and
    // `dispid` receives one id.
    unsafe {
        disp.GetIDsOfNames(
            &GUID::zeroed(),
            names.as_ptr(),
            1,
            LOCALE_USER_DEFAULT,
            &raw mut dispid,
        )
        .map_err(|e| ComError::new(format!("QuickBooks' SDK has no {name} here: {e}"), &e))?;
    }
    // IDispatch takes arguments in reverse order; `Args` frees them.
    let mut args = Args(args.into_iter().rev().collect());
    let params = DISPPARAMS {
        rgvarg: args.0.as_mut_ptr(),
        rgdispidNamedArgs: std::ptr::null_mut(),
        cArgs: args.0.len() as u32,
        cNamedArgs: 0,
    };
    let mut result = VARIANT::default();
    let mut excep = EXCEPINFO::default();
    // SAFETY: `params` points at `args.0`, which outlives the call and is
    // freed exactly once when `args` drops.
    let outcome = unsafe {
        disp.Invoke(
            dispid,
            &GUID::zeroed(),
            LOCALE_USER_DEFAULT,
            flags,
            &raw const params,
            Some(&raw mut result),
            Some(&raw mut excep),
            None,
        )
    };
    match outcome {
        Ok(()) => Ok(result),
        Err(e) => {
            let (message, code) = describe(&e, &excep);
            // SAFETY: the EXCEPINFO strings are ours to free now.
            unsafe {
                ManuallyDrop::drop(&mut excep.bstrSource);
                ManuallyDrop::drop(&mut excep.bstrDescription);
                ManuallyDrop::drop(&mut excep.bstrHelpFile);
            }
            Err(ComError {
                message: format!("{name} failed: {message}"),
                code,
            })
        }
    }
}

fn describe(e: &windows::core::Error, excep: &EXCEPINFO) -> (String, u32) {
    // `excep` was filled in by `Invoke` (or is zeroed); a zeroed BSTR is
    // null, which displays as "".
    let description = (*excep.bstrDescription).to_string();
    if e.code() == DISP_E_EXCEPTION && !description.is_empty() {
        let code = excep.scode as u32;
        (format!("{description} (0x{code:08X})"), code)
    } else {
        (e.to_string(), e.code().0 as u32)
    }
}

struct Processor {
    disp: IDispatch,
}

impl Processor {
    fn create() -> Result<Processor, ComError> {
        // SAFETY: plain COM activation on a thread that called CoInitializeEx.
        unsafe {
            let clsid = CLSIDFromProgID(w!("QBXMLRP2.RequestProcessor")).map_err(|e| {
                ComError::new(
                    format!(
                        "the QuickBooks SDK isn't installed on this machine (QBXMLRP2.RequestProcessor: {e})"
                    ),
                    &e,
                )
            })?;
            let disp: IDispatch =
                CoCreateInstance(&raw const clsid, None, CLSCTX_ALL).map_err(|e| {
                    ComError::new(
                        format!("couldn't start the QuickBooks request processor: {e}"),
                        &e,
                    )
                })?;
            Ok(Processor { disp })
        }
    }

    /// Call the request processor's method `method` with `args` (in call
    /// order).
    fn call(&self, method: &str, args: Vec<VARIANT>) -> Result<String, ComError> {
        invoke(&self.disp, method, DISPATCH_METHOD, args).map(take_string)
    }

    /// The request processor's `AuthPreferences`, as `IAuthPreferences`.
    fn auth_preferences(&self) -> Result<IDispatch, ComError> {
        let v = invoke(&self.disp, "AuthPreferences", DISPATCH_PROPERTYGET, vec![])?;
        let unknown = take_dispatch(v)
            .ok_or_else(|| ComError::other("AuthPreferences returned no object"))?;
        let mut raw = std::ptr::null_mut();
        // SAFETY: `raw` receives an owned reference to the interface asked
        // for, or stays null on failure. It's a dual interface, so its
        // vtable starts with IDispatch's.
        unsafe {
            unknown
                .query(&IID_IAUTH_PREFERENCES, &raw mut raw)
                .ok()
                .map_err(|e| {
                    ComError::new(format!("QuickBooks' SDK has no IAuthPreferences: {e}"), &e)
                })?;
            Ok(IDispatch::from_raw(raw))
        }
    }

    /// Ask QuickBooks for read-only access with no personal data. Must
    /// come after `OpenConnection2` and before `BeginSession`.
    fn request_read_only(&self) -> Result<(), ComError> {
        let prefs = self.auth_preferences()?;
        invoke(
            &prefs,
            "PutIsReadOnly",
            DISPATCH_METHOD,
            vec![Args::boolean(true)],
        )
        .map(take_string)?;
        invoke(
            &prefs,
            "PutPersonalDataPref",
            DISPATCH_METHOD,
            vec![Args::int(PDP_NOT_NEEDED)],
        )
        .map(take_string)?;
        Ok(())
    }

    /// Whether QuickBooks honoured [`Processor::request_read_only`] for
    /// the session `ticket`.
    fn read_only_obeyed(&self, ticket: &str) -> Result<bool, ComError> {
        let prefs = self.auth_preferences()?;
        invoke(
            &prefs,
            "WasAuthPreferencesObeyed",
            DISPATCH_METHOD,
            vec![Args::string(ticket)],
        )
        .map(take_bool)
    }
}

/// An open session, for as long as [`run`]'s callback lasts.
pub struct Session {
    rp: Processor,
    ticket: String,
    company_file: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("company_file", &self.company_file)
            .finish_non_exhaustive()
    }
}

/// Connect, read-only, and open the session.
fn open(config: &Config) -> Result<Session, OpenError> {
    let rp = Processor::create()?;
    rp.call(
        "OpenConnection2",
        vec![
            Args::string(&config.app_id),
            Args::string(&config.app_name),
            Args::int(LOCAL_QBD),
        ],
    )?;
    if let Err(e) = rp.request_read_only() {
        let _ = rp.call("CloseConnection", vec![]);
        let message = format!(
            "couldn't ask QuickBooks for read-only access: {}",
            e.message
        );
        return Err(e.into_open_error(message));
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
                "{}. Is QuickBooks running with the company open, and has it been allowed to talk to LibreBooks?",
                e.message
            );
            return Err(e.into_open_error(message));
        }
    };
    match rp.read_only_obeyed(&ticket) {
        Ok(true) => {}
        outcome => {
            let _ = rp.call("EndSession", vec![Args::string(&ticket)]);
            let _ = rp.call("CloseConnection", vec![]);
            let why = match outcome {
                Err(e) => e.message,
                Ok(_) => "QuickBooks didn't grant read-only access".to_owned(),
            };
            return Err(OpenError::new(format!(
                "{why}. LibreBooks only connects read-only: if \"{}\" was allowed more access before, remove it under Edit › Preferences › Integrated Applications (Company Preferences) and connect again",
                config.app_name
            )));
        }
    }
    let company_file = rp
        .call("GetCurrentCompanyFileName", vec![Args::string(&ticket)])
        .unwrap_or_default();
    Ok(Session {
        rp,
        ticket,
        company_file,
    })
}

impl Session {
    /// The company file QuickBooks has open.
    pub fn company_file(&self) -> &str {
        &self.company_file
    }

    /// Send one qbXML document, which must be queries only, and return the
    /// response.
    pub fn exchange(&self, xml: &str) -> Result<String, String> {
        read_only::check(xml)?;
        self.rp
            .call(
                "ProcessRequest",
                vec![Args::string(&self.ticket), Args::string(xml)],
            )
            .map_err(|e| e.message)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.rp.call("EndSession", vec![Args::string(&self.ticket)]);
        let _ = self.rp.call("CloseConnection", vec![]);
    }
}

/// Open a session on this thread and hand it to `f`, which sees the session or
/// why it didn't open. The session ends when `f` returns.
pub fn run<T>(config: &Config, f: impl FnOnce(Result<&Session, OpenError>) -> T) -> T {
    // SAFETY: once per call, paired with CoUninitialize below.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if hr.is_err() {
        return f(Err(OpenError::new(format!(
            "COM couldn't be initialised: {hr}"
        ))));
    }
    let out = match open(config) {
        Ok(session) => f(Ok(&session)),
        Err(e) => f(Err(e)),
    };
    // SAFETY: balances the CoInitializeEx above; every COM object is dropped.
    unsafe { CoUninitialize() };
    out
}
