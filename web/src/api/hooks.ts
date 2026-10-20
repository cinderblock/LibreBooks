// TanStack Query hooks, one per resource. Screens use these, never the client
// directly, so caching, invalidation and the SSE refresh are uniform.

import { useDebouncedValue } from '@mantine/hooks';
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useMemo } from 'react';
import type { TxnQuery } from './client';
import { useApi, useCompanyId } from './context';
import { invalidateTxnDerived, qk } from './keys';
import type {
  Account,
  ClearedStatus,
  DateRange,
  Entity,
  Item,
  ListName,
  ListQuery,
  ListRecords,
  RegisterResponse,
  ReportRequest,
  SavedKind,
  Txn,
  TxnType,
  TxnWithPostings,
} from './types';

const LIST_STALE = 60_000;

/** Who's signed in, and to which companies. */
export function useSession() {
  const api = useApi();
  return useQuery({ queryKey: ['session'], queryFn: () => api.session(), retry: false, staleTime: 5 * 60_000 });
}

export function useCompanyInfo() {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.info(co), queryFn: () => api.companyInfo(co), staleTime: 5 * 60_000 });
}

export function useList<L extends ListName>(list: L, q: ListQuery = {}) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.list(co, list, q), queryFn: () => api.list(co, list, q), staleTime: LIST_STALE });
}

/** All accounts including inactive ones (pickers filter; lookups need them all). */
export function useAccounts() {
  return useList('accounts', { include_inactive: true });
}
export function useEntities() {
  return useList('entities', { include_inactive: true });
}
export function useItems() {
  return useList('items', { include_inactive: true });
}
export function useClasses() {
  return useList('classes', { include_inactive: true });
}
export function useTerms() {
  return useList('terms');
}
export function usePaymentMethods() {
  return useList('payment-methods');
}

/** id → record, for rendering names from ids without N lookups. */
export function useIndex<T extends { id: string }>(rows: T[] | undefined): Map<string, T> {
  return useMemo(() => new Map((rows ?? []).map((r) => [r.id, r])), [rows]);
}

export function useAccountIndex(): Map<string, Account> {
  return useIndex(useAccounts().data);
}
export function useEntityIndex(): Map<string, Entity> {
  return useIndex(useEntities().data);
}
export function useItemIndex(): Map<string, Item> {
  return useIndex(useItems().data);
}

export function useTxns(q: TxnQuery, enabled = true) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({
    queryKey: qk.txns(co, q),
    queryFn: () => api.txns(co, q),
    enabled,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
  });
}

export function useTxn(id: string | null | undefined) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({
    queryKey: qk.txn(co, id ?? ''),
    queryFn: () => api.txn(co, id!),
    enabled: !!id,
    staleTime: 30_000,
  });
}

export function useRegister(accountId: string | null, range: Partial<DateRange> = {}) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({
    queryKey: qk.register(co, accountId ?? '', range),
    queryFn: () => api.register(co, accountId!, range),
    enabled: !!accountId,
    staleTime: 30_000,
  });
}

export function useBalances(range: Partial<DateRange> = {}) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.balances(co, range), queryFn: () => api.balances(co, range), staleTime: 30_000 });
}

/** account id → debit-positive balance (all dates). */
export function useBalanceMap(): Map<string, number> {
  const { data } = useBalances();
  return useMemo(() => new Map((data ?? []).map((b) => [b.account_id, b.debit_positive])), [data]);
}

/** The next number in a series: per bank account for checks, per type otherwise. */
export function useNextNumber(type: TxnType, accountId: string | null, enabled = true) {
  const api = useApi();
  const co = useCompanyId();
  const perAccount = type === 'Check' || type === 'BillPaymentCheck';
  return useQuery({
    queryKey: qk.nextNumber(co, type, perAccount ? accountId : null),
    queryFn: () => api.nextNumber(co, type, perAccount ? accountId : null),
    enabled: enabled && (!perAccount || !!accountId),
    staleTime: 0,
  });
}

export function useReportCatalog() {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.reportCatalog(co), queryFn: () => api.reportCatalog(co), staleTime: 10 * 60_000 });
}

export function useReport(req: ReportRequest | null) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({
    queryKey: req ? qk.report(co, req) : [...qk.reportsAll(co), 'none'],
    queryFn: () => api.runReport(co, req!),
    enabled: !!req,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
  });
}

/** Open invoices/credits or bills/credits for a name (Receive Payments, Pay Bills). */
export function useOpenItems(entityId: string | null, kind: 'receivable' | 'payable') {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({
    queryKey: qk.open(co, entityId ?? '', kind),
    queryFn: () => api.open(co, entityId!, kind),
    enabled: !!entityId,
    staleTime: 10_000,
  });
}

/** Payments waiting in Undeposited Funds (Make Deposits). */
export function useUndeposited(enabled = true) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.undeposited(co), queryFn: () => api.undeposited(co), enabled, staleTime: 10_000 });
}

