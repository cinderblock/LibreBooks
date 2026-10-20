// Pay Bills as plain data: every open bill in one grid (any vendor), ticked
// bills with their discount and credit, and the bill payments that result: one
// BillPaymentCheck (or BillPaymentCreditCard) per vendor, as QBD writes them.

import { blankTxn } from '~/api/blank';
import type { AccountType, Terms, Txn, TxnSummary } from '~/api/types';
import type { IsoDate } from '~/lib/dates';
import { sumCents, type Cents } from '~/lib/money';
import { termsDiscount } from './terms';

export interface BillRow {
  bill: TxnSummary;
  selected: boolean;
  /** Early-payment discount taken. */
  discount: Cents;
  /** A vendor credit (or unapplied payment) used against it. */
  credit_txn_id: string | null;
  credit_amount: Cents;
  /** Cash to pay. */
  amount: Cents;
}

export const open = (b: TxnSummary): Cents => b.open_balance ?? 0;

export function billRows(bills: readonly TxnSummary[], previous: readonly BillRow[] = []): BillRow[] {
  const prev = new Map(previous.map((r) => [r.bill.id, r]));
  return bills
    .filter((b) => open(b) > 0 && !b.is_void)
    .map((b) => prev.get(b.id) ?? { bill: b, selected: false, discount: 0, credit_txn_id: null, credit_amount: 0, amount: 0 })
    .map((r) => ({ ...r, bill: bills.find((b) => b.id === r.bill.id) ?? r.bill }));
}

/** Show bills due on or before a date, or all. */
export function filterRows(rows: readonly BillRow[], dueBy: IsoDate | null): BillRow[] {
  return rows.filter((r) => !dueBy || !r.bill.due_date || r.bill.due_date <= dueBy || r.selected);
}

/** What's left to pay in cash after the discount and credit. */
export function cashFor(r: BillRow): Cents {
  return Math.max(0, open(r.bill) - r.discount - r.credit_amount);
}

export function selectRow(rows: readonly BillRow[], id: string, selected: boolean): BillRow[] {
  return rows.map((r) => (r.bill.id === id ? { ...r, selected, amount: selected ? cashFor(r) : 0 } : r));
}

export function selectAll(rows: readonly BillRow[], selected: boolean): BillRow[] {
  return rows.map((r) => ({ ...r, selected, amount: selected ? cashFor(r) : 0 }));
}

export function setRowDiscount(rows: readonly BillRow[], id: string, discount: Cents): BillRow[] {
  return rows.map((r) => (r.bill.id === id ? { ...r, discount, selected: true, amount: cashFor({ ...r, discount }) } : r));
}

export function setRowCredit(rows: readonly BillRow[], id: string, creditId: string | null, amount: Cents): BillRow[] {
  return rows.map((r) => {
    if (r.bill.id !== id) return r;
    const next = { ...r, credit_txn_id: amount > 0 ? creditId : null, credit_amount: amount > 0 && creditId ? amount : 0, selected: true };
    return { ...next, amount: cashFor(next) };
  });
}

/** Paying less than what's open (a partial payment). */
export function setRowAmount(rows: readonly BillRow[], id: string, amount: Cents): BillRow[] {
  return rows.map((r) => (r.bill.id === id ? { ...r, amount: Math.max(0, Math.min(amount, cashFor(r))), selected: amount > 0 } : r));
}

/** The discount the bill's terms offer when paid on `payDate`. */
export function suggestedDiscount(bill: TxnSummary, terms: Terms | null | undefined, payDate: IsoDate): Cents {
  return Math.min(open(bill), termsDiscount(terms, bill.date, payDate, bill.amount));
}

export interface PayBillsTotals {
  selected: number;
  open: Cents;
  discount: Cents;
  credit: Cents;
  cash: Cents;
}

export function payBillsTotals(rows: readonly BillRow[]): PayBillsTotals {
  const sel = rows.filter((r) => r.selected);
  return {
    selected: sel.length,
    open: sumCents(sel.map((r) => open(r.bill))),
    discount: sumCents(sel.map((r) => r.discount)),
    credit: sumCents(sel.map((r) => r.credit_amount)),
    cash: sumCents(sel.map((r) => r.amount)),
  };
}

export interface PayBillsOptions {
  date: IsoDate;
  account_id: string;
  account_type: AccountType;
  /** First check number; following vendors get the next ones. Null: to be printed. */
  number: string | null;
  discount_account_id: string | null;
  memo: string | null;
}

/** One payment per vendor, in the order the bills are listed. */
export function buildBillPayments(rows: readonly BillRow[], opts: PayBillsOptions): Txn[] {
  const byVendor = new Map<string, BillRow[]>();
  for (const r of rows) {
    if (!r.selected || (r.amount <= 0 && r.discount <= 0 && r.credit_amount <= 0) || !r.bill.entity_id) continue;
    byVendor.set(r.bill.entity_id, [...(byVendor.get(r.bill.entity_id) ?? []), r]);
  }
  const card = opts.account_type === 'CreditCard';
  const numeric = opts.number && /^\d+$/.test(opts.number) ? Number(opts.number) : null;
  let n = 0;
  const out: Txn[] = [];
  for (const [vendor, list] of byVendor) {
    const t = blankTxn(card ? 'BillPaymentCreditCard' : 'BillPaymentCheck', opts.date);
    t.entity_id = vendor;
    t.account_id = opts.account_id;
    t.amount = sumCents(list.map((r) => r.amount));
    t.memo = opts.memo;
    t.is_to_be_printed = !card && opts.number === null;
    t.number = card ? null : numeric !== null ? String(numeric + n) : opts.number;
    t.applications = list.map((r) => ({
      target_txn_id: r.bill.id,
      amount: r.amount,
      discount: r.discount,
      discount_account_id: r.discount > 0 ? opts.discount_account_id : null,
      discount_class_id: null,
      credit_txn_id: r.credit_amount > 0 ? r.credit_txn_id : null,
      credit_amount: r.credit_amount,
    }));
    out.push(t);
    n++;
  }
  return out;
}

/** What stops the payments from being built, or null. */
export function validatePayBills(rows: readonly BillRow[], opts: Partial<PayBillsOptions>): string | null {
  const t = payBillsTotals(rows);
  if (t.selected === 0) return 'Tick at least one bill to pay.';
  if (!opts.account_id) return 'Choose the account to pay from.';
  if (t.discount > 0 && !opts.discount_account_id) return 'Choose the account for the discounts.';
  if (rows.some((r) => r.selected && r.discount + r.credit_amount + r.amount > open(r.bill))) return "A bill can't be paid more than what's open on it.";
  return null;
}
