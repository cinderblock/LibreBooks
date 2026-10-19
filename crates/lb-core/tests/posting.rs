//! Posting rules, one QBD form at a time, checked against what QBD itself posts.

use jiff::civil::date;
use lb_core::testing::MemoryBooks;
use lb_core::*;
use pretty_assertions::assert_eq;
use rust_decimal_macros::dec;

fn d() -> jiff::civil::Date {
    date(2026, 10, 1)
}

/// Postings as (account name, amount in cents), sorted, for readable assertions.
fn shape(books: &MemoryBooks, posted: &Posted) -> Vec<(String, i64)> {
    let mut v: Vec<(String, i64)> = posted
        .postings
        .iter()
        .map(|p| (books.name_of(p.account_id).to_owned(), p.amount.minor()))
        .collect();
    v.sort();
    v
}

fn pairs(v: &[(&str, i64)]) -> Vec<(String, i64)> {
    let mut v: Vec<(String, i64)> = v.iter().map(|(a, m)| ((*a).to_owned(), *m)).collect();
    v.sort();
    v
}

struct Fixture {
    b: MemoryBooks,
    bank: AccountId,
    card: AccountId,
    rent: AccountId,
    supplies: AccountId,
    income: AccountId,
    discounts: AccountId,
    vendor: EntityId,
    customer: EntityId,
}

fn fixture() -> Fixture {
    let mut b = MemoryBooks::new();
    let bank = b.add_account("Checking", AccountType::Bank);
    let card = b.add_account("Visa", AccountType::CreditCard);
    let rent = b.add_account("Rent", AccountType::Expense);
    let supplies = b.add_account("Supplies", AccountType::Expense);
    let income = b.add_account("Consulting Income", AccountType::Income);
    let discounts = b.add_account("Discounts", AccountType::Income);
    b.add_role(AccountRole::AccountsReceivable);
    b.add_role(AccountRole::AccountsPayable);
    b.add_role(AccountRole::UndepositedFunds);
    b.add_role(AccountRole::SalesTaxPayable);
    b.add_role(AccountRole::InventoryAsset);
    b.add_role(AccountRole::CostOfGoodsSold);
    let vendor = b.add_entity("Comcast", EntityKind::Vendor);
    let customer = b.add_entity("Acme", EntityKind::Customer);
    Fixture {
        b,
        bank,
        card,
        rent,
        supplies,
        income,
        discounts,
        vendor,
        customer,
    }
}

#[test]
fn check_credits_the_bank_and_debits_each_line() {
    let f = fixture();
    let mut t = Txn::new(TxnType::Check, d());
    t.account_id = Some(f.bank);
    t.entity_id = Some(f.vendor);
    t.lines.push(TxnLine::account(f.rent, Money(100_000)));
    t.lines.push(TxnLine::account(f.supplies, Money(2_550)));
    let p = post(&t, &f.b).unwrap();
    assert_eq!(p.total, Money(102_550));
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[
            ("Checking", -102_550),
            ("Rent", 100_000),
            ("Supplies", 2_550)
        ])
    );
    // The bank side carries the payee.
    let bank = p.postings.iter().find(|p| p.account_id == f.bank).unwrap();
    assert_eq!(bank.entity_id, Some(f.vendor));
    assert_eq!(bank.line_id, None);
}

#[test]
fn check_from_an_income_account_is_refused() {
    let f = fixture();
    let mut t = Txn::new(TxnType::Check, d());
    t.account_id = Some(f.income);
    t.lines.push(TxnLine::account(f.rent, Money(1)));
    assert!(matches!(
        post(&t, &f.b),
        Err(PostingError::WrongAccountType { .. })
    ));
}

#[test]
fn credit_card_charge_and_credit() {
    let f = fixture();
    let mut t = Txn::new(TxnType::CreditCardCharge, d());
    t.account_id = Some(f.card);
    t.lines.push(TxnLine::account(f.supplies, Money(4_999)));
    let p = post(&t, &f.b).unwrap();
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[("Visa", -4_999), ("Supplies", 4_999)])
    );

    let mut c = Txn::new(TxnType::CreditCardCredit, d());
    c.account_id = Some(f.card);
    c.lines.push(TxnLine::account(f.supplies, Money(4_999)));
    let p = post(&c, &f.b).unwrap();
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[("Visa", 4_999), ("Supplies", -4_999)])
    );
}

