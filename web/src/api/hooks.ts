// TanStack Query hooks, one per resource. Screens use these, never the client
// directly, so caching, invalidation and the SSE refresh are uniform.

import { useDebouncedValue } from '@mantine/hooks';
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useMemo } from 'react';
import type { TxnQuery } from './client';
import { useApi, useCompanyId } from './context';
import { invalidateTxnDerived, qk, sk } from './keys';
import type {
  Account,
  AttachmentQuery,
  ClearedStatus,
  DateRange,
  Entity,
  FeedQuery,
  ImportStatus,
  Item,
  ListName,
  ListQuery,
  ListRecords,
  NameBalance,
  ParityStatus,
  ReconciliationView,
  RegisterResponse,
  ReportRequest,
  SavedKind,
  StartReconcile,
  Txn,
  TxnType,
  TxnWithPostings,
} from './types';

const LIST_STALE = 60_000;

/** Who's signed in, and to which companies. */
export function useSession() {
  const api = useApi();
  return useQuery({ queryKey: sk.session, queryFn: () => api.session(), retry: false, staleTime: 5 * 60_000 });
}

/** Server-wide admin: sees the Users, Enrolment, Server and Import windows. */
export function useIsAdmin(): boolean {
  return useSession().data?.user.role === 'admin';
}

/** Invalidate a set of keys after a mutation; the screens' one-liner. */
export function useInvalidate(): (...keys: ReadonlyArray<readonly unknown[]>) => void {
  const qc = useQueryClient();
  return (...keys) => {
    for (const k of keys) void qc.invalidateQueries({ queryKey: k as unknown[] });
  };
}

// ---- users, enrolment, my account, server ---------------------------------------

export function useUsers(enabled = true) {
  const api = useApi();
  return useQuery({ queryKey: sk.users, queryFn: () => api.users(), enabled, staleTime: 30_000 });
}

export function useUser(id: string | null) {
  const api = useApi();
  return useQuery({ queryKey: sk.user(id ?? ''), queryFn: () => api.user(id!), enabled: !!id, staleTime: 30_000 });
}

export function useEnrollments(enabled = true) {
  const api = useApi();
  // Links expire on their own; keep the list honest while the window is open.
  return useQuery({ queryKey: sk.enrollments, queryFn: () => api.enrollments(), enabled, staleTime: 10_000, refetchInterval: 60_000 });
}

export function usePasskeys() {
  const api = useApi();
  return useQuery({ queryKey: sk.passkeys, queryFn: () => api.passkeys(), staleTime: 30_000 });
}

export function useDevices() {
  const api = useApi();
  return useQuery({ queryKey: sk.devices, queryFn: () => api.devices(), staleTime: 30_000 });
}

export function useSessions() {
  const api = useApi();
  return useQuery({ queryKey: sk.sessions, queryFn: () => api.sessions(), staleTime: 10_000 });
}

export function useServer(enabled = true) {
  const api = useApi();
  return useQuery({ queryKey: sk.serverView, queryFn: () => api.server(), enabled, staleTime: 15_000 });
}

export function useUpdate(enabled = true) {
  const api = useApi();
  return useQuery({ queryKey: sk.update, queryFn: () => api.update(), enabled, staleTime: 15_000 });
}

export function useTunnel(enabled = true) {
  const api = useApi();
  return useQuery({
    queryKey: sk.tunnel,
    queryFn: () => api.tunnel(),
    enabled,
    staleTime: 5_000,
    // cloudflared connects in the background; watch it while it's getting there.
    refetchInterval: (q) => {
      const st = q.state.data?.status.state.state;
      return st === 'starting' || st === 'reconnecting' ? 2_000 : false;
    },
  });
}

// ---- company: closing date, backups, attachments, QuickBooks import -------------

export function useClosingDate() {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.closingDate(co), queryFn: () => api.closingDate(co), staleTime: 60_000 });
}

export function useBackups(enabled = true) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.backups(co), queryFn: () => api.backups(co), enabled, staleTime: 15_000 });
}

export function useBackupsStatus(enabled = true) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.backupsStatus(co), queryFn: () => api.backupsStatus(co), enabled, staleTime: 15_000 });
}

/** The inbox (no record), or what's attached to one record. */
export function useAttachments(query: AttachmentQuery = {}, enabled = true) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.attachments(co, query), queryFn: () => api.attachments(co, query), enabled, staleTime: 15_000 });
}

/**
 * The QuickBooks import in one document (`ImportStatus`): the settings, what
 * is set, the running job and the last reports. While a job runs the server
 * long-polls (`wait`), answering on every progress step, and we ask again as
 * soon as it answers, so the progress bar is live without a fast timer; idle,
 * a plain fetch now and then. The `import` change events (`started`,
 * `finished`, `failed`) refetch it through `invalidateForEvent`, so a job
 * begun elsewhere shows up too.
 */
export function useQbdStatus(enabled = true) {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  return useQuery({
    queryKey: qk.qbdStatus(co),
    queryFn: () => {
      const running = !!qc.getQueryData<ImportStatus>(qk.qbdStatus(co))?.running;
      return api.qbdStatus(co, running ? 20 : undefined);
    },
    enabled,
    staleTime: 2_000,
    refetchInterval: (q) => (q.state.data?.running ? 400 : false),
  });
}

