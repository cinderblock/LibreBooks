import { describe, expect, test } from 'bun:test';
import { blankAccount } from '~/api/blank';
import { applyEntryToTxn, blankEntry, isBlankEntry, registerEntryFromTxn, registerEntryToTxn } from './registerEntry';

const day = '2026-10-19';
const bank = blankAccount('Checking', 'Bank');
const card = blankAccount('Visa', 'CreditCard');
const loan = blankAccount('Loan', 'LongTermLiability');
const prepaid = blankAccount('Prepaid', 'OtherCurrentAsset');
const ar = blankAccount('A/R', 'AccountsReceivable');

describe('register entry → transaction', () => {
  test('bank payment is a check, deposit is a deposit', () => {
    const pay = registerEntryToTxn(bank, { ...blankEntry(day), number: '1050', payee_id: 'v', left: 4_200, account_id: 'rent' });
    expect(pay.ok && pay.txn).toMatchObject({ txn_type: 'Check', account_id: bank.id, number: '1050', entity_id: 'v', amount: 4_200 });
    expect(pay.ok && pay.txn.lines[0]).toMatchObject({ kind: 'Account', account_id: 'rent', amount: 4_200 });
    const dep = registerEntryToTxn(bank, { ...blankEntry(day), payee_id: 'c', right: 10_000, account_id: 'income' });
    expect(dep.ok && dep.txn).toMatchObject({ txn_type: 'Deposit', entity_id: null });
    expect(dep.ok && dep.txn.lines[0]).toMatchObject({ kind: 'DepositLine', entity_id: 'c', amount: 10_000 });
  });

  test('credit card charge and credit', () => {
    expect(registerEntryToTxn(card, { ...blankEntry(day), left: 500, account_id: 'fuel' })).toMatchObject({ ok: true, txn: { txn_type: 'CreditCardCharge' } });
    expect(registerEntryToTxn(card, { ...blankEntry(day), right: 500, account_id: 'fuel' })).toMatchObject({ ok: true, txn: { txn_type: 'CreditCardCredit' } });
  });

  test('other accounts make a balanced journal entry with the right sides', () => {
    // Liability "Increase" (left) is a credit to the loan.
    const r = registerEntryToTxn(loan, { ...blankEntry(day), left: 1_000, account_id: 'equipment' });
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.txn.txn_type).toBe('JournalEntry');
    expect(r.txn.lines.map((l) => [l.account_id, l.amount])).toEqual([
      [loan.id, -1_000],
      ['equipment', 1_000],
    ]);
    // Asset "Increase" (right) is a debit.
    const a = registerEntryToTxn(prepaid, { ...blankEntry(day), right: 300, account_id: bank.id });
    expect(a.ok && a.txn.lines[0]!.amount).toBe(300);
  });

  test('refusals', () => {
    expect(registerEntryToTxn(bank, { ...blankEntry(day), account_id: 'x' })).toMatchObject({ ok: false, field: 'amount' });
    expect(registerEntryToTxn(bank, { ...blankEntry(day), left: 1, right: 1, account_id: 'x' })).toMatchObject({ ok: false, field: 'amount' });
    expect(registerEntryToTxn(bank, { ...blankEntry(day), left: 1 })).toMatchObject({ ok: false, field: 'account' });
    expect(registerEntryToTxn(bank, { ...blankEntry(day), left: 1, account_id: bank.id })).toMatchObject({ ok: false, field: 'account' });
    expect(registerEntryToTxn(ar, { ...blankEntry(day), left: 1, account_id: 'x' })).toMatchObject({ ok: false, field: 'form' });
  });

  test('blank detection', () => {
    expect(isBlankEntry(blankEntry(day))).toBe(true);
    expect(isBlankEntry({ ...blankEntry(day), memo: 'x' })).toBe(false);
  });
});

describe('editing a saved transaction in place', () => {
  test('a one-line check round-trips through the entry and keeps its identity', () => {
    const made = registerEntryToTxn(bank, { ...blankEntry(day), number: '1050', payee_id: 'v', left: 4_200, account_id: 'rent', memo: 'Rent' });
    if (!made.ok) throw new Error('expected a check');
    const saved = { ...made.txn, edit_seq: 3 };
    const entry = registerEntryFromTxn(bank, saved);
    expect(entry).toEqual({ date: day, number: '1050', payee_id: 'v', left: 4_200, right: null, account_id: 'rent', memo: 'Rent' });
    const edited = applyEntryToTxn(bank, { ...entry!, left: 4_500, memo: 'Rent (Oct)' }, saved);
    if (!edited.ok) throw new Error('expected ok');
    expect(edited.txn).toMatchObject({ id: saved.id, edit_seq: 3, txn_type: 'Check', amount: 4_500, memo: 'Rent (Oct)' });
    expect(edited.txn.lines[0]!.id).toBe(saved.lines[0]!.id);
  });
  test('a transfer edits from either side; a journal entry with two lines too', () => {
    const t = registerEntryToTxn(prepaid, { ...blankEntry(day), right: 300, account_id: bank.id });
    if (!t.ok) throw new Error('expected a journal');
    expect(t.txn.txn_type).toBe('JournalEntry');
    const e = registerEntryFromTxn(prepaid, t.txn);
    expect(e).toMatchObject({ right: 300, account_id: bank.id });
    const fromBank = registerEntryFromTxn(bank, t.txn);
    expect(fromBank).toMatchObject({ left: 300, account_id: prepaid.id });
  });
  test('split or unsupported transactions are not editable in place', () => {
    const made = registerEntryToTxn(bank, { ...blankEntry(day), left: 10, account_id: 'rent' });
    if (!made.ok) throw new Error('expected a check');
    const split = { ...made.txn, lines: [...made.txn.lines, { ...made.txn.lines[0]!, id: 'l2' }] };
    expect(registerEntryFromTxn(bank, split)).toBeNull();
    expect(registerEntryFromTxn(bank, { ...made.txn, txn_type: 'BillPaymentCheck' })).toBeNull();
    expect(registerEntryFromTxn(card, made.txn)).toBeNull();
  });
  test('moving the amount to the other column is refused (it would change the kind)', () => {
    const made = registerEntryToTxn(bank, { ...blankEntry(day), left: 10, account_id: 'rent' });
    if (!made.ok) throw new Error('expected a check');
    const r = applyEntryToTxn(bank, { ...registerEntryFromTxn(bank, made.txn)!, left: null, right: 10 }, made.txn);
    expect(r).toMatchObject({ ok: false, field: 'amount' });
  });
});
