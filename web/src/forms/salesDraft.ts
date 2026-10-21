// The sales-side forms' state as plain data: Create Invoices, Enter Sales
// Receipts, Create Credit Memos and Create Estimates share a header (the
// customer:job, dates, terms, addresses, sales tax) over item lines. Build a
// draft from a saved transaction or from scratch, apply the customer's defaults,
// validate before sending, turn it into a `Txn`, and map a server error back
// onto its fields.

import { blankAddress, blankTxn } from '~/api/blank';
import { describeError, errorTargets, type ApiError } from '~/api/errors';
import type { Address, Entity, Item, SalesTaxCode, Terms, Txn, TxnType } from '~/api/types';
import type { IsoDate } from '~/lib/dates';
import type { Cents } from '~/lib/money';
import { withTrailingBlank } from '~/inputs/gridRows';
import { noErrors, putGridError, type FormErrors } from './model';
import { blankSaleRow, estimatedTax, fromSaleLines, isBlankSale, locateSaleLine, recomputeSales, salesSubtotal, taxableAmount, toSaleLines, type SaleRow } from './salesLines';
import { dueDate } from './terms';

export type SalesType = 'Invoice' | 'SalesReceipt' | 'CreditMemo' | 'Estimate' | 'SalesOrder' | 'StatementCharge';

export const SALES_TYPES: readonly SalesType[] = ['Invoice', 'SalesReceipt', 'CreditMemo', 'Estimate', 'SalesOrder', 'StatementCharge'];

export function isSalesType(t: TxnType): t is SalesType {
  return (SALES_TYPES as readonly TxnType[]).includes(t);
}

/** Estimates and sales orders are kept with the same machinery but move no money. */
export function isNonPostingSale(t: SalesType): boolean {
  return t === 'Estimate' || t === 'SalesOrder';
}

/** Forms with terms and a due date: an invoice and a statement charge are owed; the rest are not. */
export function hasDueDate(t: SalesType): boolean {
  return t === 'Invoice' || t === 'StatementCharge';
}

export interface SalesDraft {
  id: string;
  edit_seq: number;
  txn_type: SalesType;
  entity_id: string | null;
  class_id: string | null;
  date: IsoDate;
  number: string;
  numberIsAuto: boolean;
  po_number: string;
  terms_id: string | null;
  due_date: IsoDate | null;
  sales_rep_id: string | null;
  ship_date: IsoDate | null;
  bill_address: Address;
  ship_address: Address;
  sales_tax_item_id: string | null;
  customer_tax_code_id: string | null;
  /** "Customer message" printed on the form. */
  message: string;
  memo: string;
  to_be_printed: boolean;
  to_be_emailed: boolean;
  /** Sales receipts: deposit to this account, or Undeposited Funds when null. */
  account_id: string | null;
  payment_method_id: string | null;
  lines: SaleRow[];
  base: Txn | null;
}

export interface SalesDefaults {
  /** The company's sales tax item (a form starts with it). */
  taxItemId: string | null;
  /** The taxable code ("Tax") new lines get. */
  taxCodeId: string | null;
  /** Deposit sales receipts to Undeposited Funds (null) or an account. */
  depositTo: string | null;
}

export function newSalesDraft(txn_type: SalesType, date: IsoDate, defaults: SalesDefaults): SalesDraft {
  return {
    id: crypto.randomUUID(),
    edit_seq: 0,
    txn_type,
    entity_id: null,
    class_id: null,
    date,
    number: '',
    numberIsAuto: true,
    po_number: '',
    terms_id: null,
    due_date: null,
    sales_rep_id: null,
    ship_date: null,
    bill_address: blankAddress(),
    ship_address: blankAddress(),
    sales_tax_item_id: defaults.taxItemId,
    customer_tax_code_id: defaults.taxCodeId,
    message: '',
    memo: '',
    to_be_printed: false,
    to_be_emailed: false,
    account_id: txn_type === 'SalesReceipt' ? defaults.depositTo : null,
    payment_method_id: null,
    lines: [blankSaleRow()],
    base: null,
  };
}

