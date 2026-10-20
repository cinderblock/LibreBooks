// FIXTURE ONLY: a report engine over the fixture books that produces lb-reports'
// generated `Report` shape, so the Report Center, the options bar and QuickZoom
// have real, drillable data without a server. It mirrors the server's catalog
// (names, categories, which options apply) and enough of its semantics for the
// UI to be exercised: column layouts, comparisons, percentages, filters, drills.
// Accrual only (a Cash request is labelled but computed accrual). The server's
// `lb-reports` is the truth; nothing outside `api/fixtures/` may import this.

import { isBalanceSheet, naturalBalance } from '~/lib/accountTypes';
import { addDays, addYears, daysBetween, fiscalQuarter, fiscalYear, firstOfMonth, formatDate, lastOfMonth, macroRange, weekStart, type IsoDate } from '~/lib/dates';
import { TXN_LABEL } from '~/lib/txnTypes';
import { ApiError } from '../errors';
import type {
  Account,
  AccountType,
  AuditEntry,
  CatalogEntry,
  Cell,
  Class,
  ColumnLayout,
  Column,
  ColumnKind,
  CompanyInfo,
  DateMacro,
  Drill,
  Entity,
  FilterKind,
  Item,
  Posting,
  Report,
  ReportCategory,
  ReportFilters,
  ReportId,
  ReportParams,
  ReportRequest,
  Row,
  RowKind,
  SortBy,
  Txn,
  TxnType,
} from '../types';
import type { StoredTxn } from './client';

export interface ReportBooks {
  info: CompanyInfo;
  /** In chart-of-accounts order. */
  accounts: Account[];
  entities: Entity[];
  items: Item[];
  classes: Class[];
  txns: StoredTxn[];
  audit: AuditEntry[];
  today: IsoDate;
}

// ---------------------------------------------------------------------------
// Catalog (a port of lb-reports' `catalog.rs`)

export const REPORT_IDS: ReportId[] = [
  'ProfitAndLoss',
  'ProfitAndLossDetail',
  'ProfitAndLossYtdComparison',
  'ProfitAndLossPrevYearComparison',
  'ProfitAndLossByJob',
  'ProfitAndLossByClass',
  'BalanceSheet',
  'BalanceSheetDetail',
  'BalanceSheetSummary',
  'BalanceSheetPrevYearComparison',
  'StatementOfCashFlows',
  'ArAgingSummary',
  'ArAgingDetail',
  'CustomerBalanceSummary',
  'CustomerBalanceDetail',
  'OpenInvoices',
  'SalesByCustomerSummary',
  'SalesByCustomerDetail',
  'SalesByItemSummary',
  'SalesByItemDetail',
  'JobProfitabilitySummary',
  'JobProfitabilityDetail',
  'ApAgingSummary',
  'ApAgingDetail',
  'VendorBalanceSummary',
  'VendorBalanceDetail',
  'UnpaidBillsDetail',
  'Vendor1099Summary',
  'ExpensesByVendorSummary',
  'ExpensesByVendorDetail',
  'InventoryValuationSummary',
  'DepositDetail',
  'CheckDetail',
  'MissingChecks',
  'TrialBalance',
  'GeneralLedger',
  'Journal',
  'TransactionDetailByAccount',
  'TransactionListByDate',
  'AccountQuickReport',
  'AuditTrail',
  'SalesTaxLiability',
  'AccountListing',
  'CustomerContactList',
  'VendorContactList',
  'ItemListing',
];

const NAME: Record<ReportId, string> = {
  ProfitAndLoss: 'Profit & Loss',
  ProfitAndLossDetail: 'Profit & Loss Detail',
  ProfitAndLossYtdComparison: 'Profit & Loss YTD Comparison',
  ProfitAndLossPrevYearComparison: 'Profit & Loss Prev Year Comparison',
  ProfitAndLossByJob: 'Profit & Loss by Job',
  ProfitAndLossByClass: 'Profit & Loss by Class',
  BalanceSheet: 'Balance Sheet',
  BalanceSheetDetail: 'Balance Sheet Detail',
  BalanceSheetSummary: 'Balance Sheet Summary',
  BalanceSheetPrevYearComparison: 'Balance Sheet Prev Year Comparison',
  StatementOfCashFlows: 'Statement of Cash Flows',
  ArAgingSummary: 'A/R Aging Summary',
  ArAgingDetail: 'A/R Aging Detail',
  CustomerBalanceSummary: 'Customer Balance Summary',
  CustomerBalanceDetail: 'Customer Balance Detail',
  OpenInvoices: 'Open Invoices',
  SalesByCustomerSummary: 'Sales by Customer Summary',
  SalesByCustomerDetail: 'Sales by Customer Detail',
  SalesByItemSummary: 'Sales by Item Summary',
  SalesByItemDetail: 'Sales by Item Detail',
  JobProfitabilitySummary: 'Job Profitability Summary',
  JobProfitabilityDetail: 'Job Profitability Detail',
  ApAgingSummary: 'A/P Aging Summary',
  ApAgingDetail: 'A/P Aging Detail',
  VendorBalanceSummary: 'Vendor Balance Summary',
  VendorBalanceDetail: 'Vendor Balance Detail',
  UnpaidBillsDetail: 'Unpaid Bills Detail',
  Vendor1099Summary: '1099 Summary',
  ExpensesByVendorSummary: 'Expenses by Vendor Summary',
  ExpensesByVendorDetail: 'Expenses by Vendor Detail',
  InventoryValuationSummary: 'Inventory Valuation Summary',
  DepositDetail: 'Deposit Detail',
  CheckDetail: 'Check Detail',
  MissingChecks: 'Missing Checks',
  TrialBalance: 'Trial Balance',
  GeneralLedger: 'General Ledger',
  Journal: 'Journal',
  TransactionDetailByAccount: 'Transaction Detail by Account',
  TransactionListByDate: 'Transaction List by Date',
  AccountQuickReport: 'Account QuickReport',
  AuditTrail: 'Audit Trail',
  SalesTaxLiability: 'Sales Tax Liability',
  AccountListing: 'Account Listing',
  CustomerContactList: 'Customer Contact List',
  VendorContactList: 'Vendor Contact List',
  ItemListing: 'Item Listing',
};

const DESCRIPTION: Record<ReportId, string> = {
  ProfitAndLoss: 'Income and expenses for a period, and the net income.',
  ProfitAndLossDetail: 'Every transaction behind each income and expense account.',
  ProfitAndLossYtdComparison: 'This period beside the fiscal year to date.',
  ProfitAndLossPrevYearComparison: 'This period beside the same period last year, with the change.',
  ProfitAndLossByJob: 'Income and expenses for each customer and job.',
  ProfitAndLossByClass: 'Income and expenses for each class.',
  BalanceSheet: 'Assets, liabilities and equity as of a date.',
  BalanceSheetDetail: 'Every transaction behind each balance sheet account, with balances.',
  BalanceSheetSummary: 'The balance sheet by account type.',
  BalanceSheetPrevYearComparison: 'The balance sheet beside the same date last year, with the change.',
  StatementOfCashFlows: 'Cash in and out by operating, investing and financing activity.',
  ArAgingSummary: 'What each customer owes, by how overdue.',
  ArAgingDetail: 'Each open invoice and credit, grouped by how overdue.',
  CustomerBalanceSummary: 'What each customer owes.',
  CustomerBalanceDetail: 'Every transaction affecting what each customer owes.',
  OpenInvoices: 'Unpaid invoices and unused credits, by customer.',
  SalesByCustomerSummary: 'Sales to each customer and job.',
  SalesByCustomerDetail: 'Each sale, by customer and job.',
  SalesByItemSummary: 'Quantity, sales and margin for each item.',
  SalesByItemDetail: 'Each sale, by item.',
  JobProfitabilitySummary: 'Revenue and cost for each customer and job, sub-jobs rolled up.',
  JobProfitabilityDetail: 'Revenue and cost for each job, by item and account.',
  ApAgingSummary: 'What is owed to each vendor, by how overdue.',
  ApAgingDetail: 'Each unpaid bill and credit, grouped by how overdue.',
  VendorBalanceSummary: 'What is owed to each vendor.',
  VendorBalanceDetail: 'Every transaction affecting what is owed to each vendor.',
  UnpaidBillsDetail: 'Unpaid bills and unused credits, by vendor.',
  Vendor1099Summary: 'Payments to 1099 vendors, by the accounts they were for.',
  ExpensesByVendorSummary: 'What was spent with each vendor.',
  ExpensesByVendorDetail: 'Each expense, by vendor.',
  InventoryValuationSummary: 'Quantity on hand, average cost and value of each inventory item.',
  DepositDetail: 'Each deposit and what went into it.',
  CheckDetail: 'Each check and payment, and what it paid for.',
  MissingChecks: 'Gaps and duplicates in check numbers.',
  TrialBalance: "Every account's debit or credit balance.",
  GeneralLedger: 'Every transaction in every account, with balances.',
  Journal: 'Every transaction with its debits and credits.',
  TransactionDetailByAccount: 'Transactions grouped by account.',
  TransactionListByDate: 'Every transaction, by date.',
  AccountQuickReport: "One account's transactions.",
  AuditTrail: 'Who entered and changed what, and when.',
  SalesTaxLiability: 'Sales tax collected and owed, by agency.',
  AccountListing: 'The chart of accounts.',
  CustomerContactList: "Customers' contact details.",
  VendorContactList: "Vendors' contact details.",
  ItemListing: 'Items, their prices and accounts.',
};

const CATEGORY_LABEL: Record<ReportCategory, string> = {
  CompanyAndFinancial: 'Company & Financial',
  CustomersAndReceivables: 'Customers & Receivables',
  Sales: 'Sales',
  Jobs: 'Jobs, Time & Mileage',
  VendorsAndPayables: 'Vendors & Payables',
  Purchases: 'Purchases',
  Inventory: 'Inventory',
  Banking: 'Banking',
  AccountantAndTaxes: 'Accountant & Taxes',
  Lists: 'Lists',
};

function category(id: ReportId): ReportCategory {
  if (id.startsWith('ProfitAndLoss') || id.startsWith('BalanceSheet') || id === 'StatementOfCashFlows') return 'CompanyAndFinancial';
  if (id.startsWith('ArAging') || id.startsWith('CustomerBalance') || id === 'OpenInvoices') return 'CustomersAndReceivables';
  if (id.startsWith('SalesBy')) return 'Sales';
  if (id.startsWith('JobProfitability')) return 'Jobs';
  if (id.startsWith('ApAging') || id.startsWith('VendorBalance') || id === 'UnpaidBillsDetail' || id === 'Vendor1099Summary') return 'VendorsAndPayables';
  if (id.startsWith('ExpensesByVendor')) return 'Purchases';
  if (id === 'InventoryValuationSummary') return 'Inventory';
  if (id === 'DepositDetail' || id === 'CheckDetail' || id === 'MissingChecks') return 'Banking';
  if (id === 'AccountListing' || id === 'CustomerContactList' || id === 'VendorContactList' || id === 'ItemListing') return 'Lists';
  return 'AccountantAndTaxes';
}

