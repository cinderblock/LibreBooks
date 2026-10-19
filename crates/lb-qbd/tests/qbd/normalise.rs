//! The normaliser alone, with a map-backed resolver: one-or-many, string scalars,
//! references, leftovers kept, links that can't be made.

use std::collections::HashMap;

use lb_core::lists::TypeListKind;
use lb_core::{
    AccountId, AccountRole, AccountType, CashFlowClass, ClassId, EntityId, EntityKind, ItemId,
    ItemType, LineKind, PaymentMethodId, SalesRepId, SalesTaxCodeId, TermsId, TxnId, TypeId,
};
use lb_qbd::QbdEntity;
use lb_qbd::qbxml::{self, NormaliseError, QbRef, Resolver};
use pretty_assertions::assert_eq;
use rust_decimal::Decimal;
use serde_json::json;
use uuid::Uuid;

/// Resolves every reference to a stable made-up id, except the ones listed as
/// missing.
#[derive(Default)]
struct Map {
    ids: HashMap<String, Uuid>,
    missing: Vec<String>,
}

impl Map {
    fn id(&mut self, r: &QbRef) -> Option<Uuid> {
        let key = r.list_id.clone().or_else(|| r.full_name.clone())?;
        if self.missing.contains(&key) {
            return None;
        }
        Some(*self.ids.entry(key).or_insert_with(Uuid::now_v7))
    }
}

impl Resolver for Map {
    fn account(&mut self, r: &QbRef) -> Result<Option<AccountId>, NormaliseError> {
        Ok(self.id(r).map(AccountId))
    }
    fn entity(&mut self, r: &QbRef) -> Result<Option<EntityId>, NormaliseError> {
        Ok(self.id(r).map(EntityId))
    }
    fn item(&mut self, r: &QbRef) -> Result<Option<ItemId>, NormaliseError> {
        Ok(self.id(r).map(ItemId))
    }
    fn class(&mut self, r: &QbRef) -> Result<Option<ClassId>, NormaliseError> {
        Ok(self.id(r).map(ClassId))
    }
    fn terms(&mut self, r: &QbRef) -> Result<Option<TermsId>, NormaliseError> {
        Ok(self.id(r).map(TermsId))
    }
    fn payment_method(&mut self, r: &QbRef) -> Result<Option<PaymentMethodId>, NormaliseError> {
        Ok(self.id(r).map(PaymentMethodId))
    }
    fn sales_rep(&mut self, r: &QbRef) -> Result<Option<SalesRepId>, NormaliseError> {
        Ok(self.id(r).map(SalesRepId))
    }
    fn sales_tax_code(&mut self, r: &QbRef) -> Result<Option<SalesTaxCodeId>, NormaliseError> {
        Ok(self.id(r).map(SalesTaxCodeId))
    }
    fn sales_tax_item(
        &mut self,
        r: &QbRef,
        _rate: Option<Decimal>,
    ) -> Result<Option<ItemId>, NormaliseError> {
        Ok(self.id(r).map(ItemId))
    }
    fn type_entry(
        &mut self,
        _list: TypeListKind,
        r: &QbRef,
    ) -> Result<Option<TypeId>, NormaliseError> {
        Ok(self.id(r).map(TypeId))
    }
    fn txn(&mut self, _t: Option<&str>, txn_id: &str) -> Result<Option<TxnId>, NormaliseError> {
        Ok(self
            .id(&QbRef {
                list_id: Some(txn_id.to_owned()),
                full_name: None,
            })
            .map(TxnId))
    }
}

fn r(id: &str) -> serde_json::Value {
    json!({"ListID": id, "FullName": id})
}

#[test]
fn accounts_map_type_role_and_details() {
    let v = json!({"ListID": "1-1", "EditSequence": "1", "Name": "Undeposited Funds", "FullName": "Undeposited Funds",
        "IsActive": "true", "Sublevel": "0", "AccountType": "OtherCurrentAsset", "SpecialAccountType": "UndepositedFunds",
        "AccountNumber": "1200", "Desc": "Payments not yet banked", "CashFlowClassification": "Operating",
        "TaxLineInfoRet": {"TaxLineID": "1", "TaxLineName": "B/S-Assets: Cash"}, "Balance": "0.00", "TotalBalance": "0.00",
        "CurrencyRef": r("USD")});
    let imp = qbxml::lists::account(&v, &mut Map::default()).unwrap();
    let a = imp.record;
    assert_eq!(a.account_type, AccountType::OtherCurrentAsset);
    assert_eq!(a.role, Some(AccountRole::UndepositedFunds));
    assert_eq!(a.number.as_deref(), Some("1200"));
    assert_eq!(a.tax_line.as_deref(), Some("B/S-Assets: Cash"));
    assert_eq!(a.cash_flow_class, Some(CashFlowClass::Operating));
    assert_eq!(imp.unmapped, vec!["Account.CurrencyRef".to_owned()]);

    let bad = json!({"Name": "X", "AccountType": "Mystery"});
    assert!(matches!(
        qbxml::lists::account(&bad, &mut Map::default()),
        Err(NormaliseError::Unsupported(_))
    ));
    let orphan = json!({"Name": "X", "AccountType": "Expense", "ParentRef": r("gone")});
    let mut map = Map {
        missing: vec!["gone".into()],
        ..Map::default()
    };
    let err = qbxml::lists::account(&orphan, &mut map).unwrap_err();
    assert_eq!(
        err.to_string(),
        "it refers to parent account \"gone\" (gone), which hasn't been imported"
    );
}

