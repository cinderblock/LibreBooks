import { describe, expect, test } from 'bun:test';
import { defaultRequest } from '~/reports/request';
import { blankLine, blankTxn } from '../blank';
import { ApiError } from '../errors';
import type { Cell, Memorized, Row } from '../types';
import { FIXTURE_COMPANY_ID, FixtureApiClient } from './client';

const co = FIXTURE_COMPANY_ID;
const today = '2026-10-19';
const make = () => new FixtureApiClient({ latencyMs: 0, today });

async function rejects(p: Promise<unknown>): Promise<ApiError> {
  try {
    await p;
  } catch (e) {
    expect(e).toBeInstanceOf(ApiError);
    return e as ApiError;
  }
  throw new Error('expected a rejection');
}

describe('fixture books', () => {
  test('every transaction balances and the trial balance nets to zero', async () => {
    const api = make();
    const balances = await api.balances(co);
    expect(balances.reduce((s, b) => s + b.debit_positive, 0)).toBe(0);
    const txns = await api.txns(co, {});
    expect(txns.length).toBeGreaterThan(40);
  });

  test('the balance sheet balances', async () => {
    const api = make();
    const r = await api.runReport(co, { ...defaultRequest('BalanceSheet'), date_macro: 'ThisFiscalYearToDate' });
    const total = (label: string) => money(r.rows.find((row) => label === textOf(row))?.cells[1]);
    expect(total('TOTAL ASSETS')).toBe(total('TOTAL LIABILITIES & EQUITY'));
    expect(total('TOTAL ASSETS')).toBeGreaterThan(0);
    expect(r.basis).toBe('Accrual');
    expect(r.subtitle.startsWith('As of ')).toBe(true);
  });

  test('summary options: monthly columns add a TOTAL, comparisons add change columns', async () => {
    const api = make();
    const monthly = await api.runReport(co, { ...defaultRequest('ProfitAndLoss'), date_macro: 'ThisFiscalYearToDate', columns: 'Month', percent_of_income: true });
    expect(monthly.columns.at(-2)?.label).toBe('TOTAL');
    expect(monthly.columns.at(-1)?.label).toBe('% of Income');
    const net = monthly.rows.find((row) => textOf(row) === 'Net Income')!;
    const months = monthly.columns.filter((c) => c.kind === 'Money' && c.label !== 'TOTAL').length;
    const sum = net.cells.filter((c, i) => monthly.columns[i]!.kind === 'Money' && monthly.columns[i]!.label !== 'TOTAL').reduce((t, c) => t + money(c), 0);
    expect(months).toBeGreaterThan(1);
    expect(money(net.cells[monthly.columns.findIndex((c) => c.label === 'TOTAL')])).toBe(sum);
    const compared = await api.runReport(co, { ...defaultRequest('ProfitAndLoss'), date_macro: 'ThisMonth', previous_period: true, dollar_change: true, percent_change: true });
    expect(compared.columns.map((c) => c.label).slice(-2)).toEqual(['$ Change', '% Change']);
  });

  test('the catalog lists every report once with its options', async () => {
    const api = make();
    const c = await api.reportCatalog(co);
    expect(c.length).toBe(46);
    expect(new Set(c.map((e) => e.id)).size).toBe(46);
    const pl = c.find((e) => e.id === 'ProfitAndLoss')!;
    expect(pl.params.basis && pl.params.comparisons && pl.params.percent_of_income).toBe(true);
    expect(c.find((e) => e.id === 'BalanceSheet')!.params.dates).toBe('AsOf');
    expect(c.filter((e) => e.category === 'Lists').length).toBe(4);
    // Every report in the catalog runs in fixture mode.
    for (const e of c) {
      const req = e.id === 'AccountQuickReport' ? { ...defaultRequest(e.id), filters: { ...defaultRequest(e.id).filters, accounts: [api.find('account', 'Checking')!] } } : e.id === 'JobProfitabilityDetail' ? { ...defaultRequest(e.id), filters: { ...defaultRequest(e.id).filters, names: [api.find('entity', 'Acme Corp')!] } } : defaultRequest(e.id);
      const r = await api.runReport(co, req);
      expect(r.id).toBe(e.id);
      expect(r.columns.length).toBeGreaterThan(0);
    }
  });

  test('register running balance ends at the account balance', async () => {
    const api = make();
    const checking = api.find('account', 'Checking')!;
    const reg = await api.register(co, checking);
    const bal = (await api.balances(co)).find((b) => b.account_id === checking)!.debit_positive;
    expect(reg.lines.at(-1)!.balance).toBe(bal);
    expect(reg.lines.some((l) => l.is_split)).toBe(true);
    expect(reg.lines.some((l) => l.cleared === 'Cleared')).toBe(true);
  });

  test('saving a check, then a stale edit, conflicts', async () => {
    const api = make();
    const txn = blankTxn('Check', today);
    txn.account_id = api.find('account', 'Checking')!;
    txn.entity_id = api.find('entity', 'Office Depot')!;
    txn.lines = [{ ...blankLine('Account'), account_id: api.find('account', 'Office Supplies')!, amount: 4_250 }];
    const saved = await api.createTxn(co, txn);
    expect(saved.total).toBe(4_250);
    expect(saved.txn.edit_seq).toBe(1);
    const again = await api.updateTxn(co, { ...saved.txn, memo: 'first edit' });
    expect(again.txn.edit_seq).toBe(2);
    const err = await rejects(api.updateTxn(co, { ...saved.txn, memo: 'stale edit' }));
    expect(err.status).toBe(409);
    expect(err.isStaleConflict).toBe(true);
  });

  test('posting errors come back as 422 with a PostingError', async () => {
    const api = make();
    const txn = blankTxn('Check', today);
    txn.lines = [{ ...blankLine('Account'), account_id: api.find('account', 'Office Supplies')!, amount: 100 }];
    const err = await rejects(api.createTxn(co, txn));
    expect(err.status).toBe(422);
    expect(err.postingError).toEqual({ code: 'MissingAccount' });

    const je = blankTxn('JournalEntry', today);
    je.lines = [{ ...blankLine('Journal'), account_id: api.find('account', 'Checking')!, amount: 100 }];
    const unbalanced = await rejects(api.createTxn(co, je));
    expect(unbalanced.postingError).toEqual({ code: 'Unbalanced', detail: 100 });
  });

  test('deleting a used account is refused as in_use; duplicates are refused', async () => {
    const api = make();
    const checking = (await api.list(co, 'accounts')).find((a) => a.full_name === 'Checking')!;
    const err = await rejects(api.deleteRecord(co, 'accounts', checking.id, checking.edit_seq));
    expect(err.code).toBe('in_use');
    const dup = await rejects(api.createRecord(co, 'accounts', { ...checking, id: 'new-id', edit_seq: 0 }));
    expect(dup.isDuplicate).toBe(true);
  });

  test('change events follow writes', async () => {
    const api = make();
    const events: string[] = [];
    const sub = api.subscribe(co, { onChange: (e) => events.push(`${e.kind}:${e.action}`) });
    const id = api.find('account', 'Checking')!;
    const reg = await api.register(co, id);
    await api.setCleared(co, id, reg.lines[0]!.txn_id, 'Uncleared');
    await new Promise((r) => setTimeout(r, 40));
    sub.close();
    expect(events).toContain('txn:cleared');
  });

  test('reports drill down to transactions and account detail', async () => {
    const api = make();
    const pl = await api.runReport(co, { ...defaultRequest('ProfitAndLoss'), date_macro: 'ThisFiscalYearToDate' });
    const cell = pl.rows.flatMap((r) => r.cells).find((c) => c.drill?.kind === 'Report')!;
    const drill = cell.drill!;
    if (drill.kind !== 'Report') throw new Error('expected a report drill');
    expect(drill.request.report).toBe('TransactionDetailByAccount');
    expect(drill.request.filters.accounts).toHaveLength(1);
    const detail = await api.runReport(co, drill.request);
    expect(detail.rows.some((r) => r.drill?.kind === 'Txn')).toBe(true);
    // The account's detail total equals the P&L figure it was drilled from.
    const figure = money(cell);
    const subtotal = detail.rows.find((r) => r.kind === 'Subtotal')!;
    expect(money(subtotal.cells[6])).toBe(figure);
    expect(money(pl.rows.find((r) => textOf(r) === 'Net Income')!.cells[1])).not.toBeNaN();
  });

  test('CSV export names the file after the report', async () => {
    const api = make();
    const file = await api.exportReport(co, { ...defaultRequest('TrialBalance') }, 'csv');
    expect(file.filename).toBe('Trial Balance.csv');
    expect((await file.blob.text()).startsWith('Juniper Hill Woodworks\r\nTrial Balance')).toBe(true);
  });

  test('memorized reports and transactions are saved documents', async () => {
    const api = make();
    const s = await api.createSaved(co, 'memorized_report', 'Monthly P&L', { ...defaultRequest('ProfitAndLoss'), date_macro: 'LastMonth' });
    expect((await api.saved(co, 'memorized_report')).map((d) => d.name)).toEqual(['Monthly P&L']);
    await api.deleteSaved(co, 'memorized_report', s.id);
    expect(await api.saved(co, 'memorized_report')).toEqual([]);

    const rent = (await api.txns(co, { types: ['Check'], text: 'Shop rent', limit: 1 }))[0]!;
    const template = (await api.txn(co, rent.id)).txn;
    const m: Memorized = { template, repeat: 'Remind', interval: 'Monthly', next_date: today, days_in_advance: 3, remaining: null, group: null };
    const saved = await api.createSaved(co, 'memorized_txn', 'Rent', m);
    expect((await api.memorizedDue(co)).map((d) => d.name)).toEqual(['Rent']);
    const fresh = await api.useMemorized(co, saved.id, '2026-11-01');
    expect(fresh.edit_seq).toBe(0);
    expect(fresh.id).not.toBe(template.id);
    expect(fresh.date).toBe('2026-11-01');
    expect(fresh.lines[0]!.amount).toBe(template.lines[0]!.amount);
    api.advanceMemorized(saved.id);
    expect(await api.memorizedDue(co)).toEqual([]);
  });

  test('open items include unused credits; undeposited lists payments not yet deposited', async () => {
    const api = make();
    const openInvoice = (await api.txns(co, { types: ['Invoice'], open_only: true }))[0]!;
    const open = await api.open(co, openInvoice.entity_id!, 'receivable');
    expect(open.every((o) => (o.open_balance ?? 0) > 0)).toBe(true);
    expect(open.some((o) => o.id === openInvoice.id)).toBe(true);
    const und = await api.undeposited(co);
    expect(und.every((u) => u.txn_type === 'ReceivePayment' && u.amount > 0)).toBe(true);
  });
});

