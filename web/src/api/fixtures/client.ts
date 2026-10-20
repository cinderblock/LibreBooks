// FIXTURE ONLY: an in-memory implementation of the whole `ApiClient` interface,
// seeded with the synthetic company in ./seed.ts. It behaves like the server
// where the UI can tell: optimistic concurrency (stale `edit_seq` → 409), posting
// errors (422 with a `PostingError`), `in_use` deletes, duplicate names, change
// events after every write, and a little latency so loading states are real.
// State lives only in memory: a reload starts over.

import { blankLine, blankTxn } from '../blank';
import { ApiError } from '../errors';
import type { ApiClient, EventHandlers, EventSubscription, FileDownload, TxnQuery } from '../client';
import { clone } from '../json';
import type {
  Account,
  AccountBalance,
  AddRequest,
  AmazonPreview,
  AuditEntry,
  CatalogEntry,
  ChangeEvent,
  ClearedStatus,
  CompanyId,
  CompanyInfo,
  CompanySummary,
  DateRange,
  Entity,
  FeedConnection,
  FeedImportRequest,
  FeedQuery,
  FeedRule,
  Health,
  ListName,
  ListQuery,
  ListRecords,
  Memorized,
  MemorizedDue,
  Posting,
  PreviewOutcome,
  ReconcileLine,
  ReconcileOverview,
  ReconcileStatus,
  Reconciliation,
  ReconciliationView,
  RegisterLine,
  RegisterResponse,
  Report,
  ReportRequest,
  Saved,
  SavedKind,
  SaveOutcome,
  Session,
  StartReconcile,
  StatementCharge,
  Txn,
  TxnSummary,
  TxnType,
  TxnWithPostings,
} from '../types';
import { accountTypeOrder, naturalBalance } from '~/lib/accountTypes';
import { addDays, addMonths, todayIso } from '~/lib/dates';
import { hierarchicalCompare } from '~/lib/tree';
import { reportToCsv } from '~/reports/rows';
import { FixtureFeeds } from './feeds';
import { fixturePost, type PostingBooks } from './post';
import { FIXTURE_COMPANY_ID, buildSeed, stressNames } from './seed';
import { fixtureCatalog, runFixtureReport } from './reports';

export interface StoredTxn {
  txn: Txn;
  postings: Posting[];
  total: number;
  /** Creation order, the tie-breaker within a day. */
  seq: number;
}

export interface FixtureOptions {
  /** Simulated round-trip time in ms (default 90). */
  latencyMs?: number;
  /** Add thousands of names and a 100k-line register (`?fixtures=stress`). */
  stress?: boolean;
  today?: string;
}

type Lists = { [L in ListName]: ListRecords[L][] };

const STRESS_ACCOUNT_ID = '00000000-0000-4000-9fff-000000000001';
const AR_TYPES: TxnType[] = ['Invoice', 'CreditMemo', 'StatementCharge'];
const AP_TYPES: TxnType[] = ['Bill', 'VendorCredit'];
const PAYMENT_TYPES: TxnType[] = ['ReceivePayment', 'BillPaymentCheck', 'BillPaymentCreditCard'];

export class FixtureApiClient implements ApiClient {
  readonly mode = 'fixtures' as const;
  readonly today: string;
  private readonly latency: number;
  private info: CompanyInfo;
  private lists: Lists;
  private txnStore = new Map<string, StoredTxn>();
  private seq = 0;
  private listeners = new Set<EventHandlers>();
  private audit: AuditEntry[] = [];
  private stressLines: RegisterLine[] | null = null;
  /** Memorized transactions and reports (`/saved/:kind`). */
  private savedDocs: Saved[] = [];
  /** Reconciliations, in progress (finished_at null) and finished. */
  private reconciliations: Reconciliation[] = [];
  /** Register lines a bank feed matched or added (txn id → accounts). */
  private downloaded = new Map<string, Set<string>>();
  private feeds: FixtureFeeds;

  constructor(opts: FixtureOptions = {}) {
    this.latency = opts.latencyMs ?? 90;
    this.today = opts.today ?? todayIso();
    const seed = buildSeed(this.today);
    this.info = seed.info;
    this.lists = {
      accounts: seed.accounts,
      entities: seed.entities,
      items: seed.items,
      classes: seed.classes,
      terms: seed.terms,
      'payment-methods': seed.paymentMethods,
      'sales-tax-codes': seed.salesTaxCodes,
      'sales-reps': seed.salesReps,
      types: seed.types,
    };
    if (opts.stress) {
      this.lists.entities.push(...stressNames(5000));
      this.lists.accounts.push({
        ...seed.accounts[0]!,
        id: STRESS_ACCOUNT_ID,
        name: 'Stress Test Checking',
        full_name: 'Stress Test Checking',
        number: '1099',
        role: null,
        description: '100,000 synthetic register lines',
      });
    }
    for (const t of seed.txns) this.loadSeedTxn(t);
    this.feeds = new FixtureFeeds({
      today: this.today,
      accounts: () => this.lists.accounts,
      entities: () => this.lists.entities,
      roleAccount: (role) => this.lists.accounts.find((a) => a.role === role),
      txns: () => [...this.txnStore.values()],
      txn: (id) => this.txnStore.get(id),
      createTxn: (txn) => {
        this.validate(txn);
        const posted = this.post(txn);
        const now = new Date().toISOString();
        const saved: Txn = { ...clone(txn), amount: posted.total, edit_seq: 1, created_at: now, modified_at: now, created_by: 'fixture-user', modified_by: 'fixture-user' };
        const stored = { txn: saved, postings: posted.postings, total: posted.total, seq: this.seq++ };
        this.txnStore.set(saved.id, stored);
        this.record('created', null, saved);
        this.emit('txn', saved.id, 'created');
        return stored;
      },
      deleteTxn: (id) => {
        const s = this.txnStore.get(id);
        if (!s) return;
        this.txnStore.delete(id);
        this.record('deleted', s.txn, null);
        this.emit('txn', id, 'deleted');
      },
      setDownloaded: (txnId, accountId, on) => {
        const set = this.downloaded.get(txnId) ?? new Set<string>();
        if (on) set.add(accountId);
        else set.delete(accountId);
        this.downloaded.set(txnId, set);
        this.emit('txn', txnId, 'updated');
      },
      emit: (kind, id, action) => this.emit(kind, id, action),
    });
  }

  // ---- plumbing -------------------------------------------------------------

  private async delay<T>(value: () => T): Promise<T> {
    const ms = this.latency ? this.latency * (0.6 + Math.random() * 0.8) : 0;
    if (ms) await new Promise((r) => setTimeout(r, ms));
    return clone(value());
  }

  private emit(kind: string, id: string, action: string) {
    const ev: ChangeEvent = { kind, id, action };
    setTimeout(() => this.listeners.forEach((h) => h.onChange(ev)), 20);
  }

  private checkCompany(co: CompanyId) {
    if (co !== FIXTURE_COMPANY_ID) throw new ApiError(404, 'not_found', 'No such company');
  }

  private postingBooks(): PostingBooks {
    const accounts = new Map(this.lists.accounts.map((a) => [a.id, a]));
    const items = new Map(this.lists.items.map((i) => [i.id, i]));
    const entities = new Map(this.lists.entities.map((e) => [e.id, e]));
    return {
      account: (id) => accounts.get(id),
      roleAccount: (role) => this.lists.accounts.find((a) => a.role === role),
      item: (id) => items.get(id),
      entity: (id) => entities.get(id),
      txnExists: (id) => this.txnStore.has(id),
    };
  }

