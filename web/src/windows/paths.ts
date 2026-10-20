// URL ↔ window. Paths are relative to the company (`/c/:companyId`), so the same
// window table works for every company. Unknown paths return null (the shell
// shows "not found" rather than guessing).

import { SUBJECT_KEYS } from '~/reports/request';
import type { WindowDesc } from './model';

/**
 * Query keys that make a report window distinct: its subject (the filters that
 * name an account, a customer…). Dates, basis and columns change in place (as
 * in QBD, where you modify the open report), so they're not part of the key.
 */
const REPORT_KEYS = SUBJECT_KEYS;

function search(params: URLSearchParams, keys?: readonly string[]): string {
  const entries = [...params].filter(([k]) => !keys || keys.includes(k)).sort(([a], [b]) => a.localeCompare(b));
  const s = new URLSearchParams(entries).toString();
  return s ? `?${s}` : '';
}

export function parseWindowPath(relPath: string): WindowDesc | null {
  const [pathPart = '', query = ''] = relPath.split('?');
  const qs = new URLSearchParams(query);
  const seg = pathPart.split('/').filter(Boolean).map(decodeURIComponent);
  const q = Object.fromEntries(qs);
  const one = (kind: WindowDesc['kind'], key: string, params: Record<string, string> = {}, path = pathPart): WindowDesc => ({
    key,
    kind,
    params: { ...q, ...params },
    path: `${path || '/'}${search(qs)}`,
  });
  if (seg.length === 0 || (seg.length === 1 && seg[0] === 'home')) return one('home', 'home', {}, '/home');
  const [a, b, c] = seg;
  switch (a) {
    case 'accounts':
      if (seg.length === 1) return one('accounts', 'accounts');
      if (seg.length === 3 && c === 'register' && b) return one('register', `register:${b}`, { accountId: b });
      if (seg.length === 3 && c === 'reconcile' && b) return one('reconcile', `reconcile:${b}`, { accountId: b });
      return null;
    case 'customers':
    case 'vendors':
    case 'employees':
      return seg.length === 1 ? one(a, a) : null;
    case 'banking':
      return seg.length === 1 ? one('banking', 'banking') : null;
    case 'feeds':
      return seg.length === 1 ? one('feeds', 'feeds') : null;
    // A prefilled form (hand-off token) is its own window: the token is in the key.
    case 'checks':
      return seg.length === 2 && b === 'new' ? one('check', `check:new${q.draft ? `:${q.draft}` : ''}`) : null;
    case 'txns':
      return seg.length === 2 && b ? one('txn', `txn:${b}`, { txnId: b }) : null;
    case 'reports':
      if (seg.length === 1) return one('reports', 'reports');
      if (seg.length === 2 && b) {
        return { key: `report:${b}${search(qs, REPORT_KEYS)}`, kind: 'report', params: { ...q, report: b }, path: `${pathPart}${search(qs)}` };
      }
      return null;
    case 'find':
      return seg.length === 1 ? one('find', 'find') : null;
    case 'lists':
      return seg.length === 2 && b ? one('list', `list:${b}`, { list: b }) : null;
    case 'company':
      return seg.length === 1 ? one('company', 'company') : null;
    case 'forms':
      return seg.length === 3 && b && c === 'new' ? one('form', `form:${b}${q.draft ? `:${q.draft}` : ''}`, { form: b }) : null;
    case 'memorized':
      return seg.length === 1 ? one('memorized', 'memorized') : null;
    // One attachments window per record (and one for the inbox alone).
    case 'attachments':
      return seg.length === 1 ? one('attachments', `attachments${q.kind && q.id ? `:${q.kind}:${q.id}` : ''}`) : null;
    case 'merge':
      return seg.length === 1 ? one('merge', 'merge') : null;
    case 'account':
      return seg.length === 1 ? one('account', 'account') : null;
    case 'admin':
      if (seg.length === 2 && b === 'users') return one('users', 'users');
      if (seg.length === 2 && b === 'enrollments') return one('enrollments', 'enrollments');
      if (seg.length === 2 && b === 'server') return one('server', 'server');
      return null;
    case 'backups':
      return seg.length === 1 ? one('backups', 'backups') : null;
    case 'import':
      return seg.length === 2 && b === 'qbd' ? one('import', 'import:qbd') : null;
    default:
      return null;
  }
}

function withQuery(path: string, params: Record<string, string | null | undefined>): string {
  const qs = new URLSearchParams(Object.entries(params).filter((e): e is [string, string] => !!e[1]));
  const s = qs.toString();
  return `${path}${s ? `?${s}` : ''}`;
}

/** Builders, so screens never hand-assemble paths. */
export const paths = {
  home: () => '/home',
  accounts: () => '/accounts',
  register: (accountId: string) => `/accounts/${encodeURIComponent(accountId)}/register`,
  reconcile: (accountId: string) => `/accounts/${encodeURIComponent(accountId)}/reconcile`,
  customers: (select?: string) => `/customers${select ? `?select=${encodeURIComponent(select)}` : ''}`,
  vendors: (select?: string) => `/vendors${select ? `?select=${encodeURIComponent(select)}` : ''}`,
  employees: (select?: string) => `/employees${select ? `?select=${encodeURIComponent(select)}` : ''}`,
  banking: () => '/banking',
  /** Bank Feeds; `tab` and `account` preselect a panel and its account. */
  feeds: (params: { tab?: string; account?: string } = {}) => {
    const qs = new URLSearchParams(Object.entries(params).filter((e): e is [string, string] => !!e[1]));
    const s = qs.toString();
    return `/feeds${s ? `?${s}` : ''}`;
  },
  /** A new check; `draft` is a hand-off token (`windows/handoff.ts`) for a prefilled one. */
  newCheck: (draft?: string) => `/checks/new${draft ? `?draft=${encodeURIComponent(draft)}` : ''}`,
  txn: (id: string) => `/txns/${encodeURIComponent(id)}`,
  reports: () => '/reports',
  report: (id: string, params: Record<string, string | null | undefined> = {}) => {
    const qs = new URLSearchParams(Object.entries(params).filter((e): e is [string, string] => !!e[1]));
    const s = qs.toString();
    return `/reports/${encodeURIComponent(id)}${s ? `?${s}` : ''}`;
  },
  find: () => '/find',
  list: (list: string) => `/lists/${encodeURIComponent(list)}`,
  company: () => '/company',
  /** A new form of a kind; `draft` is a hand-off token for a prefilled one. */
  form: (form: string, draft?: string) => `/forms/${encodeURIComponent(form)}/new${draft ? `?draft=${encodeURIComponent(draft)}` : ''}`,
  memorized: () => '/memorized',
  /** The inbox, or one record's attachments (`kind` is `txn`, `entity`, `account`, `item`). */
  attachments: (record?: { kind: string; id: string }) => withQuery('/attachments', { kind: record?.kind, id: record?.id }),
  merge: () => '/merge',
  account: () => '/account',
  users: () => '/admin/users',
  enrollments: () => '/admin/enrollments',
  server: (tab?: string) => withQuery('/admin/server', { tab }),
  backups: () => '/backups',
  qbdImport: (tab?: string) => withQuery('/import/qbd', { tab }),
};
