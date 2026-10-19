//! Full and incremental syncs of the Larkspur fixture company.
//!
//! The books, worked out by hand from the fixture transactions (debit-positive,
//! then shown in each account's natural sign):
//!
//! | Account | How | Balance |
//! | --- | --- | --- |
//! | Checking | 10000 + 500 (JEs) + 1450 + 295 (deposits) + 40 (payment) + 25 (sales receipt) − 2000 − 455 − 50 (checks) − 120 (bill payment) | 9,685.00 |
//! | Accounts Receivable | invoices 1530 + 225 + 310 + 258.25, JE 75, credit memo −130, payments −1400 − 295 − 40, discount −6 | 527.25 |
//! | Opening Balance Equity | 10000 + 1500 | 11,500.00 |
//! | Sales:Engineering | 1500 + 100 + 225 + 300 + 150 − 130 | 2,145.00 |
//! | Sales:Hardware | 50 + 50 + 10 + 100 + 25 − 120 (cable bought on a check, posted to the item's only account) | 115.00 |
//! | Discounts Given | −170 (invoice) − 6 (payment discount) | −176.00 |
//! | Interest Income | 500 + 75 (JE) + 75 (deposit line) | 650.00 |
//!
//! QBD's own figures (the fixture `Balance`s) differ in exactly two places, both
//! things the bridge can't expose: an $89.99 credit card charge (Visa, and
//! Utilities:Internet where it was spent) and a $40 statement charge paid by a
//! payment (A/R, and Sales:Engineering where it was earned).

use lb_core::{LineKind, Money};
use lb_qbd::QbdEntity;
use lb_qbd::sync::{MismatchKind, Severity, SyncError, SyncOptions, watermark_key};
use pretty_assertions::assert_eq;
use serde_json::json;

use crate::common::*;

#[tokio::test]
async fn full_import_gives_the_hand_computed_books() {
    let src = larkspur();
    let b = books();
    let report = run_clean(&b, &src, &SyncOptions::default()).await;
    assert_eq!(report.source_id, "qbd:ein-000000042");

    for (name, want) in [
        ("Checking", "9685.00"),
        ("Visa x1234", "0.00"),
        ("Accounts Receivable", "527.25"),
        ("Undeposited Funds", "0.00"),
        ("Equipment:Computers", "1200.00"),
        ("Accounts Payable", "0.00"),
        ("Sales Tax Payable", "8.25"),
        ("Opening Balance Equity", "11500.00"),
        ("Retained Earnings", "0.00"),
        ("Sales:Engineering", "2145.00"),
        ("Sales:Hardware", "115.00"),
        ("Discounts Given", "-176.00"),
        ("Subcontractors", "240.00"),
        ("Rent", "2300.00"),
        ("Utilities", "120.00"),
        ("Utilities:Internet", "80.00"),
        ("Bank Service Charges", "40.00"),
        ("Payroll Expenses", "50.00"),
        ("Interest Income", "650.00"),
    ] {
        assert_eq!(balance(&b, name), m(want), "{name}");
    }
    assert_eq!(b.check_integrity().unwrap(), Vec::<String>::new());

    let counts = |e: QbdEntity| {
        let r = report.entity(e).unwrap();
        (
            r.listed, r.created, r.updated, r.skipped, r.failed, r.partial,
        )
    };
    // The five accounts every LibreBooks company starts with are adopted.
    assert_eq!(counts(QbdEntity::Account), (24, 19, 5, 0, 0, 0));
    assert_eq!(counts(QbdEntity::Customer), (4, 4, 0, 0, 0, 0));
    assert_eq!(counts(QbdEntity::Terms), (3, 3, 0, 0, 0, 0));
    assert_eq!(counts(QbdEntity::PaymentMethod), (2, 2, 0, 0, 0, 0));
    // The member-less reimbursable-expense group is skipped, with a reason.
    assert_eq!(counts(QbdEntity::Item), (9, 8, 0, 1, 0, 0));
    assert!(report.issues.iter().any(|i| i.severity == Severity::Warning
        && i.entity == QbdEntity::Item
        && i.message.contains("no member items")));
    for (e, n) in [
        (QbdEntity::JournalEntry, 4),
        (QbdEntity::Invoice, 4),
        (QbdEntity::CreditMemo, 1),
        (QbdEntity::ReceivePayment, 3),
        (QbdEntity::Deposit, 2),
        (QbdEntity::Check, 4),
        (QbdEntity::Bill, 1),
        (QbdEntity::BillPaymentCheck, 1),
        (QbdEntity::SalesReceipt, 1),
        (QbdEntity::Estimate, 1),
        (QbdEntity::PurchaseOrder, 1),
        (QbdEntity::TimeTracking, 1),
    ] {
        assert_eq!(counts(e), (n, n, 0, 0, 0, 0), "{e:?}");
    }

    // The only disagreement with QBD's own figures: the payment QBD applied to a
    // statement charge the bridge can't show us stays unapplied here.
    assert_eq!(report.mismatches.len(), 1, "{:#?}", report.mismatches);
    let mm = &report.mismatches[0];
    assert_eq!(
        (mm.entity, mm.kind, mm.qbd, mm.ours),
        (
            QbdEntity::ReceivePayment,
            MismatchKind::OpenBalance,
            Money::ZERO,
            m("40.00")
        )
    );
    let charge = report.gaps.iter().find(|g| g.qbd_type == "Charge").unwrap();
    assert_eq!(charge.links, 1);

    // Lists the bridge doesn't expose were created as transactions named them.
    for want in [
        "sales tax code \"Tax\" (taxable)",
        "sales tax code \"Non\" (non-taxable)",
        "CustomerType \"Commercial:Manufacturing\"",
        "JobType \"Automation\"",
        "VendorType \"Supplies\"",
        "sales tax item \"CA Sales Tax\" at 8.25%",
    ] {
        assert!(
            report.created_lists.iter().any(|c| c == want),
            "{want} not in {:?}",
            report.created_lists
        );
    }
    // Unmapped list fields are counted by name, not dropped silently.
    assert_eq!(report.unmapped_fields.get("Vendor.PriceLevelRef"), Some(&1));
    assert_eq!(
        report.unmapped_fields.get("Customer.ShipToAddress"),
        Some(&1)
    );
    assert!(
        !report
            .unmapped_fields
            .keys()
            .any(|k| k.contains("SSN") || k.contains("BirthDate"))
    );
}