  private post(txn: Txn): { postings: Posting[]; total: number; sales_tax: number } {
    const r = fixturePost(txn, this.postingBooks());
    if ('code' in r) throw new ApiError(422, 'posting', 'Posting rules refused it', r);
    return r;
  }

  /** Seed transactions carry placeholders the books fill in (payment amounts, deposits). */
  private loadSeedTxn(t: Txn) {
    if (t.txn_type === 'ReceivePayment' && t.amount < 0) {
      const target = this.txnStore.get(t.applications[0]!.target_txn_id);
      t.amount = target?.total ?? 0;
      t.applications[0]!.amount = t.amount;
    }
    if (t.extra && (t.extra as { fixture_sweep_undeposited?: boolean }).fixture_sweep_undeposited) {
      t.extra = null;
      const uf = this.lists.accounts.find((a) => a.role === 'UndepositedFunds')!;
      const deposited = new Set(
        [...this.txnStore.values()].flatMap((s) => s.txn.lines.map((l) => l.linked_txn_id).filter(Boolean)),
      );
      t.lines = [...this.txnStore.values()]
        .filter((s) => s.txn.txn_type === 'ReceivePayment' && s.txn.date <= t.date && !deposited.has(s.txn.id))
        .map((s) => ({
          ...blankLine('DepositLine'),
          account_id: uf.id,
          entity_id: s.txn.entity_id,
          amount: s.total,
          linked_txn_id: s.txn.id,
          payment_method_id: s.txn.payment_method_id,
          check_number: s.txn.number,
        }));
      if (t.lines.length === 0) return;
    }
    const posted = this.post(t);
    t.amount = posted.total;
    t.edit_seq = 1;
    t.created_at = `${t.date}T16:00:00Z`;
    t.modified_at = t.created_at;
    // Bank lines older than a month have cleared (gives the register some ✓s).
    const cutoff = addDays(this.today, -30);
    const postings = posted.postings.map((p) => ({ ...p, cleared: t.date < cutoff ? ('Cleared' as const) : p.cleared }));
    this.txnStore.set(t.id, { txn: t, postings, total: posted.total, seq: this.seq++ });
  }

  // ---- server level ---------------------------------------------------------

  // Fixture mode is always signed in as the demo user; there's nothing to sign in to.
  private noAuth(): Promise<never> {
    return Promise.reject(new ApiError(400, 'invalid', 'Sign-in is not available in fixture mode; you are already the demo user.'));
  }
  loginStart() {
    return this.noAuth();
  }
  loginFinish() {
    return this.noAuth();
  }
  enrolStart() {
    return this.noAuth();
  }
  enrolFinish() {
    return this.noAuth();
  }
  logout(): Promise<void> {
    return this.delay(() => undefined);
  }

  health(): Promise<Health> {
    return this.delay(() => ({ ok: true as const, version: 'fixtures', mode: 'server' as const }));
  }
  session(): Promise<Session> {
    return this.delay(() => ({
      user: { id: 'fixture-user', display_name: 'Demo User', role: 'admin' as const, kind: 'local' as const },
      companies: [this.summary()],
      sign_in: false,
    }));
  }
  private summary(): CompanySummary {
    return { id: FIXTURE_COMPANY_ID, name: this.info.name, role: 'admin' as const };
  }
  companies(): Promise<CompanySummary[]> {
    return this.delay(() => [this.summary()]);
  }
  companyInfo(co: CompanyId): Promise<CompanyInfo> {
    this.checkCompany(co);
    return this.delay(() => this.info);
  }

  // ---- lists ----------------------------------------------------------------

  private sorted<L extends ListName>(list: L): ListRecords[L][] {
    const rows = this.lists[list] as ListRecords[L][];
    if (list === 'accounts') {
      return [...(rows as Account[])].sort(
        (a, b) => accountTypeOrder(a.account_type) - accountTypeOrder(b.account_type) || hierarchicalCompare(a.full_name, b.full_name),
      ) as ListRecords[L][];
    }
    if (rows.length && 'full_name' in (rows[0] as object)) {
      return [...rows].sort((a, b) =>
        hierarchicalCompare((a as { full_name: string }).full_name, (b as { full_name: string }).full_name),
      );
    }
    return [...rows];
  }

  list<L extends ListName>(co: CompanyId, list: L, query: ListQuery = {}): Promise<ListRecords[L][]> {
    this.checkCompany(co);
    return this.delay(() =>
      this.sorted(list).filter((r) => {
        const rec = r as { is_active?: boolean; kind?: string; item_type?: string };
        if (!query.include_inactive && rec.is_active === false) return false;
        if (query.kind && rec.kind !== query.kind) return false;
        if (query.type && rec.item_type !== query.type) return false;
        return true;
      }),
    );
  }

  getRecord<L extends ListName>(co: CompanyId, list: L, id: string): Promise<ListRecords[L]> {
    this.checkCompany(co);
    const rec = (this.lists[list] as ListRecords[L][]).find((r) => r.id === id);
    if (!rec) return Promise.reject(new ApiError(404, 'not_found', 'Not found'));
    return this.delay(() => rec);
  }

  /** Recompute `full_name`/`sublevel` from the parent chain, check uniqueness. */
  private normalize<L extends ListName>(list: L, record: ListRecords[L]): ListRecords[L] {
    const r = { ...record } as ListRecords[L] & { name?: string; full_name?: string; parent_id?: string | null; sublevel?: number };
    if ('full_name' in r && typeof r.name === 'string') {
      const all = this.lists[list] as Array<{ id: string; full_name: string; sublevel?: number }>;
      const parent = r.parent_id ? all.find((p) => p.id === r.parent_id) : undefined;
      if (r.parent_id && !parent) throw new ApiError(400, 'invalid', 'The parent does not exist', { field: 'parent_id' });
      r.full_name = parent ? `${parent.full_name}:${r.name}` : r.name;
      if ('sublevel' in r) r.sublevel = parent ? (parent.sublevel ?? 0) + 1 : 0;
      if (!r.name.trim()) throw new ApiError(400, 'invalid', 'A name is required', { field: 'name' });
      const dup = all.find((o) => o.id !== r.id && o.full_name.toLowerCase() === r.full_name!.toLowerCase());
      if (dup) throw new ApiError(409, 'duplicate', `"${r.full_name}" is already in use`, { kind: 'duplicate' });
    }
    return r;
  }

  createRecord<L extends ListName>(co: CompanyId, list: L, record: ListRecords[L]): Promise<ListRecords[L]> {
    this.checkCompany(co);
    try {
      const r = this.normalize(list, record);
      r.edit_seq = 1;
      (this.lists[list] as ListRecords[L][]).push(r);
      this.emit(kindOfList(list), r.id, 'created');
      return this.delay(() => r);
    } catch (e) {
      return Promise.reject(e);
    }
  }

