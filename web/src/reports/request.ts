// `ReportRequest` helpers: the default request for a report, the request as URL
// query params (a report window's address) and back, and the part of a request
// that makes a window distinct (its subject). Pure, unit-tested.
//
// Dates, basis, columns and comparisons change an open report in place (QBD's
// "Modify Report"), so they are in the URL but not in the window key; the
// filters that name a subject (an account's QuickReport, a customer's
// transactions, a "<parent> - Other" exact drill) are both.

import type { CatalogEntry, ClearedFilter, ColumnLayout, DateMacro, ReportFilters, ReportId, ReportRequest, SortBy } from '~/api/types';
import { macroRange, type IsoDate } from '~/lib/dates';

export const EMPTY_FILTERS: ReportFilters = {
  accounts: [],
  account_types: [],
  names: [],
  classes: [],
  items: [],
  txn_types: [],
  cleared: null,
  amount_min: null,
  amount_max: null,
  memo: null,
  exact: false,
};

/** A request with nothing chosen: the server applies the report's defaults. */
export function defaultRequest(report: ReportId): ReportRequest {
  return {
    report,
    date_macro: null,
    from: null,
    to: null,
    today: null,
    basis: null,
    columns: null,
    percent_of_income: false,
    percent_of_column: false,
    previous_period: false,
    previous_year: false,
    dollar_change: false,
    percent_change: false,
    filters: { ...EMPTY_FILTERS },
    sort_by: 'Default',
    sort_descending: false,
    show_zero_rows: false,
    title: null,
  };
}

export const COLUMN_LAYOUT_LABEL: Record<ColumnLayout, string> = {
  TotalOnly: 'Total only',
  Week: 'Week',
  Month: 'Month',
  Quarter: 'Quarter',
  Year: 'Year',
  CustomerJob: 'Customer:Job',
  Class: 'Class',
  Vendor: 'Vendor',
  Item: 'Item',
};

export const SORT_LABEL: Record<SortBy, string> = {
  Default: 'Default',
  Date: 'Date',
  Number: 'Num',
  Name: 'Name',
  Memo: 'Memo',
  Amount: 'Amount',
};

/** Query keys. Short, because they're in every report URL. */
const P = {
  dates: 'dates',
  from: 'from',
  to: 'to',
  today: 'today',
  basis: 'basis',
  columns: 'cols',
  percent_of_income: 'pct_income',
  percent_of_column: 'pct_col',
  previous_period: 'prev',
  previous_year: 'prev_year',
  dollar_change: 'dollar',
  percent_change: 'pct',
  sort: 'sort',
  desc: 'desc',
  zero: 'zero',
  title: 'title',
  accounts: 'acct',
  account_types: 'acct_type',
  names: 'name',
  classes: 'class',
  items: 'item',
  txn_types: 'txn',
  cleared: 'cleared',
  amount_min: 'amin',
  amount_max: 'amax',
  memo: 'memo',
  exact: 'exact',
} as const;

/** Keys that name a report's subject (part of the window key). */
export const SUBJECT_KEYS: readonly string[] = [P.accounts, P.account_types, P.names, P.classes, P.items, P.exact, P.title];

const list = (v: string[]) => (v.length ? v.join(',') : null);
const flag = (v: boolean) => (v ? '1' : null);

/** The request as query params; defaults are left out so URLs stay short. */
export function requestToParams(r: ReportRequest): Record<string, string | null> {
  const f = r.filters;
  return {
    [P.dates]: r.date_macro,
    [P.from]: r.date_macro ? null : r.from,
    [P.to]: r.date_macro ? null : r.to,
    [P.today]: r.today,
    [P.basis]: r.basis,
    [P.columns]: r.columns,
    [P.percent_of_income]: flag(r.percent_of_income),
    [P.percent_of_column]: flag(r.percent_of_column),
    [P.previous_period]: flag(r.previous_period),
    [P.previous_year]: flag(r.previous_year),
    [P.dollar_change]: flag(r.dollar_change),
    [P.percent_change]: flag(r.percent_change),
    [P.sort]: r.sort_by === 'Default' ? null : r.sort_by,
    [P.desc]: flag(r.sort_descending),
    [P.zero]: flag(r.show_zero_rows),
    [P.title]: r.title,
    [P.accounts]: list(f.accounts),
    [P.account_types]: list(f.account_types),
    [P.names]: list(f.names),
    [P.classes]: list(f.classes),
    [P.items]: list(f.items),
    [P.txn_types]: list(f.txn_types),
    [P.cleared]: f.cleared,
    [P.amount_min]: f.amount_min === null ? null : String(f.amount_min),
    [P.amount_max]: f.amount_max === null ? null : String(f.amount_max),
    [P.memo]: f.memo,
    [P.exact]: flag(f.exact),
  };
}

