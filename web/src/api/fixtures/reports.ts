// FIXTURE ONLY: report builders over the fixture books, producing the local
// `Report` shape (`~/reports/types`) so the ReportViewer has real, drillable data
// before `lb-reports` exists. Accrual basis only (a Cash request is labelled but
// computed accrual); the server's report engine is the real thing.

import type { Report, ReportCatalogEntry, ReportDrill, ReportRequest, ReportRow } from '~/reports/types';
import { isBalanceSheet, naturalBalance } from '~/lib/accountTypes';
import { fiscalYear, formatDate, macroRange } from '~/lib/dates';
import { TXN_LABEL } from '~/lib/txnTypes';
import { ApiError } from '../errors';
import type { Account, AccountType, CompanyInfo, DateRange, Entity } from '../types';
import type { StoredTxn } from './client';

export interface ReportBooks {
  info: CompanyInfo;
  /** In chart-of-accounts order. */
  accounts: Account[];
  entities: Entity[];
  txns: StoredTxn[];
  today: string;
}

export const FIXTURE_REPORTS: ReportCatalogEntry[] = [
  { id: 'profit_and_loss', name: 'Profit & Loss Standard', category: 'Company & Financial', params: ['range', 'basis'], default_macro: 'ThisMonthToDate' },
  { id: 'balance_sheet', name: 'Balance Sheet Standard', category: 'Company & Financial', params: ['range', 'basis'], default_macro: 'ThisFiscalYearToDate' },
  { id: 'trial_balance', name: 'Trial Balance', category: 'Accountant & Taxes', params: ['range', 'basis'], default_macro: 'ThisFiscalYearToDate' },
  { id: 'transaction_list_by_date', name: 'Transaction List by Date', category: 'Accountant & Taxes', params: ['range', 'entity'], default_macro: 'ThisMonthToDate' },
  { id: 'account_quickreport', name: 'Account QuickReport', category: 'Accountant & Taxes', params: ['range', 'account'], default_macro: 'ThisFiscalYearToDate' },
  { id: 'customer_balance_summary', name: 'Customer Balance Summary', category: 'Customers & Receivables', params: ['range'], default_macro: 'All' },
];

class RowBuilder {
  rows: ReportRow[] = [];
  private n = 0;
  push(kind: ReportRow['kind'], depth: number, cells: ReportRow['cells'], drill?: ReportDrill | null): ReportRow {
    const row: ReportRow = { id: `r${this.n++}`, kind, depth, cells, drill: drill ?? null };
    this.rows.push(row);
    return row;
  }
}

function resolveRange(req: ReportRequest, books: ReportBooks): DateRange {
  const entry = FIXTURE_REPORTS.find((r) => r.id === req.report);
  const fsm = books.info.fiscal_year_start_month;
  if (req.macro) return macroRange(req.macro, books.today, fsm);
  if (req.range) return req.range;
  return macroRange(entry?.default_macro ?? 'ThisMonthToDate', books.today, fsm);
}

/** Debit-positive sums per account over postings matching `inRange`. */
function sums(books: ReportBooks, inRange: (date: string) => boolean): Map<string, number> {
  const m = new Map<string, number>();
  for (const s of books.txns) {
    if (!inRange(s.txn.date)) continue;
    for (const p of s.postings) m.set(p.account_id, (m.get(p.account_id) ?? 0) + p.amount);
  }
  return m;
}

/**
 * Emit the account tree for `accounts` (one type group) as report rows: parents
 * with nonzero children become sections closed by "Total X"; leaves are data rows
 * drilling into the account's QuickReport. Zero branches are left out (QBD's
 * default). Returns the group's total (natural sign).
 */
function accountTree(
  b: RowBuilder,
  accounts: Account[],
  own: Map<string, number>,
  depth: number,
  drill: (a: Account) => ReportDrill,
): number {
  const set = new Set(accounts.map((a) => a.id));
  const kids = (id: string | null) => accounts.filter((a) => (id ? a.parent_id === id : !a.parent_id || !set.has(a.parent_id)));
  const totalOf = (a: Account): number => (own.get(a.id) ?? 0) + kids(a.id).reduce((s, c) => s + totalOf(c), 0);
  const hasActivity = (a: Account): boolean => (own.get(a.id) ?? 0) !== 0 || kids(a.id).some(hasActivity);
  const emit = (a: Account, d: number): number => {
    if (!hasActivity(a)) return 0;
    const children = kids(a.id).filter(hasActivity);
    const total = totalOf(a);
    if (children.length === 0) {
      b.push('data', d, [a.name, total], drill(a));
      return total;
    }
    b.push('section', d, [a.name, null]);
    const mine = own.get(a.id) ?? 0;
    if (mine) b.push('data', d + 1, [`${a.name} - Other`, mine], drill(a));
    for (const c of children) emit(c, d + 1);
    b.push('subtotal', d, [`Total ${a.name}`, total]);
    return total;
  };
  return kids(null).reduce((s, a) => s + emit(a, depth), 0);
}

