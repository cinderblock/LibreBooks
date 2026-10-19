//! A small company for report tests: a chart with subaccounts, customers with
//! jobs and a sub-job, vendors (one a 1099 vendor), service and discount items,
//! and helpers to enter transactions the way a person would.

#![allow(dead_code)]

use jiff::civil::Date;
use lb_core::lists::SalesTaxCode;
use lb_core::*;
use lb_reports::{Report, ReportId, ReportRequest};
use lb_store::{Actor, Books, accounts, entities, items, lists, txns};
use rust_decimal::Decimal;

pub struct Co {
    pub b: Books,
    pub bank: AccountId,
    pub consulting: AccountId,
    pub materials: AccountId,
    pub discounts: AccountId,
    pub rent: AccountId,
    pub utilities: AccountId,
    pub internet: AccountId,
    pub supplies: AccountId,
    pub truck: AccountId,
    pub loan: AccountId,
    pub ar: AccountId,
    pub ap: AccountId,
    pub obe: AccountId,
    pub re: AccountId,
    pub acme: EntityId,
    pub phase1: EntityId,
    pub design: EntityId,
    pub phase2: EntityId,
    pub globex: EntityId,
    pub landlord: EntityId,
    pub contractor: EntityId,
    pub labor: ItemId,
    pub parts: ItemId,
    pub discount: ItemId,
}

pub fn d(y: i16, m: i8, day: i8) -> Date {
    jiff::civil::date(y, m, day)
}

pub fn company(fiscal_start: i8) -> Co {
    company_named("Test Co", fiscal_start)
}

pub fn company_named(name: &str, fiscal_start: i8) -> Co {
    let info = CompanyInfo {
        name: name.into(),
        fiscal_year_start_month: fiscal_start,
        ..CompanyInfo::default()
    };
    let b = Books::in_memory(&info).unwrap();
    let a = Actor::system();
    let acct = |b: &Books, name: &str, t: AccountType, parent: Option<AccountId>| {
        b.write(&a, |w| {
            let mut x = accounts::new_account(name, t);
            x.parent_id = parent;
            Ok(accounts::save(w, x)?.id)
        })
        .unwrap()
    };
    let bank = acct(&b, "Checking", AccountType::Bank, None);
    let consulting = acct(&b, "Consulting Income", AccountType::Income, None);
    let materials = acct(&b, "Materials Income", AccountType::Income, None);
    let discounts = acct(&b, "Discounts Given", AccountType::Income, None);
    let rent = acct(&b, "Rent", AccountType::Expense, None);
    let utilities = acct(&b, "Utilities", AccountType::Expense, None);
    let internet = acct(&b, "Internet", AccountType::Expense, Some(utilities));
    let supplies = acct(&b, "Supplies", AccountType::Expense, None);
    let truck = acct(&b, "Truck", AccountType::FixedAsset, None);
    let loan = acct(&b, "Truck Loan", AccountType::LongTermLiability, None);
    let role = |r: AccountRole| {
        b.read(|c| accounts::by_role(c, r))
            .unwrap()
            .expect("role account")
            .id
    };
    let ar = role(AccountRole::AccountsReceivable);
    let ap = role(AccountRole::AccountsPayable);
    let obe = role(AccountRole::OpeningBalanceEquity);
    let re = role(AccountRole::RetainedEarnings);
    let ent = |b: &Books, kind: EntityKind, name: &str, parent: Option<EntityId>, is_1099: bool| {
        b.write(&a, |w| {
            let mut e = Entity::blank(kind, name);
            e.parent_id = parent;
            e.is_1099 = is_1099;
            Ok(entities::save(w, e)?.id)
        })
        .unwrap()
    };
    let acme = ent(&b, EntityKind::Customer, "Acme", None, false);
    let phase1 = ent(&b, EntityKind::Customer, "Phase 1", Some(acme), false);
    let design = ent(&b, EntityKind::Customer, "Design", Some(phase1), false);
    let phase2 = ent(&b, EntityKind::Customer, "Phase 2", Some(acme), false);
    let globex = ent(&b, EntityKind::Customer, "Globex", None, false);
    let landlord = ent(&b, EntityKind::Vendor, "Landlord", None, false);
    let contractor = ent(&b, EntityKind::Vendor, "Contractor", None, true);
    let item = |b: &Books, t: ItemType, name: &str, income: AccountId| {
        b.write(&a, |w| {
            let mut i = Item::blank(t, name);
            i.income_account_id = Some(income);
            Ok(items::save(w, i)?.id)
        })
        .unwrap()
    };
    let labor = item(&b, ItemType::Service, "Labor", consulting);
    let parts = item(&b, ItemType::Service, "Parts", materials);
    let discount = item(&b, ItemType::Discount, "Discount", discounts);
    Co {
        b,
        bank,
        consulting,
        materials,
        discounts,
        rent,
        utilities,
        internet,
        supplies,
        truck,
        loan,
        ar,
        ap,
        obe,
        re,
        acme,
        phase1,
        design,
        phase2,
        globex,
        landlord,
        contractor,
        labor,
        parts,
        discount,
    }
}

impl Co {
    pub fn save(&self, t: Txn) -> Txn {
        self.b
            .write(&Actor::system(), |w| txns::save(w, t))
            .unwrap()
            .txn
    }

