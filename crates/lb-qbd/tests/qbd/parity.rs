//! The parity check against the fixture company: everything matches except the
//! accounts touched by what the bridge can't expose (an $89.99 card charge and
//! a $40 statement charge), and those say why.

use lb_core::Money;
use lb_qbd::parity::{ParityError, ParityOptions, parity};
use lb_qbd::sync::SyncOptions;
use pretty_assertions::assert_eq;

use crate::common::*;

fn row<'a>(rows: &'a [lb_qbd::ParityRow], name: &str) -> &'a lb_qbd::ParityRow {
    rows.iter()
        .find(|r| r.full_name == name)
        .unwrap_or_else(|| panic!("no row {name}"))
}

#[tokio::test]
async fn parity_finds_exactly_what_the_bridge_cannot_supply() {
    let src = larkspur();
    let b = books();
    assert!(matches!(
        parity(&b, &src, &ParityOptions::default()).await,
        Err(ParityError::NotSynced)
    ));
    run_clean(&b, &src, &SyncOptions::default()).await;
    let report = parity(
        &b,
        &src,
        &ParityOptions {
            as_of: Some(jiff::civil::date(2026, 3, 31)),
            trial_balance: true,
        },
    )
    .await
    .unwrap();

    // Account balances (QBD's all-time `Balance`, natural sign).
    let off: Vec<(&str, Money)> = report
        .accounts
        .iter()
        .filter(|r| !r.matches())
        .map(|r| (r.full_name.as_str(), r.difference))
        .collect();
    assert_eq!(
        off,
        vec![
            ("Visa x1234", m("-89.99")),
            ("Accounts Receivable", m("-40.00")),
            ("Sales:Engineering", m("-40.00")),
            // A parent with nothing of its own is off only in its total.
            ("Sales", Money::ZERO),
            ("Utilities", Money::ZERO),
            ("Utilities:Internet", m("-89.99")),
        ]
    );
    let visa = row(&report.accounts, "Visa x1234");
    assert!(
        visa.note
            .as_deref()
            .unwrap()
            .contains("credit card charges")
    );
    let sales = row(&report.accounts, "Sales");
    assert_eq!(
        (sales.qbd_total, sales.ours_total),
        (Some(m("2300.00")), Some(m("2260.00")))
    );
    let interest = row(&report.accounts, "Interest Income");
    assert_eq!((interest.qbd, interest.ours), (m("650.00"), m("650.00")));
    assert_eq!(report.summary.accounts_off, 6);
    assert_eq!(report.summary.total_abs_difference, m("259.98"));
    assert!(
        report
            .summary
            .notes
            .iter()
            .any(|n| n.contains("credit card"))
    );
    // Non-posting accounts aren't compared; nothing is LibreBooks-only.
    assert!(report.accounts.iter().all(|r| r.qbd_list_id.is_some()));

    // Trial balance as of 2026-03-31: P&L for the fiscal year, last year's net
    // income in Retained Earnings.
    let tb = report.trial_balance.unwrap();
    assert_eq!(tb.basis, "Accrual");
    let off: Vec<&str> = tb
        .rows
        .iter()
        .filter(|r| !r.difference.is_zero())
        .map(|r| r.full_name.as_str())
        .collect();
    assert_eq!(
        off,
        vec![
            "Accounts Receivable",
            "Visa x1234",
            "Sales:Engineering",
            "Utilities:Internet"
        ]
    );
    let re = row(&tb.rows, "Retained Earnings");
    assert_eq!((re.qbd, re.ours), (m("500.00"), m("500.00")));
    let interest = row(&tb.rows, "Interest Income");
    assert_eq!((interest.qbd, interest.ours), (m("150.00"), m("150.00")));
    assert_eq!(
        (tb.qbd_debits, tb.qbd_credits),
        (m("14548.24"), m("14548.24"))
    );
    assert_eq!(
        (tb.ours_debits, tb.ours_credits),
        (m("14418.25"), m("14418.25"))
    );
    assert_eq!(report.summary.trial_balance_matching, 13);
}
