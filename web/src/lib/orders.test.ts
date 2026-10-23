import { describe, expect, test } from 'bun:test';
import { blankLine, blankTxn } from '~/api/blank';
import type { Fulfilment, LineFulfilment, Txn, TxnLine, TxnSummary } from '~/api/types';
import { toPlainString } from './decimal';
import { linkedOrderIds, orderKind, progressFromFulfilment } from './orders';

const today = '2026-10-19';

function item(id: string, quantity: string | null, rate: string, amount: number, extra: Partial<TxnLine> = {}): TxnLine {
  return { ...blankLine('Item'), id, item_id: `item-${id}`, quantity, rate, amount, ...extra };
}

/** A sales order of 10 walnut at 18.50 and 3 design hours at 85, plus a text line. */
function salesOrder(): Txn {
  const so = blankTxn('SalesOrder', today);
  so.id = 'so-1';
  so.lines = [item('l1', '10', '18.50', 18_500), item('l2', '3', '85', 25_500), { ...blankLine('Text'), id: 't', description: 'Thanks' }];
  return so;
}

/** A server `LineFulfilment`, as `GET /txns/:id/fulfilment` answers it. */
function lf(line_id: string, f: Partial<LineFulfilment> & Pick<LineFulfilment, 'ordered_amount'>): LineFulfilment {
  return { line_id, item_id: `item-${line_id}`, ordered_quantity: null, fulfilled_quantity: '0', fulfilled_amount: 0, is_closed: false, remaining_quantity: null, remaining_amount: f.ordered_amount, ...f };
}

const summary = (id: string, date: string): TxnSummary => ({ id, txn_type: 'Invoice', date, number: id, entity_id: 'c', entity_name: null, account_id: null, account_name: null, amount: 0, open_balance: 0, due_date: null, memo: null, is_void: false, is_pending: false, edit_seq: 1 });

const plain = (d: { int: bigint; scale: number }) => toPlainString(d);

