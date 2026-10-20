# lb-reports: QuickBooks Desktop's reports

## Goal

`crates/lb-reports` turns the ledger (`lb_store::Books`) into QuickBooks Desktop's
reports in one generic shape the web UI renders without knowing which report it
is: a title block, typed columns, indented rows (Header / Data / Subtotal / Total
/ Spacer) with amounts in cents in the sign a person expects, and a QuickZoom
drill-down target on every row and summary amount. Plus a catalog of reports,
CSV export, and both accrual and cash basis.

## Environment / context

- Crate: `crates/lb-reports`. Public API in `src/lib.rs`: `run(&Books,
  &ReportRequest) -> Result<Report>`, `run_on(&Connection, ...)` (inside a
  caller's read transaction), `catalog()`, `to_csv`, `export`, `file_name`,
  `resolve_dates`.
- Reads `posting`, `txn`, `application`, `account`, `entity`, `item`,
  `list_entry`, `deposit_link`, `audit` (schema: `crates/lb-store/src/schema.rs`).
- TypeScript types: `#[ts(export)]` on request/report/catalog types;
  `cargo test -p lb-reports` writes them to `web/src/api/generated/`
  (`TS_RS_EXPORT_DIR` in `.cargo/config.toml`). Exported: ReportRequest,
  ReportFilters, ReportId, Basis, ColumnLayout, ClearedFilter, SortBy, Report,
  Column, ColumnKind, Align, Row, RowKind, Cell, Value, Drill, CatalogEntry,
  ReportCategory, ReportParams, DatesKind, FilterKind, ExportFormat.
- Every cargo run goes through `node ~/.claude/bin/cpu-slots.mjs run --slots 4`.

## Decisions already made (don't re-ask)

- **One request, one result.** Every report takes `ReportRequest` (memorized
  reports are saved requests) and returns `Report`. Fields that don't apply are
  ignored; the catalog's `ReportParams` says which apply.
- **Dates**: `date_macro` (QBD's macros, `lb_core::DateMacro`) wins over
  `from`/`to`; with neither, the report's QBD default range. Relative to
  `today` (supplied; `None` = server local date) and the company's fiscal-year
  start month. "As of" reports use `to`.
- **Basis** defaults to the company's reports preference
  (`Preferences::reports_cash_basis`); Sales Tax Liability defaults to the
  sales-tax preference instead; the 1099 Summary is always cash; aging,
  balances, banking and lists are basis-free (`Report.basis = None`).
- **Money cells** are integer cents in natural sign (income and expenses
  positive, liabilities and equity positive). Percents and quantities are
  decimal strings ("12.5" means 12.5%), never floats. Percents round to one
  place, half away from zero.
- **Drill targets** (`Drill`): `Txn { txn_id }`, `Name { entity_id }`, or
  `Report { request }`: a complete request (account + range + the column's
  name/class/item, basis, today) the UI can run as-is. "Account + range" drills
  are this variant (usually Transaction Detail by Account). A "<parent> -
  Other" amount drills with `filters.exact = true` (no subaccounts).
- **Summary reports aggregate in SQL.** Columns are *slots* (a date range and a
  dimension value); slots that share a comparison set are one grouped query
  (`GROUP BY account, substr(date,1,7) | date, dimension`), then distributed.
  Twelve monthly columns cost one query. Only detail reports load rows.
- **Hierarchies** (accounts, customers:jobs, classes, items) render as QBD
  does: parent heading, children indented, the parent's own amount as
  "<parent> - Other", then "Total <parent>"; zero subtrees hidden unless
  `show_zero_rows`.
- Job attribution: a line's own name when it is a customer (an expense passed
  on to a job), else the transaction's (invoice income postings carry no name).
  Vendor attribution: the transaction's payee when a vendor, else the line's.
- Categories follow QBD's menu, including "Jobs, Time & Mileage" for job
  profitability (one more than the brief listed).

## Cash basis: the exact algorithm

Cash basis = accrual ledger + two adjustments, each summing to zero on its own
date (so a cash-basis balance sheet balances by construction):

1. **Deferral (SQL).** On an *open item* (Invoice, StatementCharge, CreditMemo,
   Bill, VendorCredit, ItemReceipt), each posting to an income-statement
   account (and to the Sales Tax Payable role account when
   `sales_tax_on_cash_basis`) is re-pointed to the transaction's A/R or A/P
   account and name, same date (`cash::deferred_sql`, a `CASE` in the fact
   SELECT). Exception: on sales-side open items, the inventory cost pair (COGS
   and the inventory asset it relieves, identified by an inventory item) stays
   as posted; QBD counts COGS when sold.
2. **Recognition (Rust, `cash::recognitions`).** Each `application` row, in
   order (date, payer entry order, seq), relieves its target by `amount +
   discount + credit_amount` and its credit (credit memo / vendor credit) by
   `credit_amount`. For an open item T with A/R total `A` and weights `w` =
   every non-A/R posting except the cost pair, after cumulative relief `C`
   (capped at |A|) the recognised-so-far shares are
   `allocate_signed(-sign(A)·C, w)`; a payment recognises the difference from
   the previous payment's shares. Deferred postings' deltas become facts dated
   on the payment, against the payment (`txn_id` = payer, `origin` = T, with
   T's name/class/item/memo), plus one balancing A/R or A/P fact. Cumulative
   allocation means a fully paid invoice recognises each line *exactly*.
   `allocate_signed` uses `Money::allocate` (largest remainder) when all
   weights share a sign, and a signed largest-remainder (floor, then hand out
   the shortfall) when an invoice mixes charges and discount lines.
3. Detail reports merge each transaction's re-pointed A/R/A/P amounts into one
   line per account.

What stays as posted: everything not an open item (checks, deposits, sales
receipts, card charges, transfers, journal entries, paychecks), payments'
unapplied A/R, applications to journal entries, balance-sheet postings on open
items (inventory bought on a bill, fixed assets, sales tax owed on the accrual
basis). Their A/R / A/P residue shows on the cash-basis balance sheet, which is
QBD's documented behaviour.

## Reports implemented (46)

Company & Financial: Profit & Loss (by period/job/class via `columns`), P&L
Detail, P&L YTD Comparison, P&L Prev Year Comparison, P&L by Job, P&L by
Class, Balance Sheet (standard, by period), Balance Sheet Detail, Balance Sheet
Summary, Balance Sheet Prev Year Comparison, Statement of Cash Flows.
Customers: A/R Aging Summary/Detail, Customer Balance Summary/Detail, Open
Invoices. Sales: Sales by Customer Summary/Detail, Sales by Item
Summary/Detail. Jobs: Job Profitability Summary/Detail. Vendors: A/P Aging
Summary/Detail, Vendor Balance Summary/Detail, Unpaid Bills Detail, 1099
Summary. Purchases: Expenses by Vendor Summary/Detail. Inventory: Inventory
Valuation Summary. Banking: Deposit Detail, Check Detail, Missing Checks.
Accountant & Taxes: Trial Balance, General Ledger, Journal, Transaction Detail
by Account, Transaction List by Date, Account QuickReport, Audit Trail, Sales
Tax Liability. Lists: Account Listing, Customer/Vendor Contact List, Item
Listing.

Key semantics:
- Balance sheet equity: **Retained Earnings** = RE-role account's postings +
  all income less expenses before the as-of date's fiscal year; **Net Income**
  = fiscal year start through the as-of date. Trial balance: P&L accounts for
  the current fiscal year only, RE carrying earlier years.
- Aging "open as of D" from the ledger and `application` rows dated ≤ D (never
  `txn.open_balance`): `open(X) = ΣA/R(X, ≤D) − s·relief as target + s·relief
  as payer (amount+discount) + s·relief as credit`, s = +1 A/R, −1 A/P. Buckets
  Current, 1–30, 31–60, 61–90, > 90 days past the due date (credits and
  payments age from their date).
- Customer/Vendor Balance reports group by the A/R/A/P *posting's* name (a
  customer's payment of a job's invoice sits with the customer), aging by the
  open item's name; totals agree, the split between a customer and its jobs
  can differ.
- Statement of Cash Flows (indirect): net income + change in every
  balance-sheet account by `cash_flow_class` (default by type); accounts
  classed NotApplicable (banks by default) are the cash.

## Findings / gotchas

- `application.date` is the paying transaction's date, which is what makes
  as-of aging and cash-basis recognition dates possible.
- Invoice income postings have `entity_id = NULL` (it comes from the line);
  the customer is the header's. Hence the job-attribution rule above.
- `lb_store` computes no open balance for journal entries, though payments may
  be applied to them; aging handles them from postings and applications.
- Sales quantities live only on form lines (`txn.doc` JSON), not postings;
  Sales by Item reads them with `json_each` in SQL, summing decimals in Rust.
- On a balance sheet QBD prints an "Accounts Receivable" heading over the
  "Accounts Receivable" account; `Report::amount` therefore takes the first
  row with that label that has an amount in the column.

## Known gaps / what's left

- Journal entries with A/R or A/P lines are not deferred on cash basis (QBD
  defers the income side when the other line is income/expense). Their A/R
  stays on the cash-basis balance sheet.
- Cash-basis quantities (Sales by Item, Sales by Customer Detail) count only
  sales not held open in A/R; recognised amounts carry no quantity.
- Sales Tax Liability on cash basis: tax columns are cash, sales columns are
  as invoiced (noted on the report). Sales on forms without a tax item are not
  listed.
- 1099 Summary groups by expense account, not 1099 boxes (no box mapping in
  preferences yet); no $600 threshold option.
- Statement of Cash Flows and Trial Balance are single-column (no period
  columns / comparisons). Balance sheet columns can only be periods.
- Filters on cash-basis recognised amounts use the payment's type and cleared
  status (Uncleared); the amount filter uses the payment's total.
- General Ledger: if the range crosses a fiscal year end, Retained Earnings'
  jump at the year start isn't shown as its own line (the Balance Sheet Detail
  folds it into the brought-forward balance and says so in a note).
- Audit Trail ranges are UTC days of the change.
- Export: CSV only; `ExportFormat` is where XLSX/PDF go.
- Not yet: memorized reports storage (the store has a `saved` table), budgets,
  multi-currency, payroll, estimates/orders and time reports.

## Progress log

- [x] Read plan, API contract, lb-core, lb-store.
- [x] Types: request, report, catalog, CSV, ts-rs bindings.
- [x] Engine: context, filters, fact source, cash basis, slots, trees.
- [x] All 46 reports.
- [x] Tests: 28 unit + 30 integration, hand-computed; clippy and fmt clean.
- [ ] Server routes (`/reports`, `/reports/run`, `/reports/export`): lb-server's job.

## Things not to do

- Don't load every posting for summary reports; add a dimension to
  `Ledger::aggregate` instead.
- Don't compute "as of" open balances from `txn.open_balance`.
- Don't use floats for percents or averages.
- Don't change `deferred_sql` without changing `cash::role_of` to match (and
  vice versa); the cash-basis balance sheet tests catch a mismatch.
