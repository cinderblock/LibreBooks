// The API's error envelope, `{ error: { code, message, detail? } }`, as a typed
// exception, plus the helpers forms use to put an error next to the right field.

import { formatMoney } from '~/lib/money';
import type { PostingError } from './generated/PostingError';
import type { Txn } from './generated/Txn';

export type ApiErrorCode =
  | 'invalid'
  | 'unauthenticated'
  | 'forbidden'
  | 'not_found'
  | 'conflict'
  | 'duplicate'
  | 'in_use'
  | 'posting'
  | 'closed_period'
  | 'rate_limited'
  | 'internal'
  // Client-side: the request never got a well-formed answer.
  | 'network'
  | 'bad_response';

export class ApiError extends Error {
  override readonly name = 'ApiError';
  constructor(
    readonly status: number,
    readonly code: ApiErrorCode | (string & {}),
    message: string,
    readonly detail?: unknown,
  ) {
    super(message);
  }

  /** A stale `edit_seq`: someone else saved this record since it was read. */
  get isStaleConflict(): boolean {
    if (this.code !== 'conflict') return false;
    const d = this.detail as { expected?: unknown; actual?: unknown } | undefined;
    // Duplicate names also come back as 409; those carry no expected/actual.
    return !d || d.expected !== undefined || d.actual !== undefined;
  }

  get isDuplicate(): boolean {
    if (this.code === 'duplicate') return true;
    const d = this.detail as { kind?: unknown } | string | undefined;
    return this.code === 'conflict' && (d === 'duplicate' || (typeof d === 'object' && d?.kind === 'duplicate'));
  }

  get postingError(): PostingError | null {
    return this.code === 'posting' && this.detail && typeof this.detail === 'object'
      ? (this.detail as PostingError)
      : null;
  }

  get closingDate(): string | null {
    const d = this.detail as { closing_date?: string } | undefined;
    return this.code === 'closed_period' ? (d?.closing_date ?? null) : null;
  }
}

export function isApiError(e: unknown): e is ApiError {
  return e instanceof ApiError;
}

/** Build an ApiError from a non-2xx response, tolerating non-JSON bodies. */
export function apiErrorFromBody(status: number, bodyText: string, parsed: unknown): ApiError {
  const env = parsed as { error?: { code?: string; message?: string; detail?: unknown } } | undefined;
  if (env?.error?.code) {
    return new ApiError(status, env.error.code, env.error.message ?? env.error.code, env.error.detail);
  }
  const fallback: Record<number, ApiErrorCode> = {
    400: 'invalid',
    401: 'unauthenticated',
    403: 'forbidden',
    404: 'not_found',
    409: 'conflict',
    422: 'posting',
    423: 'closed_period',
    429: 'rate_limited',
  };
  return new ApiError(status, fallback[status] ?? 'internal', bodyText.trim().slice(0, 200) || `HTTP ${status}`);
}

/** People-facing text for a posting error (Rust's `#[error]` strings). */
export function postingErrorMessage(e: PostingError, names?: (id: string) => string | undefined): string {
  const n = (id: string) => names?.(id) ?? 'This account';
  switch (e.code) {
    case 'Unbalanced':
      return `The transaction does not balance: debits and credits differ by ${formatMoney(e.detail)}.`;
    case 'MissingAccount':
      return 'Choose an account.';
    case 'MissingToAccount':
      return 'Choose the account to transfer to.';
    case 'UnknownAccount':
      return "The account doesn't exist.";
    case 'InactiveAccount':
      return `${n(e.detail)} is inactive.`;
    case 'NonPostingAccount':
      return "A non-posting account can't be used here.";
    case 'WrongAccountType':
      return `This needs a ${e.detail.expected} account.`;
    case 'MissingRoleAccount':
      return `The company has no ${e.detail} account yet.`;
    case 'UnknownItem':
      return "The item doesn't exist.";
    case 'ItemWithoutAccount':
      return `${e.detail} has no account to post to.`;
    case 'MissingEntity':
      return 'Choose a name.';
    case 'UnknownEntity':
      return "The name doesn't exist.";
    case 'WrongEntityKind':
      return `This needs a ${e.detail.expected}.`;
    case 'ArApLineWithoutName':
      return 'Lines in Accounts Receivable or Accounts Payable need a customer or vendor.';
    case 'TransferToSameAccount':
      return "A transfer's accounts must differ.";
    case 'NonPositiveAmount':
      return 'The amount must be more than zero.';
    case 'NoLines':
      return 'The transaction has no lines.';
    case 'OverApplied':
      return `Payments applied (${formatMoney(e.detail.applied)}) exceed the payment (${formatMoney(e.detail.amount)}).`;
    case 'UnknownTarget':
      return "The transaction being paid doesn't exist or isn't open.";
    case 'TargetForOtherName':
      return "A payment can only pay this customer's or vendor's transactions.";
    case 'TargetInOtherAccount':
      return 'The transaction being paid is in a different A/R or A/P account.';
    case 'DiscountWithoutAccount':
      return 'A discount needs an account.';
    case 'LineKindNotAllowed':
      return `${e.detail} lines aren't allowed on this form.`;
    case 'UnsupportedItem':
      return `${e.detail} items aren't supported on this form yet.`;
    case 'Overflow':
      return 'An amount is too large.';
  }
}

