// Make Deposits as plain data: payments picked out of Undeposited Funds, extra
// lines deposited straight to an account, and cash back, into one bank account.

import { blankLine, blankTxn } from '~/api/blank';
import { describeError, errorTargets, type ApiError } from '~/api/errors';
import type { Txn, TxnSummary } from '~/api/types';
import type { IsoDate } from '~/lib/dates';
import { sumCents, type Cents } from '~/lib/money';
import { withTrailingBlank } from '~/inputs/gridRows';
import { noErrors, putGridError, type FormErrors } from './model';

export interface DepositRow {
  key: string;
  lineId: string | null;
  /** A payment moved out of Undeposited Funds (linked), or anything else. */
  linked_txn_id: string | null;
  entity_id: string | null;
  account_id: string | null;
  memo: string;
  check_number: string;
  payment_method_id: string | null;
  class_id: string | null;
  amount: Cents | null;
}

export interface DepositDraft {
  id: string;
  edit_seq: number;
  account_id: string | null;
  date: IsoDate;
  memo: string;
  rows: DepositRow[];
  cash_back_account_id: string | null;
  cash_back_memo: string;
  cash_back_amount: Cents | null;
  base: Txn | null;
}

export const blankDepositRow = (): DepositRow => ({
  key: crypto.randomUUID(),
  lineId: null,
  linked_txn_id: null,
  entity_id: null,
  account_id: null,
  memo: '',
  check_number: '',
  payment_method_id: null,
  class_id: null,
  amount: null,
});

export const isBlankDeposit = (r: DepositRow) => !r.linked_txn_id && !r.entity_id && !r.account_id && !r.amount && !r.memo.trim() && !r.check_number.trim();

export function newDepositDraft(date: IsoDate, accountId: string | null): DepositDraft {
  return { id: crypto.randomUUID(), edit_seq: 0, account_id: accountId, date, memo: 'Deposit', rows: [blankDepositRow()], cash_back_account_id: null, cash_back_memo: '', cash_back_amount: null, base: null };
}

export function depositDraftFromTxn(txn: Txn): DepositDraft {
  const cash = txn.lines.find((l) => l.kind === 'CashBack');
  const rows = txn.lines
    .filter((l) => l.kind === 'DepositLine' || l.kind === 'Account')
    .map<DepositRow>((l) => ({
      key: l.id,
      lineId: l.id,
      linked_txn_id: l.linked_txn_id,
      entity_id: l.entity_id,
      account_id: l.account_id,
      memo: l.description ?? '',
      check_number: l.check_number ?? '',
      payment_method_id: l.payment_method_id,
      class_id: l.class_id,
      amount: l.amount,
    }));
  return {
    id: txn.id,
    edit_seq: txn.edit_seq,
    account_id: txn.account_id,
    date: txn.date,
    memo: txn.memo ?? '',
    rows: withTrailingBlank(rows, isBlankDeposit, blankDepositRow),
    cash_back_account_id: cash?.account_id ?? null,
    cash_back_memo: cash?.description ?? '',
    cash_back_amount: cash?.amount ?? null,
    base: txn,
  };
}

/** Add payments picked from Undeposited Funds (each one once). */
export function addPayments(d: DepositDraft, payments: readonly TxnSummary[], undepositedFundsId: string): DepositDraft {
  const have = new Set(d.rows.map((r) => r.linked_txn_id).filter(Boolean));
  const added = payments
    .filter((p) => !have.has(p.id))
    .map<DepositRow>((p) => ({
      key: crypto.randomUUID(),
      lineId: null,
      linked_txn_id: p.id,
      entity_id: p.entity_id,
      account_id: undepositedFundsId,
      memo: p.memo ?? '',
      check_number: p.number ?? '',
      payment_method_id: null,
      class_id: null,
      amount: p.amount,
    }));
  const rows = [...d.rows.filter((r) => !isBlankDeposit(r)), ...added];
  return { ...d, rows: withTrailingBlank(rows, isBlankDeposit, blankDepositRow) };
}

