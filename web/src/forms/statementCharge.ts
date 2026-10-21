// Enter Statement Charges: QBD's A/R register for one customer:job, with a
// new-charge line at the foot. The line is a `StatementCharge` transaction
// with one item line and no sales tax (QBD charges carry none); the register
// rows are the customer's A/R items with a charge / paid split and a running
// balance. Pure and unit-tested; the window only wires inputs to these.

import { blankTxn } from '~/api/blank';
import type { Txn, TxnSummary, TxnType } from '~/api/types';
import type { IsoDate } from '~/lib/dates';
import { isIsoDate } from '~/lib/dates';
import { extend, type Cents } from '~/lib/money';
import { blankSaleRow, isBlankSale, toSaleLines, type SaleRow } from './salesLines';

export interface ChargeEntry {
  date: IsoDate;
  number: string;
  due_date: IsoDate | null;
  class_id: string | null;
  /** Item, quantity, rate, amount and description, with the sales-line arithmetic. */
  row: SaleRow;
}

export function blankChargeEntry(date: IsoDate, due_date: IsoDate | null = null): ChargeEntry {
  return { date, number: '', due_date, class_id: null, row: blankSaleRow() };
}

export function isBlankCharge(e: ChargeEntry): boolean {
  return isBlankSale(e.row) && !e.number.trim();
}

export type ChargeField = 'customer' | 'date' | 'item' | 'amount' | 'form';

export type ChargeResult = { ok: true; txn: Txn } | { ok: false; field: ChargeField; message: string };

/**
 * The transaction a charge line records: one item line at quantity × rate (or
 * the amount typed), dated and due as entered, no sales tax item or code.
 */
export function chargeEntryToTxn(customerId: string | null, e: ChargeEntry): ChargeResult {
  if (!customerId) return { ok: false, field: 'customer', message: 'Choose a customer or job.' };
  if (!isIsoDate(e.date)) return { ok: false, field: 'date', message: 'Enter a date.' };
  if (!e.row.item_id) return { ok: false, field: 'item', message: 'Choose an item for the charge.' };
  const amount = e.row.amount ?? extend(e.row.quantity.trim() === '' ? '1' : e.row.quantity, e.row.rate);
  if (amount === null) return { ok: false, field: 'amount', message: 'Enter an amount, or a quantity and rate.' };
  const t = blankTxn('StatementCharge', e.date);
  t.entity_id = customerId;
  t.number = e.number.trim() || null;
  t.due_date = e.due_date;
  t.class_id = e.class_id;
  // The register's Description column is the line's; the memo repeats it so lists and the Center show it.
  t.memo = e.row.description.trim() || null;
  t.lines = toSaleLines([{ ...e.row, amount, sales_tax_code_id: null }]);
  t.amount = amount;
  return { ok: true, txn: t };
}

/** What the A/R register lists for a customer, and which side each kind lands on. */
export const AR_REGISTER_TYPES: readonly TxnType[] = ['Invoice', 'StatementCharge', 'CreditMemo', 'ReceivePayment'];

const PAID_TYPES: readonly TxnType[] = ['CreditMemo', 'ReceivePayment'];

export interface ArRow {
  summary: TxnSummary;
  /** Amt Chrg: an invoice or statement charge. */
  charge: Cents;
  /** Amt Paid: a payment or a credit memo. */
  paid: Cents;
  /** Running balance, oldest first. */
  balance: Cents;
}

/** The register's rows in date order with QBD's Amt Chrg / Amt Paid columns and the running balance (void rows count for nothing). */
export function arRegisterRows(summaries: readonly TxnSummary[]): ArRow[] {
  const sorted = [...summaries].sort((a, b) => (a.date < b.date ? -1 : a.date > b.date ? 1 : 0));
  let balance = 0;
  return sorted.map((s) => {
    const amount = s.is_void ? 0 : s.amount;
    const paid = PAID_TYPES.includes(s.txn_type) ? amount : 0;
    const charge = PAID_TYPES.includes(s.txn_type) ? 0 : amount;
    balance += charge - paid;
    return { summary: s, charge, paid, balance };
  });
}
