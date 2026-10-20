//! The qbXML transports end to end: the fixture company served as qbXML by a
//! fake QuickBooks, through `QbxmlSource` (what the Web Connector and SDK share),
//! imports to exactly the books the bridge fixtures give; and the Web
//! Connector's queue, driven the way the Web Connector drives it, does too.

use std::sync::Arc;
use std::time::Duration;

use lb_qbd::parity::{ParityOptions, parity};
use lb_qbd::qbwc::{QueueConfig, Session, SessionConfig, queued_source};
use lb_qbd::sync::{Severity, SyncError, SyncOptions};
use lb_qbd::testing::FakeQuickBooks;
use lb_qbd::{QbdEntity, QbdError, QbdSource, QbxmlSource};
use lb_store::items;
use pretty_assertions::assert_eq;
use serde_json::json;

use crate::common::*;

fn qbxml_source(page: usize) -> QbxmlSource<FakeQuickBooks> {
    QbxmlSource::new(FakeQuickBooks::new(larkspur())).with_page_size(page)
}

#[tokio::test]
async fn qbxml_imports_the_same_books_as_the_bridge_in_far_fewer_requests() {
    // Through the bridge fixtures (one GET per transaction)…
    let bridge = larkspur();
    let b1 = books();
    let r1 = run_clean(&b1, &bridge, &SyncOptions::default()).await;
    // …and through qbXML with tiny pages, so iterators have to continue.
    let src = qbxml_source(3);
    let b2 = books();
    let r2 = run_clean(&b2, &src, &SyncOptions::default()).await;

    for name in [
        "Checking",
        "Accounts Receivable",
        "Opening Balance Equity",
        "Sales:Engineering",
        "Sales:Hardware",
        "Discounts Given",
        "Interest Income",
        "Rent",
    ] {
        assert_eq!(balance(&b2, name), balance(&b1, name), "{name}");
    }
    for e in &r1.entities {
        let other = r2.entity(e.entity).expect("same entities");
        assert_eq!(
            (other.listed, other.created),
            (e.listed, e.created),
            "{:?}",
            e.entity
        );
    }
    assert_eq!(r2.mismatches, r1.mismatches);
    // Lines came with the lists: no per-transaction fetches.
    let qbxml = r2.requests.unwrap();
    let via_bridge = r1.requests.unwrap();
    assert!(qbxml < via_bridge, "qbXML {qbxml} vs bridge {via_bridge}");
    let fake = src.transport();
    assert_eq!(fake.open_iterators(), 0, "every iterator ran to its end");
    assert!(fake.requests().iter().any(|r| r == "InvoiceQueryRq"));
    // Item types came from the element names, not the typed lists.
    assert!(!fake.requests().iter().any(|r| r == "ItemServiceQueryRq"));
    let types = |b: &lb_store::Books| {
        let mut v: Vec<(String, String)> = b
            .read(|c| items::list(c, None, true))
            .unwrap()
            .into_iter()
            .map(|i| (i.full_name, format!("{:?}", i.item_type)))
            .collect();
        v.sort();
        v
    };
    assert_eq!(types(&b2), types(&b1));
    // Same records, same lines (checked on one with several lines).
    let a = txn(&b1, QbdEntity::Check, "5010-1700006100");
    let b = txn(&b2, QbdEntity::Check, "5010-1700006100");
    assert_eq!(a.lines.len(), b.lines.len());
    assert_eq!(a.amount, b.amount);
}

