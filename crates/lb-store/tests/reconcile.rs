//! Reconciliation, opening balances and attachments.

use jiff::civil::date;
use lb_core::*;
use lb_store::reconcile::{self, StatementCharge};
use lb_store::{Actor, Books, accounts, attachments, balances, txns};
use pretty_assertions::assert_eq;

fn books() -> Books {
    Books::in_memory(&CompanyInfo {
        name: "Rec Co".into(),
        ..CompanyInfo::default()
    })
    .unwrap()
}

fn me() -> Actor {
    Actor {
        user_id: Some("tester".into()),
        may_override_closing_date: false,
    }
}

fn check(
    b: &Books,
    bank: AccountId,
    expense: AccountId,
    d: jiff::civil::Date,
    cents: i64,
) -> TxnId {
    let mut t = Txn::new(TxnType::Check, d);
    t.account_id = Some(bank);
    t.lines.push(TxnLine::account(expense, Money(cents)));
    b.write(&me(), |w| txns::save(w, t)).unwrap().txn.id
}

fn deposit(
    b: &Books,
    bank: AccountId,
    income: AccountId,
    d: jiff::civil::Date,
    cents: i64,
) -> TxnId {
    let mut t = Txn::new(TxnType::Deposit, d);
    t.account_id = Some(bank);
    t.lines.push(TxnLine::deposit(income, Money(cents)));
    b.write(&me(), |w| txns::save(w, t)).unwrap().txn.id
}

#[test]
fn a_month_reconciles_with_a_service_charge() {
    let b = books();
    let (bank, rent, income, fees) = b
        .write(&me(), |w| {
            Ok((
                accounts::save(w, accounts::new_account("Checking", AccountType::Bank))?.id,
                accounts::save(w, accounts::new_account("Rent", AccountType::Expense))?.id,
                accounts::save(w, accounts::new_account("Sales", AccountType::Income))?.id,
                accounts::save(w, accounts::new_account("Bank Fees", AccountType::Expense))?.id,
            ))
        })
        .unwrap();
    b.write(&me(), |w| {
        accounts::set_opening_balance(w, bank, Money(100_000), date(2026, 8, 31))
    })
    .unwrap();
    let opening = b
        .read(|c| txns::query(c, &txns::TxnFilter::default()))
        .unwrap()[0]
        .id;
    let dep = deposit(&b, bank, income, date(2026, 9, 3), 50_000);
    let chk = check(&b, bank, rent, date(2026, 9, 10), 120_000);
    let late = check(&b, bank, rent, date(2026, 9, 29), 9_999); // hasn't cleared yet
    let _october = check(&b, bank, rent, date(2026, 10, 2), 1); // after the statement

    // Statement: 1000.00 + 500.00 − 1200.00 − 15.00 fee = 285.00
    let rec = b
        .write(&me(), |w| {
            reconcile::start(
                w,
                bank,
                date(2026, 9, 30),
                Money(28_500),
                Some(StatementCharge {
                    amount: Money(1_500),
                    date: date(2026, 9, 30),
                    account_id: fees,
                }),
                None,
            )
        })
        .unwrap();
    assert_eq!(rec.beginning_balance, Money::ZERO);
    let lines = b.read(|c| reconcile::lines(c, rec.id)).unwrap();
    // The opening balance, deposit, two September checks and the fee; not October's.
    assert_eq!(lines.len(), 5, "{lines:#?}");
    assert!(
        lines.iter().any(|l| l.marked && l.amount == Money(-1_500)),
        "the fee is ticked"
    );

    b.write(&me(), |w| {
        reconcile::mark(w, rec.id, &[opening, dep, chk], true)
    })
    .unwrap();
    let st = b.read(|c| reconcile::status(c, rec.id)).unwrap();
    assert_eq!(st.cleared_balance, Money(28_500));
    assert_eq!(st.difference, Money::ZERO);

    // Ticking the uncleared check unbalances it; finishing is then refused.
    b.write(&me(), |w| reconcile::mark(w, rec.id, &[late], true))
        .unwrap();
    assert_eq!(
        b.read(|c| reconcile::status(c, rec.id)).unwrap().difference,
        Money(9_999)
    );
    assert!(
        b.write(&me(), |w| reconcile::finish(w, rec.id, false))
            .is_err()
    );
    b.write(&me(), |w| reconcile::mark(w, rec.id, &[late], false))
        .unwrap();

    let done = b
        .write(&me(), |w| reconcile::finish(w, rec.id, false))
        .unwrap();
    assert_eq!(done.cleared_txn_ids.len(), 4);
    assert_eq!(
        b.read(|c| reconcile::cleared_status(c, chk, bank)).unwrap(),
        ClearedStatus::Cleared
    );
    assert_eq!(
        b.read(|c| reconcile::cleared_status(c, late, bank))
            .unwrap(),
        ClearedStatus::Uncleared
    );
    assert_eq!(
        b.read(|c| reconcile::beginning_balance(c, bank)).unwrap(),
        Money(28_500)
    );

    // Editing a cleared check's memo keeps it cleared (and in its reconciliation).
    let mut edited = b.read(|c| txns::get(c, chk)).unwrap();
    edited.memo = Some("rent, September".into());
    b.write(&me(), |w| txns::save(w, edited)).unwrap();
    assert_eq!(
        b.read(|c| reconcile::cleared_status(c, chk, bank)).unwrap(),
        ClearedStatus::Cleared
    );

    // Undo puts everything back.
    let undone = b
        .write(&me(), |w| reconcile::undo_last(w, bank))
        .unwrap()
        .unwrap();
    assert_eq!(undone.id, rec.id);
    assert_eq!(
        b.read(|c| reconcile::cleared_status(c, chk, bank)).unwrap(),
        ClearedStatus::Uncleared
    );
    assert_eq!(
        b.read(|c| reconcile::beginning_balance(c, bank)).unwrap(),
        Money::ZERO
    );
}

