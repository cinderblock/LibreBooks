//! The chart of accounts.
//!
//! Account types are QuickBooks Desktop's, one for one, so that an imported chart
//! keeps its meaning and reports group the same way. Amounts posted to an account
//! are signed debit-positive; an account's *natural* balance (what a register or a
//! report shows) flips the sign for credit-normal accounts.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::AccountId;
use crate::money::Money;

/// QuickBooks Desktop's account types (`AccountType` in qbXML).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum AccountType {
    Bank,
    AccountsReceivable,
    OtherCurrentAsset,
    FixedAsset,
    OtherAsset,
    AccountsPayable,
    CreditCard,
    OtherCurrentLiability,
    LongTermLiability,
    Equity,
    Income,
    CostOfGoodsSold,
    Expense,
    OtherIncome,
    OtherExpense,
    /// Estimates, purchase orders, sales orders: never touches the books.
    NonPosting,
}

/// The five classes of account in double entry, plus non-posting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum AccountClass {
    Asset,
    Liability,
    Equity,
    Income,
    Expense,
    NonPosting,
}

impl AccountType {
    pub const ALL: [AccountType; 16] = [
        AccountType::Bank,
        AccountType::AccountsReceivable,
        AccountType::OtherCurrentAsset,
        AccountType::FixedAsset,
        AccountType::OtherAsset,
        AccountType::AccountsPayable,
        AccountType::CreditCard,
        AccountType::OtherCurrentLiability,
        AccountType::LongTermLiability,
        AccountType::Equity,
        AccountType::Income,
        AccountType::CostOfGoodsSold,
        AccountType::Expense,
        AccountType::OtherIncome,
        AccountType::OtherExpense,
        AccountType::NonPosting,
    ];

    pub const fn class(self) -> AccountClass {
        match self {
            AccountType::Bank
            | AccountType::AccountsReceivable
            | AccountType::OtherCurrentAsset
            | AccountType::FixedAsset
            | AccountType::OtherAsset => AccountClass::Asset,
            AccountType::AccountsPayable
            | AccountType::CreditCard
            | AccountType::OtherCurrentLiability
            | AccountType::LongTermLiability => AccountClass::Liability,
            AccountType::Equity => AccountClass::Equity,
            AccountType::Income | AccountType::OtherIncome => AccountClass::Income,
            AccountType::CostOfGoodsSold | AccountType::Expense | AccountType::OtherExpense => {
                AccountClass::Expense
            }
            AccountType::NonPosting => AccountClass::NonPosting,
        }
    }

    /// Debit-normal accounts show a debit balance as positive.
    pub const fn is_debit_normal(self) -> bool {
        matches!(self.class(), AccountClass::Asset | AccountClass::Expense)
    }

    /// Balance sheet accounts carry their balance forward; income statement
    /// accounts start each fiscal year at zero (closed into retained earnings).
    pub const fn is_balance_sheet(self) -> bool {
        matches!(
            self.class(),
            AccountClass::Asset | AccountClass::Liability | AccountClass::Equity
        )
    }

    pub const fn is_income_statement(self) -> bool {
        matches!(self.class(), AccountClass::Income | AccountClass::Expense)
    }

    /// Accounts with a register a person balances against a statement.
    pub const fn is_reconcilable(self) -> bool {
        matches!(
            self,
            AccountType::Bank
                | AccountType::CreditCard
                | AccountType::OtherCurrentAsset
                | AccountType::OtherCurrentLiability
                | AccountType::FixedAsset
                | AccountType::OtherAsset
                | AccountType::LongTermLiability
        )
    }

    /// Accounts that have a register at all (QBD: everything but income/expense
    /// and non-posting).
    pub const fn has_register(self) -> bool {
        self.is_balance_sheet()
    }

    /// Accounts a check, deposit or transfer can draw on or pay into.
    pub const fn is_cash_like(self) -> bool {
        matches!(
            self,
            AccountType::Bank | AccountType::CreditCard | AccountType::OtherCurrentAsset
        )
    }

