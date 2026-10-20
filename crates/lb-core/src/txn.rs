//! Transactions.
//!
//! A transaction is what a person fills in on a form: a header (date, number,
//! payee, the account it draws on), lines (expenses, items, journal lines) and, for
//! payments, which open transactions it pays. From that, [`crate::posting`]
//! derives the double-entry **postings**, which are what registers and reports
//! read. The form is kept as typed so that reopening a transaction shows exactly
//! what was entered; the postings are kept so the books never have to be
//! recomputed to be read.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::entity::Address;
use crate::ids::{
    AccountId, ClassId, EntityId, ItemId, LineId, PaymentMethodId, SalesRepId, SalesTaxCodeId,
    TermsId, TxnId, UserId,
};
use crate::money::Money;

/// Every QBD transaction type. Posting ones move money between accounts;
/// non-posting ones (estimates, orders, time) are kept with the same machinery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TxnType {
    // Banking
    Check,
    Deposit,
    Transfer,
    // General
    JournalEntry,
    // Purchases
    Bill,
    BillPaymentCheck,
    BillPaymentCreditCard,
    VendorCredit,
    ItemReceipt,
    CreditCardCharge,
    CreditCardCredit,
    PurchaseOrder,
    // Sales
    Invoice,
    SalesReceipt,
    ReceivePayment,
    CreditMemo,
    StatementCharge,
    Estimate,
    SalesOrder,
    // Inventory
    InventoryAdjustment,
    BuildAssembly,
    // Taxes and payroll
    SalesTaxPaymentCheck,
    Paycheck,
    LiabilityCheck,
    // Time
    TimeTracking,
}

impl TxnType {
    pub const ALL: [TxnType; 25] = [
        TxnType::Check,
        TxnType::Deposit,
        TxnType::Transfer,
        TxnType::JournalEntry,
        TxnType::Bill,
        TxnType::BillPaymentCheck,
        TxnType::BillPaymentCreditCard,
        TxnType::VendorCredit,
        TxnType::ItemReceipt,
        TxnType::CreditCardCharge,
        TxnType::CreditCardCredit,
        TxnType::PurchaseOrder,
        TxnType::Invoice,
        TxnType::SalesReceipt,
        TxnType::ReceivePayment,
        TxnType::CreditMemo,
        TxnType::StatementCharge,
        TxnType::Estimate,
        TxnType::SalesOrder,
        TxnType::InventoryAdjustment,
        TxnType::BuildAssembly,
        TxnType::SalesTaxPaymentCheck,
        TxnType::Paycheck,
        TxnType::LiabilityCheck,
        TxnType::TimeTracking,
    ];

    /// Does this type touch the books at all.
    pub const fn is_posting(self) -> bool {
        !matches!(
            self,
            TxnType::PurchaseOrder
                | TxnType::Estimate
                | TxnType::SalesOrder
                | TxnType::TimeTracking
        )
    }

