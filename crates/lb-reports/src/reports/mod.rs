//! The reports themselves, one module per family.

mod aging;
mod audit;
mod balance;
mod banking;
mod cashflow;
mod detail;
mod lists;
mod pnl;
mod sales;
mod tax;
mod trial;

use lb_core::AccountType;

use crate::model::Report;
use crate::request::ReportId;
use crate::{Result, Run};

pub(crate) fn run(r: &Run<'_>) -> Result<Report> {
    match r.req.report {
        ReportId::ProfitAndLoss
        | ReportId::ProfitAndLossByJob
        | ReportId::ProfitAndLossByClass
        | ReportId::ProfitAndLossYtdComparison
        | ReportId::ProfitAndLossPrevYearComparison => pnl::summary(r),
        ReportId::ProfitAndLossDetail => detail::pnl_detail(r),
        ReportId::BalanceSheet | ReportId::BalanceSheetPrevYearComparison => {
            balance::balance_sheet(r, false)
        }
        ReportId::BalanceSheetSummary => balance::balance_sheet(r, true),
        ReportId::BalanceSheetDetail => detail::balance_sheet_detail(r),
        ReportId::StatementOfCashFlows => cashflow::statement(r),
        ReportId::TrialBalance => trial::trial_balance(r),
        ReportId::GeneralLedger => detail::general_ledger(r),
        ReportId::TransactionDetailByAccount => detail::by_account(r),
        ReportId::AccountQuickReport => detail::quick_report(r),
        ReportId::Journal => detail::journal(r),
        ReportId::TransactionListByDate => detail::list_by_date(r),
        ReportId::ArAgingSummary => aging::aging_summary(r, aging::Side::Receivable),
        ReportId::ApAgingSummary => aging::aging_summary(r, aging::Side::Payable),
        ReportId::ArAgingDetail => aging::aging_detail(r, aging::Side::Receivable),
        ReportId::ApAgingDetail => aging::aging_detail(r, aging::Side::Payable),
        ReportId::OpenInvoices => aging::open_items(r, aging::Side::Receivable),
        ReportId::UnpaidBillsDetail => aging::open_items(r, aging::Side::Payable),
        ReportId::CustomerBalanceSummary => aging::balance_summary(r, aging::Side::Receivable),
        ReportId::VendorBalanceSummary => aging::balance_summary(r, aging::Side::Payable),
        ReportId::CustomerBalanceDetail => aging::balance_detail(r, aging::Side::Receivable),
        ReportId::VendorBalanceDetail => aging::balance_detail(r, aging::Side::Payable),
        ReportId::SalesByCustomerSummary => sales::by_customer_summary(r),
        ReportId::SalesByCustomerDetail => sales::by_customer_detail(r),
        ReportId::SalesByItemSummary => sales::by_item_summary(r),
        ReportId::SalesByItemDetail => sales::by_item_detail(r),
        ReportId::ExpensesByVendorSummary => sales::expenses_by_vendor_summary(r),
        ReportId::ExpensesByVendorDetail => sales::expenses_by_vendor_detail(r),
        ReportId::JobProfitabilitySummary => sales::job_profitability_summary(r),
        ReportId::JobProfitabilityDetail => sales::job_profitability_detail(r),
        ReportId::InventoryValuationSummary => sales::inventory_valuation(r),
        ReportId::DepositDetail => banking::deposit_detail(r),
        ReportId::CheckDetail => banking::check_detail(r),
        ReportId::MissingChecks => banking::missing_checks(r),
        ReportId::SalesTaxLiability => tax::sales_tax_liability(r),
        ReportId::Vendor1099Summary => tax::vendor_1099_summary(r),
        ReportId::AuditTrail => audit::audit_trail(r),
        ReportId::AccountListing => lists::account_listing(r),
        ReportId::CustomerContactList => Ok(lists::contact_list(r, lb_core::EntityKind::Customer)),
        ReportId::VendorContactList => Ok(lists::contact_list(r, lb_core::EntityKind::Vendor)),
        ReportId::ItemListing => Ok(lists::item_listing(r)),
    }
}

/// QBD's profit and loss sections, in order: (heading, account types).
pub(crate) const PL_SECTIONS: [(&str, AccountType); 5] = [
    ("Income", AccountType::Income),
    ("Cost of Goods Sold", AccountType::CostOfGoodsSold),
    ("Expense", AccountType::Expense),
    ("Other Income", AccountType::OtherIncome),
    ("Other Expense", AccountType::OtherExpense),
];

/// The accounts of some types, in chart order.
pub(crate) fn accounts_of(ctx: &crate::ctx::Ctx, types: &[AccountType]) -> Vec<lb_core::AccountId> {
    ctx.accounts
        .iter()
        .filter(|a| types.contains(&a.account_type))
        .map(|a| a.id)
        .collect()
}