#[tokio::test]
async fn company_info_and_names_come_across() {
    let src = larkspur();
    let b = books();
    run_clean(&b, &src, &SyncOptions::default()).await;
    let info = b.read(lb_store::company::info).unwrap();
    assert_eq!(info.name, "Larkspur Robotics");
    assert_eq!(info.legal_name.as_deref(), Some("Larkspur Robotics, Inc."));
    assert_eq!(info.address.lines, vec!["100 Example Way ", "Suite 5"]);
    assert!(info.preferences.use_class_tracking);
    assert!(info.preferences.reports_cash_basis);
    assert!(info.preferences.sales_tax_enabled);

    let job = b
        .read(|c| lb_store::entities::by_full_name(c, "Acme Widgets:Phase 1:Install"))
        .unwrap()
        .unwrap();
    assert_eq!(job.sublevel, 2);
    let acme = b
        .read(|c| lb_store::entities::by_full_name(c, "Acme Widgets"))
        .unwrap()
        .unwrap();
    assert_eq!(acme.custom_fields.get("Region"), Some(&json!("West")));
    assert_eq!(acme.credit_limit, Some(m("5000.00")));
    assert!(acme.terms_id.is_some() && acme.sales_rep_id.is_some() && acme.type_id.is_some());
    assert_eq!(acme.bill_address.note.as_deref(), Some("dock 4"));
    let mart = b
        .read(|c| lb_store::entities::by_full_name(c, "Office Mart"))
        .unwrap()
        .unwrap();
    assert_eq!(mart.mobile.as_deref(), Some("555-0199"));
    assert!(mart.custom_fields.contains_key("Contact: Pager"));
    let landlord = b
        .read(|c| lb_store::entities::by_full_name(c, "Landlord LLC"))
        .unwrap()
        .unwrap();
    assert!(landlord.is_1099);
    assert_eq!(landlord.print_as.as_deref(), Some("Landlord, LLC"));
    // Terms and a payment method share a ListID in QBD; each resolves to its own.
    let pmt = txn(&b, QbdEntity::ReceivePayment, "3000-1700004000");
    let method: lb_core::lists::PaymentMethod = b
        .read(|c| lb_store::lists::get(c, &pmt.payment_method_id.unwrap().to_string()))
        .unwrap();
    assert_eq!(method.name, "Check");
    let inv3 = txn(&b, QbdEntity::Invoice, "2020-1700003300");
    let terms: lb_core::lists::Terms = b
        .read(|c| lb_store::lists::get(c, &inv3.terms_id.unwrap().to_string()))
        .unwrap();
    assert_eq!(terms.name, "2% 10 Net 30");
    let item = b
        .read(|c| lb_store::items::by_full_name(c, "Shipping"))
        .unwrap()
        .unwrap();
    assert_eq!(item.item_type, lb_core::ItemType::OtherCharge);
    let rush = b
        .read(|c| lb_store::items::by_full_name(c, "Engineering:Rush"))
        .unwrap()
        .unwrap();
    assert_eq!(rush.item_type, lb_core::ItemType::Service);
}

