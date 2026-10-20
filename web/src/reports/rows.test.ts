import { describe, expect, test } from 'bun:test';
import type { Cell, Report, Row, RowKind } from '~/api/types';
import { allSections, columnGroups, formatPercent, groupEnd, reportToCsv, valueText, visibleRows } from './rows';

const text = (s: string): Cell => ({ value: { type: 'Text', value: s } });
const money = (c: number): Cell => ({ value: { type: 'Money', value: c } });
const empty = (): Cell => ({ value: { type: 'Empty' } });
const row = (kind: RowKind, depth: number, label: string, amount?: number): Row => ({
  kind,
  depth,
  cells: [text(label), amount === undefined ? empty() : money(amount)],
});

// A small P&L outline: Income (two accounts) closed by Total Income, then Net Income.
const rows: Row[] = [
  row('Header', 0, 'Income'),
  row('Data', 1, 'Design Services', 10_000),
  row('Header', 1, 'Sales'),
  row('Data', 2, 'Sales:Custom Furniture', 20_000),
  row('Subtotal', 1, 'Total Sales', 20_000),
  row('Subtotal', 0, 'Total Income', 30_000),
  row('Total', 0, 'Net Income', 30_000),
];

describe('report outline', () => {
  test('a header is closed by the subtotal at its depth', () => {
    expect(groupEnd(rows, 0)).toBe(5);
    expect(groupEnd(rows, 2)).toBe(4);
    expect(groupEnd([row('Header', 0, 'Lonely'), row('Data', 1, 'x', 1)], 0)).toBe(-1);
  });
  test('collapsing shows the subtotal on the header line and hides the group', () => {
    const v = visibleRows(rows, new Set([0]));
    expect(v.map((r) => r.row.cells[0]!.value)).toEqual([text('Income').value, text('Net Income').value]);
    expect(v[0]!.collapsed).toBe(true);
    expect(v[0]!.cells[1]).toEqual(money(30_000));
  });
  test('nested collapse only hides its own group', () => {
    const v = visibleRows(rows, new Set([2]));
    expect(v).toHaveLength(5);
    expect(v[2]!.cells[1]).toEqual(money(20_000));
    expect(v[2]!.row.kind).toBe('Header');
  });
  test('allSections lists only headers that have a closing subtotal', () => {
    expect([...allSections(rows)]).toEqual([0, 2]);
  });
});

describe('cell text', () => {
  test('values format by type', () => {
    expect(valueText({ type: 'Money', value: -123_456 })).toBe('-1,234.56');
    expect(valueText({ type: 'Money', value: -5 }, { parens: true })).toBe('(0.05)');
    expect(valueText({ type: 'Date', value: '2026-10-19' })).toBe('10/19/2026');
    expect(valueText({ type: 'Date', value: '2026-10-19' }, { shortDate: true })).toBe('10/19/26');
    expect(valueText({ type: 'Percent', value: '12.5' })).toBe('12.5%');
    expect(valueText({ type: 'Number', value: '1234.5' })).toBe('1,234.5');
    expect(valueText({ type: 'Empty' })).toBe('');
    expect(formatPercent('-100')).toBe('-100%');
  });
  test('column groups span consecutive columns', () => {
    const cols = [
      { id: 'label', label: '', kind: 'Text', align: 'Left' },
      { id: 'c0', label: 'Actual', kind: 'Money', align: 'Right', group: 'Oct 2026' },
      { id: 'c1', label: '% of Income', kind: 'Percent', align: 'Right', group: 'Oct 2026' },
      { id: 'c2', label: 'Actual', kind: 'Money', align: 'Right', group: 'Sep 2026' },
    ] as const;
    expect(columnGroups(cols)).toEqual([
      { label: '', start: 0, span: 1 },
      { label: 'Oct 2026', start: 1, span: 2 },
      { label: 'Sep 2026', start: 3, span: 1 },
    ]);
  });
});

describe('CSV export', () => {
  test('writes a title block, headers and indented labels; quotes commas', () => {
    const r: Report = {
      id: 'ProfitAndLoss',
      title: 'Profit & Loss',
      company_name: 'Juniper Hill Woodworks',
      subtitle: 'October 1 through October 19, 2026',
      basis: 'Accrual',
      generated_at: '2026-10-19T00:00:00Z',
      columns: [
        { id: 'label', label: '', kind: 'Text', align: 'Left' },
        { id: 'amount', label: 'Oct 1 - 19, 26', kind: 'Money', align: 'Right' },
      ],
      rows: [row('Header', 0, 'Income'), row('Data', 1, 'Sales, net', 1_050), row('Spacer', 0, ''), row('Total', 0, 'Net Income', 1_050)],
      notes: [],
    };
    const lines = reportToCsv(r).split('\r\n');
    expect(lines.slice(0, 4)).toEqual(['Juniper Hill Woodworks', 'Profit & Loss', '"October 1 through October 19, 2026"', 'Accrual Basis']);
    expect(lines[5]).toBe(',"Oct 1 - 19, 26"');
    expect(lines[6]).toBe('Income,');
    expect(lines[7]).toBe('"  Sales, net",10.50');
    expect(lines[8]).toBe('');
    expect(lines[9]).toBe('Net Income,10.50');
  });
});