    /// QBD's abbreviation, shown in registers' "Type" column.
    pub const fn register_code(self) -> &'static str {
        match self {
            TxnType::Check => "CHK",
            TxnType::Deposit => "DEP",
            TxnType::Transfer => "TRANSFR",
            TxnType::JournalEntry => "GENJRNL",
            TxnType::Bill => "BILL",
            TxnType::BillPaymentCheck | TxnType::BillPaymentCreditCard => "BILLPMT",
            TxnType::VendorCredit => "BILLCRED",
            TxnType::ItemReceipt => "ITEM RCPT",
            TxnType::CreditCardCharge => "CC",
            TxnType::CreditCardCredit => "CC CRED",
            TxnType::PurchaseOrder => "PURCHORD",
            TxnType::Invoice => "INV",
            TxnType::SalesReceipt => "RCPT",
            TxnType::ReceivePayment => "PMT",
            TxnType::CreditMemo => "CREDMEM",
            TxnType::StatementCharge => "STMTCHG",
            TxnType::Estimate => "EST",
            TxnType::SalesOrder => "SALESORD",
            TxnType::InventoryAdjustment => "INVADJ",
            TxnType::BuildAssembly => "BUILD",
            TxnType::SalesTaxPaymentCheck => "TAXPMT",
            TxnType::Paycheck => "PAY CHK",
            TxnType::LiabilityCheck => "LIAB CHK",
            TxnType::TimeTracking => "TIME",
        }
    }

    /// Name as reports and menus show it.
    pub const fn label(self) -> &'static str {
        match self {
            TxnType::Check => "Check",
            TxnType::Deposit => "Deposit",
            TxnType::Transfer => "Transfer",
            TxnType::JournalEntry => "General Journal",
            TxnType::Bill => "Bill",
            TxnType::BillPaymentCheck => "Bill Pmt -Check",
            TxnType::BillPaymentCreditCard => "Bill Pmt -CCard",
            TxnType::VendorCredit => "Credit",
            TxnType::ItemReceipt => "Item Receipt",
            TxnType::CreditCardCharge => "Credit Card Charge",
            TxnType::CreditCardCredit => "Credit Card Credit",
            TxnType::PurchaseOrder => "Purchase Order",
            TxnType::Invoice => "Invoice",
            TxnType::SalesReceipt => "Sales Receipt",
            TxnType::ReceivePayment => "Payment",
            TxnType::CreditMemo => "Credit Memo",
            TxnType::StatementCharge => "Statement Charge",
            TxnType::Estimate => "Estimate",
            TxnType::SalesOrder => "Sales Order",
            TxnType::InventoryAdjustment => "Inventory Adjust",
            TxnType::BuildAssembly => "Build Assembly",
            TxnType::SalesTaxPaymentCheck => "Sales Tax Payment",
            TxnType::Paycheck => "Paycheck",
            TxnType::LiabilityCheck => "Liability Check",
            TxnType::TimeTracking => "Time",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            TxnType::Check => "Check",
            TxnType::Deposit => "Deposit",
            TxnType::Transfer => "Transfer",
            TxnType::JournalEntry => "JournalEntry",
            TxnType::Bill => "Bill",
            TxnType::BillPaymentCheck => "BillPaymentCheck",
            TxnType::BillPaymentCreditCard => "BillPaymentCreditCard",
            TxnType::VendorCredit => "VendorCredit",
            TxnType::ItemReceipt => "ItemReceipt",
            TxnType::CreditCardCharge => "CreditCardCharge",
            TxnType::CreditCardCredit => "CreditCardCredit",
            TxnType::PurchaseOrder => "PurchaseOrder",
            TxnType::Invoice => "Invoice",
            TxnType::SalesReceipt => "SalesReceipt",
            TxnType::ReceivePayment => "ReceivePayment",
            TxnType::CreditMemo => "CreditMemo",
            TxnType::StatementCharge => "StatementCharge",
            TxnType::Estimate => "Estimate",
            TxnType::SalesOrder => "SalesOrder",
            TxnType::InventoryAdjustment => "InventoryAdjustment",
            TxnType::BuildAssembly => "BuildAssembly",
            TxnType::SalesTaxPaymentCheck => "SalesTaxPaymentCheck",
            TxnType::Paycheck => "Paycheck",
            TxnType::LiabilityCheck => "LiabilityCheck",
            TxnType::TimeTracking => "TimeTracking",
        }
    }

    pub fn parse(s: &str) -> Option<TxnType> {
        TxnType::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// Which name list the header's name comes from.
    pub const fn header_entity(self) -> HeaderEntity {
        match self {
            TxnType::Invoice
            | TxnType::SalesReceipt
            | TxnType::ReceivePayment
            | TxnType::CreditMemo
            | TxnType::StatementCharge
            | TxnType::Estimate
            | TxnType::SalesOrder => HeaderEntity::Customer,
            TxnType::Bill
            | TxnType::BillPaymentCheck
            | TxnType::BillPaymentCreditCard
            | TxnType::VendorCredit
            | TxnType::ItemReceipt
            | TxnType::PurchaseOrder
            | TxnType::SalesTaxPaymentCheck
            | TxnType::LiabilityCheck => HeaderEntity::Vendor,
            TxnType::Paycheck => HeaderEntity::Employee,
            TxnType::Check | TxnType::CreditCardCharge | TxnType::CreditCardCredit => {
                HeaderEntity::Any
            }
            TxnType::TimeTracking => HeaderEntity::Any,
            TxnType::Deposit
            | TxnType::Transfer
            | TxnType::JournalEntry
            | TxnType::InventoryAdjustment
            | TxnType::BuildAssembly => HeaderEntity::None,
        }
    }
}

