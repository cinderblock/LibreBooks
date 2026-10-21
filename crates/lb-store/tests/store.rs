//! The store against a real SQLite file: lists, saving transactions, open
//! balances, registers, the closing date and the audit trail.

use jiff::civil::date;
use lb_core::lists::{SalesTaxCode, Terms, TermsKind};
use lb_core::*;
use lb_store::{
    Actor, Books, StoreError, accounts, audit, balances, company, entities, items, lists, orders,
    txns,
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

#[test]
fn merging_names_and_accounts_moves_everything_onto_the_survivor() {
    let b = books();
    let s = setup(&b);
    let (dup_vendor, utilities) = b
        .write(&me(), |w| {
            Ok((
                entities::save(w, Entity::blank(EntityKind::Vendor, "Landlord LLC"))?.id,
                accounts::save(w, accounts::new_account("Utilities", AccountType::Expense))?.id,
            ))
        })
        .unwrap();
    let mut t = Txn::new(TxnType::Check, date(2026, 10, 1));
    t.account_id = Some(s.bank);
    t.entity_id = Some(dup_vendor);
    t.lines.push(TxnLine::account(utilities, Money(4_200)));
    let t = b.write(&me(), |w| txns::save(w, t)).unwrap().txn;

    b.write(&me(), |w| {
        lb_store::merge::merge_entities(w, dup_vendor, s.vendor)
    })
    .unwrap();
    b.write(&me(), |w| {
        lb_store::merge::merge_accounts(w, utilities, s.rent)
    })
    .unwrap();

    let after = b.read(|c| txns::get(c, t.id)).unwrap();
    assert_eq!(after.entity_id, Some(s.vendor));
    assert_eq!(after.lines[0].account_id, Some(s.rent));
    assert_eq!(
        after.edit_seq,
        t.edit_seq + 2,
        "each merge bumps it, so a form open on the old version can't save over it"
    );
    assert!(b.read(|c| entities::find(c, dup_vendor)).unwrap().is_none());
    let sums = b.read(|c| balances::sums(c, None, None)).unwrap();
    assert_eq!(sums[&s.rent], Money(4_200));
    // Different lists don't merge.
    assert!(
        b.write(&me(), |w| lb_store::merge::merge_entities(
            w, s.customer, s.vendor
        ))
        .is_err()
    );
    assert_eq!(b.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn memorized_rent_enters_itself_each_month() {
    use lb_store::saved::{self, Interval, Memorized, Repeat};
    let b = books();
    let s = setup(&b);
    let mut rent = Txn::new(TxnType::Check, date(2026, 1, 1));
    rent.account_id = Some(s.bank);
    rent.entity_id = Some(s.vendor);
    rent.lines.push(TxnLine::account(s.rent, Money(150_000)));
    let m = Memorized {
        template: rent,
        repeat: Repeat::Automatic,
        interval: Some(Interval::Monthly),
        next_date: Some(date(2026, 8, 1)),
        days_in_advance: 0,
        remaining: Some(5),
        group: None,
    };
    b.write(&me(), |w| saved::memorize(w, None, "Rent", &m))
        .unwrap();
    // The server was off since July: August, September and October all enter.
    let entered = b
        .write(&me(), |w| saved::enter_due(w, date(2026, 10, 19)))
        .unwrap();
    assert_eq!(entered.len(), 3);
    let again = b
        .write(&me(), |w| saved::enter_due(w, date(2026, 10, 19)))
        .unwrap();
    assert!(again.is_empty(), "nothing enters twice");
    let (_, m) = b.read(saved::list_memorized).unwrap().remove(0);
    assert_eq!(m.next_date, Some(date(2026, 11, 1)));
    assert_eq!(m.remaining, Some(2));
    let sums = b.read(|c| balances::sums(c, None, None)).unwrap();
    assert_eq!(sums[&s.rent], Money(450_000));
}

#[test]
fn a_backdated_purchase_recosts_later_sales() {
    let b = books();
    let s = setup(&b);
    let (part, cogs, asset) = b
        .write(&me(), |w| {
            let cogs = accounts::save(
                w,
                accounts::new_account("COGS", AccountType::CostOfGoodsSold),
            )?
            .id;
            let asset = accounts::save(
                w,
                accounts::new_account("Inventory", AccountType::OtherCurrentAsset),
            )?
            .id;
            let mut i = Item::blank(ItemType::Inventory, "Widget");
            i.income_account_id = Some(s.income);
            i.expense_account_id = Some(cogs);
            i.asset_account_id = Some(asset);
            Ok((items::save(w, i)?.id, cogs, asset))
        })
        .unwrap();
    let buy = |b: &Books, d, qty: i64, cost_each: i64| {
        let mut t = Txn::new(TxnType::Bill, d);
        t.entity_id = Some(s.vendor);
        let mut l = TxnLine::item(part, Decimal::from(qty), Decimal::from(cost_each));
        l.kind = LineKind::Item;
        t.lines.push(l);
        b.write(&me(), |w| txns::save(w, t)).unwrap().txn
    };
    // 10 at $10, then sell 5 on the 20th: COGS $50.
    buy(&b, date(2026, 10, 1), 10, 10);
    let mut sale = Txn::new(TxnType::Invoice, date(2026, 10, 20));
    sale.entity_id = Some(s.customer);
    sale.lines
        .push(TxnLine::item(part, Decimal::from(5), Decimal::from(30)));
    let sale = b.write(&me(), |w| txns::save(w, sale)).unwrap().txn;
    let cogs_total = |b: &Books| {
        b.read(|c| balances::sums(c, None, None))
            .unwrap()
            .get(&cogs)
            .copied()
            .unwrap_or_default()
    };
    assert_eq!(cogs_total(&b), Money(5_000));

    // Another 10 at $20 entered back on the 15th: the sale's average is now $15.
    let back = buy(&b, date(2026, 10, 15), 10, 20);
    assert_eq!(cogs_total(&b), Money(7_500));
    let resaved = b.read(|c| txns::get(c, sale.id)).unwrap();
    assert_eq!(
        resaved.edit_seq,
        sale.edit_seq + 1,
        "the re-posted sale moved on"
    );
    let item = b.read(|c| items::get(c, part)).unwrap();
    assert_eq!(item.quantity_on_hand, Decimal::from(15));
    assert_eq!(item.average_cost, Decimal::from(15));

    // Deleting that purchase puts the sale back at $10 each.
    b.write(&me(), |w| txns::delete(w, back.id, back.edit_seq))
        .unwrap();
    assert_eq!(cogs_total(&b), Money(5_000));
    let sums = b.read(|c| balances::sums(c, None, None)).unwrap();
    assert_eq!(sums[&asset], Money(5_000));
    assert_eq!(b.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn a_refund_check_in_ar_is_paid_down_by_a_payment() {
    // QBD: a check to a customer posted to A/R is a receivable (a refund given
    // before the credit, or an advance); a later payment applies to it.
    let b = books();
    let s = setup(&b);
    let ar = b
        .read(|c| accounts::by_role(c, AccountRole::AccountsReceivable))
        .unwrap()
        .unwrap()
        .id;
    let mut chk = Txn::new(TxnType::Check, date(2026, 10, 1));
    chk.account_id = Some(s.bank);
    chk.entity_id = Some(s.customer);
    chk.lines
        .push(TxnLine::account(ar, Money(40_400)).with_entity(s.customer));
    let chk = b.write(&me(), |w| txns::save(w, chk)).unwrap().txn;
    assert_eq!(chk.ar_ap_account_id, Some(ar));
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
    assert_eq!(open(chk.id), Some(Money(40_400)));
    let mut pmt = Txn::new(TxnType::ReceivePayment, date(2026, 10, 10));
    pmt.entity_id = Some(s.customer);
    pmt.amount = Money(40_400);
    pmt.applications
        .push(Application::cash(chk.id, Money(40_400)));
    b.write(&me(), |w| txns::save(w, pmt)).unwrap();
    assert_eq!(open(chk.id), Some(Money::ZERO));
    // An ordinary expense check is not an open item.
    let mut plain = Txn::new(TxnType::Check, date(2026, 10, 2));
    plain.account_id = Some(s.bank);
    plain.lines.push(TxnLine::account(s.rent, Money(1)));
    let plain = b.write(&me(), |w| txns::save(w, plain)).unwrap().txn;
    assert_eq!(plain.ar_ap_account_id, None);
    assert_eq!(open(plain.id), None);
}

#[test]
fn credits_apply_to_open_invoices_oldest_first_without_moving_the_ledger() {
    let b = books();
    let s = setup(&b);
    let inv = |b: &Books, d, cents: i64| {
        let mut t = Txn::new(TxnType::Invoice, d);
        t.entity_id = Some(s.job);
        t.lines.push(TxnLine::item_amount(s.labor, Money(cents)));
        b.write(&me(), |w| txns::save(w, t)).unwrap().txn.id
    };
    let old = inv(&b, date(2026, 1, 10), 30_000);
    let newer = inv(&b, date(2026, 2, 10), 20_000);
    let mut cm = Txn::new(TxnType::CreditMemo, date(2026, 3, 1));
    cm.entity_id = Some(s.customer);
    cm.lines.push(TxnLine::item_amount(s.labor, Money(35_000)));
    let cm = b.write(&me(), |w| txns::save(w, cm)).unwrap().txn.id;
    let before = b.read(|c| balances::sums(c, None, None)).unwrap();
    assert_eq!(
        b.read(|c| lb_store::apply::names_with_unapplied_credits(c, true))
            .unwrap(),
        vec![s.customer]
    );

    let pmt = b
        .write(&me(), |w| {
            lb_store::apply::apply_customer_credits(w, s.customer)
        })
        .unwrap()
        .unwrap();
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
    assert_eq!(
        open(old),
        Some(Money::ZERO),
        "the oldest invoice is settled first"
    );
    assert_eq!(
        open(newer),
        Some(Money(15_000)),
        "the rest of the credit goes to the next"
    );
    assert_eq!(open(cm), Some(Money::ZERO), "the credit memo is used up");
    assert_eq!(open(pmt), Some(Money::ZERO));
    assert_eq!(
        b.read(|c| txns::get(c, pmt)).unwrap().date,
        date(2026, 3, 1)
    );
    let after = b.read(|c| balances::sums(c, None, None)).unwrap();
    assert_eq!(before, after, "applying a credit posts nothing");
    assert!(
        b.write(&me(), |w| lb_store::apply::apply_customer_credits(
            w, s.customer
        ))
        .unwrap()
        .is_none(),
        "nothing left to apply"
    );
}

#[test]
fn targeted_credit_application_stops_at_the_cap_and_prefers_credit_memos() {
    let b = books();
    let s = setup(&b);
    let mut inv = Txn::new(TxnType::Invoice, date(2026, 1, 10));
    inv.entity_id = Some(s.customer);
    inv.lines.push(TxnLine::item_amount(s.labor, Money(50_000)));
    let inv = b.write(&me(), |w| txns::save(w, inv)).unwrap().txn.id;
    // An older overpayment and a newer credit memo both sit unapplied.
    let mut over = Txn::new(TxnType::ReceivePayment, date(2026, 1, 5));
    over.entity_id = Some(s.customer);
    over.amount = Money(10_000);
    let over = b.write(&me(), |w| txns::save(w, over)).unwrap().txn.id;
    let mut cm = Txn::new(TxnType::CreditMemo, date(2026, 2, 1));
    cm.entity_id = Some(s.customer);
    cm.lines.push(TxnLine::item_amount(s.labor, Money(30_000)));
    let cm = b.write(&me(), |w| txns::save(w, cm)).unwrap().txn.id;
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

    b.write(&me(), |w| {
        lb_store::apply::apply_credits_to(w, inv, Money(20_000))
    })
    .unwrap()
    .unwrap();
    assert_eq!(open(inv), Some(Money(30_000)), "only the cap was applied");
    assert_eq!(open(cm), Some(Money(10_000)), "from the credit memo first");
    assert_eq!(
        open(over),
        Some(Money(10_000)),
        "the overpayment stays unapplied"
    );
}

#[test]
fn older_files_get_their_ar_links_backfilled() {
    let b = books();
    let s = setup(&b);
    let ar = b
        .read(|c| accounts::by_role(c, AccountRole::AccountsReceivable))
        .unwrap()
        .unwrap()
        .id;
    let mut chk = Txn::new(TxnType::Check, date(2026, 10, 1));
    chk.account_id = Some(s.bank);
    chk.lines
        .push(TxnLine::account(ar, Money(1_000)).with_entity(s.customer));
    let chk = b.write(&me(), |w| txns::save(w, chk)).unwrap().txn;
    // Pretend the file predates the link: strip it.
    b.write(&me(), |w| {
        w.tx.execute(
            "UPDATE txn SET ar_ap_account_id = NULL, open_balance = NULL, doc = json_remove(doc, '$.ar_ap_account_id') WHERE id = ?1",
            [chk.id.to_string()],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(b.write(&me(), txns::repair_ar_ap_links).unwrap(), 1);
    assert_eq!(
        b.write(&me(), txns::repair_ar_ap_links).unwrap(),
        0,
        "idempotent"
    );
    let open = b
        .read(|c| {
            Ok(txns::query(c, &txns::TxnFilter::default())?
                .into_iter()
                .find(|t| t.id == chk.id)
                .unwrap()
                .open_balance)
        })
        .unwrap();
    assert_eq!(open, Some(Money(1_000)));
}

#[test]
fn a_refund_check_made_out_to_another_name_is_still_the_customers_receivable() {
    let b = books();
    let s = setup(&b);
    let ar = b
        .read(|c| accounts::by_role(c, AccountRole::AccountsReceivable))
        .unwrap()
        .unwrap()
        .id;
    let other = b
        .write(&me(), |w| {
            entities::save(
                w,
                Entity::blank(EntityKind::OtherName, "Acme Accounts Payable Dept"),
            )
        })
        .unwrap()
        .id;
    let mut chk = Txn::new(TxnType::Check, date(2026, 10, 1));
    chk.account_id = Some(s.bank);
    chk.entity_id = Some(other);
    chk.lines
        .push(TxnLine::account(ar, Money(4_040)).with_entity(s.job));
    let chk = b.write(&me(), |w| txns::save(w, chk)).unwrap().txn;
    let mut pmt = Txn::new(TxnType::ReceivePayment, date(2026, 10, 10));
    pmt.entity_id = Some(s.customer);
    pmt.amount = Money(4_040);
    pmt.applications
        .push(Application::cash(chk.id, Money(4_040)));
    b.write(&me(), |w| txns::save(w, pmt)).unwrap();
    let open = b
        .read(|c| {
            Ok(txns::query(c, &txns::TxnFilter::default())?
                .into_iter()
                .find(|t| t.id == chk.id)
                .unwrap()
                .open_balance)
        })
        .unwrap();
    assert_eq!(open, Some(Money::ZERO));
}

#[test]
fn name_balances_follow_open_items_and_roll_jobs_up() {
    let b = books();
    let s = setup(&b);
    let mut inv = Txn::new(TxnType::Invoice, date(2026, 10, 1));
    inv.entity_id = Some(s.job);
    inv.lines.push(TxnLine::item(
        s.labor,
        Decimal::from(10),
        Decimal::from(220),
    ));
    let inv = b.write(&me(), |w| txns::save(w, inv)).unwrap().txn;
    let mut bill = Txn::new(TxnType::Bill, date(2026, 10, 2));
    bill.entity_id = Some(s.vendor);
    bill.lines.push(TxnLine::account(s.rent, Money(50_000)));
    b.write(&me(), |w| txns::save(w, bill)).unwrap();

    let by_name = |b: &Books| -> std::collections::HashMap<EntityId, lb_store::NameBalance> {
        b.read(balances::name_balances)
            .unwrap()
            .into_iter()
            .map(|n| (n.entity_id, n))
            .collect()
    };
    let n = by_name(&b);
    assert_eq!(n[&s.job].receivable, Money(220_000));
    assert_eq!(n[&s.customer].receivable, Money::ZERO, "nothing of its own");
    assert_eq!(
        n[&s.customer].receivable_total,
        Money(220_000),
        "the job rolls up"
    );
    assert_eq!(n[&s.vendor].payable, Money(50_000));
    assert_eq!(n[&s.vendor].receivable, Money::ZERO);

    // The customer pays most of the job's invoice: the job's balance falls,
    // the parent's total with it; a fully applied payment adds nothing.
    let mut pmt = Txn::new(TxnType::ReceivePayment, date(2026, 10, 5));
    pmt.entity_id = Some(s.customer);
    pmt.amount = Money(200_000);
    pmt.applications
        .push(Application::cash(inv.id, Money(200_000)));
    let pmt = b.write(&me(), |w| txns::save(w, pmt)).unwrap().txn;
    let n = by_name(&b);
    assert_eq!(n[&s.job].receivable, Money(20_000));
    assert_eq!(n[&s.customer].receivable_total, Money(20_000));

    // Unapplied, the payment is a credit against the customer.
    let mut pmt2 = b.read(|c| txns::get(c, pmt.id)).unwrap();
    pmt2.applications.clear();
    b.write(&me(), |w| txns::save(w, pmt2)).unwrap();
    let n = by_name(&b);
    assert_eq!(n[&s.job].receivable, Money(220_000));
    assert_eq!(n[&s.customer].receivable, Money(-200_000));
    assert_eq!(n[&s.customer].receivable_total, Money(20_000));
}

#[test]
fn an_items_opening_quantity_is_an_adjustment_against_opening_balance_equity() {
    let b = books();
    let s = setup(&b);
    let part = b
        .write(&me(), |w| {
            let cogs = accounts::save(
                w,
                accounts::new_account("COGS", AccountType::CostOfGoodsSold),
            )?
            .id;
            let asset = accounts::save(
                w,
                accounts::new_account("Inventory", AccountType::OtherCurrentAsset),
            )?
            .id;
            let mut i = Item::blank(ItemType::Inventory, "Widget");
            i.income_account_id = Some(s.income);
            i.expense_account_id = Some(cogs);
            i.asset_account_id = Some(asset);
            let id = items::save(w, i)?.id;
            items::set_opening_quantity(w, id, Decimal::from(10), Money(25_000), date(2026, 1, 1))?;
            Ok(id)
        })
        .unwrap();
    let item = b.read(|c| items::get(c, part)).unwrap();
    assert_eq!(item.quantity_on_hand, Decimal::from(10));
    assert_eq!(item.average_cost, Decimal::new(2500, 2));
    let obe = b
        .read(|c| accounts::by_role(c, AccountRole::OpeningBalanceEquity))
        .unwrap()
        .unwrap()
        .id;
    let sums = b.read(|c| balances::sums(c, None, None)).unwrap();
    assert_eq!(sums[&item.asset_account_id.unwrap()], Money(25_000));
    assert_eq!(sums[&obe], Money(-25_000));
    // Not for a service item, and never negative.
    assert!(
        b.write(&me(), |w| items::set_opening_quantity(
            w,
            s.labor,
            Decimal::ONE,
            Money(100),
            date(2026, 1, 1)
        ))
        .is_err()
    );
    assert_eq!(b.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn a_sales_tax_code_is_at_most_three_characters() {
    let b = books();
    let code = |name: &str| SalesTaxCode {
        id: SalesTaxCodeId::new(),
        name: name.into(),
        description: None,
        is_taxable: true,
        is_active: true,
        edit_seq: 0,
    };
    b.write(&me(), |w| lists::save(w, code("OOS"))).unwrap();
    assert!(matches!(
        b.write(&me(), |w| lists::save(w, code("Taxable"))),
        Err(StoreError::Invalid(_))
    ));
    assert!(b.write(&me(), |w| lists::save(w, code("  "))).is_err());
}

/// An inventory part with its own asset and COGS accounts.
fn widget(b: &Books, s: &Setup) -> ItemId {
    b.write(&me(), |w| {
        let cogs = accounts::save(
            w,
            accounts::new_account("COGS", AccountType::CostOfGoodsSold),
        )?
        .id;
        let asset = accounts::save(
            w,
            accounts::new_account("Inventory", AccountType::OtherCurrentAsset),
        )?
        .id;
        let mut i = Item::blank(ItemType::Inventory, "Widget");
        i.income_account_id = Some(s.income);
        i.expense_account_id = Some(cogs);
        i.asset_account_id = Some(asset);
        Ok(items::save(w, i)?.id)
    })
    .unwrap()
}

/// The stored open balance of a transaction, as `open_only` sees it.
fn open_balance(b: &Books, id: TxnId) -> Option<Money> {
    b.read(|c| {
        Ok(txns::query(
            c,
            &txns::TxnFilter {
                include_void: true,
                ..Default::default()
            },
        )?
        .into_iter()
        .find(|t| t.id == id)
        .unwrap()
        .open_balance)
    })
    .unwrap()
}

fn open_orders(b: &Books, t: TxnType, name: EntityId) -> Vec<TxnId> {
    b.read(|c| {
        Ok(txns::query(
            c,
            &txns::TxnFilter {
                types: vec![t],
                entity_id: Some(name),
                include_jobs: true,
                open_only: true,
                ..Default::default()
            },
        )?
        .into_iter()
        .map(|t| t.id)
        .collect())
    })
    .unwrap()
}

/// A transaction of `t` with one item line entered against `order`'s first line.
fn against(
    t: TxnType,
    d: jiff::civil::Date,
    order: &Txn,
    item: ItemId,
    qty: i64,
    rate: i64,
) -> Txn {
    let mut txn = Txn::new(t, d);
    txn.entity_id = order.entity_id;
    let mut line = TxnLine::item(item, Decimal::from(qty), Decimal::from(rate));
    line.linked_txn_id = Some(order.id);
    line.linked_line_id = Some(order.lines[0].id);
    txn.lines.push(line);
    txn
}

#[test]
fn a_purchase_order_is_received_in_parts_billed_from_the_receipt_and_closed() {
    let b = books();
    let s = setup(&b);
    let part = widget(&b, &s);
    let mut po = Txn::new(TxnType::PurchaseOrder, date(2026, 10, 1));
    po.entity_id = Some(s.vendor);
    po.number = Some("PO-1".into());
    po.lines
        .push(TxnLine::item(part, Decimal::from(10), Decimal::from(5)));
    po.lines
        .push(TxnLine::new(LineKind::Text).with_description("Deliver to the loading dock"));
    let po = b.write(&me(), |w| txns::save(w, po)).unwrap().txn;
    let line_id = po.lines[0].id;
    assert_eq!(po.amount, Money(5_000), "a PO's total is its lines'");
    assert_eq!(open_balance(&b, po.id), Some(Money(5_000)));
    assert_eq!(
        open_orders(&b, TxnType::PurchaseOrder, s.vendor),
        vec![po.id]
    );
    let f = b.read(|c| orders::fulfilment(c, po.id)).unwrap();
    assert!(!f.is_closed);
    assert_eq!(f.remaining_amount, Money(5_000));
    assert_eq!(f.lines.len(), 1, "the text line isn't fulfillable");
    assert_eq!(f.lines[0].line_id, line_id);
    assert_eq!(f.lines[0].ordered_quantity, Some(Decimal::from(10)));
    assert_eq!(f.lines[0].remaining_quantity, Some(Decimal::from(10)));

    // Receive 4 of the 10.
    let rcpt = against(TxnType::ItemReceipt, date(2026, 10, 5), &po, part, 4, 5);
    let rcpt = b.write(&me(), |w| txns::save(w, rcpt)).unwrap().txn;
    assert_eq!(open_balance(&b, po.id), Some(Money(3_000)));
    let f = b.read(|c| orders::fulfilment(c, po.id)).unwrap();
    assert!(!f.is_closed);
    assert_eq!(f.remaining_amount, Money(3_000));
    assert_eq!(f.lines[0].fulfilled_quantity, Decimal::from(4));
    assert_eq!(f.lines[0].fulfilled_amount, Money(2_000));
    assert_eq!(f.lines[0].remaining_quantity, Some(Decimal::from(6)));
    assert_eq!(f.lines[0].remaining_amount, Money(3_000));
    assert!(!f.lines[0].is_closed);
    // What was entered against the order, by one filter.
    let against_po: Vec<TxnId> = b
        .read(|c| {
            txns::query(
                c,
                &txns::TxnFilter {
                    linked_to: Some(po.id),
                    ..txns::TxnFilter::default()
                },
            )
        })
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(against_po, vec![rcpt.id]);

    // The bill for the received items links to the receipt, not the PO, and
    // doesn't count twice.
    let mut bill = Txn::new(TxnType::Bill, date(2026, 10, 7));
    bill.entity_id = Some(s.vendor);
    let mut l = TxnLine::item(part, Decimal::from(4), Decimal::from(5));
    l.linked_txn_id = Some(rcpt.id);
    l.linked_line_id = Some(rcpt.lines[0].id);
    bill.lines.push(l);
    let bill = b.write(&me(), |w| txns::save(w, bill)).unwrap().txn;
    assert_eq!(open_balance(&b, po.id), Some(Money(3_000)));
    assert_eq!(
        open_balance(&b, bill.id),
        Some(Money(2_000)),
        "the bill is owed"
    );
    assert!(
        b.read(|c| orders::fulfilment(c, rcpt.id)).is_err(),
        "a receipt isn't an order"
    );

    // Closing the PO by hand takes it off the open list; reopening it puts it back.
    let mut closed = b.read(|c| txns::get(c, po.id)).unwrap();
    closed.is_manually_closed = true;
    let closed = b.write(&me(), |w| txns::save(w, closed)).unwrap().txn;
    assert_eq!(open_balance(&b, po.id), Some(Money::ZERO));
    assert_eq!(open_orders(&b, TxnType::PurchaseOrder, s.vendor), []);
    let f = b.read(|c| orders::fulfilment(c, po.id)).unwrap();
    assert!(f.is_closed);
    assert!(f.lines[0].is_closed);
    assert_eq!(f.lines[0].remaining_quantity, Some(Decimal::ZERO));
    assert_eq!(f.lines[0].remaining_amount, Money::ZERO);
    assert_eq!(
        f.lines[0].fulfilled_quantity,
        Decimal::from(4),
        "history stays"
    );
    // Nothing more can be received against a closed order.
    assert!(matches!(
        b.write(&me(), |w| txns::save(
            w,
            against(TxnType::ItemReceipt, date(2026, 10, 8), &po, part, 1, 5)
        )),
        Err(StoreError::Invalid(_))
    ));
    // The receipt that already links to it can still be edited.
    let mut edit = b.read(|c| txns::get(c, rcpt.id)).unwrap();
    edit.memo = Some("left at the dock".into());
    b.write(&me(), |w| txns::save(w, edit)).unwrap();
    let mut reopened = b.read(|c| txns::get(c, closed.id)).unwrap();
    reopened.is_manually_closed = false;
    b.write(&me(), |w| txns::save(w, reopened)).unwrap();
    assert_eq!(open_balance(&b, po.id), Some(Money(3_000)));

    // The last 6 close it on their own; a late over-receipt is allowed and shown.
    let rest = against(TxnType::ItemReceipt, date(2026, 10, 9), &po, part, 6, 5);
    let rest = b.write(&me(), |w| txns::save(w, rest)).unwrap().txn;
    assert_eq!(open_balance(&b, po.id), Some(Money::ZERO));
    assert!(b.read(|c| orders::fulfilment(c, po.id)).unwrap().is_closed);
    let extra = against(TxnType::ItemReceipt, date(2026, 10, 10), &po, part, 2, 5);
    b.write(&me(), |w| txns::save(w, extra)).unwrap();
    let f = b.read(|c| orders::fulfilment(c, po.id)).unwrap();
    assert_eq!(f.lines[0].fulfilled_quantity, Decimal::from(12));
    assert_eq!(f.lines[0].remaining_quantity, Some(Decimal::ZERO));
    assert!(f.is_closed);

    // Voiding the receipt of 6 reopens the PO for 4 (12 − 6 = 6 received).
    b.write(&me(), |w| txns::void(w, rest.id, rest.edit_seq))
        .unwrap();
    assert_eq!(open_balance(&b, po.id), Some(Money(2_000)));
    assert_eq!(
        open_orders(&b, TxnType::PurchaseOrder, s.vendor),
        vec![po.id]
    );

    // The PO can't be deleted while receipts are entered against it.
    let po_now = b.read(|c| txns::get(c, po.id)).unwrap();
    assert!(matches!(
        b.write(&me(), |w| txns::delete(w, po.id, po_now.edit_seq)),
        Err(StoreError::Invalid(_))
    ));
    // Nor the receipt while the bill is made from it; once the bill goes,
    // deleting the receipt gives its 4 back.
    let rcpt_now = b.read(|c| txns::get(c, rcpt.id)).unwrap();
    assert!(matches!(
        b.write(&me(), |w| txns::delete(w, rcpt.id, rcpt_now.edit_seq)),
        Err(StoreError::Invalid(_))
    ));
    b.write(&me(), |w| txns::delete(w, bill.id, bill.edit_seq))
        .unwrap();
    b.write(&me(), |w| txns::delete(w, rcpt.id, rcpt_now.edit_seq))
        .unwrap();
    assert_eq!(open_balance(&b, po.id), Some(Money(4_000)));
    assert_eq!(b.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn a_sales_order_is_invoiced_in_part_by_the_jobs_customer() {
    let b = books();
    let s = setup(&b);
    let mut so = Txn::new(TxnType::SalesOrder, date(2026, 10, 1));
    so.entity_id = Some(s.job);
    so.lines.push(TxnLine::item(
        s.labor,
        Decimal::from(10),
        Decimal::from(220),
    ));
    let so = b.write(&me(), |w| txns::save(w, so)).unwrap().txn;
    assert_eq!(open_balance(&b, so.id), Some(Money(220_000)));
    assert_eq!(
        open_orders(&b, TxnType::SalesOrder, s.customer),
        vec![so.id]
    );

    // The invoice is made out to the customer; the order was for its job.
    let mut inv = against(TxnType::Invoice, date(2026, 10, 10), &so, s.labor, 4, 220);
    inv.entity_id = Some(s.customer);
    let inv = b.write(&me(), |w| txns::save(w, inv)).unwrap().txn;
    assert_eq!(open_balance(&b, so.id), Some(Money(132_000)));
    assert_eq!(
        open_balance(&b, inv.id),
        Some(Money(88_000)),
        "the invoice is owed"
    );
    let f = b.read(|c| orders::fulfilment(c, so.id)).unwrap();
    assert_eq!(f.lines[0].fulfilled_quantity, Decimal::from(4));
    assert_eq!(f.lines[0].remaining_quantity, Some(Decimal::from(6)));
    assert_eq!(f.remaining_amount, Money(132_000));
    assert!(!f.is_closed);

    // Closing the line by hand ("Clsd") closes the order with 6 never shipped.
    let mut so2 = b.read(|c| txns::get(c, so.id)).unwrap();
    so2.lines[0].is_closed = true;
    b.write(&me(), |w| txns::save(w, so2)).unwrap();
    let f = b.read(|c| orders::fulfilment(c, so.id)).unwrap();
    assert!(f.is_closed);
    assert_eq!(f.lines[0].fulfilled_quantity, Decimal::from(4));
    assert_eq!(open_balance(&b, so.id), Some(Money::ZERO));
    assert_eq!(open_orders(&b, TxnType::SalesOrder, s.customer), []);
    assert!(matches!(
        b.write(&me(), |w| txns::save(
            w,
            against(TxnType::Invoice, date(2026, 10, 12), &so, s.labor, 1, 220)
        )),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(b.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn an_estimate_is_progress_invoiced_by_amount() {
    let b = books();
    let s = setup(&b);
    let mut est = Txn::new(TxnType::Estimate, date(2026, 10, 1));
    est.entity_id = Some(s.customer);
    est.lines.push(TxnLine::item(
        s.labor,
        Decimal::from(10),
        Decimal::from(100),
    ));
    let est = b.write(&me(), |w| txns::save(w, est)).unwrap().txn;
    assert_eq!(open_balance(&b, est.id), Some(Money(100_000)));

    // A 50% progress invoice: an amount, no quantity.
    let mut inv = Txn::new(TxnType::Invoice, date(2026, 10, 10));
    inv.entity_id = Some(s.customer);
    let mut l = TxnLine::item_amount(s.labor, Money(50_000));
    l.linked_txn_id = Some(est.id);
    l.linked_line_id = Some(est.lines[0].id);
    inv.lines.push(l);
    b.write(&me(), |w| txns::save(w, inv)).unwrap();
    let f = b.read(|c| orders::fulfilment(c, est.id)).unwrap();
    assert_eq!(f.lines[0].fulfilled_amount, Money(50_000));
    assert_eq!(f.lines[0].remaining_amount, Money(50_000));
    assert_eq!(f.remaining_amount, Money(50_000));
    assert!(!f.is_closed);
    assert_eq!(open_balance(&b, est.id), Some(Money(50_000)));
    assert_eq!(open_orders(&b, TxnType::Estimate, s.customer), vec![est.id]);

    // The other half, as 5 @ 100, closes it; the quantity is along for the ride.
    let inv2 = against(TxnType::Invoice, date(2026, 10, 20), &est, s.labor, 5, 100);
    b.write(&me(), |w| txns::save(w, inv2)).unwrap();
    let f = b.read(|c| orders::fulfilment(c, est.id)).unwrap();
    assert!(f.is_closed);
    assert_eq!(f.lines[0].fulfilled_amount, Money(100_000));
    assert_eq!(f.lines[0].fulfilled_quantity, Decimal::from(5));
    assert_eq!(open_balance(&b, est.id), Some(Money::ZERO));
}

#[test]
fn links_to_orders_are_checked_on_save() {
    let b = books();
    let s = setup(&b);
    let part = widget(&b, &s);
    let (other_vendor, gadget) = b
        .write(&me(), |w| {
            let v = entities::save(w, Entity::blank(EntityKind::Vendor, "Other Supply"))?.id;
            let mut g = Item::blank(ItemType::NonInventory, "Gadget");
            g.income_account_id = Some(s.income);
            Ok((v, items::save(w, g)?.id))
        })
        .unwrap();
    let mut po = Txn::new(TxnType::PurchaseOrder, date(2026, 10, 1));
    po.entity_id = Some(s.vendor);
    po.lines
        .push(TxnLine::item(part, Decimal::from(10), Decimal::from(5)));
    let po = b.write(&me(), |w| txns::save(w, po)).unwrap().txn;
    let refused = |txn: Txn| {
        let err = b.write(&me(), |w| txns::save(w, txn)).unwrap_err();
        assert!(matches!(err, StoreError::Invalid(_)), "{err:?}");
        err.to_string()
    };

    // Another vendor's receipt.
    let mut wrong_vendor = against(TxnType::ItemReceipt, date(2026, 10, 2), &po, part, 1, 5);
    wrong_vendor.entity_id = Some(other_vendor);
    assert!(refused(wrong_vendor).contains("another vendor"));
    // A different item on the same line.
    let wrong_item = against(TxnType::ItemReceipt, date(2026, 10, 2), &po, gadget, 1, 5);
    assert!(refused(wrong_item).contains("different item"));
    // A line the PO doesn't have.
    let mut no_line = against(TxnType::ItemReceipt, date(2026, 10, 2), &po, part, 1, 5);
    no_line.lines[0].linked_line_id = Some(LineId::new());
    assert!(refused(no_line).contains("isn't on that"));
    // An order the books don't have.
    let mut no_order = against(TxnType::ItemReceipt, date(2026, 10, 2), &po, part, 1, 5);
    no_order.lines[0].linked_txn_id = Some(TxnId::new());
    assert!(refused(no_order).contains("doesn't exist"));
    // The wrong kind of form: an invoice against a PO, a check against an invoice.
    let mut inv = against(TxnType::Invoice, date(2026, 10, 2), &po, part, 1, 5);
    inv.entity_id = Some(s.customer);
    assert!(refused(inv).contains("can't be entered against"));
    let mut je = Txn::new(TxnType::JournalEntry, date(2026, 10, 2));
    let mut l = TxnLine::journal(s.rent, Money(100));
    l.linked_txn_id = Some(po.id);
    je.lines.push(l);
    je.lines.push(TxnLine::journal(s.bank, Money(-100)));
    assert!(refused(je).contains("can't be entered against"));
    // Without a line id, the item finds its line; a check may receive against a PO.
    let mut chk = Txn::new(TxnType::Check, date(2026, 10, 3));
    chk.entity_id = Some(s.vendor);
    chk.account_id = Some(s.bank);
    let mut l = TxnLine::item(part, Decimal::from(3), Decimal::from(5));
    l.linked_txn_id = Some(po.id);
    chk.lines.push(l);
    b.write(&me(), |w| txns::save(w, chk)).unwrap();
    assert_eq!(open_balance(&b, po.id), Some(Money(3_500)));
    let mut unknown_item = Txn::new(TxnType::ItemReceipt, date(2026, 10, 3));
    unknown_item.entity_id = Some(s.vendor);
    let mut l = TxnLine::item(gadget, Decimal::ONE, Decimal::from(5));
    l.linked_txn_id = Some(po.id);
    unknown_item.lines.push(l);
    assert!(refused(unknown_item).contains("no line for this item"));
    // A closed line takes nothing more.
    let mut po2 = b.read(|c| txns::get(c, po.id)).unwrap();
    po2.lines[0].is_closed = true;
    b.write(&me(), |w| txns::save(w, po2)).unwrap();
    assert!(
        refused(against(
            TxnType::ItemReceipt,
            date(2026, 10, 4),
            &po,
            part,
            1,
            5
        ))
        .contains("is closed")
    );
    assert_eq!(b.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn older_files_get_their_orders_open_balances_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("orders.lbdb");
    let info = CompanyInfo {
        name: "Older".into(),
        ..CompanyInfo::default()
    };
    let (po, so) = {
        let b = Books::create(&path, &info).unwrap();
        let s = setup(&b);
        let part = widget(&b, &s);
        let mut po = Txn::new(TxnType::PurchaseOrder, date(2026, 10, 1));
        po.entity_id = Some(s.vendor);
        po.lines
            .push(TxnLine::item(part, Decimal::from(10), Decimal::from(5)));
        let po = b.write(&me(), |w| txns::save(w, po)).unwrap().txn;
        let rcpt = against(TxnType::ItemReceipt, date(2026, 10, 5), &po, part, 4, 5);
        b.write(&me(), |w| txns::save(w, rcpt)).unwrap();
        let mut so = Txn::new(TxnType::SalesOrder, date(2026, 10, 1));
        so.entity_id = Some(s.customer);
        so.lines
            .push(TxnLine::item(s.labor, Decimal::ONE, Decimal::from(100)));
        let so = b.write(&me(), |w| txns::save(w, so)).unwrap().txn;
        // Make the file look like one written before order fulfilment.
        b.write(&Actor::system(), |w| {
            w.tx.execute_batch(
                "DROP TABLE txn_link;
                 UPDATE txn SET open_balance = NULL
                 WHERE txn_type IN ('PurchaseOrder', 'SalesOrder', 'Estimate');
                 PRAGMA user_version = 3;",
            )?;
            Ok(())
        })
        .unwrap();
        (po.id, so.id)
    };
    let b = Books::open(&path).unwrap();
    assert_eq!(open_balance(&b, po), Some(Money(3_000)), "links backfilled");
    assert_eq!(open_balance(&b, so), Some(Money(10_000)));
    let f = b.read(|c| orders::fulfilment(c, po)).unwrap();
    assert_eq!(f.lines[0].fulfilled_quantity, Decimal::from(4));
    assert_eq!(b.check_integrity().unwrap(), Vec::<String>::new());
}