#[tokio::test]
async fn incremental_sync_and_single_record_fetches_over_qbxml() {
    let src = qbxml_source(50);
    let b = books();
    run_clean(&b, &src, &SyncOptions::default()).await;
    let before = src.requests_made().unwrap();

    // Edit a check in "QuickBooks": only it is listed and re-imported.
    src.transport()
        .fixtures()
        .update(QbdEntity::Check, "5010-1700006100", |c| {
            c["Memo"] = json!("edited in QuickBooks");
            c["EditSequence"] = json!("1700009999");
            c["TimeModified"] = json!("2026-04-02T09:00:00-08:00");
        });
    let report = run_clean(&b, &src, &SyncOptions::default()).await;
    let checks = report.entity(QbdEntity::Check).unwrap();
    // The watermark is inclusive, so checks modified at the same instant as
    // the newest one are listed again and skipped by EditSequence.
    assert_eq!((checks.updated, checks.created), (1, 0));
    assert!(
        checks.listed >= 1 && checks.listed <= 4,
        "{}",
        checks.listed
    );
    assert_eq!(
        txn(&b, QbdEntity::Check, "5010-1700006100").memo.as_deref(),
        Some("edited in QuickBooks")
    );
    assert!(src.requests_made().unwrap() - before < 40);

    // A record the list didn't carry is fetched by id; a missing one is "not found".
    let got = src.get(QbdEntity::Invoice, "1A-1700003000").await;
    assert!(
        got.is_ok()
            || src
                .transport()
                .fixtures()
                .get_now(QbdEntity::Invoice, "1A-1700003000", true)
                .is_none()
    );
    let missing = src.get(QbdEntity::Check, "no-such-txn").await.unwrap_err();
    assert!(missing.is_not_found(), "{missing}");
    assert!(!missing.is_transient());
}

#[tokio::test]
async fn quickbooks_errors_are_reported_and_busy_statuses_stop_the_run() {
    let src = qbxml_source(50);
    src.transport().fail(
        "CheckQueryRq",
        3000,
        "The given object ID in the field \"list id\" is invalid.",
    );
    let b = books();
    let report = run(&b, &src, &SyncOptions::default()).await;
    let issue = report
        .issues
        .iter()
        .find(|i| i.entity == QbdEntity::Check && i.severity == Severity::Error)
        .expect("the check list failed");
    assert!(issue.message.contains("status 3000"), "{}", issue.message);
    assert!(
        report.aborted.is_none(),
        "one bad list doesn't stop the rest"
    );
    assert!(report.entity(QbdEntity::Deposit).unwrap().created > 0);

    // "QuickBooks is busy" is transient: the run stops and says so.
    let src = qbxml_source(50);
    src.transport()
        .fail("AccountQueryRq", 0x8004_0408, "Could not start QuickBooks.");
    let b = books();
    let report = run(&b, &src, &SyncOptions::default()).await;
    assert!(
        report.aborted.as_deref().unwrap_or("").contains("Account"),
        "{:?}",
        report.aborted
    );

    // An unparseable document is refused before it reaches QuickBooks.
    let e = lb_qbd::qbxml::request::check_read_only(
        "<QBXML><QBXMLMsgsRq><CheckAddRq/></QBXMLMsgsRq></QBXML>",
    )
    .unwrap_err();
    assert!(e.to_string().contains("only ever queries"));
}

