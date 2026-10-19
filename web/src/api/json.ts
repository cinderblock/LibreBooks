// JSON in and out of the API.
//
// ts-rs types `edit_seq` (a Rust i64) as `bigint`, but JSON has no bigint: the
// server writes a plain number. Reviving it into a BigInt on the way in keeps the
// generated types honest at runtime (comparisons, `=== 0n`), and the replacer
// writes bigints back out as numbers. Edit sequences never approach 2^53, so the
// round trip is exact.

const BIGINT_KEYS = new Set(['edit_seq']);

export function parseApiJson<T>(text: string): T {
  return JSON.parse(text, (key, value: unknown) =>
    BIGINT_KEYS.has(key) && typeof value === 'number' && Number.isSafeInteger(value) ? BigInt(value) : value,
  ) as T;
}

export function stringifyApiJson(value: unknown): string {
  return JSON.stringify(value, (_key, v: unknown) => (typeof v === 'bigint' ? Number(v) : v));
}

/** Deep copy that keeps bigints (structuredClone does, JSON doesn't). */
export function clone<T>(value: T): T {
  return structuredClone(value);
}
