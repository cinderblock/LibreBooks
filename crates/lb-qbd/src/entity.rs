//! What QuickBooks Desktop records the import knows about, and how each transport
//! serves them.
//!
//! The bridge's facts come from probing it live (see `plans/qbd-import.md`): which
//! paths exist, which lists can page with an iterator, and which records need a
//! second request to get their lines. The types with no bridge path are reached
//! only over qbXML (the Web Connector and the SDK). Keeping them in one table means
//! the client, the sync engine and the tests agree on them.

use lb_core::TxnType;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A kind of QBD record the import reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum QbdEntity {
    // Lists
    Account,
    Class,
    Terms,
    PaymentMethod,
    Vendor,
    Employee,
    OtherName,
    SalesRep,
    Customer,
    /// Every item type (the bridge's generic `/items`, which drops the type).
    Item,
    /// The typed item lists, used only to tell item types apart.
    ItemService,
    ItemNonInventory,
    ItemInventory,
    PayrollItemWage,
    // Transactions
    JournalEntry,
    Invoice,
    CreditMemo,
    SalesReceipt,
    Estimate,
    SalesOrder,
    PurchaseOrder,
    Bill,
    Check,
    TimeTracking,
    ReceivePayment,
    BillPaymentCheck,
    Deposit,
    // Transactions only qbXML reaches
    CreditCardCharge,
    CreditCardCredit,
    Transfer,
    VendorCredit,
    ItemReceipt,
    BillPaymentCreditCard,
    SalesTaxPaymentCheck,
    InventoryAdjustment,
    /// A statement charge.
    Charge,
}

impl QbdEntity {
    pub const ALL: [QbdEntity; 36] = [
        QbdEntity::Account,
        QbdEntity::Class,
        QbdEntity::Terms,
        QbdEntity::PaymentMethod,
        QbdEntity::Vendor,
        QbdEntity::Employee,
        QbdEntity::OtherName,
        QbdEntity::SalesRep,
        QbdEntity::Customer,
        QbdEntity::Item,
        QbdEntity::ItemService,
        QbdEntity::ItemNonInventory,
        QbdEntity::ItemInventory,
        QbdEntity::PayrollItemWage,
        QbdEntity::JournalEntry,
        QbdEntity::Invoice,
        QbdEntity::CreditMemo,
        QbdEntity::SalesReceipt,
        QbdEntity::Estimate,
        QbdEntity::SalesOrder,
        QbdEntity::PurchaseOrder,
        QbdEntity::Bill,
        QbdEntity::Check,
        QbdEntity::TimeTracking,
        QbdEntity::ReceivePayment,
        QbdEntity::BillPaymentCheck,
        QbdEntity::Deposit,
        QbdEntity::CreditCardCharge,
        QbdEntity::CreditCardCredit,
        QbdEntity::Transfer,
        QbdEntity::VendorCredit,
        QbdEntity::ItemReceipt,
        QbdEntity::BillPaymentCreditCard,
        QbdEntity::SalesTaxPaymentCheck,
        QbdEntity::InventoryAdjustment,
        QbdEntity::Charge,
    ];

    /// The order a sync imports in, so that everything a record points at is
    /// already there: accounts before anything that posts to them; vendors,
    /// employees and other names before sales reps (a rep *is* one of those);
    /// sales reps before customers (customers name their rep); items after the
    /// accounts and vendors they use; transactions that can be paid before the
    /// payments; receipts before the bills that bill them; payments before the
    /// deposits that bank them.
    pub const SYNC_ORDER: [QbdEntity; 32] = [
        QbdEntity::Account,
        QbdEntity::Class,
        QbdEntity::Terms,
        QbdEntity::PaymentMethod,
        QbdEntity::Vendor,
        QbdEntity::Employee,
        QbdEntity::OtherName,
        QbdEntity::SalesRep,
        QbdEntity::Customer,
        QbdEntity::Item,
        QbdEntity::JournalEntry,
        QbdEntity::Invoice,
        QbdEntity::Charge,
        QbdEntity::CreditMemo,
        QbdEntity::SalesReceipt,
        QbdEntity::Estimate,
        QbdEntity::SalesOrder,
        QbdEntity::PurchaseOrder,
        QbdEntity::ItemReceipt,
        QbdEntity::Bill,
        QbdEntity::VendorCredit,
        QbdEntity::Check,
        QbdEntity::CreditCardCharge,
        QbdEntity::CreditCardCredit,
        QbdEntity::Transfer,
        QbdEntity::TimeTracking,
        QbdEntity::ReceivePayment,
        QbdEntity::BillPaymentCheck,
        QbdEntity::BillPaymentCreditCard,
        QbdEntity::SalesTaxPaymentCheck,
        QbdEntity::InventoryAdjustment,
        QbdEntity::Deposit,
    ];

