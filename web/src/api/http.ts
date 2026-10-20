// The real API client: fetch against `/api/v1` (plans/api.md).
//
// - Cookies ride along (`credentials: 'include'`): browser sessions are an
//   HttpOnly `lb_session` cookie.
// - State-changing requests carry `X-LibreBooks: 1`, the CSRF guard (a browser
//   can't send a custom header cross-site without CORS approval).
// - The desktop app's local mode authenticates with a per-launch bearer token
//   instead; pass it as `token`.
// - Non-2xx answers become a typed `ApiError` from the error envelope.

import { type ApiClient, type EventHandlers, type EventSubscription, type FileDownload, type TxnQuery, txnQueryParams } from './client';
import { ApiError, apiErrorFromBody } from './errors';
import { parseApiJson, stringifyApiJson } from './json';
import type {
  AccountBalance,
  AddOutcome,
  AddRequest,
  AmazonPreview,
  AuditEntry,
  BatchOutcome,
  CatalogEntry,
  ChangeEvent,
  ClearedStatus,
  FeedAccountSummary,
  FeedAddPreview,
  FeedConnection,
  FeedDetection,
  FeedImportRequest,
  FeedQuery,
  FeedRule,
  FeedTxn,
  IngestReport,
  MatchCandidate,
  SimpleFinClaim,
  StatementAccount,
  CompanyId,
  CompanyInfo,
  CompanySummary,
  DateRange,
  EnrolFinish,
  EnrolStart,
  EnrolStartRequest,
  Health,
  ListName,
  ListQuery,
  ListRecords,
  LoginFinish,
  LoginStart,
  MemorizedDue,
  PreviewOutcome,
  ReconcileOverview,
  ReconcileStatus,
  Reconciliation,
  ReconciliationView,
  RegisterResponse,
  Report,
  ReportRequest,
  Saved,
  SavedKind,
  SaveOutcome,
  Session,
  StartReconcile,
  Txn,
  TxnSummary,
  TxnType,
  TxnWithPostings,
  User,
} from './types';

/** The filename a `Content-Disposition: attachment; filename="…"` header names. */
export function dispositionFilename(header: string | null, fallback: string): string {
  const m = header ? /filename\*?=(?:UTF-8''|")?([^";]+)"?/i.exec(header) : null;
  return m?.[1] ? decodeURIComponent(m[1]) : fallback;
}

type Method = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

/** Just the call signature (Bun's and the DOM's `typeof fetch` carry extras). */
export type FetchFn = (url: string, init: RequestInit) => Promise<Response>;

export interface HttpClientOptions {
  baseUrl?: string;
  /** Desktop app: `Authorization: Bearer <token>` (and `?token=` on the event stream). */
  token?: string | null;
  fetch?: FetchFn;
}

function rangeParams(range?: Partial<DateRange>): URLSearchParams {
  const p = new URLSearchParams();
  if (range?.from) p.set('from', range.from);
  if (range?.to) p.set('to', range.to);
  return p;
}

export class HttpApiClient implements ApiClient {
  readonly mode = 'http' as const;
  private readonly base: string;
  private readonly token: string | null;
  private readonly doFetch: FetchFn;

  constructor(opts: HttpClientOptions = {}) {
    this.base = (opts.baseUrl ?? '/api/v1').replace(/\/$/, '');
    this.token = opts.token ?? null;
    this.doFetch = opts.fetch ?? ((url, init) => fetch(url, init));
  }

  /**
   * One request, with the auth and CSRF headers, raising `ApiError` on a non-2xx
   * answer. `body` is JSON unless it's a `FormData` (multipart uploads), whose
   * content type the browser sets (it carries the boundary).
   */
  private async raw(method: Method, path: string, opts: { query?: URLSearchParams; body?: unknown; accept?: string } = {}): Promise<Response> {
    const qs = opts.query?.toString();
    const url = `${this.base}${path}${qs ? `?${qs}` : ''}`;
    const headers: Record<string, string> = { Accept: opts.accept ?? 'application/json' };
    if (method !== 'GET') headers['X-LibreBooks'] = '1';
    const multipart = typeof FormData !== 'undefined' && opts.body instanceof FormData;
    if (opts.body !== undefined && !multipart) headers['Content-Type'] = 'application/json';
    if (this.token) headers.Authorization = `Bearer ${this.token}`;
    let res: Response;
    try {
      res = await this.doFetch(url, {
        method,
        headers,
        credentials: 'include',
        body: opts.body === undefined ? undefined : multipart ? (opts.body as FormData) : stringifyApiJson(opts.body),
      });
    } catch (e) {
      throw new ApiError(0, 'network', (e as Error).message || 'Network error');
    }
    if (!res.ok) {
      const text = await res.text();
      let parsed: unknown;
      try {
        parsed = text ? JSON.parse(text) : undefined;
      } catch {
        parsed = undefined;
      }
      throw apiErrorFromBody(res.status, text, parsed);
    }
    return res;
  }

