// Receive Payments and bill payments as plain data: a header (who paid, how
// much, how, into which account) over an applications grid listing the name's
// open invoices (or bills), each taking cash, an early-payment discount and a
// credit. QBD's auto-apply (oldest first) and the rules for how much a line
// may take live here, unit-tested; the component is wiring.

import { blankTxn } from '~/api/blank';
import { describeError, errorTargets, type ApiError } from '~/api/errors';
import type { Application, Txn, TxnSummary, TxnType } from '~/api/types';
import type { IsoDate } from '~/lib/dates';
import { sumCents, type Cents } from '~/lib/money';
import { noErrors, putGridError, type FormErrors } from './model';

export type PaymentType = 'ReceivePayment' | 'BillPaymentCheck' | 'BillPaymentCreditCard';

export function isPaymentType(t: TxnType): t is PaymentType {
  return t === 'ReceivePayment' || t === 'BillPaymentCheck' || t === 'BillPaymentCreditCard';
}

/** What a payment can pay: invoices and charges (receivable), bills (payable). */
export const TARGET_TYPES: Record<'receivable' | 'payable', TxnType[]> = {
  receivable: ['Invoice', 'StatementCharge'],
  payable: ['Bill'],
};

/** What can be applied instead of cash: credit memos / vendor credits, and unapplied payments. */
export const CREDIT_TYPES: Record<'receivable' | 'payable', TxnType[]> = {
  receivable: ['CreditMemo', 'ReceivePayment'],
  payable: ['VendorCredit', 'BillPaymentCheck', 'BillPaymentCreditCard'],
};

export interface AppRow {
  target_txn_id: string;
  /** The invoice or bill as listed (date, number, amount, open balance). */
  summary: TxnSummary;
  /** Cash applied to it on this payment. */
  amount: Cents;
  discount: Cents;
  discount_account_id: string | null;
  credit_txn_id: string | null;
  credit_amount: Cents;
}

export interface PaymentDraft {
  id: string;
  edit_seq: number;
  txn_type: PaymentType;
  entity_id: string | null;
  /** Amount received / paid; null while blank. */
  amount: Cents | null;
  date: IsoDate;
  /** Reference / check number. */
  number: string;
  numberIsAuto: boolean;
  memo: string;
  payment_method_id: string | null;
  /** Deposit to (receivable; null = Undeposited Funds) or pay from (payable: a bank or card). */
  account_id: string | null;
  to_be_printed: boolean;
  rows: AppRow[];
  base: Txn | null;
}

export function paymentKind(t: PaymentType): 'receivable' | 'payable' {
  return t === 'ReceivePayment' ? 'receivable' : 'payable';
}

export function newPaymentDraft(txn_type: PaymentType, date: IsoDate, accountId: string | null): PaymentDraft {
  return {
    id: crypto.randomUUID(),
    edit_seq: 0,
    txn_type,
    entity_id: null,
    amount: null,
    date,
    number: '',
    numberIsAuto: txn_type === 'BillPaymentCheck',
    memo: '',
    payment_method_id: null,
    account_id: accountId,
    to_be_printed: false,
    rows: [],
    base: null,
  };
}

/** A placeholder summary for a target the lists haven't supplied (a paid-off invoice on a saved payment). */
function placeholder(a: Application, type: TxnType): TxnSummary {
  return {
    id: a.target_txn_id,
    txn_type: type,
    date: '',
    number: null,
    entity_id: null,
    entity_name: null,
    account_id: null,
    account_name: null,
    amount: a.amount + a.discount + a.credit_amount,
    open_balance: 0,
    due_date: null,
    memo: null,
    is_void: false,
    is_pending: false,
    edit_seq: 0,
  };
}

export function paymentDraftFromTxn(txn: Txn): PaymentDraft {
  const type = isPaymentType(txn.txn_type) ? txn.txn_type : 'ReceivePayment';
  const targetType = TARGET_TYPES[paymentKind(type)][0]!;
  return {
    id: txn.id,
    edit_seq: txn.edit_seq,
    txn_type: type,
    entity_id: txn.entity_id,
    amount: txn.amount,
    date: txn.date,
    number: txn.number ?? '',
    numberIsAuto: false,
    memo: txn.memo ?? '',
    payment_method_id: txn.payment_method_id,
    account_id: txn.account_id,
    to_be_printed: txn.is_to_be_printed,
    rows: txn.applications.map((a) => ({
      target_txn_id: a.target_txn_id,
      summary: placeholder(a, targetType),
      amount: a.amount,
      discount: a.discount,
      discount_account_id: a.discount_account_id,
      credit_txn_id: a.credit_txn_id,
      credit_amount: a.credit_amount,
    })),
    base: txn,
  };
}

