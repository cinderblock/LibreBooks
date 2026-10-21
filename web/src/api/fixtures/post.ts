// FIXTURE ONLY. A simplified TypeScript mirror of `crates/lb-core/src/posting.rs`
// so fixture mode can produce registers, balances and reports that add up. The
// server's posting rules are the truth; nothing outside `api/fixtures/` may call
// this. It covers the transaction types the seed uses and the error cases the
// forms need to exercise (missing account, missing name, unbalanced…); inventory
// costing, sales-tax groups and multi-currency are deliberately out.

import { ACCOUNT_TYPE_LABEL } from '~/lib/accountTypes';
import { percentOf } from '~/lib/money';
import type { AccountRole } from '../generated/AccountRole';
import type { Account, AccountType, Entity, Item, Posting, PostingError, Txn, TxnLine } from '../types';

export interface PostingBooks {
  account(id: string): Account | undefined;
  roleAccount(role: AccountRole): Account | undefined;
  item(id: string): Item | undefined;
  entity(id: string): Entity | undefined;
  /** Open amount of an invoice/bill a payment applies to (fixture: always the target exists). */
  txnExists(id: string): boolean;
}

export interface Posted {
  postings: Posting[];
  total: number;
  sales_tax: number;
}

class Fail {
  constructor(readonly error: PostingError) {}
}

function fail(error: PostingError): never {
  throw new Fail(error);
}

function posting(account_id: string, amount: number, extra: Partial<Posting> = {}): Posting {
  return {
    account_id,
    amount,
    entity_id: null,
    class_id: null,
    item_id: null,
    line_id: null,
    memo: null,
    quantity: null,
    cleared: 'Uncleared',
    ...extra,
  };
}

export function fixturePost(txn: Txn, books: PostingBooks): Posted | PostingError {
  try {
    const posted = postInner(txn, books);
    if (txn.is_pending) posted.postings = [];
    if (txn.is_void) {
      posted.postings = posted.postings.map((p) => ({ ...p, amount: 0 }));
      posted.total = 0;
      posted.sales_tax = 0;
    }
    return posted;
  } catch (e) {
    if (e instanceof Fail) return e.error;
    throw e;
  }
}