type Shape = 'Summary' | 'PeriodSummary' | 'AsOf' | 'NameSummary' | 'List' | 'Detail';

function shape(id: ReportId): Shape {
  switch (id) {
    case 'ProfitAndLoss':
    case 'ProfitAndLossByJob':
    case 'ProfitAndLossByClass':
      return 'Summary';
    case 'ProfitAndLossPrevYearComparison':
    case 'ProfitAndLossYtdComparison':
    case 'BalanceSheet':
    case 'BalanceSheetSummary':
    case 'BalanceSheetPrevYearComparison':
    case 'StatementOfCashFlows':
      return 'PeriodSummary';
    case 'ArAgingSummary':
    case 'ApAgingSummary':
    case 'CustomerBalanceSummary':
    case 'VendorBalanceSummary':
    case 'InventoryValuationSummary':
    case 'TrialBalance':
    case 'ArAgingDetail':
    case 'ApAgingDetail':
    case 'OpenInvoices':
    case 'UnpaidBillsDetail':
      return 'AsOf';
    case 'SalesByCustomerSummary':
    case 'SalesByItemSummary':
    case 'ExpensesByVendorSummary':
    case 'JobProfitabilitySummary':
    case 'Vendor1099Summary':
    case 'SalesTaxLiability':
      return 'NameSummary';
    case 'AccountListing':
    case 'CustomerContactList':
    case 'VendorContactList':
    case 'ItemListing':
      return 'List';
    default:
      return 'Detail';
  }
}

function defaultDates(id: ReportId): DateMacro | null {
  switch (id) {
    case 'BalanceSheet':
    case 'BalanceSheetDetail':
    case 'BalanceSheetSummary':
    case 'BalanceSheetPrevYearComparison':
    case 'StatementOfCashFlows':
    case 'ProfitAndLossYtdComparison':
    case 'ProfitAndLossPrevYearComparison':
      return 'ThisFiscalYearToDate';
    case 'ArAgingSummary':
    case 'ArAgingDetail':
    case 'ApAgingSummary':
    case 'ApAgingDetail':
    case 'OpenInvoices':
    case 'UnpaidBillsDetail':
    case 'InventoryValuationSummary':
    case 'Journal':
    case 'AuditTrail':
      return 'Today';
    case 'CustomerBalanceSummary':
    case 'CustomerBalanceDetail':
    case 'VendorBalanceSummary':
    case 'VendorBalanceDetail':
    case 'MissingChecks':
    case 'AccountQuickReport':
      return 'All';
    case 'TransactionListByDate':
      return 'ThisWeek';
    case 'Vendor1099Summary':
      return 'LastCalendarYear';
    case 'SalesTaxLiability':
      return 'LastMonth';
    case 'AccountListing':
    case 'CustomerContactList':
    case 'VendorContactList':
    case 'ItemListing':
      return null;
    default:
      return 'ThisMonthToDate';
  }
}

const NO_BASIS = new Set<ReportId>([
  'ArAgingSummary',
  'ArAgingDetail',
  'ApAgingSummary',
  'ApAgingDetail',
  'CustomerBalanceSummary',
  'CustomerBalanceDetail',
  'VendorBalanceSummary',
  'VendorBalanceDetail',
  'OpenInvoices',
  'UnpaidBillsDetail',
  'InventoryValuationSummary',
  'DepositDetail',
  'CheckDetail',
  'MissingChecks',
  'AuditTrail',
  'Vendor1099Summary',
  'AccountListing',
  'CustomerContactList',
  'VendorContactList',
  'ItemListing',
]);

const SUMMARY_LAYOUTS: ColumnLayout[] = ['TotalOnly', 'Week', 'Month', 'Quarter', 'Year', 'CustomerJob', 'Class', 'Vendor', 'Item'];
const PERIOD_LAYOUTS: ColumnLayout[] = ['TotalOnly', 'Week', 'Month', 'Quarter', 'Year'];
const ALL_FILTERS: FilterKind[] = ['Accounts', 'AccountTypes', 'Names', 'Classes', 'Items', 'TxnTypes', 'Cleared', 'Amount', 'Memo'];
const DETAIL_SORTS: SortBy[] = ['Default', 'Date', 'Number', 'Name', 'Memo', 'Amount'];

function params(id: ReportId): ReportParams {
  const sh = shape(id);
  const asOf = sh === 'AsOf' || id === 'BalanceSheet' || id === 'BalanceSheetSummary' || id === 'BalanceSheetPrevYearComparison';
  const isPl = ['ProfitAndLoss', 'ProfitAndLossByJob', 'ProfitAndLossByClass', 'ProfitAndLossYtdComparison', 'ProfitAndLossPrevYearComparison'].includes(id);
  let columns: ColumnLayout[] = [];
  let comparisons = false;
  if (sh === 'Summary') {
    columns = SUMMARY_LAYOUTS;
    comparisons = true;
  } else if (sh === 'PeriodSummary') {
    columns = id === 'ProfitAndLossYtdComparison' || id === 'StatementOfCashFlows' ? [] : PERIOD_LAYOUTS;
    comparisons = id !== 'ProfitAndLossYtdComparison';
  }
  return {
    dates: sh === 'List' ? 'None' : asOf ? 'AsOf' : 'Range',
    default_dates: defaultDates(id),
    basis: !NO_BASIS.has(id),
    columns,
    default_columns: id === 'ProfitAndLossByJob' ? 'CustomerJob' : id === 'ProfitAndLossByClass' ? 'Class' : 'TotalOnly',
    comparisons,
    percent_of_income: isPl,
    percent_of_column: (sh === 'Summary' || sh === 'PeriodSummary') && !isPl,
    filters: sh === 'List' ? [] : ALL_FILTERS,
    sorts: sh === 'Detail' ? DETAIL_SORTS : ['Default'],
    show_zero_rows: sh === 'Summary' || sh === 'PeriodSummary' || sh === 'AsOf' || sh === 'NameSummary',
  };
}

export function fixtureCatalog(): CatalogEntry[] {
  return REPORT_IDS.map((id) => ({ id, name: NAME[id], category: category(id), category_label: CATEGORY_LABEL[category(id)], description: DESCRIPTION[id], params: params(id) }));
}

// ---------------------------------------------------------------------------
// Cells, rows and columns

const text = (s: string | null | undefined, drill?: Drill): Cell => ({ value: s ? { type: 'Text', value: s } : { type: 'Empty' }, ...(drill ? { drill } : {}) });
const money = (c: number, drill?: Drill): Cell => ({ value: { type: 'Money', value: c }, ...(drill ? { drill } : {}) });
const date = (d: string): Cell => ({ value: { type: 'Date', value: d } });
const empty = (): Cell => ({ value: { type: 'Empty' } });
const num = (s: string | null): Cell => ({ value: s ? { type: 'Number', value: s } : { type: 'Empty' } });
/** `part / whole` as a one-place percent, or empty when the whole is zero. */
const pct = (part: number, whole: number): Cell =>
  whole === 0 ? empty() : { value: { type: 'Percent', value: (Math.round((part / whole) * 1000) / 10).toFixed(1) } };

class Rows {
  rows: Row[] = [];
  push(kind: RowKind, depth: number, cells: Cell[], drill?: Drill): Row {
    const row: Row = { kind, depth, cells, ...(drill ? { drill } : {}) };
    this.rows.push(row);
    return row;
  }
  spacer(n = 1) {
    this.push('Spacer', 0, [empty(), ...Array.from({ length: n }, empty)]);
  }
}

const col = (id: string, label: string, kind: ColumnKind, group?: string): Column => ({
  id,
  label,
  kind,
  align: kind === 'Text' || kind === 'Date' ? 'Left' : 'Right',
  ...(group ? { group } : {}),
});

const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
const longDate = (d: IsoDate) => `${MONTHS[Number(d.slice(5, 7)) - 1]} ${Number(d.slice(8, 10))}, ${d.slice(0, 4)}`;
const shortDate = (d: IsoDate) => `${MONTHS[Number(d.slice(5, 7)) - 1]!.slice(0, 3)} ${Number(d.slice(8, 10))}, ${d.slice(2, 4)}`;

/** QBD's subtitle: "January 1 through October 19, 2026", "As of …", "All Transactions". */
export function dateLine(from: IsoDate | null, to: IsoDate | null, asOf = false): string {
  if (asOf) return to ? `As of ${longDate(to)}` : 'All Transactions';
  if (!from && !to) return 'All Transactions';
  if (!from) return `Through ${longDate(to!)}`;
  if (!to) return `From ${longDate(from)}`;
  const sameYear = from.slice(0, 4) === to.slice(0, 4);
  const f = sameYear ? `${MONTHS[Number(from.slice(5, 7)) - 1]} ${Number(from.slice(8, 10))}` : longDate(from);
  return `${f} through ${longDate(to)}`;
}

/** A column heading for a date range: "Oct 1 - 19, 26", "Jan 1 - Dec 31, 26". */
function rangeLabel(from: IsoDate | null, to: IsoDate | null): string {
  if (!from && !to) return 'All';
  if (!from) return `As of ${shortDate(to!)}`;
  if (!to) return `From ${shortDate(from)}`;
  const sameMonth = from.slice(0, 7) === to.slice(0, 7);
  if (sameMonth) return `${MONTHS[Number(from.slice(5, 7)) - 1]!.slice(0, 3)} ${Number(from.slice(8, 10))} - ${Number(to.slice(8, 10))}, ${to.slice(2, 4)}`;
  return `${MONTHS[Number(from.slice(5, 7)) - 1]!.slice(0, 3)} ${Number(from.slice(8, 10))} - ${shortDate(to)}`;
}

// ---------------------------------------------------------------------------
// Dates, filters, postings

interface Resolved {
  from: IsoDate | null;
  to: IsoDate | null;
  today: IsoDate;
  asOf: boolean;
}

function resolve(books: ReportBooks, req: ReportRequest): Resolved {
  const today = req.today ?? books.today;
  const fsm = books.info.fiscal_year_start_month;
  const p = params(req.report);
  const asOf = p.dates === 'AsOf';
  if (p.dates === 'None') return { from: null, to: null, today, asOf };
  let from = req.from;
  let to = req.to;
  const macro = req.date_macro ?? (!from && !to ? p.default_dates : null);
  if (macro) ({ from, to } = macroRange(macro, today, fsm));
  if (asOf) return { from: null, to: to ?? today, today, asOf };
  return { from, to, today, asOf };
}