/// Which names a header's name field offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum HeaderEntity {
    None,
    Customer,
    Vendor,
    Employee,
    Any,
}

/// What a line on a form is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum LineKind {
    /// An "Expenses" tab line: an account and an amount.
    Account,
    /// An "Items" tab line: an item, quantity, rate, amount.
    Item,
    /// A general journal line; `amount` is debit-positive.
    Journal,
    /// A deposit line: received from, from account, amount. May deposit a payment
    /// waiting in Undeposited Funds (`linked_txn_id`).
    DepositLine,
    /// The "Cash back goes to" line on a deposit.
    CashBack,
    /// A non-posting descriptive line (text only, or a subtotal).
    Text,
}

/// Whether an expense has been passed on to the customer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum BillableStatus {
    #[default]
    NotBillable,
    Billable,
    HasBeenBilled,
}

/// A line on a transaction's form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TxnLine {
    pub id: LineId,
    pub kind: LineKind,
    pub account_id: Option<AccountId>,
    pub item_id: Option<ItemId>,
    pub description: Option<String>,
    #[ts(type = "string | null")]
    pub quantity: Option<Decimal>,
    pub unit_of_measure: Option<String>,
    /// Price per unit, or a percent for percent items (`rate_is_percent`).
    #[ts(type = "string | null")]
    pub rate: Option<Decimal>,
    pub rate_is_percent: bool,
    /// The line's amount as shown on the form. For [`LineKind::Journal`] lines,
    /// debit-positive. For everything else, in the form's own sense (a positive
    /// expense on a check, a positive sale on an invoice).
    pub amount: Money,
    pub class_id: Option<ClassId>,
    /// The name on the line: the customer:job an expense is for, a journal line's
    /// name, a deposit line's "Received From".
    pub entity_id: Option<EntityId>,
    pub billable: BillableStatus,
    pub sales_tax_code_id: Option<SalesTaxCodeId>,
    /// For deposits: the payment method and check number of what is deposited.
    pub payment_method_id: Option<PaymentMethodId>,
    pub check_number: Option<String>,
    /// For deposit lines that move a payment out of Undeposited Funds; for item
    /// lines received against a purchase order or billed from a receipt.
    pub linked_txn_id: Option<TxnId>,
    pub linked_line_id: Option<LineId>,
    pub service_date: Option<jiff::civil::Date>,
    pub other1: Option<String>,
    pub other2: Option<String>,
    /// Inventory adjustments: quantity change; builds: quantity built.
    #[ts(type = "string | null")]
    pub quantity_change: Option<Decimal>,
    /// Anything an import carried that has no field yet, kept verbatim.
    #[ts(type = "Record<string, unknown> | null")]
    pub extra: Option<serde_json::Value>,
}

impl TxnLine {
    /// An empty line of a kind.
    pub fn new(kind: LineKind) -> TxnLine {
        TxnLine {
            id: LineId::new(),
            kind,
            account_id: None,
            item_id: None,
            description: None,
            quantity: None,
            unit_of_measure: None,
            rate: None,
            rate_is_percent: false,
            amount: Money::ZERO,
            class_id: None,
            entity_id: None,
            billable: BillableStatus::NotBillable,
            sales_tax_code_id: None,
            payment_method_id: None,
            check_number: None,
            linked_txn_id: None,
            linked_line_id: None,
            service_date: None,
            other1: None,
            other2: None,
            quantity_change: None,
            extra: None,
        }
    }

    /// An "Expenses"-tab line.
    pub fn account(account_id: AccountId, amount: Money) -> TxnLine {
        TxnLine {
            account_id: Some(account_id),
            amount,
            ..TxnLine::new(LineKind::Account)
        }
    }

    /// An "Items"-tab line with quantity and rate; the amount is extended.
    pub fn item(item_id: ItemId, quantity: Decimal, rate: Decimal) -> TxnLine {
        TxnLine {
            item_id: Some(item_id),
            quantity: Some(quantity),
            rate: Some(rate),
            amount: Money::extend(quantity, rate).unwrap_or(Money::ZERO),
            ..TxnLine::new(LineKind::Item)
        }
    }

