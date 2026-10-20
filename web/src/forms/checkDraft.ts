// The Write Checks form's state as plain data: build it from a saved check or
// from scratch, apply QBD's header-amount behaviour, validate before sending,
// turn it into a `Txn`, and map a server error back onto its fields. The
// component is wiring; the rules live (and are tested) here. Other purchase
// forms (bills, credit card charges) will reuse most of this.

import { blankTxn } from '~/api/blank';
import { describeError, errorTargets, type ApiError } from '~/api/errors';
import type { Entity, Txn } from '~/api/types';
import type { IsoDate } from '~/lib/dates';
import { sumCents, type Cents } from '~/lib/money';
import {
  blankExpense,
  blankItemRow,
  expensesTotal,
  fromTxnLines,
  isBlankExpense,
  isBlankItem,
  itemsTotal,
  locateLine,
  toTxnLines,
  type ExpenseRow,
  type ItemRow,
} from './purchaseLines';
import { withTrailingBlank } from '~/inputs/gridRows';

export interface CheckDraft {
  id: string;
  /** 0 for a new check. */
  edit_seq: number;
  account_id: string | null;
  number: string;
  /** The number came from the server's next-number; replace it when the account changes. */
  numberIsAuto: boolean;
  date: IsoDate;
  entity_id: string | null;
  memo: string;
  to_be_printed: boolean;
  expenses: ExpenseRow[];
  items: ItemRow[];
  /** The saved transaction this draft edits (fields the form doesn't show ride along). */
  base: Txn | null;
}

export function newCheckDraft(date: IsoDate, accountId: string | null): CheckDraft {
  return {
    id: crypto.randomUUID(),
    edit_seq: 0,
    account_id: accountId,
    number: '',
    numberIsAuto: true,
    date,
    entity_id: null,
    memo: '',
    to_be_printed: false,
    expenses: [blankExpense()],
    items: [blankItemRow()],
    base: null,
  };
}

export function draftFromTxn(txn: Txn): CheckDraft {
  const { expenses, items } = fromTxnLines(txn.lines);
  return {
    id: txn.id,
    edit_seq: txn.edit_seq,
    account_id: txn.account_id,
    number: txn.number ?? '',
    numberIsAuto: false,
    date: txn.date,
    entity_id: txn.entity_id,
    memo: txn.memo ?? '',
    to_be_printed: txn.is_to_be_printed,
    expenses: withTrailingBlank(expenses, isBlankExpense, blankExpense),
    items: withTrailingBlank(items, isBlankItem, blankItemRow),
    base: txn,
  };
}

export function checkTotal(d: CheckDraft): Cents {
  return sumCents([expensesTotal(d.expenses), itemsTotal(d.items)]);
}

/**
 * Typing the check's amount in the header (QBD): with no amounts on the lines
 * yet it goes on the first expense line; otherwise the difference goes on the
 * blank line at the bottom of the Expenses tab, so the lines always add up to
 * what the header says.
 */
export function setHeaderAmount(d: CheckDraft, amount: Cents | null): CheckDraft {
  const target = amount ?? 0;
  const total = checkTotal(d);
  if (target === total) return d;
  const expenses = withTrailingBlank([...d.expenses], isBlankExpense, blankExpense);
  if (total === 0) {
    expenses[0] = { ...expenses[0]!, amount: target };
  } else {
    const last = expenses.length - 1;
    expenses[last] = { ...expenses[last]!, amount: (expenses[last]!.amount ?? 0) + (target - total) };
  }
  return { ...d, expenses: withTrailingBlank(expenses, isBlankExpense, blankExpense) };
}

/**
 * Choosing a payee on a new, empty check: QBD's "recall the last transaction for
 * this name" (lines and memo from `last`), else the vendor's default expense account.
 */
export function applyPayee(d: CheckDraft, payee: Entity | undefined, last: Txn | null): CheckDraft {
  const next = { ...d, entity_id: payee?.id ?? null };
  const untouched = d.edit_seq === 0 && checkTotal(d) === 0 && d.expenses.every(isBlankExpense) && d.items.every(isBlankItem);
  if (!payee || !untouched) return next;
  if (last) {
    const { expenses, items } = fromTxnLines(last.lines);
    const fresh = <R extends { key: string; lineId: string | null }>(r: R): R => ({ ...r, key: crypto.randomUUID(), lineId: null });
    return {
      ...next,
      memo: last.memo ?? '',
      expenses: withTrailingBlank(expenses.map(fresh), isBlankExpense, blankExpense),
      items: withTrailingBlank(items.map(fresh), isBlankItem, blankItemRow),
    };
  }
  if (payee.default_expense_account_id) {
    const expenses = [...d.expenses];
    expenses[0] = { ...expenses[0]!, account_id: payee.default_expense_account_id };
    return { ...next, expenses: withTrailingBlank(expenses, isBlankExpense, blankExpense) };
  }
  return next;
}