function descendants(all: ReadonlyArray<{ id: string; full_name: string }>, ids: readonly string[], exact: boolean): Set<string> {
  const out = new Set(ids);
  if (exact) return out;
  const roots = all.filter((a) => out.has(a.id));
  for (const a of all) if (roots.some((r) => a.full_name.startsWith(`${r.full_name}:`))) out.add(a.id);
  return out;
}

/** Does this posting (and its transaction) pass the request's filters? */
function filterFor(books: ReportBooks, f: ReportFilters): (s: StoredTxn, p: Posting) => boolean {
  const accounts = f.accounts.length ? descendants(books.accounts, f.accounts, f.exact) : null;
  const names = f.names.length ? descendants(books.entities, f.names, f.exact) : null;
  const classes = f.classes.length ? descendants(books.classes, f.classes, f.exact) : null;
  const items = f.items.length ? descendants(books.items, f.items, f.exact) : null;
  const types = f.account_types.length ? new Set<AccountType>(f.account_types) : null;
  const txnTypes = f.txn_types.length ? new Set<TxnType>(f.txn_types) : null;
  const acct = new Map(books.accounts.map((a) => [a.id, a]));
  const memo = f.memo?.toLowerCase() ?? null;
  return (s, p) => {
    if (accounts && !accounts.has(p.account_id)) return false;
    if (types && !types.has(acct.get(p.account_id)?.account_type ?? 'NonPosting')) return false;
    if (names && !((p.entity_id && names.has(p.entity_id)) || (s.txn.entity_id && names.has(s.txn.entity_id)))) return false;
    if (classes && !(p.class_id && classes.has(p.class_id))) return false;
    if (items && !(p.item_id && items.has(p.item_id))) return false;
    if (txnTypes && !txnTypes.has(s.txn.txn_type)) return false;
    if (f.cleared === 'Cleared' && p.cleared !== 'Cleared') return false;
    if (f.cleared === 'NotCleared' && p.cleared === 'Cleared') return false;
    if (f.amount_min !== null && Math.abs(s.total) < f.amount_min) return false;
    if (f.amount_max !== null && Math.abs(s.total) > f.amount_max) return false;
    if (memo && !`${s.txn.memo ?? ''} ${p.memo ?? ''}`.toLowerCase().includes(memo)) return false;
    return true;
  };
}

const inRange = (d: IsoDate, from: IsoDate | null, to: IsoDate | null) => (!from || d >= from) && (!to || d <= to);

/** Debit-positive sums per account over postings in the range passing `pred`. */
function sums(books: ReportBooks, from: IsoDate | null, to: IsoDate | null, pred: (s: StoredTxn, p: Posting) => boolean): Map<string, number> {
  const m = new Map<string, number>();
  for (const s of books.txns) {
    if (s.txn.is_void || !inRange(s.txn.date, from, to)) continue;
    for (const p of s.postings) if (pred(s, p)) m.set(p.account_id, (m.get(p.account_id) ?? 0) + p.amount);
  }
  return m;
}

// ---------------------------------------------------------------------------
// Column slots for summaries

interface Slot {
  label: string;
  group?: string;
  from: IsoDate | null;
  to: IsoDate | null;
  /** A dimension (a class, a customer, a vendor, an item); the posting must belong to it. */
  dim?: (s: StoredTxn, p: Posting) => boolean;
  /** Filters added to a drill from this column. */
  drill?: Partial<ReportFilters>;
}

function periodSlots(layout: ColumnLayout, from: IsoDate, to: IsoDate, fsm: number): Slot[] {
  const out: Slot[] = [];
  let cursor = from;
  const step = (d: IsoDate): [IsoDate, IsoDate] => {
    switch (layout) {
      case 'Week': {
        const s = weekStart(d);
        return [s, addDays(s, 6)];
      }
      case 'Quarter':
        return fiscalQuarter(d, fsm, 0);
      case 'Year':
        return fiscalYear(d, fsm, 0);
      default:
        return [firstOfMonth(d), lastOfMonth(d)];
    }
  };
  const label = ([s, e]: [IsoDate, IsoDate]): string => {
    switch (layout) {
      case 'Month':
        return `${MONTHS[Number(s.slice(5, 7)) - 1]!.slice(0, 3)} ${s.slice(2, 4)}`;
      case 'Year':
        return fsm === 1 ? s.slice(0, 4) : `FY ${e.slice(0, 4)}`;
      default:
        return rangeLabel(s, e);
    }
  };
  let guard = 0;
  while (cursor <= to && guard++ < 400) {
    const [s, e] = step(cursor);
    const clampedFrom = s < from ? from : s;
    const clampedTo = e > to ? to : e;
    out.push({ label: label([s, e]), from: clampedFrom, to: clampedTo });
    cursor = addDays(e, 1);
  }
  return out;
}

function dimensionSlots(books: ReportBooks, layout: ColumnLayout, from: IsoDate | null, to: IsoDate | null): Slot[] {
  const base = { from, to };
  const usedIn = (pick: (s: StoredTxn, p: Posting) => string | null): Set<string> => {
    const used = new Set<string>();
    for (const s of books.txns) if (!s.txn.is_void && inRange(s.txn.date, from, to)) for (const p of s.postings) {
      const v = pick(s, p);
      if (v) used.add(v);
    }
    return used;
  };
  switch (layout) {
    case 'Class': {
      const pick = (_s: StoredTxn, p: Posting) => p.class_id;
      const used = usedIn(pick);
      const slots = books.classes.filter((c) => used.has(c.id)).map<Slot>((c) => ({ ...base, label: c.full_name, dim: (s, p) => pick(s, p) === c.id, drill: { classes: [c.id], exact: true } }));
      slots.push({ ...base, label: 'Unclassified', dim: (s, p) => !pick(s, p) });
      return slots;
    }
    case 'CustomerJob': {
      // A line's own customer when it has one (an expense passed on), else the transaction's.
      const pick = (s: StoredTxn, p: Posting) => {
        const ent = p.entity_id ?? s.txn.entity_id;
        return ent && books.entities.find((e) => e.id === ent)?.kind === 'Customer' ? ent : null;
      };
      const used = usedIn(pick);
      const slots = books.entities.filter((e) => e.kind === 'Customer' && used.has(e.id)).map<Slot>((e) => ({ ...base, label: e.full_name, dim: (s, p) => pick(s, p) === e.id, drill: { names: [e.id], exact: true } }));
      slots.push({ ...base, label: 'No name', dim: (s, p) => !pick(s, p) });
      return slots;
    }
    case 'Vendor': {
      const pick = (s: StoredTxn, p: Posting) => {
        const ent = s.txn.entity_id ?? p.entity_id;
        return ent && books.entities.find((e) => e.id === ent)?.kind === 'Vendor' ? ent : null;
      };
      const used = usedIn(pick);
      const slots = books.entities.filter((e) => e.kind === 'Vendor' && used.has(e.id)).map<Slot>((e) => ({ ...base, label: e.full_name, dim: (s, p) => pick(s, p) === e.id, drill: { names: [e.id], exact: true } }));
      slots.push({ ...base, label: 'No vendor', dim: (s, p) => !pick(s, p) });
      return slots;
    }
    case 'Item': {
      const pick = (_s: StoredTxn, p: Posting) => p.item_id;
      const used = usedIn(pick);
      const slots = books.items.filter((i) => used.has(i.id)).map<Slot>((i) => ({ ...base, label: i.full_name, dim: (s, p) => pick(s, p) === i.id, drill: { items: [i.id], exact: true } }));
      slots.push({ ...base, label: 'No item', dim: (s, p) => !pick(s, p) });
      return slots;
    }
    default:
      return [{ ...base, label: rangeLabel(from, to) }];
  }
}

/** The slots a summary request asks for: layout, then comparison columns. */
function slotsFor(books: ReportBooks, req: ReportRequest, r: Resolved, asOf: boolean): { slots: Slot[]; compare: 'period' | 'year' | null; notes: string[] } {
  const notes: string[] = [];
  const fsm = books.info.fiscal_year_start_month;
  const layout = req.columns ?? params(req.report).default_columns;
  let slots: Slot[];
  if (['Week', 'Month', 'Quarter', 'Year'].includes(layout)) {
    const from = r.from ?? (asOf ? fiscalYear(r.to!, fsm, 0)[0] : books.txns.reduce((lo, s) => (s.txn.date < lo ? s.txn.date : lo), r.to ?? r.today));
    const to = r.to ?? r.today;
    slots = periodSlots(layout, from, to, fsm);
    if (asOf) slots = slots.map((s) => ({ ...s, from: null }));
  } else {
    slots = dimensionSlots(books, layout, asOf ? null : r.from, r.to ?? (asOf ? r.today : null));
  }
  let compare: 'period' | 'year' | null = null;
  const comparable = params(req.report).comparisons && slots.length === 1 && !slots[0]!.dim;
  if ((req.previous_period || req.previous_year) && !comparable) notes.push('Comparison columns apply to a single-period, total-only layout and were left out.');
  if (comparable && (req.previous_year || req.previous_period || req.report === 'ProfitAndLossPrevYearComparison' || req.report === 'BalanceSheetPrevYearComparison' || req.report === 'ProfitAndLossYtdComparison')) {
    const cur = slots[0]!;
    if (req.report === 'ProfitAndLossYtdComparison') {
      const [fy] = fiscalYear(cur.to ?? r.today, fsm, 0);
      slots = [cur, { label: `${fy.slice(0, 4)} YTD`, from: fy, to: cur.to }];
      compare = null;
    } else if (req.previous_year || req.report === 'ProfitAndLossPrevYearComparison' || req.report === 'BalanceSheetPrevYearComparison') {
      slots = [cur, { label: rangeLabel(cur.from ? addYears(cur.from, -1) : null, addYears(cur.to ?? r.today, -1)), from: cur.from ? addYears(cur.from, -1) : null, to: addYears(cur.to ?? r.today, -1) }];
      compare = 'year';
    } else {
      const to = cur.to ?? r.today;
      const from = cur.from ?? to;
      const len = daysBetween(from, to) + 1;
      slots = [cur, { label: rangeLabel(asOf ? null : addDays(from, -len), addDays(to, -len)), from: asOf ? null : addDays(from, -len), to: addDays(to, -len) }];
      compare = 'period';
    }
  }
  return { slots, compare, notes };
}

interface Expander {
  columns: Column[];
  /** The values for the slots (one per slot) → the amount cells of a row. */
  cells(values: number[], drill?: (slot: number) => Drill | undefined): Cell[];
}

/**
 * Columns for a set of slots, with the options' extra columns: a TOTAL when
 * there are several period/dimension slots, "% of Income" / "% of Column" after
 * each amount, and "$ Change" / "% Change" for comparisons.
 */