    /// Lines in these accounts need a name (customer for A/R, vendor for A/P).
    pub const fn requires_name(self) -> bool {
        matches!(
            self,
            AccountType::AccountsReceivable | AccountType::AccountsPayable
        )
    }

    /// Display order of the types on the chart of accounts and the reports.
    pub const fn sort_order(self) -> u8 {
        self as u8
    }

    /// The natural (displayed) balance for a debit-positive sum.
    pub const fn natural(self, debit_positive: Money) -> Money {
        if self.is_debit_normal() {
            debit_positive
        } else {
            Money(-debit_positive.0)
        }
    }

    /// Short label as QBD shows it in the chart of accounts.
    pub const fn label(self) -> &'static str {
        match self {
            AccountType::Bank => "Bank",
            AccountType::AccountsReceivable => "Accounts Receivable",
            AccountType::OtherCurrentAsset => "Other Current Asset",
            AccountType::FixedAsset => "Fixed Asset",
            AccountType::OtherAsset => "Other Asset",
            AccountType::AccountsPayable => "Accounts Payable",
            AccountType::CreditCard => "Credit Card",
            AccountType::OtherCurrentLiability => "Other Current Liability",
            AccountType::LongTermLiability => "Long Term Liability",
            AccountType::Equity => "Equity",
            AccountType::Income => "Income",
            AccountType::CostOfGoodsSold => "Cost of Goods Sold",
            AccountType::Expense => "Expense",
            AccountType::OtherIncome => "Other Income",
            AccountType::OtherExpense => "Other Expense",
            AccountType::NonPosting => "Non-Posting",
        }
    }

    /// The qbXML spelling (`AccountType` enum values).
    pub fn from_qbxml(s: &str) -> Option<AccountType> {
        Some(match s {
            "Bank" => AccountType::Bank,
            "AccountsReceivable" => AccountType::AccountsReceivable,
            "OtherCurrentAsset" => AccountType::OtherCurrentAsset,
            "FixedAsset" => AccountType::FixedAsset,
            "OtherAsset" => AccountType::OtherAsset,
            "AccountsPayable" => AccountType::AccountsPayable,
            "CreditCard" => AccountType::CreditCard,
            "OtherCurrentLiability" => AccountType::OtherCurrentLiability,
            "LongTermLiability" => AccountType::LongTermLiability,
            "Equity" => AccountType::Equity,
            "Income" => AccountType::Income,
            "CostOfGoodsSold" => AccountType::CostOfGoodsSold,
            "Expense" => AccountType::Expense,
            "OtherIncome" => AccountType::OtherIncome,
            "OtherExpense" => AccountType::OtherExpense,
            "NonPosting" => AccountType::NonPosting,
            _ => return None,
        })
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            AccountType::Bank => "Bank",
            AccountType::AccountsReceivable => "AccountsReceivable",
            AccountType::OtherCurrentAsset => "OtherCurrentAsset",
            AccountType::FixedAsset => "FixedAsset",
            AccountType::OtherAsset => "OtherAsset",
            AccountType::AccountsPayable => "AccountsPayable",
            AccountType::CreditCard => "CreditCard",
            AccountType::OtherCurrentLiability => "OtherCurrentLiability",
            AccountType::LongTermLiability => "LongTermLiability",
            AccountType::Equity => "Equity",
            AccountType::Income => "Income",
            AccountType::CostOfGoodsSold => "CostOfGoodsSold",
            AccountType::Expense => "Expense",
            AccountType::OtherIncome => "OtherIncome",
            AccountType::OtherExpense => "OtherExpense",
            AccountType::NonPosting => "NonPosting",
        }
    }
}

