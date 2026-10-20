//! The QuickBooks import over the API: settings (with the write-only bridge
//! key), the Web Connector's `.qwc` and password, and a whole sync and parity
//! check driven by a fake Web Connector client speaking SOAP to `/qbwc`, with
//! lb-qbd's fake QuickBooks running every qbXML document it collects.

mod support;

use std::path::Path;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lb_auth::Role;
use lb_qbd::qbwc::{QueueConfig, SessionConfig};
use lb_qbd::qbxml::xml::to_json;
use lb_qbd::testing::{FakeQuickBooks, FixtureSource};
use lb_server::OpsOptions;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use support::Server;

fn larkspur() -> FixtureSource {
    FixtureSource::load_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../lb-qbd/testdata/larkspur"),
    )
    .expect("lb-qbd's fixtures")
}

/// A server whose Web Connector waits are short enough for a test.
fn server() -> Server {
    let s = Server::open(OpsOptions::default());
    s.state.imports.set_timing(
        QueueConfig {
            first_poll_wait: Duration::from_secs(10),
            in_session_wait: Duration::from_secs(10),
        },
        SessionConfig {
            poll_wait: Duration::from_secs(10),
            settle_wait: Duration::from_millis(300),
        },
    );
    s.state.imports.set_page_size(4);
    s
}

/// The value under the first `…Result` element of a SOAP response.
fn soap_result(xml: &str) -> Value {
    fn find(v: &Value) -> Option<Value> {
        if let Value::Object(m) = v {
            for (k, v) in m {
                if k.ends_with("Result") {
                    return Some(v.clone());
                }
                if let Some(found) = find(v) {
                    return Some(found);
                }
            }
        }
        None
    }
    find(&to_json(xml).expect("SOAP is XML")).expect("a …Result element")
}

fn result_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(m) => m
            .get("#text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        _ => String::new(),
    }
}

/// A Web Connector: speaks SOAP to `/qbwc` and runs what it gets in a fake
/// QuickBooks.
struct WebConnector<'a> {
    server: &'a Server,
    quickbooks: FakeQuickBooks,
    user: String,
    password: String,
    /// The company's import route. When set, the connector lets the job finish
    /// before getLastError/closeConnection, the way a slow machine would.
    settle: Option<String>,
}

/// One update run as the Web Connector sees it.
#[derive(Debug, Default)]
struct Run {
    outcome: String,
    rounds: usize,
    percents: Vec<i32>,
    last_error: String,
    summary: String,
}