function expander(req: ReportRequest, slots: Slot[], compare: 'period' | 'year' | null, bases: number[] | null, withTotal: boolean): Expander {
  const columns: Column[] = [col('label', '', 'Text')];
  const many = slots.length > 1 && !compare && withTotal;
  const pctOf = req.percent_of_income ? '% of Income' : req.percent_of_column ? '% of Column' : null;
  slots.forEach((s, i) => {
    columns.push(col(`c${i}`, s.label, 'Money', s.group));
    if (pctOf && !compare) columns.push(col(`p${i}`, pctOf, 'Percent', s.group));
  });
  if (many) {
    columns.push(col('total', 'TOTAL', 'Money'));
    if (pctOf) columns.push(col('ptotal', pctOf, 'Percent'));
  }
  if (compare) {
    if (req.dollar_change) columns.push(col('dchange', '$ Change', 'Money'));
    if (req.percent_change) columns.push(col('pchange', '% Change', 'Percent'));
    if (pctOf) columns.push(col('p0', `${pctOf} (${slots[0]!.label})`, 'Percent'));
  }
  return {
    columns,
    cells(values, drill) {
      const out: Cell[] = [];
      values.forEach((v, i) => {
        out.push(money(v, drill?.(i)));
        if (pctOf && !compare) out.push(pct(v, bases?.[i] ?? 0));
      });
      if (many) {
        const total = values.reduce((a, b) => a + b, 0);
        out.push(money(total));
        if (pctOf) out.push(pct(total, (bases ?? []).reduce((a, b) => a + b, 0)));
      }
      if (compare) {
        const [cur = 0, prev = 0] = values;
        if (req.dollar_change) out.push(money(cur - prev));
        if (req.percent_change) out.push(pct(cur - prev, Math.abs(prev)));
        if (pctOf) out.push(pct(cur, bases?.[0] ?? 0));
      }
      return out;
    },
  };
}

// ---------------------------------------------------------------------------
// Account trees

interface Tree {
  /** Natural-sign values per slot for each account's own postings. */
  own: Map<string, number[]>;
  n: number;
}

/**
 * Emit the account tree for `accounts` (one type group) as report rows: parents
 * with activity become headers closed by "Total X"; leaves are data rows that
 * drill into the account's transaction detail; a parent's own amount is
 * "<parent> - Other". Zero branches are left out unless `showZero`. Returns the
 * group's totals per slot.
 */
function accountTree(b: Rows, accounts: Account[], tree: Tree, depth: number, ex: Expander, drill: (a: Account, slot: number | null, exact: boolean) => Drill | undefined, showZero: boolean): number[] {
  const set = new Set(accounts.map((a) => a.id));
  const zero = () => Array.from({ length: tree.n }, () => 0);
  const add = (x: number[], y: number[]) => x.map((v, i) => v + (y[i] ?? 0));
  const kids = (id: string | null) => accounts.filter((a) => (id ? a.parent_id === id : !a.parent_id || !set.has(a.parent_id)));
  const ownOf = (a: Account) => tree.own.get(a.id) ?? zero();
  const totalOf = (a: Account): number[] => kids(a.id).reduce((s, c) => add(s, totalOf(c)), ownOf(a));
  const active = (a: Account): boolean => showZero || ownOf(a).some((v) => v !== 0) || kids(a.id).some(active);
  const emit = (a: Account, d: number): number[] => {
    if (!active(a)) return zero();
    const children = kids(a.id).filter(active);
    const total = totalOf(a);
    if (children.length === 0) {
      b.push('Data', d, [text(a.name), ...ex.cells(total, (i) => drill(a, i, false))], drill(a, null, false));
      return total;
    }
    b.push('Header', d, [text(a.name), ...ex.cells(zero()).map(() => empty())]);
    const mine = ownOf(a);
    if (mine.some((v) => v !== 0) || showZero) b.push('Data', d + 1, [text(`${a.name} - Other`), ...ex.cells(mine, (i) => drill(a, i, true))], drill(a, null, true));
    for (const c of children) emit(c, d + 1);
    b.push('Subtotal', d, [text(`Total ${a.name}`), ...ex.cells(total)]);
    return total;
  };
  return kids(null).reduce((s, a) => add(s, emit(a, depth)), zero());
}

function group(b: Rows, label: string, depth: number, accounts: Account[], tree: Tree, ex: Expander, drill: (a: Account, slot: number | null, exact: boolean) => Drill | undefined, showZero: boolean): number[] {
  const zero = Array.from({ length: tree.n }, () => 0);
  const any = accounts.some((a) => (tree.own.get(a.id) ?? []).some((v) => v !== 0));
  if (!any && !showZero) return zero;
  b.push('Header', depth, [text(label), ...ex.cells(zero).map(() => empty())]);
  const total = accountTree(b, accounts, tree, depth + 1, ex, drill, showZero);
  b.push('Subtotal', depth, [text(`Total ${label}`), ...ex.cells(total)]);
  return total;
}

// ---------------------------------------------------------------------------
// The engine

function finish(books: ReportBooks, req: ReportRequest, title: string, subtitle: string, columns: Column[], rows: Row[], notes: string[] = []): Report {
  return {
    id: req.report,
    title: req.title ?? title,
    company_name: books.info.name,
    subtitle,
    basis: params(req.report).basis ? (req.basis ?? (books.info.preferences.reports_cash_basis ? 'Cash' : 'Accrual')) : null,
    generated_at: new Date().toISOString(),
    columns,
    rows,
    notes: [...(req.basis === 'Cash' ? ['Fixture mode computes cash basis as accrual.'] : []), ...notes],
  };
}