#[test]
fn bad_scalars_name_the_field() {
    let v = json!({"TxnID": "1-1", "TxnDate": "2026-13-45", "AccountRef": r("bank")});
    let err = qbxml::txns::txn(QbdEntity::Check, &v, &mut Map::default()).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("Check.TxnDate is \"2026-13-45\""),
        "{err}"
    );
    let v = json!({"TxnID": "1-1", "TxnDate": "2026-01-01", "AccountRef": r("bank"),
        "ExpenseLineRet": {"AccountRef": r("rent"), "Amount": "12.345"}});
    let err = qbxml::txns::txn(QbdEntity::Check, &v, &mut Map::default()).unwrap_err();
    assert!(
        err.to_string().contains("Check.ExpenseLineRet.Amount"),
        "{err}"
    );
}

#[test]
fn journal_entries_accept_one_or_many_lines_in_entry_order() {
    let v = json!({"TxnID": "A0-1", "TxnDate": "2026-01-31", "RefNumber": "7", "IsAdjustment": "true",
        "JournalCreditLine": [
            {"TxnLineID": "A2-1", "AccountRef": r("equity"), "Amount": "100.00"},
            {"TxnLineID": "A4-1", "AccountRef": r("ar"), "Amount": "25.00", "EntityRef": r("cust")}
        ],
        "JournalDebitLine": {"TxnLineID": "A3-1", "AccountRef": r("cash"), "Amount": "125.00", "Memo": "in",
            "BillableStatus": "Billable"}});
    let imp = qbxml::txns::txn(QbdEntity::JournalEntry, &v, &mut Map::default()).unwrap();
    let t = imp.txn;
    let got: Vec<(LineKind, i64)> = t.lines.iter().map(|l| (l.kind, l.amount.0)).collect();
    assert_eq!(
        got,
        vec![
            (LineKind::Journal, -10_000),
            (LineKind::Journal, 12_500),
            (LineKind::Journal, -2_500)
        ]
    );
    assert!(t.is_adjustment);
    assert!(t.lines[2].entity_id.is_some());
    assert_eq!(t.lines[1].description.as_deref(), Some("in"));
    assert_eq!(t.lines[1].extra.as_ref().unwrap()["TxnLineID"], "A3-1");
    assert_eq!(t.number.as_deref(), Some("7"));
}

#[test]
fn payments_report_links_they_cannot_make() {
    let v = json!({"TxnID": "P-1", "TxnDate": "2026-03-10", "CustomerRef": r("cust"), "TotalAmount": "100.00",
    "UnusedPayment": "0.00",
    "AppliedToTxnRet": [
        {"TxnID": "INV-1", "TxnType": "Invoice", "Amount": "60.00",
         "LinkedTxn": [
            {"TxnID": "CM-1", "TxnType": "CreditMemo", "Amount": "-5.00"},
            {"TxnID": "P-2", "TxnType": "ReceivePayment", "Amount": "-10.00"}
         ]},
        {"TxnID": "SC-1", "TxnType": "Charge", "Amount": "40.00"},
        {"TxnID": "INV-9", "TxnType": "Invoice", "Amount": "0.00"}
    ]});
    let mut map = Map {
        missing: vec!["SC-1".into(), "INV-9".into()],
        ..Map::default()
    };
    let imp = qbxml::txns::txn(QbdEntity::ReceivePayment, &v, &mut map).unwrap();
    // Cash to the invoice, the credit memo as a credit-only application; the other
    // payment linked to the same invoice is not a credit.
    assert_eq!(imp.txn.applications.len(), 2);
    assert_eq!(imp.txn.applications[0].amount.0, 6000);
    assert_eq!(imp.txn.applications[1].credit_amount.0, 500);
    assert_eq!(imp.txn.amount.0, 10_000);
    assert_eq!(imp.qbd_open_balance, Some(lb_core::Money::ZERO));
    let links: Vec<(String, bool)> = imp
        .missing_links
        .iter()
        .map(|l| (l.txn_id.clone(), l.exposed))
        .collect();
    assert_eq!(
        links,
        vec![("SC-1".to_owned(), false), ("INV-9".to_owned(), true)]
    );
    assert_eq!(
        imp.missing_links[0].to_string(),
        "payment applied to Charge SC-1 (the bridge doesn't expose that type)"
    );
}