function group(
  b: RowBuilder,
  label: string,
  depth: number,
  accounts: Account[],
  own: Map<string, number>,
  drill: (a: Account) => ReportDrill,
): number {
  if (!accounts.some((a) => own.get(a.id))) return 0;
  b.push('section', depth, [label, null]);
  const total = accountTree(b, accounts, own, depth + 1, drill);
  b.push('subtotal', depth, [`Total ${label}`, total]);
  return total;
}

function header(books: ReportBooks, req: ReportRequest, title: string, range: DateRange, columns: Report['columns'], rows: ReportRow[], subtitle?: string): Report {
  return {
    title,
    subtitle: subtitle ?? (range.from || range.to ? `${range.from ? formatDate(range.from) : 'All'} through ${range.to ? formatDate(range.to) : 'today'}` : 'All Transactions'),
    company_name: books.info.name,
    basis: req.basis ?? 'Accrual',
    range,
    columns,
    rows,
    generated_at: new Date().toISOString(),
    request: req,
  };
}

export function runFixtureReport(books: ReportBooks, req: ReportRequest): Report {
  const range = resolveRange(req, books);
  const inRange = (d: string) => (!range.from || d >= range.from) && (!range.to || d <= range.to);
  const quick = (a: Account): ReportDrill => ({
    kind: 'report',
    request: { report: 'account_quickreport', account_id: a.id, range, basis: req.basis ?? null },
  });
  const ofTypes = (...types: AccountType[]) => books.accounts.filter((a) => types.includes(a.account_type));
  const naturalMap = (raw: Map<string, number>) =>
    new Map([...raw].map(([id, v]) => [id, naturalBalance(books.accounts.find((a) => a.id === id)?.account_type ?? 'Expense', v)]));

  switch (req.report) {
    case 'profit_and_loss': {
      const own = naturalMap(sums(books, inRange));
      const b = new RowBuilder();
      b.push('section', 0, ['Ordinary Income/Expense', null]);
      const income = group(b, 'Income', 1, ofTypes('Income'), own, quick);
      const cogs = group(b, 'Cost of Goods Sold', 1, ofTypes('CostOfGoodsSold'), own, quick);
      b.push('total', 1, ['Gross Profit', income - cogs]);
      const expense = group(b, 'Expense', 1, ofTypes('Expense'), own, quick);
      const ordinary = income - cogs - expense;
      b.push('subtotal', 0, ['Net Ordinary Income', ordinary]);
      let other = 0;
      if (ofTypes('OtherIncome', 'OtherExpense').some((a) => own.get(a.id))) {
        b.push('section', 0, ['Other Income/Expense', null]);
        const oi = group(b, 'Other Income', 1, ofTypes('OtherIncome'), own, quick);
        const oe = group(b, 'Other Expense', 1, ofTypes('OtherExpense'), own, quick);
        other = oi - oe;
        b.push('subtotal', 0, ['Net Other Income', other]);
      }
      b.push('total', 0, ['Net Income', ordinary + other]);
      return header(books, req, 'Profit & Loss', range, [
        { id: 'label', label: '', kind: 'text' },
        { id: 'amount', label: 'Amount', kind: 'money' },
      ], b.rows);
    }

    case 'balance_sheet':
    case 'trial_balance': {
      const asOf = range.to ?? books.today;
      const [fyStart] = fiscalYear(asOf, books.info.fiscal_year_start_month, 0);
      const cumulative = sums(books, (d) => d <= asOf);
      const ytd = sums(books, (d) => d >= fyStart && d <= asOf);
      const bsOwn = new Map<string, number>();
      let priorIncome = 0; // debit-positive, P&L accounts before this fiscal year
      let ytdIncome = 0;
      for (const a of books.accounts) {
        const c = cumulative.get(a.id) ?? 0;
        if (isBalanceSheet(a.account_type)) bsOwn.set(a.id, c);
        else {
          const y = ytd.get(a.id) ?? 0;
          ytdIncome += y;
          priorIncome += c - y;
        }
      }
      const re = books.accounts.find((a) => a.role === 'RetainedEarnings');
      if (re) bsOwn.set(re.id, (bsOwn.get(re.id) ?? 0) + priorIncome);

      if (req.report === 'trial_balance') {
        const b = new RowBuilder();
        let dr = 0;
        let cr = 0;
        for (const a of books.accounts) {
          const v = isBalanceSheet(a.account_type) ? (bsOwn.get(a.id) ?? 0) : (ytd.get(a.id) ?? 0);
          if (!v) continue;
          if (v > 0) dr += v;
          else cr -= v;
          b.push('data', 0, [a.full_name, v > 0 ? v : null, v < 0 ? -v : null], quick(a));
        }
        b.push('total', 0, ['TOTAL', dr, cr]);
        return header(books, req, 'Trial Balance', { from: null, to: asOf }, [
          { id: 'label', label: '', kind: 'text' },
          { id: 'debit', label: 'Debit', kind: 'money' },
          { id: 'credit', label: 'Credit', kind: 'money' },
        ], b.rows, `As of ${formatDate(asOf)}`);
      }

      const own = naturalMap(bsOwn);
      const b = new RowBuilder();
      b.push('section', 0, ['ASSETS', null]);
      b.push('section', 1, ['Current Assets', null]);
      const bank = group(b, 'Checking/Savings', 2, ofTypes('Bank'), own, quick);
      const ar = group(b, 'Accounts Receivable', 2, ofTypes('AccountsReceivable'), own, quick);
      const oca = group(b, 'Other Current Assets', 2, ofTypes('OtherCurrentAsset'), own, quick);
      b.push('subtotal', 1, ['Total Current Assets', bank + ar + oca]);
      const fixed = group(b, 'Fixed Assets', 1, ofTypes('FixedAsset'), own, quick);
      const otherA = group(b, 'Other Assets', 1, ofTypes('OtherAsset'), own, quick);
      const assets = bank + ar + oca + fixed + otherA;
      b.push('subtotal', 0, ['TOTAL ASSETS', assets]);
      b.push('section', 0, ['LIABILITIES & EQUITY', null]);
      b.push('section', 1, ['Liabilities', null]);
      b.push('section', 2, ['Current Liabilities', null]);
      const ap = group(b, 'Accounts Payable', 3, ofTypes('AccountsPayable'), own, quick);
      const cc = group(b, 'Credit Cards', 3, ofTypes('CreditCard'), own, quick);
      const ocl = group(b, 'Other Current Liabilities', 3, ofTypes('OtherCurrentLiability'), own, quick);
      b.push('subtotal', 2, ['Total Current Liabilities', ap + cc + ocl]);
      const ltl = group(b, 'Long Term Liabilities', 2, ofTypes('LongTermLiability'), own, quick);
      const liabilities = ap + cc + ocl + ltl;
      b.push('subtotal', 1, ['Total Liabilities', liabilities]);
      b.push('section', 1, ['Equity', null]);
      const equityAccounts = accountTree(b, ofTypes('Equity'), own, 2, quick);
      const netIncome = -ytdIncome; // credit-normal
      b.push('data', 2, ['Net Income', netIncome], {
        kind: 'report',
        request: { report: 'profit_and_loss', range: { from: fyStart, to: asOf }, basis: req.basis ?? null },
      });
      const equity = equityAccounts + netIncome;
      b.push('subtotal', 1, ['Total Equity', equity]);
      b.push('subtotal', 0, ['TOTAL LIABILITIES & EQUITY', liabilities + equity]);
      return header(books, req, 'Balance Sheet', { from: null, to: asOf }, [
        { id: 'label', label: '', kind: 'text' },
        { id: 'amount', label: formatDate(asOf), kind: 'money' },
      ], b.rows, `As of ${formatDate(asOf)}`);
    }

    case 'account_quickreport': {
      const acct = books.accounts.find((a) => a.id === req.account_id);
      if (!acct) throw new ApiError(400, 'invalid', 'Choose an account for the QuickReport', { field: 'account_id' });
      const family = new Set([acct.id]);
      for (const a of books.accounts) if (a.full_name.startsWith(`${acct.full_name}:`)) family.add(a.id);
      const names = new Map(books.entities.map((e) => [e.id, e.full_name]));
      const accountNames = new Map(books.accounts.map((a) => [a.id, a.full_name]));
      const bs = isBalanceSheet(acct.account_type);
      let balance = 0;
      const b = new RowBuilder();
      b.push('section', 0, [acct.full_name, null, null, null, null, null, null, null]);
      const rows = books.txns
        .filter((s) => s.postings.some((p) => family.has(p.account_id)))
        .sort((x, y) => (x.txn.date < y.txn.date ? -1 : x.txn.date > y.txn.date ? 1 : x.seq - y.seq));
      let total = 0;
      for (const s of rows) {
        const amt = naturalBalance(acct.account_type, s.postings.filter((p) => family.has(p.account_id)).reduce((t, p) => t + p.amount, 0));
        if (range.from && s.txn.date < range.from) {
          if (bs) balance += amt;
          continue;
        }
        if (range.to && s.txn.date > range.to) continue;
        balance += amt;
        total += amt;
        const others = [...new Set(s.postings.filter((p) => !family.has(p.account_id)).map((p) => p.account_id))];
        b.push(
          'data',
          1,
          [
            TXN_LABEL[s.txn.txn_type],
            s.txn.date,
            s.txn.number,
            s.txn.entity_id ? (names.get(s.txn.entity_id) ?? null) : null,
            s.txn.memo,
            others.length === 1 ? (accountNames.get(others[0]!) ?? null) : '-SPLIT-',
            amt,
            balance,
          ],
          { kind: 'txn', txn_id: s.txn.id, txn_type: s.txn.txn_type },
        );
      }
      b.push('subtotal', 0, [`Total ${acct.full_name}`, null, null, null, null, null, total, balance]);
      return header(books, req, `${acct.full_name} QuickReport`, range, [
        { id: 'type', label: 'Type', kind: 'text' },
        { id: 'date', label: 'Date', kind: 'date' },
        { id: 'num', label: 'Num', kind: 'text' },
        { id: 'name', label: 'Name', kind: 'text' },
        { id: 'memo', label: 'Memo', kind: 'text' },
        { id: 'split', label: 'Split', kind: 'text' },
        { id: 'amount', label: 'Amount', kind: 'money' },
        { id: 'balance', label: 'Balance', kind: 'money' },
      ], b.rows);
    }

    case 'transaction_list_by_date': {
      const names = new Map(books.entities.map((e) => [e.id, e.full_name]));
      const accountNames = new Map(books.accounts.map((a) => [a.id, a.full_name]));
      const family = new Set<string>();
      if (req.entity_id) {
        family.add(req.entity_id);
        for (const e of books.entities) if (e.parent_id && family.has(e.parent_id)) family.add(e.id);
      }
      const b = new RowBuilder();
      const rows = books.txns
        .filter((s) => inRange(s.txn.date) && (!req.entity_id || (s.txn.entity_id && family.has(s.txn.entity_id))))
        .sort((x, y) => (x.txn.date < y.txn.date ? -1 : x.txn.date > y.txn.date ? 1 : x.seq - y.seq));
      for (const s of rows) {
        const main = s.txn.account_id ?? s.postings.find((p) => !p.line_id)?.account_id ?? null;
        const others = [...new Set(s.postings.filter((p) => p.account_id !== main).map((p) => p.account_id))];
        b.push(
          'data',
          0,
          [
            TXN_LABEL[s.txn.txn_type],
            s.txn.date,
            s.txn.number,
            s.txn.entity_id ? (names.get(s.txn.entity_id) ?? null) : null,
            s.txn.memo,
            main ? (accountNames.get(main) ?? null) : null,
            others.length === 1 ? (accountNames.get(others[0]!) ?? null) : others.length ? '-SPLIT-' : null,
            s.total,
          ],
          { kind: 'txn', txn_id: s.txn.id, txn_type: s.txn.txn_type },
        );
      }
      const entityName = req.entity_id ? names.get(req.entity_id) : null;
      return header(books, req, entityName ? `Transactions: ${entityName}` : 'Transaction List by Date', range, [
        { id: 'type', label: 'Type', kind: 'text' },
        { id: 'date', label: 'Date', kind: 'date' },
        { id: 'num', label: 'Num', kind: 'text' },
        { id: 'name', label: 'Name', kind: 'text' },
        { id: 'memo', label: 'Memo', kind: 'text' },
        { id: 'account', label: 'Account', kind: 'text' },
        { id: 'split', label: 'Split', kind: 'text' },
        { id: 'amount', label: 'Amount', kind: 'money' },
      ], b.rows);
    }

    case 'customer_balance_summary': {
      const asOf = range.to ?? books.today;
      const ar = new Set(books.accounts.filter((a) => a.account_type === 'AccountsReceivable').map((a) => a.id));
      const own = new Map<string, number>();
      for (const s of books.txns) {
        if (s.txn.date > asOf) continue;
        for (const p of s.postings) if (ar.has(p.account_id) && p.entity_id) own.set(p.entity_id, (own.get(p.entity_id) ?? 0) + p.amount);
      }
      const customers = books.entities.filter((e) => e.kind === 'Customer');
      const b = new RowBuilder();
      const asAccounts = customers.map((c) => ({ ...c, account_type: 'AccountsReceivable' as const })) as unknown as Account[];
      const total = accountTree(b, asAccounts, own, 0, (c) => ({
        kind: 'report',
        request: { report: 'transaction_list_by_date', entity_id: c.id, macro: 'All' },
      }));
      b.push('total', 0, ['TOTAL', total]);
      return header(books, req, 'Customer Balance Summary', { from: null, to: asOf }, [
        { id: 'label', label: '', kind: 'text' },
        { id: 'amount', label: formatDate(asOf), kind: 'money' },
      ], b.rows, `As of ${formatDate(asOf)}`);
    }

    default:
      throw new ApiError(404, 'not_found', `No report "${req.report}"`);
  }
}