export function salesDraftFromTxn(txn: Txn): SalesDraft {
  return {
    id: txn.id,
    edit_seq: txn.edit_seq,
    txn_type: isSalesType(txn.txn_type) ? txn.txn_type : 'Invoice',
    entity_id: txn.entity_id,
    class_id: txn.class_id,
    date: txn.date,
    number: txn.number ?? '',
    // An unsaved hand-off without a number (an invoice drawn from an order) takes the next one.
    numberIsAuto: txn.edit_seq === 0 && !txn.number,
    po_number: txn.po_number ?? '',
    terms_id: txn.terms_id,
    due_date: txn.due_date,
    sales_rep_id: txn.sales_rep_id,
    ship_date: txn.ship_date,
    bill_address: txn.bill_address,
    ship_address: txn.ship_address,
    sales_tax_item_id: txn.sales_tax_item_id,
    customer_tax_code_id: txn.customer_tax_code_id,
    message: txn.message ?? '',
    memo: txn.memo ?? '',
    to_be_printed: txn.is_to_be_printed,
    to_be_emailed: txn.is_to_be_emailed,
    account_id: txn.account_id,
    payment_method_id: txn.payment_method_id,
    lines: withTrailingBlank(fromSaleLines(txn.lines), isBlankSale, blankSaleRow),
    base: txn,
  };
}

/** Choosing a customer brings its terms (→ due date), tax code, rep and addresses. */
export function applyCustomer(d: SalesDraft, customer: Entity | undefined, terms: Terms | null | undefined, parent?: Entity): SalesDraft {
  if (!customer) return { ...d, entity_id: null };
  // A job's addresses and terms come from the customer above it.
  const src = customer.parent_id && parent ? parent : customer;
  const termsId = customer.terms_id ?? src.terms_id ?? d.terms_id;
  return {
    ...d,
    entity_id: customer.id,
    terms_id: termsId,
    due_date: hasDueDate(d.txn_type) ? (dueDate(terms, d.date) ?? d.due_date) : d.due_date,
    customer_tax_code_id: customer.sales_tax_code_id ?? src.sales_tax_code_id ?? d.customer_tax_code_id,
    sales_rep_id: customer.sales_rep_id ?? src.sales_rep_id ?? d.sales_rep_id,
    bill_address: hasAddress(d.bill_address) ? d.bill_address : withName(src, src.bill_address),
    ship_address: hasAddress(d.ship_address) ? d.ship_address : withName(src, hasAddress(src.ship_address) ? src.ship_address : src.bill_address),
  };
}

function hasAddress(a: Address): boolean {
  return (a.lines?.length ?? 0) > 0 || !!a.city || !!a.postal_code;
}

/** QBD prints the name on the first address line. */
function withName(e: Entity, a: Address): Address {
  const name = e.company_name ?? e.full_name.split(':')[0] ?? e.name;
  const lines = a.lines ?? [];
  return { ...a, lines: lines[0] === name ? lines : [name, ...lines] };
}

export function applySalesTerms(d: SalesDraft, terms: Terms | null | undefined): SalesDraft {
  return { ...d, terms_id: terms?.id ?? null, due_date: dueDate(terms, d.date) ?? d.due_date };
}

export interface SalesTotals {
  subtotal: Cents;
  taxable: Cents;
  tax: Cents;
  total: Cents;
}

/** Client-side totals (the server's preview is authoritative; this keeps them live while typing). */
export function salesTotals(d: SalesDraft, items: ReadonlyMap<string, Item>, codes: ReadonlyMap<string, SalesTaxCode>): SalesTotals {
  const subtotal = salesSubtotal(d.lines, items);
  const taxable = taxableAmount(d.lines, items, codes);
  const customerTaxable = d.customer_tax_code_id ? (codes.get(d.customer_tax_code_id)?.is_taxable ?? true) : true;
  const tax = estimatedTax(taxable, d.sales_tax_item_id ? items.get(d.sales_tax_item_id) : undefined, items, customerTaxable);
  return { subtotal, taxable, tax, total: subtotal + tax };
}

