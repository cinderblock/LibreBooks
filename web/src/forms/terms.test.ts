import { describe, expect, test } from 'bun:test';
import type { Terms } from '~/api/types';
import { discountDate, dueDate, termsDiscount } from './terms';

const standard = (due: number, discDays: number | null = null, pct: string | null = null): Terms => ({
  id: 's',
  name: 'x',
  is_active: true,
  kind: { type: 'Standard', due_days: due, discount_days: discDays, discount_percent: pct },
  edit_seq: 1,
});
const dateDriven = (day: number, cutoff: number, discDay: number | null = null, pct: string | null = null): Terms => ({
  id: 'd',
  name: 'x',
  is_active: true,
  kind: { type: 'DateDriven', day_of_month: day, cutoff_days: cutoff, discount_day_of_month: discDay, discount_percent: pct },
  edit_seq: 1,
});

describe('terms', () => {
  test('standard terms count days', () => {
    expect(dueDate(standard(30), '2026-10-19')).toBe('2026-11-18');
    expect(dueDate(standard(0), '2026-10-19')).toBe('2026-10-19');
    expect(dueDate(null, '2026-10-19')).toBeNull();
  });
  test('date-driven terms land on the day of this or next month', () => {
    expect(dueDate(dateDriven(15, 5), '2026-10-02')).toBe('2026-10-15');
    // Within the cutoff of the 15th: next month.
    expect(dueDate(dateDriven(15, 5), '2026-10-12')).toBe('2026-11-15');
    // Past the day: next month, clamped to its length.
    expect(dueDate(dateDriven(31, 0), '2026-10-05')).toBe('2026-10-31');
    expect(dueDate(dateDriven(31, 0), '2026-11-05')).toBe('2026-11-30');
  });
  test('discounts apply until the discount date', () => {
    const t = standard(30, 10, '2');
    expect(discountDate(t, '2026-10-01')).toBe('2026-10-11');
    expect(termsDiscount(t, '2026-10-01', '2026-10-11', 100_000)).toBe(2_000);
    expect(termsDiscount(t, '2026-10-01', '2026-10-12', 100_000)).toBe(0);
    expect(termsDiscount(standard(30), '2026-10-01', '2026-10-02', 100_000)).toBe(0);
  });
});