  private async request<T>(method: Method, path: string, opts: { query?: URLSearchParams; body?: unknown } = {}): Promise<T> {
    const res = await this.raw(method, path, opts);
    const text = res.status === 204 ? '' : await res.text();
    if (text === '') return undefined as T;
    try {
      return parseApiJson<T>(text);
    } catch {
      throw new ApiError(res.status, 'bad_response', 'The server sent something that is not JSON.');
    }
  }

  /** A file the server produced, named by its Content-Disposition. */
  private async download(method: Method, path: string, fallback: string, opts: { query?: URLSearchParams; body?: unknown } = {}): Promise<FileDownload> {
    const res = await this.raw(method, path, { ...opts, accept: '*/*' });
    return { blob: await res.blob(), filename: dispositionFilename(res.headers.get('content-disposition'), fallback) };
  }

  private co(co: CompanyId, path: string): string {
    return `/c/${encodeURIComponent(co)}${path}`;
  }

  health() {
    return this.request<Health>('GET', '/health');
  }
  session() {
    return this.request<Session>('GET', '/session');
  }
  companies() {
    return this.request<CompanySummary[]>('GET', '/companies');
  }
  loginStart() {
    return this.request<LoginStart>('POST', '/auth/login/start');
  }
  loginFinish(body: LoginFinish) {
    return this.request<{ user: User }>('POST', '/auth/login/finish', { body });
  }
  enrolStart(body: EnrolStartRequest) {
    return this.request<EnrolStart>('POST', '/auth/enroll/start', { body });
  }
  enrolFinish(body: EnrolFinish) {
    return this.request<{ user: User }>('POST', '/auth/enroll/finish', { body });
  }
  logout() {
    return this.request<void>('POST', '/auth/logout');
  }
  companyInfo(co: CompanyId) {
    return this.request<CompanyInfo>('GET', this.co(co, '/info'));
  }

  list<L extends ListName>(co: CompanyId, list: L, query: ListQuery = {}) {
    const p = new URLSearchParams();
    if (query.include_inactive) p.set('include_inactive', 'true');
    if (query.kind) p.set('kind', query.kind);
    if (query.type) p.set('type', query.type);
    return this.request<ListRecords[L][]>('GET', this.co(co, `/${list}`), { query: p });
  }
  getRecord<L extends ListName>(co: CompanyId, list: L, id: string) {
    return this.request<ListRecords[L]>('GET', this.co(co, `/${list}/${encodeURIComponent(id)}`));
  }
  createRecord<L extends ListName>(co: CompanyId, list: L, record: ListRecords[L]) {
    return this.request<ListRecords[L]>('POST', this.co(co, `/${list}`), { body: record });
  }
  updateRecord<L extends ListName>(co: CompanyId, list: L, record: ListRecords[L]) {
    return this.request<ListRecords[L]>('PUT', this.co(co, `/${list}/${encodeURIComponent(record.id)}`), { body: record });
  }
  deleteRecord(co: CompanyId, list: ListName, id: string, editSeq: number) {
    return this.request<void>('DELETE', this.co(co, `/${list}/${encodeURIComponent(id)}`), {
      query: new URLSearchParams({ edit_seq: String(editSeq) }),
    });
  }

  txns(co: CompanyId, query?: TxnQuery) {
    return this.request<TxnSummary[]>('GET', this.co(co, '/txns'), { query: txnQueryParams(query) });
  }
  txn(co: CompanyId, id: string) {
    return this.request<TxnWithPostings>('GET', this.co(co, `/txns/${encodeURIComponent(id)}`));
  }
  createTxn(co: CompanyId, txn: Txn) {
    return this.request<SaveOutcome>('POST', this.co(co, '/txns'), { body: txn });
  }
  updateTxn(co: CompanyId, txn: Txn) {
    return this.request<SaveOutcome>('PUT', this.co(co, `/txns/${encodeURIComponent(txn.id)}`), { body: txn });
  }
  previewTxn(co: CompanyId, txn: Txn) {
    return this.request<PreviewOutcome>('POST', this.co(co, '/txns/preview'), { body: txn });
  }
  voidTxn(co: CompanyId, id: string, editSeq: number) {
    return this.request<SaveOutcome>('POST', this.co(co, `/txns/${encodeURIComponent(id)}/void`), {
      body: { edit_seq: editSeq },
    });
  }
  deleteTxn(co: CompanyId, id: string, editSeq: number) {
    return this.request<void>('DELETE', this.co(co, `/txns/${encodeURIComponent(id)}`), {
      query: new URLSearchParams({ edit_seq: String(editSeq) }),
    });
  }
  async nextNumber(co: CompanyId, type: TxnType, accountId?: string | null) {
    const p = new URLSearchParams({ type });
    if (accountId) p.set('account_id', accountId);
    const r = await this.request<{ number: string | null }>('GET', this.co(co, '/txns/next-number'), { query: p });
    return r.number;
  }
  history(co: CompanyId, id: string) {
    return this.request<AuditEntry[]>('GET', this.co(co, `/txns/${encodeURIComponent(id)}/history`));
  }

