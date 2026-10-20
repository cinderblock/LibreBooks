// Transfer Funds as plain data: from one balance-sheet account to another.

import { blankTxn } from '~/api/blank';
import { describeError, errorTargets, type ApiError } from '~/api/errors';
import type { Txn } from '~/api/types';
import type { IsoDate } from '~/lib/dates';
import type { Cents } from '~/lib/money';
import { noErrors, type FormErrors } from './model';

export interface TransferDraft {
  id: string;
  edit_seq: number;
  date: IsoDate;
  account_id: string | null;
  to_account_id: string | null;
  amount: Cents | null;
  memo: string;
  base: Txn | null;
}

export function newTransferDraft(date: IsoDate, fromId: string | null): TransferDraft {
  return { id: crypto.randomUUID(), edit_seq: 0, date, account_id: fromId, to_account_id: null, amount: null, memo: 'Funds Transfer', base: null };
}

export function transferDraftFromTxn(txn: Txn): TransferDraft {
  return { id: txn.id, edit_seq: txn.edit_seq, date: txn.date, account_id: txn.account_id, to_account_id: txn.to_account_id, amount: txn.amount, memo: txn.memo ?? '', base: txn };
}

export function validateTransfer(d: TransferDraft): FormErrors {
  const e = noErrors();
  if (!d.date) e.header.date = 'Enter a date';
  if (!d.account_id) e.header.account = 'Choose the account to transfer from';
  if (!d.to_account_id) e.header.to_account = 'Choose the account to transfer to';
  if (d.account_id && d.account_id === d.to_account_id) e.header.to_account = 'Choose a different account';
  if ((d.amount ?? 0) <= 0) e.header.amount = 'Enter the amount to transfer';
  return e;
}

export function transferDraftToTxn(d: TransferDraft): Txn {
  const base = d.base ?? blankTxn('Transfer', d.date);
  return { ...base, id: d.id, txn_type: 'Transfer', edit_seq: d.edit_seq, date: d.date, account_id: d.account_id, to_account_id: d.to_account_id, amount: d.amount ?? 0, memo: d.memo.trim() || null, lines: [] };
}

export function transferServerErrors(_d: TransferDraft, sent: Txn, err: ApiError): FormErrors {
  const e = noErrors();
  const msg = describeError(err);
  for (const t of errorTargets(sent, err)) {
    if (t.field === 'line' || t.field === 'lines' || t.field === 'form') e.form.push(msg);
    else e.header[t.field] = msg;
  }
  return e;
}

export function isTransferEmpty(d: TransferDraft): boolean {
  return !d.to_account_id && !d.amount;
}

export function transferFingerprint(d: TransferDraft): string {
  return JSON.stringify([d.date, d.account_id, d.to_account_id, d.amount, d.memo.trim()]);
}

export function nextTransferDraft(d: TransferDraft): TransferDraft {
  return newTransferDraft(d.date, d.account_id);
}
