// API shapes from plans/api.md that ts-rs does not generate (yet).
//
// Everything that *is* generated is imported from ./generated and re-exported
// here so screens have one import site. When the server crate starts exporting
// one of the local stand-ins below (SaveOutcome, the session shape, the reconcile
// envelopes…), delete the local definition and re-export the generated one.

import type { Account } from './generated/Account';
import type { AmazonMatch } from './generated/AmazonMatch';
import type { AmazonOrder } from './generated/AmazonOrder';
import type { Class } from './generated/Class';
import type { CompanyAccess } from './generated/CompanyAccess';
import type { CsvDetection } from './generated/CsvDetection';
import type { CsvMapping } from './generated/CsvMapping';
import type { DeviceKeyInfo } from './generated/DeviceKeyInfo';
import type { FeedFilter } from './generated/FeedFilter';
import type { FileKind } from './generated/FileKind';
import type { PasskeyInfo } from './generated/PasskeyInfo';
import type { PortableManifest } from './generated/PortableManifest';
import type { Statement } from './generated/Statement';
import type { StatementAccount } from './generated/StatementAccount';
import type { StatementCharge } from './generated/StatementCharge';
import type { User } from './generated/User';
import type { CompanyId } from './generated/CompanyId';
import type { Entity } from './generated/Entity';
import type { EntityKind } from './generated/EntityKind';
import type { Item } from './generated/Item';
import type { ItemType } from './generated/ItemType';
import type { Money } from './generated/Money';
import type { PaymentMethod } from './generated/PaymentMethod';
import type { Posting } from './generated/Posting';
import type { ReconcileLine } from './generated/ReconcileLine';
import type { ReconcileStatus } from './generated/ReconcileStatus';
import type { Reconciliation } from './generated/Reconciliation';
import type { RegisterLine } from './generated/RegisterLine';
import type { Repeat } from './generated/Repeat';
import type { Role } from './generated/Role';
import type { SalesRep } from './generated/SalesRep';
import type { SalesTaxCode } from './generated/SalesTaxCode';
import type { SessionKind } from './generated/SessionKind';
import type { Terms } from './generated/Terms';
import type { Txn } from './generated/Txn';
import type { TypeEntry } from './generated/TypeEntry';

export type { Account } from './generated/Account';
export type { AccountBalance } from './generated/AccountBalance';
export type { CashFlowClass } from './generated/CashFlowClass';
export type { ItemComponent } from './generated/ItemComponent';
export type { JobStatus } from './generated/JobStatus';
export type { PaymentMethodKind } from './generated/PaymentMethodKind';
export type { AccountId } from './generated/AccountId';
export type { AccountRole } from './generated/AccountRole';
export type { AccountType } from './generated/AccountType';
export type { Address } from './generated/Address';
export type { EnrolFinish } from './generated/EnrolFinish';
export type { EnrolStart } from './generated/EnrolStart';
export type { EnrolStartRequest } from './generated/EnrolStartRequest';
export type { LoginFinish } from './generated/LoginFinish';
export type { LoginStart } from './generated/LoginStart';
export type { Role } from './generated/Role';
export type { SessionKind } from './generated/SessionKind';
export type { User } from './generated/User';
export type { UserId } from './generated/UserId';
export type { UserUpdate } from './generated/UserUpdate';
export type { CompanyAccess } from './generated/CompanyAccess';
export type { PasskeyInfo } from './generated/PasskeyInfo';
export type { SessionInfo } from './generated/SessionInfo';
export type { DeviceKeyInfo } from './generated/DeviceKeyInfo';
export type { EnrolmentInfo } from './generated/EnrolmentInfo';
export type { EnrolmentPurpose } from './generated/EnrolmentPurpose';
export type { NewEnrolment } from './generated/NewEnrolment';
/** `POST /auth/enrollments`: the minted token plus the link to share (shown once, never stored). */
export type { MintedEnrolmentLink } from './generated/MintedEnrolmentLink';
/** `manifest.json` inside a `.lbk`, and `from` on `POST /companies/import`. */
export type { PortableManifest } from './generated/PortableManifest';
export type { Application } from './generated/Application';
export type { AttachmentMeta } from './generated/AttachmentMeta';
export type { Class } from './generated/Class';
export type { ClassId } from './generated/ClassId';
export type { ClearedStatus } from './generated/ClearedStatus';
export type { CompanyId } from './generated/CompanyId';
export type { DateMacro } from './generated/DateMacro';
export type { DateRange } from './generated/DateRange';
export type { Entity } from './generated/Entity';
export type { EntityId } from './generated/EntityId';
export type { EntityKind } from './generated/EntityKind';
export type { Item } from './generated/Item';
export type { ItemId } from './generated/ItemId';
export type { ItemType } from './generated/ItemType';
export type { LineKind } from './generated/LineKind';
export type { Money } from './generated/Money';
export type { PaymentMethod } from './generated/PaymentMethod';
export type { Posting } from './generated/Posting';
export type { PostingError } from './generated/PostingError';
export type { RegisterLine } from './generated/RegisterLine';
export type { SalesRep } from './generated/SalesRep';
export type { SalesTaxCode } from './generated/SalesTaxCode';
export type { Terms } from './generated/Terms';
export type { TermsKind } from './generated/TermsKind';
export type { Txn } from './generated/Txn';
export type { TxnFilter } from './generated/TxnFilter';
export type { TxnId } from './generated/TxnId';
export type { TxnLine } from './generated/TxnLine';
export type { TxnSummary } from './generated/TxnSummary';
export type { TxnType } from './generated/TxnType';
export type { TypeEntry } from './generated/TypeEntry';
export type { NameBalance } from './generated/NameBalance';
export type { Reconciliation } from './generated/Reconciliation';
export type { ReconciliationId } from './generated/ReconciliationId';
export type { ReconcileLine } from './generated/ReconcileLine';
export type { ReconcileStatus } from './generated/ReconcileStatus';
export type { StatementCharge } from './generated/StatementCharge';
export type { Saved } from './generated/Saved';
export type { Memorized } from './generated/Memorized';
export type { Repeat } from './generated/Repeat';
export type { Interval } from './generated/Interval';

