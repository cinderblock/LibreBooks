import { describe, expect, test } from 'bun:test';
import { blankItem } from '~/api/blank';
import type { Item, SalesTaxCode } from '~/api/types';
import { blankSaleRow, chooseSaleItem, estimatedTax, recomputeSales, salesSubtotal, setSaleAmount, setSaleQuantity, setSaleRate, taxRate, taxableAmount, toSaleLines } from './salesLines';

const design: Item = { ...blankItem('Design', 'Service'), id: 'design', sales_description: 'Design (per hour)', sales_price: '85.00', sales_tax_code_id: 'non' };
const walnut: Item = { ...blankItem('Walnut', 'NonInventory'), id: 'walnut', sales_description: 'Walnut', sales_price: '18.50', sales_tax_code_id: 'tax' };
const subtotal: Item = { ...blankItem('Subtotal', 'Subtotal'), id: 'sub' };
const discount: Item = { ...blankItem('Repeat Discount', 'Discount'), id: 'disc', sales_price: '10', is_percent: true, sales_description: '10% off' };
const tax: Item = { ...blankItem('County Tax', 'SalesTax'), id: 'taxitem', tax_rate: '7.25' };
const taxA: Item = { ...blankItem('State', 'SalesTax'), id: 'ta', tax_rate: '6' };
const taxB: Item = { ...blankItem('City', 'SalesTax'), id: 'tb', tax_rate: '1.25' };
const group: Item = { ...blankItem('Combined', 'SalesTaxGroup'), id: 'tg', components: [{ item_id: 'ta', quantity: '1' }, { item_id: 'tb', quantity: '1' }] };
const items = new Map<string, Item>([design, walnut, subtotal, discount, tax, taxA, taxB, group].map((i) => [i.id, i]));
const codes = new Map<string, SalesTaxCode>([
  ['tax', { id: 'tax', name: 'Tax', description: null, is_taxable: true, is_active: true, edit_seq: 1 }],
  ['non', { id: 'non', name: 'Non', description: null, is_taxable: false, is_active: true, edit_seq: 1 }],
]);

describe('sales lines', () => {
  test('choosing an item fills description, price, tax code and amount', () => {
    const r = chooseSaleItem({ ...blankSaleRow(), quantity: '4' }, design, 'tax');
    expect(r).toMatchObject({ item_id: 'design', description: 'Design (per hour)', rate: '85.00', amount: 34_000, sales_tax_code_id: 'non' });
    // Without the item's own code, the default (taxable) applies.
    expect(chooseSaleItem(blankSaleRow(), { ...walnut, sales_tax_code_id: null }, 'tax').sales_tax_code_id).toBe('tax');
  });
  test('quantity, rate and amount keep each other in step', () => {
    let r = chooseSaleItem(blankSaleRow(), walnut, 'tax');
    r = setSaleQuantity(r, '3');
    expect(r.amount).toBe(5_550);
    r = setSaleRate(r, '20');
    expect(r.amount).toBe(6_000);
    r = setSaleAmount(r, 6_100);
    expect(r.rate).toBe('20.33333');
  });
  test('subtotals sum the lines above; a percent discount applies to the line above it', () => {
    const rows = [
      { ...chooseSaleItem(blankSaleRow(), design, 'tax'), amount: 10_000 },
      { ...chooseSaleItem(blankSaleRow(), walnut, 'tax'), amount: 5_000 },
      chooseSaleItem(blankSaleRow(), subtotal, 'tax'),
      chooseSaleItem(blankSaleRow(), discount, 'tax'),
    ];
    const out = recomputeSales(rows, items);
    expect(out[2]!.amount).toBe(15_000);
    expect(out[2]!.description).toBe('Subtotal');
    expect(out[3]!.amount).toBe(-1_500);
    expect(salesSubtotal(out, items)).toBe(13_500);
    expect(taxableAmount(out, items, codes)).toBe(5_000);
  });
  test('tax: a single item, a group, and a non-taxable customer', () => {
    expect(taxRate(group, items)).toBe('7.25');
    expect(estimatedTax(10_000, tax, items, true)).toBe(725);
    expect(estimatedTax(10_000, group, items, true)).toBe(725);
    expect(estimatedTax(10_000, tax, items, false)).toBe(0);
    expect(estimatedTax(10_000, undefined, items, true)).toBe(0);
  });
  test('rows become Item lines with normalised decimals; blanks dropped', () => {
    const lines = toSaleLines([{ ...chooseSaleItem(blankSaleRow(), walnut, 'tax'), quantity: '2.0' }, blankSaleRow()]);
    expect(lines).toHaveLength(1);
    expect(lines[0]).toMatchObject({ kind: 'Item', item_id: 'walnut', quantity: '2', rate: '18.5', amount: 1_850 });
  });
});
