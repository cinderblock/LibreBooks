// The purchase-side forms' state as plain data: Write Checks, Enter Bills,
// Enter Credit Card Charges/Credits, Enter Vendor Credits, Receive Items and
// Create Purchase Orders all share a header (who, when, how much) over the
// Expenses and Items tabs. Build a draft from a saved transaction or from
// scratch, apply QBD's header-amount behaviour, validate before sending, turn
// it into a `Txn`, and map a server error back onto its fields. The components
// are wiring; the rules live (and are tested) here.

import { blankTxn } from '~/api/blank';
import { describeError, errorTargets, type ApiError } from '~/api/errors';
import type { Entity, Terms, Txn, TxnType } from '~/api/types';
import type { IsoDate } from '~/lib/dates';
import { sumCents, type Cents } from '~/lib/money';
import { withTrailingBlank } from '~/inputs/gridRows';
import { noErrors, putGridError, type FormErrors } from './model';
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
import { dueDate } from './terms';

export type PurchaseType = 'Check' | 'Bill' | 'CreditCardCharge' | 'CreditCardCredit' | 'VendorCredit' | 'ItemReceipt' | 'PurchaseOrder';

export const PURCHASE_TYPES: readonly PurchaseType[] = ['Check', 'Bill', 'CreditCardCharge', 'CreditCardCredit', 'VendorCredit', 'ItemReceipt', 'PurchaseOrder'];

export function isPurchaseType(t: TxnType): t is PurchaseType {
  return (PURCHASE_TYPES as readonly TxnType[]).includes(t);
}

/** Forms whose header has a cash account (bank or card); the rest post to A/P or nowhere. */
export function hasCashAccount(t: PurchaseType): boolean {
  return t === 'Check' || t === 'CreditCardCharge' || t === 'CreditCardCredit';
}

/** Forms addressed to a vendor only (A/P, orders). Checks and card charges take any name. */
export function vendorOnly(t: PurchaseType): boolean {
  return !hasCashAccount(t);
}

/** Non-posting: kept with the same machinery, but moves no money. */
export function isNonPosting(t: PurchaseType): boolean {
  return t === 'PurchaseOrder';
}

export interface PurchaseDraft {
  id: string;
  /** 0 for a new transaction. */
  edit_seq: number;
  txn_type: PurchaseType;
  /** The bank or card account (checks, card charges); null for bills, credits, orders. */
  account_id: string | null;
  number: string;
  /** The number came from the server's next-number; replace it when the account changes. */
  numberIsAuto: boolean;
  date: IsoDate;
  entity_id: string | null;
  memo: string;
  to_be_printed: boolean;
  /** Bills: terms and the due date they imply (editable). */
  terms_id: string | null;
  due_date: IsoDate | null;
  class_id: string | null;
  expenses: ExpenseRow[];
  items: ItemRow[];
  /** The saved transaction this draft edits (fields the form doesn't show ride along). */
  base: Txn | null;
}

export function newPurchaseDraft(txn_type: PurchaseType, date: IsoDate, accountId: string | null): PurchaseDraft {
  return {
    id: crypto.randomUUID(),
    edit_seq: 0,
    txn_type,
    account_id: hasCashAccount(txn_type) ? accountId : null,
    number: '',
    numberIsAuto: true,
    date,
    entity_id: null,
    memo: '',
    to_be_printed: false,
    terms_id: null,
    due_date: null,
    class_id: null,
    expenses: [blankExpense()],
    items: [blankItemRow()],
    base: null,
  };
}

export function purchaseDraftFromTxn(txn: Txn): PurchaseDraft {
  const { expenses, items } = fromTxnLines(txn.lines);
  return {
    id: txn.id,
    edit_seq: txn.edit_seq,
    txn_type: isPurchaseType(txn.txn_type) ? txn.txn_type : 'Check',
    account_id: txn.account_id,
    number: txn.number ?? '',
    numberIsAuto: false,
    date: txn.date,
    entity_id: txn.entity_id,
    memo: txn.memo ?? '',
    to_be_printed: txn.is_to_be_printed,
    terms_id: txn.terms_id,
    due_date: txn.due_date,
    class_id: txn.class_id,
    expenses: withTrailingBlank(expenses, isBlankExpense, blankExpense),
    items: withTrailingBlank(items, isBlankItem, blankItemRow),
    base: txn,
  };
}

export function purchaseTotal(d: PurchaseDraft): Cents {
  return sumCents([expensesTotal(d.expenses), itemsTotal(d.items)]);
}

/**
 * Typing the amount in the header (QBD): with no amounts on the lines yet it
 * goes on the first expense line (first item line for item-only forms);
 * otherwise the difference goes on the blank line at the bottom of the
 * Expenses tab, so the lines always add up to what the header says.
 */
export function setHeaderAmount(d: PurchaseDraft, amount: Cents | null): PurchaseDraft {
  const target = amount ?? 0;
  const total = purchaseTotal(d);
  if (target === total) return d;
  if (d.txn_type === 'PurchaseOrder') {
    const items = withTrailingBlank([...d.items], isBlankItem, blankItemRow);
    const i = total === 0 ? 0 : items.length - 1;
    items[i] = { ...items[i]!, amount: (total === 0 ? 0 : (items[i]!.amount ?? 0)) + (target - total) };
    return { ...d, items: withTrailingBlank(items, isBlankItem, blankItemRow) };
  }
  const expenses = withTrailingBlank([...d.expenses], isBlankExpense, blankExpense);
  if (total === 0) {
    expenses[0] = { ...expenses[0]!, amount: target };
  } else {
    const last = expenses.length - 1;
    expenses[last] = { ...expenses[last]!, amount: (expenses[last]!.amount ?? 0) + (target - total) };
  }
  return { ...d, expenses: withTrailingBlank(expenses, isBlankExpense, blankExpense) };
}

