import { describe, expect, test } from 'bun:test';
import { blankItem } from '~/api/blank';
import {
  blankExpense,
  blankItemRow,
  chooseItem,
  fromTxnLines,
  locateLine,
  setItemAmount,
  setQuantity,
  setRate,
  toTxnLines,
} from './purchaseLines';
import { withTrailingBlank } from '~/inputs/gridRows';
import { isBlankExpense } from './purchaseLines';

const walnut = { ...blankItem('Walnut', 'NonInventory'), purchase_cost: '12.75', purchase_description: 'Black walnut (bd ft)' };

describe('item line auto-fill', () => {
  test('choosing an item fills description, cost and amount (blank qty = 1)', () => {
    const r = chooseItem(blankItemRow(), walnut);
    expect(r.description).toBe('Black walnut (bd ft)');
    expect(r.rate).toBe('12.75');
    expect(r.amount).toBe(1275);
  });
  test('quantity × rate is exact and rounds half away from zero', () => {
    let r = chooseItem(blankItemRow(), walnut);
    r = setQuantity(r, '3');
    expect(r.amount).toBe(3825);
    r = setRate(setQuantity(r, '0.5'), '0.05');
    expect(r.amount).toBe(3); // 0.025 → 0.03
    r = setRate(setQuantity(r, '3'), '0.1');
    expect(r.amount).toBe(30); // a float would make 0.30000000000000004
  });
  test('editing the amount back-solves the rate and keeps the amount exact', () => {
    let r = setQuantity(chooseItem(blankItemRow(), walnut), '3');
    r = setItemAmount(r, 10000);
    expect(r.rate).toBe('33.33333');
    expect(r.amount).toBe(10000);
    r = setItemAmount(setQuantity(r, ''), 4250);
    expect(r.rate).toBe('42.50');
  });
  test('a bad quantity leaves the amount alone', () => {
    const r = setQuantity(chooseItem(blankItemRow(), walnut), 'abc');
    expect(r.amount).toBe(1275);
  });
});

describe('rows ↔ API lines', () => {
  test('round trip, blank rows dropped, ids kept', () => {
    const e = { ...blankExpense(), account_id: 'rent', amount: 185000, memo: 'October' };
    const i = setQuantity(chooseItem(blankItemRow(), walnut), '2');
    const lines = toTxnLines([e, blankExpense()], [i, blankItemRow()]);
    expect(lines).toHaveLength(2);
    expect(lines[0]).toMatchObject({ kind: 'Account', account_id: 'rent', amount: 185000, description: 'October' });
    expect(lines[1]).toMatchObject({ kind: 'Item', item_id: walnut.id, quantity: '2', rate: '12.75', amount: 2550 });
    const back = fromTxnLines(lines);
    expect(back.expenses[0]).toMatchObject({ account_id: 'rent', amount: 185000, memo: 'October', lineId: lines[0]!.id });
    expect(back.items[0]).toMatchObject({ item_id: walnut.id, quantity: '2', amount: 2550 });
  });
  test('billable only with a customer:job', () => {
    const e = { ...blankExpense(), account_id: 'x', amount: 1, billable: true };
    expect(toTxnLines([e], [])[0]!.billable).toBe('NotBillable');
    expect(toTxnLines([{ ...e, entity_id: 'job' }], [])[0]!.billable).toBe('Billable');
  });
  test('locating a server error line index in the tabs', () => {
    const e1 = { ...blankExpense(), account_id: 'a', amount: 1 };
    const e2 = { ...blankExpense(), account_id: 'b', amount: 2 };
    const it = { ...blankItemRow(), item_id: 'i', amount: 3 };
    const expenses = [e1, blankExpense(), e2];
    expect(locateLine(expenses, [it], 1)).toEqual({ tab: 'expenses', row: 2 });
    expect(locateLine(expenses, [it], 2)).toEqual({ tab: 'items', row: 0 });
    expect(locateLine(expenses, [it], 3)).toBeNull();
  });
});

describe('trailing blank line', () => {
  test('exactly one blank at the end', () => {
    const filled = { ...blankExpense(), account_id: 'a' };
    expect(withTrailingBlank([], isBlankExpense, blankExpense)).toHaveLength(1);
    expect(withTrailingBlank([filled], isBlankExpense, blankExpense)).toHaveLength(2);
    const many = withTrailingBlank([filled, blankExpense(), blankExpense(), blankExpense()], isBlankExpense, blankExpense);
    expect(many).toHaveLength(2);
    // A blank line the user inserted in the middle stays.
    const mid = withTrailingBlank([filled, blankExpense(), filled], isBlankExpense, blankExpense);
    expect(mid).toHaveLength(4);
  });
});