export function runFixtureReport(books: ReportBooks, req: ReportRequest): Report {
  if (!REPORT_IDS.includes(req.report)) throw new ApiError(404, 'not_found', `No report "${req.report}"`);
  const r = resolve(books, req);
  const pred = filterFor(books, req.filters);
  const accountById = new Map(books.accounts.map((a) => [a.id, a]));
  const entityName = (id: string | null) => (id ? (books.entities.find((e) => e.id === id)?.full_name ?? null) : null);
  const accountName = (id: string | null) => (id ? (accountById.get(id)?.full_name ?? null) : null);
  const natural = (id: string, debit: number) => naturalBalance(accountById.get(id)?.account_type ?? 'Expense', debit);
  const ofTypes = (...types: AccountType[]) => books.accounts.filter((a) => types.includes(a.account_type));
  const fsm = books.info.fiscal_year_start_month;

  /** Drill from an account's figure to its transactions in a slot. */
  const detailDrill = (a: Account, slot: Slot | undefined, exact: boolean, extra: Partial<ReportFilters> = {}): Drill => ({
    kind: 'Report',
    request: {
      ...req,
      report: 'TransactionDetailByAccount',
      date_macro: null,
      from: slot ? slot.from : r.from,
      to: slot ? slot.to : r.to,
      columns: null,
      previous_period: false,
      previous_year: false,
      dollar_change: false,
      percent_change: false,
      percent_of_income: false,
      percent_of_column: false,
      title: null,
      filters: { ...req.filters, accounts: [a.id], exact, ...(slot?.drill ?? {}), ...extra },
    },
  });

  switch (req.report) {
    // ---- Profit & Loss family ------------------------------------------------
    case 'ProfitAndLoss':
    case 'ProfitAndLossByJob':
    case 'ProfitAndLossByClass':
    case 'ProfitAndLossYtdComparison':
    case 'ProfitAndLossPrevYearComparison': {
      const { slots, compare, notes } = slotsFor(books, req, r, false);
      const own = new Map<string, number[]>();
      const incomeBase: number[] = [];
      slots.forEach((slot, i) => {
        const m = sums(books, slot.from, slot.to, (s, p) => pred(s, p) && (!slot.dim || slot.dim(s, p)));
        let income = 0;
        for (const [id, v] of m) {
          const a = accountById.get(id);
          if (!a || isBalanceSheet(a.account_type)) continue;
          const arr = own.get(id) ?? Array.from({ length: slots.length }, () => 0);
          arr[i] = natural(id, v);
          own.set(id, arr);
          if (a.account_type === 'Income' || a.account_type === 'OtherIncome') income += arr[i]!;
        }
        incomeBase[i] = income;
      });
      const tree: Tree = { own, n: slots.length };
      const ex = expander(req, slots, compare, req.percent_of_income ? incomeBase : null, true);
      const drill = (a: Account, i: number | null, exact: boolean) => detailDrill(a, i === null ? undefined : slots[i], exact);
      const b = new Rows();
      const minus = (x: number[], y: number[]) => x.map((v, i) => v - (y[i] ?? 0));
      const plus = (x: number[], y: number[]) => x.map((v, i) => v + (y[i] ?? 0));
      b.push('Header', 0, [text('Ordinary Income/Expense'), ...ex.cells(incomeBase.map(() => 0)).map(() => empty())]);
      const income = group(b, 'Income', 1, ofTypes('Income'), tree, ex, drill, req.show_zero_rows);
      const cogs = group(b, 'Cost of Goods Sold', 1, ofTypes('CostOfGoodsSold'), tree, ex, drill, req.show_zero_rows);
      b.push('Total', 1, [text('Gross Profit'), ...ex.cells(minus(income, cogs))]);
      const expense = group(b, 'Expense', 1, ofTypes('Expense'), tree, ex, drill, req.show_zero_rows);
      const ordinary = minus(minus(income, cogs), expense);
      b.push('Subtotal', 0, [text('Net Ordinary Income'), ...ex.cells(ordinary)]);
      let other = ordinary.map(() => 0);
      if (ofTypes('OtherIncome', 'OtherExpense').some((a) => own.get(a.id)?.some((v) => v)) || req.show_zero_rows) {
        b.push('Header', 0, [text('Other Income/Expense'), ...ex.cells(other).map(() => empty())]);
        const oi = group(b, 'Other Income', 1, ofTypes('OtherIncome'), tree, ex, drill, req.show_zero_rows);
        const oe = group(b, 'Other Expense', 1, ofTypes('OtherExpense'), tree, ex, drill, req.show_zero_rows);
        other = minus(oi, oe);
        b.push('Subtotal', 0, [text('Net Other Income'), ...ex.cells(other)]);
      }
      b.push('Total', 0, [text('Net Income'), ...ex.cells(plus(ordinary, other))]);
      return finish(books, req, NAME[req.report], dateLine(r.from, r.to), ex.columns, b.rows, notes);
    }

    // ---- Balance sheet family ------------------------------------------------
    case 'BalanceSheet':
    case 'BalanceSheetSummary':
    case 'BalanceSheetPrevYearComparison': {
      const { slots, compare, notes } = slotsFor(books, req, r, true);
      const own = new Map<string, number[]>();
      const netIncome: number[] = [];
      const set = (id: string, i: number, v: number) => {
        const arr = own.get(id) ?? Array.from({ length: slots.length }, () => 0);
        arr[i] = v;
        own.set(id, arr);
      };
      slots.forEach((slot, i) => {
        const asOf = slot.to ?? r.today;
        const [fyStart] = fiscalYear(asOf, fsm, 0);
        const cumulative = sums(books, null, asOf, pred);
        let prior = 0;
        let ytd = 0;
        for (const [id, v] of cumulative) {
          const a = accountById.get(id);
          if (!a) continue;
          if (isBalanceSheet(a.account_type)) set(id, i, natural(id, v));
          else {
            const y = sums(books, fyStart, asOf, (s, p) => p.account_id === id && pred(s, p)).get(id) ?? 0;
            ytd += y;
            prior += v - y;
          }
        }
        const re = books.accounts.find((a) => a.role === 'RetainedEarnings');
        if (re) set(re.id, i, (own.get(re.id)?.[i] ?? 0) - prior);
        netIncome[i] = -ytd;
      });
      const tree: Tree = { own, n: slots.length };
      const summary = req.report === 'BalanceSheetSummary';
      const ex = expander(req, slots, compare, null, true);
      const drill = (a: Account, i: number | null, exact: boolean) => detailDrill(a, i !== null && slots[i] ? { ...slots[i]!, from: null } : { label: '', from: null, to: r.to ?? r.today }, exact);
      const b = new Rows();
      const plus = (x: number[], y: number[]) => x.map((v, i) => v + (y[i] ?? 0));
      const zero = slots.map(() => 0);
      const grp = (label: string, depth: number, types: AccountType[]) => {
        if (!summary) return group(b, label, depth, ofTypes(...types), tree, ex, drill, req.show_zero_rows);
        const total = ofTypes(...types).reduce((s, a) => plus(s, own.get(a.id) ?? zero), zero);
        if (total.some((v) => v) || req.show_zero_rows) b.push('Data', depth, [text(label), ...ex.cells(total)]);
        return total;
      };
      const hdr = (label: string, depth: number) => b.push('Header', depth, [text(label), ...zero.map(() => empty())]);
      hdr('ASSETS', 0);
      hdr('Current Assets', 1);
      const bank = grp('Checking/Savings', 2, ['Bank']);
      const ar = grp('Accounts Receivable', 2, ['AccountsReceivable']);
      const oca = grp('Other Current Assets', 2, ['OtherCurrentAsset']);
      const currentAssets = plus(plus(bank, ar), oca);
      b.push('Subtotal', 1, [text('Total Current Assets'), ...ex.cells(currentAssets)]);
      const fixed = grp('Fixed Assets', 1, ['FixedAsset']);
      const otherA = grp('Other Assets', 1, ['OtherAsset']);
      const assets = plus(plus(currentAssets, fixed), otherA);
      b.push('Subtotal', 0, [text('TOTAL ASSETS'), ...ex.cells(assets)]);
      hdr('LIABILITIES & EQUITY', 0);
      hdr('Liabilities', 1);
      hdr('Current Liabilities', 2);
      const ap = grp('Accounts Payable', 3, ['AccountsPayable']);
      const cc = grp('Credit Cards', 3, ['CreditCard']);
      const ocl = grp('Other Current Liabilities', 3, ['OtherCurrentLiability']);
      const currentL = plus(plus(ap, cc), ocl);
      b.push('Subtotal', 2, [text('Total Current Liabilities'), ...ex.cells(currentL)]);
      const ltl = grp('Long Term Liabilities', 2, ['LongTermLiability']);
      const liabilities = plus(currentL, ltl);
      b.push('Subtotal', 1, [text('Total Liabilities'), ...ex.cells(liabilities)]);
      hdr('Equity', 1);
      const equityAccounts = summary ? grp('Equity', 2, ['Equity']) : accountTree(b, ofTypes('Equity'), tree, 2, ex, drill, req.show_zero_rows);
      b.push('Data', 2, [
        text('Net Income'),
        ...ex.cells(netIncome, (i) => ({
          kind: 'Report',
          request: { ...req, report: 'ProfitAndLoss', date_macro: null, from: fiscalYear(slots[i]?.to ?? r.today, fsm, 0)[0], to: slots[i]?.to ?? r.today, columns: null, title: null },
        })),
      ]);
      const equity = plus(equityAccounts, netIncome);
      b.push('Subtotal', 1, [text('Total Equity'), ...ex.cells(equity)]);
      b.push('Subtotal', 0, [text('TOTAL LIABILITIES & EQUITY'), ...ex.cells(plus(liabilities, equity))]);
      return finish(books, req, NAME[req.report], dateLine(null, r.to, true), ex.columns, b.rows, notes);
    }

    case 'StatementOfCashFlows': {
      const from = r.from ?? fiscalYear(r.to ?? r.today, fsm, 0)[0];
      const to = r.to ?? r.today;
      const period = sums(books, from, to, pred);
      const b = new Rows();
      let netIncome = 0;
      for (const [id, v] of period) if (!isBalanceSheet(accountById.get(id)?.account_type ?? 'Bank')) netIncome -= v;
      const columns = [col('label', '', 'Text'), col('c0', rangeLabel(from, to), 'Money')];
      const change = (types: AccountType[]) =>
        ofTypes(...types)
          .filter((a) => !a.role || a.role !== 'RetainedEarnings')
          .map((a) => ({ a, v: -(period.get(a.id) ?? 0) }))
          .filter(({ v }) => v !== 0 || req.show_zero_rows);
      const section = (label: string, rows: Array<{ a: Account; v: number }>, lead?: [string, number]) => {
        b.push('Header', 0, [text(label), empty()]);
        let total = lead?.[1] ?? 0;
        if (lead) b.push('Data', 1, [text(lead[0]), money(lead[1])]);
        if (rows.length) b.push('Header', 1, [text('Adjustments to reconcile Net Income to net cash'), empty()]);
        for (const { a, v } of rows) {
          b.push('Data', 2, [text(a.full_name), money(v, detailDrill(a, { label: '', from, to }, false))]);
          total += v;
        }
        b.push('Subtotal', 0, [text(`Net cash provided by ${label}`), money(total)]);
        return total;
      };
      const op = section('OPERATING ACTIVITIES', change(['AccountsReceivable', 'OtherCurrentAsset', 'AccountsPayable', 'CreditCard', 'OtherCurrentLiability']), ['Net Income', netIncome]);
      const inv = section('INVESTING ACTIVITIES', change(['FixedAsset', 'OtherAsset']));
      const fin = section('FINANCING ACTIVITIES', change(['LongTermLiability', 'Equity']));
      const net = op + inv + fin;
      b.push('Total', 0, [text('Net cash increase for period'), money(net)]);
      const opening = [...sums(books, null, addDays(from, -1), pred)].filter(([id]) => accountById.get(id)?.account_type === 'Bank').reduce((s, [, v]) => s + v, 0);
      b.push('Data', 0, [text('Cash at beginning of period'), money(opening)]);
      b.push('Total', 0, [text('Cash at end of period'), money(opening + net)]);
      return finish(books, req, NAME[req.report], dateLine(from, to), columns, b.rows, ['Indirect method; bank accounts are the cash.']);
    }

    case 'TrialBalance': {
      const asOf = r.to ?? r.today;
      const [fyStart] = fiscalYear(asOf, fsm, 0);
      const cumulative = sums(books, null, asOf, pred);
      const ytd = sums(books, fyStart, asOf, pred);
      let prior = 0;
      const b = new Rows();
      let dr = 0;
      let cr = 0;
      const rows: Array<[Account, number]> = [];
      for (const a of books.accounts) {
        const c = cumulative.get(a.id) ?? 0;
        if (isBalanceSheet(a.account_type)) rows.push([a, c]);
        else {
          const y = ytd.get(a.id) ?? 0;
          prior += c - y;
          rows.push([a, y]);
        }
      }
      const re = books.accounts.find((a) => a.role === 'RetainedEarnings');
      for (const row of rows) {
        const [a] = row;
        if (re && a.id === re.id) row[1] += prior;
        const v = row[1];
        if (!v && !req.show_zero_rows) continue;
        if (v > 0) dr += v;
        else cr -= v;
        b.push('Data', 0, [text(a.full_name), v > 0 ? money(v, detailDrill(a, { label: '', from: null, to: asOf }, false)) : empty(), v < 0 ? money(-v, detailDrill(a, { label: '', from: null, to: asOf }, false)) : empty()]);
      }
      b.push('Total', 0, [text('TOTAL'), money(dr), money(cr)]);
      return finish(books, req, 'Trial Balance', dateLine(null, asOf, true), [col('label', '', 'Text'), col('debit', 'Debit', 'Money'), col('credit', 'Credit', 'Money')], b.rows);
    }

    // ---- Detail reports --------------------------------------------------------
    case 'AccountQuickReport':
    case 'TransactionDetailByAccount':
    case 'GeneralLedger':
    case 'ProfitAndLossDetail':
    case 'BalanceSheetDetail': {
      const quick = req.report === 'AccountQuickReport';
      if (quick && req.filters.accounts.length === 0) throw new ApiError(400, 'invalid', 'Choose an account for the QuickReport', { field: 'accounts' });
      let accounts = books.accounts;
      if (req.report === 'ProfitAndLossDetail') accounts = accounts.filter((a) => !isBalanceSheet(a.account_type));
      if (req.report === 'BalanceSheetDetail') accounts = accounts.filter((a) => isBalanceSheet(a.account_type));
      if (req.filters.accounts.length) {
        const fam = descendants(books.accounts, req.filters.accounts, req.filters.exact);
        accounts = accounts.filter((a) => fam.has(a.id));
      }
      const b = new Rows();
      const columns = [col('type', 'Type', 'Text'), col('date', 'Date', 'Date'), col('num', 'Num', 'Text'), col('name', 'Name', 'Text'), col('memo', 'Memo', 'Text'), col('split', 'Split', 'Text'), col('amount', 'Amount', 'Money'), col('balance', 'Balance', 'Money')];
      const sorted = [...books.txns].sort((x, y) => (x.txn.date < y.txn.date ? -1 : x.txn.date > y.txn.date ? 1 : x.seq - y.seq));
      let grand = 0;
      const emitAccount = (a: Account, depth: number) => {
        const bs = isBalanceSheet(a.account_type);
        const mine = sorted.filter((s) => !s.txn.is_void && s.postings.some((p) => p.account_id === a.id && pred(s, p)));
        if (!mine.length && !req.show_zero_rows) return;
        let balance = bs ? mine.filter((s) => r.from && s.txn.date < r.from).reduce((t, s) => t + natural(a.id, s.postings.filter((p) => p.account_id === a.id).reduce((u, p) => u + p.amount, 0)), 0) : 0;
        const inside = mine.filter((s) => inRange(s.txn.date, r.from, r.to));
        if (!inside.length && !req.show_zero_rows) return;
        b.push('Header', depth, [text(a.full_name), empty(), empty(), empty(), empty(), empty(), empty(), empty()]);
        if (bs && r.from) b.push('Data', depth + 1, [text('Beginning Balance'), empty(), empty(), empty(), empty(), empty(), empty(), money(balance)]);
        const detail = inside.map((s) => {
          const amt = natural(a.id, s.postings.filter((p) => p.account_id === a.id && pred(s, p)).reduce((t, p) => t + p.amount, 0));
          const others = [...new Set(s.postings.filter((p) => p.account_id !== a.id).map((p) => p.account_id))];
          return { s, amt, split: others.length === 1 ? accountName(others[0]!) : others.length ? '-SPLIT-' : null };
        });
        sortDetail(detail.map((d) => ({ ...d, name: entityName(d.s.txn.entity_id) })), req);
        let total = 0;
        for (const d of detail) {
          balance += d.amt;
          total += d.amt;
          b.push('Data', depth + 1, [text(TXN_LABEL[d.s.txn.txn_type]), date(d.s.txn.date), text(d.s.txn.number), text(entityName(d.s.txn.entity_id)), text(d.s.txn.memo), text(d.split), money(d.amt), money(balance)], { kind: 'Txn', txn_id: d.s.txn.id });
        }
        b.push('Subtotal', depth, [text(`Total ${a.full_name}`), empty(), empty(), empty(), empty(), empty(), money(total), money(balance)]);
        grand += total;
      };
      for (const a of accounts) emitAccount(a, 0);
      if (!quick) b.push('Total', 0, [text('TOTAL'), empty(), empty(), empty(), empty(), empty(), money(grand), empty()]);
      const title = quick ? `${accountName(req.filters.accounts[0]!) ?? 'Account'} QuickReport` : NAME[req.report];
      return finish(books, req, title, dateLine(r.from, r.to), columns, b.rows);
    }

    case 'TransactionListByDate':
    case 'CheckDetail':
    case 'DepositDetail':
    case 'ExpensesByVendorDetail':
    case 'SalesByCustomerDetail':
    case 'CustomerBalanceDetail':
    case 'VendorBalanceDetail': {
      const types: TxnType[] | null =
        req.report === 'CheckDetail'
          ? ['Check', 'BillPaymentCheck', 'SalesTaxPaymentCheck', 'LiabilityCheck', 'Paycheck']
          : req.report === 'DepositDetail'
            ? ['Deposit']
            : req.report === 'ExpensesByVendorDetail'
              ? ['Check', 'Bill', 'CreditCardCharge', 'CreditCardCredit', 'VendorCredit']
              : req.report === 'SalesByCustomerDetail'
                ? ['Invoice', 'SalesReceipt', 'CreditMemo', 'StatementCharge']
                : req.report === 'CustomerBalanceDetail'
                  ? ['Invoice', 'CreditMemo', 'ReceivePayment', 'StatementCharge']
                  : req.report === 'VendorBalanceDetail'
                    ? ['Bill', 'VendorCredit', 'BillPaymentCheck', 'BillPaymentCreditCard']
                    : null;
      const grouped = ['ExpensesByVendorDetail', 'SalesByCustomerDetail', 'CustomerBalanceDetail', 'VendorBalanceDetail'].includes(req.report);
      const kind: Entity['kind'] = req.report === 'ExpensesByVendorDetail' || req.report === 'VendorBalanceDetail' ? 'Vendor' : 'Customer';
      const rows = [...books.txns]
        .filter((s) => !s.txn.is_void && inRange(s.txn.date, r.from, r.to) && (!types || types.includes(s.txn.txn_type)) && s.postings.some((p) => pred(s, p)))
        .sort((x, y) => (x.txn.date < y.txn.date ? -1 : x.txn.date > y.txn.date ? 1 : x.seq - y.seq));
      const columns = [col('type', 'Type', 'Text'), col('date', 'Date', 'Date'), col('num', 'Num', 'Text'), col('name', 'Name', 'Text'), col('memo', 'Memo', 'Text'), col('account', 'Account', 'Text'), col('split', 'Split', 'Text'), col('amount', 'Amount', 'Money')];
      const b = new Rows();
      const line = (s: StoredTxn, depth: number) => {
        const main = s.txn.account_id ?? s.postings.find((p) => !p.line_id)?.account_id ?? null;
        const others = [...new Set(s.postings.filter((p) => p.account_id !== main).map((p) => p.account_id))];
        const signed = ['CreditMemo', 'VendorCredit', 'ReceivePayment', 'BillPaymentCheck', 'BillPaymentCreditCard'].includes(s.txn.txn_type) && grouped ? -s.total : s.total;
        b.push('Data', depth, [text(TXN_LABEL[s.txn.txn_type]), date(s.txn.date), text(s.txn.number), text(entityName(s.txn.entity_id)), text(s.txn.memo), text(accountName(main)), text(others.length === 1 ? accountName(others[0]!) : others.length ? '-SPLIT-' : null), money(signed)], { kind: 'Txn', txn_id: s.txn.id });
        return signed;
      };
      if (!grouped) {
        const detail = rows.map((s) => ({ s, amt: s.total, name: entityName(s.txn.entity_id) }));
        sortDetail(detail, req);
        let total = 0;
        for (const d of detail) total += line(d.s, 0);
        b.push('Total', 0, [text('TOTAL'), empty(), empty(), empty(), empty(), empty(), empty(), money(total)]);
      } else {
        const names = books.entities.filter((e) => e.kind === kind);
        let grand = 0;
        for (const e of names) {
          const mine = rows.filter((s) => s.txn.entity_id === e.id);
          if (!mine.length) continue;
          b.push('Header', 0, [text(e.full_name), empty(), empty(), empty(), empty(), empty(), empty(), empty()]);
          let total = 0;
          for (const s of mine) total += line(s, 1);
          b.push('Subtotal', 0, [text(`Total ${e.full_name}`), empty(), empty(), empty(), empty(), empty(), empty(), money(total)]);
          grand += total;
        }
        b.push('Total', 0, [text('TOTAL'), empty(), empty(), empty(), empty(), empty(), empty(), money(grand)]);
      }
      const nameId = req.filters.names[0];
      const title = req.report === 'TransactionListByDate' && nameId ? `Transactions: ${entityName(nameId) ?? ''}` : NAME[req.report];
      return finish(books, req, title, dateLine(r.from, r.to), columns, b.rows);
    }

    case 'Journal': {
      const rows = [...books.txns]
        .filter((s) => !s.txn.is_void && inRange(s.txn.date, r.from, r.to) && s.postings.some((p) => pred(s, p)))
        .sort((x, y) => (x.txn.date < y.txn.date ? -1 : x.txn.date > y.txn.date ? 1 : x.seq - y.seq));
      const b = new Rows();
      let dr = 0;
      let cr = 0;
      rows.forEach((s, i) => {
        b.push('Header', 0, [text(String(i + 1)), text(TXN_LABEL[s.txn.txn_type]), date(s.txn.date), text(s.txn.number), text(entityName(s.txn.entity_id)), text(s.txn.memo), empty(), empty(), empty()], { kind: 'Txn', txn_id: s.txn.id });
        let d = 0;
        let c = 0;
        for (const p of s.postings) {
          b.push('Data', 1, [empty(), empty(), empty(), empty(), text(entityName(p.entity_id)), text(p.memo), text(accountName(p.account_id)), p.amount > 0 ? money(p.amount) : empty(), p.amount < 0 ? money(-p.amount) : empty()], { kind: 'Txn', txn_id: s.txn.id });
          if (p.amount > 0) d += p.amount;
          else c -= p.amount;
        }
        b.push('Subtotal', 0, [empty(), empty(), empty(), empty(), empty(), empty(), empty(), money(d), money(c)]);
        dr += d;
        cr += c;
      });
      b.push('Total', 0, [text('TOTAL'), empty(), empty(), empty(), empty(), empty(), empty(), money(dr), money(cr)]);
      const columns = [col('trans', 'Trans #', 'Text'), col('type', 'Type', 'Text'), col('date', 'Date', 'Date'), col('num', 'Num', 'Text'), col('name', 'Name', 'Text'), col('memo', 'Memo', 'Text'), col('account', 'Account', 'Text'), col('debit', 'Debit', 'Money'), col('credit', 'Credit', 'Money')];
      return finish(books, req, 'Journal', dateLine(r.from, r.to), columns, b.rows);
    }

    case 'MissingChecks': {
      const bank = req.filters.accounts[0] ?? books.accounts.find((a) => a.account_type === 'Bank')?.id;
      const checks = books.txns
        .filter((s) => (s.txn.txn_type === 'Check' || s.txn.txn_type === 'BillPaymentCheck') && s.txn.account_id === bank && s.txn.number && /^\d+$/.test(s.txn.number) && inRange(s.txn.date, r.from, r.to))
        .sort((x, y) => Number(x.txn.number) - Number(y.txn.number));
      const b = new Rows();
      let prev: number | null = null;
      for (const s of checks) {
        const n = Number(s.txn.number);
        if (prev !== null && n > prev + 1) b.push('Data', 0, [text(n - prev - 1 === 1 ? '*** Missing number' : `*** Missing numbers ${prev + 1} - ${n - 1}`), empty(), empty(), empty(), empty(), empty()]);
        if (prev === n) b.push('Data', 0, [text('*** Duplicate number'), empty(), empty(), empty(), empty(), empty()]);
        b.push('Data', 0, [text(TXN_LABEL[s.txn.txn_type]), date(s.txn.date), text(s.txn.number), text(entityName(s.txn.entity_id)), text(s.txn.memo), money(-s.total)], { kind: 'Txn', txn_id: s.txn.id });
        prev = n;
      }
      const columns = [col('type', 'Type', 'Text'), col('date', 'Date', 'Date'), col('num', 'Num', 'Text'), col('name', 'Name', 'Text'), col('memo', 'Memo', 'Text'), col('amount', 'Amount', 'Money')];
      return finish(books, req, `Missing Checks${bank ? `: ${accountName(bank) ?? ''}` : ''}`, dateLine(r.from, r.to), columns, b.rows);
    }

    // ---- Names: balances, aging, open items ---------------------------------
    case 'CustomerBalanceSummary':
    case 'VendorBalanceSummary':
    case 'SalesByCustomerSummary':
    case 'ExpensesByVendorSummary':
    case 'JobProfitabilitySummary':
    case 'Vendor1099Summary': {
      const customer = ['CustomerBalanceSummary', 'SalesByCustomerSummary', 'JobProfitabilitySummary'].includes(req.report);
      const kind: Entity['kind'] = customer ? 'Customer' : 'Vendor';
      const asOf = r.to ?? r.today;
      const balanceReport = req.report === 'CustomerBalanceSummary' || req.report === 'VendorBalanceSummary';
      const from = balanceReport ? null : r.from;
      const own = new Map<string, number[]>();
      const add = (id: string, i: number, v: number) => {
        const arr = own.get(id) ?? [0, 0, 0];
        arr[i] = (arr[i] ?? 0) + v;
        own.set(id, arr);
      };
      for (const s of books.txns) {
        if (s.txn.is_void || !inRange(s.txn.date, from, asOf)) continue;
        for (const p of s.postings) {
          if (!pred(s, p)) continue;
          const a = accountById.get(p.account_id);
          if (!a) continue;
          const ent = p.entity_id ?? s.txn.entity_id;
          const entity = ent ? books.entities.find((e) => e.id === ent) : undefined;
          if (!entity || entity.kind !== kind) continue;
          if (balanceReport) {
            if (a.account_type === (customer ? 'AccountsReceivable' : 'AccountsPayable')) add(entity.id, 0, customer ? p.amount : -p.amount);
          } else if (req.report === 'SalesByCustomerSummary') {
            if (a.account_type === 'Income' || a.account_type === 'OtherIncome') add(entity.id, 0, -p.amount);
          } else if (req.report === 'ExpensesByVendorSummary') {
            if (a.account_type === 'Expense' || a.account_type === 'CostOfGoodsSold' || a.account_type === 'OtherExpense') add(entity.id, 0, p.amount);
          } else if (req.report === 'JobProfitabilitySummary') {
            if (a.account_type === 'Income' || a.account_type === 'OtherIncome') add(entity.id, 0, -p.amount);
            if (a.account_type === 'Expense' || a.account_type === 'CostOfGoodsSold' || a.account_type === 'OtherExpense') add(entity.id, 1, p.amount);
          } else if (req.report === 'Vendor1099Summary') {
            if (entity.is_1099 && (a.account_type === 'Expense' || a.account_type === 'CostOfGoodsSold') && ['Check', 'BillPaymentCheck', 'CreditCardCharge'].includes(s.txn.txn_type)) add(entity.id, 0, p.amount);
          }
        }
      }
      const names = books.entities.filter((e) => e.kind === kind && (req.report !== 'Vendor1099Summary' || e.is_1099));
      const asAccounts = names.map((e) => ({ ...e, account_type: 'AccountsReceivable' as const })) as unknown as Account[];
      const job = req.report === 'JobProfitabilitySummary';
      const slots: Slot[] = job
        ? [
            { label: 'Act. Revenue', from, to: asOf },
            { label: 'Act. Cost', from, to: asOf },
            { label: '($) Diff.', from, to: asOf },
          ]
        : [{ label: balanceReport ? shortDate(asOf) : rangeLabel(from, asOf), from, to: asOf }];
      for (const [id, arr] of own) if (job) arr[2] = (arr[0] ?? 0) - (arr[1] ?? 0);
      const tree: Tree = { own, n: slots.length };
      const ex = expander(req, slots, null, null, false);
      const drill = (e: Account): Drill => ({
        kind: 'Report',
        request: { ...req, report: customer ? 'CustomerBalanceDetail' : 'VendorBalanceDetail', date_macro: null, from, to: asOf, title: null, filters: { ...req.filters, names: [e.id] } },
      });
      const b = new Rows();
      const total = accountTree(b, asAccounts, tree, 0, ex, (a) => drill(a), req.show_zero_rows);
      b.push('Total', 0, [text('TOTAL'), ...ex.cells(total)]);
      return finish(books, req, NAME[req.report], balanceReport ? dateLine(null, asOf, true) : dateLine(from, asOf), ex.columns, b.rows);
    }

    case 'ArAgingSummary':
    case 'ApAgingSummary':
    case 'ArAgingDetail':
    case 'ApAgingDetail':
    case 'OpenInvoices':
    case 'UnpaidBillsDetail': {
      const receivable = req.report.startsWith('Ar') || req.report === 'OpenInvoices';
      const asOf = r.to ?? r.today;
      const openTypes: TxnType[] = receivable ? ['Invoice', 'StatementCharge', 'CreditMemo'] : ['Bill', 'VendorCredit'];
      const credits: TxnType[] = receivable ? ['CreditMemo'] : ['VendorCredit'];
      const items = books.txns
        .filter((s) => !s.txn.is_void && openTypes.includes(s.txn.txn_type) && s.txn.date <= asOf && s.txn.entity_id)
        .map((s) => {
          const applied = books.txns.filter((o) => !o.txn.is_void && o.txn.date <= asOf).reduce((t, o) => t + o.txn.applications.filter((a) => a.target_txn_id === s.txn.id).reduce((u, a) => u + a.amount + a.discount + a.credit_amount, 0), 0);
          const open = s.total - applied;
          return { s, open: credits.includes(s.txn.txn_type) ? -open : open };
        })
        .filter(({ open }) => open !== 0);
      const bucket = (s: StoredTxn): number => {
        const due = s.txn.due_date ?? s.txn.date;
        const late = daysBetween(due, asOf);
        return late <= 0 ? 0 : late <= 30 ? 1 : late <= 60 ? 2 : late <= 90 ? 3 : 4;
      };
      const names = books.entities.filter((e) => e.kind === (receivable ? 'Customer' : 'Vendor'));
      const b = new Rows();
      if (req.report === 'ArAgingSummary' || req.report === 'ApAgingSummary') {
        const labels = ['Current', '1 - 30', '31 - 60', '61 - 90', '> 90'];
        const own = new Map<string, number[]>();
        for (const it of items) {
          const arr = own.get(it.s.txn.entity_id!) ?? [0, 0, 0, 0, 0];
          arr[bucket(it.s)] = (arr[bucket(it.s)] ?? 0) + it.open;
          own.set(it.s.txn.entity_id!, arr);
        }
        const slots: Slot[] = labels.map((label) => ({ label, from: null, to: asOf }));
        const ex = expander(req, slots, null, null, true);
        const asAccounts = names.map((e) => ({ ...e, account_type: 'AccountsReceivable' as const })) as unknown as Account[];
        const total = accountTree(b, asAccounts, { own, n: 5 }, 0, ex, (e) => ({ kind: 'Report', request: { ...req, report: receivable ? 'ArAgingDetail' : 'ApAgingDetail', title: null, filters: { ...req.filters, names: [e.id] } } }), req.show_zero_rows);
        b.push('Total', 0, [text('TOTAL'), ...ex.cells(total)]);
        return finish(books, req, NAME[req.report], dateLine(null, asOf, true), ex.columns, b.rows);
      }
      const columns = [col('type', 'Type', 'Text'), col('date', 'Date', 'Date'), col('num', 'Num', 'Text'), col('name', 'Name', 'Text'), col('due', 'Due Date', 'Date'), col('aging', 'Aging', 'Number'), col('open', 'Open Balance', 'Money')];
      const line = (it: { s: StoredTxn; open: number }, depth: number) => {
        const late = daysBetween(it.s.txn.due_date ?? it.s.txn.date, asOf);
        b.push('Data', depth, [text(TXN_LABEL[it.s.txn.txn_type]), date(it.s.txn.date), text(it.s.txn.number), text(entityName(it.s.txn.entity_id)), it.s.txn.due_date ? date(it.s.txn.due_date) : empty(), num(late > 0 ? String(late) : null), money(it.open)], { kind: 'Txn', txn_id: it.s.txn.id });
      };
      let grand = 0;
      if (req.report === 'OpenInvoices' || req.report === 'UnpaidBillsDetail') {
        for (const e of names) {
          const mine = items.filter((it) => it.s.txn.entity_id === e.id);
          if (!mine.length) continue;
          b.push('Header', 0, [text(e.full_name), empty(), empty(), empty(), empty(), empty(), empty()]);
          let total = 0;
          for (const it of mine) {
            line(it, 1);
            total += it.open;
          }
          b.push('Subtotal', 0, [text(`Total ${e.full_name}`), empty(), empty(), empty(), empty(), empty(), money(total)]);
          grand += total;
        }
      } else {
        const labels = ['Current', '1 - 30', '31 - 60', '61 - 90', '> 90'];
        labels.forEach((label, i) => {
          const mine = items.filter((it) => bucket(it.s) === i && (!req.filters.names.length || descendants(books.entities, req.filters.names, req.filters.exact).has(it.s.txn.entity_id!)));
          if (!mine.length) return;
          b.push('Header', 0, [text(label), empty(), empty(), empty(), empty(), empty(), empty()]);
          let total = 0;
          for (const it of mine) {
            line(it, 1);
            total += it.open;
          }
          b.push('Subtotal', 0, [text(`Total ${label}`), empty(), empty(), empty(), empty(), empty(), money(total)]);
          grand += total;
        });
      }
      b.push('Total', 0, [text('TOTAL'), empty(), empty(), empty(), empty(), empty(), money(grand)]);
      return finish(books, req, NAME[req.report], dateLine(null, asOf, true), columns, b.rows);
    }

    case 'SalesByItemSummary':
    case 'SalesByItemDetail': {
      const columns = req.report === 'SalesByItemSummary'
        ? [col('label', '', 'Text'), col('qty', 'Qty', 'Number'), col('amount', 'Amount', 'Money'), col('pct', '% of Sales', 'Percent'), col('avg', 'Avg Price', 'Money')]
        : [col('type', 'Type', 'Text'), col('date', 'Date', 'Date'), col('num', 'Num', 'Text'), col('name', 'Name', 'Text'), col('qty', 'Qty', 'Number'), col('rate', 'Sales Price', 'Money'), col('amount', 'Amount', 'Money')];
      const sales = books.txns.filter((s) => !s.txn.is_void && ['Invoice', 'SalesReceipt', 'CreditMemo', 'StatementCharge'].includes(s.txn.txn_type) && inRange(s.txn.date, r.from, r.to));
      const b = new Rows();
      const perItem = new Map<string, Array<{ s: StoredTxn; qty: number; amount: number; rate: number }>>();
      for (const s of sales) {
        const sign = s.txn.txn_type === 'CreditMemo' ? -1 : 1;
        for (const l of s.txn.lines) {
          if (l.kind !== 'Item' || !l.item_id) continue;
          if (!s.postings.some((p) => p.line_id === l.id && pred(s, p))) continue;
          const qty = Number(l.quantity ?? '1') || 0;
          const list = perItem.get(l.item_id) ?? [];
          list.push({ s, qty: sign * qty, amount: sign * l.amount, rate: Math.round(Number(l.rate ?? '0') * 100) });
          perItem.set(l.item_id, list);
        }
      }
      const total = [...perItem.values()].flat().reduce((t, x) => t + x.amount, 0);
      let grand = 0;
      for (const item of books.items) {
        const list = perItem.get(item.id);
        if (!list) continue;
        const qty = list.reduce((t, x) => t + x.qty, 0);
        const amount = list.reduce((t, x) => t + x.amount, 0);
        grand += amount;
        if (req.report === 'SalesByItemSummary') {
          b.push('Data', 0, [text(item.full_name), num(String(qty)), money(amount), pct(amount, total), money(qty ? Math.round(amount / qty) : 0)], {
            kind: 'Report',
            request: { ...req, report: 'SalesByItemDetail', title: null, filters: { ...req.filters, items: [item.id], exact: true } },
          });
        } else {
          b.push('Header', 0, [text(item.full_name), empty(), empty(), empty(), empty(), empty(), empty()]);
          for (const x of list) b.push('Data', 1, [text(TXN_LABEL[x.s.txn.txn_type]), date(x.s.txn.date), text(x.s.txn.number), text(entityName(x.s.txn.entity_id)), num(String(x.qty)), money(x.rate), money(x.amount)], { kind: 'Txn', txn_id: x.s.txn.id });
          b.push('Subtotal', 0, [text(`Total ${item.full_name}`), empty(), empty(), empty(), num(String(qty)), empty(), money(amount)]);
        }
      }
      b.push('Total', 0, req.report === 'SalesByItemSummary' ? [text('TOTAL'), empty(), money(grand), pct(grand, total), empty()] : [text('TOTAL'), empty(), empty(), empty(), empty(), empty(), money(grand)]);
      return finish(books, req, NAME[req.report], dateLine(r.from, r.to), columns, b.rows);
    }

    case 'SalesTaxLiability': {
      const b = new Rows();
      const columns = [col('label', '', 'Text'), col('sales', 'Total Sales', 'Money'), col('taxable', 'Taxable Sales', 'Money'), col('tax', 'Tax Collected', 'Money'), col('owed', 'Sales Tax Payable', 'Money')];
      const stp = books.accounts.find((a) => a.role === 'SalesTaxPayable');
      const sales = books.txns.filter((s) => !s.txn.is_void && ['Invoice', 'SalesReceipt', 'CreditMemo'].includes(s.txn.txn_type) && inRange(s.txn.date, r.from, r.to));
      for (const agency of books.entities.filter((e) => books.items.some((i) => i.item_type === 'SalesTax' && i.tax_agency_id === e.id))) {
        const items = books.items.filter((i) => i.item_type === 'SalesTax' && i.tax_agency_id === agency.id);
        b.push('Header', 0, [text(agency.full_name), empty(), empty(), empty(), empty()]);
        let tot = [0, 0, 0, 0];
        for (const item of items) {
          const mine = sales.filter((s) => s.txn.sales_tax_item_id === item.id);
          const total = mine.reduce((t, s) => t + s.total, 0);
          const tax = mine.reduce((t, s) => t + (stp ? s.postings.filter((p) => p.account_id === stp.id).reduce((u, p) => u - p.amount, 0) : 0), 0);
          const taxable = mine.reduce((t, s) => t + s.txn.lines.filter((l) => l.sales_tax_code_id).reduce((u, l) => u + l.amount, 0), 0);
          const owed = stp ? [...sums(books, null, r.to, (s, p) => p.account_id === stp.id && pred(s, p))].reduce((t, [, v]) => t - v, 0) : 0;
          b.push('Data', 1, [text(item.full_name), money(total - tax), money(taxable), money(tax), money(owed)]);
          tot = [tot[0]! + total - tax, tot[1]! + taxable, tot[2]! + tax, owed];
        }
        b.push('Subtotal', 0, [text(`Total ${agency.full_name}`), ...tot.map((v) => money(v))]);
      }
      return finish(books, req, 'Sales Tax Liability', dateLine(r.from, r.to), columns, b.rows, ['Sales columns are as invoiced.']);
    }

    case 'JobProfitabilityDetail': {
      const job = req.filters.names[0];
      if (!job) throw new ApiError(400, 'invalid', 'Choose a customer or job', { field: 'names' });
      const fam = descendants(books.entities, [job], req.filters.exact);
      const b = new Rows();
      const columns = [col('label', '', 'Text'), col('rev', 'Act. Revenue', 'Money'), col('cost', 'Act. Cost', 'Money'), col('diff', '($) Diff.', 'Money')];
      const by = new Map<string, [number, number]>();
      for (const s of books.txns) {
        if (s.txn.is_void || !inRange(s.txn.date, r.from, r.to)) continue;
        for (const p of s.postings) {
          const ent = p.entity_id ?? s.txn.entity_id;
          if (!ent || !fam.has(ent) || !pred(s, p)) continue;
          const a = accountById.get(p.account_id);
          if (!a) continue;
          const key = p.item_id ? `item:${p.item_id}` : `acct:${a.id}`;
          const cur = by.get(key) ?? [0, 0];
          if (a.account_type === 'Income' || a.account_type === 'OtherIncome') cur[0] -= p.amount;
          else if (a.account_type === 'Expense' || a.account_type === 'CostOfGoodsSold' || a.account_type === 'OtherExpense') cur[1] += p.amount;
          else continue;
          by.set(key, cur);
        }
      }
      let rev = 0;
      let cost = 0;
      for (const [key, [rv, c]] of by) {
        const label = key.startsWith('item:') ? (books.items.find((i) => i.id === key.slice(5))?.full_name ?? key) : (accountName(key.slice(5)) ?? key);
        b.push('Data', 0, [text(label), money(rv), money(c), money(rv - c)]);
        rev += rv;
        cost += c;
      }
      b.push('Total', 0, [text('TOTAL'), money(rev), money(cost), money(rev - cost)]);
      return finish(books, req, `Job Profitability Detail: ${entityName(job) ?? ''}`, dateLine(r.from, r.to), columns, b.rows);
    }

    case 'InventoryValuationSummary': {
      const b = new Rows();
      const columns = [col('label', '', 'Text'), col('qty', 'On Hand', 'Number'), col('cost', 'Avg Cost', 'Money'), col('value', 'Asset Value', 'Money')];
      for (const i of books.items.filter((x) => x.item_type === 'Inventory' || x.item_type === 'InventoryAssembly')) {
        b.push('Data', 0, [text(i.full_name), num(i.quantity_on_hand), money(Math.round(Number(i.average_cost) * 100)), money(Math.round(Number(i.quantity_on_hand) * Number(i.average_cost) * 100))]);
      }
      if (!b.rows.length) b.push('Data', 0, [text('No inventory items'), empty(), empty(), empty()]);
      return finish(books, req, NAME[req.report], dateLine(null, r.to ?? r.today, true), columns, b.rows);
    }

    case 'AuditTrail': {
      const b = new Rows();
      const columns = [col('at', 'Entered/Last Modified', 'Text'), col('user', 'Last modified by', 'Text'), col('state', 'State', 'Text'), col('type', 'Type', 'Text'), col('date', 'Date', 'Date'), col('num', 'Num', 'Text'), col('name', 'Name', 'Text'), col('amount', 'Amount', 'Money')];
      for (const a of [...books.audit].reverse()) {
        const t = (a.after ?? a.before) as Txn | null;
        if (!t) continue;
        const day = a.at.slice(0, 10);
        if (!inRange(day, r.from, r.to)) continue;
        b.push('Data', 0, [text(`${formatDate(day)} ${a.at.slice(11, 19)}`), text(a.user), text(a.action), text(TXN_LABEL[t.txn_type]), date(t.date), text(t.number), text(entityName(t.entity_id)), money(t.amount)], { kind: 'Txn', txn_id: t.id });
      }
      if (!b.rows.length) b.push('Data', 0, [text('No changes recorded in this period'), empty(), empty(), empty(), empty(), empty(), empty(), empty()]);
      return finish(books, req, 'Audit Trail', dateLine(r.from, r.to), columns, b.rows);
    }

    // ---- Lists ---------------------------------------------------------------------
    case 'AccountListing': {
      const b = new Rows();
      const totals = sums(books, null, null, () => true);
      for (const a of books.accounts.filter((x) => x.is_active)) b.push('Data', 0, [text(a.full_name), text(a.account_type), isBalanceSheet(a.account_type) ? money(natural(a.id, totals.get(a.id) ?? 0)) : empty(), text(a.description), text(a.number)]);
      return finish(books, req, 'Account Listing', dateLine(null, r.today, true), [col('name', 'Account', 'Text'), col('type', 'Type', 'Text'), col('balance', 'Balance Total', 'Money'), col('desc', 'Description', 'Text'), col('num', 'Accnt. #', 'Text')], b.rows);
    }
    case 'CustomerContactList':
    case 'VendorContactList': {
      const b = new Rows();
      const kind = req.report === 'CustomerContactList' ? 'Customer' : 'Vendor';
      for (const e of books.entities.filter((x) => x.kind === kind && x.is_active)) {
        const addr = [...(e.bill_address.lines ?? []), [e.bill_address.city, e.bill_address.state, e.bill_address.postal_code].filter(Boolean).join(' ')].filter(Boolean).join(', ');
        const contact = e.contact ?? ([e.first_name, e.last_name].filter(Boolean).join(' ') || null);
        b.push('Data', 0, [text(e.full_name), text(addr || null), text(contact), text(e.phone), text(e.email)], { kind: 'Name', entity_id: e.id });
      }
      return finish(books, req, NAME[req.report], dateLine(null, r.today, true), [col('name', kind, 'Text'), col('addr', 'Bill to', 'Text'), col('contact', 'Contact', 'Text'), col('phone', 'Phone', 'Text'), col('email', 'Email', 'Text')], b.rows);
    }
    case 'ItemListing': {
      const b = new Rows();
      for (const i of books.items.filter((x) => x.is_active)) b.push('Data', 0, [text(i.full_name), text(i.sales_description ?? i.purchase_description), text(i.item_type), i.purchase_cost ? money(Math.round(Number(i.purchase_cost) * 100)) : empty(), i.sales_price && !i.is_percent ? money(Math.round(Number(i.sales_price) * 100)) : empty(), text(accountName(i.income_account_id ?? i.expense_account_id))]);
      return finish(books, req, 'Item Listing', dateLine(null, r.today, true), [col('name', 'Item', 'Text'), col('desc', 'Description', 'Text'), col('type', 'Type', 'Text'), col('cost', 'Cost', 'Money'), col('price', 'Price', 'Money'), col('account', 'Account', 'Text')], b.rows);
    }
  }
}

/** Order detail rows as the request asks; Default keeps date order. */
function sortDetail<T extends { s: StoredTxn; amt?: number; name: string | null }>(rows: T[], req: ReportRequest) {
  const key = (d: T): string | number => {
    switch (req.sort_by) {
      case 'Date':
        return d.s.txn.date;
      case 'Number':
        return d.s.txn.number ?? '';
      case 'Name':
        return d.name ?? '';
      case 'Memo':
        return d.s.txn.memo ?? '';
      case 'Amount':
        return d.amt ?? d.s.total;
      default:
        return '';
    }
  };
  if (req.sort_by !== 'Default') rows.sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : 0));
  if (req.sort_descending) rows.reverse();
}

