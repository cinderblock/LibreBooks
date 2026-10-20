//! Online providers against local mock servers (never the real internet):
//! SimpleFIN's claim and download, OFX Direct Connect's requests and answers,
//! and a whole sync recorded on its connection.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use jiff::civil::{date, datetime};
use lb_core::{AccountType, CompanyInfo, Money};
use lb_feeds::connections::{self, FeedConnection, ProviderSettings};
use lb_feeds::providers::ofx_direct::{
    self, OfxDirect, OfxDirectSettings, OfxVersion, RequestContext,
};
use lb_feeds::providers::simplefin::{self, SimpleFin};
use lb_feeds::providers::{self, FeedProvider};
use lb_feeds::secrets::MemorySecrets;
use lb_feeds::{AccountKind, FeedError, Secret, SecretRef};
use lb_store::{Actor, Books, accounts};
use pretty_assertions::assert_eq;

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!("{}/testdata/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

/// What the mock saw, for assertions.
#[derive(Clone, Default)]
struct Seen {
    claims: Arc<Mutex<u32>>,
    start_dates: Arc<Mutex<Vec<String>>>,
    bodies: Arc<Mutex<Vec<String>>>,
    port: Arc<Mutex<u16>>,
}

async fn serve(app: Router) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

/// A SimpleFIN bridge: one setup token (`/claim/DEMO`, usable once) and
/// `/simplefin/accounts` behind basic auth `demo:s3cret`.
async fn simplefin_bridge() -> (u16, Seen) {
    async fn claim(State(seen): State<Seen>) -> impl IntoResponse {
        let mut n = seen.claims.lock().unwrap();
        *n += 1;
        if *n > 1 {
            return (StatusCode::FORBIDDEN, String::new());
        }
        let port = *seen.port.lock().unwrap();
        (
            StatusCode::OK,
            format!("http://demo:s3cret@127.0.0.1:{port}/simplefin"),
        )
    }
    async fn accounts(
        State(seen): State<Seen>,
        headers: HeaderMap,
        Query(q): Query<std::collections::HashMap<String, String>>,
    ) -> impl IntoResponse {
        use base64::Engine;
        let want = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("demo:s3cret")
        );
        if headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            != Some(&want)
        {
            return (StatusCode::FORBIDDEN, Vec::new());
        }
        if let Some(s) = q.get("start-date") {
            seen.start_dates.lock().unwrap().push(s.clone());
        }
        (StatusCode::OK, fixture("simplefin/accounts.json"))
    }
    let seen = Seen::default();
    let app = Router::new()
        .route("/claim/DEMO", post(claim))
        .route("/simplefin/accounts", get(accounts))
        .with_state(seen.clone());
    let port = serve(app).await;
    *seen.port.lock().unwrap() = port;
    (port, seen)
}

