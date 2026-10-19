import { describe, expect, test } from 'bun:test';
import {
  addMonths,
  dateKey,
  dateWarning,
  formatDate,
  isIsoDate,
  macroRange,
  parseTypedDate,
  weekStart,
} from './dates';

// 2026-10-19 is a Monday (as in period.rs's tests).
const today = '2026-10-19';

describe('weeks start Sunday', () => {
  test('week_start', () => {
    expect(weekStart('2026-10-19')).toBe('2026-10-18');
    expect(weekStart('2026-10-18')).toBe('2026-10-18');
  });
});

describe('macroRange (port of DateMacro::range)', () => {
  test('calendar-year company', () => {
    expect(macroRange('ThisMonth', today, 1)).toEqual({ from: '2026-10-01', to: '2026-10-31' });
    expect(macroRange('LastMonth', today, 1)).toEqual({ from: '2026-09-01', to: '2026-09-30' });
    expect(macroRange('ThisFiscalYearToDate', today, 1)).toEqual({ from: '2026-01-01', to: today });
    expect(macroRange('LastFiscalYear', today, 1)).toEqual({ from: '2025-01-01', to: '2025-12-31' });
    expect(macroRange('ThisFiscalQuarter', today, 1)).toEqual({ from: '2026-10-01', to: '2026-12-31' });
    expect(macroRange('LastFiscalQuarter', today, 1)).toEqual({ from: '2026-07-01', to: '2026-09-30' });
    expect(macroRange('LastMonthToDate', today, 1)).toEqual({ from: '2026-09-01', to: '2026-09-19' });
    expect(macroRange('ThisWeek', today, 1)).toEqual({ from: '2026-10-18', to: '2026-10-24' });
    expect(macroRange('All', today, 1)).toEqual({ from: null, to: null });
    expect(macroRange('NextFourWeeks', today, 1)).toEqual({ from: '2026-10-25', to: '2026-11-21' });
  });
  test('July fiscal year', () => {
    const t = '2026-03-05';
    expect(macroRange('ThisFiscalYear', t, 7)).toEqual({ from: '2025-07-01', to: '2026-06-30' });
    expect(macroRange('ThisFiscalQuarter', t, 7)).toEqual({ from: '2026-01-01', to: '2026-03-31' });
    expect(macroRange('NextFiscalQuarter', t, 7)).toEqual({ from: '2026-04-01', to: '2026-06-30' });
    expect(macroRange('LastFiscalQuarter', t, 7)).toEqual({ from: '2025-10-01', to: '2025-12-31' });
  });
  test('last month to date clamps to short months', () => {
    expect(macroRange('LastMonthToDate', '2026-03-31', 1)).toEqual({ from: '2026-02-01', to: '2026-02-28' });
  });
});

describe('dateKey (port of period::date_key)', () => {
  const d = '2026-02-14';
  test('keys', () => {
    expect(dateKey(d, today, '+')).toBe('2026-02-15');
    expect(dateKey(d, today, '=')).toBe('2026-02-15');
    expect(dateKey(d, today, '-')).toBe('2026-02-13');
    expect(dateKey(d, today, 't')).toBe(today);
    expect(dateKey(d, today, 'M')).toBe('2026-02-01');
    expect(dateKey(d, today, 'h')).toBe('2026-02-28');
    expect(dateKey(d, today, 'y')).toBe('2026-01-01');
    expect(dateKey(d, today, 'r')).toBe('2026-12-31');
    expect(dateKey(d, today, 'w')).toBe('2026-02-08');
    expect(dateKey(d, today, 'k')).toBe('2026-02-14');
    expect(dateKey(d, today, '[')).toBe('2026-02-07');
    expect(dateKey(d, today, ']')).toBe('2026-02-21');
    expect(dateKey(d, today, 'x')).toBeNull();
  });
  test('crosses month and year boundaries', () => {
    expect(dateKey('2026-12-31', today, '+')).toBe('2027-01-01');
    expect(dateKey('2024-03-01', today, '-')).toBe('2024-02-29');
  });
});

describe('parseTypedDate', () => {
  const cases: Array<[string, string | null]> = [
    ['10/19', '2026-10-19'],
    ['10/19/26', '2026-10-19'],
    ['10-19-26', '2026-10-19'],
    ['10.19.2026', '2026-10-19'],
    ['1/2/27', '2027-01-02'],
    ['12/31/99', '1999-12-31'],
    ['1019', '2026-10-19'],
    ['101926', '2026-10-19'],
    ['10192026', '2026-10-19'],
    ['119', '2026-01-19'],
    ['5', '2026-10-05'],
    ['t', today],
    ['Today', today],
    ['2026-02-28', '2026-02-28'],
    ['2/30', null],
    ['13/1', null],
    ['abc', null],
    ['', null],
    ['1/2/345', null],
  ];
  for (const [text, iso] of cases) {
    test(JSON.stringify(text), () => expect(parseTypedDate(text, today)).toBe(iso));
  }
});

describe('helpers', () => {
  test('formatDate', () => expect(formatDate('2026-01-05')).toBe('01/05/2026'));
  test('isIsoDate', () => {
    expect(isIsoDate('2026-02-29')).toBe(false);
    expect(isIsoDate('2024-02-29')).toBe(true);
    expect(isIsoDate('2026-1-1')).toBe(false);
  });
  test('addMonths clamps', () => expect(addMonths('2026-01-31', 1)).toBe('2026-02-28'));
  test('dateWarning', () => {
    expect(dateWarning('2026-10-19', today, 90, 30)).toBeNull();
    expect(dateWarning('2026-06-01', today, 90, 30)).toContain('past');
    expect(dateWarning('2026-12-01', today, 90, 30)).toContain('future');
    expect(dateWarning('2020-01-01', today, null, null)).toBeNull();
  });
});