/** The last parity report; polled the same way while a parity job runs. */
export function useQbdParity(enabled = true) {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  return useQuery({
    queryKey: qk.qbdParity(co),
    queryFn: () => {
      const running = !!qc.getQueryData<ParityStatus>(qk.qbdParity(co))?.running;
      return api.qbdParity(co, running ? 20 : undefined);
    },
    enabled,
    staleTime: 2_000,
    refetchInterval: (q) => (q.state.data?.running ? 400 : false),
  });
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

/** Per-name A/R and A/P balances with jobs rolled up (the centres' Balance and Balance Total). */
export function useNameBalances(enabled = true) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.nameBalances(co), queryFn: () => api.nameBalances(co), enabled, staleTime: 30_000 });
}

/** entity id → its `NameBalance`; names with nothing open are absent (a zero balance). */
export function useNameBalanceMap(enabled = true): Map<string, NameBalance> {
  const { data } = useNameBalances(enabled);
  return useMemo(() => new Map((data ?? []).map((b) => [b.entity_id, b])), [data]);
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

export function useReconcileOverview(accountId: string | null) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.reconcileOverview(co, accountId ?? ''), queryFn: () => api.reconcileOverview(co, accountId!), enabled: !!accountId, staleTime: 10_000 });
}

export function useReconciliation(id: string | null) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.reconciliation(co, id ?? ''), queryFn: () => api.reconciliation(co, id!), enabled: !!id, staleTime: 5_000 });
}

/** Start, mark, finish, discard and undo; each refreshes the reconcile queries and the register. */
export function useReconcileActions(accountId: string | null) {
  const api = useApi();
  const co = useCompanyId();
  const qc = useQueryClient();
  const refresh = () => {
    void qc.invalidateQueries({ queryKey: qk.reconcileAll(co) });
    void qc.invalidateQueries({ queryKey: qk.registerAll(co) });
  };
  const start = useMutation({ mutationFn: (body: StartReconcile) => api.startReconcile(co, accountId!, body), onSuccess: refresh });
  const mark = useMutation({
    mutationFn: ({ id, txnIds, marked }: { id: string; txnIds: string[]; marked: boolean }) => api.markReconcile(co, id, txnIds, marked),
    // The ✓ flips at once; the server's status replaces ours when it answers.
    onMutate: async ({ id, txnIds, marked }) => {
      await qc.cancelQueries({ queryKey: qk.reconciliation(co, id) });
      const before = qc.getQueryData<ReconciliationView>(qk.reconciliation(co, id));
      if (before) qc.setQueryData<ReconciliationView>(qk.reconciliation(co, id), { ...before, lines: before.lines.map((l) => (txnIds.includes(l.txn_id) ? { ...l, marked } : l)) });
      return { before };
    },
    onError: (_e, { id }, ctx) => {
      if (ctx?.before) qc.setQueryData(qk.reconciliation(co, id), ctx.before);
    },
    onSuccess: (status, { id }) => {
      qc.setQueryData<ReconciliationView>(qk.reconciliation(co, id), (old) => (old ? { ...old, status } : old));
    },
  });
  const finish = useMutation({ mutationFn: ({ id, adjust }: { id: string; adjust: boolean }) => api.finishReconcile(co, id, adjust), onSuccess: () => { refresh(); invalidateTxnDerived(qc, co); } });
  const discard = useMutation({ mutationFn: (id: string) => api.discardReconcile(co, id), onSuccess: refresh });
  const undo = useMutation({ mutationFn: () => api.undoReconcile(co, accountId!), onSuccess: refresh });
  return { start, mark, finish, discard, undo };
}

// ---- bank feeds ---------------------------------------------------------------

export function useFeedsSummary() {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.feedsSummary(co), queryFn: () => api.feedsSummary(co), staleTime: 10_000 });
}

export function useFeedTxns(q: FeedQuery, enabled = true) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.feedTxns(co, q), queryFn: () => api.feedTxns(co, q), enabled, staleTime: 10_000, placeholderData: keepPreviousData });
}

export function useFeedCandidates(id: string | null) {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.feedCandidates(co, id ?? ''), queryFn: () => api.feedCandidates(co, id!), enabled: !!id, staleTime: 10_000 });
}

export function useFeedConnections() {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.feedConnections(co), queryFn: () => api.feedConnections(co), staleTime: 30_000 });
}

export function useFeedRules() {
  const api = useApi();
  const co = useCompanyId();
  return useQuery({ queryKey: qk.feedRules(co), queryFn: () => api.feedRules(co), staleTime: 30_000 });
}

/** Every feed write refreshes the feed queries and the registers (downloaded marks). */
export function useFeedInvalidate() {
  const co = useCompanyId();
  const qc = useQueryClient();
  return () => {
    void qc.invalidateQueries({ queryKey: qk.feedsAll(co) });
    invalidateTxnDerived(qc, co);
  };
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
