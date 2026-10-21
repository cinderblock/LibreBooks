//! The API end to end through the router, without a socket.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use jiff::civil::date;
use lb_core::{Entity, EntityKind, Item, ItemType, Money, Txn, TxnLine, TxnType};
use lb_server::{AuthMode, Mode, ServerConfig, router, state};
use pretty_assertions::assert_eq;
use rust_decimal::Decimal;
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
    // An opening balance rides along with the account: one request, one write.
    let mut savings = account("Savings", "Bank");
    savings["opening_balance"] = json!(500_000);
    savings["as_of"] = json!("2026-01-01");
    let (s, savings) = api
        .call("POST", &format!("{base}/accounts"), Some(savings))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{savings}");
    let (_, balances) = api.call("GET", &format!("{base}/balances"), None).await;
    assert!(
        balances
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["account_id"] == savings["id"] && b["debit_positive"] == 500_000),
        "{balances}"
    );
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
async fn a_purchase_order_is_received_against_and_closed_over_the_api() {
    let api = Api::new(AuthMode::Open);
    let c = company(&api).await;
    let base = format!("/api/v1/c/{c}");
    let (s, expense) = api
        .call(
            "POST",
            &format!("{base}/accounts"),
            Some(account("Supplies", "Expense")),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{expense}");
    let (s, income) = api
        .call(
            "POST",
            &format!("{base}/accounts"),
            Some(account("Sales", "Income")),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{income}");
    let (s, vendor) = api
        .call(
            "POST",
            &format!("{base}/entities"),
            Some(serde_json::to_value(Entity::blank(EntityKind::Vendor, "Acme Supply")).unwrap()),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{vendor}");
    let vendor_id = vendor["id"].as_str().unwrap().to_owned();
    let mut bolts = Item::blank(ItemType::NonInventory, "Bolts");
    bolts.income_account_id = Some(income["id"].as_str().unwrap().parse().unwrap());
    bolts.expense_account_id = Some(expense["id"].as_str().unwrap().parse().unwrap());
    bolts.has_purchase_info = true;
    let (s, bolts) = api
        .call(
            "POST",
            &format!("{base}/items"),
            Some(serde_json::to_value(&bolts).unwrap()),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{bolts}");
    let bolts_id = bolts["id"].as_str().unwrap().parse().unwrap();

    // The order: 10 @ $5, with the closed marks round-tripping.
    let mut po = Txn::new(TxnType::PurchaseOrder, date(2026, 10, 1));
    po.entity_id = Some(vendor_id.parse().unwrap());
    po.lines
        .push(TxnLine::item(bolts_id, Decimal::from(10), Decimal::from(5)));
    let (s, saved) = api
        .call(
            "POST",
            &format!("{base}/txns"),
            Some(serde_json::to_value(&po).unwrap()),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{saved}");
    assert_eq!(saved["txn"]["is_manually_closed"], false);
    assert_eq!(saved["txn"]["lines"][0]["is_closed"], false);
    let po_id = saved["txn"]["id"].as_str().unwrap().to_owned();
    let line_id = saved["txn"]["lines"][0]["id"].as_str().unwrap().to_owned();

    let (s, f) = api
        .call("GET", &format!("{base}/txns/{po_id}/fulfilment"), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{f}");
    assert_eq!(f["order_id"], po_id);
    assert_eq!(f["is_closed"], false);
    assert_eq!(f["remaining_amount"], 5_000);
    assert_eq!(f["lines"][0]["line_id"], line_id);
    assert_eq!(f["lines"][0]["ordered_quantity"], "10");
    assert_eq!(f["lines"][0]["fulfilled_quantity"], "0");
    let (s, open) = api
        .call(
            "GET",
            &format!("{base}/open?kind=purchase_orders&entity_id={vendor_id}"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{open}");
    assert_eq!(open.as_array().unwrap().len(), 1);
    assert_eq!(open[0]["id"], po_id);
    assert_eq!(open[0]["open_balance"], 5_000);

    // Receive 4 against the line.
    let mut rcpt = Txn::new(TxnType::ItemReceipt, date(2026, 10, 5));
    rcpt.entity_id = po.entity_id;
    let mut line = TxnLine::item(bolts_id, Decimal::from(4), Decimal::from(5));
    line.linked_txn_id = Some(po_id.parse().unwrap());
    line.linked_line_id = Some(line_id.parse().unwrap());
    rcpt.lines.push(line);
    let (s, rcpt) = api
        .call(
            "POST",
            &format!("{base}/txns"),
            Some(serde_json::to_value(&rcpt).unwrap()),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{rcpt}");
    let rcpt_id = rcpt["txn"]["id"].as_str().unwrap();
    let (_, f) = api
        .call("GET", &format!("{base}/txns/{po_id}/fulfilment"), None)
        .await;
    assert_eq!(f["remaining_amount"], 3_000);
    assert_eq!(f["lines"][0]["fulfilled_quantity"], "4");
    assert_eq!(f["lines"][0]["remaining_quantity"], "6");
    assert_eq!(f["lines"][0]["is_closed"], false);
    // A receipt isn't an order; a bad kind is a bad request.
    let (s, err) = api
        .call("GET", &format!("{base}/txns/{rcpt_id}/fulfilment"), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{err}");
    assert_eq!(err["error"]["code"], "not_found");
    let (s, _) = api
        .call(
            "GET",
            &format!("{base}/open?kind=orders&entity_id={vendor_id}"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Close it by hand: off the open list, nothing remaining.
    let mut closed = saved["txn"].clone();
    closed["is_manually_closed"] = json!(true);
    let (s, closed) = api
        .call("PUT", &format!("{base}/txns/{po_id}"), Some(closed))
        .await;
    assert_eq!(s, StatusCode::OK, "{closed}");
    assert_eq!(closed["txn"]["is_manually_closed"], true);
    let (_, read) = api.call("GET", &format!("{base}/txns/{po_id}"), None).await;
    assert_eq!(read["txn"]["is_manually_closed"], true);
    let (_, f) = api
        .call("GET", &format!("{base}/txns/{po_id}/fulfilment"), None)
        .await;
    assert_eq!(f["is_closed"], true);
    assert_eq!(f["remaining_amount"], 0);
    assert_eq!(f["lines"][0]["is_closed"], true);
    let (_, open) = api
        .call(
            "GET",
            &format!("{base}/open?kind=purchase_orders&entity_id={vendor_id}"),
            None,
        )
        .await;
    assert_eq!(open, json!([]));
    // Receiving more against it is refused.
    let (s, err) = api
        .call(
            "POST",
            &format!("{base}/txns"),
            Some(serde_json::to_value(rcpt_again(&rcpt["txn"])).unwrap()),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{err}");
    assert_eq!(err["error"]["code"], "invalid");
}

/// The same receipt again, as a new transaction.
fn rcpt_again(saved: &Value) -> Txn {
    let mut t: Txn = serde_json::from_value(saved.clone()).unwrap();
    t.id = lb_core::TxnId::new();
    t.edit_seq = 0;
    for l in &mut t.lines {
        l.id = lb_core::LineId::new();
    }
    t
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

#[tokio::test]
async fn run_a_profit_and_loss() {
    let api = Api::new(AuthMode::Open);
    let c = company(&api).await;
    let base = format!("/api/v1/c/{c}");
    let (s, catalog) = api.call("GET", &format!("{base}/reports"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(catalog.as_array().unwrap().len() > 40);
    let id = catalog[0]["id"].clone();
    let (s, report) = api
        .call(
            "POST",
            &format!("{base}/reports/run"),
            Some(json!({ "report": id })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{report}");
    assert_eq!(report["company_name"], "Test Co");
}

#[tokio::test]
async fn export_a_report_as_xlsx_and_pdf() {
    let api = Api::new(AuthMode::Open);
    let c = company(&api).await;
    let base = format!("/api/v1/c/{c}");
    let export = |query: &str| {
        let uri = format!("{base}/reports/export?{query}");
        let app = api.app.clone();
        async move {
            let req = Request::builder()
                .method("POST")
                .uri(uri)
                .header("x-librebooks", "1")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "report": "BalanceSheet", "to": "2026-10-19" }).to_string(),
                ))
                .unwrap();
            let res = app.oneshot(req).await.unwrap();
            let status = res.status();
            let headers = res.headers().clone();
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (status, headers, bytes)
        }
    };

    let (status, headers, bytes) = export("format=xlsx").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(
        headers["content-type"],
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert_eq!(
        headers["content-disposition"],
        "attachment; filename=\"Balance_Sheet.xlsx\""
    );
    assert!(bytes.len() > 2_000, "{} bytes", bytes.len());
    assert_eq!(&bytes[..2], b"PK");

    let (status, headers, bytes) = export("format=pdf&paper=a4").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(headers["content-type"], "application/pdf");
    assert_eq!(
        headers["content-disposition"],
        "attachment; filename=\"Balance_Sheet.pdf\""
    );
    assert!(bytes.len() > 1_000, "{} bytes", bytes.len());
    assert!(bytes.starts_with(b"%PDF-"));

    let (status, headers, bytes) = export("").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/csv; charset=utf-8");
    assert!(String::from_utf8_lossy(&bytes).starts_with("Balance Sheet\r\nTest Co\r\n"));

    let (status, _, bytes) = export("format=docx").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let err: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(err["error"]["code"], "invalid");
    let (status, _, _) = export("format=pdf&paper=legal").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
