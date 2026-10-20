import { describe, expect, test } from 'bun:test';
import { blankEntity, blankItem, blankLine, blankTxn } from '~/api/blank';
import { ApiError } from '~/api/errors';
import type { Item, SalesTaxCode, Terms } from '~/api/types';
import { applyCustomer, isSalesEmpty, newSalesDraft, salesDraftFromTxn, salesDraftToTxn, salesFingerprint, salesServerErrors, salesTotals, setSaleLines, validateSales } from './salesDraft';
import { blankSaleRow, chooseSaleItem, setSaleQuantity } from './salesLines';

const today = '2026-10-19';
const defaults = { taxItemId: 'taxitem', taxCodeId: 'tax', depositTo: null };
const net30: Terms = { id: 't30', name: 'Net 30', is_active: true, kind: { type: 'Standard', due_days: 30, discount_days: null, discount_percent: null }, edit_seq: 1 };
const walnut: Item = { ...blankItem('Walnut', 'NonInventory'), id: 'walnut', sales_price: '10.00', sales_tax_code_id: 'tax' };
const design: Item = { ...blankItem('Design', 'Service'), id: 'design', sales_price: '85.00', sales_tax_code_id: 'non' };
const tax: Item = { ...blankItem('County Tax', 'SalesTax'), id: 'taxitem', tax_rate: '7.25' };
const items = new Map<string, Item>([walnut, design, tax].map((i) => [i.id, i]));
const codes = new Map<string, SalesTaxCode>([
  ['tax', { id: 'tax', name: 'Tax', description: null, is_taxable: true, is_active: true, edit_seq: 1 }],
  ['non', { id: 'non', name: 'Non', description: null, is_taxable: false, is_active: true, edit_seq: 1 }],
]);

describe('sales draft', () => {
  test('a customer brings terms, due date, tax code, rep and addresses (a job from its customer)', () => {
    const acme = { ...blankEntity('Acme', 'Customer'), id: 'acme', company_name: 'Acme Corp', terms_id: 't30', sales_tax_code_id: 'non', bill_address: { lines: ['1 Main St'], city: 'Juniper', state: 'OR', postal_code: '97000', country: null, note: null } };
    const job = { ...blankEntity('Lobby', 'Customer', acme), id: 'job' };
    const d = applyCustomer(newSalesDraft('Invoice', today, defaults), job, net30, acme);
    expect(d).toMatchObject({ entity_id: 'job', terms_id: 't30', due_date: '2026-11-18', customer_tax_code_id: 'non' });
    expect(d.bill_address.lines).toEqual(['Acme Corp', '1 Main St']);
    expect(d.ship_address.city).toBe('Juniper');
  });

  test('totals: subtotal, taxable, tax and total; a non-taxable customer pays no tax', () => {
    let d = newSalesDraft('Invoice', today, defaults);
    d = setSaleLines(d, [setSaleQuantity(chooseSaleItem(blankSaleRow(), walnut, 'tax'), '10'), chooseSaleItem(blankSaleRow(), design, 'tax')], items);
    const t = salesTotals(d, items, codes);
    expect(t).toEqual({ subtotal: 18_500, taxable: 10_000, tax: 725, total: 19_225 });
    expect(salesTotals({ ...d, customer_tax_code_id: 'non' }, items, codes).tax).toBe(0);
    const txn = salesDraftToTxn(d, items, codes);
    expect(txn.amount).toBe(19_225);
    expect(txn.lines).toHaveLength(2);
    expect(txn.sales_tax_item_id).toBe('taxitem');
  });

  test('a sales receipt carries its deposit account; an invoice its due date; neither the other', () => {
    const inv = { ...newSalesDraft('Invoice', today, defaults), due_date: '2026-11-01', account_id: 'bank' };
    expect(salesDraftToTxn(inv, items, codes)).toMatchObject({ due_date: '2026-11-01', account_id: null });
    const rcpt = { ...newSalesDraft('SalesReceipt', today, { ...defaults, depositTo: 'bank' }), due_date: '2026-11-01' };
    expect(salesDraftToTxn(rcpt, items, codes)).toMatchObject({ due_date: null, account_id: 'bank' });
  });

  test('validation and emptiness', () => {
    const d = newSalesDraft('Invoice', today, defaults);
    expect(isSalesEmpty(d)).toBe(true);
    const e = validateSales(d);
    expect(e.header.entity).toBeDefined();
    expect(e.form).toHaveLength(1);
    const withLine = setSaleLines({ ...d, entity_id: 'c' }, [{ ...blankSaleRow(), description: 'x', amount: 5 }], items);
    expect(validateSales(withLine).grids.lines![0]!.item).toBeDefined();
  });

  test('round trip and fingerprint', () => {
    const t = blankTxn('Invoice', today);
    t.entity_id = 'c';
    t.number = '2101';
    t.edit_seq = 2;
    t.ship_date = '2026-10-25';
    t.sales_tax_item_id = 'taxitem';
    t.lines = [{ ...blankLine('Item'), item_id: 'walnut', quantity: '2', rate: '10', amount: 2_000, sales_tax_code_id: 'tax' }];
    const d = salesDraftFromTxn(t);
    const back = salesDraftToTxn(d, items, codes);
    expect(back).toMatchObject({ id: t.id, edit_seq: 2, number: '2101', ship_date: '2026-10-25', amount: 2_145 });
    expect(back.lines[0]!.id).toBe(t.lines[0]!.id);
    expect(salesFingerprint(d)).toBe(salesFingerprint({ ...d, lines: d.lines.map((r) => ({ ...r, key: 'k' })) }));
    expect(salesFingerprint({ ...d, memo: 'x' })).not.toBe(salesFingerprint(d));
  });

  test('server errors: an unknown item marks its line; a missing customer the header', () => {
    let d = newSalesDraft('Invoice', today, defaults);
    d = setSaleLines(d, [{ ...blankSaleRow(), item_id: 'gone', amount: 100 }], items);
    const sent = salesDraftToTxn(d, items, codes);
    const e = salesServerErrors(d, sent, new ApiError(422, 'posting', 'x', { code: 'UnknownItem', detail: 'gone' }));
    expect(e.grids.lines![0]!.item).toBeDefined();
    const m = salesServerErrors(d, sent, new ApiError(422, 'posting', 'x', { code: 'MissingEntity' }));
    expect(m.header.entity).toBe('Choose a name.');
  });
});
