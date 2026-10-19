// The real API client: fetch against `/api/v1` (plans/api.md).
//
// - Cookies ride along (`credentials: 'include'`): browser sessions are an
//   HttpOnly `lb_session` cookie.
// - State-changing requests carry `X-LibreBooks: 1`, the CSRF guard (a browser
//   can't send a custom header cross-site without CORS approval).
// - The desktop app's local mode authenticates with a per-launch bearer token
//   instead; pass it as `token`.
// - Non-2xx answers become a typed `ApiError` from the error envelope.

import type { Report, ReportCatalogEntry, ReportRequest } from '~/reports/types';
import { type ApiClient, type EventHandlers, type EventSubscription, type TxnQuery, txnQueryParams } from './client';
import { ApiError, apiErrorFromBody } from './errors';
import { parseApiJson, stringifyApiJson } from './json';
import type {
  AccountBalance,
  AuditEntry,
  ChangeEvent,
  ClearedStatus,
  CompanyId,
  CompanyInfo,
  CompanySummary,
  DateRange,
  Health,
  ListName,
  ListQuery,
  ListRecords,
  OpenTxn,
  PreviewOutcome,
  RegisterResponse,
  SaveOutcome,
  Session,
  Txn,
  TxnSummary,
  TxnType,
  TxnWithPostings,
} from './types';

type Method = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

/** Just the call signature (Bun's and the DOM's `typeof fetch` carry extras). */
export type FetchFn = (url: string, init: RequestInit) => Promise<Response>;

export interface HttpClientOptions {
  baseUrl?: string;
  /** Desktop local mode: `Authorization: Bearer <token>`. */
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

  private async request<T>(method: Method, path: string, opts: { query?: URLSearchParams; body?: unknown } = {}): Promise<T> {
    const qs = opts.query?.toString();
    const url = `${this.base}${path}${qs ? `?${qs}` : ''}`;
    const headers: Record<string, string> = { Accept: 'application/json' };
    if (method !== 'GET') headers['X-LibreBooks'] = '1';
    if (opts.body !== undefined) headers['Content-Type'] = 'application/json';
    if (this.token) headers.Authorization = `Bearer ${this.token}`;
    let res: Response;
    try {
      res = await this.doFetch(url, {
        method,
        headers,
        credentials: 'include',
        body: opts.body === undefined ? undefined : stringifyApiJson(opts.body),
      });
    } catch (e) {
      throw new ApiError(0, 'network', (e as Error).message || 'Network error');
    }
    const text = res.status === 204 ? '' : await res.text();
    if (!res.ok) {
      let parsed: unknown;
      try {
        parsed = text ? JSON.parse(text) : undefined;
      } catch {
        parsed = undefined;
      }
      throw apiErrorFromBody(res.status, text, parsed);
    }
    if (text === '') return undefined as T;
    try {
      return parseApiJson<T>(text);
    } catch {
      throw new ApiError(res.status, 'bad_response', 'The server sent something that is not JSON.');
    }
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
  deleteRecord(co: CompanyId, list: ListName, id: string, editSeq: bigint) {
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
  voidTxn(co: CompanyId, id: string, editSeq: bigint) {
    return this.request<SaveOutcome>('POST', this.co(co, `/txns/${encodeURIComponent(id)}/void`), {
      body: { edit_seq: editSeq },
    });
  }
  deleteTxn(co: CompanyId, id: string, editSeq: bigint) {
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
    return this.request<OpenTxn[]>('GET', this.co(co, '/open'), {
      query: new URLSearchParams({ entity_id: entityId, kind }),
    });
  }

  reportCatalog(co: CompanyId) {
    return this.request<ReportCatalogEntry[]>('GET', this.co(co, '/reports'));
  }
  runReport(co: CompanyId, request: ReportRequest) {
    return this.request<Report>('POST', this.co(co, '/reports/run'), { body: request });
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
    const url = `${this.base}/events?company=${encodeURIComponent(co)}`;
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
