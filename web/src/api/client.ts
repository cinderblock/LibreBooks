// The one interface every screen talks to. Two implementations: `HttpApiClient`
// (the real server, plans/api.md) and `FixtureApiClient` (in-memory synthetic
// books for `?fixtures=1`). Screens never know which one they have.

import type {
  AccountBalance,
  AttachmentMeta,
  AttachmentQuery,
  AttachmentRecord,
  BackupList,
  BackupRecord,
  CompanyBackupsStatus,
  DeviceKeyInfo,
  EnrolmentInfo,
  ImportSettings,
  ImportStatus,
  MergeRequest,
  MintedEnrolmentLink,
  NameBalance,
  NewEnrolment,
  ParityBody,
  ParityStatus,
  PasskeyInfo,
  RestoreRequest,
  Restored,
  Role,
  SecretInfo,
  ServerBackupsStatus,
  ServerSettings,
  ServerView,
  SessionInfo,
  SyncBody,
  TunnelUpdate,
  TunnelView,
  UpdateInstalled,
  UpdateView,
  UserDetail,
  UserUpdate,
  VerifyReport,
  AddOutcome,
  AddRequest,
  AmazonPreview,
  AuditEntry,
  BatchOutcome,
  CatalogEntry,
  ChangeEvent,
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
  ClearedStatus,
  CompanyId,
  CompanyInfo,
  CompanySummary,
  DateRange,
  EnrolFinish,
  EnrolStart,
  EnrolStartRequest,
  Health,
  ImportedCompany,
  ListName,
  ListQuery,
  ListRecords,
  LoginFinish,
  LoginStart,
  MemorizedDue,
  OpeningFor,
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
  /** Admin: a new, empty company from its information (`POST /companies`); answers what `GET /companies` lists for it. */
  createCompany(info: CompanyInfo): Promise<CompanySummary>;
  /**
   * Admin: a NEW company from a `.lbk` file (`POST /companies/import`,
   * multipart `file` plus an optional `name`); nothing existing is replaced.
   * 400 without a file; 422 `damaged` when the file isn't a sound `.lbk`.
   */
  importCompany(file: File, name?: string | null): Promise<ImportedCompany>;

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
  /**
   * `POST /:list` → the saved record. `opening` rides with the record for an
   * account (its opening balance) or an inventory item (on hand and value): the
   * server posts the entry in the same write, so a new account never lands
   * without its balance.
   */
  createRecord<L extends ListName>(co: CompanyId, list: L, record: ListRecords[L], opening?: OpeningFor<L> | null): Promise<ListRecords[L]>;
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
   * Every name with an open A/R or A/P item: what it owes us (`receivable`) and
   * what we owe it (`payable`), both positive, credits and unapplied payments
   * counting against; `*_total` rolls jobs up to their parent (the Customer and
   * Vendor Centers' Balance and Balance Total).
   */
  nameBalances(co: CompanyId): Promise<NameBalance[]>;
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

  // ---- bank feeds (plans/api.md "Bank feeds") ----
  feedsSummary(co: CompanyId): Promise<FeedAccountSummary[]>;
  feedTxns(co: CompanyId, query?: FeedQuery): Promise<FeedTxn[]>;
  feedTxn(co: CompanyId, id: string): Promise<FeedTxn>;
  /** Look at a file before importing: its kind, a CSV mapping guess with a preview, or the OFX statements. */
  feedDetect(co: CompanyId, file: File): Promise<FeedDetection>;
  feedImport(co: CompanyId, req: FeedImportRequest): Promise<IngestReport>;
  feedCandidates(co: CompanyId, id: string, days?: number): Promise<MatchCandidate[]>;
  feedMatch(co: CompanyId, id: string, txnId: string): Promise<void>;
  feedPreview(co: CompanyId, id: string, req: AddRequest): Promise<FeedAddPreview>;
  feedAdd(co: CompanyId, id: string, req: AddRequest): Promise<AddOutcome>;
  feedAddAll(co: CompanyId, ids: string[], req?: AddRequest): Promise<BatchOutcome>;
  feedIgnore(co: CompanyId, ids: string[]): Promise<number>;
  feedUndo(co: CompanyId, id: string): Promise<FeedTxn>;
  feedConnections(co: CompanyId): Promise<FeedConnection[]>;
  createFeedConnection(co: CompanyId, c: FeedConnection): Promise<FeedConnection>;
  updateFeedConnection(co: CompanyId, c: FeedConnection): Promise<FeedConnection>;
  deleteFeedConnection(co: CompanyId, id: string, editSeq: number): Promise<void>;
  /** Write-only: a password, PIN, access URL or token, kept by the server. */
  setFeedCredentials(co: CompanyId, id: string, value: string): Promise<FeedConnection>;
  syncFeedConnection(co: CompanyId, id: string): Promise<IngestReport>;
  feedRemoteAccounts(co: CompanyId, id: string): Promise<StatementAccount[]>;
  simplefinClaim(co: CompanyId, setupToken: string): Promise<SimpleFinClaim>;
  feedRules(co: CompanyId): Promise<FeedRule[]>;
  createFeedRule(co: CompanyId, r: FeedRule): Promise<FeedRule>;
  updateFeedRule(co: CompanyId, r: FeedRule): Promise<FeedRule>;
  deleteFeedRule(co: CompanyId, id: string): Promise<void>;
  reorderFeedRules(co: CompanyId, ids: string[]): Promise<FeedRule[]>;
  /** Re-run the rules over waiting transactions; returns how many got a suggestion. */
  applyFeedRules(co: CompanyId, ids?: string[], accountId?: string | null): Promise<number>;
  amazonOrders(co: CompanyId, file: File, accountId?: string | null): Promise<AmazonPreview>;
  amazonApply(co: CompanyId, preview: AmazonPreview): Promise<number>;

  // ---- users and enrolment (admin; plans/api.md "Server-level") ----
  users(): Promise<User[]>;
  user(id: string): Promise<UserDetail>;
  updateUser(id: string, update: UserUpdate): Promise<User>;
  deleteUser(id: string): Promise<void>;
  grantAccess(userId: string, co: CompanyId, role: Role): Promise<void>;
  revokeAccess(userId: string, co: CompanyId): Promise<void>;
  enrollments(): Promise<EnrolmentInfo[]>;
  /** Mint a link; `token` and `url` are shown once and never stored. */
  createEnrollment(body: NewEnrolment): Promise<MintedEnrolmentLink>;
  revokeEnrollment(id: string): Promise<void>;

  // ---- my account ----
  passkeys(): Promise<PasskeyInfo[]>;
  renamePasskey(id: string, name: string): Promise<void>;
  deletePasskey(id: string): Promise<void>;
  devices(): Promise<DeviceKeyInfo[]>;
  revokeDevice(id: string): Promise<void>;
  sessions(): Promise<SessionInfo[]>;
  revokeSession(id: string): Promise<void>;

  // ---- server administration (plans/server-ops.md) ----
  server(): Promise<ServerView>;
  /** The whole settings document; 400 `invalid` with `detail.problems` when something must be fixed first. */
  putServer(settings: ServerSettings): Promise<ServerView>;
  secrets(): Promise<SecretInfo[]>;
  /** Write-only: the value is never returned. */
  putSecret(name: string, value: string): Promise<void>;
  deleteSecret(name: string): Promise<void>;
  update(): Promise<UpdateView>;
  checkUpdate(): Promise<UpdateView>;
  installUpdate(restart: boolean): Promise<UpdateInstalled>;
  tunnel(): Promise<TunnelView>;
  putTunnel(update: TunnelUpdate): Promise<TunnelView>;
  serverBackupsStatus(): Promise<ServerBackupsStatus>;

  // ---- company: info, closing date, backups, attachments, merge ----
  putCompanyInfo(co: CompanyId, info: CompanyInfo): Promise<CompanyInfo>;
  closingDate(co: CompanyId): Promise<string | null>;
  putClosingDate(co: CompanyId, date: string | null): Promise<string | null>;
  backups(co: CompanyId): Promise<BackupList>;
  backupsStatus(co: CompanyId): Promise<CompanyBackupsStatus>;
  runBackup(co: CompanyId): Promise<BackupRecord>;
  verifyBackup(co: CompanyId, id: string, destination: number, unlock?: RestoreRequest): Promise<VerifyReport>;
  /** Restores into a NEW company; nothing existing is replaced. */
  restoreBackup(co: CompanyId, id: string, destination: number, body: RestoreRequest): Promise<Restored>;
  exportCompany(co: CompanyId): Promise<FileDownload>;
  /** The inbox (nothing attached yet), what's attached to one record, or (`all`) every document. */
  attachments(co: CompanyId, query?: AttachmentQuery): Promise<AttachmentMeta[]>;
  uploadAttachment(co: CompanyId, file: File, record?: AttachmentRecord): Promise<AttachmentMeta>;
  /** Attach to a record, or detach (null) back into the inbox. */
  attachRecord(co: CompanyId, id: string, record: AttachmentRecord): Promise<AttachmentMeta>;
  deleteAttachment(co: CompanyId, id: string): Promise<void>;
  /** Where the browser can fetch the file itself (an <img src>, a link). */
  attachmentUrl(co: CompanyId, id: string): string;
  merge(co: CompanyId, req: MergeRequest): Promise<void>;

  // ---- QuickBooks import (`/import/qbd/*`, admin) ----
  /** `wait` seconds (at most 120): long-poll until the running job finishes. */
  qbdStatus(co: CompanyId, wait?: number): Promise<ImportStatus>;
  /**
   * The settings whole, plus the write-only bridge key (saved as a secret;
   * never returned). 400 `invalid` lists `detail.problems`; 409 `duplicate`
   * when another company uses the Web Connector user name.
   */
  putQbdSettings(co: CompanyId, settings: ImportSettings, bridgeKey?: string | null): Promise<ImportStatus>;
  /** 202 with the status; 409 `running` while a job is going. */
  qbdSync(co: CompanyId, opts?: Partial<SyncBody>): Promise<ImportStatus>;
  qbdParityRun(co: CompanyId, opts?: Partial<ParityBody>): Promise<ImportStatus>;
  /** The last parity report and whether a parity job runs; `wait` long-polls like `qbdStatus`. */
  qbdParity(co: CompanyId, wait?: number): Promise<ParityStatus>;
  qbwcFile(co: CompanyId): Promise<FileDownload>;
  /**
   * Set the Web Connector password (null back), or let the server make one:
   * the generated password, shown this once and never again.
   */
  qbwcPassword(co: CompanyId, password?: string | null): Promise<string | null>;

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
