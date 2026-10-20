//! The bridge client against a mock bridge: an in-process axum server that
//! behaves like the real one (envelope, FastAPI-style errors, iterators where
//! supported, `ITERATOR_NOT_SUPPORTED` where not, `max_returned` caps) over the
//! fixture company. Every request it sees is logged, and every test checks that
//! nothing but GET was ever sent.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use lb_qbd::QbdEntity;
use lb_qbd::client::{Active, BridgeClient, BridgeConfig, ListQuery, QbdSource, ReportQuery};
use lb_qbd::parity::{ParityOptions, parity};
use lb_qbd::sync::SyncOptions;
use lb_qbd::testing::FixtureSource;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use crate::common::*;

const KEY: &str = "test-key-not-real";

#[derive(Clone, Debug)]
struct Logged {
    method: Method,
    path: String,
    query: HashMap<String, String>,
    key: Option<String>,
}

#[derive(Default)]
struct MockState {
    fixture: FixtureSource,
    log: Mutex<Vec<Logged>>,
    iterators: Mutex<HashMap<String, VecDeque<Value>>>,
    next_iterator: AtomicU32,
    /// Lists that refuse iterators beyond the ones the real bridge refuses.
    refuse_iterator: Vec<QbdEntity>,
    /// Answer `/company` with 503 this many times first.
    company_failures: Mutex<u32>,
}

#[derive(Clone)]
struct Mock(Arc<MockState>);

impl Mock {
    fn new(fixture: FixtureSource) -> Mock {
        Mock(Arc::new(MockState {
            fixture,
            ..MockState::default()
        }))
    }

    fn log(&self) -> Vec<Logged> {
        self.0.log.lock().unwrap().clone()
    }

    fn assert_only_gets(&self) {
        let log = self.log();
        assert!(!log.is_empty());
        for l in &log {
            assert_eq!(l.method, Method::GET, "{} {}", l.method, l.path);
            assert_eq!(l.key.as_deref(), Some(KEY), "{}", l.path);
        }
    }
}

fn ok(data: Value, meta: Option<Value>) -> Response {
    let mut body = json!({"ok": true});
    body["data"] = data;
    if let Some(m) = meta {
        body["meta"] = m;
    }
    (StatusCode::OK, axum::Json(body)).into_response()
}

fn err(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        axum::Json(json!({"detail": {"ok": false, "error": {"code": code, "message": message}}})),
    )
        .into_response()
}

async fn handle(
    State(mock): State<Mock>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let query: HashMap<String, String> = uri
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .into_owned()
                .collect()
        })
        .unwrap_or_default();
    let path = uri.path().to_owned();
    mock.0.log.lock().unwrap().push(Logged {
        method: method.clone(),
        path: path.clone(),
        query: query.clone(),
        key: headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
    });
    if method != Method::GET {
        return err(StatusCode::METHOD_NOT_ALLOWED, "NO", "read-only mock");
    }
    if headers.get("x-api-key").and_then(|v| v.to_str().ok()) != Some(KEY) {
        return err(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "bad key");
    }
    let f = &mock.0.fixture;
    match path.as_str() {
        "/api/v1/company" => {
            {
                let mut left = mock.0.company_failures.lock().unwrap();
                if *left > 0 {
                    *left -= 1;
                    return (StatusCode::SERVICE_UNAVAILABLE, "QuickBooks is busy").into_response();
                }
            }
            return ok(f.company().await.unwrap(), None);
        }
        "/api/v1/company/preferences" => return ok(f.preferences().await.unwrap(), None),
        _ => {}
    }
    if let Some(slug) = path.strip_prefix("/api/v1/reports/") {
        return match f.report(slug, &ReportQuery::default()).await {
            Ok(r) => ok(r, None),
            Err(_) => err(StatusCode::NOT_FOUND, "NOT_FOUND", "no such report"),
        };
    }
    if let Some(entity) = QbdEntity::ALL.into_iter().find(|e| e.path() == path) {
        return list(&mock, entity, &query);
    }
    // GET {path}/{id}: the longest entity path that prefixes it.
    let found = QbdEntity::ALL
        .into_iter()
        .filter(|e| path.starts_with(&format!("{}/", e.path())))
        .max_by_key(|e| e.path().len());
    if let Some(entity) = found {
        let id = &path[entity.path().len() + 1..];
        return match f.get(entity, id).await {
            Ok(v) => ok(v, None),
            Err(_) => err(
                StatusCode::NOT_FOUND,
                "NOT_FOUND",
                &format!("{} not found", entity.name()),
            ),
        };
    }
    (
        StatusCode::NOT_FOUND,
        axum::Json(json!({"detail": "Not Found"})),
    )
        .into_response()
}