describe('reconciliation', () => {
  test('start, mark, finish with an adjustment, then undo', async () => {
    const api = make();
    const checking = api.find('account', 'Checking')!;
    const before = await api.reconcileOverview(co, checking);
    expect(before.in_progress).toBeNull();
    const rec = await api.startReconcile(co, checking, { statement_date: today, ending_balance: before.beginning_balance + 1_000, service_charge: { amount: 1_000, date: today, account_id: api.find('account', 'Bank Service Charges')! }, interest: null });
    const chargeId = rec.service_charge_txn_id!;
    expect(chargeId).toBeTruthy();
    const view = await api.reconciliation(co, rec.id);
    expect(view.lines.length).toBeGreaterThan(0);
    expect(view.status.difference).toBe(1_000);
    // Mark the service charge: the difference grows by its 10.00 (a payment lowers the cleared balance).
    const status = await api.markReconcile(co, rec.id, [chargeId], true);
    expect(status.marked_count).toBe(1);
    expect(status.difference).toBe(2_000);
    await expect(api.finishReconcile(co, rec.id, false)).rejects.toMatchObject({ code: 'invalid' });
    const done = await api.finishReconcile(co, rec.id, true);
    expect(done.finished_at).not.toBeNull();
    expect(done.adjustment_txn_id).not.toBeNull();
    expect(done.cleared_txn_ids).toContain(chargeId);
    const reg = await api.register(co, checking);
    expect(reg.lines.find((l) => l.txn_id === chargeId)?.cleared).toBe('Cleared');
    const after = await api.reconcileOverview(co, checking);
    expect(after.history[0]?.id).toBe(rec.id);
    expect(after.beginning_balance).toBe(before.beginning_balance + 1_000);
    const undone = await api.undoReconcile(co, checking);
    expect(undone?.id).toBe(rec.id);
    expect((await api.register(co, checking)).lines.find((l) => l.txn_id === chargeId)?.cleared).toBe('Uncleared');
  });
  test('discard unmarks', async () => {
    const api = make();
    const checking = api.find('account', 'Checking')!;
    const rec = await api.startReconcile(co, checking, { statement_date: today, ending_balance: 0, service_charge: null, interest: null });
    const view = await api.reconciliation(co, rec.id);
    await api.markReconcile(co, rec.id, [view.lines[0]!.txn_id], true);
    await api.discardReconcile(co, rec.id);
    expect((await api.reconcileOverview(co, checking)).in_progress).toBeNull();
    expect((await api.register(co, checking)).lines.every((l) => l.cleared !== 'Pending')).toBe(true);
  });
});

const textOf = (row: Row) => (row.cells[0]?.value.type === 'Text' ? row.cells[0].value.value : '');
const money = (c: Cell | undefined) => (c?.value.type === 'Money' ? c.value.value : Number.NaN);
