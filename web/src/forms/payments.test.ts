import { describe, expect, test } from 'bun:test';
import { ApiError } from '~/api/errors';
import type { Terms, TxnSummary } from '~/api/types';
import { appliedCash, autoApply, availableFor, newPaymentDraft, paymentDraftFromTxn, paymentDraftToTxn, paymentServerErrors, setPaymentAmount, setRowCredit, setRowDiscount, syncTargets, toggleRow, unapplied, validatePayment } from './paymentDraft';
import { billRows, buildBillPayments, filterRows, payBillsTotals, selectAll, selectRow, setRowAmount, setRowCredit as setBillCredit, setRowDiscount as setBillDiscount, suggestedDiscount, validatePayBills } from './billPayments';

const today = '2026-10-19';
const inv = (id: string, date: string, amount: number, open = amount, entity = 'acme', type: TxnSummary['txn_type'] = 'Invoice'): TxnSummary => ({
  id,
  txn_type: type,
  date,
  number: id,
  entity_id: entity,
  entity_name: 'Acme',
  account_id: null,
  account_name: null,
  amount,
  open_balance: open,
  due_date: date,
  memo: null,
  is_void: false,
  is_pending: false,
  edit_seq: 1,
});

describe('receive payments', () => {
  const targets = [inv('i2', '2026-09-01', 50_000), inv('i1', '2026-08-01', 30_000)];
  test('targets sync oldest first with amount 0; auto-apply fills them in order', () => {
    let d = syncTargets({ ...newPaymentDraft('ReceivePayment', today, null), entity_id: 'acme' }, targets);
    expect(d.rows.map((r) => r.target_txn_id)).toEqual(['i1', 'i2']);
    d = setPaymentAmount(d, 60_000, true);
    expect(d.rows.map((r) => r.amount)).toEqual([30_000, 30_000]);
    expect(unapplied(d)).toBe(0);
    d = setPaymentAmount(d, 90_000, true);
    // Already applied by hand (or auto): the amount changes, the applications stay.
    expect(appliedCash(d)).toBe(60_000);
    expect(unapplied(d)).toBe(30_000);
    expect(autoApply(d).rows.map((r) => r.amount)).toEqual([30_000, 50_000]);
  });
  test('ticking a row takes what it needs, or what is left; untick takes it back', () => {
    let d = syncTargets({ ...newPaymentDraft('ReceivePayment', today, null), entity_id: 'acme' }, targets);
    d = toggleRow(d, 1, true);
    // No amount typed yet: the payment becomes what was ticked.
    expect(d.amount).toBe(50_000);
    expect(d.rows[1]!.amount).toBe(50_000);
    d = toggleRow(d, 1, true);
    expect(d.rows[1]!.amount).toBe(0);
    d = setPaymentAmount({ ...d, rows: d.rows.map((r) => ({ ...r, amount: 0 })) }, 10_000, false);
    d = toggleRow(d, 0, false);
    expect(d.rows[0]!.amount).toBe(10_000);
  });
  test('discounts and credits reduce what cash is needed; validation guards the totals', () => {
    let d = syncTargets({ ...newPaymentDraft('ReceivePayment', today, null), entity_id: 'acme' }, targets);
    d = setRowDiscount(d, 0, 600, 'disc');
    d = setRowCredit(d, 0, 'cm1', 4_000);
    d = setPaymentAmount(d, 25_400, true);
    expect(d.rows[0]!.amount).toBe(25_400);
    expect(validatePayment(d).header.amount).toBeUndefined();
    const over = { ...d, rows: d.rows.map((r, i) => (i === 0 ? { ...r, amount: 40_000 } : r)) };
    expect(validatePayment(over).header.amount).toContain('more than');
    expect(validatePayment(over).grids.rows![0]!.amount).toContain('open');
    expect(validatePayment(setRowDiscount(d, 0, 100, null)).grids.rows![0]!.discount).toBeDefined();
    const txn = paymentDraftToTxn(d);
    expect(txn.applications).toEqual([{ target_txn_id: 'i1', amount: 25_400, discount: 600, discount_account_id: 'disc', discount_class_id: null, credit_txn_id: 'cm1', credit_amount: 4_000 }]);
  });
  test('editing a saved payment keeps its applications even once the invoice is paid off', () => {
    const saved = paymentDraftToTxn(setPaymentAmount(syncTargets({ ...newPaymentDraft('ReceivePayment', today, null), entity_id: 'acme' }, targets), 30_000, true));
    saved.edit_seq = 2;
    let d = paymentDraftFromTxn(saved);
    expect(d.rows).toHaveLength(1);
    // The invoice is no longer open (this payment paid it), but it's still this payment's target.
    d = syncTargets(d, [inv('i1', '2026-08-01', 30_000, 0), inv('i2', '2026-09-01', 50_000)]);
    expect(d.rows.map((r) => r.target_txn_id)).toEqual(['i1', 'i2']);
    expect(availableFor(d, d.rows[0]!)).toBe(30_000);
  });
  test('server errors: an unknown target marks its row; over-applied marks the amount', () => {
    const d = syncTargets({ ...newPaymentDraft('ReceivePayment', today, null), entity_id: 'acme', amount: 100 }, targets);
    const sent = paymentDraftToTxn(d);
    expect(paymentServerErrors(d, sent, new ApiError(422, 'posting', 'x', { code: 'UnknownTarget', detail: 'i2' })).grids.rows![1]!.amount).toBeDefined();
    expect(paymentServerErrors(d, sent, new ApiError(422, 'posting', 'x', { code: 'OverApplied', detail: { applied: 2, amount: 1 } })).header.amount).toBeDefined();
  });
});