#[tokio::test]
async fn simplefin_claims_once_and_downloads() {
    use base64::Engine;
    let (port, seen) = simplefin_bridge().await;
    let token = base64::engine::general_purpose::STANDARD
        .encode(format!("http://127.0.0.1:{port}/claim/DEMO"));
    let access = simplefin::claim(&client(), &token).await.unwrap();
    assert_eq!(
        access.expose(),
        format!("http://demo:s3cret@127.0.0.1:{port}/simplefin")
    );
    // A token works once.
    let again = simplefin::claim(&client(), &token).await.unwrap_err();
    assert!(matches!(again, FeedError::Unauthorized { .. }), "{again:?}");
    assert!(again.to_string().contains("already used"));

    let sf = SimpleFin::new(client(), &access).unwrap();
    assert!(!format!("{sf:?}").contains("s3cret"));
    let statements = sf.fetch(Some(date(2026, 9, 1))).await.unwrap();
    assert_eq!(
        seen.start_dates.lock().unwrap().as_slice(),
        ["1788220800".to_owned()],
        "start-date is 2026-09-01T00:00:00Z"
    );
    assert_eq!(statements.len(), 2);
    let chk = &statements[0];
    assert_eq!(chk.account.remote_id.as_deref(), Some("ACT-1111-2222"));
    assert_eq!(chk.account.account_id_masked, "****2222");
    assert_eq!(chk.account.institution.as_deref(), Some("Example Bank"));
    assert_eq!(chk.account.name.as_deref(), Some("Everyday Checking"));
    assert_eq!(chk.account.account_type, AccountKind::Other);
    assert_eq!(chk.ledger_balance, Some(Money(519_851)));
    assert_eq!(chk.available_balance, Some(Money(509_851)));
    assert_eq!(chk.balance_date, Some(date(2026, 10, 14)));
    let t = &chk.transactions;
    assert_eq!(t.len(), 3, "the pending one is left for later");
    assert_eq!(
        (t[0].fitid.as_str(), t[0].posted, t[0].amount),
        ("TRN-001", date(2026, 10, 1), Money(250_000))
    );
    assert_eq!(t[1].payee.as_deref(), Some("Landlord LLC"));
    assert_eq!(t[1].memo.as_deref(), Some("October rent"));
    assert_eq!(t[2].raw["extra"]["category"], "Utilities");
    assert!(
        chk.warnings
            .contains(&"SimpleFIN: Connection to Example Credit Union may need attention".into())
    );
    let card = &statements[1];
    assert_eq!(card.transactions.len(), 1);
    assert_eq!(card.transactions[0].posted, date(2026, 9, 26));
    assert!(card.warnings.iter().any(|w| w.contains("CC-2 was skipped")));

    // Wrong credentials: refused, and the message doesn't repeat them.
    let bad = SimpleFin::new(
        client(),
        &Secret::new(format!("http://demo:nope@127.0.0.1:{port}/simplefin")),
    )
    .unwrap();
    let err = bad.fetch(None).await.unwrap_err();
    assert!(matches!(err, FeedError::Unauthorized { .. }));
    assert!(!err.to_string().contains("nope"));
}

#[tokio::test]
async fn simplefin_refuses_plain_http_off_this_machine() {
    let err = SimpleFin::new(
        client(),
        &Secret::new("http://u:p@bridge.example.test/simplefin"),
    )
    .unwrap_err();
    assert_eq!(err.to_string(), "SimpleFIN must be reached over https");
}

fn me() -> Actor {
    Actor {
        user_id: Some("tester".into()),
        may_override_closing_date: false,
    }
}