  register(co: CompanyId, accountId: string, range?: Partial<DateRange>) {
    return this.request<RegisterResponse>('GET', this.co(co, `/accounts/${encodeURIComponent(accountId)}/register`), {
      query: rangeParams(range),
    });
  }
  setCleared(co: CompanyId, accountId: string, txnId: string, status: ClearedStatus) {
    return this.request<void>(
      'PUT',
      this.co(co, `/accounts/${encodeURIComponent(accountId)}/register/${encodeURIComponent(txnId)}/cleared`),
      { body: { status } },
    );
  }
  balances(co: CompanyId, range?: Partial<DateRange>) {
    return this.request<AccountBalance[]>('GET', this.co(co, '/balances'), { query: rangeParams(range) });
  }
  open(co: CompanyId, entityId: string, kind: 'receivable' | 'payable') {
    return this.request<TxnSummary[]>('GET', this.co(co, '/open'), {
      query: new URLSearchParams({ entity_id: entityId, kind }),
    });
  }
  undeposited(co: CompanyId) {
    return this.request<TxnSummary[]>('GET', this.co(co, '/undeposited'));
  }

  reportCatalog(co: CompanyId) {
    return this.request<CatalogEntry[]>('GET', this.co(co, '/reports'));
  }
  runReport(co: CompanyId, request: ReportRequest) {
    return this.request<Report>('POST', this.co(co, '/reports/run'), { body: request });
  }
  exportReport(co: CompanyId, request: ReportRequest, format: 'csv') {
    return this.download('POST', this.co(co, '/reports/export'), `${request.title ?? request.report}.${format}`, {
      query: new URLSearchParams({ format }),
      body: request,
    });
  }

  saved(co: CompanyId, kind: SavedKind) {
    return this.request<Saved[]>('GET', this.co(co, `/saved/${kind}`));
  }
  createSaved(co: CompanyId, kind: SavedKind, name: string, doc: unknown) {
    return this.request<Saved>('POST', this.co(co, `/saved/${kind}`), { body: { name, doc } });
  }
  updateSaved(co: CompanyId, kind: SavedKind, id: string, name: string, doc: unknown) {
    return this.request<Saved>('PUT', this.co(co, `/saved/${kind}/${encodeURIComponent(id)}`), { body: { name, doc } });
  }
  deleteSaved(co: CompanyId, kind: SavedKind, id: string) {
    return this.request<void>('DELETE', this.co(co, `/saved/${kind}/${encodeURIComponent(id)}`));
  }
  memorizedDue(co: CompanyId) {
    return this.request<MemorizedDue[]>('GET', this.co(co, '/memorized/due'));
  }
  useMemorized(co: CompanyId, id: string, date?: string | null) {
    const p = new URLSearchParams();
    if (date) p.set('date', date);
    return this.request<Txn>('GET', this.co(co, `/memorized/${encodeURIComponent(id)}/use`), { query: p });
  }

