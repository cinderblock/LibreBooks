// Exact decimal arithmetic for quantities, rates and percents.
//
// The API carries these as decimal strings ("1.5", "19.99", "8.625") because the
// server holds them as `rust_decimal`. Parsing them into a JS number would make
// `0.1 * 3` come out as 0.30000000000000004, so every operation here works on a
// BigInt scaled integer instead: `{ int: 1999n, scale: 2 }` is 19.99.

export interface Dec {
  /** The digits as an integer: value = int / 10^scale. */
  readonly int: bigint;
  /** Number of decimal places (>= 0). */
  readonly scale: number;
}

const TEN = 10n;

export function pow10(n: number): bigint {
  let r = 1n;
  for (let i = 0; i < n; i++) r *= TEN;
  return r;
}

export const ZERO: Dec = { int: 0n, scale: 0 };

/**
 * Parse what people type into a quantity or rate field: `1.5`, `-2`, `1,234.5`,
 * `.25`, `+3`, `(4)` (accounting negative). Returns null for anything else.
 */
export function parseDecimal(text: string | null | undefined): Dec | null {
  if (text == null) return null;
  let s = text.trim();
  if (s === '') return null;
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
  s = s.replace(/[,_]/g, '');
  const m = /^(\d*)(?:\.(\d*))?$/.exec(s);
  if (!m) return null;
  const whole = m[1] ?? '';
  const frac = m[2] ?? '';
  if (whole === '' && frac === '') return null;
  const int = BigInt((whole || '0') + frac);
  return { int: negative ? -int : int, scale: frac.length };
}

/** Bring two decimals to a common scale. */
function align(a: Dec, b: Dec): [bigint, bigint, number] {
  if (a.scale === b.scale) return [a.int, b.int, a.scale];
  if (a.scale > b.scale) return [a.int, b.int * pow10(a.scale - b.scale), a.scale];
  return [a.int * pow10(b.scale - a.scale), b.int, b.scale];
}

export function add(a: Dec, b: Dec): Dec {
  const [x, y, scale] = align(a, b);
  return { int: x + y, scale };
}

export function sub(a: Dec, b: Dec): Dec {
  const [x, y, scale] = align(a, b);
  return { int: x - y, scale };
}

export function mul(a: Dec, b: Dec): Dec {
  return { int: a.int * b.int, scale: a.scale + b.scale };
}

export function neg(a: Dec): Dec {
  return { int: -a.int, scale: a.scale };
}

export function isZero(a: Dec): boolean {
  return a.int === 0n;
}

export function compare(a: Dec, b: Dec): number {
  const [x, y] = align(a, b);
  return x < y ? -1 : x > y ? 1 : 0;
}

/**
 * Integer division of `n` by `d`, rounding half away from zero (as
 * `rust_decimal`'s `MidpointAwayFromZero` and QBD do). `d` must be non-zero.
 */
export function divRoundHalfAway(n: bigint, d: bigint): bigint {
  if (d === 0n) throw new RangeError('division by zero');
  const negative = n < 0n !== d < 0n;
  const an = n < 0n ? -n : n;
  const ad = d < 0n ? -d : d;
  let q = an / ad;
  const rem = an % ad;
  if (rem * 2n >= ad) q += 1n;
  return negative ? -q : q;
}

/** Round to `places` decimal places, half away from zero. */
export function round(a: Dec, places: number): Dec {
  if (a.scale <= places) return { int: a.int * pow10(places - a.scale), scale: places };
  return { int: divRoundHalfAway(a.int, pow10(a.scale - places)), scale: places };
}

/** `a / b` to `places` decimal places, rounded half away from zero. Null if b is 0. */
export function div(a: Dec, b: Dec, places: number): Dec | null {
  if (b.int === 0n) return null;
  // a.int/10^as ÷ b.int/10^bs = (a.int × 10^bs) / (b.int × 10^as); scale the
  // numerator by 10^places first so the integer division keeps that many places.
  const num = a.int * pow10(b.scale + places);
  const den = b.int * pow10(a.scale);
  return { int: divRoundHalfAway(num, den), scale: places };
}

/** Drop trailing zeros after the point, keeping at least `minScale` places. */
export function trim(a: Dec, minScale = 0): Dec {
  let { int, scale } = a;
  while (scale > minScale && int % TEN === 0n) {
    int /= TEN;
    scale -= 1;
  }
  return { int, scale };
}

/** Plain `-1234.5` (no grouping). */
export function toPlainString(a: Dec): string {
  const negative = a.int < 0n;
  const digits = (negative ? -a.int : a.int).toString();
  if (a.scale === 0) return (negative ? '-' : '') + digits;
  const padded = digits.padStart(a.scale + 1, '0');
  const whole = padded.slice(0, padded.length - a.scale);
  const frac = padded.slice(padded.length - a.scale);
  return `${negative ? '-' : ''}${whole}.${frac}`;
}

/** Group the integer part with commas: `1234567.5` → `1,234,567.5`. */
export function groupThousands(plain: string): string {
  const negative = plain.startsWith('-');
  const body = negative ? plain.slice(1) : plain;
  const [whole = '', frac] = body.split('.');
  const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return (negative ? '-' : '') + grouped + (frac !== undefined ? `.${frac}` : '');
}

/**
 * Format a decimal for a quantity or rate column: trailing zeros trimmed but at
 * least `minScale` places, grouped. `formatDecimal(parse("19.990"), 2)` → `19.99`.
 */
export function formatDecimal(a: Dec, minScale = 0, grouping = true): string {
  const plain = toPlainString(trim(a, minScale));
  return grouping ? groupThousands(plain) : plain;
}

/** The canonical string the API wants: plain, trailing zeros trimmed. */
export function toApiString(a: Dec): string {
  return toPlainString(trim(a));
}

/** Normalise user text to the API's decimal string, or null if not a number. */
export function normalizeDecimalText(text: string | null | undefined): string | null {
  const d = parseDecimal(text);
  return d ? toApiString(d) : null;
}