#[test]
fn deposit_with_undeposited_funds_and_cash_back() {
    let f = fixture();
    let uf = f.b.role_account(AccountRole::UndepositedFunds).unwrap();
    let mut t = Txn::new(TxnType::Deposit, d());
    t.account_id = Some(f.bank);
    t.lines
        .push(TxnLine::deposit(uf, Money(300_000)).with_entity(f.customer));
    t.lines.push(TxnLine::deposit(f.income, Money(25_233)));
    let mut cb = TxnLine::new(LineKind::CashBack);
    cb.account_id = Some(f.supplies);
    cb.amount = Money(5_000);
    t.lines.push(cb);
    let p = post(&t, &f.b).unwrap();
    assert_eq!(p.total, Money(320_233));
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[
            ("Checking", 320_233),
            ("Undeposited Funds", -300_000),
            ("Consulting Income", -25_233),
            ("Supplies", 5_000),
        ])
    );
}

#[test]
fn transfer_moves_between_two_accounts() {
    let f = fixture();
    let mut t = Txn::new(TxnType::Transfer, d());
    t.account_id = Some(f.bank);
    t.to_account_id = Some(f.card);
    t.amount = Money(50_000);
    let p = post(&t, &f.b).unwrap();
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[("Checking", -50_000), ("Visa", 50_000)])
    );

    t.to_account_id = Some(f.bank);
    assert_eq!(post(&t, &f.b), Err(PostingError::TransferToSameAccount));
}

#[test]
fn journal_entry_must_balance_and_name_ar_lines() {
    let f = fixture();
    let ar = f.b.role_account(AccountRole::AccountsReceivable).unwrap();
    let mut t = Txn::new(TxnType::JournalEntry, d());
    t.lines.push(TxnLine::journal(f.rent, Money(10_000)));
    t.lines.push(TxnLine::journal(f.bank, Money(-9_000)));
    assert_eq!(post(&t, &f.b), Err(PostingError::Unbalanced(Money(1_000))));

    t.lines[1].amount = Money(-10_000);
    let p = post(&t, &f.b).unwrap();
    assert_eq!(p.total, Money(10_000));

    let mut je = Txn::new(TxnType::JournalEntry, d());
    je.lines.push(TxnLine::journal(ar, Money(500)));
    je.lines.push(TxnLine::journal(f.income, Money(-500)));
    assert_eq!(post(&je, &f.b), Err(PostingError::ArApLineWithoutName));
    je.lines[0].entity_id = Some(f.customer);
    assert!(post(&je, &f.b).is_ok());
}

#[test]
fn bill_and_vendor_credit_go_through_ap() {
    let f = fixture();
    let mut t = Txn::new(TxnType::Bill, d());
    t.entity_id = Some(f.vendor);
    t.lines.push(TxnLine::account(f.rent, Money(120_000)));
    let p = post(&t, &f.b).unwrap();
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[("Accounts Payable", -120_000), ("Rent", 120_000)])
    );
    let ap = p.postings.iter().find(|p| p.line_id.is_none()).unwrap();
    assert_eq!(ap.entity_id, Some(f.vendor));

    t.txn_type = TxnType::VendorCredit;
    let p = post(&t, &f.b).unwrap();
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[("Accounts Payable", 120_000), ("Rent", -120_000)])
    );
}

#[test]
fn bill_needs_a_vendor() {
    let f = fixture();
    let mut t = Txn::new(TxnType::Bill, d());
    t.lines.push(TxnLine::account(f.rent, Money(1)));
    assert_eq!(post(&t, &f.b), Err(PostingError::MissingEntity));
    t.entity_id = Some(f.customer);
    assert!(matches!(
        post(&t, &f.b),
        Err(PostingError::WrongEntityKind { .. })
    ));
}

