// API shapes from plans/api.md that ts-rs does not generate (yet).
//
// Everything that *is* generated is imported from ./generated and re-exported
// here so screens have one import site. When the server crate starts exporting
// one of the local stand-ins below (SaveOutcome, the session shape…), delete the local definition and re-export the generated one.

import type { Account } from './generated/Account';
import type { Class } from './generated/Class';
import type { CompanyId } from './generated/CompanyId';
import type { Entity } from './generated/Entity';
import type { EntityKind } from './generated/EntityKind';
import type { Item } from './generated/Item';
import type { ItemType } from './generated/ItemType';
import type { Money } from './generated/Money';
import type { PaymentMethod } from './generated/PaymentMethod';
import type { Posting } from './generated/Posting';
import type { RegisterLine } from './generated/RegisterLine';
import type { SalesRep } from './generated/SalesRep';
import type { SalesTaxCode } from './generated/SalesTaxCode';
import type { Terms } from './generated/Terms';
import type { Txn } from './generated/Txn';
import type { TypeEntry } from './generated/TypeEntry';
import type { Role } from './generated/Role';
import type { SessionKind } from './generated/SessionKind';

export type { Account } from './generated/Account';
export type { AccountBalance } from './generated/AccountBalance';
export type { AccountId } from './generated/AccountId';
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
export type { Application } from './generated/Application';
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
export type { Txn } from './generated/Txn';
export type { TxnFilter } from './generated/TxnFilter';
export type { TxnId } from './generated/TxnId';
export type { TxnLine } from './generated/TxnLine';
export type { TxnSummary } from './generated/TxnSummary';
export type { TxnType } from './generated/TxnType';
export type { TypeEntry } from './generated/TypeEntry';

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
  user: { id: string | null; display_name: string; role: Role; kind?: SessionKind };
  companies: CompanySummary[];
  sign_in?: boolean;
}

/** One `event: change` from `/events?company=` (`data: {kind, id, action}`). */
export interface ChangeEvent {
  /** The record kind: `txn`, `account`, `entity`, `item`, `class`, … */
  kind: string;
  id: string;
  /** `created`, `updated`, `deleted`, `voided`, … */
  action: string;
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

/** One open invoice/bill/credit for Receive Payments / Pay Bills. */
export interface OpenTxn {
  txn_id: string;
  txn_type: Txn['txn_type'];
  date: string;
  number: string | null;
  due_date: string | null;
  amount: Money;
  open_balance: Money;
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