/** Choosing terms (or a date with terms set) recomputes the due date. */
export function applyTerms(d: PurchaseDraft, terms: Terms | null | undefined): PurchaseDraft {
  return { ...d, terms_id: terms?.id ?? null, due_date: dueDate(terms, d.date) ?? d.due_date };
}

/**
 * Choosing a payee on a new, empty form: QBD's "recall the last transaction
 * for this name" (lines and memo from `last`), else the vendor's default
 * expense account and terms.
 */
export function applyPayee(d: PurchaseDraft, payee: Entity | undefined, last: Txn | null, terms?: Terms | null): PurchaseDraft {
  let next: PurchaseDraft = { ...d, entity_id: payee?.id ?? null };
  const untouched = d.edit_seq === 0 && purchaseTotal(d) === 0 && d.expenses.every(isBlankExpense) && d.items.every(isBlankItem);
  if (!payee || !untouched) return next;
  if (d.txn_type === 'Bill' && payee.terms_id) next = applyTerms(next, terms ?? null);
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
  if (payee.default_expense_account_id && d.txn_type !== 'PurchaseOrder') {
    const expenses = [...d.expenses];
    expenses[0] = { ...expenses[0]!, account_id: payee.default_expense_account_id };
    return { ...next, expenses: withTrailingBlank(expenses, isBlankExpense, blankExpense) };
  }
  return next;
}

export function purchaseDraftToTxn(d: PurchaseDraft): Txn {
  const base = d.base ?? blankTxn(d.txn_type, d.date);
  return {
    ...base,
    id: d.id,
    txn_type: d.txn_type,
    edit_seq: d.edit_seq,
    account_id: hasCashAccount(d.txn_type) ? d.account_id : null,
    number: d.to_be_printed && d.txn_type === 'Check' ? null : d.number.trim() || null,
    date: d.date,
    entity_id: d.entity_id,
    memo: d.memo.trim() || null,
    is_to_be_printed: d.txn_type === 'Check' ? d.to_be_printed : base.is_to_be_printed,
    terms_id: d.txn_type === 'Bill' ? d.terms_id : base.terms_id,
    due_date: d.txn_type === 'Bill' ? d.due_date : base.due_date,
    class_id: d.class_id,
    amount: purchaseTotal(d),
    lines: toTxnLines(d.expenses, d.items),
  };
}

/** What can be caught before asking the server. */
export function validatePurchase(d: PurchaseDraft): FormErrors {
  const e = noErrors();
  const t = d.txn_type;
  if (hasCashAccount(t) && !d.account_id) e.header.account = t === 'Check' ? 'Choose the account to write the check from' : 'Choose the credit card account';
  if (vendorOnly(t) && !d.entity_id) e.header.entity = 'Choose a vendor';
  if (!d.date) e.header.date = 'Enter a date';
  d.expenses.forEach((r, i) => {
    if (isBlankExpense(r)) return;
    if (!r.account_id) putGridError(e, 'expenses', i, 'account', 'Choose an account for this line');
    if (r.amount === null && r.account_id) putGridError(e, 'expenses', i, 'amount', 'Enter an amount');
  });
  d.items.forEach((r, i) => {
    if (isBlankItem(r)) return;
    if (!r.item_id) putGridError(e, 'items', i, 'item', 'Choose an item for this line');
  });
  const expenseLines = d.expenses.filter((r) => !isBlankExpense(r)).length;
  const itemLines = d.items.filter((r) => !isBlankItem(r)).length;
  if (t === 'PurchaseOrder') {
    if (expenseLines) e.form.push('A purchase order has item lines only.');
    if (itemLines === 0) e.form.push('Enter at least one item line.');
  } else if (expenseLines + itemLines === 0) e.form.push('Enter at least one expense or item line.');
  if (purchaseTotal(d) < 0) e.header.amount = "The amount can't be negative";
  return e;
}

/** Put a server error next to the fields it's about. */
export function purchaseServerErrors(d: PurchaseDraft, sent: Txn, err: ApiError): FormErrors {
  const e = noErrors();
  const msg = describeError(err);
  for (const t of errorTargets(sent, err)) {
    switch (t.field) {
      case 'line': {
        const where = locateLine(d.expenses, d.items, t.index);
        if (!where) e.form.push(msg);
        else {
          const col = t.column === 'any' ? (where.tab === 'expenses' ? 'account' : 'item') : t.column === 'entity' ? 'customer' : t.column;
          putGridError(e, where.tab, where.row, col, msg);
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

/** Nothing entered yet on a new form (closing it needn't ask). */
export function isPurchaseEmpty(d: PurchaseDraft): boolean {
  return !d.entity_id && !d.memo.trim() && d.expenses.every(isBlankExpense) && d.items.every(isBlankItem);
}

/** A stable fingerprint of what the user can change (dirty tracking of saved records). */
export function purchaseFingerprint(d: PurchaseDraft): string {
  const t = purchaseDraftToTxn(d);
  return JSON.stringify([
    t.account_id,
    t.number,
    t.date,
    t.entity_id,
    t.memo,
    t.is_to_be_printed,
    t.terms_id,
    t.due_date,
    t.class_id,
    t.lines.map((l) => [l.kind, l.account_id, l.item_id, l.amount, l.description, l.quantity, l.rate, l.entity_id, l.billable, l.class_id]),
  ]);
}

/** After Save & New: the same kind of form, same account and date, nothing else. */
export function nextPurchaseDraft(d: PurchaseDraft): PurchaseDraft {
  return newPurchaseDraft(d.txn_type, d.date, d.account_id);
}