/** One-line description of any error, for toasts and banners. */
export function describeError(e: unknown): string {
  if (isApiError(e)) {
    const pe = e.postingError;
    if (pe) return postingErrorMessage(pe);
    if (e.code === 'closed_period' && e.closingDate) {
      return `The books are closed through ${e.closingDate}. Choose a later date.`;
    }
    if (e.isStaleConflict) return 'Someone else changed this since you opened it.';
    if (e.code === 'network') return "Can't reach the LibreBooks server.";
    return e.message;
  }
  return e instanceof Error ? e.message : String(e);
}

/** Where on a form an error belongs. */
export type FieldTarget =
  | { field: 'date' | 'account' | 'to_account' | 'entity' | 'amount' | 'number' | 'lines' | 'form' }
  | { field: 'line'; index: number; column: 'account' | 'item' | 'amount' | 'entity' | 'any' };

/**
 * Map a save error to the fields it is about, so the form can mark them inline
 * instead of only showing a toast. Uses the transaction as sent to resolve ids
 * in the error detail to the header or to particular lines.
 */
export function errorTargets(txn: Txn, e: ApiError): FieldTarget[] {
  if (e.code === 'closed_period') return [{ field: 'date' }];
  if (e.isDuplicate) return [{ field: 'number' }];
  const pe = e.postingError;
  if (!pe) {
    // 400 `invalid` may name a field (`detail.field`: "date", "lines[2].amount").
    const field = (e.detail as { field?: string } | undefined)?.field;
    const m = field ? /^lines\[(\d+)\]\.(\w+)$/.exec(field) : null;
    if (m) {
      const column = (['account', 'item', 'amount', 'entity'] as const).find((c) => m[2]?.startsWith(c)) ?? 'any';
      return [{ field: 'line', index: Number(m[1]), column }];
    }
    if (field === 'date' || field === 'account_id' || field === 'entity_id' || field === 'amount' || field === 'number') {
      const map = { date: 'date', account_id: 'account', entity_id: 'entity', amount: 'amount', number: 'number' } as const;
      return [{ field: map[field] }];
    }
    return [{ field: 'form' }];
  }
  const linesWithAccount = (id: string) =>
    txn.lines.flatMap((l, index) => (l.account_id === id ? [{ field: 'line' as const, index, column: 'account' as const }] : []));
  switch (pe.code) {
    case 'MissingAccount': {
      if (!txn.account_id) return [{ field: 'account' }];
      const missing = txn.lines.flatMap((l, index) =>
        l.kind !== 'Text' && l.kind !== 'Item' && !l.account_id ? [{ field: 'line' as const, index, column: 'account' as const }] : [],
      );
      return missing.length ? missing : [{ field: 'form' }];
    }
    case 'MissingToAccount':
    case 'TransferToSameAccount':
      return [{ field: 'to_account' }];
    case 'UnknownAccount':
    case 'InactiveAccount':
    case 'NonPostingAccount': {
      const id = pe.detail;
      const out: FieldTarget[] = [];
      if (txn.account_id === id) out.push({ field: 'account' });
      if (txn.to_account_id === id) out.push({ field: 'to_account' });
      out.push(...linesWithAccount(id));
      return out.length ? out : [{ field: 'form' }];
    }
    case 'WrongAccountType':
      return txn.account_id === pe.detail.account ? [{ field: 'account' }] : linesWithAccount(pe.detail.account);
    case 'UnknownItem': {
      // The detail is the unknown id, or the nil id when a line has no item.
      const out = txn.lines.flatMap((l, index) =>
        l.kind === 'Item' && (l.item_id === pe.detail || !l.item_id)
          ? [{ field: 'line' as const, index, column: 'item' as const }]
          : [],
      );
      return out.length ? out : [{ field: 'lines' }];
    }
    case 'UnsupportedItem':
    case 'ItemWithoutAccount':
      // The detail names the item (or its type), not its id: point at the tab.
      return [{ field: 'lines' }];
    case 'MissingEntity':
    case 'UnknownEntity':
    case 'WrongEntityKind':
      return [{ field: 'entity' }];
    case 'ArApLineWithoutName':
      return [{ field: 'lines' }];
    case 'NonPositiveAmount':
      return [{ field: 'amount' }];
    case 'NoLines':
    case 'LineKindNotAllowed':
      return [{ field: 'lines' }];
    default:
      return [{ field: 'form' }];
  }
}
