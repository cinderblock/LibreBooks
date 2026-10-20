import { describe, expect, test } from 'bun:test';
import { hierarchicalCompare, leafName, parentPath, rollUp } from './tree';

describe('hierarchicalCompare', () => {
  test('children sort directly under their parent', () => {
    const names = ['Sales Tax', 'Sales:Repairs', 'Sales', 'Sales:Cabinetry', 'Rent'];
    expect([...names].sort(hierarchicalCompare)).toEqual(['Rent', 'Sales', 'Sales:Cabinetry', 'Sales:Repairs', 'Sales Tax']);
  });
  test('numeric aware and case-insensitive', () => {
    expect(['job 10', 'Job 2'].sort(hierarchicalCompare)).toEqual(['Job 2', 'job 10']);
  });
});

describe('rollUp', () => {
  test('adds descendants into ancestors', () => {
    const nodes = [
      { id: 'a', parent_id: null, full_name: 'A' },
      { id: 'b', parent_id: 'a', full_name: 'A:B' },
      { id: 'c', parent_id: 'b', full_name: 'A:B:C' },
      { id: 'd', parent_id: null, full_name: 'D' },
    ];
    const own = new Map([
      ['a', 1],
      ['b', 10],
      ['c', 100],
    ]);
    const t = rollUp(nodes, own);
    expect(t.get('a')).toBe(111);
    expect(t.get('b')).toBe(110);
    expect(t.get('c')).toBe(100);
    expect(t.get('d')).toBeUndefined();
  });
});

test('name parts', () => {
  expect(leafName('Cost of Goods Sold:Materials')).toBe('Materials');
  expect(parentPath('Cost of Goods Sold:Materials')).toBe('Cost of Goods Sold');
  expect(parentPath('Checking')).toBe('');
});
