//! The store against a real SQLite file: lists, saving transactions, open
//! balances, registers, the closing date and the audit trail.

use jiff::civil::date;
use lb_core::lists::{SalesTaxCode, Terms, TermsKind};
use lb_core::*;
use lb_store::{
    Actor, Books, StoreError, accounts, audit, balances, company, entities, items, lists, txns,
};
use pretty_assertions::assert_eq;
use rust_decimal::Decimal;

fn books() -> Books {
    let info = CompanyInfo {
        name: "Test Co".into(),
        ..CompanyInfo::default()
    };
    Books::in_memory(&info).unwrap()
}

fn me() -> Actor {
    Actor {
        user_id: Some("tester".into()),
        may_override_closing_date: false,
    }
}

struct Setup {
    bank: AccountId,
    rent: AccountId,
    income: AccountId,
    vendor: EntityId,
    customer: EntityId,
    job: EntityId,
    labor: ItemId,
}

fn setup(b: &Books) -> Setup {
    b.write(&me(), |w| {
        let bank = accounts::save(w, accounts::new_account("Checking", AccountType::Bank))?.id;
        let rent = accounts::save(w, accounts::new_account("Rent", AccountType::Expense))?.id;
        let income = accounts::save(
            w,
            accounts::new_account("Consulting Income", AccountType::Income),
        )?
        .id;
        let vendor = entities::save(w, Entity::blank(EntityKind::Vendor, "Landlord"))?.id;
        let customer = entities::save(w, Entity::blank(EntityKind::Customer, "Acme"))?.id;
        let mut j = Entity::blank(EntityKind::Customer, "Phase 1");
        j.parent_id = Some(customer);
        let job = entities::save(w, j)?.id;
        let mut labor = Item::blank(ItemType::Service, "Labor");
        labor.income_account_id = Some(income);
        let labor = items::save(w, labor)?.id;
        Ok(Setup {
            bank,
            rent,
            income,
            vendor,
            customer,
            job,
            labor,
        })
    })
    .unwrap()
}

