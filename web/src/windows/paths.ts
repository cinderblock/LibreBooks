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
      return null;
    case 'customers':
    case 'vendors':
    case 'employees':
      return seg.length === 1 ? one(a, a) : null;
    case 'banking':
      return seg.length === 1 ? one('banking', 'banking') : null;
    case 'checks':
      return seg.length === 2 && b === 'new' ? one('check', 'check:new') : null;
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
      return seg.length === 3 && b && c === 'new' ? one('form', `form:${b}`, { form: b }) : null;
    default:
      return null;
  }
}

/** Builders, so screens never hand-assemble paths. */
export const paths = {
  home: () => '/home',
  accounts: () => '/accounts',
  register: (accountId: string) => `/accounts/${encodeURIComponent(accountId)}/register`,
  customers: (select?: string) => `/customers${select ? `?select=${encodeURIComponent(select)}` : ''}`,
  vendors: (select?: string) => `/vendors${select ? `?select=${encodeURIComponent(select)}` : ''}`,
  employees: (select?: string) => `/employees${select ? `?select=${encodeURIComponent(select)}` : ''}`,
  banking: () => '/banking',
  newCheck: () => '/checks/new',
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
  form: (form: string) => `/forms/${encodeURIComponent(form)}/new`,
};
