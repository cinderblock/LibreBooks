// Helpers for QBD's `Parent:Child` lists (accounts, names, items, classes).

export interface TreeNode {
  id: string;
  parent_id: string | null;
  full_name: string;
}

/**
 * Compare full names level by level, case-insensitively, so every child sorts
 * directly under its parent. (Plain string order would put "Sales Tax" between
 * "Sales" and "Sales:Cabinetry", since ' ' sorts before ':'.)
 */
export function hierarchicalCompare(a: string, b: string): number {
  const as = a.split(':');
  const bs = b.split(':');
  const n = Math.min(as.length, bs.length);
  for (let i = 0; i < n; i++) {
    const c = as[i]!.localeCompare(bs[i]!, undefined, { sensitivity: 'base', numeric: true });
    if (c !== 0) return c;
  }
  return as.length - bs.length;
}

/**
 * Totals including every descendant ("Balance Total" on the chart of accounts,
 * a parent row's total on a report). `own` maps id → the node's own amount.
 */
export function rollUp<T extends TreeNode>(nodes: readonly T[], own: ReadonlyMap<string, number>): Map<string, number> {
  const total = new Map<string, number>();
  const byId = new Map(nodes.map((n) => [n.id, n]));
  for (const n of nodes) {
    const amount = own.get(n.id) ?? 0;
    if (amount === 0) continue;
    // Walk up the parent chain adding this node's own amount to every ancestor.
    let cur: T | undefined = n;
    let guard = 0;
    while (cur && guard++ < 64) {
      total.set(cur.id, (total.get(cur.id) ?? 0) + amount);
      cur = cur.parent_id ? byId.get(cur.parent_id) : undefined;
    }
  }
  return total;
}

/** Children lists keyed by parent id (`''` for top level), in the input order. */
export function childrenOf<T extends TreeNode>(nodes: readonly T[]): Map<string, T[]> {
  const out = new Map<string, T[]>();
  for (const n of nodes) {
    const key = n.parent_id ?? '';
    const list = out.get(key);
    if (list) list.push(n);
    else out.set(key, [n]);
  }
  return out;
}

/** The leaf part of a full name: `Cost of Goods Sold:Materials` → `Materials`. */
export function leafName(fullName: string): string {
  const i = fullName.lastIndexOf(':');
  return i < 0 ? fullName : fullName.slice(i + 1);
}

/** The parent path of a full name, or '' at the top level. */
export function parentPath(fullName: string): string {
  const i = fullName.lastIndexOf(':');
  return i < 0 ? '' : fullName.slice(0, i);
}