impl WebConnector<'_> {
    async fn soap(&self, method: &str, params: &[(&str, &str)]) -> (StatusCode, Value) {
        let mut body = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\"><soap:Body><{method} xmlns=\"http://developer.intuit.com/\">"
        );
        for (k, v) in params {
            body.push_str(&format!("<{k}>{}</{k}>", quick_xml_escape(v)));
        }
        body.push_str(&format!("</{method}></soap:Body></soap:Envelope>"));
        let req = Request::builder()
            .method("POST")
            .uri("/qbwc")
            .header("content-type", "text/xml; charset=utf-8")
            .header(
                "SOAPAction",
                format!("\"http://developer.intuit.com/{method}\""),
            );
        // No cookie, no X-LibreBooks header: the Web Connector sends neither.
        let res = tower::ServiceExt::oneshot(
            self.server.app.clone(),
            req.body(Body::from(body)).unwrap(),
        )
        .await
        .unwrap();
        let status = res.status();
        let bytes = http_body_util::BodyExt::collect(res.into_body())
            .await
            .unwrap()
            .to_bytes();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        (status, soap_result(&text))
    }

    /// `authenticate`: the ticket and the outcome string.
    async fn authenticate(&self, password: &str) -> (String, String) {
        let (st, v) = self
            .soap(
                "authenticate",
                &[("strUserName", &self.user), ("strPassword", password)],
            )
            .await;
        assert_eq!(st, StatusCode::OK);
        let strings = v["string"].as_array().expect("two strings");
        (result_text(&strings[0]), result_text(&strings[1]))
    }

    /// A whole update run, the way the Web Connector does it.
    async fn update(&self) -> Run {
        let mut run = Run::default();
        let (ticket, outcome) = self.authenticate(&self.password).await;
        run.outcome = outcome.clone();
        if outcome == "none" || outcome == "nvu" || outcome == "busy" {
            return run;
        }
        let mut hcp =
            "<QBXML><QBXMLMsgsRs><HostQueryRs statusCode=\"0\"/></QBXMLMsgsRs></QBXML>".to_owned();
        loop {
            let (_, v) = self
                .soap(
                    "sendRequestXML",
                    &[
                        ("ticket", &ticket),
                        ("strHCPResponse", &hcp),
                        ("strCompanyFileName", r"C:\Books\Larkspur.QBW"),
                        ("qbXMLCountry", "US"),
                        ("qbXMLMajorVers", "13"),
                        ("qbXMLMinorVers", "0"),
                    ],
                )
                .await;
            hcp.clear();
            let request = result_text(&v);
            if request.is_empty() {
                break;
            }
            run.rounds += 1;
            assert!(run.rounds < 1000, "the update never ends");
            let response = self.quickbooks.answer(&request);
            let (_, v) = self
                .soap(
                    "receiveResponseXML",
                    &[
                        ("ticket", &ticket),
                        ("response", &response),
                        ("hresult", ""),
                        ("message", ""),
                    ],
                )
                .await;
            let percent: i32 = result_text(&v).parse().expect("an integer");
            run.percents.push(percent);
            if !(0..100).contains(&percent) {
                break;
            }
        }
        if let Some(base) = &self.settle {
            // The real Web Connector asks getLastError only after QuickBooks has
            // answered; by then the job may well have finished and reported.
            for _ in 0..20 {
                let (_, v) = self
                    .server
                    .call("GET", &format!("{base}/status?wait=2"), None)
                    .await;
                if v["running"].is_null() {
                    break;
                }
            }
        }
        let (_, v) = self.soap("getLastError", &[("ticket", &ticket)]).await;
        run.last_error = result_text(&v);
        let (_, v) = self.soap("closeConnection", &[("ticket", &ticket)]).await;
        run.summary = result_text(&v);
        run
    }
}

fn quick_xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn balance(s: &Server, company: &str, full_name: &str) -> i64 {
    let id: lb_core::CompanyId = company.parse().unwrap();
    let books = s.state.companies.get(id).unwrap();
    books
        .read(|c| {
            let a = lb_store::accounts::by_full_name(c, full_name)?.expect("the account");
            let sums = lb_store::balances::sums(c, None, None)?;
            let raw = sums.get(&a.id).copied().unwrap_or(lb_core::Money::ZERO);
            Ok(a.account_type.natural(raw))
        })
        .unwrap()
        .minor()
}

