// Drawing on an order: the lines and header a new invoice takes from a sales
// order or an estimate, a new item receipt or bill from a purchase order, a new
// bill from an item receipt. Each drawn line carries `linked_txn_id` /
// `linked_line_id` back to the line it fulfils; the server sums those into the
// order's fulfilment (`lib/orders.ts` presents it) and refuses a link to a
// closed line or order. Pure and unit-tested; the dialogs only collect the
// choices.

import { blankLine, blankTxn, newId } from '~/api/blank';
import type { Txn, TxnLine, TxnType } from '~/api/types';
import { ZERO, compare, div, isZero, mul, parseDecimal, round, toApiString, trim, type Dec } from '~/lib/decimal';
import { centsToDec, decToCents, extend, percentOf, type Cents } from '~/lib/money';
import { type LineProgress, type OrderProgress, type OrderType, LINK_TARGETS } from '~/lib/orders';

/** QBD's three ways to invoice an estimate. */
export type ProgressChoice = 'all' | 'percent' | 'selected';

/** How much of each order line a new document takes, by line id (absent → nothing). */
export type Draw = ReadonlyMap<string, { quantity: Dec; amount: Cents }>;

/** Everything that is left: the whole order (minus what earlier documents took). */
export function drawRemaining(progress: OrderProgress): Draw {
  const out = new Map<string, { quantity: Dec; amount: Cents }>();
  for (const l of progress.lines) {
    if (isZero(l.remaining) && l.amountRemaining === 0) continue;
    out.set(l.line.id, { quantity: l.remaining, amount: l.amountRemaining });
  }
  return out;
}

/**
 * A quantity of each line, as typed in the "To Invoice" / "To Receive" column:
 * the amount follows the order line's rate, or its proportion of the line's
 * amount when the line has no rate (a flat charge invoiced in parts).
 */
export function drawQuantities(progress: OrderProgress, quantities: ReadonlyMap<string, string>): Draw {
  const out = new Map<string, { quantity: Dec; amount: Cents }>();
  for (const l of progress.lines) {
    if (l.closed) continue;
    const q = parseDecimal(quantities.get(l.line.id));
    if (!q || compare(q, ZERO) <= 0) continue;
    out.set(l.line.id, { quantity: trim(q), amount: amountForQuantity(l, q) });
  }
  return out;
}

/** The money for `quantity` of an order line: quantity × rate, else the matching share of the line's amount. */
export function amountForQuantity(l: LineProgress, quantity: Dec): Cents {
  if (l.line.rate && !l.line.rate_is_percent) {
    const amt = extend(toApiString(quantity), l.line.rate);
    if (amt !== null) return amt;
  }
  if (isZero(l.ordered)) return 0;
  const share = div(mul(centsToDec(l.amount), quantity), l.ordered, 6);
  return share ? (decToCents(share) ?? 0) : 0;
}

/**
 * Progress invoicing by percent: each line's estimate amount × percent, rounded
 * per line (as QBD does; the lines' sum is the invoice). Quantities stay the
 * estimate's, so the invoice reads "10 at 18.50, 50%" rather than "5 at 18.50".
 */
export function drawPercent(progress: OrderProgress, percent: string): Draw {
  const out = new Map<string, { quantity: Dec; amount: Cents }>();
  const p = parseDecimal(percent);
  if (!p || compare(p, ZERO) <= 0) return out;
  for (const l of progress.lines) {
    if (l.closed) continue;
    const amount = percentOf(l.amount, percent) ?? 0;
    if (amount === 0) continue;
    out.set(l.line.id, { quantity: l.ordered, amount });
  }
  return out;
}

/**
 * Progress invoicing by amount per line ("selected items or different
 * percentages of each item"): the amounts as typed, with the estimate's
 * quantities. A line's percent column is derived from its amount, so typing
 * either one lands here as an amount (see `percentToAmount`).
 */
export function drawAmounts(progress: OrderProgress, amounts: ReadonlyMap<string, Cents | null>): Draw {
  const out = new Map<string, { quantity: Dec; amount: Cents }>();
  for (const l of progress.lines) {
    if (l.closed) continue;
    const amount = amounts.get(l.line.id);
    if (!amount) continue;
    out.set(l.line.id, { quantity: l.ordered, amount });
  }
  return out;
}

/** `amount` as a percent of the line's estimate amount, to two places ("50" for half). Null for a zero line. */
export function amountToPercent(l: LineProgress, amount: Cents): string | null {
  if (l.amount === 0) return null;
  const pct = div({ int: BigInt(amount) * 100n, scale: 0 }, { int: BigInt(l.amount), scale: 0 }, 2);
  return pct ? toApiString(pct) : null;
}

/** A percent of the line's estimate amount, in cents (half away from zero). */
export function percentToAmount(l: LineProgress, percent: string): Cents | null {
  return percentOf(l.amount, percent);
}