#[test]
fn a_difference_can_be_adjusted_away() {
    let b = books();
    let (card, supplies) = b
        .write(&me(), |w| {
            Ok((
                accounts::save(w, accounts::new_account("Visa", AccountType::CreditCard))?.id,
                accounts::save(w, accounts::new_account("Supplies", AccountType::Expense))?.id,
            ))
        })
        .unwrap();
    let mut t = Txn::new(TxnType::CreditCardCharge, date(2026, 9, 5));
    t.account_id = Some(card);
    t.lines.push(TxnLine::account(supplies, Money(4_000)));
    let charge = b.write(&me(), |w| txns::save(w, t)).unwrap().txn.id;
    // The statement says 40.37 owed: 37 cents we never entered.
    let rec = b
        .write(&me(), |w| {
            reconcile::start(w, card, date(2026, 9, 30), Money(4_037), None, None)
        })
        .unwrap();
    b.write(&me(), |w| reconcile::mark(w, rec.id, &[charge], true))
        .unwrap();
    let st = b.read(|c| reconcile::status(c, rec.id)).unwrap();
    assert_eq!(
        st.cleared_balance,
        Money(4_000),
        "a card's balance owed is positive"
    );
    assert_eq!(st.difference, Money(37));
    let done = b
        .write(&me(), |w| reconcile::finish(w, rec.id, true))
        .unwrap();
    assert!(done.adjustment_txn_id.is_some());
    assert_eq!(
        b.read(|c| reconcile::beginning_balance(c, card)).unwrap(),
        Money(4_037)
    );
    let sums = b.read(|c| balances::sums(c, None, None)).unwrap();
    assert_eq!(sums[&card], Money(-4_037));
    assert_eq!(b.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn income_accounts_are_not_reconciled() {
    let b = books();
    let income = b
        .write(&me(), |w| {
            accounts::save(w, accounts::new_account("Sales", AccountType::Income))
        })
        .unwrap()
        .id;
    assert!(
        b.write(&me(), |w| reconcile::start(
            w,
            income,
            date(2026, 9, 30),
            Money::ZERO,
            None,
            None
        ))
        .is_err()
    );
}

#[test]
fn opening_balances_go_against_opening_balance_equity() {
    let b = books();
    let loan = b
        .write(&me(), |w| {
            accounts::save(
                w,
                accounts::new_account("Truck Loan", AccountType::LongTermLiability),
            )
        })
        .unwrap()
        .id;
    b.write(&me(), |w| {
        accounts::set_opening_balance(w, loan, Money(2_000_000), date(2026, 1, 1))
    })
    .unwrap();
    let obe = b
        .read(|c| accounts::by_role(c, AccountRole::OpeningBalanceEquity))
        .unwrap()
        .unwrap()
        .id;
    let sums = b.read(|c| balances::sums(c, None, None)).unwrap();
    assert_eq!(
        sums[&loan],
        Money(-2_000_000),
        "a liability is credit-normal"
    );
    assert_eq!(sums[&obe], Money(2_000_000));
    let ar = b
        .read(|c| accounts::by_role(c, AccountRole::AccountsReceivable))
        .unwrap()
        .unwrap()
        .id;
    assert!(
        b.write(&me(), |w| accounts::set_opening_balance(
            w,
            ar,
            Money(1),
            date(2026, 1, 1)
        ))
        .is_err()
    );
}

#[test]
fn receipts_wait_in_the_inbox_then_attach() {
    let b = books();
    let photo = b"\xff\xd8\xff\xe0 fake jpeg";
    let m = b
        .write(&me(), |w| {
            attachments::add(w, None, "IMG_0001.jpg", "image/jpeg", photo)
        })
        .unwrap();
    assert_eq!(m.size, photo.len() as u64);
    assert_eq!(b.read(attachments::inbox).unwrap().len(), 1);
    let txn = TxnId::new().to_string();
    b.write(&me(), |w| attachments::attach(w, m.id, Some(("txn", &txn))))
        .unwrap();
    assert_eq!(
        b.read(attachments::inbox).unwrap(),
        [] as [lb_store::attachments::AttachmentMeta; 0]
    );
    let list = b.read(|c| attachments::for_record(c, "txn", &txn)).unwrap();
    assert_eq!(list.len(), 1);
    let (_, data) = b.read(|c| attachments::get(c, m.id)).unwrap();
    assert_eq!(data, photo);
    assert!(
        b.write(&me(), |w| attachments::add(
            w,
            None,
            "empty.pdf",
            "application/pdf",
            b""
        ))
        .is_err()
    );
}
