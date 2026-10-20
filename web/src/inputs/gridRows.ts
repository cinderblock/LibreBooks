// Row bookkeeping shared by line grids (pure, so it's unit-tested without React).

/** Exactly one blank line at the end; blank lines elsewhere are the user's. */
export function withTrailingBlank<R>(rows: R[], isBlank: (r: R) => boolean, makeBlank: () => R): R[] {
  let end = rows.length;
  while (end > 1 && isBlank(rows[end - 1]!) && isBlank(rows[end - 2]!)) end--;
  const trimmed = end === rows.length ? rows : rows.slice(0, end);
  return trimmed.length === 0 || !isBlank(trimmed[trimmed.length - 1]!) ? [...trimmed, makeBlank()] : trimmed;
}
