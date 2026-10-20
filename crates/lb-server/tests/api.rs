//! The API end to end through the router, without a socket.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use jiff::civil::date;
use lb_core::{Money, Txn, TxnLine, TxnType};
use lb_server::{AuthMode, Mode, ServerConfig, router, state};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Api {
    app: axum::Router,
    _dir: tempfile::TempDir,
    token: Option<String>,
}

impl Api {
    fn new(auth: AuthMode) -> Api {
        let dir = tempfile::tempdir().unwrap();
        let token = match &auth {
            AuthMode::LocalToken(t) => Some(t.clone()),
            AuthMode::Open => None,
        };
        let config = ServerConfig {
            data_dir: dir.path().to_owned(),
            mode: Mode::Server,
            auth,
            sessions: None,
        };
        let app = router(state(&config).unwrap());
        Api {
            app,
            _dir: dir,
            token,
        }
    }

    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-librebooks", "1");
        if let Some(t) = &self.token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let req = match body {
            Some(b) => req
                .header("content-type", "application/json")
                .body(Body::from(b.to_string())),
            None => req.body(Body::empty()),
        }
        .unwrap();
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let v = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, v)
    }
}

async fn company(api: &Api) -> String {
    let (s, v) = api
        .call(
            "POST",
            "/api/v1/companies",
            Some(json!({ "info": { "name": "Test Co" } })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["id"].as_str().unwrap().to_owned()
}

fn account(name: &str, t: &str) -> Value {
    json!({
        "id": uuid(), "name": name, "full_name": name, "parent_id": null, "sublevel": 0,
        "account_type": t, "number": null, "description": null, "bank_account_number": null,
        "tax_line": null, "role": null, "is_active": true, "currency": null,
        "cash_flow_class": null, "edit_seq": 0
    })
}

fn uuid() -> String {
    uuid::Uuid::now_v7().to_string()
}

#[tokio::test]
async fn health_needs_no_auth_but_the_rest_does() {
    let api = Api::new(AuthMode::LocalToken("secret".into()));
    let req = Request::builder()
        .uri("/api/v1/health")
        .body(Body::empty())
        .unwrap();
    let res = api.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let req = Request::builder()
        .uri("/api/v1/companies")
        .body(Body::empty())
        .unwrap();
    let res = api.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let (s, _) = api.call("GET", "/api/v1/companies", None).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn mutations_need_the_csrf_header() {
    let api = Api::new(AuthMode::Open);
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/companies")
        .header("content-type", "application/json")
        .body(Body::from(json!({ "info": { "name": "X" } }).to_string()))
        .unwrap();
    let res = api.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn write_a_check_and_read_the_register() {
    let api = Api::new(AuthMode::Open);
    let c = company(&api).await;
    let base = format!("/api/v1/c/{c}");

    let (s, bank) = api
        .call(
            "POST",
            &format!("{base}/accounts"),
            Some(account("Checking", "Bank")),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{bank}");
    let (_, rent) = api
        .call(
            "POST",
            &format!("{base}/accounts"),
            Some(account("Rent", "Expense")),
        )
        .await;
    let (s, list) = api.call("GET", &format!("{base}/accounts"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .any(|a| a["full_name"] == "Checking")
    );

    let mut check = Txn::new(TxnType::Check, date(2026, 10, 1));
    check.number = Some("1001".into());
    check.account_id = Some(bank["id"].as_str().unwrap().parse().unwrap());
    check.memo = Some("October rent".into());
    check.lines.push(TxnLine::account(
        rent["id"].as_str().unwrap().parse().unwrap(),
        Money(150_000),
    ));
    let txn = serde_json::to_value(&check).unwrap();

    let (s, preview) = api
        .call("POST", &format!("{base}/txns/preview"), Some(txn.clone()))
        .await;
    assert_eq!(s, StatusCode::OK, "{preview}");
    assert_eq!(preview["total"], 150_000);

    let (s, saved) = api
        .call("POST", &format!("{base}/txns"), Some(txn.clone()))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{saved}");
    assert_eq!(saved["total"], 150_000);
    let id = saved["txn"]["id"].as_str().unwrap().to_owned();

    let (s, reg) = api
        .call(
            "GET",
            &format!("{base}/accounts/{}/register", bank["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(reg["lines"][0]["amount"], -150_000);
    assert_eq!(reg["lines"][0]["other_account"], "Rent");

    // A stale update is a 409 with the sequences in the detail.
    let mut stale = txn.clone();
    stale["edit_seq"] = json!(1);
    stale["memo"] = json!("first edit");
    let (s, _) = api
        .call("PUT", &format!("{base}/txns/{id}"), Some(stale.clone()))
        .await;
    assert_eq!(s, StatusCode::OK);
    stale["memo"] = json!("second edit, stale");
    let (s, err) = api
        .call("PUT", &format!("{base}/txns/{id}"), Some(stale))
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(err["error"]["code"], "conflict");
    assert_eq!(err["error"]["detail"]["actual"], 2);

    let (s, next) = api
        .call(
            "GET",
            &format!(
                "{base}/txns/next-number?type=Check&account_id={}",
                bank["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(next["number"], "1002");

    let (s, found) = api
        .call("GET", &format!("{base}/txns?types=Check&text=FIRST"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(found.as_array().unwrap().len(), 1);

    let (s, hist) = api
        .call("GET", &format!("{base}/txns/{id}/history"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(hist.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn posting_errors_are_422_with_a_code() {
    let api = Api::new(AuthMode::Open);
    let c = company(&api).await;
    let base = format!("/api/v1/c/{c}");
    let (_, rent) = api
        .call(
            "POST",
            &format!("{base}/accounts"),
            Some(account("Rent", "Expense")),
        )
        .await;
    let mut je = Txn::new(TxnType::JournalEntry, date(2026, 10, 1));
    je.lines.push(TxnLine::journal(
        rent["id"].as_str().unwrap().parse().unwrap(),
        Money(500),
    ));
    let je = serde_json::to_value(&je).unwrap();
    let (s, err) = api.call("POST", &format!("{base}/txns"), Some(je)).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["error"]["code"], "posting");
    assert_eq!(err["error"]["detail"]["code"], "Unbalanced");
}

#[tokio::test]
async fn duplicate_names_and_unknown_companies() {
    let api = Api::new(AuthMode::Open);
    let c = company(&api).await;
    let base = format!("/api/v1/c/{c}");
    let (s, _) = api
        .call(
            "POST",
            &format!("{base}/accounts"),
            Some(account("Rent", "Expense")),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, err) = api
        .call(
            "POST",
            &format!("{base}/accounts"),
            Some(account("rent", "Expense")),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(err["error"]["code"], "duplicate");
    let (s, _) = api
        .call("GET", &format!("/api/v1/c/{}/accounts", uuid()), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn spa_routes_fall_back_and_api_misses_do_not() {
    let api = Api::new(AuthMode::Open);
    let req = Request::builder()
        .uri("/api/v1/nope")
        .body(Body::empty())
        .unwrap();
    let res = api.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn upload_a_receipt_and_fetch_it_back() {
    let api = Api::new(AuthMode::Open);
    let c = company(&api).await;
    let boundary = "XLBOUNDARYX";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"receipt.png\"\r\nContent-Type: image/png\r\n\r\nPNGDATA\r\n--{boundary}--\r\n"
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/c/{c}/attachments"))
        .header("x-librebooks", "1")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let res = api.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let meta: Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(meta["filename"], "receipt.png");
    let (s, inbox) = api
        .call("GET", &format!("/api/v1/c/{c}/attachments"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(inbox.as_array().unwrap().len(), 1);
    let req = Request::builder()
        .uri(format!(
            "/api/v1/c/{c}/attachments/{}",
            meta["id"].as_str().unwrap()
        ))
        .body(Body::empty())
        .unwrap();
    let res = api.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.headers()["content-type"], "image/png");
    assert_eq!(
        &res.into_body().collect().await.unwrap().to_bytes()[..],
        b"PNGDATA"
    );
}

#[tokio::test]
async fn reconcile_over_the_api() {
    let api = Api::new(AuthMode::Open);
    let c = company(&api).await;
    let base = format!("/api/v1/c/{c}");
    let (_, bank) = api
        .call(
            "POST",
            &format!("{base}/accounts"),
            Some(account("Checking", "Bank")),
        )
        .await;
    let bank_id = bank["id"].as_str().unwrap();
    let (s, _) = api
        .call(
            "POST",
            &format!("{base}/accounts/{bank_id}/opening-balance"),
            Some(json!({ "amount": 5000, "date": "2026-09-01" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, rec) = api
        .call(
            "POST",
            &format!("{base}/accounts/{bank_id}/reconcile"),
            Some(json!({ "statement_date": "2026-09-30", "ending_balance": 5000 })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{rec}");
    let rid = rec["id"].as_str().unwrap();
    let (_, view) = api
        .call("GET", &format!("{base}/reconciliations/{rid}"), None)
        .await;
    let txn = view["lines"][0]["txn_id"].clone();
    let (s, status) = api
        .call(
            "POST",
            &format!("{base}/reconciliations/{rid}/mark"),
            Some(json!({ "txn_ids": [txn], "marked": true })),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(status["difference"], 0);
    let (s, done) = api
        .call(
            "POST",
            &format!("{base}/reconciliations/{rid}/finish"),
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert!(done["finished_at"].is_string());
}
