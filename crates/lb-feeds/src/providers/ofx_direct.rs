//! OFX Direct Connect: asking the bank's own OFX server for a statement, the way
//! QuickBooks Desktop and Quicken do.
//!
//! A request is an OFX document: a sign-on (user id, password, the bank's
//! `ORG`/`FID`, and the client application's `APPID`/`APPVER`) and a statement
//! request for one account and date range. It is POSTed with `Content-Type:
//! application/x-ofx`; the answer is an OFX document like a downloaded file.
//!
//! Banks still offering Direct Connect usually only accept applications they
//! know, by `APPID`/`APPVER`: Quicken for Windows is `QWIN` with a version like
//! `2700` (2700 is a recent Quicken 2019+ value; banks keep lists and drop old
//! versions), QuickBooks Desktop is `QBW` with e.g. `2700`. Both are settings, not
//! constants, because which one works varies by bank and changes over time.
//! Some banks also require a `CLIENTUID` (a stable random id per installation,
//! which the person may have to approve by email the first time). The bank's
//! URL, `ORG` and `FID` come from the bank or a directory such as ofxhome.

use futures::future::BoxFuture;
use jiff::civil::{Date, DateTime};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{FeedProvider, check_secure};
use crate::error::{FeedError, Result};
use crate::ofx;
use crate::secrets::{Secret, SecretRef};
use crate::statement::{AccountKind, Statement, StatementAccount};

const NAME: &str = "the bank's OFX server";

/// Which OFX version to speak. 1.x is SGML, 2.x XML.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum OfxVersion {
    #[default]
    V102,
    V103,
    V151,
    V160,
    V200,
    V211,
    V220,
}

impl OfxVersion {
    pub const fn number(self) -> u32 {
        match self {
            OfxVersion::V102 => 102,
            OfxVersion::V103 => 103,
            OfxVersion::V151 => 151,
            OfxVersion::V160 => 160,
            OfxVersion::V200 => 200,
            OfxVersion::V211 => 211,
            OfxVersion::V220 => 220,
        }
    }

    pub const fn is_xml(self) -> bool {
        self.number() >= 200
    }
}

fn default_app_id() -> String {
    "QWIN".into()
}

fn default_app_ver() -> String {
    "2700".into()
}

/// A Direct Connect login and account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OfxDirectSettings {
    /// The bank's OFX server, e.g. `https://ofx.examplebank.com/ofx/ofx.dll`.
    pub url: String,
    /// The bank's `ORG` and `FID` (from the bank or ofxhome).
    pub org: String,
    pub fid: String,
    /// The routing number (bank accounts; card accounts have none).
    pub bank_id: Option<String>,
    /// The account number as the bank knows it.
    pub account_number: String,
    /// Checking, savings… or a credit card (which uses the card request).
    pub account_kind: AccountKind,
    pub user_id: String,
    /// The password (or Direct Connect PIN), kept by the server.
    pub password: SecretRef,
    /// The application to claim to be: `QWIN` (Quicken) or `QBW` (QuickBooks).
    #[serde(default = "default_app_id")]
    pub app_id: String,
    /// Its version: `2700` and similar. Banks drop old versions; if sign-on fails
    /// with "unsupported application", try a newer one.
    #[serde(default = "default_app_ver")]
    pub app_ver: String,
    #[serde(default)]
    pub version: OfxVersion,
    /// A stable id for this installation, for banks that require one.
    pub client_uid: Option<String>,
    /// Some servers refuse unknown user agents; Quicken sends `InetClntApp/3.0`.
    pub user_agent: Option<String>,
}

/// One request's variable parts (the clock and ids), so a request can be built
/// deterministically for tests.
#[derive(Clone, Debug)]
pub struct RequestContext<'a> {
    pub password: &'a Secret,
    /// The client's clock (`DTCLIENT`).
    pub now: DateTime,
    /// The transaction id (`TRNUID`) and file id (`NEWFILEUID`).
    pub trnuid: &'a str,
    pub file_uid: &'a str,
}

/// Writes OFX in either dialect: SGML leaves without closing tags, or XML with.
struct Writer {
    xml: bool,
    out: String,
}