  updateRecord<L extends ListName>(co: CompanyId, list: L, record: ListRecords[L]): Promise<ListRecords[L]> {
    this.checkCompany(co);
    const rows = this.lists[list] as ListRecords[L][];
    const i = rows.findIndex((r) => r.id === record.id);
    if (i < 0) return Promise.reject(new ApiError(404, 'not_found', 'Not found'));
    const current = rows[i]!;
    if (current.edit_seq !== record.edit_seq) {
      return Promise.reject(
        new ApiError(409, 'conflict', 'Someone else changed this record', { expected: Number(record.edit_seq), actual: Number(current.edit_seq) }),
      );
    }
    try {
      const r = this.normalize(list, record);
      r.edit_seq = current.edit_seq + 1;
      rows[i] = r;
      // Renaming a parent renames its children's full names.
      if ('full_name' in r) {
        for (const child of rows) {
          const c = child as { parent_id?: string | null; id: string };
          if (c.parent_id === r.id) rows[rows.indexOf(child)] = this.normalize(list, child);
        }
      }
      this.emit(kindOfList(list), r.id, 'updated');
      return this.delay(() => r);
    } catch (e) {
      return Promise.reject(e);
    }
  }

  deleteRecord(co: CompanyId, list: ListName, id: string, editSeq: number): Promise<void> {
    this.checkCompany(co);
    const rows = this.lists[list] as Array<{ id: string; edit_seq: number; parent_id?: string | null }>;
    const i = rows.findIndex((r) => r.id === id);
    if (i < 0) return Promise.reject(new ApiError(404, 'not_found', 'Not found'));
    if (rows[i]!.edit_seq !== editSeq) {
      return Promise.reject(new ApiError(409, 'conflict', 'Someone else changed this record', { expected: Number(editSeq), actual: Number(rows[i]!.edit_seq) }));
    }
    const used =
      rows.some((r) => r.parent_id === id) ||
      [...this.txnStore.values()].some(
        (s) =>
          s.txn.entity_id === id ||
          s.txn.account_id === id ||
          s.txn.to_account_id === id ||
          s.postings.some((p) => p.account_id === id || p.entity_id === id || p.item_id === id) ||
          s.txn.lines.some((l) => l.account_id === id || l.item_id === id || l.entity_id === id),
      );
    if (used) {
      return Promise.reject(new ApiError(409, 'in_use', 'It is used in transactions or has subitems; make it inactive instead.'));
    }
    rows.splice(i, 1);
    this.emit(kindOfList(list), id, 'deleted');
    return this.delay(() => undefined);
  }

  // ---- transactions ---------------------------------------------------------

  /** Cash or credit applied to each open transaction. */
  private appliedTo(): Map<string, number> {
    const m = new Map<string, number>();
    for (const s of this.txnStore.values()) {
      if (s.txn.is_void) continue;
      for (const a of s.txn.applications) {
        m.set(a.target_txn_id, (m.get(a.target_txn_id) ?? 0) + a.amount + a.discount + a.credit_amount);
        // A credit used against the target is used up by that much.
        if (a.credit_txn_id && a.credit_amount) m.set(a.credit_txn_id, (m.get(a.credit_txn_id) ?? 0) + a.credit_amount);
      }
    }
    return m;
  }

  private openBalance(s: StoredTxn, applied: Map<string, number>): number | null {
    if (s.txn.is_void) return 0;
    // A payment's open balance is what it hasn't applied yet (an unapplied credit).
    if (PAYMENT_TYPES.includes(s.txn.txn_type)) return s.txn.amount - s.txn.applications.reduce((t, a) => t + a.amount + a.discount, 0);
    if (![...AR_TYPES, ...AP_TYPES].includes(s.txn.txn_type)) return null;
    return s.total - (applied.get(s.txn.id) ?? 0);
  }

  private summaryOf(s: StoredTxn, applied: Map<string, number>): TxnSummary {
    const t = s.txn;
    const names = new Map(this.lists.entities.map((e) => [e.id, e.full_name]));
    const accounts = new Map(this.lists.accounts.map((a) => [a.id, a.full_name]));
    const roleId = AR_TYPES.includes(t.txn_type)
      ? this.lists.accounts.find((a) => a.role === 'AccountsReceivable')?.id
      : AP_TYPES.includes(t.txn_type)
        ? this.lists.accounts.find((a) => a.role === 'AccountsPayable')?.id
        : undefined;
    const account_id = t.account_id ?? t.ar_ap_account_id ?? roleId ?? null;
    return {
      id: t.id,
      txn_type: t.txn_type,
      date: t.date,
      number: t.number,
      entity_id: t.entity_id,
      entity_name: t.entity_id ? (names.get(t.entity_id) ?? null) : null,
      account_id,
      account_name: account_id ? (accounts.get(account_id) ?? null) : null,
      amount: s.total,
      open_balance: this.openBalance(s, applied),
      due_date: t.due_date,
      memo: t.memo,
      is_void: t.is_void,
      is_pending: t.is_pending,
      edit_seq: t.edit_seq,
    };
  }

  /** The name and, with `include_jobs`, every job under it. */
  private entityFamily(id: string): Set<string> {
    const out = new Set([id]);
    let grew = true;
    while (grew) {
      grew = false;
      for (const e of this.lists.entities) {
        if (e.parent_id && out.has(e.parent_id) && !out.has(e.id)) {
          out.add(e.id);
          grew = true;
        }
      }
    }
    return out;
  }

  private filterTxns(q: TxnQuery): StoredTxn[] {
    const applied = this.appliedTo();
    const family = q.entity_id ? (q.include_jobs ? this.entityFamily(q.entity_id) : new Set([q.entity_id])) : null;
    const names = new Map(this.lists.entities.map((e) => [e.id, e.full_name.toLowerCase()]));
    const text = q.text?.toLowerCase();
    let out = [...this.txnStore.values()].filter((s) => {
      const t = s.txn;
      if (q.types?.length && !q.types.includes(t.txn_type)) return false;
      if (q.from && t.date < q.from) return false;
      if (q.to && t.date > q.to) return false;
      if (family && !(t.entity_id && family.has(t.entity_id)) && !t.lines.some((l) => l.entity_id && family.has(l.entity_id))) return false;
      if (q.account_id && !s.postings.some((p) => p.account_id === q.account_id) && t.account_id !== q.account_id) return false;
      if (q.number && t.number !== q.number) return false;
      if (q.amount != null && Math.abs(s.total) !== Math.abs(q.amount)) return false;
      if (q.open_only && !((this.openBalance(s, applied) ?? 0) > 0)) return false;
      if (!q.include_void && t.is_void) return false;
      if (text) {
        const hay = [t.memo, t.number, t.entity_id ? names.get(t.entity_id) : null, ...t.lines.map((l) => l.description)]
          .filter(Boolean)
          .join(' ')
          .toLowerCase();
        if (!hay.includes(text)) return false;
      }
      return true;
    });
    out.sort((a, b) => (a.txn.date < b.txn.date ? -1 : a.txn.date > b.txn.date ? 1 : a.seq - b.seq));
    if (q.descending) out.reverse();
    const offset = q.offset ?? 0;
    out = out.slice(offset, q.limit != null ? offset + q.limit : undefined);
    return out;
  }

  txns(co: CompanyId, query: TxnQuery = {}): Promise<TxnSummary[]> {
    this.checkCompany(co);
    const applied = this.appliedTo();
    return this.delay(() => this.filterTxns(query).map((s) => this.summaryOf(s, applied)));
  }