#[test]
fn deposits_keep_unresolvable_payment_links_in_extra() {
    let v = json!({"TxnID": "D-1", "TxnDate": "2026-03-10", "DepositToAccountRef": r("bank"), "DepositTotal": "10.00",
        "DepositLineRet": {"TxnType": "ReceivePayment", "TxnID": "P-404", "TxnLineID": "D2-1",
            "AccountRef": r("uf"), "Amount": "10.00"}});
    let mut map = Map {
        missing: vec!["P-404".into()],
        ..Map::default()
    };
    let imp = qbxml::txns::txn(QbdEntity::Deposit, &v, &mut map).unwrap();
    let line = &imp.txn.lines[0];
    assert_eq!(line.kind, LineKind::DepositLine);
    assert_eq!(line.linked_txn_id, None);
    assert_eq!(line.extra.as_ref().unwrap()["TxnID"], "P-404");
    assert!(imp.missing_links[0].exposed);
    assert_eq!(imp.qbd_total.unwrap().0, 1000);
}

#[test]
fn items_by_type() {
    let disc = json!({"ListID": "I-1", "Name": "Loyalty", "ItemDesc": "10% off", "DiscountRatePercent": "10.0",
        "AccountRef": r("discounts")});
    let t = qbxml::lists::classify_item(&disc, None);
    let it = qbxml::lists::item(t, &disc, &mut Map::default())
        .unwrap()
        .record;
    assert_eq!(it.item_type, ItemType::Discount);
    assert!(it.is_percent);
    assert_eq!(it.sales_price, Some(Decimal::from(10)));
    let both = json!({"ListID": "I-2", "Name": "Install", "SalesAndPurchase": {"SalesPrice": "100.00",
        "IncomeAccountRef": r("inc"), "PurchaseCost": "60.00", "ExpenseAccountRef": r("exp"), "PrefVendorRef": r("v")}});
    let it = qbxml::lists::item(ItemType::Service, &both, &mut Map::default())
        .unwrap()
        .record;
    assert!(it.has_purchase_info);
    assert!(it.income_account_id.is_some() && it.expense_account_id.is_some());
    assert!(it.preferred_vendor_id.is_some());
    let empty_group = json!({"ListID": "I-3", "Name": "Reimb Group", "IsPrintItemsInGroup": "false",
        "SpecialItemType": "ReimbursableExpenseGroup"});
    assert!(matches!(
        qbxml::lists::item(ItemType::Group, &empty_group, &mut Map::default()),
        Err(NormaliseError::Unsupported(_))
    ));
}

#[test]
fn names_by_kind() {
    let v = json!({"ListID": "E-1", "Name": "Dana Smith", "IsActive": "false", "FirstName": "Dana",
        "EmployeeAddress": {"Addr1": "1 Example Ct", "Addr3": "Unit 2", "City": "Springfield"},
        "HiredDate": "2024-02-01", "SSN": "000-00-0000", "BirthDate": "1990-01-01", "Gender": "Female"});
    let imp = qbxml::lists::entity(EntityKind::Employee, &v, &mut Map::default()).unwrap();
    let e = imp.record;
    assert!(!e.is_active);
    assert_eq!(e.bill_address.lines, vec!["1 Example Ct", "Unit 2"]);
    assert_eq!(e.hired_date, Some(jiff::civil::date(2024, 2, 1)));
    // Personal data is neither imported nor reported as "unmapped".
    assert!(imp.unmapped.is_empty(), "{:?}", imp.unmapped);
    let json = serde_json::to_string(&e).unwrap();
    assert!(!json.contains("000-00-0000") && !json.contains("1990-01-01"));
}

#[test]
fn terms_both_kinds() {
    let std = json!({"ListID": "T-1", "Name": "2% 10 Net 30", "StdDueDays": "30", "StdDiscountDays": "10", "DiscountPct": "2.00"});
    let dd = json!({"ListID": "T-2", "Name": "15th", "DayOfMonthDue": "15", "DueNextMonthDays": "5", "DiscountPct": "0.00"});
    let s = qbxml::lists::terms(&std).unwrap().record;
    let d = qbxml::lists::terms(&dd).unwrap().record;
    assert_eq!(
        s.due_date(jiff::civil::date(2026, 1, 1)),
        jiff::civil::date(2026, 1, 31)
    );
    assert_eq!(
        d.due_date(jiff::civil::date(2026, 3, 12)),
        jiff::civil::date(2026, 4, 15)
    );
}