impl Writer {
    fn new(version: OfxVersion, file_uid: &str) -> Writer {
        let v = version.number();
        let out = if version.is_xml() {
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\r\n\
                 <?OFX OFXHEADER=\"200\" VERSION=\"{v}\" SECURITY=\"NONE\" OLDFILEUID=\"NONE\" NEWFILEUID=\"{file_uid}\"?>\r\n"
            )
        } else {
            format!(
                "OFXHEADER:100\r\nDATA:OFXSGML\r\nVERSION:{v}\r\nSECURITY:NONE\r\nENCODING:USASCII\r\n\
                 CHARSET:1252\r\nCOMPRESSION:NONE\r\nOLDFILEUID:NONE\r\nNEWFILEUID:{file_uid}\r\n\r\n"
            )
        };
        Writer {
            xml: version.is_xml(),
            out,
        }
    }

    fn open(&mut self, tag: &str) {
        self.out.push_str(&format!("<{tag}>\r\n"));
    }

    fn close(&mut self, tag: &str) {
        self.out.push_str(&format!("</{tag}>\r\n"));
    }

    fn elem(&mut self, tag: &str, value: &str) {
        let v = escape(value);
        if self.xml {
            self.out.push_str(&format!("<{tag}>{v}</{tag}>\r\n"));
        } else {
            self.out.push_str(&format!("<{tag}>{v}\r\n"));
        }
    }

    fn signon(&mut self, s: &OfxDirectSettings, ctx: &RequestContext<'_>) {
        self.open("SIGNONMSGSRQV1");
        self.open("SONRQ");
        self.elem("DTCLIENT", &ofx_datetime(ctx.now));
        self.elem("USERID", &s.user_id);
        self.elem("USERPASS", ctx.password.expose());
        self.elem("LANGUAGE", "ENG");
        self.open("FI");
        self.elem("ORG", &s.org);
        self.elem("FID", &s.fid);
        self.close("FI");
        self.elem("APPID", &s.app_id);
        self.elem("APPVER", &s.app_ver);
        // CLIENTUID arrived in OFX 1.0.3.
        if let Some(uid) = s.client_uid.as_deref().filter(|u| !u.is_empty())
            && s.version != OfxVersion::V102
        {
            self.elem("CLIENTUID", uid);
        }
        self.close("SONRQ");
        self.close("SIGNONMSGSRQV1");
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace(['\r', '\n'], " ")
}

fn ofx_datetime(t: DateTime) -> String {
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}.000",
        t.year(),
        t.month(),
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    )
}

fn ofx_date(d: Date) -> String {
    format!("{:04}{:02}{:02}", d.year(), d.month(), d.day())
}

/// The statement request for an account, from `start` (to `end`, or now).
pub fn statement_request(
    s: &OfxDirectSettings,
    ctx: &RequestContext<'_>,
    start: Date,
    end: Option<Date>,
) -> String {
    let mut w = Writer::new(s.version, ctx.file_uid);
    w.open("OFX");
    w.signon(s, ctx);
    let card = s.account_kind == AccountKind::CreditCard;
    let (msgs, trnrq, stmtrq) = if card {
        ("CREDITCARDMSGSRQV1", "CCSTMTTRNRQ", "CCSTMTRQ")
    } else {
        ("BANKMSGSRQV1", "STMTTRNRQ", "STMTRQ")
    };
    w.open(msgs);
    w.open(trnrq);
    w.elem("TRNUID", ctx.trnuid);
    w.open(stmtrq);
    if card {
        w.open("CCACCTFROM");
        w.elem("ACCTID", &s.account_number);
        w.close("CCACCTFROM");
    } else {
        w.open("BANKACCTFROM");
        w.elem("BANKID", s.bank_id.as_deref().unwrap_or_default());
        w.elem("ACCTID", &s.account_number);
        w.elem("ACCTTYPE", s.account_kind.ofx_name());
        w.close("BANKACCTFROM");
    }
    w.open("INCTRAN");
    w.elem("DTSTART", &ofx_date(start));
    if let Some(e) = end {
        w.elem("DTEND", &ofx_date(e));
    }
    w.elem("INCLUDE", "Y");
    w.close("INCTRAN");
    w.close(stmtrq);
    w.close(trnrq);
    w.close(msgs);
    w.close("OFX");
    w.out
}

/// The account-list request (`ACCTINFORQ`): which accounts this login can
/// download, for setting a connection up.
pub fn account_list_request(s: &OfxDirectSettings, ctx: &RequestContext<'_>) -> String {
    let mut w = Writer::new(s.version, ctx.file_uid);
    w.open("OFX");
    w.signon(s, ctx);
    w.open("SIGNUPMSGSRQV1");
    w.open("ACCTINFOTRNRQ");
    w.elem("TRNUID", ctx.trnuid);
    w.open("ACCTINFORQ");
    w.elem("DTACCTUP", "19700101000000.000");
    w.close("ACCTINFORQ");
    w.close("ACCTINFOTRNRQ");
    w.close("SIGNUPMSGSRQV1");
    w.close("OFX");
    w.out
}