const split = (v: string | undefined) => (v ? v.split(',').filter(Boolean) : []);
const cents = (v: string | undefined) => (v !== undefined && /^-?\d+$/.test(v) ? Number(v) : null);

/** A window's params (`report` plus the keys above) back to a request. */
export function paramsToRequest(p: Record<string, string>): ReportRequest {
  const r = defaultRequest((p.report ?? 'ProfitAndLoss') as ReportId);
  const basis = p[P.basis];
  return {
    ...r,
    date_macro: (p[P.dates] as DateMacro | undefined) ?? null,
    from: p[P.from] ?? null,
    to: p[P.to] ?? null,
    today: p[P.today] ?? null,
    basis: basis === 'Cash' || basis === 'Accrual' ? basis : null,
    columns: (p[P.columns] as ColumnLayout | undefined) ?? null,
    percent_of_income: p[P.percent_of_income] === '1',
    percent_of_column: p[P.percent_of_column] === '1',
    previous_period: p[P.previous_period] === '1',
    previous_year: p[P.previous_year] === '1',
    dollar_change: p[P.dollar_change] === '1',
    percent_change: p[P.percent_change] === '1',
    sort_by: (p[P.sort] as SortBy | undefined) ?? 'Default',
    sort_descending: p[P.desc] === '1',
    show_zero_rows: p[P.zero] === '1',
    title: p[P.title] ?? null,
    filters: {
      accounts: split(p[P.accounts]),
      account_types: split(p[P.account_types]) as ReportFilters['account_types'],
      names: split(p[P.names]),
      classes: split(p[P.classes]),
      items: split(p[P.items]),
      txn_types: split(p[P.txn_types]) as ReportFilters['txn_types'],
      cleared: (p[P.cleared] as ClearedFilter | undefined) ?? null,
      amount_min: cents(p[P.amount_min]),
      amount_max: cents(p[P.amount_max]),
      memo: p[P.memo] ?? null,
      exact: p[P.exact] === '1',
    },
  };
}

/**
 * Fill in what the catalog says applies when the request left it open: the
 * default dates (so the Dates picker shows "This Month-to-date", not "Custom").
 */
export function withDefaults(r: ReportRequest, entry: CatalogEntry | undefined): ReportRequest {
  if (!entry) return r;
  const out = { ...r };
  if (entry.params.dates !== 'None' && !out.date_macro && !out.from && !out.to) out.date_macro = entry.params.default_dates;
  if (entry.params.columns.length && !out.columns) out.columns = entry.params.default_columns;
  return out;
}

/** The dates a request means (for the From/To fields), given today and the fiscal year start. */
export function requestRange(r: ReportRequest, today: IsoDate, fiscalStartMonth: number): { from: IsoDate | null; to: IsoDate | null } {
  if (r.date_macro) return macroRange(r.date_macro, r.today ?? today, fiscalStartMonth);
  return { from: r.from, to: r.to };
}

/** How many filters are set, for the Filters button's badge. */
export function filterCount(f: ReportFilters): number {
  return (
    [f.accounts, f.account_types, f.names, f.classes, f.items, f.txn_types].filter((l) => l.length > 0).length +
    (f.cleared ? 1 : 0) +
    (f.amount_min !== null || f.amount_max !== null ? 1 : 0) +
    (f.memo ? 1 : 0)
  );
}