#[tokio::test]
async fn lines_come_back_in_entry_order() {
    let src = larkspur();
    let b = books();
    run_clean(&b, &src, &SyncOptions::default()).await;

    // Debit, credit, debit: as entered, though QBD sends debits and credits apart.
    let je = txn(&b, QbdEntity::JournalEntry, "1010-1700002000");
    let amounts: Vec<Money> = je.lines.iter().map(|l| l.amount).collect();
    assert_eq!(amounts, vec![m("1200.00"), m("-1500.00"), m("300.00")]);
    assert!(je.is_adjustment);

    // Items, a text line, a group (its own line carries no amount) with its
    // members in order, the subtotal and the discount.
    let inv = txn(&b, QbdEntity::Invoice, "2000-1700003000");
    let shape: Vec<(LineKind, Money)> = inv.lines.iter().map(|l| (l.kind, l.amount)).collect();
    assert_eq!(
        shape,
        vec![
            (LineKind::Item, m("1500.00")),
            (LineKind::Item, m("50.00")),
            (LineKind::Text, Money::ZERO),
            (LineKind::Item, Money::ZERO),
            (LineKind::Item, m("100.00")),
            (LineKind::Item, m("50.00")),
            (LineKind::Item, m("1700.00")),
            (LineKind::Item, m("-170.00")),
        ]
    );
    assert_eq!(inv.amount, m("1530.00"));
    assert_eq!(
        inv.lines[2].description.as_deref(),
        Some("Thanks for choosing Larkspur")
    );
    assert_eq!(
        inv.lines[3].extra.as_ref().unwrap()["TotalAmount"],
        "150.00"
    );
    assert!(inv.lines[7].rate_is_percent);
    assert_eq!(inv.message.as_deref(), Some("Thank you for your business."));
    // Fields with no LibreBooks home are kept verbatim.
    let extra = inv.extra.as_ref().unwrap();
    assert_eq!(extra["TxnNumber"], "10");
    assert_eq!(extra["TemplateRef"]["FullName"], "Intuit Service Invoice");

    // A check's expense, item and group lines, entry order.
    let chk = txn(&b, QbdEntity::Check, "5010-1700006100");
    let kinds: Vec<LineKind> = chk.lines.iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        vec![
            LineKind::Account,
            LineKind::Item,
            LineKind::Item,
            LineKind::Item,
            LineKind::Item,
            LineKind::Account
        ]
    );
    assert_eq!(chk.lines[0].billable, lb_core::BillableStatus::Billable);
    assert!(chk.lines[0].entity_id.is_some());
}