fn list(mock: &Mock, entity: QbdEntity, q: &HashMap<String, String>) -> Response {
    let max: usize = q
        .get("max_returned")
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    if let Some(it) = q.get("iterator_id") {
        if !entity.supports_iterator() || mock.0.refuse_iterator.contains(&entity) {
            return err(
                StatusCode::BAD_REQUEST,
                "ITERATOR_NOT_SUPPORTED",
                &format!("{} does not support iterator pagination.", entity.name()),
            );
        }
        let mut iterators = mock.0.iterators.lock().unwrap();
        let id = if it == "Start" {
            let rows = mock.0.fixture.list_now(entity, &query_of(q));
            let id = format!(
                "{{it-{}}}",
                mock.0.next_iterator.fetch_add(1, Ordering::SeqCst)
            );
            iterators.insert(id.clone(), rows.into());
            id
        } else {
            it.clone()
        };
        let Some(rows) = iterators.get_mut(&id) else {
            return err(StatusCode::BAD_REQUEST, "BAD_ITERATOR", "unknown iterator");
        };
        let page: Vec<Value> = (0..max).map_while(|_| rows.pop_front()).collect();
        let remaining = rows.len();
        return ok(
            Value::Array(page.clone()),
            Some(json!({"count": page.len(), "iterator_id": id, "remaining": remaining})),
        );
    }
    let mut rows = mock.0.fixture.list_now(entity, &query_of(q));
    rows.truncate(max);
    let n = rows.len();
    ok(Value::Array(rows), Some(json!({"count": n})))
}

fn query_of(q: &HashMap<String, String>) -> ListQuery {
    ListQuery {
        modified_after: q.get("modified_after").cloned(),
        from_date: q.get("from_date").and_then(|s| s.parse().ok()),
        to_date: q.get("to_date").and_then(|s| s.parse().ok()),
        active: match q.get("active").map(String::as_str) {
            Some("All") => Active::All,
            Some("InactiveOnly") => Active::InactiveOnly,
            // The real bridge's default.
            _ => Active::ActiveOnly,
        },
        ..ListQuery::default()
    }
}

async fn start(mock: Mock) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().fallback(handle).with_state(mock);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn client(base: &str, key: &str) -> BridgeClient {
    let mut cfg = BridgeConfig::new(base, key).unwrap();
    cfg.politeness_delay = Duration::ZERO;
    cfg.retry_delay = Duration::from_millis(5);
    cfg.page_size = 3;
    cfg.timeout = Duration::from_secs(20);
    BridgeClient::new(cfg).unwrap()
}

#[tokio::test]
async fn lists_page_through_iterators_and_details_come_whole() {
    let mock = Mock::new(larkspur());
    let c = client(&start(mock.clone()).await, KEY);
    let checks = c
        .list(
            QbdEntity::Check,
            &ListQuery {
                from_date: Some(jiff::civil::date(2026, 2, 1)),
                ..ListQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(checks.len(), 4);
    assert!(checks.iter().all(|c| c.get("ExpenseLineRet").is_none()));
    let log = mock.log();
    assert_eq!(log.len(), 2, "two pages of three");
    assert_eq!(
        log[0].query.get("iterator_id").map(String::as_str),
        Some("Start")
    );
    assert_eq!(
        log[0].query.get("from_date").map(String::as_str),
        Some("2026-02-01")
    );
    assert_eq!(
        log[0].query.get("max_returned").map(String::as_str),
        Some("3")
    );
    assert_eq!(
        log[1].query.get("iterator_id").map(String::as_str),
        Some("{it-0}")
    );
    // Transactions don't take `active`; lists always ask for All.
    assert!(!log[0].query.contains_key("active"));

    let one = c.get(QbdEntity::Check, "5010-1700006100").await.unwrap();
    assert!(one.get("ItemGroupLineRet").is_some());
    let missing = c.get(QbdEntity::Check, "0-0").await.unwrap_err();
    assert!(missing.is_not_found());
    assert_eq!(missing.code(), Some("NOT_FOUND"));
    assert_eq!(c.requests_made(), Some(4));
    mock.assert_only_gets();
}

#[tokio::test]
async fn lists_that_cannot_iterate_come_in_one_call() {
    let mock = Mock(Arc::new(MockState {
        fixture: larkspur(),
        refuse_iterator: vec![QbdEntity::Customer],
        ..MockState::default()
    }));
    let c = client(&start(mock.clone()).await, KEY);
    // Known not to iterate: one call, everything, inactive included.
    let accounts = c
        .list(QbdEntity::Account, &ListQuery::default())
        .await
        .unwrap();
    assert_eq!(accounts.len(), 24);
    let log = mock.log();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].query.get("active").map(String::as_str), Some("All"));
    assert_eq!(
        log[0].query.get("max_returned").map(String::as_str),
        Some("5000")
    );
    assert!(!log[0].query.contains_key("iterator_id"));
    // Learned at run time: the first list costs one refused request, the next none.
    assert_eq!(
        c.list(QbdEntity::Customer, &ListQuery::default())
            .await
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        c.list(QbdEntity::Customer, &ListQuery::default())
            .await
            .unwrap()
            .len(),
        4
    );
    assert_eq!(mock.log().len(), 1 + 2 + 1);
    mock.assert_only_gets();
}