/// Accounts the books need to find without a person choosing them. QBD creates
/// these on demand ("Undeposited Funds", "Opening Balance Equity", …); LibreBooks
/// marks them with a role so a rename does not lose them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum AccountRole {
    /// Default A/R for invoices when none is chosen.
    AccountsReceivable,
    /// Default A/P for bills.
    AccountsPayable,
    /// Where payments wait for a deposit.
    UndepositedFunds,
    /// Balancing account for opening balances.
    OpeningBalanceEquity,
    /// Prior years' net income lands here (virtually, at report time).
    RetainedEarnings,
    /// Sales tax collected and owed.
    SalesTaxPayable,
    /// Inventory on hand, at average cost.
    InventoryAsset,
    /// Cost of inventory sold.
    CostOfGoodsSold,
    /// Payroll withholdings and employer taxes owed.
    PayrollLiabilities,
    /// Payroll expense.
    PayrollExpenses,
    /// QBD's "Ask My Accountant" catch-all.
    AskMyAccountant,
    /// Rounding and reconciliation discrepancies.
    ReconciliationDiscrepancies,
    /// Estimates.
    Estimates,
    /// Purchase orders.
    PurchaseOrders,
    /// Sales orders.
    SalesOrders,
}

impl AccountRole {
    /// The type a newly created account for this role gets, and its usual QBD name.
    pub const fn default_shape(self) -> (AccountType, &'static str) {
        match self {
            AccountRole::AccountsReceivable => {
                (AccountType::AccountsReceivable, "Accounts Receivable")
            }
            AccountRole::AccountsPayable => (AccountType::AccountsPayable, "Accounts Payable"),
            AccountRole::UndepositedFunds => (AccountType::OtherCurrentAsset, "Undeposited Funds"),
            AccountRole::OpeningBalanceEquity => (AccountType::Equity, "Opening Balance Equity"),
            AccountRole::RetainedEarnings => (AccountType::Equity, "Retained Earnings"),
            AccountRole::SalesTaxPayable => {
                (AccountType::OtherCurrentLiability, "Sales Tax Payable")
            }
            AccountRole::InventoryAsset => (AccountType::OtherCurrentAsset, "Inventory Asset"),
            AccountRole::CostOfGoodsSold => (AccountType::CostOfGoodsSold, "Cost of Goods Sold"),
            AccountRole::PayrollLiabilities => {
                (AccountType::OtherCurrentLiability, "Payroll Liabilities")
            }
            AccountRole::PayrollExpenses => (AccountType::Expense, "Payroll Expenses"),
            AccountRole::AskMyAccountant => (AccountType::Expense, "Ask My Accountant"),
            AccountRole::ReconciliationDiscrepancies => {
                (AccountType::Expense, "Reconciliation Discrepancies")
            }
            AccountRole::Estimates => (AccountType::NonPosting, "Estimates"),
            AccountRole::PurchaseOrders => (AccountType::NonPosting, "Purchase Orders"),
            AccountRole::SalesOrders => (AccountType::NonPosting, "Sales Orders"),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            AccountRole::AccountsReceivable => "AccountsReceivable",
            AccountRole::AccountsPayable => "AccountsPayable",
            AccountRole::UndepositedFunds => "UndepositedFunds",
            AccountRole::OpeningBalanceEquity => "OpeningBalanceEquity",
            AccountRole::RetainedEarnings => "RetainedEarnings",
            AccountRole::SalesTaxPayable => "SalesTaxPayable",
            AccountRole::InventoryAsset => "InventoryAsset",
            AccountRole::CostOfGoodsSold => "CostOfGoodsSold",
            AccountRole::PayrollLiabilities => "PayrollLiabilities",
            AccountRole::PayrollExpenses => "PayrollExpenses",
            AccountRole::AskMyAccountant => "AskMyAccountant",
            AccountRole::ReconciliationDiscrepancies => "ReconciliationDiscrepancies",
            AccountRole::Estimates => "Estimates",
            AccountRole::PurchaseOrders => "PurchaseOrders",
            AccountRole::SalesOrders => "SalesOrders",
        }
    }