export function draftToTxn(d: CheckDraft): Txn {
  const base = d.base ?? blankTxn('Check', d.date);
  return {
    ...base,
    id: d.id,
    txn_type: 'Check',
    edit_seq: d.edit_seq,
    account_id: d.account_id,
    number: d.to_be_printed ? null : d.number.trim() || null,
    date: d.date,
    entity_id: d.entity_id,
    memo: d.memo.trim() || null,
    is_to_be_printed: d.to_be_printed,
    amount: checkTotal(d),
    lines: toTxnLines(d.expenses, d.items),
  };
}

export type HeaderField = 'account' | 'date' | 'entity' | 'amount' | 'number';

export interface FormErrors {
  header: Partial<Record<HeaderField, string>>;
  expenses: Record<number, Record<string, string>>;
  items: Record<number, Record<string, string>>;
  /** Errors that belong to no single field. */
  form: string[];
}

export const NO_ERRORS: FormErrors = { header: {}, expenses: {}, items: {}, form: [] };

export function hasErrors(e: FormErrors): boolean {
  return Object.keys(e.header).length > 0 || Object.keys(e.expenses).length > 0 || Object.keys(e.items).length > 0 || e.form.length > 0;
}

function put(map: Record<number, Record<string, string>>, row: number, col: string, msg: string) {
  map[row] = { ...(map[row] ?? {}), [col]: msg };
}

/** What can be caught before asking the server. */
export function validateCheck(d: CheckDraft): FormErrors {
  const e: FormErrors = { header: {}, expenses: {}, items: {}, form: [] };
  if (!d.account_id) e.header.account = 'Choose the account to write the check from';
  if (!d.date) e.header.date = 'Enter a date';
  d.expenses.forEach((r, i) => {
    if (isBlankExpense(r)) return;
    if (!r.account_id) put(e.expenses, i, 'account', 'Choose an account for this line');
    if (r.amount === null && r.account_id) put(e.expenses, i, 'amount', 'Enter an amount');
  });
  d.items.forEach((r, i) => {
    if (isBlankItem(r)) return;
    if (!r.item_id) put(e.items, i, 'item', 'Choose an item for this line');
  });
  const lines = d.expenses.filter((r) => !isBlankExpense(r)).length + d.items.filter((r) => !isBlankItem(r)).length;
  if (lines === 0) e.form.push('Enter at least one expense or item line.');
  if (checkTotal(d) < 0) e.header.amount = "A check's amount can't be negative";
  return e;
}

/** Put a server error next to the fields it's about. */
export function serverErrors(d: CheckDraft, sent: Txn, err: ApiError): FormErrors {
  const e: FormErrors = { header: {}, expenses: {}, items: {}, form: [] };
  const msg = describeError(err);
  for (const t of errorTargets(sent, err)) {
    switch (t.field) {
      case 'line': {
        const where = locateLine(d.expenses, d.items, t.index);
        if (!where) e.form.push(msg);
        else {
          const col = t.column === 'any' ? (where.tab === 'expenses' ? 'account' : 'item') : t.column === 'entity' ? 'customer' : t.column;
          put(where.tab === 'expenses' ? e.expenses : e.items, where.row, col, msg);
        }
        break;
      }
      case 'lines':
      case 'form':
      case 'to_account':
        e.form.push(msg);
        break;
      default:
        e.header[t.field] = msg;
    }
  }
  return e;
}

/** Nothing entered yet on a new check (closing it needn't ask). */
export function isDraftEmpty(d: CheckDraft): boolean {
  return !d.entity_id && !d.memo.trim() && d.expenses.every(isBlankExpense) && d.items.every(isBlankItem);
}

/** A stable fingerprint of what the user can change (dirty tracking of saved checks). */
export function draftFingerprint(d: CheckDraft): string {
  const t = draftToTxn(d);
  return JSON.stringify([t.account_id, t.number, t.date, t.entity_id, t.memo, t.is_to_be_printed, t.lines.map((l) => [l.kind, l.account_id, l.item_id, l.amount, l.description, l.quantity, l.rate, l.entity_id, l.billable, l.class_id])]);
}