export function useSaved(kind: SavedKind) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.saved(co, kind), queryFn: () => api.saved(co, kind), staleTime: 60_000 });
}

/** Create (no id) or replace (with id) a saved document. */
export function useSaveSaved(kind: SavedKind) {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, name, doc }: { id?: string | null; name: string; doc: unknown }) =>
      id ? api.updateSaved(co, kind, id, name, doc) : api.createSaved(co, kind, name, doc),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: qk.savedAll(co) });
      void qc.invalidateQueries({ queryKey: qk.memorizedDue(co) });
    },
  });
}

export function useDeleteSaved(kind: SavedKind) {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.deleteSaved(co, kind, id),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: qk.savedAll(co) });
      void qc.invalidateQueries({ queryKey: qk.memorizedDue(co) });
    },
  });
}

export function useMemorizedDue() {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.memorizedDue(co), queryFn: () => api.memorizedDue(co), staleTime: 60_000 });
}

export function useSalesTaxCodes() {
  return useList('sales-tax-codes', { include_inactive: true });
}
export function useSalesReps() {
  return useList('sales-reps');
}

/**
 * `POST /txns/preview` for a form as it stands: the server's postings, total
 * and sales tax (the truth for totals) without saving. Debounced, so typing
 * doesn't flood the server; keyed by the transaction's content.
 */
export function useTxnPreview(txn: Txn | null, enabled = true) {
  const api = useApi();
  const co = useCompanyId();
  const [debounced] = useDebouncedValue(txn, 400);
  const key = useMemo(() => (debounced ? JSON.stringify([debounced.txn_type, debounced.entity_id, debounced.account_id, debounced.sales_tax_item_id, debounced.customer_tax_code_id, debounced.lines, debounced.applications, debounced.amount]) : ''), [debounced]);
  return useQuery({
    queryKey: ['c', co, 'preview', key],
    queryFn: () => api.previewTxn(co, debounced!),
    enabled: enabled && !!debounced && debounced.lines.some((l) => l.kind !== 'Text' && (l.amount !== 0 || l.item_id || l.account_id)),
    staleTime: 60_000,
    retry: false,
    placeholderData: keepPreviousData,
  });
}

/** Create (edit_seq 0) or update a transaction; refreshes everything it touches. */
export function useSaveTxn() {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (txn: Txn) => (txn.edit_seq === 0 ? api.createTxn(co, txn) : api.updateTxn(co, txn)),
    onSuccess: (out) => {
      qc.setQueryData<TxnWithPostings>(qk.txn(co, out.txn.id), { txn: out.txn, postings: out.postings });
      invalidateTxnDerived(qc, co, out.txn.id);
    },
  });
}

export function useDeleteTxn() {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, editSeq }: { id: string; editSeq: number }) => api.deleteTxn(co, id, editSeq),
    onSuccess: (_v, { id }) => {
      qc.removeQueries({ queryKey: qk.txn(co, id) });
      invalidateTxnDerived(qc, co);
    },
  });
}

export function useVoidTxn() {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, editSeq }: { id: string; editSeq: number }) => api.voidTxn(co, id, editSeq),
    onSuccess: (out) => {
      qc.setQueryData<TxnWithPostings>(qk.txn(co, out.txn.id), { txn: out.txn, postings: out.postings });
      invalidateTxnDerived(qc, co, out.txn.id);
    },
  });
}

export function useSaveRecord<L extends ListName>(list: L) {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (rec: ListRecords[L]) => (rec.edit_seq === 0 ? api.createRecord(co, list, rec) : api.updateRecord(co, list, rec)),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: qk.listAll(co, list) });
    },
  });
}

export function useDeleteRecord(list: ListName) {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, editSeq }: { id: string; editSeq: number }) => api.deleteRecord(co, list, id, editSeq),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: qk.listAll(co, list) });
    },
  });
}

/**
 * Toggle a register line's cleared mark, optimistically: the ✓ flips at once and
 * rolls back if the server refuses.
 */
export function useSetCleared(accountId: string) {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  const prefix = ['c', co, 'register', accountId] as const;
  return useMutation({
    mutationFn: ({ txnId, status }: { txnId: string; status: ClearedStatus }) => api.setCleared(co, accountId, txnId, status),
    onMutate: async ({ txnId, status }) => {
      await qc.cancelQueries({ queryKey: prefix });
      const snapshot = qc.getQueriesData<RegisterResponse>({ queryKey: prefix });
      qc.setQueriesData<RegisterResponse>({ queryKey: prefix }, (old) =>
        old ? { ...old, lines: old.lines.map((l) => (l.txn_id === txnId ? { ...l, cleared: status } : l)) } : old,
      );
      return { snapshot };
    },
    onError: (_e, _v, ctx) => {
      for (const [key, data] of ctx?.snapshot ?? []) qc.setQueryData(key, data);
    },
  });
}
