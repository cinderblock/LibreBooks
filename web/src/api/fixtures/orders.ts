// The fixture's half of order fulfilment, a line-for-line mirror of the
// server's `lb-store/src/orders.rs`: the `Fulfilment` that `GET
// /txns/:id/fulfilment` answers (an order's lines against what the linked lines
// of saved, non-void transactions have drawn), and the link rules the server
// checks when the drawing transaction is saved (400 `invalid`, the same
// messages). Nothing in the UI reads this; it reads the API, and the fixture
// answers as the server does.

import type { Fulfilment, LineFulfilment, Txn, TxnLine, TxnType } from '../types';
import { ApiError } from '../errors';
import { ZERO, add, compare, div, isZero, mul, parseDecimal, round, sub, toApiString, type Dec } from '~/lib/decimal';
import { centsToDec, decToCents, type Cents } from '~/lib/money';
import { TXN_LABEL } from '~/lib/txnTypes';

/** `TxnType::is_order`. */
export function isOrder(t: TxnType): boolean {
  return t === 'PurchaseOrder' || t === 'SalesOrder' || t === 'Estimate';
}

/** `TxnType::link_targets`: what a line of each form may link to, other than a deposit's payment. */
export function linkTargets(t: TxnType): readonly TxnType[] {
  switch (t) {
    case 'ItemReceipt':
    case 'Check':
    case 'CreditCardCharge':
      return ['PurchaseOrder'];
    case 'Bill':
      return ['PurchaseOrder', 'ItemReceipt'];
    case 'Invoice':
      return ['SalesOrder', 'Estimate'];
    case 'SalesReceipt':
      return ['SalesOrder'];
    default:
      return [];
  }
}

/** The order line a link without a line id lands on: the first with the same item (`matching_line`). */
function matchingLine(target: Txn, line: TxnLine): TxnLine | undefined {
  if (!line.item_id) return undefined;
  return target.lines.find((l) => l.kind === 'Item' && l.item_id === line.item_id);
}

/** How much of `fulfilled` counts toward an ordered amount of `ordered`'s sign (`toward`). */
const toward = (fulfilled: Cents, ordered: Cents): Cents => (ordered < 0 ? -fulfilled : fulfilled);

/** `amount × part / whole`, rounded to money half away from zero (`prorate`). */
export function prorate(amount: Cents, part: Dec, whole: Dec): Cents {
  if (isZero(whole)) return 0;
  const v = div(mul(centsToDec(amount), part), whole, 6);
  if (!v) return 0;
  return decToCents(round(v, 2)) ?? 0;
}

const dec = (s: string | null): Dec | null => parseDecimal(s);
const str = (d: Dec): string => toApiString(d);

export interface FulfilmentBooks {
  /** Every non-void transaction (the `txn_link` join: only saved, non-void lines count). */
  linkedTo(orderId: string): readonly Txn[];
  /** Subtotal and group items can't be received or invoiced (`fulfillable`). */
  isSubtotalOrGroup(itemId: string): boolean;
}

/** `orders::compute`: the order's fulfillable lines against what has been drawn on each. */
export function computeFulfilment(order: Txn, books: FulfilmentBooks): Fulfilment {
  const linked = new Map<string, { quantity: Dec; amount: Cents }>();
  for (const t of books.linkedTo(order.id)) {
    if (t.is_void) continue;
    for (const line of t.lines) {
      if (line.linked_txn_id !== order.id) continue;
      const target = line.linked_line_id ?? matchingLine(order, line)?.id;
      if (!target) continue;
      const e = linked.get(target) ?? { quantity: ZERO, amount: 0 };
      linked.set(target, { quantity: add(e.quantity, dec(line.quantity) ?? ZERO), amount: e.amount + line.amount });
    }
  }
  const byAmountOnly = order.txn_type === 'Estimate';
  const lines: LineFulfilment[] = [];
  for (const l of order.lines) {
    if (!fulfillable(l, books)) continue;
    const drawn = linked.get(l.id) ?? { quantity: ZERO, amount: 0 };
    const lineQty = dec(l.quantity);
    const quantity = lineQty && !isZero(lineQty) && !byAmountOnly ? lineQty : null;
    const fully = quantity ? compare(drawn.quantity, quantity) >= 0 : toward(drawn.amount, l.amount) >= Math.abs(l.amount);
    const is_closed = !!order.is_manually_closed || !!l.is_closed || fully;
    const remainingQty = lineQty ? (is_closed ? ZERO : max0(sub(lineQty, drawn.quantity))) : null;
    let remaining_amount: Cents;
    if (is_closed) remaining_amount = 0;
    else if (quantity) remaining_amount = prorate(l.amount, remainingQty ?? ZERO, quantity);
    else {
      const left = Math.max(0, Math.abs(l.amount) - toward(drawn.amount, l.amount));
      remaining_amount = l.amount < 0 ? -left : left;
    }
    lines.push({
      line_id: l.id,
      item_id: l.item_id,
      ordered_quantity: lineQty ? str(lineQty) : null,
      fulfilled_quantity: str(drawn.quantity),
      ordered_amount: l.amount,
      fulfilled_amount: drawn.amount,
      is_closed,
      remaining_quantity: remainingQty ? str(remainingQty) : null,
      remaining_amount,
    });
  }
  const is_closed = !!order.is_manually_closed || lines.every((l) => l.is_closed);
  return {
    order_id: order.id,
    is_closed,
    remaining_amount: is_closed ? 0 : lines.reduce((s, l) => s + l.remaining_amount, 0),
    lines,
  };
}

