// Outline logic for report rows (collapse/expand) and CSV export. Pure.

import { formatDate } from '~/lib/dates';
import { centsToPlain } from '~/lib/money';
import type { Report, ReportCell, ReportRow } from './types';

export interface VisibleRow {
  row: ReportRow;
  /** What to show: a collapsed section carries its subtotal's figures. */
  cells: ReportCell[];
  collapsible: boolean;
  collapsed: boolean;
}

/** Index of the last row in the group a section at `i` opens. */
export function groupEnd(rows: readonly ReportRow[], i: number): number {
  const d = rows[i]!.depth;
  for (let j = i + 1; j < rows.length; j++) {
    const r = rows[j]!;
    if (r.depth < d) return j - 1;
    if (r.depth === d) return r.kind === 'subtotal' ? j : j - 1;
  }
  return rows.length - 1;
}

export function visibleRows(rows: readonly ReportRow[], collapsed: ReadonlySet<string>): VisibleRow[] {
  const out: VisibleRow[] = [];
  let i = 0;
  while (i < rows.length) {
    const r = rows[i]!;
    if (r.kind !== 'section') {
      out.push({ row: r, cells: r.cells, collapsible: false, collapsed: false });
      i++;
      continue;
    }
    if (!collapsed.has(r.id)) {
      out.push({ row: r, cells: r.cells, collapsible: true, collapsed: false });
      i++;
      continue;
    }
    const end = groupEnd(rows, i);
    const closing = rows[end];
    const total = closing && closing.kind === 'subtotal' && closing.depth === r.depth ? closing : null;
    out.push({ row: r, cells: [r.cells[0] ?? null, ...(total ? total.cells.slice(1) : r.cells.slice(1))], collapsible: true, collapsed: true });
    i = end + 1;
  }
  return out;
}

/** Every section id (the "Collapse" button). */
export function allSections(rows: readonly ReportRow[]): Set<string> {
  return new Set(rows.filter((r) => r.kind === 'section').map((r) => r.id));
}

function csvCell(v: string): string {
  return /[",\n\r]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
}

/** The report as CSV: indented labels, plain amounts, a title block on top. */
export function reportToCsv(r: Report): string {
  const lines: string[] = [];
  lines.push(csvCell(r.company_name), csvCell(r.title));
  if (r.subtitle) lines.push(csvCell(r.subtitle));
  lines.push(csvCell(`${r.basis} Basis`), '');
  lines.push(r.columns.map((c) => csvCell(c.label)).join(','));
  for (const row of r.rows) {
    lines.push(
      row.cells
        .map((cell, i) => {
          const col = r.columns[i];
          if (cell === null || cell === undefined) return '';
          if (col?.kind === 'money' && typeof cell === 'number') return centsToPlain(cell);
          if (col?.kind === 'date' && typeof cell === 'string') return formatDate(cell);
          const text = String(cell);
          return csvCell(i === 0 ? `${'  '.repeat(row.depth)}${text}` : text);
        })
        .join(','),
    );
  }
  return `${lines.join('\r\n')}\r\n`;
}