  txn(co: CompanyId, id: string): Promise<TxnWithPostings> {
    this.checkCompany(co);
    const s = this.txnStore.get(id);
    if (!s) return Promise.reject(new ApiError(404, 'not_found', 'That transaction no longer exists.'));
    return this.delay(() => ({ txn: s.txn, postings: s.postings }));
  }

  private validate(txn: Txn) {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(txn.date)) throw new ApiError(400, 'invalid', 'Enter a valid date', { field: 'date' });
    for (const [i, l] of txn.lines.entries()) {
      if (!Number.isSafeInteger(l.amount)) throw new ApiError(400, 'invalid', 'Amounts must be whole cents', { field: `lines[${i}].amount` });
    }
  }

  private record(action: string, before: Txn | null, after: Txn | null) {
    const t = (after ?? before)!;
    this.audit.push({
      id: String(this.audit.length + 1),
      at: new Date().toISOString(),
      user: 'Demo User',
      action,
      record_kind: 'txn',
      record_id: t.id,
      before,
      after,
    });
  }

  createTxn(co: CompanyId, txn: Txn): Promise<SaveOutcome> {
    this.checkCompany(co);
    try {
      this.validate(txn);
      if (this.txnStore.has(txn.id)) throw new ApiError(409, 'conflict', 'That transaction already exists', { expected: 0, actual: 1 });
      const posted = this.post(txn);
      const now = new Date().toISOString();
      const saved: Txn = { ...clone(txn), amount: posted.total, edit_seq: 1, created_at: now, modified_at: now, created_by: 'fixture-user', modified_by: 'fixture-user' };
      this.txnStore.set(saved.id, { txn: saved, postings: posted.postings, total: posted.total, seq: this.seq++ });
      this.record('created', null, saved);
      this.emit('txn', saved.id, 'created');
      return this.delay(() => ({ txn: saved, postings: posted.postings, total: posted.total, sales_tax: posted.sales_tax, warnings: [] }));
    } catch (e) {
      return Promise.reject(e);
    }
  }

  updateTxn(co: CompanyId, txn: Txn): Promise<SaveOutcome> {
    this.checkCompany(co);
    const current = this.txnStore.get(txn.id);
    if (!current) return Promise.reject(new ApiError(404, 'not_found', 'That transaction no longer exists.'));
    if (current.txn.edit_seq !== txn.edit_seq) {
      return Promise.reject(
        new ApiError(409, 'conflict', 'Someone else changed this transaction', { expected: Number(txn.edit_seq), actual: Number(current.txn.edit_seq) }),
      );
    }
    try {
      this.validate(txn);
      const posted = this.post(txn);
      // Keep cleared marks on the accounts that still have a posting.
      const cleared = new Map(current.postings.map((p) => [p.account_id, p.cleared]));
      const postings = posted.postings.map((p) => ({ ...p, cleared: cleared.get(p.account_id) ?? p.cleared }));
      const saved: Txn = { ...clone(txn), amount: posted.total, edit_seq: current.txn.edit_seq + 1, modified_at: new Date().toISOString(), modified_by: 'fixture-user' };
      this.txnStore.set(saved.id, { txn: saved, postings, total: posted.total, seq: current.seq });
      this.record('updated', current.txn, saved);
      this.emit('txn', saved.id, 'updated');
      return this.delay(() => ({ txn: saved, postings, total: posted.total, sales_tax: posted.sales_tax, warnings: [] }));
    } catch (e) {
      return Promise.reject(e);
    }
  }

  previewTxn(co: CompanyId, txn: Txn): Promise<PreviewOutcome> {
    this.checkCompany(co);
    try {
      const posted = this.post(txn);
      return this.delay(() => ({ postings: posted.postings, total: posted.total, sales_tax: posted.sales_tax, warnings: [] }));
    } catch (e) {
      return Promise.reject(e);
    }
  }

  voidTxn(co: CompanyId, id: string, editSeq: number): Promise<SaveOutcome> {
    const s = this.txnStore.get(id);
    if (!s) return Promise.reject(new ApiError(404, 'not_found', 'Not found'));
    return this.updateTxn(co, { ...clone(s.txn), edit_seq: editSeq, is_void: true, memo: `VOID: ${s.txn.memo ?? ''}`.trim() });
  }

  deleteTxn(co: CompanyId, id: string, editSeq: number): Promise<void> {
    this.checkCompany(co);
    const s = this.txnStore.get(id);
    if (!s) return Promise.reject(new ApiError(404, 'not_found', 'Not found'));
    if (s.txn.edit_seq !== editSeq) {
      return Promise.reject(new ApiError(409, 'conflict', 'Someone else changed this transaction', { expected: Number(editSeq), actual: Number(s.txn.edit_seq) }));
    }
    if ([...this.txnStore.values()].some((o) => o.txn.applications.some((a) => a.target_txn_id === id) || o.txn.lines.some((l) => l.linked_txn_id === id))) {
      return Promise.reject(new ApiError(409, 'in_use', 'Payments are applied to it; delete those first.'));
    }
    this.txnStore.delete(id);
    this.record('deleted', s.txn, null);
    this.emit('txn', id, 'deleted');
    return this.delay(() => undefined);
  }

  nextNumber(co: CompanyId, type: TxnType, accountId?: string | null): Promise<string | null> {
    this.checkCompany(co);
    return this.delay(() => {
      let max = 0;
      for (const s of this.txnStore.values()) {
        const t = s.txn;
        const sameSeries =
          type === 'Check' || type === 'BillPaymentCheck'
            ? (t.txn_type === 'Check' || t.txn_type === 'BillPaymentCheck') && (!accountId || t.account_id === accountId)
            : t.txn_type === type;
        if (sameSeries && t.number && /^\d+$/.test(t.number)) max = Math.max(max, Number(t.number));
      }
      return max ? String(max + 1) : type === 'Check' ? '1001' : null;
    });
  }

  history(co: CompanyId, id: string): Promise<AuditEntry[]> {
    this.checkCompany(co);
    return this.delay(() => this.audit.filter((a) => a.record_id === id));
  }

  // ---- registers and balances ----------------------------------------------

  register(co: CompanyId, accountId: string, range: Partial<DateRange> = {}): Promise<RegisterResponse> {
    this.checkCompany(co);
    if (accountId === STRESS_ACCOUNT_ID) return this.delay(() => ({ opening: 0, lines: this.stressRegister() }));
    if (!this.lists.accounts.some((a) => a.id === accountId)) return Promise.reject(new ApiError(404, 'not_found', 'No such account'));
    return this.delay(() => this.buildRegister(accountId, range));
  }

  private buildRegister(accountId: string, range: Partial<DateRange>): RegisterResponse {
    const applied = this.appliedTo();
    const names = new Map(this.lists.entities.map((e) => [e.id, e.full_name]));
    const accounts = new Map(this.lists.accounts.map((a) => [a.id, a.full_name]));
    const rows = [...this.txnStore.values()]
      .filter((s) => s.postings.some((p) => p.account_id === accountId) || (s.txn.is_void && s.txn.account_id === accountId))
      .sort((a, b) => (a.txn.date < b.txn.date ? -1 : a.txn.date > b.txn.date ? 1 : a.seq - b.seq));
    let opening = 0;
    let balance = 0;
    const lines: RegisterLine[] = [];
    for (const s of rows) {
      const mine = s.postings.filter((p) => p.account_id === accountId);
      const amount = mine.reduce((sum, p) => sum + p.amount, 0);
      if (range.from && s.txn.date < range.from) {
        opening += amount;
        balance += amount;
        continue;
      }
      if (range.to && s.txn.date > range.to) continue;
      balance += amount;
      const others = [...new Set(s.postings.filter((p) => p.account_id !== accountId).map((p) => p.account_id))];
      const cleared: ClearedStatus = mine.length && mine.every((p) => p.cleared === 'Cleared') ? 'Cleared' : mine.some((p) => p.cleared === 'Pending') ? 'Pending' : 'Uncleared';
      lines.push({
        txn_id: s.txn.id,
        txn_type: s.txn.txn_type,
        date: s.txn.date,
        number: s.txn.number,
        payee_id: s.txn.entity_id,
        payee: s.txn.entity_id ? (names.get(s.txn.entity_id) ?? null) : null,
        other_account_id: others.length === 1 ? others[0]! : null,
        other_account: others.length === 1 ? (accounts.get(others[0]!) ?? null) : others.length > 1 ? '-split-' : null,
        is_split: others.length > 1,
        memo: s.txn.memo ?? mine[0]?.memo ?? null,
        amount,
        cleared,
        balance,
        is_void: s.txn.is_void,
        edit_seq: s.txn.edit_seq,
        due_date: s.txn.due_date,
        open_balance: this.openBalance(s, applied),
        // QBD's lightning bolt: a bank feed matched or added this line (not yet in the generated type).
        ...(this.downloaded.get(s.txn.id)?.has(accountId) ? { downloaded: true } : {}),
      } as RegisterLine);
    }
    return { opening, lines };
  }

  /** 100k synthetic lines, built once, for scrolling performance. */
  private stressRegister(): RegisterLine[] {
    if (this.stressLines) return this.stressLines;
    const payees = this.lists.entities.filter((e) => e.kind === 'Vendor');
    const expenseAccounts = this.lists.accounts.filter((a) => a.account_type === 'Expense');
    let balance = 0;
    const start = addDays(this.today, -3650);
    this.stressLines = Array.from({ length: 100_000 }, (_, i) => {
      const deposit = i % 7 === 0;
      const amount = deposit ? 250_000 + (i % 13) * 1_000 : -((i * 7919) % 90_000) - 500;
      balance += amount;
      const payee = payees[i % payees.length]!;
      const other = expenseAccounts[i % expenseAccounts.length]!;
      return {
        txn_id: `stress-${i}`,
        txn_type: deposit ? 'Deposit' : 'Check',
        date: addDays(start, Math.floor((i * 3650) / 100_000)),
        number: deposit ? null : String(10_000 + i),
        payee_id: payee.id,
        payee: payee.full_name,
        other_account_id: other.id,
        other_account: other.full_name,
        is_split: false,
        memo: deposit ? 'Deposit' : null,
        amount,
        cleared: i < 99_000 ? 'Cleared' : 'Uncleared',
        balance,
        is_void: false,
        edit_seq: 1,
        due_date: null,
        open_balance: null,
      } satisfies RegisterLine;
    });
    return this.stressLines;
  }

  setCleared(co: CompanyId, accountId: string, txnId: string, status: ClearedStatus): Promise<void> {
    this.checkCompany(co);
    const s = this.txnStore.get(txnId);
    if (!s) return Promise.reject(new ApiError(404, 'not_found', 'Not found'));
    s.postings = s.postings.map((p) => (p.account_id === accountId ? { ...p, cleared: status } : p));
    this.emit('txn', txnId, 'cleared');
    return this.delay(() => undefined);
  }

  balances(co: CompanyId, range: Partial<DateRange> = {}): Promise<AccountBalance[]> {
    this.checkCompany(co);
    return this.delay(() => {
      const sums = new Map<string, number>();
      for (const s of this.txnStore.values()) {
        if (range.from && s.txn.date < range.from) continue;
        if (range.to && s.txn.date > range.to) continue;
        for (const p of s.postings) sums.set(p.account_id, (sums.get(p.account_id) ?? 0) + p.amount);
      }
      if (this.stressLines || this.lists.accounts.some((a) => a.id === STRESS_ACCOUNT_ID)) {
        const lines = this.stressRegister();
        sums.set(STRESS_ACCOUNT_ID, lines[lines.length - 1]?.balance ?? 0);
      }
      return [...sums].map(([account_id, debit_positive]) => ({ account_id, debit_positive }));
    });
  }

  /**
   * Open items for a name and its jobs, as `TxnSummary` rows whose
   * `open_balance` is what is left: invoices/bills still owed, and credits and
   * unapplied payments still unused (so a payment form can apply them).
   */
  open(co: CompanyId, entityId: string, kind: 'receivable' | 'payable'): Promise<TxnSummary[]> {
    this.checkCompany(co);
    const applied = this.appliedTo();
    const types: TxnType[] = kind === 'receivable' ? [...AR_TYPES, 'ReceivePayment'] : [...AP_TYPES, 'BillPaymentCheck', 'BillPaymentCreditCard'];
    const family = this.entityFamily(entityId);
    return this.delay(() =>
      [...this.txnStore.values()]
        .filter((s) => types.includes(s.txn.txn_type) && s.txn.entity_id && family.has(s.txn.entity_id) && !s.txn.is_void)
        .sort((a, b) => (a.txn.date < b.txn.date ? -1 : a.txn.date > b.txn.date ? 1 : a.seq - b.seq))
        .map((s) => ({ s, open: this.openBalance(s, applied) ?? 0 }))
        .filter(({ open }) => open > 0)
        .map(({ s, open }) => ({ ...this.summaryOf(s, applied), open_balance: open })),
    );
  }

  /** Payments (and sales receipts) sitting in Undeposited Funds, not yet on a deposit. */
  undeposited(co: CompanyId): Promise<TxnSummary[]> {
    this.checkCompany(co);
    const applied = this.appliedTo();
    const uf = this.lists.accounts.find((a) => a.role === 'UndepositedFunds');
    if (!uf) return this.delay(() => []);
    const deposited = new Set([...this.txnStore.values()].flatMap((s) => (s.txn.txn_type === 'Deposit' && !s.txn.is_void ? s.txn.lines.map((l) => l.linked_txn_id).filter(Boolean) : [])));
    return this.delay(() =>
      [...this.txnStore.values()]
        .filter((s) => !s.txn.is_void && s.txn.txn_type !== 'Deposit' && !deposited.has(s.txn.id) && s.postings.some((p) => p.account_id === uf.id && p.amount > 0))
        .sort((a, b) => (a.txn.date < b.txn.date ? -1 : a.txn.date > b.txn.date ? 1 : a.seq - b.seq))
        .map((s) => ({ ...this.summaryOf(s, applied), account_id: uf.id, account_name: uf.full_name, amount: s.postings.filter((p) => p.account_id === uf.id).reduce((t, p) => t + p.amount, 0) })),
    );
  }

  // ---- reports --------------------------------------------------------------

  reportCatalog(co: CompanyId): Promise<CatalogEntry[]> {
    this.checkCompany(co);
    return this.delay(() => fixtureCatalog());
  }

  private buildReport(request: ReportRequest): Report {
    return runFixtureReport(
      {
        info: this.info,
        accounts: this.sorted('accounts'),
        entities: this.lists.entities,
        items: this.lists.items,
        classes: this.lists.classes,
        txns: [...this.txnStore.values()],
        audit: this.audit,
        today: this.today,
      },
      request,
    );
  }

  runReport(co: CompanyId, request: ReportRequest): Promise<Report> {
    this.checkCompany(co);
    try {
      const report = this.buildReport(request);
      return this.delay(() => report);
    } catch (e) {
      return Promise.reject(e);
    }
  }

  exportReport(co: CompanyId, request: ReportRequest, format: 'csv'): Promise<FileDownload> {
    this.checkCompany(co);
    try {
      const report = this.buildReport(request);
      const name = `${report.title.replace(/[^\w -]+/g, '').trim() || 'report'}.${format}`;
      return this.delay(() => ({ blob: new Blob([reportToCsv(report)], { type: 'text/csv;charset=utf-8' }), filename: name }));
    } catch (e) {
      return Promise.reject(e);
    }
  }

  // ---- saved documents ------------------------------------------------------

  saved(co: CompanyId, kind: SavedKind): Promise<Saved[]> {
    this.checkCompany(co);
    return this.delay(() => this.savedDocs.filter((s) => s.kind === kind).sort((a, b) => a.name.localeCompare(b.name)));
  }

  createSaved(co: CompanyId, kind: SavedKind, name: string, doc: unknown): Promise<Saved> {
    this.checkCompany(co);
    if (!name.trim()) return Promise.reject(new ApiError(400, 'invalid', 'A name is required', { field: 'name' }));
    const now = new Date().toISOString();
    const s: Saved = { id: crypto.randomUUID(), kind, name: name.trim(), doc: clone(doc), created_at: now, modified_at: now };
    this.savedDocs.push(s);
    this.emit('saved', s.id, 'created');
    return this.delay(() => s);
  }

  updateSaved(co: CompanyId, kind: SavedKind, id: string, name: string, doc: unknown): Promise<Saved> {
    this.checkCompany(co);
    const i = this.savedDocs.findIndex((s) => s.id === id && s.kind === kind);
    if (i < 0) return Promise.reject(new ApiError(404, 'not_found', 'Not found'));
    const s: Saved = { ...this.savedDocs[i]!, name: name.trim(), doc: clone(doc), modified_at: new Date().toISOString() };
    this.savedDocs[i] = s;
    this.emit('saved', s.id, 'updated');
    return this.delay(() => s);
  }

  deleteSaved(co: CompanyId, kind: SavedKind, id: string): Promise<void> {
    this.checkCompany(co);
    const i = this.savedDocs.findIndex((s) => s.id === id && s.kind === kind);
    if (i < 0) return Promise.reject(new ApiError(404, 'not_found', 'Not found'));
    this.savedDocs.splice(i, 1);
    this.emit('saved', id, 'deleted');
    return this.delay(() => undefined);
  }

  memorizedDue(co: CompanyId): Promise<MemorizedDue[]> {
    this.checkCompany(co);
    return this.delay(() =>
      this.savedDocs
        .filter((s) => s.kind === 'memorized_txn')
        .flatMap((s) => {
          const m = s.doc as Memorized;
          if (m.repeat === 'Never' || m.remaining === 0 || !m.next_date) return [];
          const horizon = addDays(this.today, m.days_in_advance);
          return m.next_date <= horizon ? [{ id: s.id, name: s.name, due: m.next_date, repeat: m.repeat }] : [];
        })
        .sort((a, b) => a.due.localeCompare(b.due)),
    );
  }

  useMemorized(co: CompanyId, id: string, date?: string | null): Promise<Txn> {
    this.checkCompany(co);
    const s = this.savedDocs.find((d) => d.id === id && d.kind === 'memorized_txn');
    if (!s) return Promise.reject(new ApiError(404, 'not_found', 'No such memorized transaction'));
    const m = s.doc as Memorized;
    const t: Txn = {
      ...clone(m.template),
      id: crypto.randomUUID(),
      date: date ?? this.today,
      number: null,
      edit_seq: 0,
      created_at: null,
      modified_at: null,
      created_by: null,
      modified_by: null,
      is_void: false,
      due_date: null,
      discount_date: null,
    };
    t.lines = t.lines.map((l) => ({ ...l, id: crypto.randomUUID() }));
    return this.delay(() => t);
  }

  /** The store advances a memorized transaction's schedule after it's entered. */
  advanceMemorized(id: string) {
    const s = this.savedDocs.find((d) => d.id === id && d.kind === 'memorized_txn');
    if (!s) return;
    const m = s.doc as Memorized;
    if (!m.next_date || !m.interval) return;
    const months: Partial<Record<NonNullable<Memorized['interval']>, number>> = { Monthly: 1, Quarterly: 3, SemiAnnually: 6, Annually: 12 };
    const days: Partial<Record<NonNullable<Memorized['interval']>, number>> = { Daily: 1, Weekly: 7, BiWeekly: 14, SemiMonthly: 15 };
    const next = months[m.interval] ? addMonths(m.next_date, months[m.interval]!) : addDays(m.next_date, days[m.interval] ?? 30);
    s.doc = { ...m, next_date: next, remaining: m.remaining === null ? null : Math.max(0, m.remaining - 1) };
    s.modified_at = new Date().toISOString();
    this.emit('saved', s.id, 'updated');
  }

  // ---- reconciliation --------------------------------------------------------
  //
  // While a reconciliation is in progress its marked lines are Pending in the
  // account; finishing makes them Cleared and records them on it; undoing the
  // last one makes them Uncleared again.

  private reconcileStatus(rec: Reconciliation): ReconcileStatus {
    const lines = this.reconcileLines(rec);
    let inc = 0;
    let dec = 0;
    let count = 0;
    for (const l of lines) {
      if (!l.marked) continue;
      count++;
      if (l.amount < 0) dec += l.amount;
      else inc += l.amount;
    }
    const cleared = rec.beginning_balance + inc + dec;
    return { beginning_balance: rec.beginning_balance, marked_increases: inc, marked_decreases: dec, cleared_balance: cleared, ending_balance: rec.ending_balance, difference: rec.ending_balance - cleared, marked_count: count };
  }

  private reconcileLines(rec: Reconciliation): ReconcileLine[] {
    const type = this.lists.accounts.find((a) => a.id === rec.account_id)?.account_type ?? 'Bank';
    const names = new Map(this.lists.entities.map((e) => [e.id, e.full_name]));
    return [...this.txnStore.values()]
      .filter((s) => !s.txn.is_void && s.postings.some((p) => p.account_id === rec.account_id && p.cleared !== 'Cleared' && (s.txn.date <= rec.statement_date || p.cleared === 'Pending')))
      .sort((a, b) => (a.txn.date < b.txn.date ? -1 : a.txn.date > b.txn.date ? 1 : a.seq - b.seq))
      .map((s) => {
        const mine = s.postings.filter((p) => p.account_id === rec.account_id);
        const amount = mine.reduce((t, p) => t + p.amount, 0);
        return {
          txn_id: s.txn.id,
          txn_type: s.txn.txn_type,
          date: s.txn.date,
          number: s.txn.number,
          payee: s.txn.entity_id ? (names.get(s.txn.entity_id) ?? null) : null,
          memo: s.txn.memo,
          amount: naturalBalance(type, amount),
          marked: mine.some((p) => p.cleared === 'Pending'),
        };
      })
      .filter((l) => l.amount !== 0);
  }

  private beginningBalance(accountId: string): number {
    const type = this.lists.accounts.find((a) => a.id === accountId)?.account_type ?? 'Bank';
    let sum = 0;
    for (const s of this.txnStore.values()) for (const p of s.postings) if (p.account_id === accountId && p.cleared === 'Cleared') sum += p.amount;
    return naturalBalance(type, sum);
  }

  reconcileOverview(co: CompanyId, accountId: string): Promise<ReconcileOverview> {
    this.checkCompany(co);
    if (!this.lists.accounts.some((a) => a.id === accountId)) return Promise.reject(new ApiError(404, 'not_found', 'No such account'));
    return this.delay(() => ({
      in_progress: this.reconciliations.find((r) => r.account_id === accountId && !r.finished_at) ?? null,
      history: this.reconciliations.filter((r) => r.account_id === accountId && r.finished_at).sort((a, b) => b.statement_date.localeCompare(a.statement_date)),
      beginning_balance: this.beginningBalance(accountId),
    }));
  }

  startReconcile(co: CompanyId, accountId: string, body: StartReconcile): Promise<Reconciliation> {
    this.checkCompany(co);
    if (this.reconciliations.some((r) => r.account_id === accountId && !r.finished_at)) return Promise.reject(new ApiError(409, 'conflict', 'A reconciliation of this account is already in progress'));
    const charge = (c: StatementCharge | null, memo: string, sign: 1 | -1): string | null => {
      if (!c || !c.amount) return null;
      const t = blankTxn(sign < 0 ? 'Check' : 'Deposit', c.date);
      t.account_id = accountId;
      t.memo = memo;
      t.amount = c.amount;
      t.lines = [{ ...blankLine(sign < 0 ? 'Account' : 'DepositLine'), account_id: c.account_id, amount: c.amount, description: memo }];
      const posted = this.post(t);
      const saved: Txn = { ...t, amount: posted.total, edit_seq: 1, created_at: new Date().toISOString(), modified_at: new Date().toISOString() };
      this.txnStore.set(saved.id, { txn: saved, postings: posted.postings, total: posted.total, seq: this.seq++ });
      this.emit('txn', saved.id, 'created');
      return saved.id;
    };
    const rec: Reconciliation = {
      id: crypto.randomUUID(),
      account_id: accountId,
      statement_date: body.statement_date,
      beginning_balance: this.beginningBalance(accountId),
      ending_balance: body.ending_balance,
      service_charge_txn_id: charge(body.service_charge, 'Service Charge', -1),
      interest_txn_id: charge(body.interest, 'Interest Earned', 1),
      adjustment_txn_id: null,
      finished_at: null,
      finished_by: null,
      cleared_txn_ids: [],
    };
    this.reconciliations.push(rec);
    this.emit('reconciliation', rec.id, 'created');
    return this.delay(() => rec);
  }

  reconciliation(co: CompanyId, id: string): Promise<ReconciliationView> {
    this.checkCompany(co);
    const rec = this.reconciliations.find((r) => r.id === id);
    if (!rec) return Promise.reject(new ApiError(404, 'not_found', 'No such reconciliation'));
    return this.delay(() => ({ reconciliation: rec, lines: this.reconcileLines(rec), status: this.reconcileStatus(rec) }));
  }

  markReconcile(co: CompanyId, id: string, txnIds: string[], marked: boolean): Promise<ReconcileStatus> {
    this.checkCompany(co);
    const rec = this.reconciliations.find((r) => r.id === id && !r.finished_at);
    if (!rec) return Promise.reject(new ApiError(404, 'not_found', 'No reconciliation in progress'));
    for (const txnId of txnIds) {
      const s = this.txnStore.get(txnId);
      if (!s) continue;
      s.postings = s.postings.map((p) => (p.account_id === rec.account_id && p.cleared !== 'Cleared' ? { ...p, cleared: marked ? 'Pending' : 'Uncleared' } : p));
    }
    this.emit('reconciliation', id, 'updated');
    return this.delay(() => this.reconcileStatus(rec));
  }

  finishReconcile(co: CompanyId, id: string, adjust: boolean): Promise<Reconciliation> {
    this.checkCompany(co);
    const rec = this.reconciliations.find((r) => r.id === id && !r.finished_at);
    if (!rec) return Promise.reject(new ApiError(404, 'not_found', 'No reconciliation in progress'));
    const status = this.reconcileStatus(rec);
    if (status.difference !== 0 && !adjust) return Promise.reject(new ApiError(400, 'invalid', `The difference is ${(status.difference / 100).toFixed(2)}; mark more lines or enter an adjustment.`));
    if (status.difference !== 0) {
      const disc = this.lists.accounts.find((a) => a.role === 'ReconciliationDiscrepancies') ?? this.lists.accounts.find((a) => a.full_name === 'Ask My Accountant')!;
      const t = blankTxn('JournalEntry', rec.statement_date);
      const type = this.lists.accounts.find((a) => a.id === rec.account_id)?.account_type ?? 'Bank';
      const here = naturalBalance(type, status.difference); // debit-positive effect that brings the account to the statement
      t.memo = 'Balance Adjustment';
      t.amount = Math.abs(here);
      t.lines = [
        { ...blankLine('Journal'), account_id: rec.account_id, amount: here, description: t.memo },
        { ...blankLine('Journal'), account_id: disc.id, amount: -here, description: t.memo },
      ];
      const posted = this.post(t);
      const saved: Txn = { ...t, edit_seq: 1, created_at: new Date().toISOString(), modified_at: new Date().toISOString() };
      this.txnStore.set(saved.id, { txn: saved, postings: posted.postings.map((p) => (p.account_id === rec.account_id ? { ...p, cleared: 'Pending' as const } : p)), total: posted.total, seq: this.seq++ });
      this.emit('txn', saved.id, 'created');
      rec.adjustment_txn_id = saved.id;
    }
    const cleared: string[] = [];
    for (const s of this.txnStore.values()) {
      if (s.postings.some((p) => p.account_id === rec.account_id && p.cleared === 'Pending')) {
        s.postings = s.postings.map((p) => (p.account_id === rec.account_id && p.cleared === 'Pending' ? { ...p, cleared: 'Cleared' } : p));
        cleared.push(s.txn.id);
      }
    }
    rec.cleared_txn_ids = cleared;
    rec.finished_at = new Date().toISOString();
    rec.finished_by = 'Demo User';
    this.emit('reconciliation', id, 'finished');
    return this.delay(() => rec);
  }

  discardReconcile(co: CompanyId, id: string): Promise<void> {
    this.checkCompany(co);
    const i = this.reconciliations.findIndex((r) => r.id === id && !r.finished_at);
    if (i < 0) return Promise.reject(new ApiError(404, 'not_found', 'No reconciliation in progress'));
    const rec = this.reconciliations[i]!;
    for (const s of this.txnStore.values()) s.postings = s.postings.map((p) => (p.account_id === rec.account_id && p.cleared === 'Pending' ? { ...p, cleared: 'Uncleared' } : p));
    this.reconciliations.splice(i, 1);
    this.emit('reconciliation', id, 'deleted');
    return this.delay(() => undefined);
  }

  undoReconcile(co: CompanyId, accountId: string): Promise<Reconciliation | null> {
    this.checkCompany(co);
    if (this.reconciliations.some((r) => r.account_id === accountId && !r.finished_at)) return Promise.reject(new ApiError(400, 'invalid', 'Finish or discard the reconciliation in progress first'));
    const done = this.reconciliations.filter((r) => r.account_id === accountId && r.finished_at).sort((a, b) => b.statement_date.localeCompare(a.statement_date));
    const last = done[0];
    if (!last) return this.delay(() => null);
    for (const id of last.cleared_txn_ids) {
      const s = this.txnStore.get(id);
      if (s) s.postings = s.postings.map((p) => (p.account_id === accountId && p.cleared === 'Cleared' ? { ...p, cleared: 'Uncleared' } : p));
    }
    this.reconciliations = this.reconciliations.filter((r) => r.id !== last.id);
    this.emit('reconciliation', last.id, 'undone');
    return this.delay(() => last);
  }

  // ---- bank feeds (FIXTURE: ./feeds.ts) --------------------------------------

  private feed<T>(co: CompanyId, f: () => T): Promise<T> {
    this.checkCompany(co);
    try {
      const v = f();
      return this.delay(() => v);
    } catch (e) {
      return Promise.reject(e);
    }
  }
  feedsSummary(co: CompanyId) {
    return this.feed(co, () => this.feeds.summary());
  }
  feedTxns(co: CompanyId, query?: FeedQuery) {
    return this.feed(co, () => this.feeds.list(query));
  }
  feedTxn(co: CompanyId, id: string) {
    return this.feed(co, () => this.feeds.get(id));
  }
  async feedDetect(co: CompanyId, file: File) {
    const text = await file.text();
    return this.feed(co, () => this.feeds.detect(file.name, text));
  }
  async feedImport(co: CompanyId, req: FeedImportRequest) {
    const text = await req.file.text();
    return this.feed(co, () => this.feeds.import(req, req.file.name, text));
  }
  feedCandidates(co: CompanyId, id: string, days?: number) {
    return this.feed(co, () => this.feeds.candidates(id, days));
  }
  feedMatch(co: CompanyId, id: string, txnId: string) {
    return this.feed(co, () => this.feeds.match(id, txnId));
  }
  feedPreview(co: CompanyId, id: string, req: AddRequest) {
    return this.feed(co, () => {
      const p = this.feeds.preview(id, req);
      const posted = this.post(p.txn);
      return { ...p, postings: posted.postings, total: posted.total, sales_tax: posted.sales_tax };
    });
  }
  feedAdd(co: CompanyId, id: string, req: AddRequest) {
    return this.feed(co, () => this.feeds.add(id, req));
  }
  feedAddAll(co: CompanyId, ids: string[], req?: AddRequest) {
    return this.feed(co, () => this.feeds.addAll(ids, req));
  }
  feedIgnore(co: CompanyId, ids: string[]) {
    return this.feed(co, () => this.feeds.ignore(ids));
  }
  feedUndo(co: CompanyId, id: string) {
    return this.feed(co, () => this.feeds.undo(id));
  }
  feedConnections(co: CompanyId) {
    return this.feed(co, () => this.feeds.connections);
  }
  createFeedConnection(co: CompanyId, c: FeedConnection) {
    return this.feed(co, () => this.feeds.createConnection(c));
  }
  updateFeedConnection(co: CompanyId, c: FeedConnection) {
    return this.feed(co, () => this.feeds.updateConnection(c));
  }
  deleteFeedConnection(co: CompanyId, id: string, editSeq: number) {
    return this.feed(co, () => this.feeds.deleteConnection(id, editSeq));
  }
  setFeedCredentials(co: CompanyId, id: string, value: string) {
    return this.feed(co, () => this.feeds.setCredentials(id, value));
  }
  syncFeedConnection(co: CompanyId, id: string) {
    return this.feed(co, () => this.feeds.sync(id));
  }
  feedRemoteAccounts(co: CompanyId, id: string) {
    return this.feed(co, () => this.feeds.remoteAccounts(id));
  }
  simplefinClaim(co: CompanyId, setupToken: string) {
    return this.feed(co, () => this.feeds.simplefinClaim(setupToken));
  }
  feedRules(co: CompanyId) {
    return this.feed(co, () => [...this.feeds.rules].sort((a, b) => Number(a.priority) - Number(b.priority)));
  }
  createFeedRule(co: CompanyId, r: FeedRule) {
    return this.feed(co, () => this.feeds.createRule(r));
  }
  updateFeedRule(co: CompanyId, r: FeedRule) {
    return this.feed(co, () => this.feeds.updateRule(r));
  }
  deleteFeedRule(co: CompanyId, id: string) {
    return this.feed(co, () => this.feeds.deleteRule(id));
  }
  reorderFeedRules(co: CompanyId, ids: string[]) {
    return this.feed(co, () => this.feeds.reorderRules(ids));
  }
  applyFeedRules(co: CompanyId, ids?: string[], accountId?: string | null) {
    return this.feed(co, () => this.feeds.applyRules(ids, accountId));
  }
  async amazonOrders(co: CompanyId, file: File, accountId?: string | null) {
    const text = await file.text();
    return this.feed(co, () => this.feeds.amazonOrders(text, accountId));
  }
  amazonApply(co: CompanyId, preview: AmazonPreview) {
    return this.feed(co, () => this.feeds.amazonApply(preview));
  }

  // ---- events ---------------------------------------------------------------

  subscribe(co: CompanyId, handlers: EventHandlers): EventSubscription {
    this.checkCompany(co);
    this.listeners.add(handlers);
    handlers.onStatus?.('connecting');
    setTimeout(() => handlers.onStatus?.('open'), 50);
    return { close: () => this.listeners.delete(handlers) };
  }

  // ---- dev helpers (window.__librebooks) -------------------------------------

  /** Pretend another user saved this transaction: bumps edit_seq and emits. */
  simulateRemoteEdit(txnId: string): boolean {
    const s = this.txnStore.get(txnId);
    if (!s) return false;
    s.txn = { ...s.txn, edit_seq: s.txn.edit_seq + 1, memo: `${s.txn.memo ?? ''} (edited elsewhere)`.trim(), modified_by: 'someone-else' };
    this.emit('txn', txnId, 'updated');
    return true;
  }

  /** Ids of the seeded records, for tests and the console. */
  find(kind: 'account' | 'entity' | 'item', fullName: string): string | undefined {
    const list: Array<{ id: string; full_name: string }> =
      kind === 'account' ? this.lists.accounts : kind === 'entity' ? (this.lists.entities as Entity[]) : this.lists.items;
    return list.find((r) => r.full_name === fullName)?.id;
  }
}

function kindOfList(list: ListName): string {
  switch (list) {
    case 'accounts':
      return 'account';
    case 'entities':
      return 'entity';
    case 'items':
      return 'item';
    case 'classes':
      return 'class';
    case 'payment-methods':
      return 'payment_method';
    case 'sales-tax-codes':
      return 'sales_tax_code';
    case 'sales-reps':
      return 'sales_rep';
    case 'terms':
      return 'terms';
    case 'types':
      return 'type';
  }
}

export { FIXTURE_COMPANY_ID, STRESS_ACCOUNT_ID };
