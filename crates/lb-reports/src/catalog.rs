//! The Reports menu: every report, its QBD category, and which settings apply to
//! it, so the UI builds the menu and each report's settings panel from data.

use lb_core::DateMacro;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::request::{ColumnLayout, ReportId, SortBy};

/// QBD's Reports menu sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ReportCategory {
    CompanyAndFinancial,
    CustomersAndReceivables,
    Sales,
    Jobs,
    VendorsAndPayables,
    Purchases,
    Inventory,
    Banking,
    AccountantAndTaxes,
    Lists,
}

impl ReportCategory {
    pub const fn label(self) -> &'static str {
        match self {
            ReportCategory::CompanyAndFinancial => "Company & Financial",
            ReportCategory::CustomersAndReceivables => "Customers & Receivables",
            ReportCategory::Sales => "Sales",
            ReportCategory::Jobs => "Jobs, Time & Mileage",
            ReportCategory::VendorsAndPayables => "Vendors & Payables",
            ReportCategory::Purchases => "Purchases",
            ReportCategory::Inventory => "Inventory",
            ReportCategory::Banking => "Banking",
            ReportCategory::AccountantAndTaxes => "Accountant & Taxes",
            ReportCategory::Lists => "Lists",
        }
    }
}

/// How a report takes dates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum DatesKind {
    /// From and to.
    Range,
    /// A single "as of" date (`to`).
    AsOf,
    /// No dates (lists).
    None,
}

/// The filters a report honours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum FilterKind {
    Accounts,
    AccountTypes,
    Names,
    Classes,
    Items,
    TxnTypes,
    Cleared,
    Amount,
    Memo,
}

const ALL_FILTERS: &[FilterKind] = &[
    FilterKind::Accounts,
    FilterKind::AccountTypes,
    FilterKind::Names,
    FilterKind::Classes,
    FilterKind::Items,
    FilterKind::TxnTypes,
    FilterKind::Cleared,
    FilterKind::Amount,
    FilterKind::Memo,
];

const SUMMARY_LAYOUTS: &[ColumnLayout] = &[
    ColumnLayout::TotalOnly,
    ColumnLayout::Week,
    ColumnLayout::Month,
    ColumnLayout::Quarter,
    ColumnLayout::Year,
    ColumnLayout::CustomerJob,
    ColumnLayout::Class,
    ColumnLayout::Vendor,
    ColumnLayout::Item,
];

const PERIOD_LAYOUTS: &[ColumnLayout] = &[
    ColumnLayout::TotalOnly,
    ColumnLayout::Week,
    ColumnLayout::Month,
    ColumnLayout::Quarter,
    ColumnLayout::Year,
];

const DETAIL_SORTS: &[SortBy] = &[
    SortBy::Default,
    SortBy::Date,
    SortBy::Number,
    SortBy::Name,
    SortBy::Memo,
    SortBy::Amount,
];

/// Which settings apply to a report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReportParams {
    pub dates: DatesKind,
    /// The range used when a request names none.
    pub default_dates: Option<DateMacro>,
    pub basis: bool,
    /// Layouts offered; empty when the columns are fixed.
    pub columns: Vec<ColumnLayout>,
    pub default_columns: ColumnLayout,
    /// Previous period / previous year comparison columns.
    pub comparisons: bool,
    pub percent_of_income: bool,
    pub percent_of_column: bool,
    pub filters: Vec<FilterKind>,
    pub sorts: Vec<SortBy>,
    pub show_zero_rows: bool,
}

/// One entry on the Reports menu.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogEntry {
    pub id: ReportId,
    pub name: String,
    pub category: ReportCategory,
    pub category_label: String,
    pub description: String,
    pub params: ReportParams,
}