#[tokio::test]
async fn parity_over_qbxml_reads_the_trial_balance_report() {
    let src = qbxml_source(50);
    let b = books();
    run_clean(&b, &src, &SyncOptions::default()).await;
    let via_qbxml = parity(
        &b,
        &src,
        &ParityOptions {
            as_of: Some(jiff::civil::date(2026, 3, 31)),
            trial_balance: true,
        },
    )
    .await
    .unwrap();
    let via_bridge = parity(
        &b,
        &larkspur(),
        &ParityOptions {
            as_of: Some(jiff::civil::date(2026, 3, 31)),
            trial_balance: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(via_qbxml.summary, via_bridge.summary);
    assert_eq!(via_qbxml.accounts, via_bridge.accounts);
    let tb = via_qbxml.trial_balance.unwrap();
    assert_eq!(tb.rows.len(), via_bridge.trial_balance.unwrap().rows.len());
    assert!(
        src.transport()
            .requests()
            .iter()
            .any(|r| r == "GeneralSummaryReportQueryRq")
    );
}

/// Drive the queue exactly as the Web Connector drives the SOAP endpoint, with
/// the fake QuickBooks running each document.
#[tokio::test]
async fn the_web_connector_queue_runs_a_whole_sync() {
    let quickbooks = Arc::new(FakeQuickBooks::new(larkspur()));
    let (source, handle) = queued_source(QueueConfig {
        first_poll_wait: Duration::from_secs(5),
        in_session_wait: Duration::from_secs(5),
    });
    let source = source.with_page_size(4);
    let b = books();
    let engine = tokio::spawn({
        let b = b.clone();
        let handle = handle.clone();
        async move {
            let report = lb_qbd::sync(&b, &source, &SyncOptions::default(), &|p| {
                handle.set_progress(p);
            })
            .await
            .expect("sync runs");
            drop(source);
            report
        }
    });

    // The Web Connector's loop.
    let session = Session::new(
        handle.clone(),
        SessionConfig {
            poll_wait: Duration::from_secs(5),
            settle_wait: Duration::from_millis(500),
        },
    );
    let mut rounds: u64 = 0;
    let mut heartbeats: u64 = 0;
    let mut last_percent = 0;
    loop {
        let request = session.send_request_xml().await;
        if request.is_empty() {
            break;
        }
        rounds += 1;
        assert!(rounds < 500, "the loop should end");
        if request.contains("HostQueryRq") {
            heartbeats += 1;
        }
        let response = quickbooks.answer(&request);
        let percent = session.receive_response_xml(response, "", "").await;
        assert!((1..=100).contains(&percent), "{percent}");
        last_percent = percent;
        if percent == 100 {
            break;
        }
    }
    assert_eq!(
        last_percent, 100,
        "the normal ending is 100 → closeConnection"
    );
    assert_eq!(session.get_last_error(), "NoOp");
    let summary = session.close_connection();
    assert!(summary.starts_with("LibreBooks: done"), "{summary}");
    let report = engine.await.unwrap();
    assert!(report.errors().next().is_none(), "{}", report.summary());
    assert_eq!(balance(&b, "Checking"), m("9685.00"));
    assert_eq!(balance(&b, "Accounts Receivable"), m("527.25"));
    assert!(rounds >= 20, "{rounds} rounds");
    assert_eq!(handle.answered() + heartbeats, rounds);
}

#[tokio::test]
async fn a_web_connector_that_never_comes_ends_the_sync_cleanly() {
    let (source, handle) = queued_source(QueueConfig {
        first_poll_wait: Duration::from_millis(100),
        in_session_wait: Duration::from_millis(100),
    });
    let b = books();
    // The very first question (which company is this?) never gets an answer,
    // so the sync fails outright with the reason, rather than hanging.
    let err = lb_qbd::sync(&b, &source, &SyncOptions::default(), &|_| {})
        .await
        .unwrap_err();
    assert!(
        matches!(err, SyncError::Bridge(QbdError::Timeout { .. })),
        "{err}"
    );
    assert!(handle.is_closed());
    assert!(
        handle.last_error().unwrap().contains("Web Connector"),
        "{:?}",
        handle.last_error()
    );
}

#[tokio::test]
async fn a_connection_error_mid_run_aborts_the_sync() {
    let quickbooks = Arc::new(FakeQuickBooks::new(larkspur()));
    let (source, handle) = queued_source(QueueConfig {
        first_poll_wait: Duration::from_secs(5),
        in_session_wait: Duration::from_secs(5),
    });
    let b = books();
    let engine = tokio::spawn({
        let b = b.clone();
        async move {
            let r = lb_qbd::sync(&b, &source, &SyncOptions::default(), &|_| {}).await;
            drop(source);
            r
        }
    });
    let session = Session::new(
        handle,
        SessionConfig {
            poll_wait: Duration::from_secs(5),
            settle_wait: Duration::from_millis(200),
        },
    );
    // Answer a few, then QuickBooks goes away.
    for _ in 0..3 {
        let request = session.send_request_xml().await;
        assert_ne!(request, "");
        session
            .receive_response_xml(quickbooks.answer(&request), "", "")
            .await;
    }
    let request = session.send_request_xml().await;
    assert_ne!(request, "");
    assert_eq!(
        session
            .receive_response_xml(String::new(), "0x80040408", "Could not start QuickBooks.")
            .await,
        -1
    );
    assert_eq!(
        session.connection_error("0x80040408", "Could not start QuickBooks."),
        "done"
    );
    assert!(
        session
            .get_last_error()
            .contains("Could not start QuickBooks")
    );
    let report = engine
        .await
        .unwrap()
        .expect("the engine reports, not fails");
    assert!(report.aborted.is_some(), "{}", report.summary());
    assert_eq!(session.send_request_xml().await, "");
    assert!(session.close_connection().contains("stopped"));
}