    pub fn parse(s: &str) -> Option<AccountRole> {
        Some(match s {
            "AccountsReceivable" => AccountRole::AccountsReceivable,
            "AccountsPayable" => AccountRole::AccountsPayable,
            "UndepositedFunds" => AccountRole::UndepositedFunds,
            "OpeningBalanceEquity" => AccountRole::OpeningBalanceEquity,
            "RetainedEarnings" => AccountRole::RetainedEarnings,
            "SalesTaxPayable" => AccountRole::SalesTaxPayable,
            "InventoryAsset" => AccountRole::InventoryAsset,
            "CostOfGoodsSold" => AccountRole::CostOfGoodsSold,
            "PayrollLiabilities" => AccountRole::PayrollLiabilities,
            "PayrollExpenses" => AccountRole::PayrollExpenses,
            "AskMyAccountant" => AccountRole::AskMyAccountant,
            "ReconciliationDiscrepancies" => AccountRole::ReconciliationDiscrepancies,
            "Estimates" => AccountRole::Estimates,
            "PurchaseOrders" => AccountRole::PurchaseOrders,
            "SalesOrders" => AccountRole::SalesOrders,
            _ => return None,
        })
    }
}

/// An account in the chart of accounts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Account {
    pub id: AccountId,
    /// This account's own name, without its parents: `Materials`.
    pub name: String,
    /// `Cost of Goods Sold:Materials`. Maintained by the store from the parent chain.
    pub full_name: String,
    pub parent_id: Option<AccountId>,
    /// Depth below the top level (0 for a top-level account).
    pub sublevel: u32,
    pub account_type: AccountType,
    /// Optional account number shown before the name when numbering is on.
    pub number: Option<String>,
    pub description: Option<String>,
    /// The bank's account number (for bank and credit card accounts).
    pub bank_account_number: Option<String>,
    /// Tax-line mapping (e.g. `B/S-Assets: Cash`).
    pub tax_line: Option<String>,
    pub role: Option<AccountRole>,
    pub is_active: bool,
    /// ISO 4217. `None` means the home currency.
    pub currency: Option<String>,
    /// QBD's cash flow classification for the statement of cash flows.
    pub cash_flow_class: Option<CashFlowClass>,
    /// Edit sequence, bumped on every save (optimistic concurrency).
    #[ts(type = "number")]
    pub edit_seq: i64,
}

impl Account {
    /// Separator between levels in a full name, as QBD writes it.
    pub const SEPARATOR: char = ':';

    pub fn full_name_for(parent_full_name: Option<&str>, name: &str) -> String {
        match parent_full_name {
            Some(p) => format!("{p}{}{name}", Self::SEPARATOR),
            None => name.to_owned(),
        }
    }
}

/// Where an account's changes go on the statement of cash flows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum CashFlowClass {
    Operating,
    Investing,
    Financing,
    NotApplicable,
}

impl CashFlowClass {
    /// QBD's default by account type.
    pub const fn default_for(t: AccountType) -> CashFlowClass {
        match t {
            AccountType::AccountsReceivable
            | AccountType::OtherCurrentAsset
            | AccountType::AccountsPayable
            | AccountType::CreditCard
            | AccountType::OtherCurrentLiability => CashFlowClass::Operating,
            AccountType::FixedAsset | AccountType::OtherAsset => CashFlowClass::Investing,
            AccountType::LongTermLiability | AccountType::Equity => CashFlowClass::Financing,
            _ => CashFlowClass::NotApplicable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_and_normal_balances() {
        assert!(AccountType::Bank.is_debit_normal());
        assert!(AccountType::Expense.is_debit_normal());
        assert!(AccountType::CostOfGoodsSold.is_debit_normal());
        assert!(!AccountType::CreditCard.is_debit_normal());
        assert!(!AccountType::Income.is_debit_normal());
        assert!(!AccountType::Equity.is_debit_normal());
        assert_eq!(AccountType::CreditCard.natural(Money(-500)), Money(500));
        assert_eq!(AccountType::Bank.natural(Money(-500)), Money(-500));
    }

    #[test]
    fn qbxml_names_round_trip() {
        for t in AccountType::ALL {
            assert_eq!(AccountType::from_qbxml(t.as_str()), Some(t));
        }
    }

    #[test]
    fn full_names() {
        assert_eq!(Account::full_name_for(None, "Expenses"), "Expenses");
        assert_eq!(
            Account::full_name_for(Some("Cost of Goods Sold"), "Materials"),
            "Cost of Goods Sold:Materials"
        );
    }
}