#[test]
fn invoice_with_sales_tax() {
    let mut f = fixture();
    let taxable = f.b.add_tax_code(true);
    let non = f.b.add_tax_code(false);
    let agency = f.b.add_entity("State Board", EntityKind::Vendor);
    let mut tax = Item::blank(ItemType::SalesTax, "CA");
    tax.tax_rate = Some(dec!(8.25));
    tax.tax_agency_id = Some(agency);
    let tax_id = f.b.add_item(tax);
    let widget_id = {
        let mut i = Item::blank(ItemType::NonInventory, "Widget");
        i.income_account_id = Some(f.income);
        i.sales_tax_code_id = Some(taxable);
        f.b.add_item(i)
    };
    let labor = f.b.add_service("Labor", f.income);

    let mut t = Txn::new(TxnType::Invoice, d());
    t.entity_id = Some(f.customer);
    t.sales_tax_item_id = Some(tax_id);
    t.lines.push(TxnLine::item(widget_id, dec!(2), dec!(50)));
    let mut l = TxnLine::item(labor, dec!(3), dec!(100));
    l.sales_tax_code_id = Some(non);
    t.lines.push(l);
    let p = post(&t, &f.b).unwrap();
    assert_eq!(p.sales_tax, Money(825));
    assert_eq!(p.total, Money(40_825));
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[
            ("Accounts Receivable", 40_825),
            ("Consulting Income", -10_000),
            ("Consulting Income", -30_000),
            ("Sales Tax Payable", -825),
        ])
    );
    let stp = p
        .postings
        .iter()
        .find(|p| p.item_id == Some(tax_id))
        .unwrap();
    assert_eq!(stp.entity_id, Some(agency));

    // A non-taxable customer pays no tax at all.
    t.customer_tax_code_id = Some(non);
    let p = post(&t, &f.b).unwrap();
    assert_eq!(p.sales_tax, Money::ZERO);
    assert_eq!(p.total, Money(40_000));
}

#[test]
fn invoice_of_inventory_moves_cost_to_cogs() {
    let mut f = fixture();
    let asset = f.b.role_account(AccountRole::InventoryAsset).unwrap();
    let mut part = Item::blank(ItemType::Inventory, "Gear");
    part.income_account_id = Some(f.income);
    part.asset_account_id = Some(asset);
    let part_id = f.b.add_item(part);
    f.b.costs.insert(part_id, dec!(12.50));

    let mut t = Txn::new(TxnType::Invoice, d());
    t.entity_id = Some(f.customer);
    t.lines.push(TxnLine::item(part_id, dec!(4), dec!(40)));
    let p = post(&t, &f.b).unwrap();
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[
            ("Accounts Receivable", 16_000),
            ("Consulting Income", -16_000),
            ("Cost of Goods Sold", 5_000),
            ("Inventory Asset", -5_000),
        ])
    );
    let a = p.postings.iter().find(|p| p.account_id == asset).unwrap();
    assert_eq!(a.quantity, Some(dec!(-4)));

    // A credit memo returns it.
    t.txn_type = TxnType::CreditMemo;
    let p = post(&t, &f.b).unwrap();
    let a = p.postings.iter().find(|p| p.account_id == asset).unwrap();
    assert_eq!(a.amount, Money(5_000));
    assert_eq!(a.quantity, Some(dec!(4)));
}

#[test]
fn sales_receipt_defaults_to_undeposited_funds() {
    let mut f = fixture();
    let labor = f.b.add_service("Labor", f.income);
    let mut t = Txn::new(TxnType::SalesReceipt, d());
    t.entity_id = Some(f.customer);
    t.lines.push(TxnLine::item(labor, dec!(1), dec!(99.99)));
    let p = post(&t, &f.b).unwrap();
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[("Undeposited Funds", 9_999), ("Consulting Income", -9_999)])
    );
    t.account_id = Some(f.bank);
    let p = post(&t, &f.b).unwrap();
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[("Checking", 9_999), ("Consulting Income", -9_999)])
    );
}

#[test]
fn receive_payment_with_discount_and_overpayment() {
    let mut f = fixture();
    let ar = f.b.role_account(AccountRole::AccountsReceivable).unwrap();
    let inv = TxnId::new();
    f.b.open.insert(
        inv,
        OpenTxnInfo {
            txn_type: TxnType::Invoice,
            entity_id: Some(f.customer),
            ar_ap_account_id: Some(ar),
        },
    );
    let mut t = Txn::new(TxnType::ReceivePayment, d());
    t.entity_id = Some(f.customer);
    t.amount = Money(10_000);
    let mut a = Application::cash(inv, Money(9_800));
    a.discount = Money(200);
    a.discount_account_id = Some(f.discounts);
    t.applications.push(a);
    let p = post(&t, &f.b).unwrap();
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[
            ("Undeposited Funds", 10_000),
            ("Accounts Receivable", -10_000),
            ("Discounts", 200),
            ("Accounts Receivable", -200),
        ])
    );

    t.applications[0].amount = Money(10_001);
    assert_eq!(
        post(&t, &f.b),
        Err(PostingError::OverApplied {
            applied: Money(10_001),
            amount: Money(10_000)
        })
    );
}