#[tokio::test]
async fn payments_credits_discounts_and_deposits_link_up() {
    let src = larkspur();
    let b = books();
    run_clean(&b, &src, &SyncOptions::default()).await;
    let inv1 = txn(&b, QbdEntity::Invoice, "2000-1700003000");
    let cm = txn(&b, QbdEntity::CreditMemo, "2100-1700003100");
    let pmt1 = txn(&b, QbdEntity::ReceivePayment, "3000-1700004000");
    assert_eq!(pmt1.applications.len(), 2);
    assert_eq!(pmt1.applications[0].target_txn_id, inv1.id);
    assert_eq!(pmt1.applications[0].amount, m("1400.00"));
    assert_eq!(pmt1.applications[1].credit_txn_id, Some(cm.id));
    assert_eq!(pmt1.applications[1].credit_amount, m("130.00"));
    assert_eq!(open_balance(&b, inv1.id), Some(Money::ZERO));
    assert_eq!(open_balance(&b, cm.id), Some(Money::ZERO));

    // A discount, a payment applied to a journal entry, another payment linked to
    // the same invoice ignored, a dollar left unapplied.
    let pmt2 = txn(&b, QbdEntity::ReceivePayment, "3010-1700004100");
    assert_eq!(pmt2.applications.len(), 2);
    assert_eq!(pmt2.applications[0].discount, m("6.00"));
    assert_eq!(open_balance(&b, pmt2.id), Some(m("1.00")));
    let inv2 = txn(&b, QbdEntity::Invoice, "2010-1700003200");
    assert_eq!(open_balance(&b, inv2.id), Some(Money::ZERO));

    // The deposit takes the payment out of Undeposited Funds.
    let dep = txn(&b, QbdEntity::Deposit, "4000-1700005000");
    assert_eq!(dep.lines[0].linked_txn_id, Some(pmt1.id));
    assert_eq!(dep.lines[0].check_number.as_deref(), Some("5512"));
    assert_eq!(dep.lines.last().unwrap().kind, LineKind::CashBack);
    assert_eq!(dep.amount, m("1450.00"));
    assert_eq!(balance(&b, "Undeposited Funds"), Money::ZERO);

    // Bill and bill payment.
    let bill = txn(&b, QbdEntity::Bill, "6000-1700007000");
    assert_eq!(open_balance(&b, bill.id), Some(Money::ZERO));

    // Voided check, time entry, non-posting forms.
    assert!(txn(&b, QbdEntity::Check, "5020-1700006200").is_void);
    let time = txn(&b, QbdEntity::TimeTracking, "7300-1700008300");
    assert_eq!(time.lines[0].quantity, Some("2.5".parse().unwrap()));
    assert_eq!(time.lines[0].extra.as_ref().unwrap()["Duration"], "PT2H30M");
    assert_eq!(
        txn(&b, QbdEntity::Estimate, "7100-1700008100").amount,
        m("450.00")
    );
    assert_eq!(
        txn(&b, QbdEntity::PurchaseOrder, "7200-1700008200").amount,
        m("300.00")
    );
    // Sales tax computed by LibreBooks matches QBD's.
    let inv4 = txn(&b, QbdEntity::Invoice, "2030-1700003400");
    assert_eq!(inv4.amount, m("258.25"));
}

#[tokio::test]
async fn a_second_sync_fetches_nothing_unchanged() {
    let src = larkspur();
    let b = books();
    run_clean(&b, &src, &SyncOptions::default()).await;
    let watermark = b
        .read(|c| lb_store::company::setting(c, &watermark_key(QbdEntity::Check)))
        .unwrap();
    assert_eq!(watermark.as_deref(), Some("2026-03-31T12:00:00-08:00"));
    src.clear_gets();
    let again = run_clean(&b, &src, &SyncOptions::default()).await;
    assert!(src.gets().is_empty(), "fetched {:?}", src.gets());
    for e in &again.entities {
        assert_eq!(e.created + e.updated, 0, "{:?} changed", e.entity);
        // A deliberately skipped record has nothing local to compare with, so it
        // is looked at (and skipped) again; that costs no request.
        assert_eq!(e.unchanged + e.skipped, e.listed, "{:?}", e.entity);
    }
    assert_eq!(again.entity(QbdEntity::Item).unwrap().skipped, 1);
    assert_eq!(balance(&b, "Checking"), m("9685.00"));
}