function postInner(txn: Txn, books: PostingBooks): Posted {
  const out: Posting[] = [];
  const accountOf = (id: string | null, types: readonly AccountType[] = []): Account => {
    if (!id) fail({ code: 'MissingAccount' });
    const a = books.account(id);
    if (!a) fail({ code: 'UnknownAccount', detail: id });
    if (!a.is_active) fail({ code: 'InactiveAccount', detail: id });
    if (a.account_type === 'NonPosting') fail({ code: 'NonPostingAccount', detail: id });
    if (types.length && !types.includes(a.account_type)) {
      fail({
        code: 'WrongAccountType',
        detail: { account: id, expected: types.map((t) => ACCOUNT_TYPE_LABEL[t]).join(' or ') },
      });
    }
    return a;
  };
  const role = (r: AccountRole): Account => books.roleAccount(r) ?? fail({ code: 'MissingRoleAccount', detail: r });
  const header = (account: Account, amount: number) =>
    out.push(posting(account.id, amount, { entity_id: txn.entity_id, class_id: txn.class_id, memo: txn.memo }));
  const linePosting = (line: TxnLine, account: Account, amount: number) => {
    const p = posting(account.id, amount, {
      entity_id: line.entity_id,
      class_id: line.class_id ?? txn.class_id,
      item_id: line.item_id,
      line_id: line.id,
      memo: line.description ?? txn.memo,
    });
    if (account.account_type === 'AccountsReceivable' || account.account_type === 'AccountsPayable') {
      if (!p.entity_id) fail({ code: 'ArApLineWithoutName' });
    }
    out.push(p);
  };
  const needEntity = (kind?: Entity['kind']) => {
    if (!txn.entity_id) fail({ code: 'MissingEntity' });
    const e = books.entity(txn.entity_id);
    if (!e) fail({ code: 'UnknownEntity', detail: txn.entity_id });
    if (kind && e.kind !== kind) fail({ code: 'WrongEntityKind', detail: { entity: e.id, expected: kind.toLowerCase() } });
  };
  // Debit (sign +1) or credit (-1) each purchase-side line; returns the lines' total.
  const purchaseLines = (sign: 1 | -1): number => {
    let total = 0;
    for (const line of txn.lines) {
      if (line.kind === 'Text') continue;
      if (line.kind === 'Account') {
        linePosting(line, accountOf(line.account_id), sign * line.amount);
      } else if (line.kind === 'Item') {
        if (!line.item_id) fail({ code: 'UnknownItem', detail: '00000000-0000-0000-0000-000000000000' });
        const item = books.item(line.item_id) ?? fail({ code: 'UnknownItem', detail: line.item_id });
        if (item.item_type === 'Subtotal' || item.item_type === 'Group') continue;
        const acctId =
          item.item_type === 'Inventory' || item.item_type === 'InventoryAssembly'
            ? (item.asset_account_id ?? role('InventoryAsset').id)
            : (item.expense_account_id ?? (item.has_purchase_info ? null : item.income_account_id));
        if (!acctId) fail({ code: 'ItemWithoutAccount', detail: item.full_name });
        linePosting(line, accountOf(acctId), sign * line.amount);
      } else {
        fail({ code: 'LineKindNotAllowed', detail: line.kind });
      }
      total += line.amount;
    }
    return total;
  };

  let total = 0;
  let salesTax = 0;
  switch (txn.txn_type) {
    case 'Check':
    case 'Paycheck':
    case 'LiabilityCheck':
    case 'CreditCardCharge': {
      const bank = accountOf(
        txn.account_id,
        txn.txn_type === 'CreditCardCharge' ? ['CreditCard'] : ['Bank', 'OtherCurrentAsset', 'CreditCard'],
      );
      if (txn.txn_type === 'Paycheck') needEntity('Employee');
      total = purchaseLines(1);
      header(bank, -total);
      break;
    }
    case 'CreditCardCredit': {
      const card = accountOf(txn.account_id, ['CreditCard']);
      total = purchaseLines(-1);
      header(card, total);
      break;
    }
    case 'Bill':
    case 'ItemReceipt':
    case 'VendorCredit': {
      needEntity('Vendor');
      const ap = txn.ar_ap_account_id ? accountOf(txn.ar_ap_account_id, ['AccountsPayable']) : role('AccountsPayable');
      const sign = txn.txn_type === 'VendorCredit' ? -1 : 1;
      total = purchaseLines(sign);
      header(ap, -sign * total);
      break;
    }
    case 'Deposit': {
      const bank = accountOf(txn.account_id, ['Bank', 'OtherCurrentAsset', 'CreditCard']);
      let gross = 0;
      let cashBack = 0;
      for (const line of txn.lines) {
        if (line.kind === 'Text') continue;
        if (line.kind === 'DepositLine' || line.kind === 'Account') {
          linePosting(line, accountOf(line.account_id), -line.amount);
          gross += line.amount;
        } else if (line.kind === 'CashBack') {
          linePosting(line, accountOf(line.account_id), line.amount);
          cashBack += line.amount;
        } else fail({ code: 'LineKindNotAllowed', detail: line.kind });
      }
      total = gross - cashBack;
      out.push(posting(bank.id, total, { class_id: txn.class_id, memo: txn.memo }));
      break;
    }
    case 'Transfer': {
      const from = accountOf(txn.account_id);
      if (!txn.to_account_id) fail({ code: 'MissingToAccount' });
      const to = accountOf(txn.to_account_id);
      if (from.id === to.id) fail({ code: 'TransferToSameAccount' });
      if (txn.amount <= 0) fail({ code: 'NonPositiveAmount' });
      out.push(posting(from.id, -txn.amount, { memo: txn.memo }), posting(to.id, txn.amount, { memo: txn.memo }));
      total = txn.amount;
      break;
    }
    case 'JournalEntry': {
      if (txn.lines.every((l) => l.kind === 'Text')) fail({ code: 'NoLines' });
      let sum = 0;
      for (const line of txn.lines) {
        if (line.kind === 'Text') continue;
        if (line.kind !== 'Journal') fail({ code: 'LineKindNotAllowed', detail: line.kind });
        linePosting(line, accountOf(line.account_id), line.amount);
        sum += line.amount;
        if (line.amount > 0) total += line.amount;
      }
      if (sum !== 0) fail({ code: 'Unbalanced', detail: sum });
      break;
    }
    case 'InventoryAdjustment': {
      // QBD's Adjust Quantity/Value on Hand: each Item line moves `quantity_change`
      // of an inventory item and `amount` of value into its asset account; the
      // header account (Opening Balance Equity for a new item's opening quantity,
      // an expense for shrinkage) takes the other side. The value is given, not
      // costed: the fixture keeps no cost layers.
      const offset = accountOf(txn.account_id);
      if (txn.lines.every((l) => l.kind === 'Text')) fail({ code: 'NoLines' });
      let sum = 0;
      for (const line of txn.lines) {
        if (line.kind === 'Text') continue;
        if (line.kind !== 'Item') fail({ code: 'LineKindNotAllowed', detail: line.kind });
        if (!line.item_id) fail({ code: 'UnknownItem', detail: '00000000-0000-0000-0000-000000000000' });
        const item = books.item(line.item_id) ?? fail({ code: 'UnknownItem', detail: line.item_id });
        if (item.item_type !== 'Inventory' && item.item_type !== 'InventoryAssembly') fail({ code: 'UnsupportedItem', detail: `${item.full_name} is not an inventory item` });
        const asset = accountOf(item.asset_account_id ?? role('InventoryAsset').id);
        linePosting(line, asset, line.amount);
        out[out.length - 1]!.quantity = line.quantity_change;
        sum += line.amount;
        total += Math.abs(line.amount);
      }
      header(offset, -sum);
      break;
    }
    case 'Invoice':
    case 'StatementCharge':
    case 'CreditMemo':
    case 'SalesReceipt': {
      needEntity('Customer');
      const sign = txn.txn_type === 'CreditMemo' ? -1 : 1;
      let taxable = 0;
      for (const line of txn.lines) {
        if (line.kind === 'Text') continue;
        let acct: Account;
        if (line.kind === 'Account') acct = accountOf(line.account_id);
        else if (line.kind === 'Item') {
          if (!line.item_id) fail({ code: 'UnknownItem', detail: '00000000-0000-0000-0000-000000000000' });
          const item = books.item(line.item_id) ?? fail({ code: 'UnknownItem', detail: line.item_id });
          if (item.item_type === 'Subtotal' || item.item_type === 'Group') continue;
          if (!item.income_account_id) fail({ code: 'ItemWithoutAccount', detail: item.full_name });
          acct = accountOf(item.income_account_id);
        } else fail({ code: 'LineKindNotAllowed', detail: line.kind });
        linePosting(line, acct, -sign * line.amount);
        total += line.amount;
        if (line.sales_tax_code_id) taxable += line.amount;
      }
      if (txn.sales_tax_item_id && taxable) {
        const taxItem = books.item(txn.sales_tax_item_id);
        salesTax = percentOf(taxable, taxItem?.tax_rate ?? '0') ?? 0;
        if (salesTax) {
          out.push(posting(role('SalesTaxPayable').id, -sign * salesTax, { entity_id: taxItem?.tax_agency_id ?? null }));
          total += salesTax;
        }
      }
      const debitSide =
        txn.txn_type === 'SalesReceipt'
          ? txn.account_id
            ? accountOf(txn.account_id)
            : role('UndepositedFunds')
          : txn.ar_ap_account_id
            ? accountOf(txn.ar_ap_account_id, ['AccountsReceivable'])
            : role('AccountsReceivable');
      header(debitSide, sign * total);
      break;
    }
    case 'ReceivePayment': {
      needEntity('Customer');
      const deposit = txn.account_id ? accountOf(txn.account_id) : role('UndepositedFunds');
      const ar = txn.ar_ap_account_id ? accountOf(txn.ar_ap_account_id) : role('AccountsReceivable');
      const applied = txn.applications.reduce((s, a) => s + a.amount, 0);
      if (applied > txn.amount) fail({ code: 'OverApplied', detail: { applied, amount: txn.amount } });
      for (const a of txn.applications) if (!books.txnExists(a.target_txn_id)) fail({ code: 'UnknownTarget', detail: a.target_txn_id });
      out.push(posting(deposit.id, txn.amount, { entity_id: txn.entity_id, memo: txn.memo }));
      header(ar, -txn.amount);
      total = txn.amount;
      break;
    }
    case 'BillPaymentCheck':
    case 'BillPaymentCreditCard': {
      needEntity('Vendor');
      const bank = accountOf(
        txn.account_id,
        txn.txn_type === 'BillPaymentCreditCard' ? ['CreditCard'] : ['Bank', 'OtherCurrentAsset'],
      );
      const ap = txn.ar_ap_account_id ? accountOf(txn.ar_ap_account_id) : role('AccountsPayable');
      if (txn.amount <= 0) fail({ code: 'NonPositiveAmount' });
      header(bank, -txn.amount);
      header(ap, txn.amount);
      total = txn.amount;
      break;
    }
    case 'PurchaseOrder':
    case 'Estimate':
    case 'SalesOrder':
    case 'TimeTracking':
      return { postings: [], total: txn.lines.reduce((s, l) => s + l.amount, 0), sales_tax: 0 };
    default:
      fail({ code: 'UnsupportedItem', detail: `${txn.txn_type} (fixture mode)` });
  }
  const sum = out.reduce((s, p) => s + p.amount, 0);
  if (sum !== 0) fail({ code: 'Unbalanced', detail: sum });
  return { postings: out, total, sales_tax: salesTax };
}
