// What every transaction form's state machine shares: the errors shape (header
// fields, grid cells by grid/row/column, and form-wide messages) and the spec a
// form hands to `useTxnForm` so the hook can build, validate, send and reload
// its draft without knowing what the form is.

import type { ApiError } from '~/api/errors';
import type { SaveOutcome, Txn } from '~/api/types';
import type { TxnQuery } from '~/api/client';

export interface FormErrors {
  header: Record<string, string>;
  /** grid key → row index → column key → message. */
  grids: Record<string, Record<number, Record<string, string>>>;
  /** Errors that belong to no single field. */
  form: string[];
}

export const NO_ERRORS: FormErrors = { header: {}, grids: {}, form: [] };

export function noErrors(): FormErrors {
  return { header: {}, grids: {}, form: [] };
}

export function hasErrors(e: FormErrors): boolean {
  return Object.keys(e.header).length > 0 || Object.values(e.grids).some((g) => Object.keys(g).length > 0) || e.form.length > 0;
}

export function putGridError(e: FormErrors, grid: string, row: number, col: string, msg: string) {
  const g = (e.grids[grid] ??= {});
  g[row] = { ...(g[row] ?? {}), [col]: msg };
}

export function gridErrors(e: FormErrors, grid: string): Record<number, Record<string, string>> {
  return e.grids[grid] ?? {};
}

export function gridHasErrors(e: FormErrors, grid: string): boolean {
  return Object.keys(e.grids[grid] ?? {}).length > 0;
}

/** What `useTxnForm` needs from a form's pure model. */
export interface TxnFormSpec<D> {
  /** "check", "bill", "invoice": used in toasts and confirmations (a function when the draft can change kind). */
  noun: string | ((d: D) => string);
  newDraft(): D;
  fromTxn(txn: Txn): D;
  toTxn(d: D): Txn;
  validate(d: D): FormErrors;
  /** Nothing entered yet (closing a new form needn't ask). */
  isEmpty(d: D): boolean;
  /** A stable fingerprint of what the user can change, for dirty tracking of saved records. */
  fingerprint(d: D): string;
  /** Put a server error next to the fields it's about. */
  serverErrors(d: D, sent: Txn, err: ApiError): FormErrors;
  /** The draft to show after Save & New (keeps the account and date, as QBD does). */
  nextDraft(d: D): D;
  /** Company-relative path of a fresh form of this kind (Save & New after editing). */
  newPath: string;
  /** The toast after a save. */
  savedMessage(out: SaveOutcome): string;
  /** Previous / Next (Page Up / Page Down) walk this query, in date order; null for none. */
  siblings?(d: D): TxnQuery | null;
  /** The window title. */
  title(d: D, saved: Txn | null): string;
}
