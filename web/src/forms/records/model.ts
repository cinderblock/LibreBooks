// What the list-record editors' pure models share: the server-error mapping
// (a duplicate name goes on the name, a 400 `invalid` on the field it names,
// anything else on the form), text ↔ nullable string, decimal checks, and the
// "not inside itself" rule for picking a parent in a hierarchy.

import { describeError, type ApiError } from '~/api/errors';
import { parseDecimal } from '~/lib/decimal';
import { noErrors, type FormErrors } from '../model';

/**
 * Map a save error to the field it is about. `fieldMap` renames the server's
 * field (`parent_id`) to the editor's (`parent`); unknown fields keep their name.
 */
export function recordServerErrors(err: ApiError, fieldMap: Record<string, string> = {}): FormErrors {
  const e = noErrors();
  const msg = describeError(err);
  if (err.isDuplicate) {
    e.header.name = msg;
    return e;
  }
  const field = (err.detail as { field?: string } | undefined)?.field;
  if (err.code === 'invalid' && field) {
    e.header[fieldMap[field] ?? field] = msg;
    return e;
  }
  e.form.push(msg);
  return e;
}

/** A trimmed field, or null when blank (the API's nullable strings). */
export function textOrNull(s: string | null | undefined): string | null {
  const t = (s ?? '').trim();
  return t ? t : null;
}

export function requireName(e: FormErrors, name: string, what = 'a name') {
  if (!name.trim()) e.header.name = `Enter ${what}.`;
}

/** A decimal-string field (rates, percents, quantities): blank is fine, anything that isn't a number is not. */
export function decimalError(text: string | null | undefined): string | undefined {
  if (text == null || text.trim() === '') return undefined;
  return parseDecimal(text) ? undefined : 'Not a number';
}

/** A decimal field as the API takes it (`"1,234.50"` → `"1234.50"`, `"(2)"` → `"-2"`), or null when blank. */
export function decimalOrNull(text: string | null | undefined): string | null {
  if (text == null || text.trim() === '') return null;
  const d = parseDecimal(text);
  if (!d) return text.trim();
  const neg = d.int < 0n;
  const digits = (neg ? -d.int : d.int).toString().padStart(d.scale + 1, '0');
  const whole = digits.slice(0, digits.length - d.scale) || '0';
  const frac = d.scale ? `.${digits.slice(digits.length - d.scale)}` : '';
  return `${neg ? '-' : ''}${whole}${frac}`;
}

/** A whole number ≥ `min`, from a number input's value; null when blank. */
export function wholeNumber(v: string | number, min = 0): number | null {
  if (v === '' || v === null || v === undefined) return null;
  const n = Math.round(Number(v));
  return Number.isFinite(n) ? Math.max(min, n) : null;
}

interface Node {
  id: string;
  full_name: string;
}

/**
 * Records that may be the parent of `self`: not itself and nothing below it (a
 * record can't be inside itself). With no `self` (a new record), everything.
 */
export function possibleParents<T extends Node>(all: readonly T[], self: Node | null): T[] {
  if (!self) return [...all];
  const prefix = `${self.full_name}:`;
  return all.filter((a) => a.id !== self.id && !a.full_name.startsWith(prefix));
}

/** `Parent:Child` for the editor's title when a draft names its parent. */
export function fullNameOf(name: string, parent: Node | null | undefined): string {
  return parent ? `${parent.full_name}:${name}` : name;
}