#[test]
fn bill_payment_by_check_and_by_card() {
    let mut f = fixture();
    let ap = f.b.role_account(AccountRole::AccountsPayable).unwrap();
    let bill = TxnId::new();
    f.b.open.insert(
        bill,
        OpenTxnInfo {
            txn_type: TxnType::Bill,
            entity_id: Some(f.vendor),
            ar_ap_account_id: Some(ap),
        },
    );
    let mut t = Txn::new(TxnType::BillPaymentCheck, d());
    t.entity_id = Some(f.vendor);
    t.account_id = Some(f.bank);
    t.amount = Money(120_000);
    t.applications.push(Application::cash(bill, Money(120_000)));
    let p = post(&t, &f.b).unwrap();
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[("Checking", -120_000), ("Accounts Payable", 120_000)])
    );

    t.txn_type = TxnType::BillPaymentCreditCard;
    assert!(matches!(
        post(&t, &f.b),
        Err(PostingError::WrongAccountType { .. })
    ));
    t.account_id = Some(f.card);
    let p = post(&t, &f.b).unwrap();
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[("Visa", -120_000), ("Accounts Payable", 120_000)])
    );
}

#[test]
fn void_keeps_shape_with_zero_amounts() {
    let f = fixture();
    let mut t = Txn::new(TxnType::Check, d());
    t.account_id = Some(f.bank);
    t.lines.push(TxnLine::account(f.rent, Money(100)));
    t.is_void = true;
    let p = post(&t, &f.b).unwrap();
    assert_eq!(p.total, Money::ZERO);
    assert_eq!(p.postings.len(), 2);
    assert!(p.postings.iter().all(|p| p.amount.is_zero()));
}

#[test]
fn pending_and_non_posting_post_nothing() {
    let mut f = fixture();
    let labor = f.b.add_service("Labor", f.income);
    let mut t = Txn::new(TxnType::Invoice, d());
    t.entity_id = Some(f.customer);
    t.lines.push(TxnLine::item(labor, dec!(1), dec!(10)));
    t.is_pending = true;
    assert_eq!(
        post(&t, &f.b).unwrap().postings,
        [] as [lb_core::Posting; 0]
    );

    let mut e = Txn::new(TxnType::Estimate, d());
    e.lines.push(TxnLine::item(labor, dec!(1), dec!(10)));
    let p = post(&e, &f.b).unwrap();
    assert_eq!(p.postings, [] as [lb_core::Posting; 0]);
    assert_eq!(p.total, Money(1_000));
}

#[test]
fn missing_role_account_is_reported_for_the_store_to_create() {
    let mut b = MemoryBooks::new();
    let rent = b.add_account("Rent", AccountType::Expense);
    let vendor = b.add_entity("V", EntityKind::Vendor);
    let mut t = Txn::new(TxnType::Bill, d());
    t.entity_id = Some(vendor);
    t.lines.push(TxnLine::account(rent, Money(1)));
    assert_eq!(
        post(&t, &b),
        Err(PostingError::MissingRoleAccount(
            AccountRole::AccountsPayable
        ))
    );
}

#[test]
fn inventory_adjustment_against_an_adjustment_account() {
    let mut f = fixture();
    let asset = f.b.role_account(AccountRole::InventoryAsset).unwrap();
    let shrink =
        f.b.add_account("Inventory Shrinkage", AccountType::CostOfGoodsSold);
    let mut part = Item::blank(ItemType::Inventory, "Gear");
    part.asset_account_id = Some(asset);
    let part_id = f.b.add_item(part);
    let mut t = Txn::new(TxnType::InventoryAdjustment, d());
    t.account_id = Some(shrink);
    let mut l = TxnLine::item_amount(part_id, Money(-2_500));
    l.quantity_change = Some(dec!(-2));
    t.lines.push(l);
    let p = post(&t, &f.b).unwrap();
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[("Inventory Asset", -2_500), ("Inventory Shrinkage", 2_500)])
    );
}

#[test]
fn sales_tax_payment_names_the_agency() {
    let mut f = fixture();
    let stp = f.b.role_account(AccountRole::SalesTaxPayable).unwrap();
    let agency = f.b.add_entity("State Board", EntityKind::Vendor);
    let mut t = Txn::new(TxnType::SalesTaxPaymentCheck, d());
    t.entity_id = Some(agency);
    t.account_id = Some(f.bank);
    t.amount = Money(825);
    let p = post(&t, &f.b).unwrap();
    assert_eq!(
        shape(&f.b, &p),
        pairs(&[("Checking", -825), ("Sales Tax Payable", 825)])
    );
    let s = p.postings.iter().find(|p| p.account_id == stp).unwrap();
    assert_eq!(s.entity_id, Some(agency));
}
