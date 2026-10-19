// Money: a signed whole number of cents in a JS `number`.
//
// Mirrors `crates/lb-core/src/money.rs`. Cents are exact in a double up to
// Number.MAX_SAFE_INTEGER (about $90 trillion), so adding and subtracting
// integers is safe; anything involving a rate or a percent goes through the exact
// decimal module and rounds half away from zero, as QBD does.

import {
  type Dec,
  div as decDiv,
  groupThousands,
  mul,
  parseDecimal,
  round,
  toPlainString,
  trim,
} from './decimal';

export type Cents = number;

export type MoneyParseError = 'empty' | 'invalid' | 'too_precise' | 'overflow';

export type MoneyParse = { ok: true; cents: Cents } | { ok: false; error: MoneyParseError };

const MAX = BigInt(Number.MAX_SAFE_INTEGER);

/** Human messages for parse errors, phrased like the Rust ones. */
export const MONEY_ERROR_TEXT: Record<MoneyParseError, string> = {
  empty: 'No amount given',
  invalid: 'Not an amount',
  too_precise: 'More than two decimal places',
  overflow: 'Too large',
};

/**
 * Accepts what people and QuickBooks write: `1234.5`, `1,234.50`, `$1,234.50`,
 * `-12`, `(12.00)` for a negative, `+3`, `.5`, `$-5`. Same rules as Rust's
 * `Money::from_str`.
 */
export function parseMoney(text: string): MoneyParse {
  let s = text.trim();
  if (s === '') return { ok: false, error: 'empty' };
  let negative = false;
  if (s.startsWith('(') && s.endsWith(')')) {
    negative = true;
    s = s.slice(1, -1).trim();
  }
  if (s.startsWith('-')) {
    negative = !negative;
    s = s.slice(1).trimStart();
  } else if (s.startsWith('+')) {
    s = s.slice(1).trimStart();
  }
  if (s.startsWith('$')) s = s.slice(1).trimStart();
  // A sign may also follow the symbol: "$-5.00".
  if (s.startsWith('-')) {
    negative = !negative;
    s = s.slice(1);
  }
  const cleaned = s.replace(/[,_]/g, '');
  if (cleaned === '') return { ok: false, error: 'invalid' };
  const dot = cleaned.indexOf('.');
  const whole = dot < 0 ? cleaned : cleaned.slice(0, dot);
  const frac = dot < 0 ? '' : cleaned.slice(dot + 1);
  if (!/^\d*$/.test(whole) || !/^\d*$/.test(frac)) return { ok: false, error: 'invalid' };
  if (whole === '' && frac === '') return { ok: false, error: 'invalid' };
  let fracUsed = frac;
  if (frac.length > 2) {
    // Trailing zeros beyond two places are harmless ("1.500").
    if (/[^0]/.test(frac.slice(2))) return { ok: false, error: 'too_precise' };
    fracUsed = frac.slice(0, 2);
  }
  const minor = BigInt(whole || '0') * 100n + BigInt(fracUsed.padEnd(2, '0') || '0');
  if (minor > MAX) return { ok: false, error: 'overflow' };
  const cents = Number(minor);
  return { ok: true, cents: negative ? -cents : cents };
}

export interface FormatOptions {
  /** `1,234.56` (default) or `1234.56`. */
  grouping?: boolean;
  /** Prefix `$`. */
  symbol?: boolean;
  /** Negative as `(12.00)` instead of `-12.00` (reports). */
  parens?: boolean;
  /** Render zero as an empty string (register columns). */
  blankZero?: boolean;
}

/** `123456` → `1,234.56`. */
export function formatMoney(cents: Cents, opts: FormatOptions = {}): string {
  const { grouping = true, symbol = false, parens = false, blankZero = false } = opts;
  if (cents === 0 && blankZero) return '';
  const negative = cents < 0;
  const abs = Math.abs(cents);
  const whole = Math.trunc(abs / 100);
  const frac = abs % 100;
  const wholeText = grouping ? groupThousands(String(whole)) : String(whole);
  const body = `${symbol ? '$' : ''}${wholeText}.${String(frac).padStart(2, '0')}`;
  if (!negative) return body;
  return parens ? `(${body})` : `-${body}`;
}

/** Plain `-1234.50` as Rust's `Display` writes it. */
export function centsToPlain(cents: Cents): string {
  return formatMoney(cents, { grouping: false });
}

/** Sum cents, throwing if the total leaves the exact range. */
export function sumCents(values: Iterable<Cents>): Cents {
  let total = 0;
  for (const v of values) total += v;
  if (!Number.isSafeInteger(total)) throw new RangeError('amount too large');
  return total;
}

/** Cents as an exact decimal of major units. */
export function centsToDec(cents: Cents): Dec {
  return { int: BigInt(cents), scale: 2 };
}

/** A decimal amount of major units to cents, half away from zero. */
export function decToCents(d: Dec): Cents | null {
  const r = round(d, 2).int;
  if (r > MAX || r < -MAX) return null;
  return Number(r);
}

