import { describe, expect, test } from 'bun:test';
import { blankAccount } from '~/api/blank';
import { blankEntry, isBlankEntry, registerEntryToTxn } from './registerEntry';

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
