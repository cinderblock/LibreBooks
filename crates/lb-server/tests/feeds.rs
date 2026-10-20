//! Bank Feeds over the API: a file import, the queue, adding and matching,
//! rules, and a SimpleFIN connection against a local stand-in bridge.

mod support;

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use base64::Engine as _;
use jiff::civil::date;
use lb_auth::Role;
use lb_core::{Money, Txn, TxnLine, TxnType};
use lb_server::OpsOptions;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use support::Server;

/// A checking statement: rent out, payroll in.
const OFX: &str = "OFXHEADER:100\r\nDATA:OFXSGML\r\nVERSION:102\r\nSECURITY:NONE\r\nENCODING:USASCII\r\nCHARSET:1252\r\nCOMPRESSION:NONE\r\nOLDFILEUID:NONE\r\nNEWFILEUID:NONE\r\n\r\n\
<OFX><SIGNONMSGSRSV1><SONRS><STATUS><CODE>0<SEVERITY>INFO</STATUS><DTSERVER>20261015120000<LANGUAGE>ENG</SONRS></SIGNONMSGSRSV1>\r\n\
<BANKMSGSRSV1><STMTTRNRS><TRNUID>1<STATUS><CODE>0<SEVERITY>INFO</STATUS>\r\n\
<STMTRS><CURDEF>USD<BANKACCTFROM><BANKID>121000248<ACCTID>1234567890<ACCTTYPE>CHECKING</BANKACCTFROM>\r\n\
<BANKTRANLIST><DTSTART>20261001<DTEND>20261015\r\n\
<STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20261005<TRNAMT>-1200.00<FITID>A1<NAME>LANDLORD LLC<MEMO>October rent</STMTTRN>\r\n\
<STMTTRN><TRNTYPE>CREDIT<DTPOSTED>20261010<TRNAMT>2500.00<FITID>A2<NAME>ACME PAYROLL</STMTTRN>\r\n\
<STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20261012<TRNAMT>-89.99<FITID>A3<NAME>COMCAST CABLE</STMTTRN>\r\n\
</BANKTRANLIST><LEDGERBAL><BALAMT>1210.01<DTASOF>20261015</LEDGERBAL></STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>\r\n";

fn by_payee<'a>(txns: &'a Value, payee: &str) -> &'a Value {
    txns.as_array()
        .unwrap()
        .iter()
        .find(|t| t["payee"].as_str().unwrap_or_default().contains(payee))
        .unwrap_or_else(|| panic!("no {payee} in {txns}"))
}

