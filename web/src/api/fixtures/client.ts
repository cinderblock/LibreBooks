// FIXTURE ONLY: an in-memory implementation of the whole `ApiClient` interface,
// seeded with the synthetic company in ./seed.ts. It behaves like the server
// where the UI can tell: optimistic concurrency (stale `edit_seq` → 409), posting
// errors (422 with a `PostingError`), `in_use` deletes, duplicate names, change
// events after every write, and a little latency so loading states are real.
// State lives only in memory: a reload starts over.

import { blankLine } from '../blank';
import { ApiError } from '../errors';
import type { ApiClient, EventHandlers, EventSubscription, TxnQuery } from '../client';
import { clone } from '../json';
import type {
  Account,
  AccountBalance,
  AuditEntry,
  ChangeEvent,
  ClearedStatus,
  CompanyId,
  CompanyInfo,
  CompanySummary,
  DateRange,
  Entity,
  Health,
  ListName,
  ListQuery,
  ListRecords,
  OpenTxn,
  Posting,
  PreviewOutcome,
  RegisterLine,
  RegisterResponse,
  SaveOutcome,
  Session,
  Txn,
  TxnSummary,
  TxnType,
  TxnWithPostings,
} from '../types';
import type { Report, ReportCatalogEntry, ReportRequest } from '~/reports/types';
import { accountTypeOrder } from '~/lib/accountTypes';
import { addDays, todayIso } from '~/lib/dates';
import { hierarchicalCompare } from '~/lib/tree';
import { fixturePost, type PostingBooks } from './post';
import { FIXTURE_COMPANY_ID, buildSeed, stressNames } from './seed';
import { FIXTURE_REPORTS, runFixtureReport } from './reports';

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
    t.edit_seq = 1n;
    t.created_at = `${t.date}T16:00:00Z`;
    t.modified_at = t.created_at;
    // Bank lines older than a month have cleared (gives the register some ✓s).
    const cutoff = addDays(this.today, -30);
    const postings = posted.postings.map((p) => ({ ...p, cleared: t.date < cutoff ? ('Cleared' as const) : p.cleared }));
    this.txnStore.set(t.id, { txn: t, postings, total: posted.total, seq: this.seq++ });
  }

  // ---- server level ---------------------------------------------------------

  health(): Promise<Health> {
    return this.delay(() => ({ ok: true as const, version: 'fixtures', mode: 'server' as const }));
  }
  session(): Promise<Session> {
    return this.delay(() => ({
      user: { id: 'fixture-user', display_name: 'Demo User', role: 'admin' },
      companies: [this.summary()],
    }));
  }
  private summary(): CompanySummary {
    return { id: FIXTURE_COMPANY_ID, name: this.info.name, role: 'admin' };
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
      r.edit_seq = 1n;
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
      r.edit_seq = current.edit_seq + 1n;
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

  deleteRecord(co: CompanyId, list: ListName, id: string, editSeq: bigint): Promise<void> {
    this.checkCompany(co);
    const rows = this.lists[list] as Array<{ id: string; edit_seq: bigint; parent_id?: string | null }>;
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
      }
    }
    return m;
  }

  private openBalance(s: StoredTxn, applied: Map<string, number>): number | null {
    if (![...AR_TYPES, ...AP_TYPES].includes(s.txn.txn_type)) return null;
    if (s.txn.is_void) return 0;
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
      const saved: Txn = { ...clone(txn), amount: posted.total, edit_seq: 1n, created_at: now, modified_at: now, created_by: 'fixture-user', modified_by: 'fixture-user' };
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
      const saved: Txn = { ...clone(txn), amount: posted.total, edit_seq: current.txn.edit_seq + 1n, modified_at: new Date().toISOString(), modified_by: 'fixture-user' };
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

  voidTxn(co: CompanyId, id: string, editSeq: bigint): Promise<SaveOutcome> {
    const s = this.txnStore.get(id);
    if (!s) return Promise.reject(new ApiError(404, 'not_found', 'Not found'));
    return this.updateTxn(co, { ...clone(s.txn), edit_seq: editSeq, is_void: true, memo: `VOID: ${s.txn.memo ?? ''}`.trim() });
  }

  deleteTxn(co: CompanyId, id: string, editSeq: bigint): Promise<void> {
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
      });
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
        edit_seq: 1n,
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

  open(co: CompanyId, entityId: string, kind: 'receivable' | 'payable'): Promise<OpenTxn[]> {
    this.checkCompany(co);
    const applied = this.appliedTo();
    const types = kind === 'receivable' ? AR_TYPES : AP_TYPES;
    const family = this.entityFamily(entityId);
    return this.delay(() =>
      [...this.txnStore.values()]
        .filter((s) => types.includes(s.txn.txn_type) && s.txn.entity_id && family.has(s.txn.entity_id))
        .map((s) => ({ s, open: this.openBalance(s, applied) ?? 0 }))
        .filter(({ open }) => open > 0)
        .map(({ s, open }) => ({
          txn_id: s.txn.id,
          txn_type: s.txn.txn_type,
          date: s.txn.date,
          number: s.txn.number,
          due_date: s.txn.due_date,
          amount: s.total,
          open_balance: open,
        })),
    );
  }

  // ---- reports --------------------------------------------------------------

  reportCatalog(co: CompanyId): Promise<ReportCatalogEntry[]> {
    this.checkCompany(co);
    return this.delay(() => FIXTURE_REPORTS);
  }

  runReport(co: CompanyId, request: ReportRequest): Promise<Report> {
    this.checkCompany(co);
    try {
      const report = runFixtureReport(
        {
          info: this.info,
          accounts: this.sorted('accounts'),
          entities: this.lists.entities,
          txns: [...this.txnStore.values()],
          today: this.today,
        },
        request,
      );
      return this.delay(() => report);
    } catch (e) {
      return Promise.reject(e);
    }
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
    s.txn = { ...s.txn, edit_seq: s.txn.edit_seq + 1n, memo: `${s.txn.memo ?? ''} (edited elsewhere)`.trim(), modified_by: 'someone-else' };
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