#[tokio::test]
async fn edits_in_quickbooks_update_the_local_record() {
    let src = larkspur();
    let b = books();
    run_clean(&b, &src, &SyncOptions::default()).await;
    let before = txn(&b, QbdEntity::Check, "5000-1700006000");

    assert!(src.update(QbdEntity::Check, "5000-1700006000", |c| {
        c["EditSequence"] = json!("1775000000");
        c["TimeModified"] = json!("2026-04-01T08:00:00-08:00");
        c["Amount"] = json!("2100.00");
        c["ExpenseLineRet"]["Amount"] = json!("2100.00");
    }));
    src.put(
        QbdEntity::Check,
        json!({"TxnID": "5040-1775000100", "TimeCreated": "2026-04-01T09:00:00-08:00",
            "TimeModified": "2026-04-01T09:00:00-08:00", "EditSequence": "1775000100",
            "AccountRef": {"ListID": "80000010-1700000000", "FullName": "Checking"},
            "PayeeEntityRef": {"ListID": "80000040-1700000000", "FullName": "Pacific Power"},
            "RefNumber": "1006", "TxnDate": "2026-04-01", "Amount": "33.00",
            "ExpenseLineRet": {"TxnLineID": "5041-1775000100",
                "AccountRef": {"ListID": "8000001E-1700000000", "FullName": "Utilities"}, "Amount": "33.00"}}),
    );
    src.clear_gets();
    let report = run_clean(&b, &src, &SyncOptions::default()).await;
    let checks = report.entity(QbdEntity::Check).unwrap();
    assert_eq!((checks.updated, checks.created), (1, 1));
    // Only the two changed checks were fetched.
    assert_eq!(src.gets().len(), 2, "{:?}", src.gets());

    let after = txn(&b, QbdEntity::Check, "5000-1700006000");
    assert_eq!(after.id, before.id);
    assert_eq!(after.edit_seq, before.edit_seq + 1);
    assert_eq!(after.lines[0].id, before.lines[0].id, "line ids are kept");
    assert_eq!(balance(&b, "Rent"), m("2400.00"));
    assert_eq!(balance(&b, "Utilities"), m("153.00"));
    assert_eq!(balance(&b, "Checking"), m("9552.00"));
    assert_eq!(
        b.read(|c| lb_store::company::setting(c, &watermark_key(QbdEntity::Check)))
            .unwrap()
            .as_deref(),
        Some("2026-04-01T09:00:00-08:00")
    );
}

#[tokio::test]
async fn the_sweep_deletes_what_quickbooks_deleted() {
    let src = larkspur();
    let b = books();
    src.put(
        QbdEntity::Vendor,
        json!({"ListID": "80000043-1700000000", "TimeModified": "2026-03-31T12:00:00-08:00",
            "EditSequence": "1700000000", "Name": "Never Used", "IsActive": "true"}),
    );
    run_clean(&b, &src, &SyncOptions::default()).await;
    assert!(src.remove(QbdEntity::Check, "5030-1700006300"));
    assert!(src.remove(QbdEntity::Vendor, "80000043-1700000000"));
    // Still used by a check here, so it can only be made inactive.
    assert!(src.remove(QbdEntity::Vendor, "80000041-1700000000"));

    // Without a sweep nothing is deleted.
    run_clean(&b, &src, &SyncOptions::default()).await;
    assert_eq!(balance(&b, "Payroll Expenses"), m("50.00"));

    let report = run_clean(
        &b,
        &src,
        &SyncOptions {
            sweep: true,
            ..SyncOptions::default()
        },
    )
    .await;
    assert_eq!(report.entity(QbdEntity::Check).unwrap().deleted, 1);
    let vendors = report.entity(QbdEntity::Vendor).unwrap();
    assert_eq!((vendors.deleted, vendors.deactivated), (1, 1));
    assert_eq!(balance(&b, "Payroll Expenses"), Money::ZERO);
    assert!(
        b.read(|c| lb_store::entities::by_full_name(c, "Never Used"))
            .unwrap()
            .is_none()
    );
    let landlord = b
        .read(|c| lb_store::entities::by_full_name(c, "Landlord LLC"))
        .unwrap()
        .unwrap();
    assert!(!landlord.is_active);
    assert_eq!(b.check_integrity().unwrap(), Vec::<String>::new());
}

