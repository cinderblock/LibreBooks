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

describe('bank feeds', () => {
  test('the queue has suggestions; a download with a register twin has a strong candidate; match marks the line downloaded', async () => {
    const api = make();
    const summary = await api.feedsSummary(co);
    expect(summary.length).toBeGreaterThan(0);
    const queue = await api.feedTxns(co, { statuses: ['New'] });
    expect(queue.length).toBeGreaterThan(3);
    const fee = queue.find((t) => t.payee === 'MONTHLY SERVICE FEE')!;
    expect(fee.suggestion?.rule_id).toBeTruthy();
    expect(fee.suggestion?.auto_add).toBe(true);
    const twin = queue.find((t) => t.check_number)!;
    const cands = await api.feedCandidates(co, twin.id);
    expect(cands[0]?.strong).toBe(true);
    expect(cands[0]?.reasons.join(' ')).toContain('Check number');
    await api.feedMatch(co, twin.id, cands[0]!.txn_id);
    expect((await api.feedTxn(co, twin.id)).status).toBe('Matched');
    const reg = await api.register(co, twin.account_id);
    expect((reg.lines.find((l) => l.txn_id === cands[0]!.txn_id) as { downloaded?: boolean }).downloaded).toBe(true);
    await api.feedUndo(co, twin.id);
    expect((await api.feedTxn(co, twin.id)).status).toBe('New');
  });
  test('add writes a transaction from the suggestion; undo deletes it; ignore and rules', async () => {
    const api = make();
    const fee = (await api.feedTxns(co, { statuses: ['New'] })).find((t) => t.payee === 'MONTHLY SERVICE FEE')!;
    const preview = await api.feedPreview(co, fee.id, { entity_id: null, account_id: null, class_id: null, memo: null, number: null });
    expect(preview.txn.txn_type).toBe('Check');
    expect(preview.postings.length).toBe(2);
    const out = await api.feedAdd(co, fee.id, { entity_id: null, account_id: null, class_id: null, memo: 'Bank fee', number: null });
    const saved = await api.txn(co, out.txn_id);
    expect(saved.txn.memo).toBe('Bank fee');
    expect(saved.txn.amount).toBe(1_200);
    await api.feedUndo(co, fee.id);
    await expect(api.txn(co, out.txn_id)).rejects.toMatchObject({ status: 404 });
    expect(await api.feedIgnore(co, [fee.id])).toBe(1);
    const rules = await api.feedRules(co);
    const r = await api.createFeedRule(co, { ...rules[0]!, id: '', name: 'Cafe', conditions: [{ kind: 'Text', field: 'Payee', op: 'Contains', value: 'CAFE' }], actions: { ...rules[0]!.actions, rename_to: 'Riverside Cafe', account_id: api.find('account', 'Meals and Entertainment')!, entity_id: null, auto_add: false } });
    expect(await api.applyFeedRules(co)).toBeGreaterThan(0);
    const cafe = (await api.feedTxns(co, { statuses: ['New'] })).find((t) => t.payee?.includes('CAFE'))!;
    expect(cafe.suggestion?.rule_id).toBe(r.id);
  });
  test('a CSV is detected, mapped, previewed and imported through the pipeline', async () => {
    const api = make();
    // The fee row repeats a seeded download (same date, amount and payee → the same synthesised id).
    const csv = ['Date,Description,Amount', '10/16/2026,ZELLE FROM NGUYEN DAVID,500.00', '10/17/2026,SHELL OIL 57444281,-62.30', '10/18/2026,MONTHLY SERVICE FEE,-12.00'].join('\n');
    const file = new File([csv], 'checking.csv', { type: 'text/csv' });
    const det = await api.feedDetect(co, file);
    expect(det.kind).toBe('Csv');
    expect(det.csv?.confident).toBe(true);
    expect(det.csv?.preview.transactions).toHaveLength(3);
    expect(det.csv?.preview.transactions[1]?.amount).toBe(-6_230);
    const checking = api.find('account', 'Checking')!;
    const report = await api.feedImport(co, { file, account_id: checking, mapping: det.csv!.mapping });
    // One of the three is already in the queue (same date, amount, payee → same synthesised id).
    expect(report.import.imported.length + report.import.duplicates).toBe(3);
    expect(report.import.duplicates).toBeGreaterThanOrEqual(1);
    // The bank-fee rule adds automatically when nothing in the register matches.
    expect(report.auto_added.length + report.waiting + report.matched.length).toBe(report.import.imported.length);
    const ofx = '<OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS><CURDEF>USD<BANKACCTFROM><BANKID>123456789<ACCTID>00004471</BANKACCTFROM><BANKTRANLIST><STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20261015<TRNAMT>-45.00<FITID>abc1<NAME>TOOL BARN</STMTTRN></BANKTRANLIST><LEDGERBAL><BALAMT>1234.56<DTASOF>20261019</LEDGERBAL></STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>';
    const d2 = await api.feedDetect(co, new File([ofx], 'stmt.qbo'));
    expect(d2.kind).toBe('Ofx');
    expect(d2.statements?.[0]?.transactions[0]).toMatchObject({ fitid: 'abc1', amount: -4_500, posted: '2026-10-15' });
    expect(d2.statements?.[0]?.ledger_balance).toBe(123_456);
  });
  test('SimpleFIN claim lists accounts; Amazon orders match card downloads', async () => {
    const api = make();
    const claim = await api.simplefinClaim(co, 'aHR0cHM6Ly9icmlkZ2U');
    expect(claim.accounts.length).toBe(2);
    const orders = ['Order ID,Order Date,Total Owed,Product Name,Ship Date', `111-2223334-4445556,${addDaysIso('2026-10-19', -3)},48.17,USB-C cable,${addDaysIso('2026-10-19', -2)}`].join('\n');
    const p = await api.amazonOrders(co, new File([orders], 'orders.csv'));
    expect(p.orders).toHaveLength(1);
    expect(p.matches).toHaveLength(1);
    expect(p.matches[0]?.memo).toContain('USB-C cable');
    expect(await api.amazonApply(co, p)).toBe(1);
  });
});

function addDaysIso(d: string, n: number): string {
  const t = new Date(`${d}T00:00:00Z`);
  t.setUTCDate(t.getUTCDate() + n);
  return t.toISOString().slice(0, 10);
}

const textOf = (row: Row) => (row.cells[0]?.value.type === 'Text' ? row.cells[0].value.value : '');
const money = (c: Cell | undefined) => (c?.value.type === 'Money' ? c.value.value : Number.NaN);
