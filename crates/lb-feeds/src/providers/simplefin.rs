//! SimpleFIN Bridge (<https://www.simplefin.org/protocol.html>).
//!
//! The cheapest way for one person to get bank downloads: they pay the bridge
//! (about $15 a year), link their banks there, and hand LibreBooks a **setup
//! token**. The token is a base64-encoded claim URL; POSTing to it (once) returns
//! an **access URL** of the form `https://user:pass@bridge.example/simplefin`.
//! That URL is the credential: whoever holds it can read the accounts, so it is
//! kept as a [`Secret`] and never logged, and requests send its user and password
//! as basic auth to the bare URL rather than leaving them in the URL.
//!
//! `GET {access}/accounts?start-date=<unix seconds>` returns every linked account
//! with its balance and transactions. Amounts are strings, positive for money
//! in, like ours. Transaction ids are stable and become the fitid. `posted` is a
//! Unix time; its UTC date is used (bridges stamp posted transactions at a fixed
//! time of the bank's date).

use futures::future::BoxFuture;
use jiff::civil::Date;
use serde::Deserialize;

use super::{FeedProvider, check_secure};
use crate::error::{FeedError, Result};
use crate::secrets::Secret;
use crate::statement::{
    AccountKind, Statement, StatementAccount, StatementTxn, TrnType, fingerprint,
};
use crate::text;

const NAME: &str = "SimpleFIN";

/// Exchange a setup token for an access URL. A token works once: claiming it
/// again fails, which is also how a person can tell if someone else used it.
pub async fn claim(client: &reqwest::Client, setup_token: &str) -> Result<Secret> {
    let claim_url = decode_setup_token(setup_token)?;
    let url = url::Url::parse(&claim_url)
        .map_err(|_| FeedError::Invalid("the setup token doesn't hold a valid URL".into()))?;
    check_secure(&url, NAME)?;
    let resp = client
        .post(url)
        .header(reqwest::header::CONTENT_LENGTH, "0")
        .send()
        .await
        .map_err(|e| FeedError::network(NAME, e))?;
    let status = resp.status();
    if status == reqwest::StatusCode::FORBIDDEN {
        return Err(FeedError::Unauthorized {
            provider: NAME,
            message:
                "this setup token was already used or has expired; make a new one at the bridge"
                    .into(),
        });
    }
    if !status.is_success() {
        return Err(FeedError::Provider {
            provider: NAME,
            message: format!("the bridge answered {status} to the setup token"),
        });
    }
    let body = resp.text().await.map_err(|e| FeedError::network(NAME, e))?;
    let access = body.trim().to_owned();
    let parsed = url::Url::parse(&access).map_err(|_| FeedError::Provider {
        provider: NAME,
        message: "the bridge's answer isn't an access URL".into(),
    })?;
    check_secure(&parsed, NAME)?;
    if parsed.username().is_empty() {
        return Err(FeedError::Provider {
            provider: NAME,
            message: "the bridge's access URL has no credentials in it".into(),
        });
    }
    Ok(Secret::new(access))
}

/// A setup token is the claim URL in base64 (standard or URL-safe alphabet,
/// padded or not, with stray whitespace from copying).
pub fn decode_setup_token(token: &str) -> Result<String> {
    use base64::Engine;
    let compact: String = token.chars().filter(|c| !c.is_whitespace()).collect();
    let engines = [
        &base64::engine::general_purpose::STANDARD,
        &base64::engine::general_purpose::STANDARD_NO_PAD,
        &base64::engine::general_purpose::URL_SAFE,
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ];
    engines
        .iter()
        .find_map(|e| e.decode(&compact).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| s.starts_with("http"))
        .ok_or_else(|| FeedError::Invalid("that isn't a SimpleFIN setup token".into()))
}