// Reports (lb-reports).
export type { Report } from './generated/Report';
export type { ReportRequest } from './generated/ReportRequest';
export type { ReportParams } from './generated/ReportParams';
export type { ReportFilters } from './generated/ReportFilters';
export type { ReportCategory } from './generated/ReportCategory';
export type { ReportId } from './generated/ReportId';
export type { Row } from './generated/Row';
export type { RowKind } from './generated/RowKind';
export type { Column } from './generated/Column';
export type { ColumnKind } from './generated/ColumnKind';
export type { ColumnLayout } from './generated/ColumnLayout';
export type { Cell } from './generated/Cell';
export type { Value } from './generated/Value';
export type { Drill } from './generated/Drill';
export type { CatalogEntry } from './generated/CatalogEntry';
export type { DatesKind } from './generated/DatesKind';
export type { Basis } from './generated/Basis';
export type { Align } from './generated/Align';
export type { SortBy } from './generated/SortBy';
export type { FilterKind } from './generated/FilterKind';
export type { ClearedFilter } from './generated/ClearedFilter';
export type { ExportFormat } from './generated/ExportFormat';

// Bank feeds (lb-feeds).
export type { FeedAccountSummary } from './generated/FeedAccountSummary';
export type { FeedConnection } from './generated/FeedConnection';
export type { FeedConnectionId } from './generated/FeedConnectionId';
export type { FeedFilter } from './generated/FeedFilter';
export type { FeedRule } from './generated/FeedRule';
export type { FeedRuleId } from './generated/FeedRuleId';
export type { FeedStatus } from './generated/FeedStatus';
export type { FeedTxn } from './generated/FeedTxn';
export type { FeedTxnId } from './generated/FeedTxnId';
export type { Condition } from './generated/Condition';
export type { RuleActions } from './generated/RuleActions';
export type { TextField } from './generated/TextField';
export type { TextOp } from './generated/TextOp';
export type { MatchCandidate } from './generated/MatchCandidate';
export type { AddRequest } from './generated/AddRequest';
export type { AddOutcome } from './generated/AddOutcome';
export type { AddFailure } from './generated/AddFailure';
export type { BatchOutcome } from './generated/BatchOutcome';
export type { Suggestion } from './generated/Suggestion';
export type { CsvColumn } from './generated/CsvColumn';
export type { CsvDetection } from './generated/CsvDetection';
export type { CsvLayout } from './generated/CsvLayout';
export type { CsvMapping } from './generated/CsvMapping';
export type { AmountColumns } from './generated/AmountColumns';
export type { DateOrder } from './generated/DateOrder';
export type { Direction } from './generated/Direction';
export type { FileKind } from './generated/FileKind';
export type { IngestReport } from './generated/IngestReport';
export type { ImportSummary } from './generated/ImportSummary';
export type { Statement } from './generated/Statement';
export type { StatementAccount } from './generated/StatementAccount';
export type { StatementTxn } from './generated/StatementTxn';
export type { ProviderSettings } from './generated/ProviderSettings';
export type { OfxDirectSettings } from './generated/OfxDirectSettings';
export type { OfxVersion } from './generated/OfxVersion';
export type { TrnType } from './generated/TrnType';
export type { AmazonMatch } from './generated/AmazonMatch';
export type { AmazonOrder } from './generated/AmazonOrder';
export type { AmazonItem } from './generated/AmazonItem';

