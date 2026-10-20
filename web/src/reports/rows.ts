// Outline logic for report rows (collapse/expand), cell formatting and CSV
// export, over lb-reports' generated `Report`. Pure.
//
// A `Header` row opens a group that the next `Subtotal` (or `Total`) row at the
// same depth closes; collapsing the header hides everything between and shows
// the subtotal's figures on the header's own line (QBD's "Collapse"). Rows have
// no ids of their own, so the outline is keyed by row index.

import { formatDate, formatDateShort } from '~/lib/dates';
import { formatDecimal, parseDecimal } from '~/lib/decimal';
import { centsToPlain, formatMoney } from '~/lib/money';
import type { Cell, Column, Report, Row, Value } from '~/api/types';

export interface VisibleRow {
  index: number;
  row: Row;
  /** What to show: a collapsed header carries its subtotal's figures. */
  cells: Cell[];
  collapsible: boolean;
  collapsed: boolean;
}

const closes = (r: Row) => r.kind === 'Subtotal' || r.kind === 'Total';

/** Index of the row closing the group a header at `i` opens, or -1 if nothing does. */
export function groupEnd(rows: readonly Row[], i: number): number {
  const d = rows[i]!.depth;
  for (let j = i + 1; j < rows.length; j++) {
    const r = rows[j]!;
    if (r.depth < d) return -1;
    if (r.depth === d) return closes(r) ? j : -1;
  }
  return -1;
}

export function visibleRows(rows: readonly Row[], collapsed: ReadonlySet<number>): VisibleRow[] {
  const out: VisibleRow[] = [];
  let i = 0;
  while (i < rows.length) {
    const r = rows[i]!;
    const end = r.kind === 'Header' ? groupEnd(rows, i) : -1;
    if (end < 0) {
      out.push({ index: i, row: r, cells: r.cells, collapsible: false, collapsed: false });
      i++;
      continue;
    }
    if (!collapsed.has(i)) {
      out.push({ index: i, row: r, cells: r.cells, collapsible: true, collapsed: false });
      i++;
      continue;
    }
    const total = rows[end]!;
    out.push({ index: i, row: r, cells: [r.cells[0] ?? { value: { type: 'Empty' } }, ...total.cells.slice(1)], collapsible: true, collapsed: true });
    i = end + 1;
  }
  return out;
}

/** Every collapsible header's index (the "Collapse" button). */
export function allSections(rows: readonly Row[]): Set<number> {
  const out = new Set<number>();
  rows.forEach((r, i) => {
    if (r.kind === 'Header' && groupEnd(rows, i) >= 0) out.add(i);
  });
  return out;
}

/** `"12.5"` → `12.5%`; percents come as decimal strings, one place. */
export function formatPercent(text: string): string {
  const d = parseDecimal(text);
  return d ? `${formatDecimal(d, 0, true)}%` : text;
}

export function formatNumber(text: string): string {
  const d = parseDecimal(text);
  return d ? formatDecimal(d, 0, true) : text;
}

/** A cell's text as the viewer shows it. */
export function valueText(v: Value, opts: { shortDate?: boolean; parens?: boolean } = {}): string {
  switch (v.type) {
    case 'Empty':
      return '';
    case 'Text':
      return v.value;
    case 'Money':
      return formatMoney(v.value, { parens: opts.parens });
    case 'Date':
      return opts.shortDate ? formatDateShort(v.value) : formatDate(v.value);
    case 'Number':
      return formatNumber(v.value);
    case 'Percent':
      return formatPercent(v.value);
  }
}

export function isNegative(v: Value): boolean {
  if (v.type === 'Money') return v.value < 0;
  if (v.type === 'Percent' || v.type === 'Number') return v.value.trim().startsWith('-');
  return false;
}

function csvCell(v: string): string {
  return /[",\n\r]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
}

/** One cell as CSV text: plain amounts, ISO-free US dates, labels indented by depth. */
function csvValue(v: Value, first: boolean, depth: number): string {
  switch (v.type) {
    case 'Empty':
      return '';
    case 'Money':
      return centsToPlain(v.value);
    case 'Date':
      return formatDate(v.value);
    case 'Number':
    case 'Percent':
      return v.value;
    case 'Text':
      return csvCell(first ? `${'  '.repeat(depth)}${v.value}` : v.value);
  }
}

/** The report as CSV: a title block, then the columns (with their groups) and rows. */
export function reportToCsv(r: Report): string {
  const lines: string[] = [];
  lines.push(csvCell(r.company_name), csvCell(r.title), csvCell(r.subtitle));
  if (r.basis) lines.push(csvCell(`${r.basis} Basis`));
  lines.push('');
  if (r.columns.some((c) => c.group)) lines.push(r.columns.map((c) => csvCell(c.group ?? '')).join(','));
  lines.push(r.columns.map((c: Column) => csvCell(c.label)).join(','));
  for (const row of r.rows) {
    if (row.kind === 'Spacer') {
      lines.push('');
      continue;
    }
    lines.push(row.cells.map((cell, i) => csvValue(cell.value, i === 0, row.depth)).join(','));
  }
  return `${lines.join('\r\n')}\r\n`;
}

/** Consecutive columns sharing a `group`, for a spanning header row. */
export function columnGroups(columns: readonly Column[]): Array<{ label: string; start: number; span: number }> {
  const out: Array<{ label: string; start: number; span: number }> = [];
  columns.forEach((c, i) => {
    const label = c.group ?? '';
    const last = out[out.length - 1];
    if (last && last.label === label && last.start + last.span === i) last.span++;
    else out.push({ label, start: i, span: 1 });
  });
  return out;
}