export function depositSubtotal(d: DepositDraft): Cents {
  return sumCents(d.rows.map((r) => r.amount ?? 0));
}

export function depositTotal(d: DepositDraft): Cents {
  return depositSubtotal(d) - (d.cash_back_amount ?? 0);
}

export function validateDeposit(d: DepositDraft): FormErrors {
  const e = noErrors();
  if (!d.account_id) e.header.account = 'Choose the account to deposit to';
  if (!d.date) e.header.date = 'Enter a date';
  const lines = d.rows.filter((r) => !isBlankDeposit(r));
  if (lines.length === 0) e.form.push('Add at least one payment or line.');
  d.rows.forEach((r, i) => {
    if (isBlankDeposit(r)) return;
    if (!r.account_id) putGridError(e, 'rows', i, 'account', 'Choose the account this comes from');
    if (!r.amount) putGridError(e, 'rows', i, 'amount', 'Enter an amount');
  });
  if ((d.cash_back_amount ?? 0) > 0 && !d.cash_back_account_id) e.header.cash_back = 'Choose the cash back account';
  if ((d.cash_back_amount ?? 0) > depositSubtotal(d)) e.header.cash_back = "Cash back can't exceed the deposit";
  return e;
}

export function depositDraftToTxn(d: DepositDraft): Txn {
  const base = d.base ?? blankTxn('Deposit', d.date);
  const lines = d.rows
    .filter((r) => !isBlankDeposit(r))
    .map((r) => ({
      ...blankLine(r.linked_txn_id ? 'DepositLine' : 'Account'),
      id: r.lineId ?? r.key,
      linked_txn_id: r.linked_txn_id,
      entity_id: r.entity_id,
      account_id: r.account_id,
      description: r.memo.trim() || null,
      check_number: r.check_number.trim() || null,
      payment_method_id: r.payment_method_id,
      class_id: r.class_id,
      amount: r.amount ?? 0,
    }));
  if ((d.cash_back_amount ?? 0) > 0) {
    lines.push({ ...blankLine('CashBack'), id: base.lines.find((l) => l.kind === 'CashBack')?.id ?? crypto.randomUUID(), account_id: d.cash_back_account_id, description: d.cash_back_memo.trim() || null, amount: d.cash_back_amount ?? 0 });
  }
  return { ...base, id: d.id, txn_type: 'Deposit', edit_seq: d.edit_seq, account_id: d.account_id, date: d.date, memo: d.memo.trim() || null, amount: depositTotal(d), lines };
}

export function depositServerErrors(d: DepositDraft, sent: Txn, err: ApiError): FormErrors {
  const e = noErrors();
  const msg = describeError(err);
  const sentRows = d.rows.filter((r) => !isBlankDeposit(r));
  for (const t of errorTargets(sent, err)) {
    if (t.field === 'line') {
      const row = sentRows[t.index];
      const i = row ? d.rows.indexOf(row) : -1;
      if (i >= 0) putGridError(e, 'rows', i, t.column === 'any' || t.column === 'item' ? 'account' : t.column, msg);
      else e.header.cash_back = msg;
    } else if (t.field === 'lines' || t.field === 'form' || t.field === 'to_account') e.form.push(msg);
    else e.header[t.field] = msg;
  }
  return e;
}

export function isDepositEmpty(d: DepositDraft): boolean {
  return d.rows.every(isBlankDeposit) && !d.cash_back_amount;
}

export function depositFingerprint(d: DepositDraft): string {
  const t = depositDraftToTxn(d);
  return JSON.stringify([t.account_id, t.date, t.memo, t.lines.map((l) => [l.kind, l.linked_txn_id, l.entity_id, l.account_id, l.description, l.check_number, l.payment_method_id, l.class_id, l.amount])]);
}

export function nextDepositDraft(d: DepositDraft): DepositDraft {
  return newDepositDraft(d.date, d.account_id);
}