  reconcileOverview(co: CompanyId, accountId: string) {
    return this.request<ReconcileOverview>('GET', this.co(co, `/accounts/${encodeURIComponent(accountId)}/reconcile`));
  }
  startReconcile(co: CompanyId, accountId: string, body: StartReconcile) {
    return this.request<Reconciliation>('POST', this.co(co, `/accounts/${encodeURIComponent(accountId)}/reconcile`), { body });
  }
  reconciliation(co: CompanyId, id: string) {
    return this.request<ReconciliationView>('GET', this.co(co, `/reconciliations/${encodeURIComponent(id)}`));
  }
  markReconcile(co: CompanyId, id: string, txnIds: string[], marked: boolean) {
    return this.request<ReconcileStatus>('POST', this.co(co, `/reconciliations/${encodeURIComponent(id)}/mark`), { body: { txn_ids: txnIds, marked } });
  }
  finishReconcile(co: CompanyId, id: string, adjust: boolean) {
    return this.request<Reconciliation>('POST', this.co(co, `/reconciliations/${encodeURIComponent(id)}/finish`), { body: { adjust } });
  }
  discardReconcile(co: CompanyId, id: string) {
    return this.request<void>('DELETE', this.co(co, `/reconciliations/${encodeURIComponent(id)}`));
  }
  async undoReconcile(co: CompanyId, accountId: string) {
    const r = await this.request<{ undone: Reconciliation | null }>('POST', this.co(co, `/accounts/${encodeURIComponent(accountId)}/reconcile/undo`));
    return r.undone;
  }

  // ---- bank feeds -----------------------------------------------------------

  private feedQuery(q: FeedQuery = {}): URLSearchParams {
    const p = new URLSearchParams();
    for (const [k, v] of Object.entries(q)) {
      if (v === null || v === undefined) continue;
      if (Array.isArray(v)) {
        if (v.length) p.set(k, v.join(','));
      } else if (typeof v === 'boolean') {
        if (v) p.set(k, 'true');
      } else p.set(k, String(v));
    }
    return p;
  }
  feedsSummary(co: CompanyId) {
    return this.request<FeedAccountSummary[]>('GET', this.co(co, '/feeds'));
  }
  feedTxns(co: CompanyId, query?: FeedQuery) {
    return this.request<FeedTxn[]>('GET', this.co(co, '/feeds/txns'), { query: this.feedQuery(query) });
  }
  feedTxn(co: CompanyId, id: string) {
    return this.request<FeedTxn>('GET', this.co(co, `/feeds/txns/${encodeURIComponent(id)}`));
  }
  feedDetect(co: CompanyId, file: File) {
    const body = new FormData();
    body.append('file', file, file.name);
    return this.request<FeedDetection>('POST', this.co(co, '/feeds/detect'), { body });
  }
  feedImport(co: CompanyId, req: FeedImportRequest) {
    const body = new FormData();
    body.append('file', req.file, req.file.name);
    if (req.account_id) body.append('account_id', req.account_id);
    if (req.connection_id) body.append('connection_id', req.connection_id);
    if (req.mapping) body.append('mapping', JSON.stringify(req.mapping));
    if (req.statement !== null && req.statement !== undefined) body.append('statement', String(req.statement));
    return this.request<IngestReport>('POST', this.co(co, '/feeds/import'), { body });
  }
  feedCandidates(co: CompanyId, id: string, days?: number) {
    const p = new URLSearchParams();
    if (days) p.set('days', String(days));
    return this.request<MatchCandidate[]>('GET', this.co(co, `/feeds/txns/${encodeURIComponent(id)}/candidates`), { query: p });
  }
  feedMatch(co: CompanyId, id: string, txnId: string) {
    return this.request<void>('POST', this.co(co, `/feeds/txns/${encodeURIComponent(id)}/match`), { body: { txn_id: txnId } });
  }
  feedPreview(co: CompanyId, id: string, req: AddRequest) {
    return this.request<FeedAddPreview>('POST', this.co(co, `/feeds/txns/${encodeURIComponent(id)}/preview`), { body: req });
  }
  feedAdd(co: CompanyId, id: string, req: AddRequest) {
    return this.request<AddOutcome>('POST', this.co(co, `/feeds/txns/${encodeURIComponent(id)}/add`), { body: req });
  }
  feedAddAll(co: CompanyId, ids: string[], req?: AddRequest) {
    return this.request<BatchOutcome>('POST', this.co(co, '/feeds/txns/add-all'), { body: { ids, request: req ?? null } });
  }
  async feedIgnore(co: CompanyId, ids: string[]) {
    const r = await this.request<{ ignored: number }>('POST', this.co(co, '/feeds/txns/ignore'), { body: { ids } });
    return r.ignored;
  }
  feedUndo(co: CompanyId, id: string) {
    return this.request<FeedTxn>('POST', this.co(co, `/feeds/txns/${encodeURIComponent(id)}/undo`));
  }
  feedConnections(co: CompanyId) {
    return this.request<FeedConnection[]>('GET', this.co(co, '/feeds/connections'));
  }
  createFeedConnection(co: CompanyId, c: FeedConnection) {
    return this.request<FeedConnection>('POST', this.co(co, '/feeds/connections'), { body: c });
  }
  updateFeedConnection(co: CompanyId, c: FeedConnection) {
    return this.request<FeedConnection>('PUT', this.co(co, `/feeds/connections/${encodeURIComponent(c.id)}`), { body: c });
  }
  deleteFeedConnection(co: CompanyId, id: string, editSeq: number) {
    return this.request<void>('DELETE', this.co(co, `/feeds/connections/${encodeURIComponent(id)}`), { query: new URLSearchParams({ edit_seq: String(editSeq) }) });
  }
  setFeedCredentials(co: CompanyId, id: string, value: string) {
    return this.request<FeedConnection>('PUT', this.co(co, `/feeds/connections/${encodeURIComponent(id)}/credentials`), { body: { value } });
  }
  syncFeedConnection(co: CompanyId, id: string) {
    return this.request<IngestReport>('POST', this.co(co, `/feeds/connections/${encodeURIComponent(id)}/sync`));
  }
  feedRemoteAccounts(co: CompanyId, id: string) {
    return this.request<StatementAccount[]>('POST', this.co(co, `/feeds/connections/${encodeURIComponent(id)}/remote-accounts`));
  }
  simplefinClaim(co: CompanyId, setupToken: string) {
    return this.request<SimpleFinClaim>('POST', this.co(co, '/feeds/simplefin/claim'), { body: { setup_token: setupToken } });
  }
  feedRules(co: CompanyId) {
    return this.request<FeedRule[]>('GET', this.co(co, '/feeds/rules'));
  }
  createFeedRule(co: CompanyId, r: FeedRule) {
    return this.request<FeedRule>('POST', this.co(co, '/feeds/rules'), { body: r });
  }
  updateFeedRule(co: CompanyId, r: FeedRule) {
    return this.request<FeedRule>('PUT', this.co(co, `/feeds/rules/${encodeURIComponent(r.id)}`), { body: r });
  }
  deleteFeedRule(co: CompanyId, id: string) {
    return this.request<void>('DELETE', this.co(co, `/feeds/rules/${encodeURIComponent(id)}`));
  }
  reorderFeedRules(co: CompanyId, ids: string[]) {
    return this.request<FeedRule[]>('POST', this.co(co, '/feeds/rules/reorder'), { body: { ids } });
  }
  async applyFeedRules(co: CompanyId, ids?: string[], accountId?: string | null) {
    const r = await this.request<{ applied: number }>('POST', this.co(co, '/feeds/rules/apply'), { body: { ids: ids ?? null, account_id: accountId ?? null } });
    return r.applied;
  }
  amazonOrders(co: CompanyId, file: File, accountId?: string | null) {
    const body = new FormData();
    body.append('file', file, file.name);
    if (accountId) body.append('account_id', accountId);
    return this.request<AmazonPreview>('POST', this.co(co, '/feeds/amazon/orders'), { body });
  }
  async amazonApply(co: CompanyId, preview: AmazonPreview) {
    const r = await this.request<{ annotated: number }>('POST', this.co(co, '/feeds/amazon/apply'), { body: preview });
    return r.annotated;
  }