#[tokio::test]
async fn a_web_connector_syncs_and_checks_parity_through_the_server() {
    let s = server();
    let c = s.company("Larkspur Labs").await;
    let base = format!("/api/v1/c/{c}/import/qbd");

    // Fresh company: bridge by default, nothing set, no job.
    let (st, v) = s.call("GET", &base, None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["settings"]["transport"], "bridge");
    assert_eq!(v["qbwc_password_set"], false);
    assert_eq!(v["running"], Value::Null);
    assert_eq!(v["sdk_available"], cfg!(all(windows, feature = "qbsdk")));
    let owner_id = v["settings"]["qbwc"]["owner_id"].clone();

    // Switch to the Web Connector.
    let (st, v) = s
        .call(
            "PUT",
            &base,
            Some(json!({
                "transport": "qbwc",
                "qbwc": { "user_name": "larkspur", "app_url": "https://books.example.com", "run_every_minutes": 30 },
                "company_file": r"C:\Books\Larkspur.QBW",
            })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["settings"]["transport"], "qbwc");
    assert_eq!(
        v["settings"]["qbwc"]["owner_id"], owner_id,
        "the .qwc ids never rotate"
    );

    // A generated password, shown once.
    let (st, v) = s.call("POST", &format!("{base}/qbwc/password"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let password = v["password"].as_str().unwrap().to_owned();
    assert!(password.len() >= 32);
    let (_, v) = s.call("GET", &base, None).await;
    assert_eq!(v["qbwc_password_set"], true);
    assert!(
        !v.to_string().contains(&password),
        "the password is never shown again"
    );

    // The .qwc file.
    let r = s.raw("GET", &format!("{base}/qbwc.qwc"), None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let qwc = r.text();
    assert!(
        qwc.contains("<AppURL>https://books.example.com/qbwc</AppURL>"),
        "{qwc}"
    );
    assert!(qwc.contains("<AppSupport>https://books.example.com/</AppSupport>"));
    assert!(qwc.contains("<UserName>larkspur</UserName>"));
    assert!(qwc.contains("<IsReadOnly>true</IsReadOnly>"));
    assert!(qwc.contains("<RunEveryNMinutes>30</RunEveryNMinutes>"));
    assert!(qwc.contains(&format!(
        "<OwnerID>{{{}}}</OwnerID>",
        owner_id.as_str().unwrap().to_uppercase()
    )));
    assert!(
        r.headers["content-disposition"]
            .to_str()
            .unwrap()
            .contains("librebooks-larkspur-labs.qwc")
    );

    let wc = WebConnector {
        server: &s,
        quickbooks: FakeQuickBooks::new(larkspur()),
        user: "larkspur".into(),
        password,
        settle: Some(base.clone()),
    };
    // Wrong password: "nvu". Right password, nothing queued: "none".
    assert_eq!(wc.authenticate("wrong").await.1, "nvu");
    assert_eq!(wc.authenticate(&wc.password).await.1, "none");
    assert_eq!(
        WebConnector {
            user: "nobody".into(),
            ..wc.clone_with(&s)
        }
        .authenticate(&wc.password)
        .await
        .1,
        "nvu"
    );

    // Start a sync: it waits for the Web Connector.
    let (st, v) = s
        .call("POST", &format!("{base}/sync"), Some(json!({})))
        .await;
    assert_eq!(st, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v["running"]["kind"], "sync");
    assert_eq!(v["running"]["transport"], "qbwc");
    let (st, v) = s
        .call("POST", &format!("{base}/sync"), Some(json!({})))
        .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "running");
    let (st, v) = s.call("POST", &format!("{base}/parity"), None).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");

    // The Web Connector polls and runs the whole thing.
    let run = wc.update().await;
    assert_eq!(
        run.outcome, r"C:\Books\Larkspur.QBW",
        "open this company file"
    );
    assert_eq!(run.percents.last(), Some(&100), "{run:?}");
    assert!(
        run.percents.iter().all(|p| (1..=100).contains(p)),
        "{run:?}"
    );
    assert!(run.rounds > 25, "{run:?}");
    assert_eq!(run.last_error, "NoOp");
    assert!(run.summary.starts_with("LibreBooks: done"), "{run:?}");
    // Everything the Web Connector carried was a query, and paging ran out.
    let requests = wc.quickbooks.requests();
    assert!(
        requests.iter().all(|r| r.ends_with("QueryRq")),
        "{requests:?}"
    );
    assert!(requests.iter().any(|r| r == "ItemQueryRq"));
    assert_eq!(wc.quickbooks.open_iterators(), 0);

    // The report is there, the books are right, and nothing is running.
    let (st, v) = s.call("GET", &format!("{base}/status?wait=10"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["running"], Value::Null, "{v}");
    assert_eq!(v["last_error"], Value::Null, "{v}");
    let report = &v["last_sync"];
    assert_eq!(report["source_id"], "qbd:ein-000000042");
    assert_eq!(report["aborted"], Value::Null);
    let errors = report["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["severity"] == "Error")
        .count();
    assert_eq!(errors, 0, "{}", report["issues"]);
    assert_eq!(balance(&s, &c, "Checking"), 968_500);
    assert_eq!(balance(&s, &c, "Accounts Receivable"), 52_725);
    let reports_dir = s.dir.path().join("imports");
    assert!(std::fs::read_dir(&reports_dir).unwrap().count() >= 1);
    // Nothing more to do for the Web Connector.
    assert_eq!(wc.update().await.outcome, "none");

    // Parity goes through the same queue.
    let (st, v) = s
        .call(
            "POST",
            &format!("{base}/parity"),
            Some(json!({ "as_of": "2026-03-31" })),
        )
        .await;
    assert_eq!(st, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v["running"]["kind"], "parity");
    let run = wc.update().await;
    assert_eq!(run.percents.last(), Some(&100), "{run:?}");
    let (st, v) = s.call("GET", &format!("{base}/parity?wait=10"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["running"], false);
    assert!(
        v["report"]["summary"]["accounts_compared"]
            .as_u64()
            .unwrap()
            > 10,
        "{v}"
    );
    assert!(
        v["report"]["trial_balance"]["rows"]
            .as_array()
            .unwrap()
            .len()
            > 5
    );
    assert!(
        wc.quickbooks
            .requests()
            .iter()
            .any(|r| r == "GeneralSummaryReportQueryRq")
    );

    // A QuickBooks error on one list is reported, the rest still runs.
    wc.quickbooks
        .fail("CheckQueryRq", 3000, "The given object ID is invalid.");
    let (st, _) = s
        .call(
            "POST",
            &format!("{base}/sync"),
            Some(json!({ "full": true })),
        )
        .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let run = wc.update().await;
    assert_eq!(run.percents.last(), Some(&100), "{run:?}");
    let (_, v) = s.call("GET", &format!("{base}/status?wait=10"), None).await;
    let issues = v["last_sync"]["issues"].as_array().unwrap();
    assert!(
        issues.iter().any(|i| i["entity"] == "Check"
            && i["severity"] == "Error"
            && i["message"].as_str().unwrap().contains("3000")),
        "{issues:?}"
    );
    assert_eq!(v["last_sync"]["aborted"], Value::Null);
}

impl WebConnector<'_> {
    fn clone_with<'b>(&self, server: &'b Server) -> WebConnector<'b> {
        WebConnector {
            server,
            quickbooks: FakeQuickBooks::new(larkspur()),
            user: self.user.clone(),
            password: self.password.clone(),
            settle: None,
        }
    }
}

#[tokio::test]
async fn a_connection_error_mid_run_is_reported() {
    let s = server();
    let c = s.company("Larkspur Labs").await;
    let base = format!("/api/v1/c/{c}/import/qbd");
    let (st, _) = s
        .call(
            "PUT",
            &base,
            Some(json!({ "transport": "qbwc", "qbwc": { "user_name": "lark" } })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    let (_, v) = s
        .call(
            "POST",
            &format!("{base}/qbwc/password"),
            Some(json!({ "value": "correct horse battery" })),
        )
        .await;
    assert_eq!(v, Value::Null, "a given password isn't echoed");
    let wc = WebConnector {
        server: &s,
        quickbooks: FakeQuickBooks::new(larkspur()),
        user: "lark".into(),
        password: "correct horse battery".into(),
        settle: None,
    };
    let (st, _) = s.call("POST", &format!("{base}/sync"), None).await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let (ticket, outcome) = wc.authenticate(&wc.password).await;
    assert_eq!(outcome, "", "no company file configured: use the open one");
    // Another Web Connector run while this session is open is told "busy".
    assert_eq!(wc.authenticate(&wc.password).await.1, "busy");
    let (_, v) = s.call("GET", &base, None).await;
    assert_eq!(v["running"]["web_connector_connected"], true);

    // Answer two documents, then QuickBooks fails.
    for _ in 0..2 {
        let (_, v) = wc
            .soap(
                "sendRequestXML",
                &[
                    ("ticket", &ticket),
                    ("strHCPResponse", ""),
                    ("strCompanyFileName", ""),
                    ("qbXMLCountry", "US"),
                    ("qbXMLMajorVers", "13"),
                    ("qbXMLMinorVers", "0"),
                ],
            )
            .await;
        let request = result_text(&v);
        assert!(request.contains("QueryRq"), "{request}");
        let response = wc.quickbooks.answer(&request);
        let (_, v) = wc
            .soap(
                "receiveResponseXML",
                &[
                    ("ticket", &ticket),
                    ("response", &response),
                    ("hresult", ""),
                    ("message", ""),
                ],
            )
            .await;
        assert!(result_text(&v).parse::<i32>().unwrap() > 0);
    }
    let (_, v) = wc
        .soap(
            "sendRequestXML",
            &[
                ("ticket", &ticket),
                ("strHCPResponse", ""),
                ("strCompanyFileName", ""),
                ("qbXMLCountry", "US"),
                ("qbXMLMajorVers", "13"),
                ("qbXMLMinorVers", "0"),
            ],
        )
        .await;
    assert_ne!(result_text(&v), "");
    let (_, v) = wc
        .soap(
            "receiveResponseXML",
            &[
                ("ticket", &ticket),
                ("response", ""),
                ("hresult", "0x80040408"),
                ("message", "Could not start QuickBooks."),
            ],
        )
        .await;
    assert_eq!(result_text(&v), "-1");
    let (_, v) = wc.soap("getLastError", &[("ticket", &ticket)]).await;
    assert!(
        result_text(&v).contains("Could not start QuickBooks"),
        "{v}"
    );
    let (_, v) = wc.soap("closeConnection", &[("ticket", &ticket)]).await;
    assert!(result_text(&v).contains("stopped"), "{v}");
    // The ticket is gone; the job ended with the failure on record.
    let (_, v) = wc.soap("sendRequestXML", &[("ticket", &ticket)]).await;
    assert_eq!(result_text(&v), "");
    let (_, v) = s.call("GET", &format!("{base}/status?wait=10"), None).await;
    assert_eq!(v["running"], Value::Null, "{v}");
    let aborted = v["last_sync"]["aborted"].as_str().unwrap_or("").to_owned();
    let error = v["last_error"].as_str().unwrap_or("").to_owned();
    assert!(
        aborted.contains("Could not start QuickBooks")
            || error.contains("Could not start QuickBooks"),
        "aborted {aborted:?} error {error:?}"
    );
}

#[tokio::test]
async fn settings_are_checked_and_the_bridge_key_is_write_only() {
    let s = server();
    let c = s.company("Bridge Co").await;
    let base = format!("/api/v1/c/{c}/import/qbd");

    // The bridge needs a URL and a key.
    let (st, v) = s
        .call(
            "PUT",
            &base,
            Some(json!({ "transport": "bridge", "bridge": { "url": "" } })),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let problems = v["error"]["detail"]["problems"].as_array().unwrap();
    assert_eq!(problems.len(), 2, "{problems:?}");
    let (st, v) = s
        .call(
            "PUT",
            &base,
            Some(json!({
                "transport": "bridge",
                "bridge": { "url": "http://10.255.0.18:8743", "delay_ms": 50 },
                "bridge_key": "sk-live-ABCDEF0123456789",
            })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["bridge_key_set"], true);
    assert_eq!(
        v["settings"]["bridge"]["key_ref"],
        format!("secret:qbd.{c}.bridge-key")
    );
    assert!(
        !v.to_string().contains("sk-live"),
        "the key is never returned"
    );
    let (_, v) = s.call("GET", "/api/v1/server/secrets", None).await;
    assert!(
        v.as_array()
            .unwrap()
            .iter()
            .any(|e| e["name"] == format!("qbd.{c}.bridge-key"))
    );
    // Saving again without the key keeps it.
    let (st, v) = s
        .call(
            "PUT",
            &base,
            Some(json!({ "transport": "bridge", "bridge": { "url": "http://10.255.0.18:8743" } })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["bridge_key_set"], true);

    // A bridge sync can't start without reaching the bridge: the job fails and
    // says so (no bridge is listening on this port).
    let (st, v) = s
        .call(
            "PUT",
            &base,
            Some(json!({ "transport": "bridge", "bridge": { "url": "http://127.0.0.1:9", "delay_ms": 0 } })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, v) = s
        .call("POST", &format!("{base}/sync"), Some(json!({})))
        .await;
    assert_eq!(st, StatusCode::ACCEPTED, "{v}");
    let (_, v) = s.call("GET", &format!("{base}/status?wait=60"), None).await;
    assert_eq!(v["running"], Value::Null, "{v}");
    assert!(
        v["last_error"].as_str().unwrap_or("").contains("bridge"),
        "{v}"
    );

    // Web Connector user names are unique across companies; passwords have a minimum.
    let (st, _) = s
        .call(
            "PUT",
            &base,
            Some(json!({ "transport": "qbwc", "qbwc": { "user_name": "shared" } })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    let other = s.company("Other Co").await;
    let (st, v) = s
        .call(
            "PUT",
            &format!("/api/v1/c/{other}/import/qbd"),
            Some(json!({ "transport": "qbwc", "qbwc": { "user_name": "shared" } })),
        )
        .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "duplicate");
    let (st, _) = s
        .call(
            "POST",
            &format!("{base}/qbwc/password"),
            Some(json!({ "value": "short" })),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // The .qwc refuses a plain-HTTP LAN address (the Web Connector would too).
    let (st, _) = s
        .call(
            "PUT",
            &base,
            Some(json!({ "transport": "qbwc", "qbwc": { "user_name": "shared", "app_url": "http://10.0.0.5:8740" } })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    let r = s.raw("GET", &format!("{base}/qbwc.qwc"), None).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text());
    assert!(r.text().contains("https"));
    // Without an app URL the server's public URL is used.
    let (st, _) = s
        .call(
            "PUT",
            &base,
            Some(
                json!({ "transport": "qbwc", "qbwc": { "user_name": "shared", "app_url": null } }),
            ),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    let (_, view) = s.call("GET", "/api/v1/server", None).await;
    let mut settings = view["settings"].clone();
    settings["public_urls"] = json!(["https://books.example.com"]);
    let (st, v) = s.call("PUT", "/api/v1/server", Some(settings)).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let r = s.raw("GET", &format!("{base}/qbwc.qwc"), None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert!(
        r.text()
            .contains("<AppURL>https://books.example.com/qbwc</AppURL>")
    );

    // The SOAP endpoint answers a browser and refuses a non-SOAP body.
    let r = s.raw("GET", "/qbwc", None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.text().contains("Web Connector"));
    let req = Request::builder()
        .method("POST")
        .uri("/qbwc")
        .header("content-type", "text/xml");
    let r = s.send(req, Body::from("not xml")).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.text().contains("soap:Fault"));
    let req = Request::builder()
        .method("POST")
        .uri("/qbwc")
        .header("content-type", "text/xml");
    let r = s
        .send(
            req,
            Body::from("<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\"><soap:Body><serverVersion xmlns=\"http://developer.intuit.com/\"/></soap:Body></soap:Envelope>"),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.text().contains("<serverVersionResult>LibreBooks "));
}

#[tokio::test]
async fn import_routes_need_an_admin_on_the_company() {
    let mut s = Server::with_sessions(OpsOptions::default());
    s.cookie = Some(s.sign_in("Ada", Role::Admin).await);
    let c = s.company("Larkspur Labs").await;
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
    let base = format!("/api/v1/c/{c}/import/qbd");
    for (method, path, body) in [
        ("GET", base.clone(), None),
        ("PUT", base.clone(), Some(json!({ "transport": "qbwc" }))),
        ("POST", format!("{base}/sync"), None),
        ("POST", format!("{base}/parity"), None),
        ("GET", format!("{base}/qbwc.qwc"), None),
        ("POST", format!("{base}/qbwc/password"), None),
    ] {
        let (st, v) = s.call(method, &path, body).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{method} {path}: {v}");
    }
    s.cookie = None;
    let (st, _) = s.call("GET", &base, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

/// Twenty wrong passwords in ten minutes and the client is told "nvu" even
/// with the right one: `/qbwc` is reachable through the tunnel without a
/// session, so it gets the same brake as the passkey endpoints.
#[tokio::test]
async fn web_connector_sign_in_failures_are_rate_limited() {
    let s = server();
    let c = s.company("Larkspur Labs").await;
    let base = format!("/api/v1/c/{c}/import/qbd");
    let (st, v) = s
        .call(
            "PUT",
            &base,
            Some(json!({ "transport": "qbwc", "qbwc": { "user_name": "larkspur" } })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (_, v) = s.call("POST", &format!("{base}/qbwc/password"), None).await;
    let password = v["password"].as_str().unwrap().to_owned();
    let wc = WebConnector {
        server: &s,
        quickbooks: FakeQuickBooks::new(larkspur()),
        user: "larkspur".into(),
        password,
        settle: None,
    };
    assert_eq!(wc.authenticate(&wc.password).await.1, "none");
    for _ in 0..lb_auth::RateLimit::default().max_failures {
        assert_eq!(wc.authenticate("wrong").await.1, "nvu");
    }
    // The tests' requests carry no peer address, so they are all one client.
    assert_eq!(
        wc.authenticate(&wc.password).await.1,
        "nvu",
        "blocked after too many failures, right password or not"
    );
    // Housekeeping's prune changes nothing while the failures are fresh.
    s.state.imports.prune();
    assert_eq!(wc.authenticate(&wc.password).await.1, "nvu");
}