// QuickBooks import (lb-qbd).
export type { SyncReport } from './generated/SyncReport';
export type { SyncIssue } from './generated/SyncIssue';
export type { SyncProgress } from './generated/SyncProgress';
export type { SyncPhase } from './generated/SyncPhase';
export type { EntityReport } from './generated/EntityReport';
export type { Mismatch } from './generated/Mismatch';
export type { Gap } from './generated/Gap';
export type { ParityReport } from './generated/ParityReport';
export type { ParityRow } from './generated/ParityRow';
export type { ParitySummary } from './generated/ParitySummary';
export type { TrialBalanceParity } from './generated/TrialBalanceParity';
export type { QbdEntity } from './generated/QbdEntity';
export type { Severity } from './generated/Severity';
export type { ImportSettings } from './generated/ImportSettings';
export type { QbdTransport } from './generated/QbdTransport';
export type { BridgeSettings } from './generated/BridgeSettings';
export type { QbwcSettings } from './generated/QbwcSettings';
export type { SdkSettings } from './generated/SdkSettings';
export type { FileMode } from './generated/FileMode';
// The server's envelopes around them (lb-server): the status document
// (`GET /import/qbd` and `…/status`), a running job, and the request bodies.
// Both bodies are `#[serde(default)]` on the server, so the client sends a
// `Partial<SyncBody>` / `Partial<ParityBody>`.
export type { ImportStatus } from './generated/ImportStatus';
export type { RunningJob } from './generated/RunningJob';
export type { JobKind } from './generated/JobKind';
export type { SyncBody } from './generated/SyncBody';
export type { ParityBody } from './generated/ParityBody';
export type { ParityStatus } from './generated/ParityStatus';

// ---------------------------------------------------------------------------
// Server-level

export interface Health {
  ok: true;
  version: string;
  mode: 'server' | 'desktop';
}

export interface CompanySummary {
  id: CompanyId;
  name: string;
  role: Role;
}

/**
 * `GET /session`. `kind` says how this session signed in (passkey, device key,
 * or the desktop app's local token); `sign_in` is true when the server uses
 * sign-in at all (so "Sign out" means something).
 */
export interface Session {
  user: { id: string | null; display_name: string; role: Role; kind?: SessionKind | null };
  companies: CompanySummary[];
  sign_in?: boolean;
}

/** `POST /companies/import` → 201: a NEW company made from the file; nothing existing is replaced. */
export interface ImportedCompany {
  id: CompanyId;
  name: string;
  from: PortableManifest;
}

/** One `event: change` from `/events?company=` (`data: {kind, id, action}`). */
export interface ChangeEvent {
  /** The record kind: `txn`, `account`, `entity`, `item`, `class`, … */
  kind: string;
  id: string;
  /** `created`, `updated`, `deleted`, `voided`, … */
  action: string;
}

/** `GET /users/:id`: the user with their company access, passkeys and devices. */
export interface UserDetail {
  user: User;
  companies: CompanyAccess[];
  passkeys: PasskeyInfo[];
  devices: DeviceKeyInfo[];
}

// ---------------------------------------------------------------------------
// Server administration (plans/server-ops.md). Settings shapes mirror
// `ServerSettings` and its parts; not generated (the ops module has no ts-rs).

export interface BackupSchedule {
  enabled: boolean;
  /** ISO 8601 duration (`PT6H`), or null for `daily_at`. */
  every: string | null;
  /** `HH:MM:SS` local time. */
  daily_at: string | null;
}

export type BackupDestination =
  | { Local: { dir: string } }
  | { S3: { endpoint: string; region: string; bucket: string; prefix: string; access_key_id: string; secret_access_key_ref: string; path_style: boolean } };

export type BackupEncryption = 'None' | { Recipients: { recipients: string[] } } | { Passphrase: { passphrase_ref: string; work_factor?: number | null } };

export interface BackupRetention {
  keep_last: number;
  daily: number;
  weekly: number;
  monthly: number;
  yearly: number;
}

export interface BackupConfig {
  schedule: BackupSchedule;
  destinations: BackupDestination[];
  encryption: BackupEncryption;
  retention: BackupRetention;
  compression_level: number;
  time_zone: string | null;
  work_dir: string | null;
}