/// The accounts in an account-list response. Their numbers are kept masked (and
/// fingerprinted); the full number is the person's to type into the settings.
pub fn parse_account_list(body: &[u8]) -> Result<Vec<StatementAccount>> {
    let doc = ofx::parse(body)?;
    if let Some(s) = &doc.signon
        && !s.is_ok()
    {
        return Err(signon_error(s));
    }
    let root = doc.root.unwrap_or_else(|| ofx::parse_tree(""));
    let mut out = Vec::new();
    for info in root.find_all("ACCTINFO") {
        let mut a = StatementAccount {
            name: info.text("DESC").map(str::to_owned),
            ..StatementAccount::default()
        };
        if let Some(from) = info.path(&["BANKACCTINFO", "BANKACCTFROM"]) {
            let bank_id = from.text("BANKID");
            a.bank_id = bank_id.map(str::to_owned);
            a.account_type = from
                .text("ACCTTYPE")
                .map_or(AccountKind::Checking, AccountKind::from_ofx);
            if let Some(n) = from.text("ACCTID") {
                a.set_number(bank_id, n);
            }
        } else if let Some(from) = info.path(&["CCACCTINFO", "CCACCTFROM"]) {
            a.account_type = AccountKind::CreditCard;
            if let Some(n) = from.text("ACCTID") {
                a.set_number(None, n);
            }
        } else {
            continue;
        }
        out.push(a);
    }
    Ok(out)
}

fn signon_error(s: &ofx::OfxStatus) -> FeedError {
    let message = s.describe();
    if matches!(s.code, 15500 | 15502 | 15000) {
        FeedError::Unauthorized {
            provider: NAME,
            message,
        }
    } else {
        FeedError::Provider {
            provider: NAME,
            message,
        }
    }
}

/// A Direct Connect account with its password.
pub struct OfxDirect {
    client: reqwest::Client,
    settings: OfxDirectSettings,
    url: url::Url,
    password: Secret,
}

impl std::fmt::Debug for OfxDirect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfxDirect")
            .field("url", &self.url.as_str())
            .field("org", &self.settings.org)
            .finish_non_exhaustive()
    }
}

impl OfxDirect {
    pub fn new(
        client: reqwest::Client,
        settings: OfxDirectSettings,
        password: Secret,
    ) -> Result<OfxDirect> {
        let url = url::Url::parse(&settings.url)
            .map_err(|_| FeedError::Invalid("the bank's OFX URL isn't valid".into()))?;
        check_secure(&url, NAME)?;
        Ok(OfxDirect {
            client,
            settings,
            url,
            password,
        })
    }

    fn context<'a>(&'a self, ids: &'a (String, String)) -> RequestContext<'a> {
        RequestContext {
            password: &self.password,
            now: jiff::Zoned::now().datetime(),
            trnuid: &ids.0,
            file_uid: &ids.1,
        }
    }

    /// POST an OFX request and return the answer's bytes.
    pub async fn post(&self, body: String) -> Result<Vec<u8>> {
        let mut req = self
            .client
            .post(self.url.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/x-ofx")
            .header(
                reqwest::header::ACCEPT,
                "application/ofx, application/x-ofx, */*",
            )
            .header(reqwest::header::CONNECTION, "close");
        if let Some(ua) = self.settings.user_agent.as_deref() {
            req = req.header(reqwest::header::USER_AGENT, ua);
        }
        let resp = req
            .body(body)
            .send()
            .await
            .map_err(|e| FeedError::network(NAME, e))?;
        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| FeedError::network(NAME, e))?;
        // Some servers answer errors with a 400 and a valid OFX body: prefer its
        // status block.
        if !status.is_success() && !ofx::looks_like_ofx(&bytes) {
            return Err(FeedError::Provider {
                provider: NAME,
                message: format!("the server answered {status}"),
            });
        }
        Ok(bytes.to_vec())
    }

    /// The accounts this login can download.
    pub async fn accounts(&self) -> Result<Vec<StatementAccount>> {
        let ids = new_ids();
        let body = account_list_request(&self.settings, &self.context(&ids));
        parse_account_list(&self.post(body).await?)
    }
}

fn new_ids() -> (String, String) {
    (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().simple().to_string(),
    )
}

impl FeedProvider for OfxDirect {
    fn name(&self) -> &'static str {
        "OFX Direct Connect"
    }

    fn fetch(&self, since: Option<Date>) -> BoxFuture<'_, Result<Vec<Statement>>> {
        Box::pin(async move {
            let today = jiff::Zoned::now().date();
            let start = since.unwrap_or_else(|| {
                today
                    .checked_sub(jiff::Span::new().days(90))
                    .unwrap_or(today)
            });
            let ids = new_ids();
            let body = statement_request(&self.settings, &self.context(&ids), start, None);
            let answer = self.post(body).await?;
            let doc = ofx::parse(&answer)?;
            if let Some(s) = &doc.signon
                && !s.is_ok()
            {
                return Err(signon_error(s));
            }
            if doc.statements.is_empty() {
                return Err(match doc.statement_errors.first() {
                    Some(e) => FeedError::Provider {
                        provider: NAME,
                        message: e.describe(),
                    },
                    None => FeedError::Provider {
                        provider: NAME,
                        message: "the answer had no statement in it".into(),
                    },
                });
            }
            Ok(doc.statements)
        })
    }
}
