// A transaction typed straight into a register's blank bottom line (QBD lets you
// record checks, deposits, card charges and simple journal entries that way).
// Which transaction type it becomes depends on the account type and on which
// amount column was filled, as in QBD.

import { blankLine, blankTxn } from '~/api/blank';
import type { Account, Txn } from '~/api/types';
import { registerColumns } from '~/lib/accountTypes';
import type { IsoDate } from '~/lib/dates';
import type { Cents } from '~/lib/money';

export interface RegisterEntry {
  date: IsoDate;
  number: string;
  payee_id: string | null;
  /** The register's left / right amount columns (Payment / Deposit on a bank). */
  left: Cents | null;
  right: Cents | null;
  account_id: string | null;
  memo: string;
}

export const blankEntry = (date: IsoDate): RegisterEntry => ({
  date,
  number: '',
  payee_id: null,
  left: null,
  right: null,
  account_id: null,
  memo: '',
});

export function isBlankEntry(e: RegisterEntry): boolean {
  return !e.number.trim() && !e.payee_id && !e.left && !e.right && !e.account_id && !e.memo.trim();
}

export type EntryResult = { ok: true; txn: Txn } | { ok: false; field: 'amount' | 'account' | 'form'; message: string };

export function registerEntryToTxn(account: Account, e: RegisterEntry): EntryResult {
  const left = e.left ?? 0;
  const right = e.right ?? 0;
  const cols = registerColumns(account.account_type);
  if ((left > 0) === (right > 0)) {
    return { ok: false, field: 'amount', message: `Enter either ${cols.left} or ${cols.right}.` };
  }
  if (left < 0 || right < 0) return { ok: false, field: 'amount', message: 'Amounts in a register are positive; use the other column.' };
  if (!e.account_id) return { ok: false, field: 'account', message: 'Choose the account on the other side.' };
  if (e.account_id === account.id) return { ok: false, field: 'account', message: 'Choose a different account than this register.' };
  const memo = e.memo.trim() || null;
  const number = e.number.trim() || null;
  const amount = left || right;

  const simple = (type: Txn['txn_type'], kind: 'Account' | 'DepositLine'): Txn => {
    const t = blankTxn(type, e.date);
    t.account_id = account.id;
    t.number = number;
    t.memo = memo;
    t.amount = amount;
    if (type !== 'Deposit') t.entity_id = e.payee_id;
    t.lines = [{ ...blankLine(kind), account_id: e.account_id, amount, entity_id: type === 'Deposit' ? e.payee_id : null, description: memo }];
    return t;
  };

  switch (account.account_type) {
    case 'Bank':
      // Payment column → a check; Deposit column → a deposit.
      return { ok: true, txn: left ? simple('Check', 'Account') : simple('Deposit', 'DepositLine') };
    case 'CreditCard':
      // Charge column → a card charge; Payment column → a card credit.
      return { ok: true, txn: simple(left ? 'CreditCardCharge' : 'CreditCardCredit', 'Account') };
    case 'AccountsReceivable':
    case 'AccountsPayable':
      return {
        ok: false,
        field: 'form',
        message:
          account.account_type === 'AccountsReceivable'
            ? 'Use Create Invoices or Receive Payments to change Accounts Receivable.'
            : 'Use Enter Bills or Pay Bills to change Accounts Payable.',
      };
    default: {
      // Other balance-sheet accounts: a two-line general journal entry.
      const debitHere = (left > 0) === cols.leftIsDebit;
      const here = debitHere ? amount : -amount;
      const t = blankTxn('JournalEntry', e.date);
      t.number = number;
      t.memo = memo;
      t.amount = amount;
      t.lines = [
        { ...blankLine('Journal'), account_id: account.id, amount: here, entity_id: e.payee_id, description: memo },
        { ...blankLine('Journal'), account_id: e.account_id, amount: -here, entity_id: e.payee_id, description: memo },
      ];
      return { ok: true, txn: t };
    }
  }
}
