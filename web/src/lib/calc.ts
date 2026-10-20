// QBD's built-in amount calculator.
//
// In any QuickBooks amount field you can type `12*3+4` and press `=` (or Enter,
// or Tab) to get 40.00. The arithmetic here is exact: every number is a rational
// (BigInt numerator / denominator), so `100/3*3` is exactly 100, and only the
// final result rounds to cents, half away from zero.
//
// Grammar (usual precedence, left-associative):
//   expr   := term (('+' | '-') term)*
//   term   := unary (('*' | 'x' | '/') unary)*
//   unary  := ('+' | '-') unary | atom
//   atom   := number | '(' expr ')'
// Accounting notation: a parenthesised bare number, `(12.00)`, means -12.00, not
// a grouping (QBD reports and statements write negatives that way, and people
// paste them). `(2+3)*4` is still a grouping.

import { divRoundHalfAway } from './decimal';
import type { Cents } from './money';

interface Rat {
  n: bigint;
  d: bigint; // always > 0
}

export type CalcResult = { ok: true; cents: Cents } | { ok: false; error: string };

type Token =
  | { t: 'num'; v: Rat }
  | { t: 'op'; v: '+' | '-' | '*' | '/' }
  | { t: 'lp' }
  | { t: 'rp' };

const MAX = BigInt(Number.MAX_SAFE_INTEGER);

function gcd(a: bigint, b: bigint): bigint {
  a = a < 0n ? -a : a;
  b = b < 0n ? -b : b;
  while (b) [a, b] = [b, a % b];
  return a || 1n;
}

function norm(n: bigint, d: bigint): Rat {
  if (d < 0n) {
    n = -n;
    d = -d;
  }
  const g = gcd(n, d);
  return { n: n / g, d: d / g };
}

function parseNumber(text: string): Rat | null {
  const s = text.replace(/[,$]/g, '');
  const m = /^(\d*)(?:\.(\d*))?$/.exec(s);
  if (!m || (m[1] === '' && (m[2] ?? '') === '')) return null;
  const frac = m[2] ?? '';
  return norm(BigInt((m[1] || '0') + frac), 10n ** BigInt(frac.length));
}

function tokenize(input: string): Token[] | string {
  const tokens: Token[] = [];
  let i = 0;
  while (i < input.length) {
    const c = input[i]!;
    if (c === ' ' || c === '\t') {
      i++;
      continue;
    }
    if (c === '=') {
      // A trailing `=` is the "evaluate" key; anywhere else it is a typo.
      if (input.slice(i + 1).trim() !== '') return 'Unexpected "="';
      break;
    }
    if (c === '+' || c === '-' || c === '*' || c === '/') {
      tokens.push({ t: 'op', v: c });
      i++;
      continue;
    }
    if (c === 'x' || c === 'X' || c === '×') {
      tokens.push({ t: 'op', v: '*' });
      i++;
      continue;
    }
    if (c === '÷') {
      tokens.push({ t: 'op', v: '/' });
      i++;
      continue;
    }
    if (c === '(') {
      tokens.push({ t: 'lp' });
      i++;
      continue;
    }
    if (c === ')') {
      tokens.push({ t: 'rp' });
      i++;
      continue;
    }
    if (/[\d.,$]/.test(c)) {
      let j = i;
      while (j < input.length && /[\d.,$]/.test(input[j]!)) j++;
      const v = parseNumber(input.slice(i, j));
      if (!v) return `"${input.slice(i, j)}" is not a number`;
      tokens.push({ t: 'num', v });
      i = j;
      continue;
    }
    return `Unexpected "${c}"`;
  }
  return tokens;
}

class Parser {
  private i = 0;
  constructor(private readonly tokens: Token[]) {}

  parse(): Rat {
    if (this.tokens.length === 0) throw new Error('No amount given');
    const v = this.expr();
    if (this.i < this.tokens.length) throw new Error('Unexpected input');
    return v;
  }

  private peek(): Token | undefined {
    return this.tokens[this.i];
  }

  private expr(): Rat {
    let left = this.term();
    for (;;) {
      const t = this.peek();
      if (t?.t !== 'op' || (t.v !== '+' && t.v !== '-')) return left;
      this.i++;
      const right = this.term();
      left =
        t.v === '+'
          ? norm(left.n * right.d + right.n * left.d, left.d * right.d)
          : norm(left.n * right.d - right.n * left.d, left.d * right.d);
    }
  }

  private term(): Rat {
    let left = this.unary();
    for (;;) {
      const t = this.peek();
      if (t?.t !== 'op' || (t.v !== '*' && t.v !== '/')) return left;
      this.i++;
      const right = this.unary();
      if (t.v === '*') left = norm(left.n * right.n, left.d * right.d);
      else {
        if (right.n === 0n) throw new Error('Division by zero');
        left = norm(left.n * right.d, left.d * right.n);
      }
    }
  }

  private unary(): Rat {
    const t = this.peek();
    if (t?.t === 'op' && (t.v === '-' || t.v === '+')) {
      this.i++;
      const v = this.unary();
      return t.v === '-' ? { n: -v.n, d: v.d } : v;
    }
    return this.atom();
  }

  private atom(): Rat {
    const t = this.peek();
    if (!t) throw new Error('Incomplete expression');
    if (t.t === 'num') {
      this.i++;
      return t.v;
    }
    if (t.t === 'lp') {
      // `(12.00)`: accounting negative.
      const a = this.tokens[this.i + 1];
      const b = this.tokens[this.i + 2];
      if (a?.t === 'num' && b?.t === 'rp') {
        this.i += 3;
        return { n: -a.v.n, d: a.v.d };
      }
      this.i++;
      const v = this.expr();
      if (this.peek()?.t !== 'rp') throw new Error('Missing ")"');
      this.i++;
      return v;
    }
    throw new Error('Incomplete expression');
  }
}

/** Evaluate an amount expression to cents. */
export function evaluate(input: string): CalcResult {
  const tokens = tokenize(input.trim());
  if (typeof tokens === 'string') return { ok: false, error: tokens };
  try {
    const v = new Parser(tokens).parse();
    const cents = divRoundHalfAway(v.n * 100n, v.d);
    if (cents > MAX || cents < -MAX) return { ok: false, error: 'Too large' };
    return { ok: true, cents: Number(cents) };
  } catch (e) {
    return { ok: false, error: (e as Error).message };
  }
}

/**
 * Does the text contain arithmetic (as opposed to a plain, possibly signed or
 * parenthesised, amount)? The amount field shows its "tape" only then.
 */
export function isExpression(text: string): boolean {
  const s = text.trim().replace(/=$/, '').trim();
  if (s === '') return false;
  // Strip one leading sign and accounting parens; what's left must be operator-free.
  const body = s.replace(/^\(\s*([^()]*)\s*\)$/, '$1').replace(/^[+-]/, '');
  return /[-+*/x×÷()]/.test(body);
}
