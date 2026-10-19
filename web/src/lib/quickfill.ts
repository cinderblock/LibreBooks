// QuickFill matching: QBD-style type-ahead over hierarchical names.
//
// QBD lists are trees written `Parent:Child:Grandchild`. People type the part
// they remember, so `Mat` must find `Cost of Goods Sold:Materials`, `cost:mat`
// must find it level by level, and `goods` must find it by a word. Matches are
// ranked so the most literal one wins the inline completion, and ties keep the
// list's own (hierarchical) order so the dropdown still reads like the tree.
//
// Preparation lowercases and splits once per list (thousands of entries), so a
// keystroke is a linear scan over precomputed strings: fast enough for 10k+.

export interface QuickFillEntry {
  /** `Cost of Goods Sold:Materials`. */
  fullName: string;
  /** Extra prefix-matchable keys: an account number, a SKU, a company name. */
  keys?: readonly string[];
}

export interface Prepared {
  full: string;
  segments: string[];
  words: string[];
  keys: string[];
}

export function prepare(entries: readonly QuickFillEntry[]): Prepared[] {
  return entries.map((e) => {
    const full = e.fullName.toLowerCase();
    const segments = full.split(':');
    const words = segments.flatMap((s) => s.split(/[\s,&/()\-.]+/).filter(Boolean));
    return { full, segments, words, keys: (e.keys ?? []).map((k) => k.toLowerCase()) };
  });
}

/** Lower is better. */
export const Rank = {
  FullPrefix: 0,
  LevelPath: 1,
  SegmentPrefix: 2,
  KeyPrefix: 3,
  WordPrefix: 4,
  Substring: 5,
  None: 99,
} as const;
export type Rank = (typeof Rank)[keyof typeof Rank];

/**
 * `cost:mat` → each part prefixes consecutive segments, starting anywhere. With
 * `childrenOnly` (the query ended in a colon) a deeper segment must follow.
 */
function levelPathMatch(segments: string[], parts: string[], childrenOnly: boolean): boolean {
  const need = parts.length + (childrenOnly ? 1 : 0);
  for (let start = 0; start + need <= segments.length; start++) {
    let ok = true;
    for (let k = 0; k < parts.length; k++) {
      if (!segments[start + k]!.startsWith(parts[k]!.trim())) {
        ok = false;
        break;
      }
    }
    if (ok) return true;
  }
  return false;
}

export function rankOf(p: Prepared, q: string): Rank {
  if (q === '') return Rank.FullPrefix;
  if (p.full.startsWith(q)) return Rank.FullPrefix;
  if (q.includes(':')) {
    const parts = q.split(':');
    // A trailing colon ("cost:") means "the children of Cost…".
    const childrenOnly = parts[parts.length - 1] === '';
    if (childrenOnly) parts.pop();
    return parts.length > 0 && levelPathMatch(p.segments, parts, childrenOnly) ? Rank.LevelPath : Rank.None;
  }
  for (let i = 1; i < p.segments.length; i++) if (p.segments[i]!.startsWith(q)) return Rank.SegmentPrefix;
  for (const k of p.keys) if (k.startsWith(q)) return Rank.KeyPrefix;
  for (const w of p.words) if (w.startsWith(q)) return Rank.WordPrefix;
  if (p.full.includes(q)) return Rank.Substring;
  return Rank.None;
}

/**
 * Indices of the entries matching `query`, best first; ties keep list order.
 * An empty query returns every index in order.
 */
export function search(prepared: readonly Prepared[], query: string): number[] {
  const q = query.trim().toLowerCase();
  if (q === '') return prepared.map((_, i) => i);
  // Bucket by rank: a stable, allocation-light sort for big lists.
  const buckets: number[][] = [[], [], [], [], [], []];
  for (let i = 0; i < prepared.length; i++) {
    const r = rankOf(prepared[i]!, q);
    if (r !== Rank.None) buckets[r]!.push(i);
  }
  return buckets.flat();
}

/**
 * Inline completion as you type: if `fullName` starts with what was typed
 * (case-insensitively), the text to show is the typed part plus the rest of
 * the name, with the rest selected. Null when the best match isn't a prefix match
 * (then the dropdown highlights it but the text is left as typed).
 */
export function inlineCompletion(typed: string, fullName: string): string | null {
  if (typed === '' || !fullName.toLowerCase().startsWith(typed.toLowerCase())) return null;
  if (fullName.length === typed.length) return null;
  return typed + fullName.slice(typed.length);
}

/** The exact (case-insensitive) full-name match, if any: what a typed name commits to. */
export function exactIndex(prepared: readonly Prepared[], text: string): number {
  const t = text.trim().toLowerCase();
  if (t === '') return -1;
  return prepared.findIndex((p) => p.full === t);
}
