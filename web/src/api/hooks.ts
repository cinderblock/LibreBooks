// TanStack Query hooks, one per resource. Screens use these, never the client
// directly, so caching, invalidation and the SSE refresh are uniform.

import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useMemo } from 'react';
import type { ReportRequest } from '~/reports/types';
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
  Txn,
  TxnType,
  TxnWithPostings,
} from './types';

const LIST_STALE = 60_000;

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

export function useNextNumber(type: TxnType, accountId: string | null, enabled = true) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({
    queryKey: qk.nextNumber(co, type, accountId),
    queryFn: () => api.nextNumber(co, type, accountId),
    enabled: enabled && !!accountId,
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
    queryKey: qk.report(co, req ?? { report: '' }),
    queryFn: () => api.runReport(co, req!),
    enabled: !!req?.report,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
  });
}

/** Create (edit_seq 0) or update a transaction; refreshes everything it touches. */
export function useSaveTxn() {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (txn: Txn) => (txn.edit_seq === 0n ? api.createTxn(co, txn) : api.updateTxn(co, txn)),
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
    mutationFn: ({ id, editSeq }: { id: string; editSeq: bigint }) => api.deleteTxn(co, id, editSeq),
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
    mutationFn: ({ id, editSeq }: { id: string; editSeq: bigint }) => api.voidTxn(co, id, editSeq),
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
    mutationFn: (rec: ListRecords[L]) => (rec.edit_seq === 0n ? api.createRecord(co, list, rec) : api.updateRecord(co, list, rec)),
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
    mutationFn: ({ id, editSeq }: { id: string; editSeq: bigint }) => api.deleteRecord(co, list, id, editSeq),
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
