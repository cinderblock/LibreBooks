import { describe, expect, test } from 'bun:test';
import { blankEntity, blankLine, blankTxn } from '~/api/blank';
import { ApiError } from '~/api/errors';
import type { Terms } from '~/api/types';
import {
  applyPayee,
  applyTerms,
  isPurchaseEmpty,
  newPurchaseDraft,
  purchaseDraftFromTxn,
  purchaseDraftToTxn,
  purchaseFingerprint,
  purchaseServerErrors,
  purchaseTotal,
  setHeaderAmount,
  validatePurchase,
} from './purchaseDraft';

const today = '2026-10-19';
const net30: Terms = { id: 't30', name: 'Net 30', is_active: true, kind: { type: 'Standard', due_days: 30, discount_days: null, discount_percent: null }, edit_seq: 1 };

describe('header amount', () => {
  test('goes on the first line when the lines are empty', () => {
    const d = setHeaderAmount(newPurchaseDraft('Check', today, 'bank'), 12_500);
    expect(d.expenses[0]!.amount).toBe(12_500);
    expect(purchaseTotal(d)).toBe(12_500);
    expect(d.expenses).toHaveLength(2); // the trailing blank line came back
  });
  test('puts the difference on the trailing blank line', () => {
    let d = newPurchaseDraft('Bill', today, null);
    d = { ...d, expenses: [{ ...d.expenses[0]!, account_id: 'rent', amount: 10_000 }] };
    d = setHeaderAmount(d, 15_000);
    expect(d.expenses.map((r) => r.amount)).toEqual([10_000, 5_000, null]);
    expect(purchaseTotal(d)).toBe(15_000);
  });
  test('a purchase order has item lines only, so the amount goes there', () => {
    const d = setHeaderAmount(newPurchaseDraft('PurchaseOrder', today, null), 9_900);
    expect(d.items[0]!.amount).toBe(9_900);
    expect(d.expenses.every((r) => r.amount === null)).toBe(true);
  });
});

describe('payee', () => {
  const vendor = { ...blankEntity('Pacific Power', 'Vendor'), default_expense_account_id: 'electric', terms_id: 't30' };
  test("prefills the vendor's default expense account", () => {
    const d = applyPayee(newPurchaseDraft('Check', today, 'bank'), vendor, null);
    expect(d.entity_id).toBe(vendor.id);
    expect(d.expenses[0]!.account_id).toBe('electric');
  });
  test("a bill also takes the vendor's terms and computes the due date", () => {
    const d = applyPayee(newPurchaseDraft('Bill', today, null), vendor, null, net30);
    expect(d.terms_id).toBe('t30');
    expect(d.due_date).toBe('2026-11-18');
  });
  test('recalls the last transaction for the name', () => {
    const last = blankTxn('Check', '2026-09-01');
    last.memo = 'Acct 5521';
    last.lines = [{ ...blankLine('Account'), account_id: 'electric', amount: 23_456 }];
    const d = applyPayee(newPurchaseDraft('Check', today, 'bank'), vendor, last);
    expect(d.memo).toBe('Acct 5521');
    expect(d.expenses[0]).toMatchObject({ account_id: 'electric', amount: 23_456, lineId: null });
    expect(d.expenses[0]!.key).not.toBe(last.lines[0]!.id);
  });
  test('leaves a form that already has lines alone', () => {
    let d = newPurchaseDraft('Check', today, 'bank');
    d = { ...d, expenses: [{ ...d.expenses[0]!, account_id: 'rent', amount: 1 }] };
    expect(applyPayee(d, vendor, null).expenses[0]!.account_id).toBe('rent');
  });
  test('changing terms recomputes the due date from the date', () => {
    const d = applyTerms({ ...newPurchaseDraft('Bill', '2026-10-01', null) }, net30);
    expect(d.due_date).toBe('2026-10-31');
    expect(applyTerms(d, null).terms_id).toBeNull();
  });
});