/// Split an access URL into the bare URL and its basic-auth credentials.
fn split_access(access: &Secret) -> Result<(url::Url, String, String)> {
    let mut url = url::Url::parse(access.expose()).map_err(|_| FeedError::Unauthorized {
        provider: NAME,
        message: "the saved access URL is damaged; reconnect SimpleFIN".into(),
    })?;
    check_secure(&url, NAME)?;
    let user = percent_decode(url.username());
    let pass = percent_decode(url.password().unwrap_or_default());
    // Neither can fail on an http(s) URL.
    let _ = url.set_username("");
    let _ = url.set_password(None);
    Ok((url, user, pass))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A SimpleFIN connection with its access URL.
pub struct SimpleFin {
    client: reqwest::Client,
    base: url::Url,
    user: String,
    pass: Secret,
}

impl std::fmt::Debug for SimpleFin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleFin")
            .field("host", &self.base.host_str())
            .finish_non_exhaustive()
    }
}

impl SimpleFin {
    pub fn new(client: reqwest::Client, access: &Secret) -> Result<SimpleFin> {
        let (base, user, pass) = split_access(access)?;
        Ok(SimpleFin {
            client,
            base,
            user,
            pass: Secret::new(pass),
        })
    }

    fn accounts_url(&self) -> String {
        format!("{}/accounts", self.base.as_str().trim_end_matches('/'))
    }

    /// Fetch and read `/accounts`.
    pub async fn accounts(&self, since: Option<Date>) -> Result<Download> {
        let mut req = self
            .client
            .get(self.accounts_url())
            .basic_auth(&self.user, Some(self.pass.expose()));
        if let Some(d) = since {
            let ts = d
                .to_zoned(jiff::tz::TimeZone::UTC)
                .map_err(|e| FeedError::Invalid(e.to_string()))?
                .timestamp()
                .as_second();
            req = req.query(&[("start-date", ts.to_string())]);
        }
        let resp = req.send().await.map_err(|e| FeedError::network(NAME, e))?;
        let status = resp.status();
        if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(FeedError::Unauthorized {
                provider: NAME,
                message: "the bridge refused the access URL (it may have been revoked); reconnect SimpleFIN"
                    .into(),
            });
        }
        if status == reqwest::StatusCode::PAYMENT_REQUIRED {
            return Err(FeedError::Provider {
                provider: NAME,
                message: "the bridge subscription needs paying".into(),
            });
        }
        if !status.is_success() {
            return Err(FeedError::Provider {
                provider: NAME,
                message: format!("the bridge answered {status}"),
            });
        }
        let body = resp
            .bytes()
            .await
            .map_err(|e| FeedError::network(NAME, e))?;
        parse_accounts(&body)
    }
}

impl FeedProvider for SimpleFin {
    fn name(&self) -> &'static str {
        NAME
    }

    fn fetch(&self, since: Option<Date>) -> BoxFuture<'_, Result<Vec<Statement>>> {
        Box::pin(async move {
            let mut d = self.accounts(since).await?;
            if d.statements.is_empty() && !d.errors.is_empty() {
                return Err(FeedError::Provider {
                    provider: NAME,
                    message: d.errors.join("; "),
                });
            }
            // The bridge's messages ("reauthenticate with Example Bank") travel
            // with every statement, so whichever account a connection reads, the
            // person sees them.
            for s in &mut d.statements {
                s.warnings
                    .extend(d.errors.iter().map(|e| format!("SimpleFIN: {e}")));
            }
            Ok(d.statements)
        })
    }
}

/// What `/accounts` returned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Download {
    pub statements: Vec<Statement>,
    /// The bridge's messages ("You must reauthenticate with Example Bank").
    pub errors: Vec<String>,
}