    /// An invoice of item lines `(item, amount in cents)`.
    pub fn invoice(&self, date: Date, customer: EntityId, lines: &[(ItemId, i64)]) -> Txn {
        let mut t = Txn::new(TxnType::Invoice, date);
        t.entity_id = Some(customer);
        for (i, amt) in lines {
            t.lines.push(TxnLine::item_amount(*i, Money(*amt)));
        }
        self.save(t)
    }

    pub fn invoice_due(&self, date: Date, due: Date, customer: EntityId, amount: i64) -> Txn {
        let mut t = Txn::new(TxnType::Invoice, date);
        t.entity_id = Some(customer);
        t.due_date = Some(due);
        t.lines
            .push(TxnLine::item_amount(self.labor, Money(amount)));
        self.save(t)
    }

    pub fn credit_memo(&self, date: Date, customer: EntityId, amount: i64) -> Txn {
        let mut t = Txn::new(TxnType::CreditMemo, date);
        t.entity_id = Some(customer);
        t.lines
            .push(TxnLine::item_amount(self.labor, Money(amount)));
        self.save(t)
    }

    /// A customer payment deposited straight to the bank, applied as given.
    pub fn payment(
        &self,
        date: Date,
        customer: EntityId,
        amount: i64,
        apply: &[(TxnId, i64)],
    ) -> Txn {
        let mut t = Txn::new(TxnType::ReceivePayment, date);
        t.entity_id = Some(customer);
        t.account_id = Some(self.bank);
        t.amount = Money(amount);
        for (target, a) in apply {
            t.applications.push(Application::cash(*target, Money(*a)));
        }
        self.save(t)
    }

    /// A check from the bank with expense lines `(account, cents)`.
    pub fn check(
        &self,
        date: Date,
        payee: EntityId,
        number: &str,
        lines: &[(AccountId, i64)],
    ) -> Txn {
        let mut t = Txn::new(TxnType::Check, date);
        t.account_id = Some(self.bank);
        t.entity_id = Some(payee);
        t.number = Some(number.into());
        for (acct, amt) in lines {
            t.lines.push(TxnLine::account(*acct, Money(*amt)));
        }
        self.save(t)
    }

    pub fn bill(&self, date: Date, due: Date, vendor: EntityId, lines: &[(AccountId, i64)]) -> Txn {
        let mut t = Txn::new(TxnType::Bill, date);
        t.entity_id = Some(vendor);
        t.due_date = Some(due);
        for (acct, amt) in lines {
            t.lines.push(TxnLine::account(*acct, Money(*amt)));
        }
        self.save(t)
    }

    pub fn bill_payment(
        &self,
        date: Date,
        vendor: EntityId,
        number: &str,
        apply: &[(TxnId, i64)],
    ) -> Txn {
        let mut t = Txn::new(TxnType::BillPaymentCheck, date);
        t.entity_id = Some(vendor);
        t.account_id = Some(self.bank);
        t.number = Some(number.into());
        t.amount = Money(apply.iter().map(|x| x.1).sum());
        for (target, a) in apply {
            t.applications.push(Application::cash(*target, Money(*a)));
        }
        self.save(t)
    }

    /// A journal entry of `(account, debit-positive cents)` lines.
    pub fn journal(&self, date: Date, lines: &[(AccountId, i64)]) -> Txn {
        let mut t = Txn::new(TxnType::JournalEntry, date);
        for (acct, amt) in lines {
            t.lines.push(TxnLine::journal(*acct, Money(*amt)));
        }
        self.save(t)
    }

    /// A 10% sales tax item paid to a new agency, and a taxable code.
    pub fn sales_tax(&self) -> (ItemId, SalesTaxCodeId, EntityId) {
        self.b
            .write(&Actor::system(), |w| {
                let agency = entities::save(w, Entity::blank(EntityKind::Vendor, "State Tax"))?.id;
                let mut t = Item::blank(ItemType::SalesTax, "State 10%");
                t.tax_rate = Some(Decimal::from(10));
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
                Ok((t, code, agency))
            })
            .unwrap()
    }

    pub fn run(&self, req: &ReportRequest) -> Report {
        lb_reports::run(&self.b, req).unwrap_or_else(|e| panic!("{:?} failed: {e}", req.report))
    }
}

/// A request for an explicit range, with `today` fixed.
pub fn req(id: ReportId, from: Date, to: Date) -> ReportRequest {
    ReportRequest::new(id)
        .range(Some(from), Some(to))
        .today(d(2026, 10, 19))
}

/// The amount in a row and column, or zero when the row is absent.
pub fn amt(r: &Report, label: &str, col: &str) -> i64 {
    r.amount(label, col).map_or(0, |m| m.0)
}

/// Every pair of rows labelled `a` and `b` agree in every money column.
pub fn same_amounts(r: &Report, a: &str, b: &str) {
    let ra = r.row(a).unwrap_or_else(|| panic!("no row {a}"));
    let rb = r.row(b).unwrap_or_else(|| panic!("no row {b}"));
    for (i, (x, y)) in ra.cells.iter().zip(&rb.cells).enumerate().skip(1) {
        assert_eq!(
            x.as_money(),
            y.as_money(),
            "{a} and {b} differ in column {}",
            r.columns[i].label
        );
    }
}