/** Lines after an edit: derived amounts recomputed, one trailing blank line. */
export function setSaleLines(d: SalesDraft, lines: SaleRow[], items: ReadonlyMap<string, Item>): SalesDraft {
  return { ...d, lines: withTrailingBlank(recomputeSales(lines, items), isBlankSale, blankSaleRow) };
}

export function salesDraftToTxn(d: SalesDraft, items: ReadonlyMap<string, Item>, codes: ReadonlyMap<string, SalesTaxCode>): Txn {
  const base = d.base ?? blankTxn(d.txn_type, d.date);
  const totals = salesTotals(d, items, codes);
  return {
    ...base,
    id: d.id,
    txn_type: d.txn_type,
    edit_seq: d.edit_seq,
    entity_id: d.entity_id,
    class_id: d.class_id,
    date: d.date,
    number: d.number.trim() || null,
    po_number: d.po_number.trim() || null,
    terms_id: d.terms_id,
    due_date: hasDueDate(d.txn_type) ? d.due_date : null,
    sales_rep_id: d.sales_rep_id,
    ship_date: d.ship_date,
    bill_address: d.bill_address,
    ship_address: d.ship_address,
    sales_tax_item_id: d.sales_tax_item_id,
    customer_tax_code_id: d.customer_tax_code_id,
    message: d.message.trim() || null,
    memo: d.memo.trim() || null,
    is_to_be_printed: d.to_be_printed,
    is_to_be_emailed: d.to_be_emailed,
    account_id: d.txn_type === 'SalesReceipt' ? d.account_id : null,
    payment_method_id: d.txn_type === 'SalesReceipt' ? d.payment_method_id : base.payment_method_id,
    amount: totals.total,
    lines: toSaleLines(d.lines),
  };
}

export function validateSales(d: SalesDraft): FormErrors {
  const e = noErrors();
  if (!d.entity_id) e.header.entity = 'Choose a customer or job';
  if (!d.date) e.header.date = 'Enter a date';
  d.lines.forEach((r, i) => {
    if (isBlankSale(r)) return;
    if (!r.item_id) putGridError(e, 'lines', i, 'item', 'Choose an item for this line');
  });
  if (!d.lines.some((r) => !isBlankSale(r))) e.form.push('Enter at least one line.');
  return e;
}

export function salesServerErrors(d: SalesDraft, sent: Txn, err: ApiError): FormErrors {
  const e = noErrors();
  const msg = describeError(err);
  for (const t of errorTargets(sent, err)) {
    switch (t.field) {
      case 'line': {
        const row = locateSaleLine(d.lines, t.index);
        if (row === null) e.form.push(msg);
        else putGridError(e, 'lines', row, t.column === 'any' || t.column === 'account' || t.column === 'entity' ? 'item' : t.column, msg);
        break;
      }
      case 'lines':
      case 'form':
      case 'to_account':
        e.form.push(msg);
        break;
      default:
        e.header[t.field] = msg;
    }
  }
  return e;
}

export function isSalesEmpty(d: SalesDraft): boolean {
  return !d.entity_id && !d.memo.trim() && !d.message.trim() && d.lines.every(isBlankSale);
}

export function salesFingerprint(d: SalesDraft): string {
  return JSON.stringify([
    d.entity_id,
    d.class_id,
    d.date,
    d.number.trim(),
    d.po_number.trim(),
    d.terms_id,
    d.due_date,
    d.sales_rep_id,
    d.ship_date,
    d.bill_address,
    d.ship_address,
    d.sales_tax_item_id,
    d.customer_tax_code_id,
    d.message.trim(),
    d.memo.trim(),
    d.to_be_printed,
    d.to_be_emailed,
    d.account_id,
    d.payment_method_id,
    toSaleLines(d.lines).map((l) => [l.item_id, l.description, l.quantity, l.rate, l.amount, l.sales_tax_code_id, l.class_id]),
  ]);
}

export function nextSalesDraft(d: SalesDraft, defaults: SalesDefaults): SalesDraft {
  return newSalesDraft(d.txn_type, d.date, { ...defaults, taxItemId: d.sales_tax_item_id ?? defaults.taxItemId });
}