/**
 * `quantity × rate`, rounded to cents, as an invoice or bill line computes it
 * (Rust's `Money::extend`). Inputs are decimal strings; null if either is blank
 * or not a number.
 */
export function extend(quantity: string | null, rate: string | null): Cents | null {
  const q = parseDecimal(quantity);
  const r = parseDecimal(rate);
  if (!q || !r) return null;
  return decToCents(mul(q, r));
}

/** `cents × percent / 100`, rounded (Rust's `Money::percent`). */
export function percentOf(cents: Cents, percent: string): Cents | null {
  const p = parseDecimal(percent);
  if (!p) return null;
  const product = mul(centsToDec(cents), p); // major units × percent
  // ÷100: shift the scale by two.
  return decToCents({ int: product.int, scale: product.scale + 2 });
}

/**
 * Back-solve a rate from an amount and a quantity, as QBD does when you edit a
 * line's amount: `rate = amount / quantity`, kept to `places` decimals (QBD keeps
 * up to five) with trailing zeros trimmed but at least two places. Null when the
 * quantity is blank or zero.
 */
export function backSolveRate(amount: Cents, quantity: string | null, places = 5): string | null {
  const q = parseDecimal(quantity);
  if (!q || q.int === 0n) return null;
  const r = decDiv(centsToDec(amount), q, places);
  return r ? toPlainString(trim(r, 2)) : null;
}

/**
 * Split `total` across `weights` in proportion so the parts add up exactly
 * (largest remainder; ties to the earliest), as Rust's `Money::allocate`.
 */
export function allocate(total: Cents, weights: number[]): Cents[] {
  if (weights.length === 0) return [];
  let ws = weights.map((w) => BigInt(Math.abs(Math.trunc(w))));
  let totalWeight = ws.reduce((a, b) => a + b, 0n);
  if (totalWeight === 0n) {
    ws = ws.map(() => 1n);
    totalWeight = BigInt(ws.length);
  }
  const amount = BigInt(total);
  const parts: bigint[] = [];
  const remainders: Array<[number, bigint]> = [];
  ws.forEach((w, i) => {
    const exact = amount * w;
    const part = exact / totalWeight; // truncates toward zero
    parts.push(part);
    const rem = exact - part * totalWeight;
    remainders.push([i, rem < 0n ? -rem : rem]);
  });
  let left = amount - parts.reduce((a, b) => a + b, 0n);
  remainders.sort((a, b) => (b[1] > a[1] ? 1 : b[1] < a[1] ? -1 : a[0] - b[0]));
  const step = left > 0n ? 1n : -1n;
  for (const [i] of remainders) {
    if (left === 0n) break;
    parts[i] = (parts[i] ?? 0n) + step;
    left -= step;
  }
  return parts.map(Number);
}

const ONES = [
  '',
  'One',
  'Two',
  'Three',
  'Four',
  'Five',
  'Six',
  'Seven',
  'Eight',
  'Nine',
  'Ten',
  'Eleven',
  'Twelve',
  'Thirteen',
  'Fourteen',
  'Fifteen',
  'Sixteen',
  'Seventeen',
  'Eighteen',
  'Nineteen',
];
const TENS = ['', '', 'Twenty', 'Thirty', 'Forty', 'Fifty', 'Sixty', 'Seventy', 'Eighty', 'Ninety'];
const SCALES = ['', 'Thousand', 'Million', 'Billion', 'Trillion'];

function underThousand(n: number): string {
  const parts: string[] = [];
  const h = Math.floor(n / 100);
  const rest = n % 100;
  if (h) parts.push(`${ONES[h]} Hundred`);
  if (rest >= 20) {
    const t = TENS[Math.floor(rest / 10)] ?? '';
    const o = ONES[rest % 10] ?? '';
    parts.push(o ? `${t}-${o}` : t);
  } else if (rest) {
    parts.push(ONES[rest] ?? '');
  }
  return parts.join(' ');
}

/**
 * The amount line of a printed check: `1234.56` → "One Thousand Two Hundred
 * Thirty-Four and 56/100". Negative amounts are not printable; they return ''.
 */
export function amountInWords(cents: Cents): string {
  if (cents < 0) return '';
  let whole = Math.floor(cents / 100);
  const frac = String(cents % 100).padStart(2, '0');
  if (whole === 0) return `Zero and ${frac}/100`;
  const groups: string[] = [];
  let scale = 0;
  while (whole > 0) {
    const chunk = whole % 1000;
    if (chunk) groups.unshift(`${underThousand(chunk)}${SCALES[scale] ? ` ${SCALES[scale]}` : ''}`);
    whole = Math.floor(whole / 1000);
    scale += 1;
  }
  return `${groups.join(' ')} and ${frac}/100`;
}

/** Exactly representable? (For guarding values from the network.) */
export function isCents(v: unknown): v is Cents {
  return typeof v === 'number' && Number.isSafeInteger(v);
}

/** Debit-positive amount → the natural sign for an account (credit-normal flips). */
export function natural(debitPositive: Cents, debitNormal: boolean): Cents {
  return debitNormal ? debitPositive : -debitPositive;
}
