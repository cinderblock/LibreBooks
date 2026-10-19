// LOCAL STAND-IN for the report shapes `lb-reports` will export through ts-rs
// (`Report`, `ReportRequest`). Swap these for the generated types when they land
// in `web/src/api/generated/`, then fix whatever the compiler flags; keep the
// semantics described here (they are what the ReportViewer relies on).
//
// A report is a grid: columns, then flat rows each carrying its `depth` in the
// outline. A `section` row opens a group that the next `subtotal` row at the same
// depth closes; collapsing the section hides everything between and shows the
// subtotal's figures on the section's own line (as QBD's "Collapse" does).

import type { AccountId } from '~/api/generated/AccountId';
import type { DateMacro } from '~/api/generated/DateMacro';
import type { DateRange } from '~/api/generated/DateRange';
import type { EntityId } from '~/api/generated/EntityId';
import type { TxnId } from '~/api/generated/TxnId';
import type { TxnType } from '~/api/generated/TxnType';

export type ReportBasis = 'Accrual' | 'Cash';

export interface ReportRequest {
  /** Catalog id: `profit_and_loss`, `balance_sheet`, `account_quickreport`, … */
  report: string;
  /** A date macro wins over `range` when both are given. */
  macro?: DateMacro | null;
  range?: DateRange | null;
  basis?: ReportBasis | null;
  account_id?: AccountId | null;
  entity_id?: EntityId | null;
}

export type ReportColumnKind = 'text' | 'date' | 'money' | 'quantity' | 'percent';

export interface ReportColumn {
  id: string;
  label: string;
  kind: ReportColumnKind;
}

/** Money cells are cents; text/date cells strings; empty cells null. */
export type ReportCell = string | number | null;

export type ReportRowKind = 'section' | 'data' | 'subtotal' | 'total' | 'text' | 'blank';

/** QuickZoom: what double-clicking a row opens. */
export type ReportDrill =
  | { kind: 'txn'; txn_id: TxnId; txn_type?: TxnType }
  | { kind: 'report'; request: ReportRequest }
  | { kind: 'register'; account_id: AccountId };

export interface ReportRow {
  id: string;
  kind: ReportRowKind;
  depth: number;
  cells: ReportCell[];
  drill?: ReportDrill | null;
}

export interface Report {
  title: string;
  subtitle?: string | null;
  company_name: string;
  basis: ReportBasis;
  range: DateRange;
  columns: ReportColumn[];
  rows: ReportRow[];
  generated_at: string;
  request: ReportRequest;
}

export interface ReportCatalogEntry {
  id: string;
  name: string;
  category: string;
  /** Which request fields the report takes. */
  params: Array<'range' | 'basis' | 'account' | 'entity'>;
  default_macro: DateMacro;
}