    /// QuickBooks' name for it (the bridge's `GET /api/v1/entities`, and qbXML's
    /// `TxnType`), also used as the external-reference kind in the store.
    pub const fn name(self) -> &'static str {
        match self {
            QbdEntity::Account => "Account",
            QbdEntity::Class => "Class",
            QbdEntity::Terms => "Terms",
            QbdEntity::PaymentMethod => "PaymentMethod",
            QbdEntity::Vendor => "Vendor",
            QbdEntity::Employee => "Employee",
            QbdEntity::OtherName => "OtherName",
            QbdEntity::SalesRep => "SalesRep",
            QbdEntity::Customer => "Customer",
            QbdEntity::Item => "Item",
            QbdEntity::ItemService => "ItemService",
            QbdEntity::ItemNonInventory => "ItemNonInventory",
            QbdEntity::ItemInventory => "ItemInventory",
            QbdEntity::PayrollItemWage => "PayrollItemWage",
            QbdEntity::JournalEntry => "JournalEntry",
            QbdEntity::Invoice => "Invoice",
            QbdEntity::CreditMemo => "CreditMemo",
            QbdEntity::SalesReceipt => "SalesReceipt",
            QbdEntity::Estimate => "Estimate",
            QbdEntity::SalesOrder => "SalesOrder",
            QbdEntity::PurchaseOrder => "PurchaseOrder",
            QbdEntity::Bill => "Bill",
            QbdEntity::Check => "Check",
            QbdEntity::TimeTracking => "TimeTracking",
            QbdEntity::ReceivePayment => "ReceivePayment",
            QbdEntity::BillPaymentCheck => "BillPaymentCheck",
            QbdEntity::Deposit => "Deposit",
            QbdEntity::CreditCardCharge => "CreditCardCharge",
            QbdEntity::CreditCardCredit => "CreditCardCredit",
            QbdEntity::Transfer => "Transfer",
            QbdEntity::VendorCredit => "VendorCredit",
            QbdEntity::ItemReceipt => "ItemReceipt",
            QbdEntity::BillPaymentCreditCard => "BillPaymentCreditCard",
            QbdEntity::SalesTaxPaymentCheck => "SalesTaxPaymentCheck",
            QbdEntity::InventoryAdjustment => "InventoryAdjustment",
            QbdEntity::Charge => "Charge",
        }
    }

    pub fn from_name(s: &str) -> Option<QbdEntity> {
        QbdEntity::ALL.into_iter().find(|e| e.name() == s)
    }

    /// The bridge path, under the base URL. `None`: the bridge doesn't have it.
    pub const fn path(self) -> Option<&'static str> {
        Some(match self {
            QbdEntity::Account => "/api/v1/accounts",
            QbdEntity::Class => "/api/v1/classes",
            QbdEntity::Terms => "/api/v1/terms",
            QbdEntity::PaymentMethod => "/api/v1/payment-methods",
            QbdEntity::Vendor => "/api/v1/vendors",
            QbdEntity::Employee => "/api/v1/employees",
            QbdEntity::OtherName => "/api/v1/other-names",
            QbdEntity::SalesRep => "/api/v1/sales-reps",
            QbdEntity::Customer => "/api/v1/customers",
            QbdEntity::Item => "/api/v1/items",
            QbdEntity::ItemService => "/api/v1/items/service",
            QbdEntity::ItemNonInventory => "/api/v1/items/non-inventory",
            QbdEntity::ItemInventory => "/api/v1/items/inventory",
            QbdEntity::PayrollItemWage => "/api/v1/payroll-items/wage",
            QbdEntity::JournalEntry => "/api/v1/journal-entries",
            QbdEntity::Invoice => "/api/v1/invoices",
            QbdEntity::CreditMemo => "/api/v1/credit-memos",
            QbdEntity::SalesReceipt => "/api/v1/sales-receipts",
            QbdEntity::Estimate => "/api/v1/estimates",
            QbdEntity::SalesOrder => "/api/v1/sales-orders",
            QbdEntity::PurchaseOrder => "/api/v1/purchase-orders",
            QbdEntity::Bill => "/api/v1/bills",
            QbdEntity::Check => "/api/v1/checks",
            QbdEntity::TimeTracking => "/api/v1/time-tracking",
            QbdEntity::ReceivePayment => "/api/v1/payments",
            QbdEntity::BillPaymentCheck => "/api/v1/bill-payments",
            QbdEntity::Deposit => "/api/v1/deposits",
            _ => return None,
        })
    }

    pub const fn is_transaction(self) -> bool {
        self.txn_type().is_some()
    }

    /// A receivable a customer's credits can settle (not itself a credit).
    pub const fn is_charge(self) -> bool {
        matches!(
            self,
            QbdEntity::Invoice | QbdEntity::Charge | QbdEntity::JournalEntry | QbdEntity::Check
        )
    }

    /// The LibreBooks transaction type it becomes.
    pub const fn txn_type(self) -> Option<TxnType> {
        Some(match self {
            QbdEntity::JournalEntry => TxnType::JournalEntry,
            QbdEntity::Invoice => TxnType::Invoice,
            QbdEntity::CreditMemo => TxnType::CreditMemo,
            QbdEntity::SalesReceipt => TxnType::SalesReceipt,
            QbdEntity::Estimate => TxnType::Estimate,
            QbdEntity::SalesOrder => TxnType::SalesOrder,
            QbdEntity::PurchaseOrder => TxnType::PurchaseOrder,
            QbdEntity::Bill => TxnType::Bill,
            QbdEntity::Check => TxnType::Check,
            QbdEntity::TimeTracking => TxnType::TimeTracking,
            QbdEntity::ReceivePayment => TxnType::ReceivePayment,
            QbdEntity::BillPaymentCheck => TxnType::BillPaymentCheck,
            QbdEntity::Deposit => TxnType::Deposit,
            QbdEntity::CreditCardCharge => TxnType::CreditCardCharge,
            QbdEntity::CreditCardCredit => TxnType::CreditCardCredit,
            QbdEntity::Transfer => TxnType::Transfer,
            QbdEntity::VendorCredit => TxnType::VendorCredit,
            QbdEntity::ItemReceipt => TxnType::ItemReceipt,
            QbdEntity::BillPaymentCreditCard => TxnType::BillPaymentCreditCard,
            QbdEntity::SalesTaxPaymentCheck => TxnType::SalesTaxPaymentCheck,
            QbdEntity::InventoryAdjustment => TxnType::InventoryAdjustment,
            QbdEntity::Charge => TxnType::StatementCharge,
            _ => return None,
        })
    }

    /// The QBD entity for a qbXML `TxnType` value (as in `LinkedTxn` and
    /// `AppliedToTxnRet`), when the import knows the type.
    pub fn for_qbxml_txn_type(s: &str) -> Option<QbdEntity> {
        QbdEntity::ALL
            .into_iter()
            .find(|e| e.is_transaction() && e.name() == s)
    }

    /// Whether the bridge can page this list with an iterator. Probed: Account,
    /// Terms, Employee, OtherName and TimeTracking answer `ITERATOR_NOT_SUPPORTED`;
    /// the other small lists are assumed not to (qbXML's queries for them have no
    /// iterator). The client also learns this at run time, so a wrong guess costs
    /// one request.
    pub const fn supports_iterator(self) -> bool {
        !matches!(
            self,
            QbdEntity::Account
                | QbdEntity::Class
                | QbdEntity::Terms
                | QbdEntity::PaymentMethod
                | QbdEntity::Employee
                | QbdEntity::OtherName
                | QbdEntity::SalesRep
                | QbdEntity::PayrollItemWage
                | QbdEntity::TimeTracking
        )
    }

    /// Transactions whose list rows leave out their lines (and, for payments,
    /// what they pay), so each one needs `GET {path}/{id}`. A time entry, transfer
    /// or statement charge has no lines to leave out.
    pub const fn needs_detail(self) -> bool {
        self.is_transaction()
            && !matches!(
                self,
                QbdEntity::TimeTracking | QbdEntity::Transfer | QbdEntity::Charge
            )
    }

    /// The qbXML element a query response holds one of these in (`CheckRet`), for
    /// the Web Connector transport. `Item` has one element per item type; see
    /// [`crate::qbxml::lists::item_type_for_ret`].
    pub const fn ret_element(self) -> &'static str {
        match self {
            QbdEntity::Account => "AccountRet",
            QbdEntity::Class => "ClassRet",
            QbdEntity::Terms => "StandardTermsRet",
            QbdEntity::PaymentMethod => "PaymentMethodRet",
            QbdEntity::Vendor => "VendorRet",
            QbdEntity::Employee => "EmployeeRet",
            QbdEntity::OtherName => "OtherNameRet",
            QbdEntity::SalesRep => "SalesRepRet",
            QbdEntity::Customer => "CustomerRet",
            QbdEntity::Item => "ItemRet",
            QbdEntity::ItemService => "ItemServiceRet",
            QbdEntity::ItemNonInventory => "ItemNonInventoryRet",
            QbdEntity::ItemInventory => "ItemInventoryRet",
            QbdEntity::PayrollItemWage => "PayrollItemWageRet",
            QbdEntity::JournalEntry => "JournalEntryRet",
            QbdEntity::Invoice => "InvoiceRet",
            QbdEntity::CreditMemo => "CreditMemoRet",
            QbdEntity::SalesReceipt => "SalesReceiptRet",
            QbdEntity::Estimate => "EstimateRet",
            QbdEntity::SalesOrder => "SalesOrderRet",
            QbdEntity::PurchaseOrder => "PurchaseOrderRet",
            QbdEntity::Bill => "BillRet",
            QbdEntity::Check => "CheckRet",
            QbdEntity::TimeTracking => "TimeTrackingRet",
            QbdEntity::ReceivePayment => "ReceivePaymentRet",
            QbdEntity::BillPaymentCheck => "BillPaymentCheckRet",
            QbdEntity::Deposit => "DepositRet",
            QbdEntity::CreditCardCharge => "CreditCardChargeRet",
            QbdEntity::CreditCardCredit => "CreditCardCreditRet",
            QbdEntity::Transfer => "TransferRet",
            QbdEntity::VendorCredit => "VendorCreditRet",
            QbdEntity::ItemReceipt => "ItemReceiptRet",
            QbdEntity::BillPaymentCreditCard => "BillPaymentCreditCardRet",
            QbdEntity::SalesTaxPaymentCheck => "SalesTaxPaymentCheckRet",
            QbdEntity::InventoryAdjustment => "InventoryAdjustmentRet",
            QbdEntity::Charge => "ChargeRet",
        }
    }

    /// The field holding its id.
    pub const fn id_field(self) -> &'static str {
        if self.is_transaction() {
            "TxnID"
        } else {
            "ListID"
        }
    }

    /// Lists with a parent/child hierarchy (`ParentRef`, `Sublevel`).
    pub const fn is_hierarchical(self) -> bool {
        matches!(
            self,
            QbdEntity::Account | QbdEntity::Class | QbdEntity::Customer | QbdEntity::Item
        )
    }
}