#[derive(Deserialize)]
struct AccountSet {
    #[serde(default)]
    errors: Vec<String>,
    #[serde(default)]
    accounts: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct Org {
    name: Option<String>,
    domain: Option<String>,
}

#[derive(Deserialize)]
struct Account {
    org: Option<Org>,
    id: String,
    name: Option<String>,
    currency: Option<String>,
    balance: Option<String>,
    #[serde(rename = "available-balance")]
    available_balance: Option<String>,
    #[serde(rename = "balance-date")]
    balance_date: Option<i64>,
    #[serde(default)]
    transactions: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct Txn {
    id: String,
    #[serde(default)]
    posted: i64,
    amount: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    payee: Option<String>,
    #[serde(default)]
    memo: Option<String>,
    #[serde(default)]
    pending: bool,
}

fn utc_date(secs: i64) -> Option<Date> {
    jiff::Timestamp::from_second(secs)
        .ok()
        .map(|t| t.to_zoned(jiff::tz::TimeZone::UTC).date())
}

/// Read an `/accounts` response.
pub fn parse_accounts(body: &[u8]) -> Result<Download> {
    let set: AccountSet = serde_json::from_slice(body)
        .map_err(|e| FeedError::parse("SimpleFIN's answer", e.to_string()))?;
    let mut out = Download {
        errors: set.errors,
        ..Download::default()
    };
    for raw in set.accounts {
        let acct: Account = serde_json::from_value(raw)
            .map_err(|e| FeedError::parse("SimpleFIN's answer", e.to_string()))?;
        let money = |m: Option<&str>| m.and_then(|b| text::parse_amount(b).ok());
        let mut stmt = Statement {
            account: StatementAccount {
                account_id_masked: text::mask_account(&acct.id),
                account_key: fingerprint(&["simplefin", &acct.id]),
                // SimpleFIN doesn't say; the connection's LibreBooks account does.
                account_type: AccountKind::Other,
                // Custom currencies are URLs; only keep real ISO codes.
                currency: acct.currency.filter(|c| c.len() == 3),
                institution: acct
                    .org
                    .as_ref()
                    .and_then(|o| o.name.clone().or_else(|| o.domain.clone())),
                name: acct.name.clone(),
                remote_id: Some(acct.id.clone()),
                ..StatementAccount::default()
            },
            ledger_balance: money(acct.balance.as_deref()),
            available_balance: money(acct.available_balance.as_deref()),
            balance_date: acct.balance_date.and_then(utc_date),
            ..Statement::default()
        };
        for raw in acct.transactions {
            let txn: Txn = match serde_json::from_value(raw.clone()) {
                Ok(txn) => txn,
                Err(e) => {
                    stmt.warnings
                        .push(format!("a transaction was skipped: {e}"));
                    continue;
                }
            };
            if txn.pending || txn.posted == 0 {
                continue; // pending: its id and amount may still change
            }
            let Some(posted) = utc_date(txn.posted) else {
                stmt.warnings.push(format!(
                    "transaction {} has a bad date and was skipped",
                    txn.id
                ));
                continue;
            };
            let amount = match text::parse_amount(&txn.amount) {
                Ok(m) => m,
                Err(e) => {
                    stmt.warnings
                        .push(format!("transaction {} was skipped: {e}", txn.id));
                    continue;
                }
            };
            let mut line = StatementTxn::new(posted, amount);
            line.fitid = txn.id;
            line.trntype = TrnType::by_sign(amount);
            let description = txn.description.filter(|d| !d.trim().is_empty());
            (line.payee, line.memo) =
                match (txn.payee.filter(|p| !p.trim().is_empty()), description) {
                    (Some(payee), desc) => (
                        Some(payee.clone()),
                        txn.memo.or(desc.filter(|d| *d != payee)),
                    ),
                    (None, desc) => (desc, txn.memo),
                };
            line.raw = raw;
            stmt.transactions.push(line);
        }
        stmt.finish();
        out.statements.push(stmt);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_tokens_decode_in_any_base64() {
        use base64::Engine;
        let url = "https://bridge.example/simplefin/claim/DEMO";
        for e in [
            &base64::engine::general_purpose::STANDARD,
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        ] {
            let tok = e.encode(url);
            assert_eq!(decode_setup_token(&format!(" {tok}\n")).unwrap(), url);
        }
        assert!(decode_setup_token("not a token!").is_err());
        assert!(decode_setup_token("aGVsbG8=").is_err()); // "hello"
    }

    #[test]
    fn access_urls_split_into_url_and_credentials() {
        let s = Secret::new("https://us%40er:p%3Ass@bridge.example/simplefin");
        let (url, user, pass) = split_access(&s).unwrap();
        assert_eq!(url.as_str(), "https://bridge.example/simplefin");
        assert_eq!(user, "us@er");
        assert_eq!(pass, "p:ss");
        assert!(split_access(&Secret::new("http://u:p@bridge.example/x")).is_err());
        assert!(split_access(&Secret::new("http://u:p@127.0.0.1:9/x")).is_ok());
    }
}
