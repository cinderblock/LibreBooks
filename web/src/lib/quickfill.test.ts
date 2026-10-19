import { describe, expect, test } from 'bun:test';
import { exactIndex, inlineCompletion, prepare, rankOf, Rank, search } from './quickfill';

const names = [
  'Checking',
  'Cost of Goods Sold',
  'Cost of Goods Sold:Materials',
  'Cost of Goods Sold:Subcontractors',
  'Materials Testing',
  'Utilities',
  'Utilities:Electric',
  'Baker, Ellen',
  'Baker, Ellen:Kitchen Cabinets',
];
const prepared = prepare(names.map((fullName) => ({ fullName })));
const found = (q: string) => search(prepared, q).map((i) => names[i]);

describe('search', () => {
  test('empty query keeps list order', () => {
    expect(found('')).toEqual(names);
  });
  test('full-name prefix ranks first, then any colon level', () => {
    expect(found('mat')).toEqual(['Materials Testing', 'Cost of Goods Sold:Materials']);
  });
  test('level paths', () => {
    expect(found('cost:mat')).toEqual(['Cost of Goods Sold:Materials']);
    expect(found('cost of goods sold:')).toEqual([
      'Cost of Goods Sold:Materials',
      'Cost of Goods Sold:Subcontractors',
    ]);
    expect(found('util:ele')).toEqual(['Utilities:Electric']);
  });
  test('word prefixes and substrings', () => {
    expect(found('goods')).toEqual([
      'Cost of Goods Sold',
      'Cost of Goods Sold:Materials',
      'Cost of Goods Sold:Subcontractors',
    ]);
    expect(found('ellen')).toEqual(['Baker, Ellen', 'Baker, Ellen:Kitchen Cabinets']);
    expect(found('contract')).toEqual(['Cost of Goods Sold:Subcontractors']);
  });
  test('extra keys (account numbers)', () => {
    const p = prepare([{ fullName: 'Checking', keys: ['1000'] }, { fullName: 'Savings', keys: ['1010'] }]);
    expect(rankOf(p[0]!, '100')).toBe(Rank.KeyPrefix);
    expect(search(p, '101')).toEqual([1]);
  });
  test('scales to thousands quickly', () => {
    const big = prepare(
      Array.from({ length: 20_000 }, (_, i) => ({ fullName: `Parent ${i % 50}:Child ${i}:Leaf ${i * 7}` })),
    );
    const t0 = performance.now();
    for (const q of ['p', 'pa', 'par', 'chi', 'leaf 7', 'parent 3:child 1']) search(big, q);
    expect(performance.now() - t0).toBeLessThan(500);
  });
});

describe('inlineCompletion', () => {
  test('completes a prefix match preserving what was typed', () => {
    expect(inlineCompletion('che', 'Checking')).toBe('checking');
    expect(inlineCompletion('Che', 'Checking')).toBe('Checking');
    expect(inlineCompletion('mat', 'Cost of Goods Sold:Materials')).toBeNull();
    expect(inlineCompletion('Checking', 'Checking')).toBeNull();
  });
});

describe('exactIndex', () => {
  test('case-insensitive full name', () => {
    expect(exactIndex(prepared, 'utilities:electric')).toBe(6);
    expect(exactIndex(prepared, 'Electric')).toBe(-1);
  });
});
