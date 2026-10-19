//! Online sources: download statements without the person fetching a file.
//!
//! Every provider implements [`FeedProvider`]: given a start date, fetch
//! statements. The trait returns a boxed future rather than using `async fn`
//! so the server can keep a `Box<dyn FeedProvider>` per connection (native async
//! trait methods aren't object-safe). [`sync`] runs a connection end to end:
//! fetch, ingest, record how it went.
//!
//! Implemented: [`simplefin`] (SimpleFIN Bridge) and [`ofx_direct`] (the bank's
//! own OFX server, as QuickBooks' Direct Connect uses). Plaid and Teller are
//! described in `plans/feeds.md`; their settings exist so connections can be
//! saved, and [`for_connection`] reports them as not supported yet.

pub mod ofx_direct;
pub mod simplefin;

use futures::future::BoxFuture;
use jiff::civil::Date;
use lb_store::{Actor, Books};

use crate::connections::{self, FeedConnection, ProviderSettings};
use crate::error::{FeedError, Result};
use crate::ids::FeedConnectionId;
use crate::ingest::{self, IngestReport};
use crate::secrets::{self, Secrets};
use crate::statement::Statement;

/// A source that can download statements by itself.
pub trait FeedProvider: Send + Sync + std::fmt::Debug {
    /// A short name for messages ("SimpleFIN").
    fn name(&self) -> &'static str;

    /// Statements with transactions posted on or after `since` (as far back as
    /// the source allows when `None`).
    fn fetch(&self, since: Option<Date>) -> BoxFuture<'_, Result<Vec<Statement>>>;
}

/// An HTTP client for providers: rustls, a timeout, and an honest user agent.
pub fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("LibreBooks/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60))
        .connect_timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| FeedError::network("the HTTP client", e))
}

/// Credentials only travel over HTTPS. Plain HTTP is allowed to the local
/// machine (tests, and a bridge on the same box).
pub(crate) fn check_secure(url: &url::Url, provider: &'static str) -> Result<()> {
    let loopback = match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    match url.scheme() {
        "https" => Ok(()),
        "http" if loopback => Ok(()),
        _ => Err(FeedError::Invalid(format!(
            "{provider} must be reached over https"
        ))),
    }
}

/// The provider for a connection, with its secrets resolved.
pub fn for_connection(
    conn: &FeedConnection,
    secrets: &dyn Secrets,
    client: reqwest::Client,
) -> Result<Box<dyn FeedProvider>> {
    match &conn.settings {
        ProviderSettings::File { .. } => Err(FeedError::Invalid(
            "this connection imports files; there is nothing to download".into(),
        )),
        ProviderSettings::SimpleFin { access, .. } => {
            let access = secrets::resolve(secrets, access, "SimpleFIN")?;
            Ok(Box::new(simplefin::SimpleFin::new(client, &access)?))
        }
        ProviderSettings::OfxDirect(settings) => {
            let password = secrets::resolve(secrets, &settings.password, "the bank")?;
            Ok(Box::new(ofx_direct::OfxDirect::new(
                client,
                settings.clone(),
                password,
            )?))
        }
        ProviderSettings::Plaid { .. } => Err(FeedError::Unsupported("Plaid".into())),
        ProviderSettings::Teller { .. } => Err(FeedError::Unsupported("Teller".into())),
    }
}

/// How far back to ask for: a week before the last successful download (banks
/// post late, and dedupe makes overlap free), else 90 days, never before the
/// connection's start date.
pub fn since_for(conn: &FeedConnection, today: Date) -> Date {
    let week = jiff::Span::new().days(7);
    let quarter = jiff::Span::new().days(90);
    let from_last = conn
        .last_sync_at
        .map(|t| t.to_zoned(jiff::tz::TimeZone::UTC).date())
        .and_then(|d| d.checked_sub(week).ok());
    let start = from_last.unwrap_or_else(|| today.checked_sub(quarter).unwrap_or(today));
    match conn.not_before {
        Some(nb) if nb > start => nb,
        _ => start,
    }
}

/// Download and ingest one connection, recording the outcome on it (the time
/// on success, the message on failure).
pub async fn sync(
    books: &Books,
    actor: &Actor,
    id: FeedConnectionId,
    provider: &dyn FeedProvider,
    today: Date,
) -> Result<IngestReport> {
    let conn = crate::read(books, |c| connections::get(c, id))?;
    let since = since_for(&conn, today);
    match provider.fetch(Some(since)).await {
        Err(e) => {
            let message = e.to_string();
            crate::write(books, actor, |w| {
                connections::record_sync(w, id, jiff::Timestamp::now(), Some(&message))
            })?;
            Err(e)
        }
        Ok(statements) => crate::write(books, actor, |w| {
            let report = ingest::ingest_for_connection(w, &conn, &statements)?;
            connections::record_sync(w, id, jiff::Timestamp::now(), None)?;
            Ok(report)
        }),
    }
}
