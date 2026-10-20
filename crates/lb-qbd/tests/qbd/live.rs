//! A light smoke test against the real QB Bridge (GET only, by construction).
//! Never runs by default. It reads the user's live QuickBooks, so it is kept
//! small: every list (they're short), at most three of each transaction type,
//! into a throwaway in-memory company, then the parity check. About 65 requests.
//!
//! ```text
//! LB_QBD_LIVE_URL=http://10.255.0.18:8743 \
//! LB_QBD_LIVE_KEY_FILE="…/Receipts/.env" \
//!   cargo test -p lb-qbd --test qbd live -- --ignored --nocapture
//! ```
//!
//! It prints counts and messages only to the console; nothing is written to
//! disk.

use lb_qbd::client::{BridgeClient, BridgeConfig, read_env_value};
use lb_qbd::parity::{ParityOptions, parity};
use lb_qbd::sync::{Severity, SyncOptions};

use crate::common::*;

#[tokio::test]
#[ignore = "talks to the user's live QuickBooks; run by hand"]
async fn live_bridge_smoke() {
    let (Ok(url), Ok(key_file)) = (
        std::env::var("LB_QBD_LIVE_URL"),
        std::env::var("LB_QBD_LIVE_KEY_FILE"),
    ) else {
        eprintln!("set LB_QBD_LIVE_URL and LB_QBD_LIVE_KEY_FILE to run this");
        return;
    };
    let key = read_env_value(std::path::Path::new(&key_file), "QB_API_KEY")
        .unwrap()
        .expect("QB_API_KEY in the key file");
    let client = BridgeClient::new(BridgeConfig::new(&url, key).unwrap()).unwrap();
    let b = books();
    let report = run(
        &b,
        &client,
        &SyncOptions {
            txn_limit: Some(3),
            ..SyncOptions::default()
        },
    )
    .await;
    println!("{}", report.summary());
    for i in &report.issues {
        if i.severity != Severity::Info {
            println!(
                "{:?} {:?} {}: {}",
                i.severity, i.entity, i.record, i.message
            );
        }
    }
    println!("mismatches: {}", report.mismatches.len());
    for mm in &report.mismatches {
        println!("  {:?} {} {:?}", mm.entity, mm.record, mm.kind);
    }
    println!("unmapped fields: {:?}", report.unmapped_fields);
    println!("created lists: {}", report.created_lists.len());
    println!("requests so far: {:?}", report.requests);
    assert_eq!(b.check_integrity().unwrap(), Vec::<String>::new());

    let p = parity(&b, &client, &ParityOptions::default())
        .await
        .unwrap();
    println!(
        "parity: {} accounts, {} match, {} off; trial balance rows {} ({} match)",
        p.summary.accounts_compared,
        p.summary.accounts_matching,
        p.summary.accounts_off,
        p.summary.trial_balance_rows,
        p.summary.trial_balance_matching
    );
    println!(
        "total requests: {:?}",
        lb_qbd::QbdSource::requests_made(&client)
    );
}
