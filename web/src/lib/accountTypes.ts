// Account-type facts the UI needs, mirroring `crates/lb-core/src/account.rs`
// (class, debit-normal, labels, display order) plus register column headings
// the way QBD writes them.

import type { AccountClass } from '~/api/generated/AccountClass';
import type { AccountType } from '~/api/generated/AccountType';

/** Display order on the chart of accounts and reports (Rust's `sort_order`). */
export const ACCOUNT_TYPES: readonly AccountType[] = [
  'Bank',
  'AccountsReceivable',
  'OtherCurrentAsset',
  'FixedAsset',
  'OtherAsset',
  'AccountsPayable',
  'CreditCard',
  'OtherCurrentLiability',
  'LongTermLiability',
  'Equity',
  'Income',
  'CostOfGoodsSold',
  'Expense',
  'OtherIncome',
  'OtherExpense',
  'NonPosting',
];

export const ACCOUNT_TYPE_LABEL: Record<AccountType, string> = {
  Bank: 'Bank',
  AccountsReceivable: 'Accounts Receivable',
  OtherCurrentAsset: 'Other Current Asset',
  FixedAsset: 'Fixed Asset',
  OtherAsset: 'Other Asset',
  AccountsPayable: 'Accounts Payable',
  CreditCard: 'Credit Card',
  OtherCurrentLiability: 'Other Current Liability',
  LongTermLiability: 'Long Term Liability',
  Equity: 'Equity',
  Income: 'Income',
  CostOfGoodsSold: 'Cost of Goods Sold',
  Expense: 'Expense',
  OtherIncome: 'Other Income',
  OtherExpense: 'Other Expense',
  NonPosting: 'Non-Posting',
};

export function accountClass(t: AccountType): AccountClass {
  switch (t) {
    case 'Bank':
    case 'AccountsReceivable':
    case 'OtherCurrentAsset':
    case 'FixedAsset':
    case 'OtherAsset':
      return 'Asset';
    case 'AccountsPayable':
    case 'CreditCard':
    case 'OtherCurrentLiability':
    case 'LongTermLiability':
      return 'Liability';
    case 'Equity':
      return 'Equity';
    case 'Income':
    case 'OtherIncome':
      return 'Income';
    case 'CostOfGoodsSold':
    case 'Expense':
    case 'OtherExpense':
      return 'Expense';
    case 'NonPosting':
      return 'NonPosting';
  }
}

export function isDebitNormal(t: AccountType): boolean {
  const c = accountClass(t);
  return c === 'Asset' || c === 'Expense';
}

export function isBalanceSheet(t: AccountType): boolean {
  const c = accountClass(t);
  return c === 'Asset' || c === 'Liability' || c === 'Equity';
}

/** QBD gives balance-sheet accounts a register; income/expense get a QuickReport. */
export function hasRegister(t: AccountType): boolean {
  return isBalanceSheet(t);
}

/** Accounts a check can draw on (Rust's `cash_out` for checks). */
export const CHECK_ACCOUNT_TYPES: readonly AccountType[] = ['Bank', 'OtherCurrentAsset', 'CreditCard'];

/** Natural (displayed) balance from a debit-positive amount. */
export function naturalBalance(t: AccountType, debitPositive: number): number {
  return isDebitNormal(t) ? debitPositive : -debitPositive;
}

export interface RegisterColumns {
  /** Heading of the left amount column (QBD: Payment, Charge, Decrease…). */
  left: string;
  right: string;
  /** Does the left column hold debits? (Only A/R's "Amt Chrg" does.) */
  leftIsDebit: boolean;
}

/** QBD's register amount headings per account type. */
export function registerColumns(t: AccountType): RegisterColumns {
  switch (t) {
    case 'Bank':
      return { left: 'Payment', right: 'Deposit', leftIsDebit: false };
    case 'CreditCard':
      return { left: 'Charge', right: 'Payment', leftIsDebit: false };
    case 'AccountsReceivable':
      return { left: 'Amt Chrg', right: 'Amt Paid', leftIsDebit: true };
    case 'AccountsPayable':
      return { left: 'Billed', right: 'Paid', leftIsDebit: false };
    case 'OtherCurrentLiability':
    case 'LongTermLiability':
    case 'Equity':
      return { left: 'Increase', right: 'Decrease', leftIsDebit: false };
    default:
      return { left: 'Decrease', right: 'Increase', leftIsDebit: false };
  }
}

/** Split a debit-positive amount into the register's left/right columns. */
export function registerSplit(t: AccountType, debitPositive: number): { left: number; right: number } {
  const { leftIsDebit } = registerColumns(t);
  const debit = debitPositive > 0 ? debitPositive : 0;
  const credit = debitPositive < 0 ? -debitPositive : 0;
  return leftIsDebit ? { left: debit, right: credit } : { left: credit, right: debit };
}

/** Sort key: type order, then the full name (as QBD's chart lists accounts). */
export function accountTypeOrder(t: AccountType): number {
  return ACCOUNT_TYPES.indexOf(t);
}
