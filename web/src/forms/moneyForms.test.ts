import { describe, expect, test } from 'bun:test';
import { blankLine, blankTxn } from '~/api/blank';
import { ApiError } from '~/api/errors';
import type { TxnSummary } from '~/api/types';
import { addPayments, blankDepositRow, depositDraftFromTxn, depositDraftToTxn, depositServerErrors, depositTotal, newDepositDraft, validateDeposit } from './depositDraft';
import { blankJournalRow, difference, journalDraftFromTxn, journalDraftToTxn, newJournalDraft, setCredit, setDebit, validateJournal, withBalancingLine } from './journalDraft';
import { newTransferDraft, transferDraftToTxn, validateTransfer } from './transferDraft';

const today = '2026-10-19';
const payment = (id: string, amount: number): TxnSummary => ({ id, txn_type: 'ReceivePayment', date: today, number: '5001', entity_id: 'acme', entity_name: 'Acme', account_id: 'uf', account_name: 'Undeposited Funds', amount, open_balance: null, due_date: null, memo: null, is_void: false, is_pending: false, edit_seq: 1 });

describe('make deposits', () => {
  test('payments from Undeposited Funds become linked lines; cash back reduces the total', () => {
    let d = newDepositDraft(today, 'bank');
    d = addPayments(d, [payment('p1', 10_000), payment('p2', 5_000)], 'uf');
    d = addPayments(d, [payment('p1', 10_000)], 'uf'); // once only
    expect(d.rows.filter((r) => r.linked_txn_id)).toHaveLength(2);
    d = { ...d, rows: [...d.rows.slice(0, 2), { ...blankDepositRow(), account_id: 'interest', amount: 250, memo: 'Interest' }], cash_back_account_id: 'petty', cash_back_amount: 1_000 };
    expect(depositTotal(d)).toBe(14_250);
    expect(validateDeposit(d).form).toHaveLength(0);
    const t = depositDraftToTxn(d);
    expect(t.lines.map((l) => l.kind)).toEqual(['DepositLine', 'DepositLine', 'Account', 'CashBack']);
    expect(t.lines[0]).toMatchObject({ linked_txn_id: 'p1', account_id: 'uf', amount: 10_000, check_number: '5001' });
    expect(t.amount).toBe(14_250);
    const back = depositDraftFromTxn({ ...t, edit_seq: 1 });
    expect(back.cash_back_amount).toBe(1_000);
    expect(back.rows.filter((r) => !r.account_id)).toHaveLength(1); // the trailing blank
  });
  test('validation: an account, lines, and cash back within the deposit', () => {
    const d = newDepositDraft(today, null);
    const e = validateDeposit(d);
    expect(e.header.account).toBeDefined();
    expect(e.form).toHaveLength(1);
    const bad = { ...newDepositDraft(today, 'bank'), rows: [{ ...blankDepositRow(), account_id: 'x', amount: 100 }], cash_back_account_id: null, cash_back_amount: 500 };
    expect(validateDeposit(bad).header.cash_back).toContain('exceed');
  });
  test('server errors land on the line', () => {
    const d = { ...newDepositDraft(today, 'bank'), rows: [{ ...blankDepositRow(), account_id: 'old', amount: 100 }, blankDepositRow()] };
    const e = depositServerErrors(d, depositDraftToTxn(d), new ApiError(422, 'posting', 'x', { code: 'InactiveAccount', detail: 'old' }));
    expect(e.grids.rows![0]!.account).toContain('inactive');
  });
});

describe('general journal', () => {
  test('debit and credit exclude each other; the next line balances the entry', () => {
    const r = setDebit(blankJournalRow(), 10_000);
    expect(setCredit(r, 2_500)).toMatchObject({ debit: null, credit: 2_500 });
    const rows = withBalancingLine([{ ...r, account_id: 'rent' }]);
    expect(rows).toHaveLength(3);
    expect(rows[1]).toMatchObject({ credit: 10_000, debit: null });
    expect(difference(rows)).toBe(0);
  });
  test('must balance; lines need an account and an amount', () => {
    let d = newJournalDraft(today, true);
    d = { ...d, rows: [{ ...blankJournalRow(), account_id: 'a', debit: 100 }, { ...blankJournalRow(), account_id: 'b', credit: 90 }, blankJournalRow()] };
    expect(validateJournal(d).form.join(' ')).toContain("doesn't balance");
    d = { ...d, rows: [{ ...blankJournalRow(), account_id: 'a', debit: 100 }, { ...blankJournalRow(), credit: 100 }, blankJournalRow()] };
    expect(validateJournal(d).grids.rows![1]!.account).toBeDefined();
    d = { ...d, rows: [{ ...blankJournalRow(), account_id: 'a', debit: 100 }, { ...blankJournalRow(), account_id: 'b', credit: 100 }, blankJournalRow()] };
    expect(validateJournal(d).form).toHaveLength(0);
    const t = journalDraftToTxn({ ...d, number: '12', adjusting: true });
    expect(t.lines.map((l) => l.amount)).toEqual([100, -100]);
    expect(t).toMatchObject({ number: '12', is_adjustment: true, amount: 100 });
  });
  test('round trip splits signed amounts back into the columns', () => {
    const t = blankTxn('JournalEntry', today);
    t.lines = [{ ...blankLine('Journal'), account_id: 'a', amount: 500 }, { ...blankLine('Journal'), account_id: 'b', amount: -500 }];
    const d = journalDraftFromTxn(t);
    expect(d.rows[0]).toMatchObject({ debit: 500, credit: null });
    expect(d.rows[1]).toMatchObject({ debit: null, credit: 500 });
  });
});

describe('transfer funds', () => {
  test('needs two different accounts and a positive amount', () => {
    const d = newTransferDraft(today, 'checking');
    expect(validateTransfer(d).header.to_account).toBeDefined();
    expect(validateTransfer({ ...d, to_account_id: 'checking', amount: 5 }).header.to_account).toContain('different');
    expect(validateTransfer({ ...d, to_account_id: 'savings', amount: 0 }).header.amount).toBeDefined();
    const ok = { ...d, to_account_id: 'savings', amount: 50_000 };
    expect(validateTransfer(ok).header).toEqual({});
    expect(transferDraftToTxn(ok)).toMatchObject({ txn_type: 'Transfer', account_id: 'checking', to_account_id: 'savings', amount: 50_000, lines: [] });
  });
});