/** What this payment already applied to a target when it was saved (so editing can re-apply it). */
function priorApplied(d: PaymentDraft, targetId: string): Cents {
  const a = d.base?.applications.find((x) => x.target_txn_id === targetId);
  return a ? a.amount + a.discount + a.credit_amount : 0;
}

/** The most a row may take in cash, discount and credit together: what's open plus what this payment already paid. */
export function availableFor(d: PaymentDraft, row: AppRow): Cents {
  return (row.summary.open_balance ?? 0) + priorApplied(d, row.target_txn_id);
}

/**
 * Bring the grid in step with the name's open items: rows for new targets
 * (amount 0), existing amounts kept, rows for targets no longer open dropped
 * unless this payment is what paid them.
 */
export function syncTargets(d: PaymentDraft, targets: readonly TxnSummary[]): PaymentDraft {
  const byId = new Map(targets.map((t) => [t.id, t]));
  const kept = d.rows.filter((r) => byId.has(r.target_txn_id) || priorApplied(d, r.target_txn_id) > 0).map((r) => ({ ...r, summary: byId.get(r.target_txn_id) ?? r.summary }));
  const have = new Set(kept.map((r) => r.target_txn_id));
  const added = targets
    .filter((t) => !have.has(t.id) && (t.open_balance ?? 0) > 0)
    .map<AppRow>((t) => ({ target_txn_id: t.id, summary: t, amount: 0, discount: 0, discount_account_id: null, credit_txn_id: null, credit_amount: 0 }));
  const rows = [...kept, ...added].sort((a, b) => (a.summary.date < b.summary.date ? -1 : a.summary.date > b.summary.date ? 1 : 0));
  return { ...d, rows };
}

export function appliedCash(d: PaymentDraft): Cents {
  return sumCents(d.rows.map((r) => r.amount));
}

/** Received but not applied: becomes a credit on the name (QBD's "Unapplied Amount"). */
export function unapplied(d: PaymentDraft): Cents {
  return (d.amount ?? 0) - appliedCash(d);
}

/** QBD's auto-apply: the amount goes to the oldest open items first, respecting discounts and credits already set. */
export function autoApply(d: PaymentDraft): PaymentDraft {
  let left = d.amount ?? 0;
  const rows = d.rows.map((r) => {
    const room = Math.max(0, availableFor(d, r) - r.discount - r.credit_amount);
    const take = Math.max(0, Math.min(room, left));
    left -= take;
    return { ...r, amount: take };
  });
  return { ...d, rows };
}

/** Setting the amount: with nothing applied by hand yet, auto-apply (when the company wants that). */
export function setPaymentAmount(d: PaymentDraft, amount: Cents | null, autoApplyPref: boolean): PaymentDraft {
  const next = { ...d, amount };
  const touched = d.rows.some((r) => r.amount > 0);
  return autoApplyPref && !touched ? autoApply(next) : next;
}

/** Select / unselect a target: take what it needs (or all that's left), or nothing. */
export function toggleRow(d: PaymentDraft, i: number, autoApplyPref: boolean): PaymentDraft {
  const row = d.rows[i]!;
  const rows = [...d.rows];
  if (row.amount > 0) rows[i] = { ...row, amount: 0 };
  else {
    const room = Math.max(0, availableFor(d, row) - row.discount - row.credit_amount);
    const others = sumCents(rows.filter((_, j) => j !== i).map((r) => r.amount));
    const left = d.amount === null ? room : Math.max(0, d.amount - others);
    rows[i] = { ...row, amount: Math.min(room, left) };
  }
  const next = { ...d, rows };
  // With no amount typed, the payment is the sum of what was selected.
  return d.amount === null || !autoApplyPref ? { ...next, amount: d.amount === null ? appliedCash(next) || null : d.amount } : next;
}

export function setRowAmount(d: PaymentDraft, i: number, amount: Cents | null): PaymentDraft {
  const rows = [...d.rows];
  rows[i] = { ...rows[i]!, amount: amount ?? 0 };
  return { ...d, rows };
}

export function setRowDiscount(d: PaymentDraft, i: number, discount: Cents | null, accountId: string | null): PaymentDraft {
  const rows = [...d.rows];
  rows[i] = { ...rows[i]!, discount: discount ?? 0, discount_account_id: accountId };
  return { ...d, rows };
}