/// Every report, in menu order.
pub fn catalog() -> Vec<CatalogEntry> {
    ReportId::ALL
        .into_iter()
        .map(|id| CatalogEntry {
            id,
            name: id.name().to_owned(),
            category: id.category(),
            category_label: id.category().label().to_owned(),
            description: id.description().to_owned(),
            params: id.params(),
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Shape {
    /// Account summaries with flexible columns.
    Summary,
    /// Account summaries over periods only.
    PeriodSummary,
    /// Fixed columns, a range.
    Detail,
    /// Fixed columns, as of a date.
    AsOf,
    /// A summary by name or item with fixed columns.
    NameSummary,
    List,
}

impl ReportId {
    /// The name on the Reports menu and the report's default title.
    pub const fn name(self) -> &'static str {
        match self {
            ReportId::ProfitAndLoss => "Profit & Loss",
            ReportId::ProfitAndLossDetail => "Profit & Loss Detail",
            ReportId::ProfitAndLossYtdComparison => "Profit & Loss YTD Comparison",
            ReportId::ProfitAndLossPrevYearComparison => "Profit & Loss Prev Year Comparison",
            ReportId::ProfitAndLossByJob => "Profit & Loss by Job",
            ReportId::ProfitAndLossByClass => "Profit & Loss by Class",
            ReportId::BalanceSheet => "Balance Sheet",
            ReportId::BalanceSheetDetail => "Balance Sheet Detail",
            ReportId::BalanceSheetSummary => "Balance Sheet Summary",
            ReportId::BalanceSheetPrevYearComparison => "Balance Sheet Prev Year Comparison",
            ReportId::StatementOfCashFlows => "Statement of Cash Flows",
            ReportId::ArAgingSummary => "A/R Aging Summary",
            ReportId::ArAgingDetail => "A/R Aging Detail",
            ReportId::CustomerBalanceSummary => "Customer Balance Summary",
            ReportId::CustomerBalanceDetail => "Customer Balance Detail",
            ReportId::OpenInvoices => "Open Invoices",
            ReportId::SalesByCustomerSummary => "Sales by Customer Summary",
            ReportId::SalesByCustomerDetail => "Sales by Customer Detail",
            ReportId::SalesByItemSummary => "Sales by Item Summary",
            ReportId::SalesByItemDetail => "Sales by Item Detail",
            ReportId::JobProfitabilitySummary => "Job Profitability Summary",
            ReportId::JobProfitabilityDetail => "Job Profitability Detail",
            ReportId::ApAgingSummary => "A/P Aging Summary",
            ReportId::ApAgingDetail => "A/P Aging Detail",
            ReportId::VendorBalanceSummary => "Vendor Balance Summary",
            ReportId::VendorBalanceDetail => "Vendor Balance Detail",
            ReportId::UnpaidBillsDetail => "Unpaid Bills Detail",
            ReportId::Vendor1099Summary => "1099 Summary",
            ReportId::ExpensesByVendorSummary => "Expenses by Vendor Summary",
            ReportId::ExpensesByVendorDetail => "Expenses by Vendor Detail",
            ReportId::InventoryValuationSummary => "Inventory Valuation Summary",
            ReportId::DepositDetail => "Deposit Detail",
            ReportId::CheckDetail => "Check Detail",
            ReportId::MissingChecks => "Missing Checks",
            ReportId::TrialBalance => "Trial Balance",
            ReportId::GeneralLedger => "General Ledger",
            ReportId::Journal => "Journal",
            ReportId::TransactionDetailByAccount => "Transaction Detail by Account",
            ReportId::TransactionListByDate => "Transaction List by Date",
            ReportId::AccountQuickReport => "Account QuickReport",
            ReportId::AuditTrail => "Audit Trail",
            ReportId::SalesTaxLiability => "Sales Tax Liability",
            ReportId::AccountListing => "Account Listing",
            ReportId::CustomerContactList => "Customer Contact List",
            ReportId::VendorContactList => "Vendor Contact List",
            ReportId::ItemListing => "Item Listing",
        }
    }

    pub const fn category(self) -> ReportCategory {
        match self {
            ReportId::ProfitAndLoss
            | ReportId::ProfitAndLossDetail
            | ReportId::ProfitAndLossYtdComparison
            | ReportId::ProfitAndLossPrevYearComparison
            | ReportId::ProfitAndLossByJob
            | ReportId::ProfitAndLossByClass
            | ReportId::BalanceSheet
            | ReportId::BalanceSheetDetail
            | ReportId::BalanceSheetSummary
            | ReportId::BalanceSheetPrevYearComparison
            | ReportId::StatementOfCashFlows => ReportCategory::CompanyAndFinancial,
            ReportId::ArAgingSummary
            | ReportId::ArAgingDetail
            | ReportId::CustomerBalanceSummary
            | ReportId::CustomerBalanceDetail
            | ReportId::OpenInvoices => ReportCategory::CustomersAndReceivables,
            ReportId::SalesByCustomerSummary
            | ReportId::SalesByCustomerDetail
            | ReportId::SalesByItemSummary
            | ReportId::SalesByItemDetail => ReportCategory::Sales,
            ReportId::JobProfitabilitySummary | ReportId::JobProfitabilityDetail => {
                ReportCategory::Jobs
            }
            ReportId::ApAgingSummary
            | ReportId::ApAgingDetail
            | ReportId::VendorBalanceSummary
            | ReportId::VendorBalanceDetail
            | ReportId::UnpaidBillsDetail
            | ReportId::Vendor1099Summary => ReportCategory::VendorsAndPayables,
            ReportId::ExpensesByVendorSummary | ReportId::ExpensesByVendorDetail => {
                ReportCategory::Purchases
            }
            ReportId::InventoryValuationSummary => ReportCategory::Inventory,
            ReportId::DepositDetail | ReportId::CheckDetail | ReportId::MissingChecks => {
                ReportCategory::Banking
            }
            ReportId::TrialBalance
            | ReportId::GeneralLedger
            | ReportId::Journal
            | ReportId::TransactionDetailByAccount
            | ReportId::TransactionListByDate
            | ReportId::AccountQuickReport
            | ReportId::AuditTrail
            | ReportId::SalesTaxLiability => ReportCategory::AccountantAndTaxes,
            ReportId::AccountListing
            | ReportId::CustomerContactList
            | ReportId::VendorContactList
            | ReportId::ItemListing => ReportCategory::Lists,
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            ReportId::ProfitAndLoss => "Income and expenses for a period, and the net income.",
            ReportId::ProfitAndLossDetail => {
                "Every transaction behind each income and expense account."
            }
            ReportId::ProfitAndLossYtdComparison => "This period beside the fiscal year to date.",
            ReportId::ProfitAndLossPrevYearComparison => {
                "This period beside the same period last year, with the change."
            }
            ReportId::ProfitAndLossByJob => "Income and expenses for each customer and job.",
            ReportId::ProfitAndLossByClass => "Income and expenses for each class.",
            ReportId::BalanceSheet => "Assets, liabilities and equity as of a date.",
            ReportId::BalanceSheetDetail => {
                "Every transaction behind each balance sheet account, with balances."
            }
            ReportId::BalanceSheetSummary => "The balance sheet by account type.",
            ReportId::BalanceSheetPrevYearComparison => {
                "The balance sheet beside the same date last year, with the change."
            }
            ReportId::StatementOfCashFlows => {
                "Cash in and out by operating, investing and financing activity."
            }
            ReportId::ArAgingSummary => "What each customer owes, by how overdue.",
            ReportId::ArAgingDetail => "Each open invoice and credit, grouped by how overdue.",
            ReportId::CustomerBalanceSummary => "What each customer owes.",
            ReportId::CustomerBalanceDetail => {
                "Every transaction affecting what each customer owes."
            }
            ReportId::OpenInvoices => "Unpaid invoices and unused credits, by customer.",
            ReportId::SalesByCustomerSummary => "Sales to each customer and job.",
            ReportId::SalesByCustomerDetail => "Each sale, by customer and job.",
            ReportId::SalesByItemSummary => "Quantity, sales and margin for each item.",
            ReportId::SalesByItemDetail => "Each sale, by item.",
            ReportId::JobProfitabilitySummary => {
                "Revenue and cost for each customer and job, sub-jobs rolled up."
            }
            ReportId::JobProfitabilityDetail => {
                "Revenue and cost for each job, by item and account."
            }
            ReportId::ApAgingSummary => "What is owed to each vendor, by how overdue.",
            ReportId::ApAgingDetail => "Each unpaid bill and credit, grouped by how overdue.",
            ReportId::VendorBalanceSummary => "What is owed to each vendor.",
            ReportId::VendorBalanceDetail => {
                "Every transaction affecting what is owed to each vendor."
            }
            ReportId::UnpaidBillsDetail => "Unpaid bills and unused credits, by vendor.",
            ReportId::Vendor1099Summary => {
                "Payments to 1099 vendors, by the accounts they were for."
            }
            ReportId::ExpensesByVendorSummary => "What was spent with each vendor.",
            ReportId::ExpensesByVendorDetail => "Each expense, by vendor.",
            ReportId::InventoryValuationSummary => {
                "Quantity on hand, average cost and value of each inventory item."
            }
            ReportId::DepositDetail => "Each deposit and what went into it.",
            ReportId::CheckDetail => "Each check and payment, and what it paid for.",
            ReportId::MissingChecks => "Gaps and duplicates in check numbers.",
            ReportId::TrialBalance => "Every account's debit or credit balance.",
            ReportId::GeneralLedger => "Every transaction in every account, with balances.",
            ReportId::Journal => "Every transaction with its debits and credits.",
            ReportId::TransactionDetailByAccount => "Transactions grouped by account.",
            ReportId::TransactionListByDate => "Every transaction, by date.",
            ReportId::AccountQuickReport => "One account's transactions.",
            ReportId::AuditTrail => "Who entered and changed what, and when.",
            ReportId::SalesTaxLiability => "Sales tax collected and owed, by agency.",
            ReportId::AccountListing => "The chart of accounts.",
            ReportId::CustomerContactList => "Customers' contact details.",
            ReportId::VendorContactList => "Vendors' contact details.",
            ReportId::ItemListing => "Items, their prices and accounts.",
        }
    }

    fn shape(self) -> Shape {
        match self {
            ReportId::ProfitAndLoss
            | ReportId::ProfitAndLossByJob
            | ReportId::ProfitAndLossByClass => Shape::Summary,
            ReportId::ProfitAndLossPrevYearComparison
            | ReportId::ProfitAndLossYtdComparison
            | ReportId::BalanceSheet
            | ReportId::BalanceSheetSummary
            | ReportId::BalanceSheetPrevYearComparison
            | ReportId::StatementOfCashFlows => Shape::PeriodSummary,
            ReportId::ArAgingSummary
            | ReportId::ApAgingSummary
            | ReportId::CustomerBalanceSummary
            | ReportId::VendorBalanceSummary
            | ReportId::InventoryValuationSummary
            | ReportId::TrialBalance
            | ReportId::ArAgingDetail
            | ReportId::ApAgingDetail
            | ReportId::OpenInvoices
            | ReportId::UnpaidBillsDetail => Shape::AsOf,
            ReportId::SalesByCustomerSummary
            | ReportId::SalesByItemSummary
            | ReportId::ExpensesByVendorSummary
            | ReportId::JobProfitabilitySummary
            | ReportId::Vendor1099Summary
            | ReportId::SalesTaxLiability => Shape::NameSummary,
            ReportId::AccountListing
            | ReportId::CustomerContactList
            | ReportId::VendorContactList
            | ReportId::ItemListing => Shape::List,
            _ => Shape::Detail,
        }
    }

    /// The range used when a request names none (QBD's defaults).
    pub const fn default_dates(self) -> Option<DateMacro> {
        match self {
            ReportId::BalanceSheet
            | ReportId::BalanceSheetDetail
            | ReportId::BalanceSheetSummary
            | ReportId::BalanceSheetPrevYearComparison
            | ReportId::StatementOfCashFlows
            | ReportId::ProfitAndLossYtdComparison
            | ReportId::ProfitAndLossPrevYearComparison => Some(DateMacro::ThisFiscalYearToDate),
            ReportId::ArAgingSummary
            | ReportId::ArAgingDetail
            | ReportId::ApAgingSummary
            | ReportId::ApAgingDetail
            | ReportId::OpenInvoices
            | ReportId::UnpaidBillsDetail
            | ReportId::InventoryValuationSummary
            | ReportId::Journal
            | ReportId::AuditTrail => Some(DateMacro::Today),
            ReportId::CustomerBalanceSummary
            | ReportId::CustomerBalanceDetail
            | ReportId::VendorBalanceSummary
            | ReportId::VendorBalanceDetail
            | ReportId::MissingChecks
            | ReportId::AccountQuickReport => Some(DateMacro::All),
            ReportId::TransactionListByDate => Some(DateMacro::ThisWeek),
            ReportId::Vendor1099Summary => Some(DateMacro::LastCalendarYear),
            ReportId::SalesTaxLiability => Some(DateMacro::LastMonth),
            ReportId::AccountListing
            | ReportId::CustomerContactList
            | ReportId::VendorContactList
            | ReportId::ItemListing => None,
            _ => Some(DateMacro::ThisMonthToDate),
        }
    }

    /// Does the report read the books on a basis (accrual or cash).
    pub const fn uses_basis(self) -> bool {
        !matches!(
            self,
            ReportId::ArAgingSummary
                | ReportId::ArAgingDetail
                | ReportId::ApAgingSummary
                | ReportId::ApAgingDetail
                | ReportId::OpenInvoices
                | ReportId::UnpaidBillsDetail
                | ReportId::CustomerBalanceSummary
                | ReportId::CustomerBalanceDetail
                | ReportId::VendorBalanceSummary
                | ReportId::VendorBalanceDetail
                | ReportId::Vendor1099Summary
                | ReportId::InventoryValuationSummary
                | ReportId::DepositDetail
                | ReportId::CheckDetail
                | ReportId::MissingChecks
                | ReportId::AuditTrail
                | ReportId::AccountListing
                | ReportId::CustomerContactList
                | ReportId::VendorContactList
                | ReportId::ItemListing
        )
    }

    /// Does the report take a single "as of" date rather than a range.
    pub const fn is_as_of(self) -> bool {
        matches!(
            self,
            ReportId::BalanceSheet
                | ReportId::BalanceSheetSummary
                | ReportId::BalanceSheetPrevYearComparison
                | ReportId::TrialBalance
                | ReportId::ArAgingSummary
                | ReportId::ArAgingDetail
                | ReportId::ApAgingSummary
                | ReportId::ApAgingDetail
                | ReportId::OpenInvoices
                | ReportId::UnpaidBillsDetail
                | ReportId::CustomerBalanceSummary
                | ReportId::VendorBalanceSummary
                | ReportId::InventoryValuationSummary
        )
    }

    /// The layout a request without one gets.
    pub const fn default_columns(self) -> ColumnLayout {
        match self {
            ReportId::ProfitAndLossByJob => ColumnLayout::CustomerJob,
            ReportId::ProfitAndLossByClass => ColumnLayout::Class,
            _ => ColumnLayout::TotalOnly,
        }
    }

    pub fn params(self) -> ReportParams {
        let shape = self.shape();
        let dates = if matches!(shape, Shape::List) {
            DatesKind::None
        } else if self.is_as_of() {
            DatesKind::AsOf
        } else {
            DatesKind::Range
        };
        let (columns, comparisons) = match shape {
            Shape::Summary => (SUMMARY_LAYOUTS.to_vec(), true),
            Shape::PeriodSummary => (
                if matches!(
                    self,
                    ReportId::ProfitAndLossYtdComparison | ReportId::StatementOfCashFlows
                ) {
                    Vec::new()
                } else {
                    PERIOD_LAYOUTS.to_vec()
                },
                !matches!(self, ReportId::ProfitAndLossYtdComparison),
            ),
            _ => (Vec::new(), false),
        };
        let is_pl = matches!(
            self,
            ReportId::ProfitAndLoss
                | ReportId::ProfitAndLossByJob
                | ReportId::ProfitAndLossByClass
                | ReportId::ProfitAndLossYtdComparison
                | ReportId::ProfitAndLossPrevYearComparison
        );
        let filters = match shape {
            Shape::List => Vec::new(),
            _ => ALL_FILTERS.to_vec(),
        };
        let sorts = match shape {
            Shape::Detail => DETAIL_SORTS.to_vec(),
            _ => vec![SortBy::Default],
        };
        ReportParams {
            dates,
            default_dates: self.default_dates(),
            basis: self.uses_basis(),
            columns,
            default_columns: self.default_columns(),
            comparisons,
            percent_of_income: is_pl,
            percent_of_column: matches!(shape, Shape::Summary | Shape::PeriodSummary) && !is_pl,
            filters,
            sorts,
            show_zero_rows: matches!(
                shape,
                Shape::Summary | Shape::PeriodSummary | Shape::AsOf | Shape::NameSummary
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_report_is_listed_once_with_a_name() {
        let c = catalog();
        assert_eq!(c.len(), ReportId::ALL.len());
        let mut names: Vec<&str> = c.iter().map(|e| e.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), c.len());
        let pl = c.iter().find(|e| e.id == ReportId::ProfitAndLoss).unwrap();
        assert_eq!(pl.category, ReportCategory::CompanyAndFinancial);
        assert!(pl.params.basis && pl.params.comparisons && pl.params.percent_of_income);
        let bs = c.iter().find(|e| e.id == ReportId::BalanceSheet).unwrap();
        assert_eq!(bs.params.dates, DatesKind::AsOf);
        let lists = c
            .iter()
            .filter(|e| e.category == ReportCategory::Lists)
            .count();
        assert_eq!(lists, 4);
    }
}