    /// An item line with just an amount.
    pub fn item_amount(item_id: ItemId, amount: Money) -> TxnLine {
        TxnLine {
            item_id: Some(item_id),
            amount,
            ..TxnLine::new(LineKind::Item)
        }
    }

    /// A journal line: debit-positive.
    pub fn journal(account_id: AccountId, debit_positive: Money) -> TxnLine {
        TxnLine {
            account_id: Some(account_id),
            amount: debit_positive,
            ..TxnLine::new(LineKind::Journal)
        }
    }

    /// A deposit line from an account (income, or Undeposited Funds for a payment).
    pub fn deposit(from_account: AccountId, amount: Money) -> TxnLine {
        TxnLine {
            account_id: Some(from_account),
            amount,
            ..TxnLine::new(LineKind::DepositLine)
        }
    }

    #[must_use]
    pub fn with_entity(mut self, entity: EntityId) -> TxnLine {
        self.entity_id = Some(entity);
        self
    }

    #[must_use]
    pub fn with_class(mut self, class: ClassId) -> TxnLine {
        self.class_id = Some(class);
        self
    }

    #[must_use]
    pub fn with_description(mut self, d: &str) -> TxnLine {
        self.description = Some(d.to_owned());
        self
    }
}

/// A payment applied to an open transaction: a customer payment to an invoice, a
/// bill payment to a bill, a credit memo or vendor credit used up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Application {
    /// The invoice, bill, statement charge… being paid.
    pub target_txn_id: TxnId,
    /// Cash applied.
    pub amount: Money,
    /// Early-payment discount taken against it.
    pub discount: Money,
    pub discount_account_id: Option<AccountId>,
    pub discount_class_id: Option<ClassId>,
    /// A credit (credit memo, vendor credit) used against it instead of cash.
    pub credit_txn_id: Option<TxnId>,
    pub credit_amount: Money,
}

impl Application {
    pub fn cash(target: TxnId, amount: Money) -> Application {
        Application {
            target_txn_id: target,
            amount,
            discount: Money::ZERO,
            discount_account_id: None,
            discount_class_id: None,
            credit_txn_id: None,
            credit_amount: Money::ZERO,
        }
    }
}

/// Whether a register line has cleared the bank.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ClearedStatus {
    #[default]
    Uncleared,
    /// Checked off in a reconciliation not yet finished (QBD's `*`).
    Pending,
    /// Reconciled (QBD's check mark).
    Cleared,
}

