// The one interface every screen talks to. Two implementations: `HttpApiClient`
// (the real server, plans/api.md) and `FixtureApiClient` (in-memory synthetic
// books for `?fixtures=1`). Screens never know which one they have.

import type { Report, ReportCatalogEntry, ReportRequest } from '~/reports/types';
import type {
  AccountBalance,
  AuditEntry,
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
  OpenTxn,
  PreviewOutcome,
  RegisterResponse,
  SaveOutcome,
  Session,
  Txn,
  TxnFilter,
  TxnSummary,
  TxnType,
  TxnWithPostings,
  User,
} from './types';

/** Every field optional: the server fills defaults. */
export type TxnQuery = Partial<TxnFilter>;

export interface EventSubscription {
  close(): void;
}

export interface EventHandlers {
  onChange(e: ChangeEvent): void;
  /** Connection state, for the "live" indicator. */
  onStatus?(status: 'connecting' | 'open' | 'error'): void;
}

export interface ApiClient {
  readonly mode: 'http' | 'fixtures';

  health(): Promise<Health>;
  session(): Promise<Session>;
  companies(): Promise<CompanySummary[]>;

  /** Passkey sign-in: options for `startAuthentication`, then its result. */
  loginStart(): Promise<LoginStart>;
  loginFinish(body: LoginFinish): Promise<{ user: User }>;
  /** Enrolment with a one-time token: options for `startRegistration`, then its result. */
  enrolStart(body: EnrolStartRequest): Promise<EnrolStart>;
  enrolFinish(body: EnrolFinish): Promise<{ user: User }>;
  logout(): Promise<void>;

  companyInfo(co: CompanyId): Promise<CompanyInfo>;

  list<L extends ListName>(co: CompanyId, list: L, query?: ListQuery): Promise<ListRecords[L][]>;
  getRecord<L extends ListName>(co: CompanyId, list: L, id: string): Promise<ListRecords[L]>;
  createRecord<L extends ListName>(co: CompanyId, list: L, record: ListRecords[L]): Promise<ListRecords[L]>;
  updateRecord<L extends ListName>(co: CompanyId, list: L, record: ListRecords[L]): Promise<ListRecords[L]>;
  deleteRecord(co: CompanyId, list: ListName, id: string, editSeq: bigint): Promise<void>;

  txns(co: CompanyId, query?: TxnQuery): Promise<TxnSummary[]>;
  txn(co: CompanyId, id: string): Promise<TxnWithPostings>;
  createTxn(co: CompanyId, txn: Txn): Promise<SaveOutcome>;
  updateTxn(co: CompanyId, txn: Txn): Promise<SaveOutcome>;
  previewTxn(co: CompanyId, txn: Txn): Promise<PreviewOutcome>;
  voidTxn(co: CompanyId, id: string, editSeq: bigint): Promise<SaveOutcome>;
  deleteTxn(co: CompanyId, id: string, editSeq: bigint): Promise<void>;
  nextNumber(co: CompanyId, type: TxnType, accountId?: string | null): Promise<string | null>;
  history(co: CompanyId, id: string): Promise<AuditEntry[]>;

  register(co: CompanyId, accountId: string, range?: Partial<DateRange>): Promise<RegisterResponse>;
  setCleared(co: CompanyId, accountId: string, txnId: string, status: ClearedStatus): Promise<void>;
  balances(co: CompanyId, range?: Partial<DateRange>): Promise<AccountBalance[]>;
  open(co: CompanyId, entityId: string, kind: 'receivable' | 'payable'): Promise<OpenTxn[]>;

  reportCatalog(co: CompanyId): Promise<ReportCatalogEntry[]>;
  runReport(co: CompanyId, request: ReportRequest): Promise<Report>;

  /** Live changes (SSE `/events?company=`); the fixture client emits its own. */
  subscribe(co: CompanyId, handlers: EventHandlers): EventSubscription;
}

/** Query-string form of a TxnFilter, as `GET /txns` takes it. */
export function txnQueryParams(q: TxnQuery = {}): URLSearchParams {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(q)) {
    if (v === null || v === undefined) continue;
    if (Array.isArray(v)) {
      if (v.length) p.set(k, v.join(','));
    } else if (typeof v === 'boolean') {
      if (v) p.set(k, 'true');
    } else {
      p.set(k, String(v));
    }
  }
  return p;
}
