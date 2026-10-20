//! What a person asks for: QBD's report window plus its "Customize Report" dialog,
//! as one value.
//!
//! Every report takes the same [`ReportRequest`], so the UI has one settings
//! panel and memorized reports are just saved requests. A field that does not
//! apply to a report is ignored (the [catalog](crate::catalog) says which apply).
//! Dates are resolved against `today` (supplied, so a server in another time
//! zone and a test both get the day the person means) and the company's fiscal
//! year.

use jiff::civil::Date;
use lb_core::{AccountId, AccountType, ClassId, DateMacro, EntityId, ItemId, Money, TxnType};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Every report LibreBooks can run, named as QuickBooks Desktop names them.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS,
)]
#[ts(export)]
pub enum ReportId {
    // Company & Financial
    #[default]
    ProfitAndLoss,
    ProfitAndLossDetail,
    ProfitAndLossYtdComparison,
    ProfitAndLossPrevYearComparison,
    ProfitAndLossByJob,
    ProfitAndLossByClass,
    BalanceSheet,
    BalanceSheetDetail,
    BalanceSheetSummary,
    BalanceSheetPrevYearComparison,
    StatementOfCashFlows,
    // Customers & Receivables
    ArAgingSummary,
    ArAgingDetail,
    CustomerBalanceSummary,
    CustomerBalanceDetail,
    OpenInvoices,
    // Sales
    SalesByCustomerSummary,
    SalesByCustomerDetail,
    SalesByItemSummary,
    SalesByItemDetail,
    // Jobs
    JobProfitabilitySummary,
    JobProfitabilityDetail,
    // Vendors & Payables
    ApAgingSummary,
    ApAgingDetail,
    VendorBalanceSummary,
    VendorBalanceDetail,
    UnpaidBillsDetail,
    Vendor1099Summary,
    // Purchases
    ExpensesByVendorSummary,
    ExpensesByVendorDetail,
    // Inventory
    InventoryValuationSummary,
    // Banking
    DepositDetail,
    CheckDetail,
    MissingChecks,
    // Accountant & Taxes
    TrialBalance,
    GeneralLedger,
    Journal,
    TransactionDetailByAccount,
    TransactionListByDate,
    AccountQuickReport,
    AuditTrail,
    SalesTaxLiability,
    // Lists
    AccountListing,
    CustomerContactList,
    VendorContactList,
    ItemListing,
}

impl ReportId {
    pub const ALL: [ReportId; 46] = [
        ReportId::ProfitAndLoss,
        ReportId::ProfitAndLossDetail,
        ReportId::ProfitAndLossYtdComparison,
        ReportId::ProfitAndLossPrevYearComparison,
        ReportId::ProfitAndLossByJob,
        ReportId::ProfitAndLossByClass,
        ReportId::BalanceSheet,
        ReportId::BalanceSheetDetail,
        ReportId::BalanceSheetSummary,
        ReportId::BalanceSheetPrevYearComparison,
        ReportId::StatementOfCashFlows,
        ReportId::ArAgingSummary,
        ReportId::ArAgingDetail,
        ReportId::CustomerBalanceSummary,
        ReportId::CustomerBalanceDetail,
        ReportId::OpenInvoices,
        ReportId::SalesByCustomerSummary,
        ReportId::SalesByCustomerDetail,
        ReportId::SalesByItemSummary,
        ReportId::SalesByItemDetail,
        ReportId::JobProfitabilitySummary,
        ReportId::JobProfitabilityDetail,
        ReportId::ApAgingSummary,
        ReportId::ApAgingDetail,
        ReportId::VendorBalanceSummary,
        ReportId::VendorBalanceDetail,
        ReportId::UnpaidBillsDetail,
        ReportId::Vendor1099Summary,
        ReportId::ExpensesByVendorSummary,
        ReportId::ExpensesByVendorDetail,
        ReportId::InventoryValuationSummary,
        ReportId::DepositDetail,
        ReportId::CheckDetail,
        ReportId::MissingChecks,
        ReportId::TrialBalance,
        ReportId::GeneralLedger,
        ReportId::Journal,
        ReportId::TransactionDetailByAccount,
        ReportId::TransactionListByDate,
        ReportId::AccountQuickReport,
        ReportId::AuditTrail,
        ReportId::SalesTaxLiability,
        ReportId::AccountListing,
        ReportId::CustomerContactList,
        ReportId::VendorContactList,
        ReportId::ItemListing,
    ];
}

/// When income and expenses count: when invoiced and billed (accrual), or when
/// paid (cash). See `plans/reports.md` for exactly how cash basis is derived.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Basis {
    #[default]
    Accrual,
    Cash,
}

/// How a summary report's amounts are split into columns (QBD's "Display columns
/// by").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ColumnLayout {
    /// One column for the whole range.
    #[default]
    TotalOnly,
    /// Sunday-to-Saturday weeks.
    Week,
    Month,
    /// Fiscal quarters.
    Quarter,
    /// Fiscal years.
    Year,
    /// One column per customer and job, with a total per customer that has jobs.
    CustomerJob,
    Class,
    Vendor,
    Item,
}