#[tokio::test]
async fn a_full_transaction_list_is_split_by_date() {
    let fixture = FixtureSource::new(json!({}), json!({}));
    let start_day = jiff::civil::date(2020, 1, 1);
    let rows: Vec<Value> = (0..6000)
        .map(|i| {
            let d = start_day + jiff::Span::new().days(i64::from(i / 10));
            json!({"TxnID": format!("{i:X}-1"), "TxnDate": d.to_string(), "Duration": "PT1H0M"})
        })
        .collect();
    fixture.set_records(QbdEntity::TimeTracking, rows);
    let mock = Mock::new(fixture);
    let c = client(&start(mock.clone()).await, KEY);
    let got = c
        .list(QbdEntity::TimeTracking, &ListQuery::default())
        .await
        .unwrap();
    assert_eq!(got.len(), 6000);
    let ids: std::collections::HashSet<&str> =
        got.iter().map(|r| r["TxnID"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 6000, "no duplicates across date windows");
    assert!(mock.log().len() >= 3);
    mock.assert_only_gets();
}

#[tokio::test]
async fn transient_failures_are_retried_and_refusals_are_not() {
    let mock = Mock::new(larkspur());
    *mock.0.company_failures.lock().unwrap() = 2;
    let base = start(mock.clone()).await;
    let c = client(&base, KEY);
    let company = c.company().await.unwrap();
    assert_eq!(company["CompanyName"], "Larkspur Robotics");
    assert_eq!(mock.log().len(), 3);

    let wrong = client(&base, "wrong-key");
    let e = wrong.company().await.unwrap_err();
    assert!(!e.is_transient());
    assert!(e.to_string().contains("HTTP 401"), "{e}");
    assert_eq!(mock.log().len(), 4, "a refusal isn't retried");
}

#[tokio::test]
async fn sync_and_parity_run_end_to_end_over_http() {
    let mock = Mock::new(larkspur());
    let c = client(&start(mock.clone()).await, KEY);
    let b = books();
    let report = run_clean(&b, &c, &SyncOptions::default()).await;
    assert_eq!(balance(&b, "Checking"), m("9685.00"));
    assert_eq!(balance(&b, "Accounts Receivable"), m("527.25"));
    assert_eq!(report.mismatches.len(), 1);
    assert!(report.requests.unwrap() > 20);

    // A second sync lists, finds nothing changed, and fetches no details.
    let before = mock.log().len();
    run_clean(&b, &c, &SyncOptions::default()).await;
    let details = mock.log()[before..]
        .iter()
        .filter(|l| {
            QbdEntity::ALL
                .iter()
                .any(|e| l.path.starts_with(&format!("{}/", e.path())) && e.needs_detail())
        })
        .count();
    assert_eq!(details, 0);

    let p = parity(
        &b,
        &c,
        &ParityOptions {
            as_of: Some(jiff::civil::date(2026, 3, 31)),
            trial_balance: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(p.summary.accounts_off, 6);
    let tb_request = mock
        .log()
        .into_iter()
        .find(|l| l.path == "/api/v1/reports/trial-balance")
        .unwrap();
    assert_eq!(
        tb_request.query.get("basis").map(String::as_str),
        Some("Accrual")
    );
    assert_eq!(
        tb_request.query.get("to_date").map(String::as_str),
        Some("2026-03-31")
    );
    mock.assert_only_gets();
}