/** The percent of the whole estimate a set of amounts comes to, two places. */
export function drawPercentOfTotal(progress: OrderProgress, draw: Draw): Dec {
  if (progress.amount === 0) return ZERO;
  let total = 0;
  for (const d of draw.values()) total += d.amount;
  return round(div({ int: BigInt(total) * 100n, scale: 0 }, { int: BigInt(progress.amount), scale: 0 }, 4) ?? ZERO, 2);
}

export function drawTotal(draw: Draw): Cents {
  let total = 0;
  for (const d of draw.values()) total += d.amount;
  return total;
}

/**
 * The lines a new document takes from an order: one per drawn order line, in
 * the order's sequence, each linked back to its source (unless `link` is off:
 * a sales order made from an estimate copies the lines, since only an invoice
 * may link to an estimate). Quantities are the drawn ones; rate, description,
 * item, class, tax code and the customer:job on a purchase-order line ride along.
 */
export function drawnLines(order: Txn, progress: OrderProgress, draw: Draw, link = true): TxnLine[] {
  const out: TxnLine[] = [];
  for (const l of progress.lines) {
    const d = draw.get(l.line.id);
    if (!d) continue;
    const src = l.line;
    out.push({
      ...blankLine('Item'),
      id: newId(),
      item_id: src.item_id,
      description: src.description,
      quantity: toApiString(d.quantity),
      unit_of_measure: src.unit_of_measure,
      rate: src.rate,
      rate_is_percent: src.rate_is_percent,
      amount: d.amount,
      class_id: src.class_id,
      entity_id: src.entity_id,
      billable: src.billable,
      sales_tax_code_id: src.sales_tax_code_id,
      service_date: src.service_date,
      linked_txn_id: link ? order.id : null,
      linked_line_id: link ? src.id : null,
    });
  }
  return out;
}

/** Whether a document of `type` made from an order links its lines to it (the server's `TxnType::link_targets`). */
export function linksTo(type: TxnType, orderType: TxnType): boolean {
  return !(type === 'SalesOrder' && orderType === 'Estimate');
}

/** What a sales order or estimate becomes, and what a purchase order becomes. */
export function targetTypes(orderType: OrderType): readonly TxnType[] {
  return LINK_TARGETS[orderType] ?? [];
}

/**
 * A new, unsaved document drawing `draw` from `order`: an invoice or sales order
 * from a sales order or estimate (customer, class, terms, addresses, sales tax,
 * P.O. number and rep come along), an item receipt or bill from a purchase order
 * (vendor, class, memo). The caller hands it to the form through
 * `windows/handoff.ts`.
 */
export function documentFromOrder(order: Txn, progress: OrderProgress, draw: Draw, type: TxnType, date: string): Txn {
  const t = blankTxn(type, date);
  t.entity_id = order.entity_id;
  t.class_id = order.class_id;
  t.memo = order.memo;
  t.lines = drawnLines(order, progress, draw, linksTo(type, order.txn_type));
  if (order.txn_type === 'SalesOrder' || order.txn_type === 'Estimate') {
    t.terms_id = order.terms_id;
    t.sales_rep_id = order.sales_rep_id;
    t.po_number = order.po_number;
    t.bill_address = order.bill_address;
    t.ship_address = order.ship_address;
    t.sales_tax_item_id = order.sales_tax_item_id;
    t.customer_tax_code_id = order.customer_tax_code_id;
    t.message = order.message;
  }
  t.amount = t.lines.reduce((s, l) => s + l.amount, 0);
  return t;
}

/**
 * Enter Bill for Received Items: the bill takes the receipt's lines whole (QBD
 * turns the receipt into the bill; here the bill is its own transaction whose
 * lines link to the receipt's, which the server accepts for a bill of the same
 * vendor and which does not count toward the PO again: the receipt did).
 */
export function linesFromReceipt(receipt: Txn): TxnLine[] {
  return receipt.lines
    .filter((l) => l.kind === 'Item' || l.kind === 'Account')
    .map((src) => ({
      ...blankLine(src.kind),
      id: newId(),
      account_id: src.account_id,
      item_id: src.item_id,
      description: src.description,
      quantity: src.quantity,
      unit_of_measure: src.unit_of_measure,
      rate: src.rate,
      rate_is_percent: src.rate_is_percent,
      amount: src.amount,
      class_id: src.class_id,
      entity_id: src.entity_id,
      billable: src.billable,
      sales_tax_code_id: src.sales_tax_code_id,
      service_date: src.service_date,
      linked_txn_id: receipt.id,
      linked_line_id: src.id,
    }));
}

/** A new, unsaved bill for one or more item receipts of the same vendor (the first receipt's class and memo). */
export function billFromReceipts(receipts: readonly Txn[], date: string): Txn {
  const t = blankTxn('Bill', date);
  const first = receipts[0];
  t.entity_id = first?.entity_id ?? null;
  t.class_id = first?.class_id ?? null;
  t.memo = first?.memo ?? null;
  t.lines = receipts.flatMap(linesFromReceipt);
  t.amount = t.lines.reduce((s, l) => s + l.amount, 0);
  return t;
}
