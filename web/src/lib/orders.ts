// Orders and what draws on them: a sales order invoiced, an estimate
// progress-invoiced, a purchase order received or billed.
//
// The server keeps the figures (`GET /txns/:id/fulfilment` → `Fulfilment`,
// plans/orders.md): what each order line has had drawn on it through the
// `linked_txn_id` / `linked_line_id` of later documents' lines, what remains,
// which lines are closed (`TxnLine.is_closed`, QBD's "Clsd") and whether the
// order is (`Txn.is_manually_closed`, or every line closed or fully drawn).
// What's here is presentation: the server's figures joined to the order's own
// lines, in the exact decimals the forms draw with (`forms/linked.ts`), and
// the words for them.

import type { Fulfilment, LineFulfilment, Txn, TxnLine, TxnSummary, TxnType } from '~/api/types';
import type { OrderKind } from '~/api/client';
import { ZERO, div, parseDecimal, trim, type Dec } from './decimal';
import { sumCents, type Cents } from './money';

/**
 * What each kind of order offers to make from it (QBD's Create Invoice, Create
 * Sales Order, Create Item Receipt, Enter Bill). A sales order made from an
 * estimate copies the estimate's lines without linking them: the server's link
 * rules (`TxnType::link_targets`) let only an invoice draw on an estimate.
 */
export const LINK_TARGETS: Partial<Record<TxnType, readonly TxnType[]>> = {
  SalesOrder: ['Invoice'],
  Estimate: ['Invoice', 'SalesOrder'],
  PurchaseOrder: ['ItemReceipt', 'Bill'],
};

export type OrderType = 'SalesOrder' | 'Estimate' | 'PurchaseOrder';

export function isOrderType(t: TxnType): t is OrderType {
  return t === 'SalesOrder' || t === 'Estimate' || t === 'PurchaseOrder';
}

/** The `kind` `GET /open` takes for an order type. */
export function orderKind(t: OrderType): OrderKind {
  return t === 'PurchaseOrder' ? 'purchase_orders' : t === 'SalesOrder' ? 'sales_orders' : 'estimates';
}

/**
 * How an order's progress is measured: sales and purchase orders by quantity
 * (QBD's invoiced / received columns), estimates by amount (progress invoicing
 * takes a percentage of the money, not of the things).
 */
export function progressMeasure(t: OrderType): 'quantity' | 'amount' {
  return t === 'Estimate' ? 'amount' : 'quantity';
}

const ONE: Dec = { int: 1n, scale: 0 };

export interface LineProgress {
  line: TxnLine;
  /** The server's figures for the line. */
  fulfilment: LineFulfilment;
  /** The quantity ordered (blank counts as one, as on QBD's forms). */
  ordered: Dec;
  /** Σ quantities of the linked lines (may exceed `ordered`: over-receipt shows). */
  fulfilled: Dec;
  /** What is still to come: `ordered − fulfilled`, never below zero; zero once closed. */
  remaining: Dec;
  amount: Cents;
  amountFulfilled: Cents;
  /** What is left at the order's own price; zero once closed. */
  amountRemaining: Cents;
  /** Closed by hand (its own "Clsd", or the whole order's), or fully drawn: nothing more may link to it. */
  closed: boolean;
}

export interface OrderProgress {
  /** The order's lines that can be drawn on, in order (text lines, subtotals and empty lines are not among them). */
  lines: LineProgress[];
  byLineId: ReadonlyMap<string, LineProgress>;
  amount: Cents;
  amountFulfilled: Cents;
  amountRemaining: Cents;
  /** Of the order's amount, how much is fulfilled, in percent to two places (0 for an empty order). */
  percent: Dec;
  /** Closed by its lines: every line fully drawn or ticked closed, the order itself not stamped. */
  fullyFulfilled: boolean;
  /** QBD's "Closed" stamp (`Txn.is_manually_closed`). */
  manuallyClosed: boolean;
  /** Closed either way: nothing more may be drawn on it. */
  closed: boolean;
  /** The documents drawing on this order (`GET /txns?linked_to=`), oldest first. */
  linked: TxnSummary[];
}

/**
 * The server's fulfilment of an order joined to the order's lines (a figure
 * whose line the order no longer has is dropped), with the documents that
 * drew on it. `linked` may be left empty where only the figures matter.
 */
export function progressFromFulfilment(order: Txn, f: Fulfilment, linked: readonly TxnSummary[] = []): OrderProgress {
  const byId = new Map(order.lines.map((l) => [l.id, l]));
  const lines: LineProgress[] = [];
  for (const lf of f.lines) {
    const line = byId.get(lf.line_id);
    if (!line) continue;
    const ordered = parseDecimal(lf.ordered_quantity) ?? ONE;
    const fulfilled = trim(parseDecimal(lf.fulfilled_quantity) ?? ZERO);
    // A line without a quantity is one of something: its whole remains until it is drawn or closed.
    const remaining = lf.remaining_quantity !== null ? trim(parseDecimal(lf.remaining_quantity) ?? ZERO) : lf.is_closed || lf.remaining_amount === 0 ? ZERO : ONE;
    lines.push({
      line,
      fulfilment: lf,
      ordered,
      fulfilled,
      remaining,
      amount: lf.ordered_amount,
      amountFulfilled: lf.fulfilled_amount,
      amountRemaining: lf.remaining_amount,
      closed: lf.is_closed,
    });
  }
  const amount = sumCents(lines.map((l) => l.amount));
  const amountFulfilled = sumCents(lines.map((l) => l.amountFulfilled));
  const manuallyClosed = !!order.is_manually_closed;
  const percent = amount === 0 ? ZERO : (div({ int: BigInt(amountFulfilled) * 100n, scale: 0 }, { int: BigInt(amount), scale: 0 }, 2) ?? ZERO);
  return {
    lines,
    byLineId: new Map(lines.map((l) => [l.line.id, l])),
    amount,
    amountFulfilled,
    amountRemaining: f.remaining_amount,
    percent,
    fullyFulfilled: f.is_closed && !manuallyClosed,
    manuallyClosed,
    closed: f.is_closed,
    linked: [...linked].sort((a, b) => (a.date < b.date ? -1 : a.date > b.date ? 1 : 0)),
  };
}

/** The distinct transactions a document's lines draw on, in line order. */
export function linkedOrderIds(txn: Pick<Txn, 'lines'>): string[] {
  const out: string[] = [];
  for (const l of txn.lines) if (l.linked_txn_id && !out.includes(l.linked_txn_id)) out.push(l.linked_txn_id);
  return out;
}
