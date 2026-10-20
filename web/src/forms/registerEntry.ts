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

/** Transaction types a register line can edit in place (one line on the other side). */
const SIMPLE: ReadonlySet<Txn['txn_type']> = new Set(['Check', 'Deposit', 'CreditCardCharge', 'CreditCardCredit', 'Transfer', 'JournalEntry']);

/**
 * A saved transaction as a register entry, for editing it in place: only when
 * it is one of the simple kinds with one line on the other side (QBD edits
 * anything else in its form). Null means "open the form instead".
 */
export function registerEntryFromTxn(account: Account, txn: Txn): RegisterEntry | null {
  if (!SIMPLE.has(txn.txn_type) || txn.is_void) return null;
  const cols = registerColumns(account.account_type);
  const lines = txn.lines.filter((l) => l.kind !== 'Text');
  let other: string | null;
  let amount: number;
  let debitHere: boolean;
  if (txn.txn_type === 'Transfer') {
    if (txn.account_id !== account.id && txn.to_account_id !== account.id) return null;
    const from = txn.account_id === account.id;
    other = from ? txn.to_account_id : txn.account_id;
    amount = txn.amount;
    debitHere = !from;
  } else if (txn.txn_type === 'JournalEntry') {
    if (lines.length !== 2) return null;
    const mine = lines.find((l) => l.account_id === account.id);
    const theirs = lines.find((l) => l.account_id !== account.id);
    if (!mine || !theirs) return null;
    other = theirs.account_id;
    amount = Math.abs(mine.amount);
    debitHere = mine.amount > 0;
  } else {
    if (lines.length !== 1 || txn.account_id !== account.id) return null;
    const line = lines[0]!;
    other = line.account_id;
    amount = line.amount;
    // A check or card charge credits this account; a deposit or card credit debits it.
    debitHere = txn.txn_type === 'Deposit' || txn.txn_type === 'CreditCardCredit';
  }
  const left = debitHere === cols.leftIsDebit;
  return {
    date: txn.date,
    number: txn.number ?? '',
    payee_id: txn.entity_id ?? txn.lines[0]?.entity_id ?? null,
    left: left ? amount : null,
    right: left ? null : amount,
    account_id: other,
    memo: txn.memo ?? txn.lines[0]?.description ?? '',
  };
}

/**
 * Put an edited entry back onto the saved transaction it came from, keeping
 * its id, edit_seq, type and everything the register doesn't show.
 */
export function applyEntryToTxn(account: Account, entry: RegisterEntry, existing: Txn): EntryResult {
  const fresh = registerEntryToTxn(account, entry);
  if (!fresh.ok) return fresh;
  const f = fresh.txn;
  // Moving an amount to the other column changes what the transaction is (a check
  // becomes a deposit); QBD refuses that in place, and so do we.
  if (f.txn_type !== existing.txn_type && !(existing.txn_type === 'JournalEntry' && f.txn_type === 'JournalEntry')) {
    return { ok: false, field: 'amount', message: `Use the other column's form to change this ${existing.txn_type === 'Check' ? 'check' : 'transaction'} into something else.` };
  }
  const line0 = existing.lines.find((l) => l.kind !== 'Text');
  const lines = f.lines.map((l, i) => ({ ...l, id: i === 0 && line0 ? line0.id : (existing.lines[i]?.id ?? l.id) }));
  return { ok: true, txn: { ...existing, date: f.date, number: f.number, entity_id: f.entity_id, memo: f.memo, amount: f.amount, to_account_id: f.to_account_id ?? existing.to_account_id, lines } };
}

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