const max0 = (d: Dec): Dec => (compare(d, ZERO) < 0 ? ZERO : d);

/** `fulfillable`: not text, not a subtotal or group, not an empty line. */
function fulfillable(l: TxnLine, books: FulfilmentBooks): boolean {
  if (l.kind === 'Text') return false;
  const q = dec(l.quantity);
  if (l.amount === 0 && (!q || isZero(q))) return false;
  if (l.item_id && books.isSubtotalOrGroup(l.item_id)) return false;
  return true;
}

export interface LinkBooks {
  find(id: string): Txn | undefined;
  /** The customer or vendor a job rolls up to (`entities::root_of`). */
  rootOf(entityId: string): string;
  /** Whether the form's name is a customer (sales side) or a vendor. */
  isCustomerForm(t: TxnType): boolean;
}

const invalid = (message: string) => new ApiError(400, 'invalid', message);

/**
 * `orders::check_links`, before a transaction is saved: every linked line's
 * target must exist, be a type this form may draw on, belong to the same name
 * (a job rolls up to its customer), not be void, and (for a NEW link) neither
 * the order nor its line may be closed; an item line must link to a line with
 * the same item. `existing` is the saved version, whose links stay legal.
 */
export function checkLinks(txn: Txn, existing: Txn | null, books: LinkBooks): void {
  if (txn.txn_type === 'Deposit') return;
  const linked = txn.lines.filter((l) => l.linked_txn_id);
  if (!linked.length) return;
  const targets = linkTargets(txn.txn_type);
  if (!targets.length) throw invalid(`lines on a ${TXN_LABEL[txn.txn_type]} can't be entered against another transaction`);
  if (!txn.entity_id) throw invalid('choose a name before entering lines against an order');
  const root = books.rootOf(txn.entity_id);
  const previous = new Set((existing?.lines ?? []).filter((l) => l.linked_txn_id).map((l) => `${l.linked_txn_id}/${l.linked_line_id ?? ''}`));
  for (const line of linked) {
    const targetId = line.linked_txn_id!;
    if (targetId === txn.id) throw invalid("a line can't be entered against its own transaction");
    const target = books.find(targetId);
    if (!target) throw invalid("the transaction this line is entered against doesn't exist");
    const label = TXN_LABEL[target.txn_type];
    if (!targets.includes(target.txn_type)) throw invalid(`a ${TXN_LABEL[txn.txn_type]} line can't be entered against a ${label}`);
    if (!target.entity_id) throw invalid(`that ${label} has no name to match against`);
    if (books.rootOf(target.entity_id) !== root) throw invalid(`that ${label} belongs to another ${books.isCustomerForm(target.txn_type) ? 'customer' : 'vendor'}`);
    const isNew = !previous.has(`${targetId}/${line.linked_line_id ?? ''}`);
    if (isNew && target.is_void) throw invalid(`that ${label} is void`);
    if (isNew && isOrder(target.txn_type) && target.is_manually_closed) throw invalid(`that ${label} is closed`);
    let targetLine: TxnLine | undefined;
    if (line.linked_line_id) {
      targetLine = target.lines.find((l) => l.id === line.linked_line_id);
      if (!targetLine) throw invalid(`the line this one is entered against isn't on that ${label}`);
    } else targetLine = matchingLine(target, line);
    if (targetLine) {
      if (line.kind === 'Item' && targetLine.item_id !== line.item_id) throw invalid(`that line of the ${label} is for a different item`);
      if (isNew && isOrder(target.txn_type) && targetLine.is_closed) throw invalid(`that line of the ${label} is closed`);
    } else if (line.kind === 'Item') throw invalid(`that ${label} has no line for this item`);
  }
}
