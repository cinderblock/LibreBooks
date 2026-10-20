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
  AuditEntry,
  CatalogEntry,
  ChangeEvent,
  ClearedStatus,
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
  RegisterResponse,
  Report,
  ReportRequest,
  Saved,
  SavedKind,
  SaveOutcome,
  Session,
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
