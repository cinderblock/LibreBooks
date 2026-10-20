// Handing an unsaved transaction to a form window.
//
// "Enter now" on a memorized transaction, "Add more details" on a bank feed
// line and a register's QuickZoom into a new entry all need to open a form
// *prefilled* with a `Txn` that isn't saved yet. The window system identifies
// windows by URL, and a whole transaction doesn't belong in one; so the caller
// stashes it here, opens `/forms/<kind>/new?draft=<token>`, and the form takes
// it on mount. Tokens are single-use: a reload of that URL shows an empty form
// rather than re-entering something that may already have been saved.

import type { Txn } from '~/api/types';

const stash = new Map<string, Txn>();

export function stashTxn(txn: Txn): string {
  const token = crypto.randomUUID();
  stash.set(token, txn);
  return token;
}

export function takeTxn(token: string | undefined | null): Txn | null {
  if (!token) return null;
  const t = stash.get(token) ?? null;
  stash.delete(token);
  return t;
}