#[tokio::test]
async fn a_missing_link_is_retried_until_it_resolves() {
    let src = larkspur();
    let b = books();
    // An invoice that can't import (its item isn't in QuickBooks' item list) and
    // a payment of it.
    src.put(
        QbdEntity::Invoice,
        json!({"TxnID": "2040-1775000200", "TimeModified": "2026-04-02T09:00:00-08:00", "EditSequence": "1775000200",
            "CustomerRef": {"ListID": "80000033-1700000100", "FullName": "Blue Heron Cafe"},
            "TxnDate": "2026-04-02", "RefNumber": "1005", "Subtotal": "60.00", "SalesTaxTotal": "0.00", "BalanceRemaining": "0.00",
            "InvoiceLineRet": {"TxnLineID": "2041-1775000200", "ItemRef": {"ListID": "dead-0000", "FullName": "Gone"}, "Amount": "60.00"}}),
    );
    src.put(
        QbdEntity::ReceivePayment,
        json!({"TxnID": "3040-1775000300", "TimeModified": "2026-04-02T10:00:00-08:00", "EditSequence": "1775000300",
            "CustomerRef": {"ListID": "80000033-1700000100", "FullName": "Blue Heron Cafe"},
            "TxnDate": "2026-04-02", "TotalAmount": "60.00", "UnusedPayment": "0.00",
            "DepositToAccountRef": {"ListID": "80000013-1700000000", "FullName": "Undeposited Funds"},
            "AppliedToTxnRet": {"TxnID": "2040-1775000200", "TxnType": "Invoice", "Amount": "60.00"}}),
    );
    let first = run(&b, &src, &SyncOptions::default()).await;
    let inv = first.entity(QbdEntity::Invoice).unwrap();
    assert_eq!(inv.failed, 1);
    assert!(
        first
            .issues
            .iter()
            .any(|i| i.qbd_id.as_deref() == Some("2040-1775000200")
                && i.message.contains("item \"Gone\" (dead-0000)"))
    );
    // The payment imports (its money is real) without the application, and is
    // marked to be looked at again.
    let pmts = first.entity(QbdEntity::ReceivePayment).unwrap();
    assert_eq!((pmts.created, pmts.partial), (4, 1));
    // Neither watermark passes the record that needs another look (they stop on
    // it; `modified_after` is inclusive, so it's listed again).
    assert_eq!(
        inv.watermark_after.as_deref(),
        Some("2026-04-02T09:00:00-08:00")
    );
    assert_eq!(
        pmts.watermark_after.as_deref(),
        Some("2026-04-02T10:00:00-08:00")
    );

    // QuickBooks fixes the invoice.
    src.update(QbdEntity::Invoice, "2040-1775000200", |v| {
        v["EditSequence"] = json!("1775000400");
        v["TimeModified"] = json!("2026-04-03T09:00:00-08:00");
        v["InvoiceLineRet"]["ItemRef"] =
            json!({"ListID": "80000070-1700000200", "FullName": "Engineering"});
    });
    let second = run_clean(&b, &src, &SyncOptions::default()).await;
    assert_eq!(second.entity(QbdEntity::Invoice).unwrap().created, 1);
    let pmts = second.entity(QbdEntity::ReceivePayment).unwrap();
    assert_eq!((pmts.updated, pmts.partial), (1, 0));
    let pmt = txn(&b, QbdEntity::ReceivePayment, "3040-1775000300");
    assert_eq!(pmt.applications.len(), 1);
    let inv = txn(&b, QbdEntity::Invoice, "2040-1775000200");
    assert_eq!(open_balance(&b, inv.id), Some(Money::ZERO));
}

#[tokio::test]
async fn limited_runs_leave_watermarks_alone() {
    let src = larkspur();
    let b = books();
    let report = run_clean(
        &b,
        &src,
        &SyncOptions {
            txn_limit: Some(1),
            ..SyncOptions::default()
        },
    )
    .await;
    assert!(report.partial_run);
    assert_eq!(report.entity(QbdEntity::Check).unwrap().listed, 1);
    assert_eq!(
        b.read(|c| lb_store::company::setting(c, &watermark_key(QbdEntity::Check)))
            .unwrap(),
        None
    );
    // Lists are still synced in full.
    assert_eq!(report.entity(QbdEntity::Account).unwrap().listed, 24);
}

#[tokio::test]
async fn refuses_to_mix_two_companies() {
    let src = larkspur();
    let b = books();
    run_clean(&b, &src, &SyncOptions::default()).await;
    let other = lb_qbd::testing::FixtureSource::new(
        json!({"CompanyName": "Someone Else", "EIN": "99-9999999"}),
        json!({}),
    );
    let err = lb_qbd::sync::sync(&b, &other, &SyncOptions::default(), &|_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, SyncError::DifferentCompany { .. }), "{err}");
    assert!(err.to_string().contains("refusing to mix"));
}