/// qbXML `TxnType`s no transport imports yet. Paychecks and liability checks have
/// no query of their own, only `TransactionQueryRq` rows; a build's record carries
/// no amounts; LibreBooks has no card refund type. Links to them can't be imported,
/// and their postings are missing from the parity comparison.
pub const UNIMPORTED_TXN_TYPES: [&str; 6] = [
    "ARRefundCreditCard",
    "BuildAssembly",
    "LiabilityAdjustment",
    "Paycheck",
    "PayrollLiabilityCheck",
    "YTDAdjustment",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_paths_are_unique() {
        let mut paths = std::collections::HashSet::new();
        for e in QbdEntity::ALL {
            assert_eq!(QbdEntity::from_name(e.name()), Some(e));
            if let Some(path) = e.path() {
                assert!(paths.insert(path), "duplicate path {path}");
            }
        }
    }

    #[test]
    fn sync_order_puts_dependencies_first() {
        let pos = |e: QbdEntity| QbdEntity::SYNC_ORDER.iter().position(|x| *x == e);
        assert!(pos(QbdEntity::Account) < pos(QbdEntity::Item));
        assert!(pos(QbdEntity::Employee) < pos(QbdEntity::SalesRep));
        assert!(pos(QbdEntity::SalesRep) < pos(QbdEntity::Customer));
        assert!(pos(QbdEntity::Invoice) < pos(QbdEntity::ReceivePayment));
        assert!(pos(QbdEntity::Bill) < pos(QbdEntity::BillPaymentCheck));
        assert!(pos(QbdEntity::ReceivePayment) < pos(QbdEntity::Deposit));
        assert!(pos(QbdEntity::SalesReceipt) < pos(QbdEntity::Deposit));
        assert!(pos(QbdEntity::Charge) < pos(QbdEntity::ReceivePayment));
        assert!(pos(QbdEntity::ItemReceipt) < pos(QbdEntity::Bill));
        assert!(pos(QbdEntity::VendorCredit) < pos(QbdEntity::BillPaymentCheck));
        assert!(pos(QbdEntity::Bill) < pos(QbdEntity::BillPaymentCreditCard));
        // Every transaction type is synced.
        for e in QbdEntity::ALL {
            if e.is_transaction() {
                assert!(pos(e).is_some(), "{e:?} is never synced");
            }
        }
    }

    #[test]
    fn qbxml_txn_types_map_back() {
        assert_eq!(
            QbdEntity::for_qbxml_txn_type("ReceivePayment"),
            Some(QbdEntity::ReceivePayment)
        );
        assert_eq!(
            QbdEntity::for_qbxml_txn_type("CreditCardCharge"),
            Some(QbdEntity::CreditCardCharge)
        );
        assert_eq!(QbdEntity::for_qbxml_txn_type("Paycheck"), None);
        assert_eq!(QbdEntity::for_qbxml_txn_type("Account"), None);
    }
}
