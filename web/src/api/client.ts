// The one interface every screen talks to. Two implementations: `HttpApiClient`
// (the real server, plans/api.md) and `FixtureApiClient` (in-memory synthetic
// books for `?fixtures=1`). Screens never know which one they have.

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
  ReconcileOverview,
  ReconcileStatus,
  Reconciliation,
  ReconciliationView,
  RegisterResponse,
  Report,
  ReportRequest,
  StartReconcile,
  Saved,
  SavedKind,
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

/** A file the server produced (a CSV export, a company file). */
export interface FileDownload {
  blob: Blob;
  filename: string;
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
  deleteRecord(co: CompanyId, list: ListName, id: string, editSeq: number): Promise<void>;

  txns(co: CompanyId, query?: TxnQuery): Promise<TxnSummary[]>;
  txn(co: CompanyId, id: string): Promise<TxnWithPostings>;
  createTxn(co: CompanyId, txn: Txn): Promise<SaveOutcome>;
  updateTxn(co: CompanyId, txn: Txn): Promise<SaveOutcome>;
  previewTxn(co: CompanyId, txn: Txn): Promise<PreviewOutcome>;
  voidTxn(co: CompanyId, id: string, editSeq: number): Promise<SaveOutcome>;
  deleteTxn(co: CompanyId, id: string, editSeq: number): Promise<void>;
  nextNumber(co: CompanyId, type: TxnType, accountId?: string | null): Promise<string | null>;
  history(co: CompanyId, id: string): Promise<AuditEntry[]>;

  register(co: CompanyId, accountId: string, range?: Partial<DateRange>): Promise<RegisterResponse>;
  setCleared(co: CompanyId, accountId: string, txnId: string, status: ClearedStatus): Promise<void>;
  balances(co: CompanyId, range?: Partial<DateRange>): Promise<AccountBalance[]>;
  /**
   * Open items for a name and its jobs: invoices, charges and unused credits /
   * unapplied payments (receivable), or bills and unused credits (payable).
   */
  open(co: CompanyId, entityId: string, kind: 'receivable' | 'payable'): Promise<TxnSummary[]>;
  /** Payments waiting in Undeposited Funds (Make Deposits). */
  undeposited(co: CompanyId): Promise<TxnSummary[]>;

  reportCatalog(co: CompanyId): Promise<CatalogEntry[]>;
  runReport(co: CompanyId, request: ReportRequest): Promise<Report>;
  exportReport(co: CompanyId, request: ReportRequest, format: 'csv'): Promise<FileDownload>;

  /** Saved documents: memorized transactions (`doc: Memorized`) and reports (`doc: ReportRequest`). */
  saved(co: CompanyId, kind: SavedKind): Promise<Saved[]>;
  createSaved(co: CompanyId, kind: SavedKind, name: string, doc: unknown): Promise<Saved>;
  updateSaved(co: CompanyId, kind: SavedKind, id: string, name: string, doc: unknown): Promise<Saved>;
  deleteSaved(co: CompanyId, kind: SavedKind, id: string): Promise<void>;
  memorizedDue(co: CompanyId): Promise<MemorizedDue[]>;
  /** A fresh, unsaved transaction from a memorized one (`edit_seq` 0, today's date). */
  useMemorized(co: CompanyId, id: string, date?: string | null): Promise<Txn>;

  /** Reconciliation: what's in progress and finished for an account, start one, mark lines, finish, discard, undo the last. */
  reconcileOverview(co: CompanyId, accountId: string): Promise<ReconcileOverview>;
  startReconcile(co: CompanyId, accountId: string, body: StartReconcile): Promise<Reconciliation>;
  reconciliation(co: CompanyId, id: string): Promise<ReconciliationView>;
  markReconcile(co: CompanyId, id: string, txnIds: string[], marked: boolean): Promise<ReconcileStatus>;
  /** `adjust`: write the difference off to Reconciliation Discrepancies. */
  finishReconcile(co: CompanyId, id: string, adjust: boolean): Promise<Reconciliation>;
  discardReconcile(co: CompanyId, id: string): Promise<void>;
  undoReconcile(co: CompanyId, accountId: string): Promise<Reconciliation | null>;

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

/** Trigger a browser download of a file the API produced. */
export function saveDownload(file: FileDownload) {
  const a = document.createElement('a');
  a.href = URL.createObjectURL(file.blob);
  a.download = file.filename;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}
