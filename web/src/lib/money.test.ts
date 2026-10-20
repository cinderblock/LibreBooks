import { describe, expect, test } from 'bun:test';
import {
  allocate,
  amountInWords,
  backSolveRate,
  extend,
  formatMoney,
  parseMoney,
  percentOf,
  sumCents,
} from './money';
import { div, formatDecimal, mul, normalizeDecimalText, parseDecimal, round, toPlainString } from './decimal';

describe('parseMoney (mirrors Money::from_str)', () => {
  const cases: Array<[string, number]> = [
    ['0', 0],
    ['12', 1200],
    ['12.3', 1230],
    ['12.34', 1234],
    ['.5', 50],
    ['1,234.56', 123_456],
    ['$1,234.56', 123_456],
    ['-12.00', -1200],
    ['(12.00)', -1200],
    ['($5)', -500],
    ['$-5', -500],
    ['+3', 300],
    ['27505.49', 2_750_549],
    ['1.500', 150],
    ['  7.10 ', 710],
  ];
  for (const [text, cents] of cases) {
    test(JSON.stringify(text), () => expect(parseMoney(text)).toEqual({ ok: true, cents }));
  }

  test('rejects nonsense', () => {
    expect(parseMoney('')).toEqual({ ok: false, error: 'empty' });
    expect(parseMoney('abc')).toEqual({ ok: false, error: 'invalid' });
    expect(parseMoney('1.234')).toEqual({ ok: false, error: 'too_precise' });
    expect(parseMoney('1.2.3')).toEqual({ ok: false, error: 'invalid' });
    expect(parseMoney('.')).toEqual({ ok: false, error: 'invalid' });
    expect(parseMoney('99999999999999999999')).toEqual({ ok: false, error: 'overflow' });
  });
});

describe('formatMoney', () => {
  test('plain and grouped', () => {
    expect(formatMoney(0)).toBe('0.00');
    expect(formatMoney(5)).toBe('0.05');
    expect(formatMoney(-5)).toBe('-0.05');
    expect(formatMoney(123_456)).toBe('1,234.56');
    expect(formatMoney(-123_456)).toBe('-1,234.56');
    expect(formatMoney(123_456_789_01)).toBe('123,456,789.01');
    expect(formatMoney(123_456, { grouping: false })).toBe('1234.56');
  });
  test('options', () => {
    expect(formatMoney(-1200, { parens: true })).toBe('(12.00)');
    expect(formatMoney(1200, { symbol: true })).toBe('$12.00');
    expect(formatMoney(0, { blankZero: true })).toBe('');
  });
  test('round trips through the parser', () => {
    for (const c of [0, 1, -1, 99, 100, 123_456_78, -98_765_432_10, Number.MAX_SAFE_INTEGER]) {
      expect(parseMoney(formatMoney(c))).toEqual({ ok: true, cents: c });
    }
  });
});

describe('extend: quantity × rate, half away from zero (Money::extend)', () => {
  test('cases from money.rs', () => {
    expect(extend('3', '19.99')).toBe(5997);
    expect(extend('0.5', '0.05')).toBe(3); // 0.025 → 0.03
    expect(extend('-0.5', '0.05')).toBe(-3);
    expect(extend('1.333', '100')).toBe(13330);
  });
  test('no float drift', () => {
    expect(extend('3', '0.1')).toBe(30);
    expect(extend('0.1', '0.2')).toBe(2); // 0.02
    expect(extend('1.005', '1')).toBe(101); // 1.005 → 1.01 (a float would say 1.00)
  });
  test('blank or bad input', () => {
    expect(extend(null, '1')).toBeNull();
    expect(extend('x', '1')).toBeNull();
  });
});

describe('percentOf (Money::percent)', () => {
  test('cases from money.rs', () => {
    expect(percentOf(10000, '8.25')).toBe(825);
    expect(percentOf(1999, '8.625')).toBe(172);
  });
});

describe('backSolveRate', () => {
  test('amount / quantity kept to five places, trimmed', () => {
    expect(backSolveRate(10000, '3')).toBe('33.33333');
    expect(backSolveRate(10000, '4')).toBe('25.00');
    expect(backSolveRate(1999, '1')).toBe('19.99');
    expect(backSolveRate(2000, '0.5')).toBe('40.00');
  });
  test('needs a non-zero quantity', () => {
    expect(backSolveRate(100, null)).toBeNull();
    expect(backSolveRate(100, '0')).toBeNull();
  });
});

describe('allocate (Money::allocate)', () => {
  test('cases from money.rs', () => {
    expect(allocate(100, [1, 1, 1])).toEqual([34, 33, 33]);
    expect(allocate(-100, [1, 1, 1])).toEqual([-34, -33, -33]);
    expect(allocate(1000, [0, 0])).toEqual([500, 500]);
    expect(allocate(5, [])).toEqual([]);
    expect(allocate(10, [3, 7])).toEqual([3, 7]);
  });
  test('always sums', () => {
    for (let i = 0; i < 200; i++) {
      const total = Math.floor(Math.random() * 2_000_000) - 1_000_000;
      const weights = Array.from({ length: 1 + (i % 9) }, () => Math.floor(Math.random() * 2000) - 1000);
      expect(sumCents(allocate(total, weights))).toBe(total);
    }
  });
});

describe('amountInWords', () => {
  test('check amount line', () => {
    expect(amountInWords(123_456)).toBe('One Thousand Two Hundred Thirty-Four and 56/100');
    expect(amountInWords(5)).toBe('Zero and 05/100');
    expect(amountInWords(100_000_000)).toBe('One Million and 00/100');
    expect(amountInWords(2_100)).toBe('Twenty-One and 00/100');
    expect(amountInWords(11_500)).toBe('One Hundred Fifteen and 00/100');
  });
});

describe('decimal', () => {
  test('parse and print', () => {
    expect(toPlainString(parseDecimal('1,234.50')!)).toBe('1234.50');
    expect(toPlainString(parseDecimal('-.5')!)).toBe('-0.5');
    expect(toPlainString(parseDecimal('(4)')!)).toBe('-4');
    expect(parseDecimal('1.2.3')).toBeNull();
    expect(parseDecimal('')).toBeNull();
    expect(normalizeDecimalText(' 19.990 ')).toBe('19.99');
    expect(normalizeDecimalText('3.000')).toBe('3');
  });
  test('arithmetic is exact', () => {
    expect(toPlainString(mul(parseDecimal('0.1')!, parseDecimal('3')!))).toBe('0.3');
    expect(toPlainString(round(parseDecimal('2.345')!, 2))).toBe('2.35');
    expect(toPlainString(round(parseDecimal('-2.345')!, 2))).toBe('-2.35');
    expect(toPlainString(div(parseDecimal('1')!, parseDecimal('3')!, 4)!)).toBe('0.3333');
    expect(toPlainString(div(parseDecimal('2')!, parseDecimal('3')!, 4)!)).toBe('0.6667');
    expect(div(parseDecimal('1')!, parseDecimal('0')!, 2)).toBeNull();
  });
  test('formatting', () => {
    expect(formatDecimal(parseDecimal('1234.5000')!, 2)).toBe('1,234.50');
    expect(formatDecimal(parseDecimal('3.000')!)).toBe('3');
    expect(formatDecimal(parseDecimal('0.00001')!)).toBe('0.00001');
  });
});