#[test]
fn new_company_has_qbd_starting_accounts() {
    let b = books();
    let names: Vec<String> = b
        .read(|c| accounts::list(c, true))
        .unwrap()
        .into_iter()
        .map(|a| a.full_name)
        .collect();
    for n in [
        "Accounts Receivable",
        "Accounts Payable",
        "Undeposited Funds",
        "Opening Balance Equity",
        "Retained Earnings",
    ] {
        assert!(names.contains(&n.to_owned()), "{n} missing from {names:?}");
    }
    assert_eq!(b.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn subaccount_names_follow_their_parent() {
    let b = books();
    let (parent, child) = b
        .write(&me(), |w| {
            let p = accounts::save(w, accounts::new_account("Utilities", AccountType::Expense))?;
            let mut c = accounts::new_account("Internet", AccountType::Expense);
            c.parent_id = Some(p.id);
            let c = accounts::save(w, c)?;
            Ok((p, c))
        })
        .unwrap();
    assert_eq!(child.full_name, "Utilities:Internet");
    assert_eq!(child.sublevel, 1);

    let mut p = parent.clone();
    p.name = "Web-Internet Expenses".into();
    b.write(&me(), |w| accounts::save(w, p)).unwrap();
    let child = b.read(|c| accounts::get(c, child.id)).unwrap();
    assert_eq!(child.full_name, "Web-Internet Expenses:Internet");

    // A subaccount of another type is refused, and so is a cycle.
    let mut wrong = accounts::new_account("Bad", AccountType::Income);
    wrong.parent_id = Some(parent.id);
    assert!(matches!(
        b.write(&me(), |w| accounts::save(w, wrong)),
        Err(StoreError::Invalid(_))
    ));
    let mut looped = b.read(|c| accounts::get(c, parent.id)).unwrap();
    looped.parent_id = Some(child.id);
    assert!(matches!(
        b.write(&me(), |w| accounts::save(w, looped)),
        Err(StoreError::Invalid(_))
    ));
}

#[test]
fn names_are_unique_across_every_list() {
    let b = books();
    b.write(&me(), |w| {
        entities::save(w, Entity::blank(EntityKind::Vendor, "Comcast"))
    })
    .unwrap();
    let err = b
        .write(&me(), |w| {
            entities::save(w, Entity::blank(EntityKind::Customer, "comcast"))
        })
        .unwrap_err();
    assert!(matches!(err, StoreError::Duplicate { .. }), "{err:?}");
}

#[test]
fn stale_edits_are_refused() {
    let b = books();
    let a = b
        .write(&me(), |w| {
            accounts::save(w, accounts::new_account("Rent", AccountType::Expense))
        })
        .unwrap();
    let mut first = a.clone();
    first.description = Some("one".into());
    b.write(&me(), |w| accounts::save(w, first)).unwrap();
    let mut second = a;
    second.description = Some("two".into());
    assert!(matches!(
        b.write(&me(), |w| accounts::save(w, second)),
        Err(StoreError::Conflict { .. })
    ));
}

#[test]
fn check_posts_and_shows_in_the_register() {
    let b = books();
    let s = setup(&b);
    let mut t = Txn::new(TxnType::Check, date(2026, 10, 1));
    t.account_id = Some(s.bank);
    t.entity_id = Some(s.vendor);
    t.number = Some("1001".into());
    t.lines.push(TxnLine::account(s.rent, Money(150_000)));
    let out = b.write(&me(), |w| txns::save(w, t)).unwrap();
    assert_eq!(out.total, Money(150_000));
    assert_eq!(out.txn.edit_seq, 1);

    let (opening, reg) = b
        .read(|c| balances::register(c, s.bank, None, None))
        .unwrap();
    assert_eq!(opening, Money::ZERO);
    assert_eq!(reg.len(), 1);
    assert_eq!(reg[0].amount, Money(-150_000));
    assert_eq!(reg[0].balance, Money(-150_000));
    assert_eq!(reg[0].payee.as_deref(), Some("Landlord"));
    assert_eq!(reg[0].other_account.as_deref(), Some("Rent"));

    let next = b
        .read(|c| txns::next_number(c, TxnType::Check, Some(s.bank)))
        .unwrap();
    assert_eq!(next.as_deref(), Some("1002"));

    let sums = b.read(|c| balances::sums(c, None, None)).unwrap();
    assert_eq!(sums[&s.rent], Money(150_000));
    assert_eq!(b.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn invoice_payment_and_deposit_flow() {
    let b = books();
    let s = setup(&b);
    // Invoice the job.
    let mut inv = Txn::new(TxnType::Invoice, date(2026, 10, 1));
    inv.entity_id = Some(s.job);
    inv.number = Some("1000".into());
    inv.lines.push(TxnLine::item(
        s.labor,
        Decimal::from(10),
        Decimal::from(220),
    ));
    let inv = b.write(&me(), |w| txns::save(w, inv)).unwrap().txn;
    let open = |b: &Books, id| {
        b.read(|c| {
            Ok(txns::query(
                c,
                &txns::TxnFilter {
                    open_only: false,
                    ..Default::default()
                },
            )?
            .into_iter()
            .find(|t| t.id == id)
            .unwrap()
            .open_balance)
        })
        .unwrap()
    };
    assert_eq!(open(&b, inv.id), Some(Money(220_000)));
    assert!(
        inv.ar_ap_account_id.is_some(),
        "the default A/R is recorded"
    );

    // The customer (parent of the job) pays part of it.
    let mut pmt = Txn::new(TxnType::ReceivePayment, date(2026, 10, 5));
    pmt.entity_id = Some(s.customer);
    pmt.amount = Money(200_000);
    pmt.applications
        .push(Application::cash(inv.id, Money(200_000)));
    let pmt = b.write(&me(), |w| txns::save(w, pmt)).unwrap().txn;
    assert_eq!(open(&b, inv.id), Some(Money(20_000)));
    assert_eq!(open(&b, pmt.id), Some(Money::ZERO));

    // The invoice can't be deleted while paid.
    assert!(
        b.write(&me(), |w| txns::delete(w, inv.id, inv.edit_seq))
            .is_err()
    );

    // Deposit it from Undeposited Funds.
    let uf = b
        .read(|c| accounts::by_role(c, AccountRole::UndepositedFunds))
        .unwrap()
        .unwrap()
        .id;
    let mut dep = Txn::new(TxnType::Deposit, date(2026, 10, 6));
    dep.account_id = Some(s.bank);
    let mut line = TxnLine::deposit(uf, Money(200_000)).with_entity(s.customer);
    line.linked_txn_id = Some(pmt.id);
    dep.lines.push(line);
    b.write(&me(), |w| txns::save(w, dep.clone())).unwrap();
    // The same payment can't go in a second deposit.
    let mut dep2 = dep;
    dep2.id = TxnId::new();
    dep2.lines[0].id = LineId::new();
    dep2.edit_seq = 0;
    assert!(b.write(&me(), |w| txns::save(w, dep2)).is_err());

    let sums = b.read(|c| balances::sums(c, None, None)).unwrap();
    assert_eq!(sums[&s.bank], Money(200_000));
    assert_eq!(sums.get(&uf).copied().unwrap_or_default(), Money::ZERO);
    assert_eq!(sums[&s.income], Money(-220_000));

    // Removing the payment from the invoice reopens it in full.
    let mut pmt2 = b.read(|c| txns::get(c, pmt.id)).unwrap();
    pmt2.applications.clear();
    b.write(&me(), |w| txns::save(w, pmt2)).unwrap();
    assert_eq!(open(&b, inv.id), Some(Money(220_000)));
    assert_eq!(
        open(&b, pmt.id),
        Some(Money(200_000)),
        "the payment is now an unapplied credit"
    );
    assert_eq!(b.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn a_payment_cannot_pay_someone_elses_invoice() {
    let b = books();
    let s = setup(&b);
    let other = b
        .write(&me(), |w| {
            entities::save(w, Entity::blank(EntityKind::Customer, "Other"))
        })
        .unwrap()
        .id;
    let mut inv = Txn::new(TxnType::Invoice, date(2026, 10, 1));
    inv.entity_id = Some(other);
    inv.lines
        .push(TxnLine::item(s.labor, Decimal::ONE, Decimal::from(5)));
    let inv = b.write(&me(), |w| txns::save(w, inv)).unwrap().txn;
    let mut pmt = Txn::new(TxnType::ReceivePayment, date(2026, 10, 5));
    pmt.entity_id = Some(s.customer);
    pmt.amount = Money(500);
    pmt.applications.push(Application::cash(inv.id, Money(500)));
    assert!(b.write(&me(), |w| txns::save(w, pmt)).is_err());
}

#[test]
fn bill_terms_set_the_due_date() {
    let b = books();
    let s = setup(&b);
    let net30 = Terms {
        id: TermsId::new(),
        name: "Net 30".into(),
        is_active: true,
        kind: TermsKind::Standard {
            due_days: 30,
            discount_days: None,
            discount_percent: None,
        },
        edit_seq: 0,
    };
    let net30 = b.write(&me(), |w| lists::save(w, net30)).unwrap();
    let mut bill = Txn::new(TxnType::Bill, date(2026, 10, 1));
    bill.entity_id = Some(s.vendor);
    bill.terms_id = Some(net30.id);
    bill.lines.push(TxnLine::account(s.rent, Money(100)));
    let bill = b.write(&me(), |w| txns::save(w, bill)).unwrap().txn;
    assert_eq!(bill.due_date, Some(date(2026, 10, 31)));
}

#[test]
fn sales_tax_creates_its_payable_account_on_first_use() {
    let b = books();
    let s = setup(&b);
    let (tax_item, taxable) = b
        .write(&me(), |w| {
            let agency = entities::save(w, Entity::blank(EntityKind::Vendor, "CDTFA"))?.id;
            let mut t = Item::blank(ItemType::SalesTax, "CA Tax");
            t.tax_rate = Some(Decimal::new(925, 2));
            t.tax_agency_id = Some(agency);
            let t = items::save(w, t)?.id;
            let code = lists::save(
                w,
                SalesTaxCode {
                    id: SalesTaxCodeId::new(),
                    name: "Tax".into(),
                    description: None,
                    is_taxable: true,
                    is_active: true,
                    edit_seq: 0,
                },
            )?
            .id;
            Ok((t, code))
        })
        .unwrap();
    assert!(
        b.read(|c| accounts::by_role(c, AccountRole::SalesTaxPayable))
            .unwrap()
            .is_none()
    );
    let mut inv = Txn::new(TxnType::Invoice, date(2026, 10, 1));
    inv.entity_id = Some(s.customer);
    inv.sales_tax_item_id = Some(tax_item);
    let mut l = TxnLine::item(s.labor, Decimal::ONE, Decimal::from(100));
    l.sales_tax_code_id = Some(taxable);
    inv.lines.push(l);
    let out = b.write(&me(), |w| txns::save(w, inv)).unwrap();
    assert_eq!(out.sales_tax, Money(925));
    assert_eq!(out.total, Money(10_925));
    assert!(
        b.read(|c| accounts::by_role(c, AccountRole::SalesTaxPayable))
            .unwrap()
            .is_some()
    );
}

#[test]
fn closing_date_protects_the_past() {
    let b = books();
    let s = setup(&b);
    b.write(&Actor::system(), |w| {
        company::set_closing_date(w, Some(date(2026, 9, 30)))
    })
    .unwrap();
    let mut t = Txn::new(TxnType::Check, date(2026, 9, 15));
    t.account_id = Some(s.bank);
    t.lines.push(TxnLine::account(s.rent, Money(1)));
    let err = b.write(&me(), |w| txns::save(w, t.clone())).unwrap_err();
    assert!(matches!(err, StoreError::ClosedPeriod { .. }), "{err:?}");
    // An admin may, as QBD's closing date password allows.
    b.write(&Actor::system(), |w| txns::save(w, t)).unwrap();
}

#[test]
fn void_keeps_the_record_and_audits_everything() {
    let b = books();
    let s = setup(&b);
    let mut t = Txn::new(TxnType::Check, date(2026, 10, 1));
    t.account_id = Some(s.bank);
    t.memo = Some("oops".into());
    t.lines.push(TxnLine::account(s.rent, Money(999)));
    let t = b.write(&me(), |w| txns::save(w, t)).unwrap().txn;
    let v = b.write(&me(), |w| txns::void(w, t.id, t.edit_seq)).unwrap();
    assert!(v.txn.is_void);
    assert_eq!(v.txn.memo.as_deref(), Some("VOID: oops"));
    let sums = b.read(|c| balances::sums(c, None, None)).unwrap();
    assert_eq!(sums.get(&s.bank).copied().unwrap_or_default(), Money::ZERO);
    let (_, reg) = b
        .read(|c| balances::register(c, s.bank, None, None))
        .unwrap();
    assert_eq!(reg.len(), 1, "a void check still shows in the register");
    assert!(reg[0].is_void);

    let history = b
        .read(|c| audit::history(c, "txn", &t.id.to_string()))
        .unwrap();
    let actions: Vec<&str> = history.iter().map(|h| h.action.as_str()).collect();
    assert_eq!(actions, vec!["create", "update", "void"]);
    assert_eq!(history[0].user_id.as_deref(), Some("tester"));
}

#[test]
fn lists_in_use_cannot_be_deleted() {
    let b = books();
    let s = setup(&b);
    let mut t = Txn::new(TxnType::Check, date(2026, 10, 1));
    t.account_id = Some(s.bank);
    t.entity_id = Some(s.vendor);
    t.lines.push(TxnLine::account(s.rent, Money(1)));
    b.write(&me(), |w| txns::save(w, t)).unwrap();
    let rent = b.read(|c| accounts::get(c, s.rent)).unwrap();
    assert!(matches!(
        b.write(&me(), |w| accounts::delete(w, rent.id, rent.edit_seq)),
        Err(StoreError::InUse { .. })
    ));
    let v = b.read(|c| entities::get(c, s.vendor)).unwrap();
    assert!(matches!(
        b.write(&me(), |w| entities::delete(w, v.id, v.edit_seq)),
        Err(StoreError::InUse { .. })
    ));
    let unused = b
        .write(&me(), |w| {
            accounts::save(w, accounts::new_account("Unused", AccountType::Expense))
        })
        .unwrap();
    b.write(&me(), |w| accounts::delete(w, unused.id, unused.edit_seq))
        .unwrap();
}

#[test]
fn a_file_survives_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.lbdb");
    let info = CompanyInfo {
        name: "Persisted".into(),
        ..CompanyInfo::default()
    };
    {
        let b = Books::create(&path, &info).unwrap();
        setup(&b);
    }
    let b = Books::open(&path).unwrap();
    assert_eq!(b.read(company::info).unwrap().name, "Persisted");
    assert!(
        b.read(|c| accounts::by_full_name(c, "Checking"))
            .unwrap()
            .is_some()
    );
    let copy = dir.path().join("copy.lbdb");
    b.backup_to(&copy).unwrap();
    let c = Books::open(&copy).unwrap();
    assert!(
        c.read(|c| accounts::by_full_name(c, "Rent"))
            .unwrap()
            .is_some()
    );
    assert!(Books::create(&path, &info).is_err(), "won't overwrite");
}

#[test]
fn listeners_hear_committed_changes_only() {
    use std::sync::{Arc, Mutex};
    let b = books();
    let heard = Arc::new(Mutex::new(Vec::new()));
    let h = heard.clone();
    b.subscribe(move |changes| h.lock().unwrap().extend(changes.iter().map(|c| c.kind)));
    let _ = b.write(&me(), |w| {
        accounts::save(
            w,
            accounts::new_account("Rolled back", AccountType::Expense),
        )?;
        Err::<(), _>(StoreError::Invalid("no".into()))
    });
    assert!(heard.lock().unwrap().is_empty());
    b.write(&me(), |w| {
        accounts::save(w, accounts::new_account("Kept", AccountType::Expense))
    })
    .unwrap();
    assert_eq!(*heard.lock().unwrap(), vec!["account"]);
    assert!(
        b.read(|c| accounts::by_full_name(c, "Rolled back"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn find_by_text_amount_and_name() {
    let b = books();
    let s = setup(&b);
    for (n, amt, memo) in [
        ("1", 100, "paper"),
        ("2", 250, "Toner 50% off"),
        ("3", 100, "pens"),
    ] {
        let mut t = Txn::new(TxnType::Check, date(2026, 10, 1));
        t.account_id = Some(s.bank);
        t.entity_id = Some(s.vendor);
        t.number = Some(n.into());
        t.memo = Some(memo.into());
        t.lines.push(TxnLine::account(s.rent, Money(amt)));
        b.write(&me(), |w| txns::save(w, t)).unwrap();
    }
    let q = |f: txns::TxnFilter| b.read(|c| txns::query(c, &f)).unwrap();
    assert_eq!(
        q(txns::TxnFilter {
            amount: Some(Money(100)),
            ..Default::default()
        })
        .len(),
        2
    );
    assert_eq!(
        q(txns::TxnFilter {
            text: Some("50%".into()),
            ..Default::default()
        })
        .len(),
        1
    );
    assert_eq!(
        q(txns::TxnFilter {
            text: Some("landlord".into()),
            ..Default::default()
        })
        .len(),
        3
    );
    assert_eq!(
        q(txns::TxnFilter {
            entity_id: Some(s.customer),
            include_jobs: true,
            ..Default::default()
        })
        .len(),
        0
    );
    let latest = q(txns::TxnFilter {
        descending: true,
        limit: Some(1),
        ..Default::default()
    });
    assert_eq!(latest[0].number.as_deref(), Some("3"));
}

#[test]
fn a_journal_entry_in_ar_is_an_open_item_a_payment_can_pay() {
    let b = books();
    let s = setup(&b);
    let ar = b
        .read(|c| accounts::by_role(c, AccountRole::AccountsReceivable))
        .unwrap()
        .unwrap()
        .id;
    let mut je = Txn::new(TxnType::JournalEntry, date(2026, 10, 1));
    je.lines
        .push(TxnLine::journal(ar, Money(5_000)).with_entity(s.customer));
    je.lines.push(TxnLine::journal(s.income, Money(-5_000)));
    let je = b.write(&me(), |w| txns::save(w, je)).unwrap().txn;
    assert_eq!(je.ar_ap_account_id, Some(ar));
    let open = |id| {
        b.read(|c| {
            Ok(txns::query(c, &txns::TxnFilter::default())?
                .into_iter()
                .find(|t| t.id == id)
                .unwrap()
                .open_balance)
        })
        .unwrap()
    };
    assert_eq!(open(je.id), Some(Money(5_000)));
    let mut pmt = Txn::new(TxnType::ReceivePayment, date(2026, 10, 5));
    pmt.entity_id = Some(s.customer);
    pmt.amount = Money(5_000);
    pmt.applications
        .push(Application::cash(je.id, Money(5_000)));
    b.write(&me(), |w| txns::save(w, pmt)).unwrap();
    assert_eq!(open(je.id), Some(Money::ZERO));
}
