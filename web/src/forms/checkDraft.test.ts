import { describe, expect, test } from 'bun:test';
import { blankEntity, blankLine, blankTxn } from '~/api/blank';
import { ApiError } from '~/api/errors';
import {
  applyPayee,
  checkTotal,
  draftFingerprint,
  draftFromTxn,
  draftToTxn,
  newCheckDraft,
  serverErrors,
  setHeaderAmount,
  validateCheck,
} from './checkDraft';

const today = '2026-10-19';

describe('header amount', () => {
  test('goes on the first line when the lines are empty', () => {
    const d = setHeaderAmount(newCheckDraft(today, 'bank'), 12_500);
    expect(d.expenses[0]!.amount).toBe(12_500);
    expect(checkTotal(d)).toBe(12_500);
    expect(d.expenses).toHaveLength(2); // the trailing blank line came back
  });
  test('puts the difference on the trailing blank line', () => {
    let d = newCheckDraft(today, 'bank');
    d = { ...d, expenses: [{ ...d.expenses[0]!, account_id: 'rent', amount: 10_000 }] };
    d = setHeaderAmount(d, 15_000);
    expect(d.expenses.map((r) => r.amount)).toEqual([10_000, 5_000, null]);
    expect(checkTotal(d)).toBe(15_000);
  });
});

describe('payee', () => {
  const vendor = { ...blankEntity('Pacific Power', 'Vendor'), default_expense_account_id: 'electric' };
  test("prefills the vendor's default expense account", () => {
    const d = applyPayee(newCheckDraft(today, 'bank'), vendor, null);
    expect(d.entity_id).toBe(vendor.id);
    expect(d.expenses[0]!.account_id).toBe('electric');
  });
  test('recalls the last transaction for the name', () => {
    const last = blankTxn('Check', '2026-09-01');
    last.memo = 'Acct 5521';
    last.lines = [{ ...blankLine('Account'), account_id: 'electric', amount: 23_456 }];
    const d = applyPayee(newCheckDraft(today, 'bank'), vendor, last);
    expect(d.memo).toBe('Acct 5521');
    expect(d.expenses[0]).toMatchObject({ account_id: 'electric', amount: 23_456, lineId: null });
    expect(d.expenses[0]!.key).not.toBe(last.lines[0]!.id);
  });
  test('leaves a check that already has lines alone', () => {
    let d = newCheckDraft(today, 'bank');
    d = { ...d, expenses: [{ ...d.expenses[0]!, account_id: 'rent', amount: 1 }] };
    expect(applyPayee(d, vendor, null).expenses[0]!.account_id).toBe('rent');
  });
});

describe('validation', () => {
  test('needs a bank account and a line', () => {
    const e = validateCheck(newCheckDraft(today, null));
    expect(e.header.account).toBeDefined();
    expect(e.form).toHaveLength(1);
  });
  test('line problems are reported on the cell', () => {
    let d = newCheckDraft(today, 'bank');
    d = { ...d, expenses: [{ ...d.expenses[0]!, amount: 500 }] };
    expect(validateCheck(d).expenses[0]!.account).toBeDefined();
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
    const d = draftFromTxn(t);
    const back = draftToTxn(d);
    expect(back).toMatchObject({ id: t.id, edit_seq: 3, number: '1042', po_number: 'kept', amount: 185_000 });
    expect(back.lines[0]!.id).toBe(t.lines[0]!.id);
  });
  test('to be printed clears the number', () => {
    const d = { ...newCheckDraft(today, 'bank'), number: '1001', to_be_printed: true };
    expect(draftToTxn(d).number).toBeNull();
  });
  test('fingerprint ignores row keys but sees edits', () => {
    const d = newCheckDraft(today, 'bank');
    const same = { ...d, expenses: d.expenses.map((r) => ({ ...r, key: 'other' })) };
    expect(draftFingerprint(same)).toBe(draftFingerprint(d));
    expect(draftFingerprint({ ...d, memo: 'x' })).not.toBe(draftFingerprint(d));
  });
});

describe('server errors land on fields', () => {
  test('an inactive account marks the line using it', () => {
    let d = newCheckDraft(today, 'bank');
    d = { ...d, expenses: [{ ...d.expenses[0]!, account_id: 'old', amount: 100 }] };
    const sent = draftToTxn(d);
    const e = serverErrors(d, sent, new ApiError(422, 'posting', 'x', { code: 'InactiveAccount', detail: 'old' }));
    expect(e.expenses[0]!.account).toContain('inactive');
  });
  test('a closed period marks the date', () => {
    const d = newCheckDraft(today, 'bank');
    const e = serverErrors(d, draftToTxn(d), new ApiError(423, 'closed_period', 'x', { closing_date: '2026-09-30' }));
    expect(e.header.date).toContain('2026-09-30');
  });
});
