import { describe, expect, test } from 'bun:test';
import { blankItem } from '~/api/blank';
import type { Item, TxnSummary } from '~/api/types';
import { chooseSaleItem, setSaleAmount, setSaleQuantity } from './salesLines';
import { arRegisterRows, blankChargeEntry, chargeEntryToTxn, isBlankCharge } from './statementCharge';

const today = '2026-10-19';
const design: Item = { ...blankItem('Design', 'Service'), id: 'design', sales_price: '85.00', sales_description: 'Design (per hour)', sales_tax_code_id: 'tax' };

const summary = (txn_type: TxnSummary['txn_type'], date: string, amount: number, extra: Partial<TxnSummary> = {}): TxnSummary => ({
  id: `${txn_type}-${date}-${amount}`,
  txn_type,
  date,
  number: null,
  entity_id: 'c',
  entity_name: 'Customer',
  account_id: null,
  account_name: null,
  amount,
  open_balance: null,
  due_date: null,
  memo: null,
  is_void: false,
  is_pending: false,
  edit_seq: 1,
  ...extra,
});

describe('statement charges', () => {
  test('a charge line becomes a StatementCharge with one item line at quantity × rate and no sales tax', () => {
    const e = blankChargeEntry(today, '2026-11-18');
    e.number = '12';
    e.class_id = 'shop';
    e.row = setSaleQuantity(chooseSaleItem(e.row, design, 'tax'), '2');
    expect(e.row.sales_tax_code_id).toBe('tax'); // the item's code, dropped on the way out
    const r = chargeEntryToTxn('cust', e);
    expect(r.ok).toBe(true);
    if (!r.ok) throw new Error(r.message);
    expect(r.txn).toMatchObject({ txn_type: 'StatementCharge', date: today, entity_id: 'cust', number: '12', due_date: '2026-11-18', class_id: 'shop', amount: 17_000, memo: 'Design (per hour)', sales_tax_item_id: null, edit_seq: 0 });
    expect(r.txn.lines).toHaveLength(1);
    expect(r.txn.lines[0]).toMatchObject({ kind: 'Item', item_id: 'design', quantity: '2', rate: '85', amount: 17_000, description: 'Design (per hour)', sales_tax_code_id: null });
  });

  test('a typed amount wins over quantity × rate; a blank quantity counts as one; what is missing is named', () => {
    const e = blankChargeEntry(today);
    expect(chargeEntryToTxn(null, e)).toMatchObject({ ok: false, field: 'customer' });
    expect(chargeEntryToTxn('c', e)).toMatchObject({ ok: false, field: 'item' });
    e.row = chooseSaleItem(e.row, { ...design, sales_price: null }, null);
    expect(chargeEntryToTxn('c', e)).toMatchObject({ ok: false, field: 'amount' });
    e.row = { ...e.row, rate: '40' };
    const one = chargeEntryToTxn('c', e);
    expect(one.ok && one.txn.amount).toBe(4_000);
    e.row = setSaleAmount(e.row, 12_345);
    const typed = chargeEntryToTxn('c', e);
    expect(typed.ok && typed.txn.amount).toBe(12_345);
    expect(chargeEntryToTxn('c', { ...e, date: 'yesterday' })).toMatchObject({ ok: false, field: 'date' });
    expect(isBlankCharge(blankChargeEntry(today))).toBe(true);
    expect(isBlankCharge({ ...blankChargeEntry(today), number: '7' })).toBe(false);
  });

  test('the register puts charges and payments on their sides with a running balance, oldest first; void rows count for nothing', () => {
    const rows = arRegisterRows([
      summary('ReceivePayment', '2026-10-05', 5_000),
      summary('Invoice', '2026-10-01', 12_000),
      summary('StatementCharge', '2026-10-03', 1_500),
      summary('CreditMemo', '2026-10-06', 500),
      summary('Invoice', '2026-10-07', 9_999, { is_void: true }),
    ]);
    expect(rows.map((r) => [r.summary.txn_type, r.charge, r.paid, r.balance])).toEqual([
      ['Invoice', 12_000, 0, 12_000],
      ['StatementCharge', 1_500, 0, 13_500],
      ['ReceivePayment', 0, 5_000, 8_500],
      ['CreditMemo', 0, 500, 8_000],
      ['Invoice', 0, 0, 8_000],
    ]);
  });
});