impl ColumnLayout {
    /// Columns that are periods of time (as opposed to names, classes, items).
    pub const fn is_period(self) -> bool {
        matches!(
            self,
            ColumnLayout::Week | ColumnLayout::Month | ColumnLayout::Quarter | ColumnLayout::Year
        )
    }
}

/// QBD's "Cleared" filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ClearedFilter {
    /// Reconciled.
    Cleared,
    /// Not yet reconciled (uncleared or pending).
    NotCleared,
}

/// How a detail report orders the rows within each group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum SortBy {
    /// The report's own order (usually date, then entry order).
    #[default]
    Default,
    Date,
    Number,
    Name,
    Memo,
    Amount,
}

/// QBD's "Filters" tab. Every list is "any of these"; an empty list means no
/// filter. Accounts include their subaccounts, names their jobs, classes and
/// items their children, unless `exact` says otherwise.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct ReportFilters {
    pub accounts: Vec<AccountId>,
    pub account_types: Vec<AccountType>,
    /// Customers, jobs, vendors, employees, other names. A line matches when its
    /// own name or its transaction's name is one of these.
    pub names: Vec<EntityId>,
    pub classes: Vec<ClassId>,
    pub items: Vec<ItemId>,
    pub txn_types: Vec<TxnType>,
    pub cleared: Option<ClearedFilter>,
    /// The transaction's amount (ignoring its sign) is at least this.
    pub amount_min: Option<Money>,
    /// The transaction's amount (ignoring its sign) is at most this.
    pub amount_max: Option<Money>,
    /// The memo contains this text (case-insensitive).
    pub memo: Option<String>,
    /// Match the listed accounts, names, classes and items only, not what is
    /// below them (QuickZoom on a "<parent> - Other" row uses this).
    pub exact: bool,
}

impl ReportFilters {
    pub fn is_empty(&self) -> bool {
        *self == ReportFilters::default()
    }
}

/// One report, as asked for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct ReportRequest {
    pub report: ReportId,
    /// A relative range ("This Fiscal Year-to-date"). Wins over `from`/`to`.
    pub date_macro: Option<DateMacro>,
    /// An explicit range. For "as of" reports (balance sheet, aging, trial
    /// balance) only `to` matters. With neither a macro nor dates, the report's
    /// default range applies.
    pub from: Option<Date>,
    pub to: Option<Date>,
    /// The day macros are relative to. `None` means the server's local today.
    pub today: Option<Date>,
    /// `None` means the company's preference (Reports & Graphs → Summary
    /// Reports Basis).
    pub basis: Option<Basis>,
    /// `None` means the report's default layout.
    pub columns: Option<ColumnLayout>,
    /// Add "% of Income" after each amount column (profit and loss reports).
    pub percent_of_income: bool,
    /// Add "% of Column" after each amount column (other summaries).
    pub percent_of_column: bool,
    /// Compare with the period just before.
    pub previous_period: bool,
    /// Compare with the same period a year earlier.
    pub previous_year: bool,
    /// With a comparison: show the difference in dollars...
    pub dollar_change: bool,
    /// ...and as a percent of the earlier amount.
    pub percent_change: bool,
    pub filters: ReportFilters,
    pub sort_by: SortBy,
    pub sort_descending: bool,
    /// Show accounts and names with nothing in them.
    pub show_zero_rows: bool,
    /// A memorized report's own title.
    pub title: Option<String>,
}

impl Default for ReportRequest {
    fn default() -> Self {
        ReportRequest {
            report: ReportId::default(),
            date_macro: None,
            from: None,
            to: None,
            today: None,
            basis: None,
            columns: None,
            percent_of_income: false,
            percent_of_column: false,
            previous_period: false,
            previous_year: false,
            dollar_change: true,
            percent_change: true,
            filters: ReportFilters::default(),
            sort_by: SortBy::Default,
            sort_descending: false,
            show_zero_rows: false,
            title: None,
        }
    }
}

impl ReportRequest {
    /// A request for `report` with every setting at its default.
    pub fn new(report: ReportId) -> ReportRequest {
        ReportRequest {
            report,
            ..ReportRequest::default()
        }
    }

    /// The same request for an explicit range.
    #[must_use]
    pub fn range(mut self, from: Option<Date>, to: Option<Date>) -> ReportRequest {
        self.date_macro = None;
        self.from = from;
        self.to = to;
        self
    }

    #[must_use]
    pub fn basis(mut self, basis: Basis) -> ReportRequest {
        self.basis = Some(basis);
        self
    }

    #[must_use]
    pub fn columns(mut self, layout: ColumnLayout) -> ReportRequest {
        self.columns = Some(layout);
        self
    }

    #[must_use]
    pub fn today(mut self, today: Date) -> ReportRequest {
        self.today = Some(today);
        self
    }
}