  /**
   * SSE. `EventSource` reconnects by itself after a dropped connection; when the
   * browser gives up (readyState CLOSED, e.g. a 401), retry with backoff so a
   * laptop waking from sleep picks the stream back up.
   */
  subscribe(co: CompanyId, handlers: EventHandlers): EventSubscription {
    let source: EventSource | null = null;
    let closed = false;
    let retry = 1000;
    let timer: ReturnType<typeof setTimeout> | undefined;
    // EventSource can't send headers; the desktop app's token rides in the query (server accepts it for /events only).
    const url = `${this.base}/events?company=${encodeURIComponent(co)}${this.token ? `&token=${encodeURIComponent(this.token)}` : ''}`;
    const connect = () => {
      if (closed) return;
      handlers.onStatus?.('connecting');
      source = new EventSource(url, { withCredentials: true });
      source.onopen = () => {
        retry = 1000;
        handlers.onStatus?.('open');
      };
      source.addEventListener('change', (ev) => {
        try {
          handlers.onChange(JSON.parse((ev as MessageEvent<string>).data) as ChangeEvent);
        } catch {
          // A malformed event is not worth tearing the stream down for.
        }
      });
      source.onerror = () => {
        handlers.onStatus?.('error');
        if (source?.readyState === EventSource.CLOSED) {
          source = null;
          timer = setTimeout(connect, retry);
          retry = Math.min(retry * 2, 30_000);
        }
      };
    };
    connect();
    return {
      close() {
        closed = true;
        clearTimeout(timer);
        source?.close();
      },
    };
  }
}
