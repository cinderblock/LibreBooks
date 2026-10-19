import { describe, expect, test } from 'bun:test';
import { evaluate, isExpression } from './calc';

const ok = (cents: number) => ({ ok: true as const, cents });

describe('evaluate', () => {
  test('QBD calculator examples', () => {
    expect(evaluate('12*3+4=')).toEqual(ok(4000));
    expect(evaluate('12*3+4')).toEqual(ok(4000));
    expect(evaluate('100.00+5')).toEqual(ok(10500));
    expect(evaluate('100-25.5')).toEqual(ok(7450));
    expect(evaluate('2x3')).toEqual(ok(600));
  });
  test('precedence and grouping', () => {
    expect(evaluate('2+3*4')).toEqual(ok(1400));
    expect(evaluate('(2+3)*4')).toEqual(ok(2000));
    expect(evaluate('-5+2')).toEqual(ok(-300));
    expect(evaluate('--5')).toEqual(ok(500));
  });
  test('exact rationals, rounding only at the end', () => {
    expect(evaluate('100/3')).toEqual(ok(3333));
    expect(evaluate('100/3*3')).toEqual(ok(10000));
    expect(evaluate('200/3')).toEqual(ok(6667));
    expect(evaluate('0.1+0.2')).toEqual(ok(30));
    expect(evaluate('0.005')).toEqual(ok(1)); // half away from zero
    expect(evaluate('-0.005')).toEqual(ok(-1));
  });
  test('amount notation', () => {
    expect(evaluate('$1,234.56')).toEqual(ok(123_456));
    expect(evaluate('(12.00)')).toEqual(ok(-1200));
    expect(evaluate('(12)+5')).toEqual(ok(-700));
  });
  test('errors', () => {
    expect(evaluate('1/0').ok).toBe(false);
    expect(evaluate('1+').ok).toBe(false);
    expect(evaluate('(1+2').ok).toBe(false);
    expect(evaluate('abc').ok).toBe(false);
    expect(evaluate('').ok).toBe(false);
    expect(evaluate('1=2').ok).toBe(false);
  });
});

describe('isExpression', () => {
  test('plain amounts are not expressions', () => {
    for (const s of ['12', '-12', '(12.00)', '$1,234.56', '+3', '']) expect(isExpression(s)).toBe(false);
  });
  test('arithmetic is', () => {
    for (const s of ['12*3', '100+5', '1-2', '(2+3)*4', '10/4=']) expect(isExpression(s)).toBe(true);
  });
});
