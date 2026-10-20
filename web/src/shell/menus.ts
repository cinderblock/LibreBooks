// QBD's menu bar as data, shared by the desktop menu bar, the mobile sheets and
// the command palette. An item either runs a keyboard command (so its shortcut
// shows next to it) or opens a company-relative path.

import type { CommandId } from '~/keyboard/commands';
import { paths } from '~/windows/paths';

export interface MenuItem {
  label: string;
  command?: CommandId;
  path?: string;
  /** Not built yet: shown dimmed, opens a placeholder explaining so. */
  todo?: boolean;
  keywords?: string[];
}

export type MenuEntry = MenuItem | { divider: true } | { label: string; children: MenuItem[] };

export interface Menu {
  label: string;
  entries: MenuEntry[];
}

/** Every form slug (`/forms/<slug>/new`) and its menu label. */
export const FORM_LABELS: Record<string, string> = {
  invoice: 'Create Invoices',
  'sales-receipt': 'Enter Sales Receipts',
  'receive-payment': 'Receive Payments',
  'credit-memo': 'Create Credit Memos/Refunds',
  estimate: 'Create Estimates',
  bill: 'Enter Bills',
  'pay-bills': 'Pay Bills',
  'vendor-credit': 'Enter Vendor Credits',
  'purchase-order': 'Create Purchase Orders',
  'receive-items': 'Receive Items',
  deposit: 'Make Deposits',
  transfer: 'Transfer Funds',
  'cc-charge': 'Enter Credit Card Charges',
  'cc-credit': 'Enter Credit Card Credits',
  journal: 'Make General Journal Entries',
  reconcile: 'Reconcile',
  'bank-feeds': 'Bank Feeds',
  'pay-employees': 'Pay Employees',
  memorized: 'Memorized Transaction List',
};

/** Slugs with a real form; the rest open a placeholder naming them. */
const BUILT_FORMS = new Set(['bill', 'vendor-credit', 'purchase-order', 'receive-items', 'cc-charge', 'cc-credit']);

const form = (key: keyof typeof FORM_LABELS & string, extra: Partial<MenuItem> = {}): MenuItem => ({
  label: FORM_LABELS[key]!,
  path: paths.form(key),
  todo: !BUILT_FORMS.has(key),
  ...extra,
});

