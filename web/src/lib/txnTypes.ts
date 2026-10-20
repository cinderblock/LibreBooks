// Transaction-type facts the UI needs, mirroring `crates/lb-core/src/txn.rs`.

import type { HeaderEntity } from '~/api/generated/HeaderEntity';
import type { ItemType } from '~/api/generated/ItemType';
import type { TxnType } from '~/api/generated/TxnType';

export const TXN_TYPES: readonly TxnType[] = [
  'Check',
  'Deposit',
  'Transfer',
  'JournalEntry',
  'Bill',
  'BillPaymentCheck',
  'BillPaymentCreditCard',
  'VendorCredit',
  'ItemReceipt',
  'CreditCardCharge',
  'CreditCardCredit',
  'PurchaseOrder',
  'Invoice',
  'SalesReceipt',
  'ReceivePayment',
  'CreditMemo',
  'StatementCharge',
  'Estimate',
  'SalesOrder',
  'InventoryAdjustment',
  'BuildAssembly',
  'SalesTaxPaymentCheck',
  'Paycheck',
  'LiabilityCheck',
  'TimeTracking',
];

/** QBD's register "Type" column abbreviations. */
export const TXN_REGISTER_CODE: Record<TxnType, string> = {
  Check: 'CHK',
  Deposit: 'DEP',
  Transfer: 'TRANSFR',
  JournalEntry: 'GENJRNL',
  Bill: 'BILL',
  BillPaymentCheck: 'BILLPMT',
  BillPaymentCreditCard: 'BILLPMT',
  VendorCredit: 'BILLCRED',
  ItemReceipt: 'ITEM RCPT',
  CreditCardCharge: 'CC',
  CreditCardCredit: 'CC CRED',
  PurchaseOrder: 'PURCHORD',
  Invoice: 'INV',
  SalesReceipt: 'RCPT',
  ReceivePayment: 'PMT',
  CreditMemo: 'CREDMEM',
  StatementCharge: 'STMTCHG',
  Estimate: 'EST',
  SalesOrder: 'SALESORD',
  InventoryAdjustment: 'INVADJ',
  BuildAssembly: 'BUILD',
  SalesTaxPaymentCheck: 'TAXPMT',
  Paycheck: 'PAY CHK',
  LiabilityCheck: 'LIAB CHK',
  TimeTracking: 'TIME',
};

/** Names as reports and menus show them. */
export const TXN_LABEL: Record<TxnType, string> = {
  Check: 'Check',
  Deposit: 'Deposit',
  Transfer: 'Transfer',
  JournalEntry: 'General Journal',
  Bill: 'Bill',
  BillPaymentCheck: 'Bill Pmt -Check',
  BillPaymentCreditCard: 'Bill Pmt -CCard',
  VendorCredit: 'Credit',
  ItemReceipt: 'Item Receipt',
  CreditCardCharge: 'Credit Card Charge',
  CreditCardCredit: 'Credit Card Credit',
  PurchaseOrder: 'Purchase Order',
  Invoice: 'Invoice',
  SalesReceipt: 'Sales Receipt',
  ReceivePayment: 'Payment',
  CreditMemo: 'Credit Memo',
  StatementCharge: 'Statement Charge',
  Estimate: 'Estimate',
  SalesOrder: 'Sales Order',
  InventoryAdjustment: 'Inventory Adjust',
  BuildAssembly: 'Build Assembly',
  SalesTaxPaymentCheck: 'Sales Tax Payment',
  Paycheck: 'Paycheck',
  LiabilityCheck: 'Liability Check',
  TimeTracking: 'Time',
};

export function isPosting(t: TxnType): boolean {
  return !(t === 'PurchaseOrder' || t === 'Estimate' || t === 'SalesOrder' || t === 'TimeTracking');
}

/** Which name list the header's name comes from (Rust's `header_entity`). */
export function headerEntity(t: TxnType): HeaderEntity {
  switch (t) {
    case 'Invoice':
    case 'SalesReceipt':
    case 'ReceivePayment':
    case 'CreditMemo':
    case 'StatementCharge':
    case 'Estimate':
    case 'SalesOrder':
      return 'Customer';
    case 'Bill':
    case 'BillPaymentCheck':
    case 'BillPaymentCreditCard':
    case 'VendorCredit':
    case 'ItemReceipt':
    case 'PurchaseOrder':
    case 'SalesTaxPaymentCheck':
    case 'LiabilityCheck':
      return 'Vendor';
    case 'Paycheck':
      return 'Employee';
    case 'Check':
    case 'CreditCardCharge':
    case 'CreditCardCredit':
    case 'TimeTracking':
      return 'Any';
    case 'Deposit':
    case 'Transfer':
    case 'JournalEntry':
    case 'InventoryAdjustment':
    case 'BuildAssembly':
      return 'None';
  }
}

export const ITEM_TYPE_LABEL: Record<ItemType, string> = {
  Service: 'Service',
  Inventory: 'Inventory Part',
  InventoryAssembly: 'Inventory Assembly',
  NonInventory: 'Non-inventory Part',
  OtherCharge: 'Other Charge',
  Subtotal: 'Subtotal',
  Group: 'Group',
  Discount: 'Discount',
  Payment: 'Payment',
  SalesTax: 'Sales Tax Item',
  SalesTaxGroup: 'Sales Tax Group',
  FixedAsset: 'Fixed Asset',
};
