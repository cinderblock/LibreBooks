// Make General Journal Entries as plain data: debit and credit columns, the
// running difference, QBD's balancing prefill on the next line, and the rule
// that the entry must balance before it's sent.

import { blankLine, blankTxn } from '~/api/blank';
import { describeError, errorTargets, type ApiError } from '~/api/errors';
import type { Txn } from '~/api/types';
import type { IsoDate } from '~/lib/dates';
import { sumCents, type Cents } from '~/lib/money';
import { withTrailingBlank } from '~/inputs/gridRows';
import { noErrors, putGridError, type FormErrors } from './model';

export interface JournalRow {
  key: string;
  lineId: string | null;
  account_id: string | null;
  debit: Cents | null;
  credit: Cents | null;
  memo: string;
  entity_id: string | null;
  class_id: string | null;
}

export interface JournalDraft {
  id: string;
  edit_seq: number;
  date: IsoDate;
  number: string;
  numberIsAuto: boolean;
  adjusting: boolean;
  rows: JournalRow[];
  base: Txn | null;
}

export const blankJournalRow = (): JournalRow => ({ key: crypto.randomUUID(), lineId: null, account_id: null, debit: null, credit: null, memo: '', entity_id: null, class_id: null });

export const isBlankJournal = (r: JournalRow) => !r.account_id && !r.debit && !r.credit && !r.memo.trim() && !r.entity_id;

export function newJournalDraft(date: IsoDate, autoNumber: boolean): JournalDraft {
  return { id: crypto.randomUUID(), edit_seq: 0, date, number: '', numberIsAuto: autoNumber, adjusting: false, rows: [blankJournalRow()], base: null };
}

export function journalDraftFromTxn(txn: Txn): JournalDraft {
  const rows = txn.lines
    .filter((l) => l.kind === 'Journal')
    .map<JournalRow>((l) => ({
      key: l.id,
      lineId: l.id,
      account_id: l.account_id,
      debit: l.amount > 0 ? l.amount : null,
      credit: l.amount < 0 ? -l.amount : null,
      memo: l.description ?? '',
      entity_id: l.entity_id,
      class_id: l.class_id,
    }));
  return { id: txn.id, edit_seq: txn.edit_seq, date: txn.date, number: txn.number ?? '', numberIsAuto: false, adjusting: txn.is_adjustment, rows: withTrailingBlank(rows, isBlankJournal, blankJournalRow), base: txn };
}

export function debits(rows: readonly JournalRow[]): Cents {
  return sumCents(rows.map((r) => r.debit ?? 0));
}
export function credits(rows: readonly JournalRow[]): Cents {
  return sumCents(rows.map((r) => r.credit ?? 0));
}
/** Debits minus credits: zero when the entry balances. */
export function difference(rows: readonly JournalRow[]): Cents {
  return debits(rows) - credits(rows);
}

/** A debit clears the credit on its line, and the other way round (one or the other, as in QBD). */
export function setDebit(r: JournalRow, debit: Cents | null): JournalRow {
  return { ...r, debit: debit || null, credit: debit ? null : r.credit };
}
export function setCredit(r: JournalRow, credit: Cents | null): JournalRow {
  return { ...r, credit: credit || null, debit: credit ? null : r.debit };
}

/**
 * QBD prefills the next line with whatever balances the entry so far. Applied
 * when the trailing line is still blank; the person can overwrite it.
 */
export function withBalancingLine(rows: readonly JournalRow[]): JournalRow[] {
  const out = withTrailingBlank([...rows], isBlankJournal, blankJournalRow);
  const diff = difference(out);
  const last = out[out.length - 1]!;
  if (diff !== 0 && isBlankJournal(last)) {
    out[out.length - 1] = diff > 0 ? { ...last, credit: diff } : { ...last, debit: -diff };
    out.push(blankJournalRow());
  }
  return out;
}

export function validateJournal(d: JournalDraft): FormErrors {
  const e = noErrors();
  if (!d.date) e.header.date = 'Enter a date';
  const live = d.rows.filter((r) => !isBlankJournal(r));
  if (live.length === 0) e.form.push('Enter at least one line.');
  d.rows.forEach((r, i) => {
    if (isBlankJournal(r)) return;
    if (!r.account_id) putGridError(e, 'rows', i, 'account', 'Choose an account');
    if (!r.debit && !r.credit) putGridError(e, 'rows', i, 'debit', 'Enter a debit or a credit');
  });
  const diff = difference(d.rows);
  if (live.length > 0 && diff !== 0) e.form.push(`The entry doesn't balance: debits and credits differ by ${(Math.abs(diff) / 100).toFixed(2)}.`);
  return e;
}

export function journalDraftToTxn(d: JournalDraft): Txn {
  const base = d.base ?? blankTxn('JournalEntry', d.date);
  const lines = d.rows
    .filter((r) => !isBlankJournal(r))
    .map((r) => ({
      ...blankLine('Journal'),
      id: r.lineId ?? r.key,
      account_id: r.account_id,
      amount: (r.debit ?? 0) - (r.credit ?? 0),
      description: r.memo.trim() || null,
      entity_id: r.entity_id,
      class_id: r.class_id,
    }));
  return { ...base, id: d.id, txn_type: 'JournalEntry', edit_seq: d.edit_seq, date: d.date, number: d.number.trim() || null, is_adjustment: d.adjusting, amount: debits(d.rows), lines };
}

export function journalServerErrors(d: JournalDraft, sent: Txn, err: ApiError): FormErrors {
  const e = noErrors();
  const msg = describeError(err);
  const sentRows = d.rows.filter((r) => !isBlankJournal(r));
  for (const t of errorTargets(sent, err)) {
    if (t.field === 'line') {
      const row = sentRows[t.index];
      const i = row ? d.rows.indexOf(row) : -1;
      if (i >= 0) putGridError(e, 'rows', i, t.column === 'entity' ? 'name' : t.column === 'amount' ? 'debit' : 'account', msg);
      else e.form.push(msg);
    } else if (t.field === 'lines' || t.field === 'form' || t.field === 'to_account' || t.field === 'amount') e.form.push(msg);
    else e.header[t.field] = msg;
  }
  return e;
}

export function isJournalEmpty(d: JournalDraft): boolean {
  return d.rows.every(isBlankJournal);
}

export function journalFingerprint(d: JournalDraft): string {
  const t = journalDraftToTxn(d);
  return JSON.stringify([t.date, t.number, t.is_adjustment, t.lines.map((l) => [l.account_id, l.amount, l.description, l.entity_id, l.class_id])]);
}

export function nextJournalDraft(d: JournalDraft): JournalDraft {
  return newJournalDraft(d.date, d.numberIsAuto || d.number !== '');
}
