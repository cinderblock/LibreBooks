// Purchase-side form lines (Write Checks, and later Enter Bills, Credit Card
// Charges, Item Receipts): the Expenses and Items tabs as editable rows, the
// QBD auto-fill rules between their columns, and conversion to/from API lines.
// Pure functions, unit-tested; the components only wire them to inputs.

import { blankLine, newId } from '~/api/blank';
import type { Item, TxnLine } from '~/api/types';
import { normalizeDecimalText, parseDecimal } from '~/lib/decimal';
import { backSolveRate, centsToPlain, extend, sumCents, type Cents } from '~/lib/money';

export interface ExpenseRow {
  key: string;
  /** The API line id when editing a saved transaction (kept so lines keep identity). */
  lineId: string | null;
  account_id: string | null;
  amount: Cents | null;
  memo: string;
  entity_id: string | null;
  billable: boolean;
  class_id: string | null;
}

export interface ItemRow {
  key: string;
  lineId: string | null;
  item_id: string | null;
  description: string;
  /** As typed (decimal text); blank means "1" for the arithmetic, as in QBD. */
  quantity: string;
  rate: string;
  amount: Cents | null;
  entity_id: string | null;
  billable: boolean;
  class_id: string | null;
  /** The purchase-order line this line receives or bills (a receipt or bill made from a PO). */
  linked_txn_id: string | null;
  linked_line_id: string | null;
}

export const blankExpense = (): ExpenseRow => ({
  key: newId(),
  lineId: null,
  account_id: null,
  amount: null,
  memo: '',
  entity_id: null,
  billable: false,
  class_id: null,
});

export const blankItemRow = (): ItemRow => ({
  key: newId(),
  lineId: null,
  item_id: null,
  description: '',
  quantity: '',
  rate: '',
  amount: null,
  entity_id: null,
  billable: false,
  class_id: null,
  linked_txn_id: null,
  linked_line_id: null,
});

export const isBlankExpense = (r: ExpenseRow) =>
  !r.account_id && !r.amount && !r.memo.trim() && !r.entity_id && !r.class_id;

export const isBlankItem = (r: ItemRow) =>
  !r.item_id && !r.amount && !r.description.trim() && !r.quantity.trim() && !r.rate.trim() && !r.entity_id;

/** quantity × rate, treating a blank quantity as 1. Null if the rate isn't a number. */
function lineAmount(quantity: string, rate: string): Cents | null {
  return extend(quantity.trim() === '' ? '1' : quantity, rate);
}

/** Choosing an item fills its purchase description and cost, then the amount. */
export function chooseItem(row: ItemRow, item: Item | undefined): ItemRow {
  if (!item) return { ...row, item_id: null };
  const rate = item.purchase_cost ?? item.sales_price ?? '';
  return {
    ...row,
    item_id: item.id,
    description: item.purchase_description ?? item.sales_description ?? row.description,
    rate,
    amount: rate ? lineAmount(row.quantity, rate) : row.amount,
  };
}

/** Editing quantity or rate recomputes the amount (exactly, half away from zero). */
export function setQuantity(row: ItemRow, quantity: string): ItemRow {
  const amount = row.rate.trim() ? lineAmount(quantity, row.rate) : row.amount;
  return { ...row, quantity, amount: amount ?? row.amount };
}

export function setRate(row: ItemRow, rate: string): ItemRow {
  const amount = lineAmount(row.quantity, rate);
  return { ...row, rate, amount: amount ?? row.amount };
}

/**
 * Editing the amount back-solves the rate (QBD): rate = amount / quantity, kept
 * to five places. The amount stays exactly as typed, so a rate like 33.33333
 * never rounds the line's money.
 */
export function setItemAmount(row: ItemRow, amount: Cents | null): ItemRow {
  if (amount === null) return { ...row, amount };
  const q = row.quantity.trim();
  const rate = q === '' ? centsToPlain(amount) : backSolveRate(amount, q);
  return { ...row, amount, rate: rate ?? row.rate };
}

export function expensesTotal(rows: readonly ExpenseRow[]): Cents {
  return sumCents(rows.map((r) => r.amount ?? 0));
}

export function itemsTotal(rows: readonly ItemRow[]): Cents {
  return sumCents(rows.map((r) => r.amount ?? 0));
}

/** Rows → API lines (blank rows dropped; lines keep their ids when editing). */
export function toTxnLines(expenses: readonly ExpenseRow[], items: readonly ItemRow[]): TxnLine[] {
  const out: TxnLine[] = [];
  for (const r of expenses) {
    if (isBlankExpense(r)) continue;
    out.push({
      ...blankLine('Account'),
      id: r.lineId ?? r.key,
      account_id: r.account_id,
      amount: r.amount ?? 0,
      description: r.memo.trim() || null,
      entity_id: r.entity_id,
      billable: r.entity_id ? (r.billable ? 'Billable' : 'NotBillable') : 'NotBillable',
      class_id: r.class_id,
    });
  }
  for (const r of items) {
    if (isBlankItem(r)) continue;
    out.push({
      ...blankLine('Item'),
      id: r.lineId ?? r.key,
      item_id: r.item_id,
      description: r.description.trim() || null,
      quantity: normalizeDecimalText(r.quantity),
      rate: normalizeDecimalText(r.rate),
      amount: r.amount ?? 0,
      entity_id: r.entity_id,
      billable: r.entity_id ? (r.billable ? 'Billable' : 'NotBillable') : 'NotBillable',
      class_id: r.class_id,
      linked_txn_id: r.linked_txn_id,
      linked_line_id: r.linked_line_id,
    });
  }
  return out;
}

/** API lines → rows for the two tabs. */
export function fromTxnLines(lines: readonly TxnLine[]): { expenses: ExpenseRow[]; items: ItemRow[] } {
  const expenses: ExpenseRow[] = [];
  const items: ItemRow[] = [];
  for (const l of lines) {
    if (l.kind === 'Item') {
      items.push({
        key: l.id,
        lineId: l.id,
        item_id: l.item_id,
        description: l.description ?? '',
        quantity: l.quantity ?? '',
        rate: l.rate ?? '',
        amount: l.amount,
        entity_id: l.entity_id,
        billable: l.billable !== 'NotBillable',
        class_id: l.class_id,
        linked_txn_id: l.linked_txn_id,
        linked_line_id: l.linked_line_id,
      });
    } else if (l.kind === 'Account') {
      expenses.push({
        key: l.id,
        lineId: l.id,
        account_id: l.account_id,
        amount: l.amount,
        memo: l.description ?? '',
        entity_id: l.entity_id,
        billable: l.billable !== 'NotBillable',
        class_id: l.class_id,
      });
    }
  }
  return { expenses, items };
}

/** Map an API line index (as sent) back to its tab and row. */
export function locateLine(
  expenses: readonly ExpenseRow[],
  items: readonly ItemRow[],
  lineIndex: number,
): { tab: 'expenses' | 'items'; row: number } | null {
  const e = expenses.flatMap((r, i) => (isBlankExpense(r) ? [] : [i]));
  if (lineIndex < e.length) return { tab: 'expenses', row: e[lineIndex]! };
  const it = items.flatMap((r, i) => (isBlankItem(r) ? [] : [i]));
  const j = lineIndex - e.length;
  return j < it.length ? { tab: 'items', row: it[j]! } : null;
}

/** Is a quantity/rate cell's text acceptable (blank or a number)? */
export function decimalTextError(text: string): string | undefined {
  return text.trim() === '' || parseDecimal(text) ? undefined : 'Not a number';
}
