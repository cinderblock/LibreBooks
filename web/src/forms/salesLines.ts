// Sales-side form lines (Create Invoices, Enter Sales Receipts, Create Credit
// Memos, Create Estimates): item lines with quantity, rate, amount and tax code,
// QBD's auto-fill rules between the columns, subtotal and percent-discount
// items, the sales tax arithmetic, and conversion to/from API lines. Pure,
// unit-tested; the component only wires these to inputs.

import { blankLine, newId } from '~/api/blank';
import type { Item, SalesTaxCode, TxnLine } from '~/api/types';
import { normalizeDecimalText, parseDecimal } from '~/lib/decimal';
import { backSolveRate, centsToPlain, extend, percentOf, sumCents, type Cents } from '~/lib/money';

export interface SaleRow {
  key: string;
  lineId: string | null;
  item_id: string | null;
  description: string;
  /** As typed (decimal text); blank means "1" for the arithmetic, as in QBD. */
  quantity: string;
  /** A price, or a percent for percent items (`rate_is_percent`). */
  rate: string;
  rate_is_percent: boolean;
  amount: Cents | null;
  sales_tax_code_id: string | null;
  class_id: string | null;
}

export const blankSaleRow = (): SaleRow => ({
  key: newId(),
  lineId: null,
  item_id: null,
  description: '',
  quantity: '',
  rate: '',
  rate_is_percent: false,
  amount: null,
  sales_tax_code_id: null,
  class_id: null,
});

export const isBlankSale = (r: SaleRow) => !r.item_id && !r.amount && !r.description.trim() && !r.quantity.trim() && !r.rate.trim();

/** quantity × rate, treating a blank quantity as 1. Null if the rate isn't a number. */
function lineAmount(quantity: string, rate: string): Cents | null {
  return extend(quantity.trim() === '' ? '1' : quantity, rate);
}

/** Items whose amount comes from the lines above, not from quantity × rate. */
export function isSubtotalItem(item: Item | undefined): boolean {
  return item?.item_type === 'Subtotal';
}

/**
 * Choosing an item fills its sales description, price (or percent) and tax
 * code (the item's; a non-taxable customer code wins later, at posting), then
 * the amount.
 */
export function chooseSaleItem(row: SaleRow, item: Item | undefined, defaultTaxCode: string | null): SaleRow {
  if (!item) return { ...row, item_id: null };
  const rate = item.sales_price ?? '';
  const percent = item.is_percent;
  return {
    ...row,
    item_id: item.id,
    description: item.sales_description ?? row.description,
    rate,
    rate_is_percent: percent,
    sales_tax_code_id: item.item_type === 'Subtotal' || item.item_type === 'Discount' ? row.sales_tax_code_id : (item.sales_tax_code_id ?? defaultTaxCode),
    amount: percent || isSubtotalItem(item) ? row.amount : rate ? lineAmount(row.quantity, rate) : row.amount,
  };
}

export function setSaleQuantity(row: SaleRow, quantity: string): SaleRow {
  const amount = !row.rate_is_percent && row.rate.trim() ? lineAmount(quantity, row.rate) : row.amount;
  return { ...row, quantity, amount: amount ?? row.amount };
}

export function setSaleRate(row: SaleRow, rate: string): SaleRow {
  if (row.rate_is_percent) return { ...row, rate };
  const amount = lineAmount(row.quantity, rate);
  return { ...row, rate, amount: amount ?? row.amount };
}

/** Editing the amount back-solves the rate (QBD), to five places. */
export function setSaleAmount(row: SaleRow, amount: Cents | null): SaleRow {
  if (amount === null || row.rate_is_percent) return { ...row, amount };
  const q = row.quantity.trim();
  const rate = q === '' ? centsToPlain(amount) : backSolveRate(amount, q);
  return { ...row, amount, rate: rate ?? row.rate };
}

/**
 * Derived amounts, QBD-style: a Subtotal item sums the lines above it (back to
 * the previous subtotal); a percent item (a discount, a percent charge) applies
 * its percent to the line just above it, which for a subtotal means the whole
 * group. Discounts come out negative. Run after any edit.
 */