/// A transaction, as entered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Txn {
    pub id: TxnId,
    pub txn_type: TxnType,
    pub date: jiff::civil::Date,
    /// Check number, invoice number, bill reference: QBD's `RefNumber`.
    pub number: Option<String>,
    /// Payee, customer, vendor, employee.
    pub entity_id: Option<EntityId>,
    /// The cash-side account the form draws on or pays into: the bank on a check,
    /// deposit or bill payment, the card on a charge, the "from" account on a
    /// transfer, the deposit-to account on a payment or sales receipt (where `None`
    /// means Undeposited Funds), the adjustment account on an inventory adjustment.
    /// A/R and A/P live in `ar_ap_account_id`.
    pub account_id: Option<AccountId>,
    /// Transfers: the "to" account.
    pub to_account_id: Option<AccountId>,
    /// The A/R or A/P account on forms that use one (invoices, credit memos,
    /// statement charges, payments; bills, vendor credits, item receipts, bill
    /// payments). `None` means the company's default A/R or A/P.
    pub ar_ap_account_id: Option<AccountId>,
    /// The amount the form shows in its total. Transfers, payments and sales tax
    /// payments carry it here; for line-based forms it equals the lines' sum and is
    /// checked against it.
    pub amount: Money,
    pub memo: Option<String>,
    pub class_id: Option<ClassId>,
    pub terms_id: Option<TermsId>,
    pub due_date: Option<jiff::civil::Date>,
    pub ship_date: Option<jiff::civil::Date>,
    pub discount_date: Option<jiff::civil::Date>,
    pub po_number: Option<String>,
    pub sales_rep_id: Option<SalesRepId>,
    pub payment_method_id: Option<PaymentMethodId>,
    /// Sales forms: the sales tax item (or group) and the customer's tax code.
    pub sales_tax_item_id: Option<ItemId>,
    pub customer_tax_code_id: Option<SalesTaxCodeId>,
    pub bill_address: Address,
    pub ship_address: Address,
    /// "Customer message" / "Vendor message" printed on the form.
    pub message: Option<String>,
    pub is_to_be_printed: bool,
    pub is_to_be_emailed: bool,
    /// QBD's pending invoices: entered but not yet posted.
    pub is_pending: bool,
    /// Voided: amounts zeroed, kept in the books with a note.
    pub is_void: bool,
    /// Adjusting entry (journal entries).
    pub is_adjustment: bool,
    /// Home currency when `None`.
    pub currency: Option<String>,
    #[ts(type = "string | null")]
    pub exchange_rate: Option<Decimal>,
    pub lines: Vec<TxnLine>,
    pub applications: Vec<Application>,
    /// Edit sequence (optimistic concurrency); 0 for a transaction not yet saved.
    #[ts(type = "number")]
    pub edit_seq: i64,
    pub created_at: Option<jiff::Timestamp>,
    pub modified_at: Option<jiff::Timestamp>,
    pub created_by: Option<UserId>,
    pub modified_by: Option<UserId>,
    /// Fields an import carried that have no home yet, kept verbatim.
    #[ts(type = "Record<string, unknown> | null")]
    pub extra: Option<serde_json::Value>,
}

impl Txn {
    /// A blank transaction of a type, dated `date`.
    pub fn new(txn_type: TxnType, date: jiff::civil::Date) -> Txn {
        Txn {
            id: TxnId::new(),
            txn_type,
            date,
            number: None,
            entity_id: None,
            account_id: None,
            to_account_id: None,
            ar_ap_account_id: None,
            amount: Money::ZERO,
            memo: None,
            class_id: None,
            terms_id: None,
            due_date: None,
            ship_date: None,
            discount_date: None,
            po_number: None,
            sales_rep_id: None,
            payment_method_id: None,
            sales_tax_item_id: None,
            customer_tax_code_id: None,
            bill_address: Address::default(),
            ship_address: Address::default(),
            message: None,
            is_to_be_printed: false,
            is_to_be_emailed: false,
            is_pending: false,
            is_void: false,
            is_adjustment: false,
            currency: None,
            exchange_rate: None,
            lines: Vec::new(),
            applications: Vec::new(),
            edit_seq: 0,
            created_at: None,
            modified_at: None,
            created_by: None,
            modified_by: None,
            extra: None,
        }
    }

    /// The sum of lines that carry money (everything but text and cash back).
    pub fn lines_total(&self) -> Money {
        self.lines
            .iter()
            .filter(|l| !matches!(l.kind, LineKind::Text | LineKind::CashBack))
            .map(|l| l.amount)
            .sum()
    }

    pub fn cash_back(&self) -> Money {
        self.lines
            .iter()
            .filter(|l| l.kind == LineKind::CashBack)
            .map(|l| l.amount)
            .sum()
    }
}

/// One side of the double entry. Debit-positive. A transaction's postings sum to
/// zero.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Posting {
    pub account_id: AccountId,
    pub amount: Money,
    pub entity_id: Option<EntityId>,
    pub class_id: Option<ClassId>,
    pub item_id: Option<ItemId>,
    /// The form line this came from; `None` for the header side (the bank on a
    /// check, A/R on an invoice).
    pub line_id: Option<LineId>,
    pub memo: Option<String>,
    /// Inventory quantity moved (positive in, negative out), for inventory postings.
    #[ts(type = "string | null")]
    pub quantity: Option<Decimal>,
    pub cleared: ClearedStatus,
}

impl Posting {
    pub fn new(account_id: AccountId, amount: Money) -> Posting {
        Posting {
            account_id,
            amount,
            entity_id: None,
            class_id: None,
            item_id: None,
            line_id: None,
            memo: None,
            quantity: None,
            cleared: ClearedStatus::Uncleared,
        }
    }
}