#[tokio::test]
async fn import_review_add_match_and_rules() {
    let s = Server::open(OpsOptions::default());
    let c = s.company("Feeds Co").await;
    let checking = s.account(&c, "Checking", "Bank").await;
    let rent = s.account(&c, "Rent", "Expense").await;
    let utilities = s.account(&c, "Utilities", "Expense").await;
    let sales = s.account(&c, "Sales", "Income").await;
    let base = format!("/api/v1/c/{c}/feeds");

    let (st, d) = s
        .upload(
            &format!("{base}/detect"),
            Some(("stmt.qfx", OFX.as_bytes())),
            &[],
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{d}");
    assert_eq!(d["kind"], "Ofx");
    assert_eq!(
        d["statements"][0]["transactions"].as_array().unwrap().len(),
        3
    );

    // A rule first, so the import applies it.
    let (st, rule) = s
        .call(
            "POST",
            &format!("{base}/rules"),
            Some(json!({
                "id": uuid::Uuid::now_v7().to_string(), "name": "Cable", "priority": 1, "enabled": true,
                "match_all": true,
                "conditions": [{ "kind": "Text", "field": "Payee", "op": "Contains", "value": "comcast" }],
                "actions": { "rename_to": "Comcast", "account_id": utilities },
            })),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{rule}");

    let (st, report) = s
        .upload(
            &format!("{base}/import"),
            Some(("stmt.qfx", OFX.as_bytes())),
            &[("account_id", &checking)],
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{report}");
    assert_eq!(report["import"]["imported"].as_array().unwrap().len(), 3);
    assert_eq!(report["waiting"], 3);
    // Again: nothing new.
    let (_, again) = s
        .upload(
            &format!("{base}/import"),
            Some(("stmt.qfx", OFX.as_bytes())),
            &[("account_id", &checking)],
        )
        .await;
    assert_eq!(again["import"]["duplicates"], 3, "{again}");

    let (_, summary) = s.call("GET", &base, None).await;
    assert_eq!(summary[0]["account_id"], checking.as_str());
    assert_eq!(summary[0]["new"], 3);

    let (st, txns) = s
        .call(
            "GET",
            &format!("{base}/txns?statuses=new&account_id={checking}"),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{txns}");
    assert_eq!(txns.as_array().unwrap().len(), 3);
    let (st, _) = s
        .call("GET", &format!("{base}/txns?statuses=bogus"), None)
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // The rule renamed and categorised it (as a suggestion; the bank's text stays).
    let cable = by_payee(&txns, "COMCAST");
    assert_eq!(cable["suggestion"]["payee"], "Comcast", "{cable}");
    assert_eq!(
        cable["suggestion"]["account_id"],
        utilities.as_str(),
        "{cable}"
    );

    // Rent: preview, then add as a check to Rent.
    let rent_line = by_payee(&txns, "LANDLORD")["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (st, preview) = s
        .call(
            "POST",
            &format!("{base}/txns/{rent_line}/preview"),
            Some(json!({ "account_id": rent })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{preview}");
    assert_eq!(preview["txn"]["txn_type"], "Check", "{preview}");
    assert_eq!(preview["total"], 120_000);
    let (st, added) = s
        .call(
            "POST",
            &format!("{base}/txns/{rent_line}/add"),
            Some(json!({ "account_id": rent })),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{added}");
    let (_, register) = s
        .call(
            "GET",
            &format!("/api/v1/c/{c}/accounts/{checking}/register"),
            None,
        )
        .await;
    assert_eq!(register["lines"].as_array().unwrap().len(), 1, "{register}");

    // Undo puts it back in the queue (and deletes the check).
    let (st, back) = s
        .call("POST", &format!("{base}/txns/{rent_line}/undo"), None)
        .await;
    assert_eq!(st, StatusCode::OK, "{back}");
    assert_eq!(back["status"], "New");

    // Payroll: entered by hand first, then matched.
    let payroll = by_payee(&txns, "PAYROLL")["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut deposit = Txn::new(TxnType::Deposit, date(2026, 10, 9));
    deposit.account_id = Some(checking.parse().unwrap());
    deposit
        .lines
        .push(TxnLine::account(sales.parse().unwrap(), Money(250_000)));
    let (st, dep) = s
        .call(
            "POST",
            &format!("/api/v1/c/{c}/txns"),
            Some(serde_json::to_value(&deposit).unwrap()),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{dep}");
    let txn_id = dep["txn"]["id"].as_str().unwrap();
    let (st, cands) = s
        .call("GET", &format!("{base}/txns/{payroll}/candidates"), None)
        .await;
    assert_eq!(st, StatusCode::OK, "{cands}");
    assert_eq!(cands[0]["txn_id"], txn_id, "{cands}");
    let (st, v) = s
        .call(
            "POST",
            &format!("{base}/txns/{payroll}/match"),
            Some(json!({ "txn_id": txn_id })),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    let (_, matched) = s.call("GET", &format!("{base}/txns/{payroll}"), None).await;
    assert_eq!(matched["status"], "Matched", "{matched}");

    // Ignore the cable bill; add-all of nothing left is harmless.
    let cable_id = cable["id"].as_str().unwrap();
    let (_, v) = s
        .call(
            "POST",
            &format!("{base}/txns/ignore"),
            Some(json!({ "ids": [cable_id] })),
        )
        .await;
    assert_eq!(v["ignored"], 1);
    let (st, batch) = s
        .call(
            "POST",
            &format!("{base}/txns/add-all"),
            Some(json!({ "ids": [rent_line], "request": { "account_id": rent } })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{batch}");
    assert_eq!(batch["added"].as_array().unwrap().len(), 1, "{batch}");

    let (_, v) = s.call("POST", &format!("{base}/rules/apply"), None).await;
    assert!(v["applied"].is_number(), "{v}");
    let (_, rules) = s.call("GET", &format!("{base}/rules"), None).await;
    assert_eq!(rules.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_csv_without_a_sure_mapping_asks_for_one() {
    let s = Server::open(OpsOptions::default());
    let c = s.company("CSV Co").await;
    let checking = s.account(&c, "Checking", "Bank").await;
    let csv = "when,what,how much\n10/05/2026,Coffee,-4.50\n";
    let (st, v) = s
        .upload(
            &format!("/api/v1/c/{c}/feeds/import"),
            Some(("x.csv", csv.as_bytes())),
            &[("account_id", &checking)],
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let (st, v) = s
        .upload(&format!("/api/v1/c/{c}/feeds/import"), None, &[])
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("no file"));
}

// ---------------------------------------------------------------------------
// SimpleFIN

const BRIDGE_ACCOUNTS: &str = r#"{
  "errors": [],
  "accounts": [{
    "org": { "domain": "examplebank.test", "name": "Example Bank" },
    "id": "ACT-1111-2222", "name": "Everyday Checking", "currency": "USD",
    "balance": "5198.51", "balance-date": 1792000000,
    "transactions": [
      { "id": "TRN-001", "posted": 1790856000, "amount": "2500.00", "description": "ACME PAYROLL" },
      { "id": "TRN-002", "posted": 1791028800, "amount": "-1200.00", "description": "CHECK 1041", "payee": "Landlord LLC" }
    ]
  }]
}"#;

#[derive(Clone, Default)]
struct Bridge {
    port: Arc<Mutex<u16>>,
    claims: Arc<Mutex<u32>>,
}

/// A SimpleFIN bridge: `/claim/DEMO` once, then `/simplefin/accounts` behind
/// basic auth `demo:s3cret`.
async fn bridge() -> (u16, Bridge) {
    async fn claim(State(b): State<Bridge>) -> impl IntoResponse {
        let mut n = b.claims.lock().unwrap();
        *n += 1;
        if *n > 1 {
            return (StatusCode::FORBIDDEN, String::new());
        }
        let port = *b.port.lock().unwrap();
        (
            StatusCode::OK,
            format!("http://demo:s3cret@127.0.0.1:{port}/simplefin"),
        )
    }
    async fn accounts(headers: HeaderMap) -> impl IntoResponse {
        let want = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("demo:s3cret")
        );
        if headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            != Some(&want)
        {
            return (StatusCode::FORBIDDEN, String::new());
        }
        (StatusCode::OK, BRIDGE_ACCOUNTS.to_owned())
    }
    let b = Bridge::default();
    let app = axum::Router::new()
        .route("/claim/DEMO", post(claim))
        .route("/simplefin/accounts", get(accounts))
        .with_state(b.clone());
    let base = support::serve(app).await;
    let port: u16 = base.rsplit(':').next().unwrap().parse().unwrap();
    *b.port.lock().unwrap() = port;
    (port, b)
}

#[tokio::test]
async fn simplefin_claim_connect_sync_and_disconnect() {
    let (port, _bridge) = bridge().await;
    let mut s = Server::with_sessions(OpsOptions::default());
    s.cookie = Some(s.sign_in("Ada", Role::Admin).await);
    let c = s.company("Bridge Co").await;
    let checking = s.account(&c, "Checking", "Bank").await;
    // A bookkeeper with access to the company manages its feeds.
    let admin = s.cookie.take();
    let bob_cookie = s.sign_in("Bob", Role::Bookkeeper).await;
    s.cookie = Some(bob_cookie.clone());
    let (_, me) = s.call("GET", "/api/v1/session", None).await;
    let bob = me["user"]["id"].as_str().unwrap().to_owned();
    s.cookie = admin;
    let (st, v) = s
        .call(
            "PUT",
            &format!("/api/v1/users/{bob}/companies/{c}"),
            Some(json!({ "role": "bookkeeper" })),
        )
        .await;
    assert!(st.is_success(), "{st} {v}");
    s.cookie = Some(bob_cookie);
    let base = format!("/api/v1/c/{c}/feeds");

    let setup = base64::engine::general_purpose::STANDARD
        .encode(format!("http://127.0.0.1:{port}/claim/DEMO"));
    let r = s
        .raw(
            "POST",
            &format!("{base}/simplefin/claim"),
            Some(json!({ "setup_token": setup })),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert!(
        !r.text().contains("s3cret"),
        "the access URL never comes back"
    );
    let claimed = r.json();
    let access = claimed["access"].as_str().unwrap().to_owned();
    assert!(
        access.starts_with(&format!("secret:feeds.{c}.")),
        "{access}"
    );
    assert_eq!(claimed["accounts"][0]["remote_id"], "ACT-1111-2222");

    // A connection can't point at anything but this company's own secrets.
    for bad in ["secret:tunnel-token", "env:HOME", "file:/etc/passwd"] {
        let (st, v) = s
            .call(
                "POST",
                &format!("{base}/connections"),
                Some(connection(&checking, bad)),
            )
            .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }
    let (st, conn) = s
        .call(
            "POST",
            &format!("{base}/connections"),
            Some(connection(&checking, &access)),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{conn}");
    let id = conn["id"].as_str().unwrap().to_owned();

    let (st, accounts) = s
        .call(
            "POST",
            &format!("{base}/connections/{id}/remote-accounts"),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{accounts}");
    assert_eq!(accounts[0]["name"], "Everyday Checking");

    let (st, report) = s
        .call("POST", &format!("{base}/connections/{id}/sync"), None)
        .await;
    assert_eq!(st, StatusCode::OK, "{report}");
    assert_eq!(
        report["import"]["imported"].as_array().unwrap().len(),
        2,
        "{report}"
    );
    let (_, conn) = s
        .call("GET", &format!("{base}/connections/{id}"), None)
        .await;
    assert!(conn["last_sync_at"].is_string(), "{conn}");
    assert_eq!(conn["last_error"], Value::Null);

    // Wrong credentials: the connection needs reconnecting (409, not 401).
    let (st, conn) = s
        .call(
            "PUT",
            &format!("{base}/connections/{id}/credentials"),
            Some(json!({ "value": format!("http://demo:wrong@127.0.0.1:{port}/simplefin") })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{conn}");
    let claimed_name = access.strip_prefix("secret:").unwrap();
    assert!(
        !s.state.ops.secrets.contains(claimed_name).unwrap(),
        "the replaced access URL is forgotten"
    );
    let (st, v) = s
        .call("POST", &format!("{base}/connections/{id}/sync"), None)
        .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "reconnect");
    let (_, conn) = s
        .call("GET", &format!("{base}/connections/{id}"), None)
        .await;
    assert!(conn["last_error"].is_string(), "{conn}");

    // Disconnecting deletes its credentials.
    let names = |s: &lb_server::AppState| -> Vec<String> {
        s.ops
            .secrets
            .list()
            .unwrap()
            .into_iter()
            .map(|i| i.name)
            .collect()
    };
    assert!(names(&s.state).iter().any(|n| n.contains(&id)));
    let seq = conn["edit_seq"].as_i64().unwrap();
    let (st, v) = s
        .call(
            "DELETE",
            &format!("{base}/connections/{id}?edit_seq={seq}"),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    assert!(
        !names(&s.state).iter().any(|n| n.contains(&id)),
        "{:?}",
        names(&s.state)
    );
}

fn connection(account: &str, access: &str) -> Value {
    json!({
        "id": uuid::Uuid::now_v7().to_string(),
        "name": "Everyday Checking",
        "account_id": account,
        "settings": { "provider": "SimpleFin", "access": access, "remote_account_id": "ACT-1111-2222" },
        "account_key": null,
        "not_before": null,
        "last_sync_at": null,
        "last_error": null,
        "edit_seq": 0,
    })
}
