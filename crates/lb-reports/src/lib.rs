//! Reports over the ledger: QuickBooks Desktop's reports, one for one.
//!
//! A [`ReportRequest`] says which report, which dates (a QBD date macro or an
//! explicit range, relative to a supplied `today`), which basis, how to split
//! columns (by period, customer:job, class, vendor or item, with "% of income"
//! and previous-period or previous-year comparisons) and what to filter. [`run`]
//! reads the books in one consistent snapshot and returns a [`Report`]: one
//! generic shape (title block, typed columns, indented rows with subtotals and
//! totals, QuickZoom targets) that the UI renders without knowing which report
//! it is, and that [`export`] turns into a file (CSV, XLSX or PDF, all drawn from
//! the one [`render::Layout`]).
//!
//! Summary reports aggregate in SQL by account, period bucket and dimension, so a
//! company with hundreds of thousands of postings costs a handful of grouped
//! queries; detail reports load only the rows they print. Cash basis is derived
//! from the accrual ledger as two balanced adjustments ([`cash`]-module docs and
//! `plans/reports.md` give the algorithm), so a cash-basis balance sheet balances
//! by construction.
//!
//! [`catalog`] lists every report with its category and the settings that apply
//! to it, for the Reports menu and each report's settings panel.

pub mod catalog;
pub mod export;
pub mod model;
pub mod render;
pub mod request;

mod cash;
mod ctx;
pub mod dates;
mod filter;
mod ledger;
mod reports;
mod summary;

use jiff::civil::Date;
use lb_core::DateRange;
use lb_store::{Books, StoreError};
use rusqlite::Connection;

pub use catalog::{CatalogEntry, DatesKind, FilterKind, ReportCategory, ReportParams, catalog};
pub use export::{
    CsvOptions, ExportError, ExportFormat, ExportOptions, Paper, PdfOptions, export, export_with,
    file_name, to_csv,
};
pub use model::{Align, Cell, Column, ColumnKind, Drill, Report, Row, RowKind, Value};
pub use request::{
    Basis, ClearedFilter, ColumnLayout, ReportFilters, ReportId, ReportRequest, SortBy,
};

/// Why a report could not be run.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The request asks for something the report can't do; the message says what.
    #[error("{0}")]
    Invalid(String),
}

impl From<rusqlite::Error> for ReportError {
    fn from(e: rusqlite::Error) -> Self {
        ReportError::Store(StoreError::Sqlite(e))
    }
}

pub type Result<T, E = ReportError> = std::result::Result<T, E>;

/// Run a report against a company file, in one read transaction.
pub fn run(books: &Books, request: &ReportRequest) -> Result<Report> {
    books.read(|c| Ok(run_on(c, request)))?
}

/// Run a report on a connection the caller already holds (inside its own read
/// transaction, so the report sees one consistent snapshot).
pub fn run_on(c: &Connection, request: &ReportRequest) -> Result<Report> {
    let today = request.today.unwrap_or_else(|| jiff::Zoned::now().date());
    let ctx = ctx::Ctx::load(c, today)?;
    let run = Run::new(c, &ctx, request);
    reports::run(&run)
}

/// One report being run: the request resolved against the company.
#[derive(Debug)]
pub(crate) struct Run<'a> {
    pub c: &'a Connection,
    pub ctx: &'a ctx::Ctx,
    pub req: &'a ReportRequest,
    pub basis: Basis,
    /// The resolved dates. "As of" reports use `range.to`.
    pub range: DateRange,
    pub filter: filter::Filter,
}

impl<'a> Run<'a> {
    fn new(c: &'a Connection, ctx: &'a ctx::Ctx, req: &'a ReportRequest) -> Run<'a> {
        let basis = req
            .basis
            .unwrap_or(if ctx.info.preferences.reports_cash_basis {
                Basis::Cash
            } else {
                Basis::Accrual
            });
        let range = resolve_dates(req, ctx.today, ctx.fiscal_start());
        Run {
            c,
            ctx,
            req,
            basis,
            range,
            filter: filter::Filter::resolve(ctx, &req.filters),
        }
    }

    pub fn ledger(&self) -> ledger::Ledger<'a> {
        ledger::Ledger::new(self.c, self.ctx, self.basis, self.filter.clone())
    }

    /// The same books on the accrual basis (aging and balances are always accrual).
    pub fn accrual_ledger(&self) -> ledger::Ledger<'a> {
        ledger::Ledger::new(self.c, self.ctx, Basis::Accrual, self.filter.clone())
    }

    pub fn as_of(&self) -> Option<Date> {
        self.range.to
    }

    pub fn layout(&self) -> ColumnLayout {
        self.req
            .columns
            .unwrap_or_else(|| self.req.report.default_columns())
    }

    /// The report's title block around `columns` and `rows`.
    pub fn finish(
        &self,
        subtitle: String,
        columns: Vec<Column>,
        rows: Vec<Row>,
        notes: Vec<String>,
    ) -> Report {
        Report {
            id: self.req.report,
            title: self
                .req
                .title
                .clone()
                .unwrap_or_else(|| self.req.report.name().to_owned()),
            company_name: self.ctx.info.name.clone(),
            subtitle,
            basis: self.req.report.uses_basis().then_some(self.basis),
            generated_at: jiff::Timestamp::now(),
            columns,
            rows,
            notes,
        }
    }

    /// The date line for a range report.
    pub fn range_line(&self) -> String {
        dates::range_line(self.range)
    }

    /// The date line for an "as of" report.
    pub fn as_of_line(&self) -> String {
        dates::as_of_line(self.as_of())
    }

    /// QuickZoom to another report over `range`, keeping this request's basis,
    /// `today` and filters, with `edit` narrowing the filters. (An `Option`
    /// because every caller hands it straight to a cell's or row's `drill`.)
    #[allow(clippy::unnecessary_wraps)]
    pub fn drill_to(
        &self,
        report: ReportId,
        range: DateRange,
        edit: impl FnOnce(&mut ReportFilters),
    ) -> Option<Drill> {
        let mut filters = self.req.filters.clone();
        edit(&mut filters);
        let request = ReportRequest {
            report,
            date_macro: None,
            from: range.from,
            to: range.to,
            today: Some(self.ctx.today),
            basis: Some(self.basis),
            filters,
            ..ReportRequest::default()
        };
        // An unbounded range must say so explicitly, or the target's default
        // dates would apply.
        let request = if range.from.is_none() && range.to.is_none() {
            ReportRequest {
                date_macro: Some(lb_core::DateMacro::All),
                ..request
            }
        } else {
            request
        };
        Some(Drill::Report {
            request: Box::new(request),
        })
    }

    /// QuickZoom to a transaction (an `Option` for the same reason).
    #[allow(clippy::unnecessary_wraps)]
    pub fn txn_drill(id: lb_core::TxnId) -> Option<Drill> {
        Some(Drill::Txn { txn_id: id })
    }
}

/// The dates a request means: its macro, else its explicit range, else the
/// report's default.
pub fn resolve_dates(req: &ReportRequest, today: Date, fiscal_start: i8) -> DateRange {
    if let Some(m) = req.date_macro {
        return m.range(today, fiscal_start);
    }
    if req.from.is_some() || req.to.is_some() {
        return DateRange {
            from: req.from,
            to: req.to,
        };
    }
    match req.report.default_dates() {
        Some(m) => m.range(today, fiscal_start),
        None => DateRange::new(today, today),
    }
}
