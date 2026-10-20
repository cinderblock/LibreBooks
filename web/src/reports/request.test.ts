import { describe, expect, test } from 'bun:test';
import type { CatalogEntry } from '~/api/types';
import { defaultRequest, filterCount, paramsToRequest, requestRange, requestToParams, withDefaults } from './request';

describe('report requests and URLs', () => {
  test('a default request round-trips to no params', () => {
    const r = defaultRequest('BalanceSheet');
    const params = requestToParams(r);
    expect(Object.values(params).every((v) => v === null)).toBe(true);
    expect(paramsToRequest({ report: 'BalanceSheet' })).toEqual(r);
  });

  test('every chosen option survives the round trip', () => {
    const r = {
      ...defaultRequest('ProfitAndLoss'),
      date_macro: 'LastMonth' as const,
      basis: 'Cash' as const,
      columns: 'Month' as const,
      percent_of_income: true,
      previous_year: true,
      dollar_change: true,
      sort_by: 'Amount' as const,
      sort_descending: true,
      show_zero_rows: true,
      title: 'My P&L',
      filters: {
        ...defaultRequest('ProfitAndLoss').filters,
        accounts: ['a1', 'a2'],
        names: ['n1'],
        txn_types: ['Check' as const, 'Bill' as const],
        cleared: 'Cleared' as const,
        amount_min: 1_000,
        memo: 'rent',
        exact: true,
      },
    };
    const back = paramsToRequest({ report: 'ProfitAndLoss', ...Object.fromEntries(Object.entries(requestToParams(r)).filter((e): e is [string, string] => e[1] !== null)) });
    expect(back).toEqual(r);
  });

  test('a macro wins over explicit dates in the URL', () => {
    const r = { ...defaultRequest('ProfitAndLoss'), date_macro: 'ThisMonth' as const, from: '2026-01-01', to: '2026-01-31' };
    const p = requestToParams(r);
    expect(p.dates).toBe('ThisMonth');
    expect(p.from).toBeNull();
    expect(requestRange(r, '2026-10-19', 1)).toEqual({ from: '2026-10-01', to: '2026-10-31' });
    expect(requestRange({ ...r, date_macro: null }, '2026-10-19', 1)).toEqual({ from: '2026-01-01', to: '2026-01-31' });
  });

  test('the catalog fills the default dates and columns', () => {
    const entry = {
      id: 'ProfitAndLoss',
      name: 'Profit & Loss',
      category: 'CompanyAndFinancial',
      category_label: 'Company & Financial',
      description: '',
      params: {
        dates: 'Range',
        default_dates: 'ThisMonthToDate',
        basis: true,
        columns: ['TotalOnly', 'Month'],
        default_columns: 'TotalOnly',
        comparisons: true,
        percent_of_income: true,
        percent_of_column: false,
        filters: [],
        sorts: ['Default'],
        show_zero_rows: true,
      },
    } satisfies CatalogEntry;
    const r = withDefaults(defaultRequest('ProfitAndLoss'), entry);
    expect(r.date_macro).toBe('ThisMonthToDate');
    expect(r.columns).toBe('TotalOnly');
    // Explicit dates are kept.
    expect(withDefaults({ ...defaultRequest('ProfitAndLoss'), to: '2026-01-31' }, entry).date_macro).toBeNull();
  });

  test('filterCount counts set filters', () => {
    const f = { ...defaultRequest('ProfitAndLoss').filters, names: ['x'], amount_max: 5, memo: 'a' };
    expect(filterCount(f)).toBe(3);
  });
});