/** Use a credit (a credit memo, an unapplied payment) against a target instead of cash. */
export function setRowCredit(d: PaymentDraft, i: number, creditId: string | null, amount: Cents): PaymentDraft {
  const rows = [...d.rows];
  rows[i] = { ...rows[i]!, credit_txn_id: amount > 0 ? creditId : null, credit_amount: amount > 0 && creditId ? amount : 0 };
  return { ...d, rows };
}

export function validatePayment(d: PaymentDraft): FormErrors {
  const e = noErrors();
  const kind = paymentKind(d.txn_type);
  if (!d.entity_id) e.header.entity = kind === 'receivable' ? 'Choose the customer who paid' : 'Choose the vendor';
  if (!d.date) e.header.date = 'Enter a date';
  if (kind === 'payable' && !d.account_id) e.header.account = 'Choose the account to pay from';
  const cash = appliedCash(d);
  const anyCredit = d.rows.some((r) => r.credit_amount > 0 || r.discount > 0);
  if ((d.amount ?? 0) <= 0 && !anyCredit) e.header.amount = 'Enter the amount';
  if (cash > (d.amount ?? 0)) e.header.amount = `Applied ${centsText(cash)} is more than the amount received`;
  d.rows.forEach((r, i) => {
    const take = r.amount + r.discount + r.credit_amount;
    if (take > availableFor(d, r)) putGridError(e, 'rows', i, 'amount', `Only ${centsText(availableFor(d, r))} is open on this one`);
    if (r.discount > 0 && !r.discount_account_id) putGridError(e, 'rows', i, 'discount', 'Choose the discount account');
  });
  return e;
}

function centsText(c: Cents): string {
  const neg = c < 0 ? '-' : '';
  const abs = Math.abs(c);
  return `${neg}${Math.trunc(abs / 100).toLocaleString()}.${String(abs % 100).padStart(2, '0')}`;
}

export function paymentDraftToTxn(d: PaymentDraft): Txn {
  const base = d.base ?? blankTxn(d.txn_type, d.date);
  return {
    ...base,
    id: d.id,
    txn_type: d.txn_type,
    edit_seq: d.edit_seq,
    entity_id: d.entity_id,
    amount: d.amount ?? 0,
    date: d.date,
    number: d.to_be_printed && d.txn_type === 'BillPaymentCheck' ? null : d.number.trim() || null,
    memo: d.memo.trim() || null,
    payment_method_id: d.payment_method_id,
    account_id: d.account_id,
    is_to_be_printed: d.txn_type === 'BillPaymentCheck' ? d.to_be_printed : base.is_to_be_printed,
    lines: [],
    applications: d.rows
      .filter((r) => r.amount > 0 || r.discount > 0 || r.credit_amount > 0)
      .map((r) => ({
        target_txn_id: r.target_txn_id,
        amount: r.amount,
        discount: r.discount,
        discount_account_id: r.discount > 0 ? r.discount_account_id : null,
        discount_class_id: null,
        credit_txn_id: r.credit_amount > 0 ? r.credit_txn_id : null,
        credit_amount: r.credit_amount,
      })),
  };
}

export function paymentServerErrors(d: PaymentDraft, sent: Txn, err: ApiError): FormErrors {
  const e = noErrors();
  const msg = describeError(err);
  const pe = err.postingError;
  if (pe && (pe.code === 'UnknownTarget' || pe.code === 'TargetForOtherName' || pe.code === 'TargetInOtherAccount')) {
    const i = d.rows.findIndex((r) => r.target_txn_id === pe.detail);
    if (i >= 0) putGridError(e, 'rows', i, 'amount', msg);
    else e.form.push(msg);
    return e;
  }
  if (pe?.code === 'OverApplied') {
    e.header.amount = msg;
    return e;
  }
  for (const t of errorTargets(sent, err)) {
    if (t.field === 'line' || t.field === 'lines' || t.field === 'form' || t.field === 'to_account') e.form.push(msg);
    else e.header[t.field] = msg;
  }
  return e;
}

export function isPaymentEmpty(d: PaymentDraft): boolean {
  return !d.entity_id && !d.amount && !d.memo.trim() && d.rows.every((r) => r.amount === 0 && r.credit_amount === 0);
}

export function paymentFingerprint(d: PaymentDraft): string {
  const t = paymentDraftToTxn(d);
  return JSON.stringify([t.entity_id, t.amount, t.date, t.number, t.memo, t.payment_method_id, t.account_id, t.is_to_be_printed, t.applications]);
}

export function nextPaymentDraft(d: PaymentDraft): PaymentDraft {
  return newPaymentDraft(d.txn_type, d.date, d.account_id);
}
