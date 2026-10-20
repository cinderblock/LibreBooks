import { describe, expect, test } from 'bun:test';
import { allSections, groupEnd, reportToCsv, visibleRows } from './rows';
import type { Report, ReportRow } from './types';

const row = (id: string, kind: ReportRow['kind'], depth: number, label: string, amount: number | null = null): ReportRow => ({
  id,
  kind,
  depth,
  cells: [label, amount],
});

// Income
//   Sales
//     Cabinetry      100
//     Furniture      200
//   Total Sales      300
//   Design            50
// Total Income       350
// Net Income         350
const rows: ReportRow[] = [
  row('inc', 'section', 0, 'Income'),
  row('sales', 'section', 1, 'Sales'),
  row('cab', 'data', 2, 'Cabinetry', 100),
  row('fur', 'data', 2, 'Furniture', 200),
  row('tsales', 'subtotal', 1, 'Total Sales', 300),
  row('design', 'data', 1, 'Design', 50),
  row('tinc', 'subtotal', 0, 'Total Income', 350),
  row('net', 'total', 0, 'Net Income', 350),
];

describe('report outline', () => {
  test('group ends at the matching subtotal', () => {
    expect(groupEnd(rows, 0)).toBe(6);
    expect(groupEnd(rows, 1)).toBe(4);
  });
  test('expanded shows everything', () => {
    expect(visibleRows(rows, new Set()).map((v) => v.row.id)).toEqual(rows.map((r) => r.id));
  });
  test('collapsing a section shows its subtotal on its own line', () => {
    const v = visibleRows(rows, new Set(['sales']));
    expect(v.map((x) => x.row.id)).toEqual(['inc', 'sales', 'design', 'tinc', 'net']);
    expect(v[1]!.cells).toEqual(['Sales', 300]);
    expect(v[1]!.collapsed).toBe(true);
  });
  test('collapse all keeps only the top level', () => {
    const v = visibleRows(rows, allSections(rows));
    expect(v.map((x) => x.row.id)).toEqual(['inc', 'net']);
    expect(v[0]!.cells).toEqual(['Income', 350]);
  });
});

test('CSV export', () => {
  const r: Report = {
    title: 'Profit & Loss',
    subtitle: 'October 2026',
    company_name: 'Juniper Hill, LLC',
    basis: 'Accrual',
    range: { from: '2026-10-01', to: '2026-10-31' },
    columns: [
      { id: 'label', label: '', kind: 'text' },
      { id: 'amount', label: 'Amount', kind: 'money' },
    ],
    rows: [row('a', 'section', 0, 'Income'), row('b', 'data', 1, 'Sales, "Retail"', -123_456)],
    generated_at: '2026-10-19T00:00:00Z',
    request: { report: 'profit_and_loss' },
  };
  expect(reportToCsv(r)).toBe(
    ['"Juniper Hill, LLC"', 'Profit & Loss', 'October 2026', 'Accrual Basis', '', ',Amount', 'Income,', '"  Sales, ""Retail""",-1234.56', ''].join('\r\n'),
  );
});