export function recomputeSales(rows: readonly SaleRow[], items: ReadonlyMap<string, Item>): SaleRow[] {
  const out: SaleRow[] = [];
  let running = 0;
  for (const r of rows) {
    const item = r.item_id ? items.get(r.item_id) : undefined;
    let row = r;
    if (isSubtotalItem(item)) {
      row = { ...r, amount: running, description: r.description || 'Subtotal' };
      running = 0;
    } else if (r.rate_is_percent && item) {
      const above = out[out.length - 1];
      const base = above?.amount ?? 0;
      const pct = r.rate.trim() ? (percentOf(base, r.rate) ?? 0) : 0;
      const signed = item.item_type === 'Discount' ? -Math.abs(pct) : pct;
      row = { ...r, amount: signed };
      running += signed;
    } else {
      running += r.amount ?? 0;
    }
    out.push(row);
  }
  return out;
}

/** The lines' sum before tax (subtotal items are excluded: they repeat lines). */
export function salesSubtotal(rows: readonly SaleRow[], items: ReadonlyMap<string, Item>): Cents {
  return sumCents(rows.filter((r) => !isSubtotalItem(r.item_id ? items.get(r.item_id) : undefined)).map((r) => r.amount ?? 0));
}

/** The sum of taxable lines (those whose tax code is taxable). */
export function taxableAmount(rows: readonly SaleRow[], items: ReadonlyMap<string, Item>, codes: ReadonlyMap<string, SalesTaxCode>): Cents {
  return sumCents(
    rows
      .filter((r) => !isSubtotalItem(r.item_id ? items.get(r.item_id) : undefined) && r.sales_tax_code_id && codes.get(r.sales_tax_code_id)?.is_taxable)
      .map((r) => r.amount ?? 0),
  );
}

/** A sales tax item's rate, or a group's summed rates, as a percent string. */
export function taxRate(taxItem: Item | undefined, items: ReadonlyMap<string, Item>): string | null {
  if (!taxItem) return null;
  if (taxItem.item_type === 'SalesTaxGroup') {
    let total = 0n;
    let scale = 0;
    for (const c of taxItem.components) {
      const d = parseDecimal(items.get(c.item_id)?.tax_rate ?? '0');
      if (!d) continue;
      if (d.scale > scale) {
        total *= 10n ** BigInt(d.scale - scale);
        scale = d.scale;
      }
      total += d.int * 10n ** BigInt(scale - d.scale);
    }
    const s = total.toString().padStart(scale + 1, '0');
    return scale ? `${s.slice(0, -scale)}.${s.slice(-scale)}` : s;
  }
  return taxItem.tax_rate;
}

/** The tax the client expects (the server's preview is the truth; this keeps totals live). */
export function estimatedTax(taxable: Cents, taxItem: Item | undefined, items: ReadonlyMap<string, Item>, customerTaxable: boolean): Cents {
  if (!customerTaxable || !taxItem) return 0;
  const rate = taxRate(taxItem, items);
  return rate ? (percentOf(taxable, rate) ?? 0) : 0;
}

/** Rows → API lines (blank rows dropped; lines keep their ids when editing). */
export function toSaleLines(rows: readonly SaleRow[]): TxnLine[] {
  return rows
    .filter((r) => !isBlankSale(r))
    .map((r) => ({
      ...blankLine('Item'),
      id: r.lineId ?? r.key,
      item_id: r.item_id,
      description: r.description.trim() || null,
      quantity: normalizeDecimalText(r.quantity),
      rate: normalizeDecimalText(r.rate),
      rate_is_percent: r.rate_is_percent,
      amount: r.amount ?? 0,
      sales_tax_code_id: r.sales_tax_code_id,
      class_id: r.class_id,
    }));
}

export function fromSaleLines(lines: readonly TxnLine[]): SaleRow[] {
  return lines
    .filter((l) => l.kind === 'Item' || l.kind === 'Text')
    .map((l) => ({
      key: l.id,
      lineId: l.id,
      item_id: l.item_id,
      description: l.description ?? '',
      quantity: l.quantity ?? '',
      rate: l.rate ?? '',
      rate_is_percent: l.rate_is_percent,
      amount: l.amount,
      sales_tax_code_id: l.sales_tax_code_id,
      class_id: l.class_id,
    }));
}

/** Map an API line index (as sent) back to its row. */
export function locateSaleLine(rows: readonly SaleRow[], lineIndex: number): number | null {
  const sent = rows.flatMap((r, i) => (isBlankSale(r) ? [] : [i]));
  return sent[lineIndex] ?? null;
}
