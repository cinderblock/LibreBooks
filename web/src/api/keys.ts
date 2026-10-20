// TanStack Query keys, all under `['c', companyId, …]` so switching companies or
// a catch-all invalidation is one prefix.

import type { QueryClient } from '@tanstack/react-query';
import type { TxnQuery } from './client';
import type { ChangeEvent, CompanyId, DateRange, FeedQuery, ListName, ListQuery, ReportRequest, SavedKind } from './types';

export const qk = {
  company: (co: CompanyId) => ['c', co] as const,
  info: (co: CompanyId) => ['c', co, 'info'] as const,
  listAll: (co: CompanyId, list: ListName) => ['c', co, 'list', list] as const,
  list: (co: CompanyId, list: ListName, q: ListQuery = {}) => ['c', co, 'list', list, q] as const,
  txnsAll: (co: CompanyId) => ['c', co, 'txns'] as const,
  txns: (co: CompanyId, q: TxnQuery = {}) => ['c', co, 'txns', q] as const,
  txn: (co: CompanyId, id: string) => ['c', co, 'txn', id] as const,
  registerAll: (co: CompanyId) => ['c', co, 'register'] as const,
  register: (co: CompanyId, accountId: string, range: Partial<DateRange> = {}) => ['c', co, 'register', accountId, range] as const,
  balancesAll: (co: CompanyId) => ['c', co, 'balances'] as const,
  balances: (co: CompanyId, range: Partial<DateRange> = {}) => ['c', co, 'balances', range] as const,
  open: (co: CompanyId, entityId: string, kind: string) => ['c', co, 'open', entityId, kind] as const,
  openAll: (co: CompanyId) => ['c', co, 'open'] as const,
  nextNumber: (co: CompanyId, type: string, accountId: string | null) => ['c', co, 'next-number', type, accountId] as const,
  undeposited: (co: CompanyId) => ['c', co, 'undeposited'] as const,
  reportCatalog: (co: CompanyId) => ['c', co, 'reports'] as const,
  reportsAll: (co: CompanyId) => ['c', co, 'report'] as const,
  report: (co: CompanyId, req: ReportRequest) => ['c', co, 'report', req] as const,
  history: (co: CompanyId, id: string) => ['c', co, 'history', id] as const,
  reconcileAll: (co: CompanyId) => ['c', co, 'reconcile'] as const,
  reconcileOverview: (co: CompanyId, accountId: string) => ['c', co, 'reconcile', 'account', accountId] as const,
  reconciliation: (co: CompanyId, id: string) => ['c', co, 'reconcile', 'id', id] as const,
  feedsAll: (co: CompanyId) => ['c', co, 'feeds'] as const,
  feedsSummary: (co: CompanyId) => ['c', co, 'feeds', 'summary'] as const,
  feedTxns: (co: CompanyId, q: FeedQuery = {}) => ['c', co, 'feeds', 'txns', q] as const,
  feedCandidates: (co: CompanyId, id: string) => ['c', co, 'feeds', 'candidates', id] as const,
  feedConnections: (co: CompanyId) => ['c', co, 'feeds', 'connections'] as const,
  feedRules: (co: CompanyId) => ['c', co, 'feeds', 'rules'] as const,
  savedAll: (co: CompanyId) => ['c', co, 'saved'] as const,
  saved: (co: CompanyId, kind: SavedKind) => ['c', co, 'saved', kind] as const,
  memorizedDue: (co: CompanyId) => ['c', co, 'memorized-due'] as const,
};

/** Everything that depends on transactions' postings. */
export function invalidateTxnDerived(qc: QueryClient, co: CompanyId, txnId?: string) {
  void qc.invalidateQueries({ queryKey: qk.txnsAll(co) });
  void qc.invalidateQueries({ queryKey: qk.registerAll(co) });
  void qc.invalidateQueries({ queryKey: qk.balancesAll(co) });
  void qc.invalidateQueries({ queryKey: qk.openAll(co) });
  void qc.invalidateQueries({ queryKey: qk.undeposited(co) });
  void qc.invalidateQueries({ queryKey: qk.reconcileAll(co) });
  void qc.invalidateQueries({ queryKey: qk.reportsAll(co) });
  void qc.invalidateQueries({ queryKey: ['c', co, 'next-number'] });
  if (txnId) {
    void qc.invalidateQueries({ queryKey: qk.txn(co, txnId) });
    void qc.invalidateQueries({ queryKey: qk.history(co, txnId) });
  }
}

const LIST_OF_KIND: Record<string, ListName> = {
  account: 'accounts',
  entity: 'entities',
  item: 'items',
  class: 'classes',
  terms: 'terms',
  payment_method: 'payment-methods',
  sales_tax_code: 'sales-tax-codes',
  sales_rep: 'sales-reps',
  type: 'types',
};

/**
 * What a server change event makes stale. Kinds we don't recognise invalidate
 * the whole company: correct, if heavy, until the event vocabulary settles.
 */
export function invalidateForEvent(qc: QueryClient, co: CompanyId, e: ChangeEvent) {
  if (e.kind === 'txn') {
    invalidateTxnDerived(qc, co, e.id);
    return;
  }
  const list = LIST_OF_KIND[e.kind];
  if (list) {
    void qc.invalidateQueries({ queryKey: qk.listAll(co, list) });
    // Names and accounts show up inside registers, reports and transaction lists.
    if (list === 'accounts' || list === 'entities' || list === 'items') invalidateTxnDerived(qc, co);
    return;
  }
  if (e.kind === 'company' || e.kind === 'info') {
    void qc.invalidateQueries({ queryKey: qk.info(co) });
    return;
  }
  if (e.kind.startsWith('feed')) {
    void qc.invalidateQueries({ queryKey: qk.feedsAll(co) });
    // A match or add marks a register line as downloaded.
    void qc.invalidateQueries({ queryKey: qk.registerAll(co) });
    return;
  }
  if (e.kind === 'saved' || e.kind === 'memorized') {
    void qc.invalidateQueries({ queryKey: qk.savedAll(co) });
    void qc.invalidateQueries({ queryKey: qk.memorizedDue(co) });
    return;
  }
  void qc.invalidateQueries({ queryKey: qk.company(co) });
}
