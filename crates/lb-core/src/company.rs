//! Company information and preferences (QBD's "My Company" and Edit → Preferences
//! → Company Preferences).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::entity::Address;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct CompanyInfo {
    pub name: String,
    pub legal_name: Option<String>,
    pub address: Address,
    pub legal_address: Address,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    /// Employer identification number (or SSN for a sole proprietor).
    pub ein: Option<String>,
    /// 1 = January.
    pub fiscal_year_start_month: i8,
    pub tax_year_start_month: i8,
    /// Federal income tax form (Form1120, Form1120S, Form1065, Form990, Form1040…).
    pub tax_form: Option<String>,
    pub home_currency: String,
    pub preferences: Preferences,
}

impl Default for CompanyInfo {
    fn default() -> Self {
        CompanyInfo {
            name: String::new(),
            legal_name: None,
            address: Address::default(),
            legal_address: Address::default(),
            phone: None,
            email: None,
            website: None,
            ein: None,
            fiscal_year_start_month: 1,
            tax_year_start_month: 1,
            tax_form: None,
            home_currency: "USD".to_owned(),
            preferences: Preferences::default(),
        }
    }
}

/// Company-wide preferences, named as QBD names them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct Preferences {
    /// Accounting: show account numbers.
    pub use_account_numbers: bool,
    /// Accounting: class tracking, and whether to prompt when a class is left out.
    pub use_class_tracking: bool,
    pub prompt_for_class: bool,
    /// Accounting: warn when a date is this many days in the past or future.
    pub warn_days_past: Option<u32>,
    pub warn_days_future: Option<u32>,
    /// Accounting: automatically assign general journal entry numbers.
    pub auto_number_journal_entries: bool,
    /// Items & Inventory: inventory and purchase orders are active.
    pub inventory_enabled: bool,
    /// Sales & Customers.
    pub estimates_enabled: bool,
    pub sales_orders_enabled: bool,
    pub progress_invoicing: bool,
    /// Sales Tax.
    pub sales_tax_enabled: bool,
    /// Sales tax is owed when invoiced (accrual) or when paid (cash).
    pub sales_tax_on_cash_basis: bool,
    /// Reports: default basis.
    pub reports_cash_basis: bool,
    /// Time & Expenses: mark expenses billable by default, default markup percent.
    pub billable_by_default: bool,
    pub default_markup_percent: Option<String>,
    /// Payments: deposit received payments to Undeposited Funds by default.
    pub use_undeposited_funds: bool,
    /// Payments: apply a payment to the oldest invoices automatically.
    pub auto_apply_payments: bool,
    /// Checking: default accounts for each form, by role.
    pub default_check_account: Option<String>,
    pub default_deposit_account: Option<String>,
    /// Multiple currencies.
    pub multi_currency: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences {
            use_account_numbers: false,
            use_class_tracking: false,
            prompt_for_class: false,
            warn_days_past: Some(90),
            warn_days_future: Some(30),
            auto_number_journal_entries: true,
            inventory_enabled: false,
            estimates_enabled: true,
            sales_orders_enabled: false,
            progress_invoicing: false,
            sales_tax_enabled: false,
            sales_tax_on_cash_basis: false,
            reports_cash_basis: false,
            billable_by_default: true,
            default_markup_percent: None,
            use_undeposited_funds: true,
            auto_apply_payments: true,
            default_check_account: None,
            default_deposit_account: None,
            multi_currency: false,
        }
    }
}