export interface UpdateConfig {
  check: boolean;
  auto_install: boolean;
  channel: 'Stable' | 'Prerelease';
  /** ISO 8601 duration. */
  interval: string;
}

export interface TunnelConfig {
  enabled: boolean;
  token_ref: string;
  public_hostname: string | null;
  cloudflared_path: string | null;
  auto_download: boolean;
}

export interface AccessConfig {
  team_domain: string;
  audience: string[];
}

export interface FeedSyncSettings {
  auto_sync: boolean;
  every: string;
}

export interface ServerSettings {
  public_urls: string[];
  backups: BackupConfig;
  updates: UpdateConfig;
  tunnel: TunnelConfig;
  access: AccessConfig | null;
  feeds: FeedSyncSettings;
}

export interface SecretInfo {
  name: string;
  updated_at: string;
}

export interface UpdateAvailable {
  version: string;
  tag: string;
  page: string;
  asset_name: string;
  asset_url: string;
}

export interface UpdateStatus {
  current: string;
  can_install: boolean;
  install_kind: Record<string, unknown> | string;
  advice: string | null;
  last_check: string | null;
  available: UpdateAvailable | null;
  phase: { Idle?: unknown } | string | Record<string, unknown>;
}

export interface UpdateView {
  config: UpdateConfig;
  status: UpdateStatus | null;
  /** Why there is no updater (no release key in this build…). */
  unavailable: string | null;
}

export type TunnelState =
  | { state: 'stopped' }
  | { state: 'starting' }
  | { state: 'connected'; connections: number }
  | { state: 'reconnecting' }
  | { state: 'failed'; reason: string };

export interface TunnelStatus {
  state: TunnelState;
  last_log_lines: string[];
  version: string | null;
  restarts: number;
  tunnel_id: string | null;
  public_url: string | null;
}

export interface TunnelView {
  enabled: boolean;
  public_hostname: string | null;
  public_url: string | null;
  token_ref: string;
  token_set: boolean;
  tunnel_id: string | null;
  cloudflared_path: string | null;
  auto_download: boolean;
  cloudflared: { located: { path: string; origin: string }; version: string } | null;
  preparing_since: string | null;
  status: TunnelStatus;
  problems: string[];
  restart_required: string[];
}

export interface TunnelUpdate {
  enabled?: boolean;
  public_hostname?: string;
  /** Write-only; saved as the secret `tunnel-token`. */
  token?: string;
  cloudflared_path?: string;
  auto_download?: boolean;
}

export interface BackupManifest {
  format: number;
  app_version: string;
  company_name: string;
  company_slug: string;
  schema_version: number;
  created_at: string;
  id: string;
  artifact: string;
  database_size: number;
  database_sha256: string;
  stored_size: number;
  stored_sha256: string;
  compression: string;
  encryption: string;
}

export interface BackupRecord {
  manifest: BackupManifest;
  destinations: Array<{ destination: string; stored: boolean; error: string | null; pruned: string[] }>;
}

export interface SchedulerStatus {
  running: boolean;
  next_run: string | null;
  last_success_at: string | null;
  last: { started_at: string; finished_at: string; record: BackupRecord | null; error: string | null } | null;
}

export interface CompanyBackupStatus {
  company_id: CompanyId;
  company_name: string;
  status: SchedulerStatus;
}

export interface ServerView {
  version: string;
  data_dir: string;
  settings: ServerSettings;
  secrets: SecretInfo[];
  update: UpdateView;
  tunnel: TunnelView;
  backups: CompanyBackupStatus[];
  restart_required: string[];
  running: boolean;
}

export interface BackupList {
  backups: Array<{ destination: number; destination_name: string; manifest: BackupManifest }>;
  errors: string[];
}

export interface CompanyBackupsStatus {
  enabled: boolean;
  destinations: string[];
  scheduler: SchedulerStatus | null;
}

export interface VerifyReport {
  checksum_ok: boolean;
  contents_checked: boolean;
  problems: string[];
}

export interface UnlockRequest {
  identities?: string[];
  passphrase?: string;
}

export interface Restored {
  id: CompanyId;
  name: string;
  from: BackupManifest;
}

/** `POST /backups/:id/restore` body: the new company's name plus how to unlock the backup. */
export interface RestoreRequest extends UnlockRequest {
  name?: string;
}

/** `POST /server/update/install` → what was installed and whether the server is restarting into it. */
export interface UpdateInstalled {
  installed: unknown;
  restarting: boolean;
}