describe('pay bills', () => {
  const bills = [inv('b1', '2026-09-01', 100_000, 100_000, 'bob', 'Bill'), inv('b2', '2026-09-15', 40_000, 40_000, 'bob', 'Bill'), inv('b3', '2026-09-20', 25_000, 25_000, 'hardwood', 'Bill')];
  const twoTen: Terms = { id: 't', name: '2% 10 Net 30', is_active: true, kind: { type: 'Standard', due_days: 30, discount_days: 10, discount_percent: '2' }, edit_seq: 1 };
  test('rows, selection, totals and filtering by due date', () => {
    let rows = billRows(bills);
    expect(rows).toHaveLength(3);
    rows = selectRow(rows, 'b1', true);
    expect(payBillsTotals(rows)).toEqual({ selected: 1, open: 100_000, discount: 0, credit: 0, cash: 100_000 });
    rows = setBillDiscount(rows, 'b1', 2_000);
    rows = setBillCredit(rows, 'b2', 'vc1', 10_000);
    expect(payBillsTotals(rows)).toEqual({ selected: 2, open: 140_000, discount: 2_000, credit: 10_000, cash: 128_000 });
    rows = setRowAmount(rows, 'b3', 5_000);
    expect(rows[2]!.amount).toBe(5_000);
    expect(filterRows(selectAll(rows, false), '2026-09-10').map((r) => r.bill.id)).toEqual(['b1']);
    expect(suggestedDiscount(bills[0]!, twoTen, '2026-09-05')).toBe(2_000);
    expect(suggestedDiscount(bills[0]!, twoTen, '2026-09-20')).toBe(0);
  });
  test('one payment per vendor, numbered in sequence, with the applications', () => {
    let rows = selectAll(billRows(bills), true);
    rows = setBillDiscount(rows, 'b1', 2_000);
    expect(validatePayBills(rows, { account_id: 'bank', discount_account_id: null })).toContain('discounts');
    const txns = buildBillPayments(rows, { date: today, account_id: 'bank', account_type: 'Bank', number: '1090', discount_account_id: 'disc', memo: null });
    expect(txns).toHaveLength(2);
    expect(txns[0]).toMatchObject({ txn_type: 'BillPaymentCheck', entity_id: 'bob', number: '1090', amount: 138_000 });
    expect(txns[0]!.applications).toHaveLength(2);
    expect(txns[0]!.applications[0]).toMatchObject({ target_txn_id: 'b1', amount: 98_000, discount: 2_000, discount_account_id: 'disc' });
    expect(txns[1]).toMatchObject({ entity_id: 'hardwood', number: '1091', amount: 25_000 });
    const card = buildBillPayments(rows, { date: today, account_id: 'visa', account_type: 'CreditCard', number: null, discount_account_id: 'disc', memo: 'paid' });
    expect(card[0]).toMatchObject({ txn_type: 'BillPaymentCreditCard', number: null, is_to_be_printed: false, memo: 'paid' });
    const printed = buildBillPayments(rows, { date: today, account_id: 'bank', account_type: 'Bank', number: null, discount_account_id: 'disc', memo: null });
    expect(printed[0]!.is_to_be_printed).toBe(true);
  });
});
