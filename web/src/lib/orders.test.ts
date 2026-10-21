import { describe, expect, test } from 'bun:test';
import { blankLine, blankTxn } from '~/api/blank';
import type { Txn, TxnLine } from '~/api/types';
import { toPlainString } from './decimal';
import { isManuallyClosed, linkedOrderIds, orderOpenAmount, orderProgress, withManuallyClosed } from './orders';

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

function invoiceDrawing(id: string, date: string, draws: Array<[lineId: string, quantity: string | null, amount: number]>, isVoid = false): Txn {
  const inv = blankTxn('Invoice', date);
  inv.id = id;
  inv.is_void = isVoid;
  inv.lines = draws.map(([lineId, quantity, amount], i) => item(`${id}-${i}`, quantity, '0', amount, { linked_txn_id: 'so-1', linked_line_id: lineId }));
  return inv;
}

const plain = (d: { int: bigint; scale: number }) => toPlainString(d);

describe('order progress', () => {
  test('nothing drawn: every line is fully remaining and the order is open', () => {
    const p = orderProgress(salesOrder(), []);
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
    expect(p.linked).toEqual([]);
  });

  test('a partial invoice reduces the remaining quantities exactly; a void one counts for nothing', () => {
    const part = invoiceDrawing('inv-1', '2026-10-20', [['l1', '4', 7_400], ['l2', '0.5', 4_250]]);
    const voided = invoiceDrawing('inv-void', '2026-10-21', [['l1', '6', 11_100]], true);
    const p = orderProgress(salesOrder(), [voided, part]);
    expect(p.linked.map((t) => t.id)).toEqual(['inv-1']);
    const [walnut, design] = p.lines;
    expect([plain(walnut!.fulfilled), plain(walnut!.remaining)]).toEqual(['4', '6']);
    expect([plain(design!.fulfilled), plain(design!.remaining)]).toEqual(['0.5', '2.5']);
    expect(walnut!.amountFulfilled).toBe(7_400);
    expect(p.amountFulfilled).toBe(11_650);
    expect(plain(p.percent)).toBe('26.48');
    expect(p.closed).toBe(false);
  });

  test('several invoices add up; over-drawing clamps at zero and closes the order', () => {
    const a = invoiceDrawing('inv-1', '2026-10-20', [['l1', '4', 7_400], ['l2', '3', 25_500]]);
    const b = invoiceDrawing('inv-2', '2026-10-22', [['l1', '7', 12_950]]);
    const p = orderProgress(salesOrder(), [b, a]);
    expect(p.linked.map((t) => t.id)).toEqual(['inv-1', 'inv-2']); // oldest first
    expect(plain(p.byLineId.get('l1')!.fulfilled)).toBe('11');
    expect(plain(p.byLineId.get('l1')!.remaining)).toBe('0');
    expect(p.byLineId.get('l1')!.amountRemaining).toBe(0);
    expect(p.fullyFulfilled).toBe(true);
    expect(p.closed).toBe(true);
    expect(orderOpenAmount(salesOrder(), [a, b])).toBe(0);
  });

  test('a blank quantity counts as one, on the order and on what draws on it', () => {
    const so = salesOrder();
    so.lines = [item('l1', null, '75', 7_500)];
    expect(plain(orderProgress(so, []).lines[0]!.ordered)).toBe('1');
    const inv = invoiceDrawing('inv-1', today, [['l1', null, 7_500]]);
    const p = orderProgress(so, [inv]);
    expect(plain(p.lines[0]!.remaining)).toBe('0');
    expect(p.fullyFulfilled).toBe(true);
  });

  test('an estimate is measured by amount: half the money is 50%, and only the full amount closes it', () => {
    const est = { ...salesOrder(), txn_type: 'Estimate' as const };
    const half = invoiceDrawing('inv-1', today, [['l1', '10', 9_250], ['l2', '3', 12_750]]);
    const p = orderProgress(est, [half]);
    expect(plain(p.percent)).toBe('50.00');
    // By quantity both lines look fully drawn; by amount they are half done.
    expect(plain(p.lines[0]!.remaining)).toBe('0');
    expect(p.lines[0]!.amountRemaining).toBe(9_250);
    expect(p.fullyFulfilled).toBe(false);
    expect(p.closed).toBe(false);
    const rest = invoiceDrawing('inv-2', today, [['l1', '10', 9_250], ['l2', '3', 12_750]]);
    expect(orderProgress(est, [half, rest]).closed).toBe(true);
  });

  test('a manual close is kept in extra under QBD\'s tag and makes the order closed; reopening removes it', () => {
    const so = salesOrder();
    expect(isManuallyClosed(so)).toBe(false);
    const closed = withManuallyClosed({ ...so, extra: { Other: 'kept' } as Record<string, unknown> }, true);
    expect(closed.extra).toEqual({ Other: 'kept', IsManuallyClosed: true });
    expect(isManuallyClosed(closed)).toBe(true);
    expect(isManuallyClosed({ extra: { IsManuallyClosed: 'true' } })).toBe(true); // as an import would carry it
    const p = orderProgress(closed, []);
    expect(p.manuallyClosed).toBe(true);
    expect(p.fullyFulfilled).toBe(false);
    expect(p.closed).toBe(true);
    expect(orderOpenAmount(closed, [])).toBe(0);
    expect(withManuallyClosed(closed, false).extra).toEqual({ Other: 'kept' });
    expect(withManuallyClosed(withManuallyClosed(so, true), false).extra).toBeNull();
  });

  test('lines linked to another order or without a line id do not count', () => {
    const other = invoiceDrawing('inv-1', today, [['l1', '10', 18_500]]);
    other.lines[0]!.linked_txn_id = 'so-2';
    const whole = invoiceDrawing('inv-2', today, [['l1', '10', 18_500]]);
    whole.lines[0]!.linked_line_id = null;
    const p = orderProgress(salesOrder(), [other, whole]);
    expect(p.linked.map((t) => t.id)).toEqual(['inv-2']);
    expect(plain(p.lines[0]!.remaining)).toBe('10');
    expect(linkedOrderIds(whole)).toEqual(['so-1']);
    expect(linkedOrderIds(other)).toEqual(['so-2']);
  });
});