describe('validation', () => {
  test('a check needs a bank account and a line', () => {
    const e = validatePurchase(newPurchaseDraft('Check', today, null));
    expect(e.header.account).toBeDefined();
    expect(e.form).toHaveLength(1);
  });
  test('a bill needs a vendor, not an account', () => {
    const e = validatePurchase(newPurchaseDraft('Bill', today, null));
    expect(e.header.entity).toBe('Choose a vendor');
    expect(e.header.account).toBeUndefined();
  });
  test('a card charge needs the card', () => {
    expect(validatePurchase(newPurchaseDraft('CreditCardCharge', today, null)).header.account).toContain('credit card');
  });
  test('a purchase order refuses expense lines', () => {
    let d = newPurchaseDraft('PurchaseOrder', today, null);
    d = { ...d, entity_id: 'v', expenses: [{ ...d.expenses[0]!, account_id: 'rent', amount: 1 }] };
    expect(validatePurchase(d).form.join(' ')).toContain('item lines only');
  });
  test('line problems are reported on the cell', () => {
    let d = newPurchaseDraft('Check', today, 'bank');
    d = { ...d, expenses: [{ ...d.expenses[0]!, amount: 500 }] };
    expect(validatePurchase(d).grids.expenses![0]!.account).toBeDefined();
  });
});

describe('to and from Txn', () => {
  test('round trip keeps ids, edit_seq and unseen fields', () => {
    const t = blankTxn('Check', today);
    t.account_id = 'bank';
    t.number = '1042';
    t.edit_seq = 3;
    t.po_number = 'kept';
    t.lines = [{ ...blankLine('Account'), account_id: 'rent', amount: 185_000 }];
    const d = purchaseDraftFromTxn(t);
    const back = purchaseDraftToTxn(d);
    expect(back).toMatchObject({ id: t.id, edit_seq: 3, number: '1042', po_number: 'kept', amount: 185_000, txn_type: 'Check' });
    expect(back.lines[0]!.id).toBe(t.lines[0]!.id);
  });
  test('a bill keeps its terms and due date; a check never sends them', () => {
    const bill = applyTerms({ ...newPurchaseDraft('Bill', today, null), entity_id: 'v' }, net30);
    expect(purchaseDraftToTxn(bill)).toMatchObject({ txn_type: 'Bill', account_id: null, terms_id: 't30', due_date: '2026-11-18' });
    const check = { ...newPurchaseDraft('Check', today, 'bank'), terms_id: 't30', due_date: today };
    expect(purchaseDraftToTxn(check).terms_id).toBeNull();
  });
  test('to be printed clears the number', () => {
    const d = { ...newPurchaseDraft('Check', today, 'bank'), number: '1001', to_be_printed: true };
    expect(purchaseDraftToTxn(d).number).toBeNull();
  });
  test('fingerprint ignores row keys but sees edits; emptiness', () => {
    const d = newPurchaseDraft('Check', today, 'bank');
    const same = { ...d, expenses: d.expenses.map((r) => ({ ...r, key: 'other' })) };
    expect(purchaseFingerprint(same)).toBe(purchaseFingerprint(d));
    expect(purchaseFingerprint({ ...d, memo: 'x' })).not.toBe(purchaseFingerprint(d));
    expect(isPurchaseEmpty(d)).toBe(true);
    expect(isPurchaseEmpty({ ...d, memo: 'x' })).toBe(false);
  });
});

describe('server errors land on fields', () => {
  test('an inactive account marks the line using it', () => {
    let d = newPurchaseDraft('Check', today, 'bank');
    d = { ...d, expenses: [{ ...d.expenses[0]!, account_id: 'old', amount: 100 }] };
    const sent = purchaseDraftToTxn(d);
    const e = purchaseServerErrors(d, sent, new ApiError(422, 'posting', 'x', { code: 'InactiveAccount', detail: 'old' }));
    expect(e.grids.expenses![0]!.account).toContain('inactive');
  });
  test('a closed period marks the date; a missing vendor marks the name', () => {
    const d = newPurchaseDraft('Bill', today, null);
    const e = purchaseServerErrors(d, purchaseDraftToTxn(d), new ApiError(423, 'closed_period', 'x', { closing_date: '2026-09-30' }));
    expect(e.header.date).toContain('2026-09-30');
    const m = purchaseServerErrors(d, purchaseDraftToTxn(d), new ApiError(422, 'posting', 'x', { code: 'MissingEntity' }));
    expect(m.header.entity).toBe('Choose a name.');
  });
});