describe('order progress from the server fulfilment', () => {
  test('nothing drawn: every line is fully remaining and the order is open', () => {
    const f: Fulfilment = {
      order_id: 'so-1',
      is_closed: false,
      remaining_amount: 44_000,
      lines: [lf('l1', { ordered_quantity: '10', ordered_amount: 18_500, remaining_quantity: '10' }), lf('l2', { ordered_quantity: '3', ordered_amount: 25_500, remaining_quantity: '3' })],
    };
    const p = progressFromFulfilment(salesOrder(), f);
    expect(p.lines.map((l) => [plain(l.ordered), plain(l.fulfilled), plain(l.remaining)])).toEqual([
      ['10', '0', '10'],
      ['3', '0', '3'],
    ]);
    expect(p.amount).toBe(44_000);
    expect(p.amountFulfilled).toBe(0);
    expect(p.amountRemaining).toBe(44_000);
    expect(plain(p.percent)).toBe('0.00');
    expect(p.closed).toBe(false);
    expect(p.fullyFulfilled).toBe(false);
    expect(p.manuallyClosed).toBe(false);
    expect(p.linked).toEqual([]);
  });

  test('partly drawn: the figures are the server\'s, the linked documents come oldest first, a closed line is marked', () => {
    const f: Fulfilment = {
      order_id: 'so-1',
      is_closed: false,
      remaining_amount: 11_100,
      lines: [
        lf('l1', { ordered_quantity: '10', fulfilled_quantity: '4', ordered_amount: 18_500, fulfilled_amount: 7_400, remaining_quantity: '6', remaining_amount: 11_100 }),
        lf('l2', { ordered_quantity: '3', fulfilled_quantity: '0.5', ordered_amount: 25_500, fulfilled_amount: 4_250, is_closed: true, remaining_quantity: '0', remaining_amount: 0 }),
      ],
    };
    const p = progressFromFulfilment(salesOrder(), f, [summary('inv-2', '2026-10-22'), summary('inv-1', '2026-10-20')]);
    expect(p.linked.map((t) => t.id)).toEqual(['inv-1', 'inv-2']);
    const [walnut, design] = p.lines;
    expect([plain(walnut!.fulfilled), plain(walnut!.remaining), walnut!.closed]).toEqual(['4', '6', false]);
    expect([plain(design!.fulfilled), plain(design!.remaining), design!.closed]).toEqual(['0.5', '0', true]);
    expect(p.amountFulfilled).toBe(11_650);
    expect(p.amountRemaining).toBe(11_100);
    expect(plain(p.percent)).toBe('26.48');
    expect(p.byLineId.get('l2')!.amountRemaining).toBe(0);
    expect(p.closed).toBe(false);
  });

  test('closed by its lines is "fully fulfilled"; closed by hand is "manually closed"; both are closed', () => {
    const lines = [lf('l1', { ordered_quantity: '10', fulfilled_quantity: '11', ordered_amount: 18_500, fulfilled_amount: 20_350, is_closed: true, remaining_quantity: '0' , remaining_amount: 0 }), lf('l2', { ordered_quantity: '3', ordered_amount: 25_500, is_closed: true, remaining_quantity: '0', remaining_amount: 0 })];
    const byLines = progressFromFulfilment(salesOrder(), { order_id: 'so-1', is_closed: true, remaining_amount: 0, lines });
    expect(byLines.fullyFulfilled).toBe(true);
    expect(byLines.manuallyClosed).toBe(false);
    expect(byLines.closed).toBe(true);
    expect(byLines.amountRemaining).toBe(0);
    const stamped = progressFromFulfilment({ ...salesOrder(), is_manually_closed: true }, { order_id: 'so-1', is_closed: true, remaining_amount: 0, lines });
    expect(stamped.fullyFulfilled).toBe(false);
    expect(stamped.manuallyClosed).toBe(true);
    expect(stamped.closed).toBe(true);
  });

  test('a line without a quantity is one of something: its whole remains until drawn or closed', () => {
    const so = salesOrder();
    so.lines = [item('l1', null, '75', 7_500)];
    const open = progressFromFulfilment(so, { order_id: 'so-1', is_closed: false, remaining_amount: 7_500, lines: [lf('l1', { ordered_amount: 7_500 })] });
    expect([plain(open.lines[0]!.ordered), plain(open.lines[0]!.remaining)]).toEqual(['1', '1']);
    const half = progressFromFulfilment(so, { order_id: 'so-1', is_closed: false, remaining_amount: 3_750, lines: [lf('l1', { ordered_amount: 7_500, fulfilled_amount: 3_750, remaining_amount: 3_750 })] });
    expect(plain(half.lines[0]!.remaining)).toBe('1');
    expect(half.lines[0]!.amountRemaining).toBe(3_750);
    const done = progressFromFulfilment(so, { order_id: 'so-1', is_closed: true, remaining_amount: 0, lines: [lf('l1', { ordered_amount: 7_500, fulfilled_amount: 7_500, is_closed: true, remaining_amount: 0 })] });
    expect(plain(done.lines[0]!.remaining)).toBe('0');
    expect(done.closed).toBe(true);
  });

  test('a figure for a line the order no longer has is dropped; the open kinds map to the API\'s', () => {
    const p = progressFromFulfilment(salesOrder(), { order_id: 'so-1', is_closed: false, remaining_amount: 0, lines: [lf('gone', { ordered_amount: 100 })] });
    expect(p.lines).toEqual([]);
    expect(p.amount).toBe(0);
    expect(plain(p.percent)).toBe('0');
    expect([orderKind('PurchaseOrder'), orderKind('SalesOrder'), orderKind('Estimate')]).toEqual(['purchase_orders', 'sales_orders', 'estimates']);
  });

  test('the distinct transactions a document draws on, in line order', () => {
    const inv = blankTxn('Invoice', today);
    inv.lines = [item('a', '1', '1', 100, { linked_txn_id: 'so-2', linked_line_id: 'x' }), item('b', '1', '1', 100, { linked_txn_id: 'so-1', linked_line_id: 'y' }), item('c', '1', '1', 100, { linked_txn_id: 'so-2', linked_line_id: 'z' })];
    expect(linkedOrderIds(inv)).toEqual(['so-2', 'so-1']);
    expect(linkedOrderIds(blankTxn('Invoice', today))).toEqual([]);
  });
});
