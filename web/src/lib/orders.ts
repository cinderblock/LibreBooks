// Orders and what draws on them: a sales order invoiced, an estimate
// progress-invoiced, a purchase order received or billed.
//
// The API links a document's line to the order line it fulfils through
// `TxnLine.linked_txn_id` / `linked_line_id` and keeps nothing else: the server
// has no "invoiced quantity", "received quantity", "fully invoiced" or "manually
// closed" field (plans/web-ui.md, "For the server"). So everything here is
// derived, exactly, from the order and the documents linked to it:
// remaining = ordered − Σ linked quantities, per line, in BigInt-scaled decimals.
// A manual close is kept in the transaction's `extra` under QBD's own tag name,
// which the server stores verbatim.

import type { Txn, TxnLine, TxnType } from '~/api/types';
import { ZERO, add, compare, div, isZero, parseDecimal, sub, trim, type Dec } from './decimal';
import { sumCents, type Cents } from './money';

/** What each kind of order can be drawn on by. */
export const LINK_TARGETS: Partial<Record<TxnType, readonly TxnType[]>> = {
  SalesOrder: ['Invoice'],
  Estimate: ['Invoice'],
  PurchaseOrder: ['ItemReceipt', 'Bill'],
};

export type OrderType = 'SalesOrder' | 'Estimate' | 'PurchaseOrder';

export function isOrderType(t: TxnType): t is OrderType {
  return t === 'SalesOrder' || t === 'Estimate' || t === 'PurchaseOrder';
}

/**
 * How an order's progress is measured: sales and purchase orders by quantity
 * (QBD's invoiced / received columns), estimates by amount (progress invoicing
 * takes a percentage of the money, not of the things).
 */
export function progressMeasure(t: OrderType): 'quantity' | 'amount' {
  return t === 'Estimate' ? 'amount' : 'quantity';
}

/** QBD's qbXML tag for a sales or purchase order closed by hand; kept in `Txn.extra`. */
export const MANUALLY_CLOSED_KEY = 'IsManuallyClosed';

export function isManuallyClosed(txn: Pick<Txn, 'extra'>): boolean {
  const v = txn.extra?.[MANUALLY_CLOSED_KEY];
  return v === true || v === 'true';
}

/** The transaction with its manual-close mark set or cleared (the rest of `extra` kept). */
export function withManuallyClosed<T extends Pick<Txn, 'extra'>>(txn: T, closed: boolean): T {
  const rest = { ...(txn.extra ?? {}) };
  delete rest[MANUALLY_CLOSED_KEY];
  const extra = closed ? { ...rest, [MANUALLY_CLOSED_KEY]: true } : rest;
  return { ...txn, extra: Object.keys(extra).length ? extra : null };
}

const ONE: Dec = { int: 1n, scale: 0 };

/** A line's quantity for the arithmetic: blank means one, as on QBD's forms. */
export function lineQuantity(line: Pick<TxnLine, 'quantity'>): Dec {
  return parseDecimal(line.quantity) ?? ONE;
}

/** Lines that carry goods or services (text and subtotal lines don't). */
export function countsTowardOrder(line: TxnLine): boolean {
  return line.kind === 'Item' && !!line.item_id;
}

export interface LineProgress {
  line: TxnLine;
  /** The quantity ordered (blank counts as one). */
  ordered: Dec;
  /** Σ quantities of the linked lines. */
  fulfilled: Dec;
  /** `ordered − fulfilled`, never below zero. */
  remaining: Dec;
  amount: Cents;
  amountFulfilled: Cents;
  /** `amount − amountFulfilled`, never below zero. */
  amountRemaining: Cents;
}

export interface OrderProgress {
  /** The order's lines that can be drawn on, in order. */
  lines: LineProgress[];
  byLineId: ReadonlyMap<string, LineProgress>;
  amount: Cents;
  amountFulfilled: Cents;
  amountRemaining: Cents;
  /** Of the order's amount, how much is fulfilled, in percent to two places (0 for an empty order). */
  percent: Dec;
  /** Every line is fully drawn (by the order type's measure). */
  fullyFulfilled: boolean;
  manuallyClosed: boolean;
  /** Fully fulfilled or closed by hand: the order is no longer open. */
  closed: boolean;
  /** The documents drawing on this order (non-void), oldest first. */
  linked: Txn[];
}

const max0 = (d: Dec): Dec => (compare(d, ZERO) < 0 ? ZERO : d);

/**
 * What has been drawn on an order. `candidates` may be any transactions
 * (typically the name's invoices, or receipts and bills); only non-void ones
 * with a line linked to this order count.
 */
export function orderProgress(order: Txn, candidates: readonly Txn[]): OrderProgress {
  const linked = candidates
    .filter((t) => !t.is_void && t.id !== order.id && t.lines.some((l) => l.linked_txn_id === order.id))
    .sort((a, b) => (a.date < b.date ? -1 : a.date > b.date ? 1 : 0));
  const fulfilledQty = new Map<string, Dec>();
  const fulfilledAmt = new Map<string, Cents>();
  for (const t of linked) {
    for (const l of t.lines) {
      if (l.linked_txn_id !== order.id || !l.linked_line_id) continue;
      fulfilledQty.set(l.linked_line_id, add(fulfilledQty.get(l.linked_line_id) ?? ZERO, lineQuantity(l)));
      fulfilledAmt.set(l.linked_line_id, (fulfilledAmt.get(l.linked_line_id) ?? 0) + l.amount);
    }
  }
  const lines: LineProgress[] = order.lines.filter(countsTowardOrder).map((line) => {
    const ordered = lineQuantity(line);
    const fulfilled = trim(fulfilledQty.get(line.id) ?? ZERO);
    const amountFulfilled = fulfilledAmt.get(line.id) ?? 0;
    return {
      line,
      ordered,
      fulfilled,
      remaining: trim(max0(sub(ordered, fulfilled))),
      amount: line.amount,
      amountFulfilled,
      amountRemaining: Math.max(0, line.amount - amountFulfilled),
    };
  });
  const amount = sumCents(lines.map((l) => l.amount));
  const amountFulfilled = sumCents(lines.map((l) => l.amountFulfilled));
  const measure = isOrderType(order.txn_type) ? progressMeasure(order.txn_type) : 'quantity';
  const fullyFulfilled = lines.length > 0 && lines.every((l) => (measure === 'amount' ? l.amountRemaining === 0 : isZero(l.remaining)));
  const manuallyClosed = isManuallyClosed(order);
  const percent = amount === 0 ? ZERO : (div({ int: BigInt(amountFulfilled) * 100n, scale: 0 }, { int: BigInt(amount), scale: 0 }, 2) ?? ZERO);
  return {
    lines,
    byLineId: new Map(lines.map((l) => [l.line.id, l])),
    amount,
    amountFulfilled,
    amountRemaining: Math.max(0, amount - amountFulfilled),
    percent,
    fullyFulfilled,
    manuallyClosed,
    closed: fullyFulfilled || manuallyClosed,
    linked,
  };
}

/** The remaining amount on an order (0 once closed): what a server-side `open_balance` for an order would be. */
export function orderOpenAmount(order: Txn, candidates: readonly Txn[]): Cents {
  const p = orderProgress(order, candidates);
  return p.closed ? 0 : p.amountRemaining;
}

/** The distinct orders a document's lines draw on, in line order. */
export function linkedOrderIds(txn: Pick<Txn, 'lines'>): string[] {
  const out: string[] = [];
  for (const l of txn.lines) if (l.linked_txn_id && !out.includes(l.linked_txn_id)) out.push(l.linked_txn_id);
  return out;
}
