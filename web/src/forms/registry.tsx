// Which form renders which kind of transaction. A form window's URL names a
// slug (`/forms/bill/new`); a saved transaction's window (`/txns/:id`) picks the
// form by the transaction's type. Forms not built yet have no entry and fall
// back to the placeholder (new) or the read-only view (saved).

import type { ReactNode } from 'react';
import type { Txn, TxnType } from '~/api/types';
import { PURCHASE_FORM_SLUG, PurchaseForm } from './PurchaseForm';
import type { PurchaseType } from './purchaseDraft';
import { SalesForm } from './SalesForm';
import type { SalesType } from './salesDraft';
import { PaymentForm } from './PaymentForm';
import { PayBills } from './PayBills';
import { DepositForm } from './DepositForm';
import { TransferForm } from './TransferForm';
import { JournalForm } from './JournalForm';

export interface FormDef {
  label: string;
  /** Render the form for a saved transaction, a hand-off, or neither (a new one). */
  render(saved: Txn | null, initial: Txn | null): ReactNode;
  /** Transaction types this form edits. */
  types: readonly TxnType[];
}

const purchase = (type: PurchaseType, label: string): FormDef => ({
  label,
  types: [type],
  render: (saved, initial) => <PurchaseForm type={type} saved={saved} initial={initial} />,
});

const sales = (type: SalesType, label: string): FormDef => ({
  label,
  types: [type],
  render: (saved, initial) => <SalesForm type={type} saved={saved} initial={initial} />,
});

/** Slug → form. The Check form also answers at `/checks/new`. */
export const FORMS: Record<string, FormDef> = {
  check: purchase('Check', 'Write Checks'),
  bill: purchase('Bill', 'Enter Bills'),
  'vendor-credit': purchase('VendorCredit', 'Enter Vendor Credits'),
  'cc-charge': purchase('CreditCardCharge', 'Enter Credit Card Charges'),
  'cc-credit': purchase('CreditCardCredit', 'Enter Credit Card Credits'),
  'receive-items': purchase('ItemReceipt', 'Receive Items'),
  'purchase-order': purchase('PurchaseOrder', 'Create Purchase Orders'),
  invoice: sales('Invoice', 'Create Invoices'),
  'sales-receipt': sales('SalesReceipt', 'Enter Sales Receipts'),
  'credit-memo': sales('CreditMemo', 'Create Credit Memos/Refunds'),
  estimate: sales('Estimate', 'Create Estimates'),
  'receive-payment': { label: 'Receive Payments', types: ['ReceivePayment'], render: (saved, initial) => <PaymentForm type="ReceivePayment" saved={saved} initial={initial} /> },
  // Pay Bills writes bill payments; a saved one opens in the payment form.
  'pay-bills': {
    label: 'Pay Bills',
    types: ['BillPaymentCheck', 'BillPaymentCreditCard'],
    render: (saved, initial) => (saved || initial ? <PaymentForm type={(saved ?? initial)!.txn_type === 'BillPaymentCreditCard' ? 'BillPaymentCreditCard' : 'BillPaymentCheck'} saved={saved} initial={initial} /> : <PayBills />),
  },
  deposit: { label: 'Make Deposits', types: ['Deposit'], render: (saved, initial) => <DepositForm saved={saved} initial={initial} /> },
  transfer: { label: 'Transfer Funds', types: ['Transfer'], render: (saved, initial) => <TransferForm saved={saved} initial={initial} /> },
  journal: { label: 'Make General Journal Entries', types: ['JournalEntry'], render: (saved, initial) => <JournalForm saved={saved} initial={initial} /> },
};

/** The slug whose form edits a transaction of this type, if one exists. */
export function formSlugFor(type: TxnType): string | null {
  for (const [slug, def] of Object.entries(FORMS)) if (def.types.includes(type)) return slug;
  return null;
}

export function formFor(type: TxnType): FormDef | null {
  const slug = formSlugFor(type);
  return slug ? (FORMS[slug] ?? null) : null;
}

export { PURCHASE_FORM_SLUG };