export const MENUS: Menu[] = [
  {
    label: 'Company',
    entries: [
      { label: 'Home Page', command: 'nav.home', path: paths.home() },
      { label: 'Company Information', path: paths.company() },
      { divider: true },
      { label: 'Chart of Accounts', command: 'nav.chartOfAccounts', path: paths.accounts() },
      form('journal', { command: 'form.journal' }),
      { divider: true },
      { label: 'Find…', command: 'nav.find', path: paths.find() },
      { label: 'Search / Go To…', command: 'palette.open' },
    ],
  },
  {
    label: 'Customers',
    entries: [
      { label: 'Customer Center', command: 'nav.customerCenter', path: paths.customers() },
      { divider: true },
      form('invoice', { command: 'form.invoice' }),
      form('sales-receipt', { command: 'form.salesReceipt' }),
      form('receive-payment', { command: 'form.receivePayment' }),
      form('credit-memo', { command: 'form.creditMemo' }),
      form('estimate', { command: 'form.estimate' }),
    ],
  },
  {
    label: 'Vendors',
    entries: [
      { label: 'Vendor Center', command: 'nav.vendorCenter', path: paths.vendors() },
      { divider: true },
      form('bill', { command: 'form.bill', keywords: ['payable', 'vendor'] }),
      form('pay-bills', { command: 'form.payBills' }),
      form('vendor-credit', { command: 'form.vendorCredit' }),
      form('purchase-order', { command: 'form.purchaseOrder', keywords: ['po'] }),
      form('receive-items'),
    ],
  },
  {
    label: 'Employees',
    entries: [{ label: 'Employee Center', path: paths.employees() }, { divider: true }, form('pay-employees')],
  },
  {
    label: 'Banking',
    entries: [
      { label: 'Write Checks', command: 'form.writeChecks', path: paths.newCheck(), keywords: ['check', 'pay', 'expense'] },
      form('deposit', { command: 'form.deposit' }),
      form('transfer', { command: 'form.transfer' }),
      form('cc-charge', { command: 'form.ccCharge', keywords: ['credit card', 'visa'] }),
      form('cc-credit'),
      { divider: true },
      { label: 'Use Register', command: 'nav.useRegister' },
      { label: 'Banking Overview', path: paths.banking() },
      form('reconcile', { command: 'nav.reconcile' }),
      form('bank-feeds', { command: 'nav.bankFeeds', keywords: ['download', 'online banking', 'import'] }),
    ],
  },
  {
    label: 'Reports',
    entries: [
      { label: 'Report Center', command: 'nav.reports', path: paths.reports() },
      { divider: true },
      {
        label: 'Company & Financial',
        children: [
          { label: 'Profit & Loss Standard', path: paths.report('ProfitAndLoss'), keywords: ['p&l', 'income statement'] },
          { label: 'Profit & Loss Detail', path: paths.report('ProfitAndLossDetail') },
          { label: 'Profit & Loss by Class', path: paths.report('ProfitAndLossByClass') },
          { label: 'Balance Sheet Standard', path: paths.report('BalanceSheet') },
          { label: 'Statement of Cash Flows', path: paths.report('StatementOfCashFlows') },
        ],
      },
      {
        label: 'Customers & Receivables',
        children: [
          { label: 'A/R Aging Summary', path: paths.report('ArAgingSummary') },
          { label: 'Customer Balance Summary', path: paths.report('CustomerBalanceSummary') },
          { label: 'Open Invoices', path: paths.report('OpenInvoices') },
        ],
      },
      {
        label: 'Vendors & Payables',
        children: [
          { label: 'A/P Aging Summary', path: paths.report('ApAgingSummary') },
          { label: 'Vendor Balance Summary', path: paths.report('VendorBalanceSummary') },
          { label: 'Unpaid Bills Detail', path: paths.report('UnpaidBillsDetail') },
        ],
      },
      {
        label: 'Banking',
        children: [
          { label: 'Deposit Detail', path: paths.report('DepositDetail') },
          { label: 'Check Detail', path: paths.report('CheckDetail') },
          { label: 'Missing Checks', path: paths.report('MissingChecks') },
        ],
      },
      {
        label: 'Accountant & Taxes',
        children: [
          { label: 'Trial Balance', path: paths.report('TrialBalance') },
          { label: 'General Ledger', path: paths.report('GeneralLedger') },
          { label: 'Journal', path: paths.report('Journal') },
          { label: 'Transaction List by Date', path: paths.report('TransactionListByDate') },
          { label: 'Audit Trail', path: paths.report('AuditTrail') },
        ],
      },
    ],
  },
  {
    label: 'Lists',
    entries: [
      { label: 'Chart of Accounts', command: 'nav.chartOfAccounts', path: paths.accounts() },
      { label: 'Item List', path: paths.list('items') },
      { label: 'Class List', path: paths.list('classes') },
      { label: 'Terms List', path: paths.list('terms') },
      { label: 'Payment Method List', path: paths.list('payment-methods') },
      { label: 'Other Names List', path: paths.list('other-names') },
      { divider: true },
      form('memorized', { command: 'nav.memorizedTxns' }),
    ],
  },
];

/** Every leaf item, flattened (for the palette). */
export function allMenuItems(): Array<MenuItem & { menu: string }> {
  const out: Array<MenuItem & { menu: string }> = [];
  for (const m of MENUS) {
    for (const e of m.entries) {
      if ('divider' in e) continue;
      if ('children' in e) for (const c of e.children) out.push({ ...c, menu: `${m.label} › ${e.label}` });
      else out.push({ ...e, menu: m.label });
    }
  }
  return out;
}

/** Desktop left navigation (QBD's icon bar), also the mobile "More" sheet. */
export const NAV_ITEMS = [
  { label: 'Home', path: paths.home(), kinds: ['home'], icon: 'home' },
  { label: 'Customers', path: paths.customers(), kinds: ['customers'], icon: 'customers' },
  { label: 'Vendors', path: paths.vendors(), kinds: ['vendors'], icon: 'vendors' },
  { label: 'Employees', path: paths.employees(), kinds: ['employees'], icon: 'employees' },
  { label: 'Banking', path: paths.banking(), kinds: ['banking', 'register', 'check'], icon: 'banking' },
  { label: 'Reports', path: paths.reports(), kinds: ['reports', 'report'], icon: 'reports' },
  { label: 'Lists', path: paths.accounts(), kinds: ['accounts', 'list'], icon: 'lists' },
  { label: 'Company', path: paths.company(), kinds: ['company'], icon: 'company' },
] as const;