/** `GET /server/backups/status`: every company's scheduler at a glance. */
export interface ServerBackupsStatus {
  enabled: boolean;
  running: boolean;
  companies: CompanyBackupStatus[];
}

// ---------------------------------------------------------------------------
// Company

export type { CompanyInfo } from './generated/CompanyInfo';
export type { Preferences } from './generated/Preferences';

// ---------------------------------------------------------------------------
// Lists

/** `:list` path segments and the record each holds. */
export interface ListRecords {
  accounts: Account;
  entities: Entity;
  items: Item;
  classes: Class;
  terms: Terms;
  'payment-methods': PaymentMethod;
  'sales-tax-codes': SalesTaxCode;
  'sales-reps': SalesRep;
  types: TypeEntry;
}

export type ListName = keyof ListRecords;
export type ListRecord = ListRecords[ListName];

export interface ListQuery {
  include_inactive?: boolean;
  /** entities only */
  kind?: EntityKind;
  /** items only */
  type?: ItemType;
}

// ---------------------------------------------------------------------------
// Transactions

export interface TxnWithPostings {
  txn: Txn;
  postings: Posting[];
}

/** A save warning (e.g. "date is 120 days in the past"): shape not pinned yet. */
export type SaveWarning = string | { code?: string; message: string };

export interface PreviewOutcome {
  postings: Posting[];
  total: Money;
  sales_tax: Money;
  warnings: SaveWarning[];
}

export interface SaveOutcome extends PreviewOutcome {
  txn: Txn;
}

export interface RegisterResponse {
  opening: Money;
  lines: RegisterLine[];
}

export interface AuditEntry {
  id: string;
  at: string;
  user: string | null;
  action: string;
  record_kind: string;
  record_id: string;
  before: unknown;
  after: unknown;
}

export function warningText(w: SaveWarning): string {
  return typeof w === 'string' ? w : w.message;
}

// ---------------------------------------------------------------------------
// Saved documents (memorized transactions and reports)

/** The saved kinds the API accepts (`/saved/:kind`). */
export type SavedKind = 'memorized_txn' | 'memorized_report' | 'view';

/** `GET /memorized/due`: memorized transactions due on or before today. */
export interface MemorizedDue {
  id: string;
  name: string;
  due: string;
  repeat: Repeat;
}

// ---------------------------------------------------------------------------
// Reconciliation (`/accounts/:id/reconcile`, `/reconciliations/:id`)

export interface ReconcileOverview {
  in_progress: Reconciliation | null;
  /** Finished reconciliations, newest first. */
  history: Reconciliation[];
  /** What everything already reconciled adds up to, natural sign. */
  beginning_balance: Money;
}

export interface StartReconcile {
  statement_date: string;
  ending_balance: Money;
  service_charge: StatementCharge | null;
  interest: StatementCharge | null;
}

export interface ReconciliationView {
  reconciliation: Reconciliation;
  lines: ReconcileLine[];
  status: ReconcileStatus;
}

// ---------------------------------------------------------------------------
// Attachments

/**
 * `GET /attachments`: the inbox by default; `record_kind` + `record_id` for one
 * record's files; `all` for every document, attached or not (the Attached tab).
 */
export interface AttachmentQuery {
  record_kind?: string;
  record_id?: string;
  all?: boolean;
}

export type AttachmentRecord = { record_kind: string; record_id: string } | null;

// ---------------------------------------------------------------------------
// Merge (`POST /merge`): QBD's rename-to-merge, as an explicit action.

export type MergeKind = 'account' | 'entity' | 'item';

export interface MergeRequest {
  kind: MergeKind;
  /** The record that disappears; everything that pointed at it points at `into`. */
  from: string;
  into: string;
}

// ---------------------------------------------------------------------------
// Bank feeds (request shapes not in the generated types)

export type FeedQuery = Partial<FeedFilter>;

export interface FeedDetection {
  kind: FileKind;
  csv?: CsvDetection;
  statements?: Statement[];
}

export interface FeedImportRequest {
  file: File;
  account_id?: string | null;
  connection_id?: string | null;
  mapping?: CsvMapping | null;
  /** Which statement of a multi-account file. */
  statement?: number | null;
}

/** `POST /feeds/txns/:id/preview`: the transaction "Add" would make, with its postings. */
export interface FeedAddPreview {
  txn: Txn;
  postings: Posting[];
  total: Money;
  sales_tax: Money;
}

export interface SimpleFinClaim {
  access: string;
  accounts: StatementAccount[];
  errors: string[];
}

export interface AmazonPreview {
  matches: AmazonMatch[];
  orders: AmazonOrder[];
}