#[tokio::test]
async fn a_sync_ingests_and_records_how_it_went() {
    let (port, seen) = simplefin_bridge().await;
    let books = Books::in_memory(&CompanyInfo {
        name: "Sync Co".into(),
        ..CompanyInfo::default()
    })
    .unwrap();
    let checking = books
        .write(&me(), |w| {
            accounts::save(w, accounts::new_account("Checking", AccountType::Bank))
        })
        .unwrap()
        .id;
    let mut secrets = MemorySecrets::default();
    let access = secrets.insert(
        "simplefin-1",
        Secret::new(format!("http://demo:s3cret@127.0.0.1:{port}/simplefin")),
    );
    let conn = lb_feeds::write(&books, &me(), |w| {
        connections::save(
            w,
            FeedConnection::new(
                "Everyday Checking",
                Some(checking),
                ProviderSettings::SimpleFin {
                    access,
                    remote_account_id: "ACT-1111-2222".into(),
                },
            ),
        )
    })
    .unwrap();
    // The stored settings hold the secret's name, never the secret.
    let stored: String = books
        .read(|c| {
            Ok(c.query_row(
                "SELECT doc FROM feed_connection WHERE id = ?1",
                [conn.id.to_string()],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert!(stored.contains("simplefin-1") && !stored.contains("s3cret"));

    let provider = providers::for_connection(&conn, &secrets, client()).unwrap();
    let today = date(2026, 10, 19);
    let report = providers::sync(&books, &me(), conn.id, provider.as_ref(), today)
        .await
        .unwrap();
    assert_eq!(
        report.import.imported.len(),
        3,
        "only this connection's account"
    );
    assert_eq!(report.waiting, 3);
    assert!(
        report
            .import
            .warnings
            .iter()
            .any(|w| w.starts_with("SimpleFIN:"))
    );
    // No previous sync: 90 days back.
    let ninety_back = date(2026, 7, 21)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
        .as_second()
        .to_string();
    assert_eq!(seen.start_dates.lock().unwrap()[0], ninety_back);

    let after = lb_feeds::read(&books, |c| connections::get(c, conn.id)).unwrap();
    assert!(after.last_sync_at.is_some());
    assert_eq!(after.last_error, None);
    // The connection now remembers the account's fingerprint.
    assert!(after.account_key.is_some());

    // Again: nothing new, and it asks from a week before the last sync.
    let again = providers::sync(&books, &me(), conn.id, provider.as_ref(), today)
        .await
        .unwrap();
    assert_eq!(
        (again.import.imported.len(), again.import.duplicates),
        (0, 3)
    );
    assert_eq!(
        providers::since_for(&after, today),
        after
            .last_sync_at
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .date()
            .checked_sub(jiff::Span::new().days(7))
            .unwrap()
    );

    // A refused download is recorded on the connection, without the secret.
    let mut broken = MemorySecrets::default();
    broken.insert(
        "simplefin-1",
        Secret::new(format!("http://demo:wrong@127.0.0.1:{port}/simplefin")),
    );
    let after = lb_feeds::read(&books, |c| connections::get(c, conn.id)).unwrap();
    let provider = providers::for_connection(&after, &broken, client()).unwrap();
    let err = providers::sync(&books, &me(), conn.id, provider.as_ref(), today)
        .await
        .unwrap_err();
    assert!(matches!(err, FeedError::Unauthorized { .. }));
    let failed = lb_feeds::read(&books, |c| connections::get(c, conn.id)).unwrap();
    let msg = failed.last_error.unwrap();
    assert!(
        msg.contains("reconnect SimpleFIN") && !msg.contains("wrong"),
        "{msg}"
    );
    assert_eq!(
        failed.last_sync_at, after.last_sync_at,
        "kept from the last success"
    );

    // A missing secret, and providers not built yet, say so.
    let none = MemorySecrets::default();
    let err = providers::for_connection(&after, &none, client()).unwrap_err();
    assert!(err.to_string().contains("credentials are missing"));
    let plaid = FeedConnection::new(
        "Plaid",
        Some(checking),
        ProviderSettings::Plaid {
            access_token: SecretRef::new("x"),
            remote_account_id: "y".into(),
        },
    );
    let err = providers::for_connection(&plaid, &none, client()).unwrap_err();
    assert_eq!(err.to_string(), "this source isn't supported yet: Plaid");
}

fn bank_settings(url: &str) -> OfxDirectSettings {
    OfxDirectSettings {
        url: url.to_owned(),
        org: "EXAMPLECB".into(),
        fid: "99901".into(),
        bank_id: Some("999999999".into()),
        account_number: "000012345678".into(),
        account_kind: AccountKind::Checking,
        user_id: "testuser".into(),
        password: SecretRef::new("bank-password"),
        app_id: "QWIN".into(),
        app_ver: "2700".into(),
        version: OfxVersion::V102,
        // OFX 1.0.2 has no CLIENTUID; it is left out.
        client_uid: Some("6E4F0E0A-0000-4000-8000-000000000001".into()),
        user_agent: None,
    }
}

#[test]
fn direct_connect_requests_match_their_snapshots() {
    let pw = Secret::new("secret&1");
    let ctx = RequestContext {
        password: &pw,
        now: datetime(2026, 10, 19, 12, 34, 56, 0),
        trnuid: "TRN-0001",
        file_uid: "FILE0001",
    };
    let bank = ofx_direct::statement_request(
        &bank_settings("https://ofx.example.test/ofx"),
        &ctx,
        date(2026, 9, 1),
        None,
    );
    assert_eq!(
        bank,
        String::from_utf8(fixture("ofx/request-bank-102.ofx")).unwrap()
    );

    let pw = Secret::new("pw");
    let card_settings = OfxDirectSettings {
        org: "EXAMPLECARD".into(),
        fid: "77701".into(),
        bank_id: None,
        account_number: "4000001234567890".into(),
        account_kind: AccountKind::CreditCard,
        app_id: "QBW".into(),
        version: OfxVersion::V220,
        ..bank_settings("https://ofx.example.test/ofx")
    };
    let card = ofx_direct::statement_request(
        &card_settings,
        &RequestContext {
            password: &pw,
            now: datetime(2026, 10, 19, 12, 34, 56, 0),
            trnuid: "TRN-0002",
            file_uid: "FILE0002",
        },
        date(2026, 9, 1),
        Some(date(2026, 10, 19)),
    );
    assert_eq!(
        card,
        String::from_utf8(fixture("ofx/request-card-220.ofx")).unwrap()
    );

    // Our own reader reads both back.
    for req in [&bank, &card] {
        let tree = lb_feeds::ofx::parse_tree(&req[req.find("<OFX>").unwrap()..]);
        let sonrq = tree
            .child("OFX")
            .unwrap()
            .path(&["SIGNONMSGSRQV1", "SONRQ"])
            .unwrap();
        assert_eq!(sonrq.text("USERID"), Some("testuser"));
    }
}

/// An OFX server: `/ofx` answers statement requests with the checking fixture
/// and account-list requests with the account list, when the password is `pw`.
async fn ofx_server() -> (u16, Seen) {
    async fn ofx(State(seen): State<Seen>, headers: HeaderMap, body: Bytes) -> impl IntoResponse {
        let body = String::from_utf8_lossy(&body).into_owned();
        seen.bodies.lock().unwrap().push(body.clone());
        if headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            != Some("application/x-ofx")
        {
            return (StatusCode::UNSUPPORTED_MEDIA_TYPE, Vec::new());
        }
        let answer = if !body.contains("<USERPASS>pw") {
            fixture("ofx/signon-error.ofx")
        } else if body.contains("<ACCTINFORQ>") {
            fixture("ofx/acctinfo-response.ofx")
        } else {
            fixture("ofx/checking-sgml-102.ofx")
        };
        (StatusCode::OK, answer)
    }
    async fn down() -> impl IntoResponse {
        (StatusCode::SERVICE_UNAVAILABLE, "maintenance")
    }
    let seen = Seen::default();
    let app = Router::new()
        .route("/ofx", post(ofx))
        .route("/down", post(down))
        .with_state(seen.clone());
    (serve(app).await, seen)
}

#[tokio::test]
async fn direct_connect_downloads_statements_and_account_lists() {
    let (port, seen) = ofx_server().await;
    let settings = bank_settings(&format!("http://127.0.0.1:{port}/ofx"));
    let dc = OfxDirect::new(client(), settings.clone(), Secret::new("pw")).unwrap();
    assert!(!format!("{dc:?}").contains("pw\""));

    let statements = dc.fetch(Some(date(2026, 10, 1))).await.unwrap();
    assert_eq!(statements.len(), 1);
    assert_eq!(statements[0].transactions.len(), 5);
    let sent = seen.bodies.lock().unwrap()[0].clone();
    assert!(sent.starts_with("OFXHEADER:100\r\n"));
    assert!(sent.contains("<DTSTART>20261001\r\n"));
    assert!(sent.contains("<APPID>QWIN\r\n<APPVER>2700\r\n"));

    let accounts = dc.accounts().await.unwrap();
    let summary: Vec<(String, AccountKind, Option<String>)> = accounts
        .iter()
        .map(|a| (a.account_id_masked.clone(), a.account_type, a.name.clone()))
        .collect();
    assert_eq!(
        summary,
        vec![
            (
                "****5678".into(),
                AccountKind::Checking,
                Some("Everyday Checking".into())
            ),
            (
                "****7890".into(),
                AccountKind::CreditCard,
                Some("Rewards Visa".into())
            ),
        ]
    );
    // The listed account's fingerprint is the one its statements carry.
    assert_eq!(accounts[0].account_key, statements[0].account.account_key);

    // A wrong password is the bank's sign-on error.
    let wrong = OfxDirect::new(client(), settings.clone(), Secret::new("nope")).unwrap();
    let err = wrong.fetch(None).await.unwrap_err();
    assert!(matches!(err, FeedError::Unauthorized { .. }));
    assert!(err.to_string().contains("code 15500"), "{err}");

    // A server that isn't answering OFX.
    let down = OfxDirect::new(
        client(),
        bank_settings(&format!("http://127.0.0.1:{port}/down")),
        Secret::new("pw"),
    )
    .unwrap();
    let err = down.fetch(None).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "the bank's OFX server: the server answered 503 Service Unavailable"
    );
    // And one that isn't there at all.
    let gone = OfxDirect::new(
        client(),
        bank_settings("http://127.0.0.1:9/ofx"),
        Secret::new("pw"),
    )
    .unwrap();
    assert!(matches!(
        gone.fetch(None).await.unwrap_err(),
        FeedError::Network { .. }
    ));
}
