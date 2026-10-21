import { describe, expect, test } from 'bun:test';
import { blankLine, blankTxn } from '~/api/blank';
import type { Txn, TxnLine } from '~/api/types';
import { toPlainString } from '~/lib/decimal';
import { orderProgress } from '~/lib/orders';
import { amountToPercent, documentFromOrder, drawAmounts, drawPercent, drawPercentOfTotal, drawQuantities, drawRemaining, drawTotal, drawnLines, percentToAmount } from './linked';

const today = '2026-10-19';

function item(id: string, quantity: string | null, rate: string | null, amount: number, extra: Partial<TxnLine> = {}): TxnLine {
  return { ...blankLine('Item'), id, item_id: `item-${id}`, description: `Line ${id}`, quantity, rate, amount, sales_tax_code_id: 'tax', ...extra };
}

function order(type: Txn['txn_type'] = 'SalesOrder'): Txn {
  const o = blankTxn(type, today);
  o.id = 'ord';
  o.entity_id = 'cust';
  o.class_id = 'shop';
  o.terms_id = 'net30';
  o.po_number = 'PO-9';
  o.sales_tax_item_id = 'taxitem';
  o.customer_tax_code_id = 'tax';
  o.memo = 'Lobby';
  o.bill_address = { lines: ['Acme', '1 Main'], city: 'Juniper', state: 'OR', postal_code: '97000', country: null, note: null };
  o.lines = [item('a', '10', '18.50', 18_500), item('b', '3', '85', 25_500), item('c', null, null, 7_500), { ...blankLine('Text'), id: 'txt' }];
  return o;
}

const drawn = (id: string, quantity: string, amount: number): Txn => {
  const t = blankTxn('Invoice', today);
  t.id = `inv-${id}`;
  t.lines = [item(`x-${id}`, quantity, null, amount, { linked_txn_id: 'ord', linked_line_id: id })];
  return t;
};

const q = (d: { int: bigint; scale: number }) => toPlainString(d);

describe('drawing on an order', () => {
  test('the remaining draw takes what earlier documents left', () => {
    const p = orderProgress(order(), [drawn('a', '4', 7_400)]);
    const d = drawRemaining(p);
    expect([...d.entries()].map(([id, v]) => [id, q(v.quantity), v.amount])).toEqual([
      ['a', '6', 11_100],
      ['b', '3', 25_500],
      ['c', '1', 7_500],
    ]);
    expect(drawTotal(d)).toBe(44_100);
    // Fully drawn lines are left out.
    expect(drawRemaining(orderProgress(order(), [drawn('a', '10', 18_500), drawn('b', '3', 25_500), drawn('c', '1', 7_500)])).size).toBe(0);
  });

  test('typed quantities: amount follows the rate, or the share of a flat line; blanks and zeros take nothing', () => {
    const p = orderProgress(order(), []);
    const d = drawQuantities(p, new Map([['a', '2.5'], ['b', '0'], ['c', '0.5'], ['zzz', '9']]));
    expect(d.get('a')).toEqual({ quantity: { int: 25n, scale: 1 }, amount: 4_625 });
    expect(d.has('b')).toBe(false);
    // 7,500 × 0.5 / 1: half the flat charge.
    expect(d.get('c')).toEqual({ quantity: { int: 5n, scale: 1 }, amount: 3_750 });
    expect(drawQuantities(p, new Map([['a', 'ten']])).size).toBe(0);
  });

  test('a percent of the whole estimate: each line rounded on its own, quantities kept', () => {
    const p = orderProgress(order('Estimate'), []);
    const d = drawPercent(p, '50');
    expect(d.get('a')).toEqual({ quantity: { int: 10n, scale: 0 }, amount: 9_250 });
    expect(d.get('b')!.amount).toBe(12_750);
    expect(d.get('c')!.amount).toBe(3_750);
    expect(q(drawPercentOfTotal(p, d))).toBe('50.00');
    // 33.33% of 18,500 is 6,166.05 → 6,166 cents (half away from zero on the third place).
    expect(drawPercent(p, '33.33').get('a')!.amount).toBe(6_166);
    expect(drawPercent(p, '0').size).toBe(0);
    expect(drawPercent(p, '-5').size).toBe(0);
  });

  test('amounts per line, with the percent column derived both ways', () => {
    const p = orderProgress(order('Estimate'), []);
    const a = p.byLineId.get('a')!;
    expect(amountToPercent(a, 4_625)).toBe('25');
    expect(amountToPercent(a, 6_166)).toBe('33.33');
    expect(percentToAmount(a, '25')).toBe(4_625);
    expect(amountToPercent({ ...a, amount: 0 }, 100)).toBeNull();
    const d = drawAmounts(p, new Map([['a', 4_625], ['b', null], ['c', 0]]));
    expect([...d.keys()]).toEqual(['a']);
    expect(q(drawPercentOfTotal(p, d))).toBe('8.98'); // 4,625 of 51,500
  });

  test('drawn lines link back to the order line and carry its item, rate, class and tax code', () => {
    const o = order();
    o.lines[0]!.class_id = 'field';
    o.lines[0]!.entity_id = 'job';
    o.lines[0]!.billable = 'Billable';
    const p = orderProgress(o, []);
    const lines = drawnLines(o, p, drawQuantities(p, new Map([['a', '4'], ['c', '1']])));
    expect(lines).toHaveLength(2);
    expect(lines[0]).toMatchObject({ kind: 'Item', item_id: 'item-a', description: 'Line a', quantity: '4', rate: '18.50', amount: 7_400, class_id: 'field', entity_id: 'job', billable: 'Billable', sales_tax_code_id: 'tax', linked_txn_id: 'ord', linked_line_id: 'a' });
    expect(lines[0]!.id).not.toBe('a');
    expect(lines[1]).toMatchObject({ item_id: 'item-c', quantity: '1', rate: null, amount: 7_500, linked_line_id: 'c' });
  });

  test('an invoice from a sales order brings the customer, class, terms, addresses, tax and P.O. number; a receipt from a PO the vendor', () => {
    const so = order();
    const p = orderProgress(so, []);
    const inv = documentFromOrder(so, p, drawRemaining(p), 'Invoice', '2026-10-20');
    expect(inv).toMatchObject({ txn_type: 'Invoice', date: '2026-10-20', edit_seq: 0, entity_id: 'cust', class_id: 'shop', terms_id: 'net30', po_number: 'PO-9', sales_tax_item_id: 'taxitem', customer_tax_code_id: 'tax', memo: 'Lobby', amount: 51_500 });
    expect(inv.bill_address.lines).toEqual(['Acme', '1 Main']);
    expect(inv.lines.every((l) => l.linked_txn_id === 'ord')).toBe(true);
    expect(inv.id).not.toBe(so.id);
    const po = order('PurchaseOrder');
    po.entity_id = 'vendor';
    const pp = orderProgress(po, []);
    const receipt = documentFromOrder(po, pp, drawRemaining(pp), 'ItemReceipt', today);
    expect(receipt).toMatchObject({ txn_type: 'ItemReceipt', entity_id: 'vendor', class_id: 'shop', terms_id: null, po_number: null, sales_tax_item_id: null, amount: 51_500 });
  });
});
