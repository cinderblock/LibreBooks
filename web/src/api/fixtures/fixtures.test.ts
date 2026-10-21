import { describe, expect, test } from 'bun:test';
import { defaultRequest } from '~/reports/request';
import { blankCompanyInfo, blankLine, blankTxn } from '../blank';
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

describe('administration', () => {
  test('users: list, detail, role and disable rules, company access, delete', async () => {
    const api = make();
    const users = await api.users();
    expect(users.length).toBeGreaterThanOrEqual(4);
    const maya = users.find((u) => u.display_name.startsWith('Maya'))!;
    const detail = await api.user(maya.id);
    expect(detail.companies[0]).toMatchObject({ company_id: co, role: 'bookkeeper' });
    expect(detail.passkeys).toHaveLength(1);
    const updated = await api.updateUser(maya.id, { role: 'admin', disabled: true });
    expect(updated.role).toBe('admin');
    expect(updated.disabled_at).not.toBeNull();
    expect((await api.updateUser(maya.id, { disabled: false })).disabled_at).toBeNull();
    // You can't lock yourself out.
    expect((await rejects(api.updateUser('fixture-user', { disabled: true }))).code).toBe('invalid');
    expect((await rejects(api.deleteUser('fixture-user'))).code).toBe('invalid');
    await api.revokeAccess(maya.id, co);
    expect((await api.user(maya.id)).companies).toHaveLength(0);
    await api.grantAccess(maya.id, co, 'viewer');
    expect((await api.user(maya.id)).companies[0]?.role).toBe('viewer');
    await api.deleteUser(maya.id);
    expect((await api.users()).some((u) => u.id === maya.id)).toBe(false);
    expect((await rejects(api.user(maya.id))).status).toBe(404);
  });

  test('enrolment links: mint (shown once, with a url), list, revoke', async () => {
    const api = make();
    const before = (await api.enrollments()).length;
    const minted = await api.createEnrollment({ purpose: 'new_user', role: 'viewer', display_name: 'Pat', ttl_minutes: 30 });
    expect(minted.url).toContain(`/enroll#token=${minted.token}`);
    expect(minted.token.length).toBeGreaterThan(40);
    const list = await api.enrollments();
    expect(list).toHaveLength(before + 1);
    const mine = list.find((e) => e.id === minted.id)!;
    expect(mine).toMatchObject({ purpose: 'new_user', role: 'viewer', display_name: 'Pat', company_id: null });
    expect(JSON.stringify(mine)).not.toContain(minted.token);
    const device = await api.createEnrollment({ purpose: 'add_device' });
    expect((await api.enrollments()).find((e) => e.id === device.id)).toMatchObject({ purpose: 'add_device', user_id: 'fixture-user', role: null });
    await api.revokeEnrollment(minted.id);
    expect((await api.enrollments()).some((e) => e.id === minted.id)).toBe(false);
    expect((await rejects(api.revokeEnrollment(minted.id))).status).toBe(404);
    expect((await rejects(api.createEnrollment({ purpose: 'first_admin' }))).code).toBe('invalid');
  });

  test('my account: passkeys, devices and sessions', async () => {
    const api = make();
    const keys = await api.passkeys();
    expect(keys).toHaveLength(2);
    await api.renamePasskey(keys[0]!.id, 'Laptop');
    expect((await api.passkeys())[0]?.name).toBe('Laptop');
    expect((await rejects(api.renamePasskey(keys[0]!.id, ' '))).code).toBe('invalid');
    const sessions = await api.sessions();
    expect(sessions[0]?.current).toBe(true);
    const other = sessions.find((s) => !s.current)!;
    await api.revokeSession(other.id);
    expect((await api.sessions()).some((s) => s.id === other.id)).toBe(false);
    const devices = await api.devices();
    expect(devices).toHaveLength(1);
    await api.revokeDevice(devices[0]!.id);
    expect(await api.devices()).toHaveLength(0);
    // The device's session went with it.
    expect((await api.sessions()).some((s) => s.kind === 'device')).toBe(false);
    await api.deletePasskey(keys[0]!.id);
    // The last credential can't go.
    expect((await rejects(api.deletePasskey(keys[1]!.id))).code).toBe('conflict');
  });

  test('server settings: the whole document, problems first, secrets write-only', async () => {
    const api = make();
    const view = await api.server();
    expect(view.settings.backups.schedule.daily_at).toBe('02:30:00');
    expect(view.secrets.map((s) => s.name)).toEqual(['backup-passphrase']);
    const bad = await rejects(api.putServer({ ...view.settings, public_urls: ['not a url'], tunnel: { ...view.settings.tunnel, enabled: true } }));
    expect(bad.code).toBe('invalid');
    expect((bad.detail as { problems: string[] }).problems.length).toBe(2);
    const saved = await api.putServer({ ...view.settings, public_urls: ['https://books.example.org'], feeds: { auto_sync: false, every: 'PT1H' } });
    expect(saved.settings.public_urls).toEqual(['https://books.example.org']);
    expect(saved.restart_required).toContain('public_urls');
    await api.putSecret('s3-key', 'hunter2');
    expect((await api.secrets()).map((s) => s.name)).toEqual(['backup-passphrase', 's3-key']);
    expect(JSON.stringify(await api.server())).not.toContain('hunter2');
    expect((await rejects(api.deleteSecret('backup-passphrase'))).code).toBe('in_use');
    await api.deleteSecret('s3-key');
    expect((await rejects(api.putSecret('bad name!', 'x'))).code).toBe('invalid');
  });

  test('updates and the tunnel', async () => {
    const api = make();
    const u = await api.checkUpdate();
    expect(u.status?.available?.version).toBe('0.4.2');
    const installed = await api.installUpdate(false);
    expect(installed.restarting).toBe(false);
    expect((await api.update()).status?.available).toBeNull();
    expect((await rejects(api.installUpdate(true))).code).toBe('up_to_date');
    let t = await api.tunnel();
    expect(t.status.state.state).toBe('stopped');
    expect((await rejects(api.putTunnel({ enabled: true }))).code).toBe('invalid');
    t = await api.putTunnel({ token: 'cloudflared service install eyJhIjoiYWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXoifQ', public_hostname: 'books.example.com', enabled: true });
    expect(t.token_set).toBe(true);
    expect(t.status.state.state).toBe('connected');
    expect(t.public_url).toBe('https://books.example.com');
    expect(t.restart_required).toContain('tunnel.public_hostname');
    expect((await api.server()).secrets.some((s) => s.name === 'tunnel-token')).toBe(true);
  });

  test('backups: list, run now, verify needs the passphrase for contents, restore makes a new company', async () => {
    const api = make();
    const list = await api.backups(co);
    expect(list.backups).toHaveLength(5);
    const run = await api.runBackup(co);
    expect(run.destinations[0]?.stored).toBe(true);
    expect((await api.backups(co)).backups[0]?.manifest.id).toBe(run.manifest.id);
    const status = await api.backupsStatus(co);
    expect(status.scheduler?.last?.record?.manifest.id).toBe(run.manifest.id);
    const checksum = await api.verifyBackup(co, run.manifest.id, 0);
    expect(checksum).toMatchObject({ checksum_ok: true, contents_checked: false });
    const full = await api.verifyBackup(co, run.manifest.id, 0, { passphrase: 'pw' });
    expect(full).toMatchObject({ checksum_ok: true, contents_checked: true, problems: [] });
    expect((await rejects(api.restoreBackup(co, run.manifest.id, 0, {}))).code).toBe('invalid');
    const restored = await api.restoreBackup(co, run.manifest.id, 0, { passphrase: 'pw' });
    expect(restored.name).toContain('restored from');
    expect(restored.id).not.toBe(co);
    expect((await rejects(api.verifyBackup(co, 'nope', 0))).status).toBe(404);
    const file = await api.exportCompany(co);
    expect(file.filename.endsWith('.lbk')).toBe(true);
  });

  test('attachments: inbox, upload, attach to a transaction, detach, delete', async () => {
    const api = make();
    const inbox = await api.attachments(co);
    expect(inbox).toHaveLength(2);
    const txn = (await api.txns(co, { types: ['Check'], limit: 1 }))[0]!;
    const meta = await api.uploadAttachment(co, new File(['receipt'], 'receipt.txt', { type: 'text/plain' }), { record_kind: 'txn', record_id: txn.id });
    expect(meta).toMatchObject({ record_kind: 'txn', record_id: txn.id, filename: 'receipt.txt', mime: 'text/plain' });
    expect(meta.size).toBe(7);
    expect(await api.attachments(co, { record_kind: 'txn', record_id: txn.id })).toHaveLength(1);
    expect(await api.attachments(co)).toHaveLength(2);
    // `all`: everything, attached or not, newest first.
    const all = await api.attachments(co, { all: true });
    expect(all).toHaveLength(3);
    expect(all[0]!.id).toBe(meta.id);
    expect(all.filter((a) => a.record_kind === 'txn')).toHaveLength(1);
    const first = inbox[0]!;
    await api.attachRecord(co, first.id, { record_kind: 'txn', record_id: txn.id });
    expect(await api.attachments(co, { record_kind: 'txn', record_id: txn.id })).toHaveLength(2);
    await api.attachRecord(co, first.id, null);
    expect(await api.attachments(co)).toHaveLength(2);
    // Any record kind the API takes: a name, an account, an item.
    const vendor = api.find('entity', 'Office Depot')!;
    await api.attachRecord(co, first.id, { record_kind: 'entity', record_id: vendor });
    expect((await api.attachments(co, { record_kind: 'entity', record_id: vendor })).map((a) => a.id)).toEqual([first.id]);
    expect((await api.attachments(co, { all: true })).find((a) => a.id === first.id)?.record_kind).toBe('entity');
    await api.attachRecord(co, first.id, null);
    await api.deleteAttachment(co, meta.id);
    expect(await api.attachments(co, { record_kind: 'txn', record_id: txn.id })).toHaveLength(0);
    expect((await rejects(api.uploadAttachment(co, new File([], 'empty.txt')))).code).toBe('invalid');
  });

  test('closing date locks the period', async () => {
    const api = make();
    expect(await api.closingDate(co)).toBeNull();
    const txns = await api.txns(co, { types: ['Check'] });
    const oldest = [...txns].sort((a, b) => a.date.localeCompare(b.date))[0]!;
    expect(await api.putClosingDate(co, oldest.date)).toBe(oldest.date);
    const full = await api.txn(co, oldest.id);
    const err = await rejects(api.updateTxn(co, { ...full.txn, memo: 'changed' }));
    // As the server answers: 423 `closed_period` with the date in `detail`.
    expect(err.status).toBe(423);
    expect(err.code).toBe('closed_period');
    expect(err.closingDate).toBe(oldest.date);
    expect((await rejects(api.deleteTxn(co, oldest.id, full.txn.edit_seq))).code).toBe('closed_period');
    const check = { ...blankTxn('Check', oldest.date), account_id: api.find('account', 'Checking')!, lines: [{ ...blankLine('Account'), account_id: api.find('account', 'Office Supplies')!, amount: 100 }] };
    expect((await rejects(api.createTxn(co, check))).code).toBe('closed_period');
    expect((await api.createTxn(co, { ...check, date: addDaysIso(oldest.date, 1) })).txn.edit_seq).toBe(1);
    expect(await api.putClosingDate(co, null)).toBeNull();
    expect((await api.updateTxn(co, { ...full.txn, memo: 'changed' })).txn.memo).toBe('changed');
  });

  test('company info saves whole', async () => {
    const api = make();
    const info = await api.companyInfo(co);
    const saved = await api.putCompanyInfo(co, { ...info, legal_name: 'Juniper Hill Woodworks LLC', preferences: { ...info.preferences, use_class_tracking: false } });
    expect(saved.legal_name).toBe('Juniper Hill Woodworks LLC');
    expect((await api.companyInfo(co)).preferences.use_class_tracking).toBe(false);
    expect((await rejects(api.putCompanyInfo(co, { ...info, name: '' }))).code).toBe('invalid');
  });

  test('merge moves every reference and removes the duplicate', async () => {
    const api = make();
    const vendors = (await api.list(co, 'entities', { kind: 'Vendor' })).filter((v) => !v.parent_id);
    const [a, b] = vendors;
    const before = (await api.txns(co, { entity_id: a!.id })).length + (await api.txns(co, { entity_id: b!.id })).length;
    expect(before).toBeGreaterThan(0);
    await api.merge(co, { kind: 'entity', from: a!.id, into: b!.id });
    expect((await api.list(co, 'entities', { kind: 'Vendor' })).some((v) => v.id === a!.id)).toBe(false);
    expect((await api.txns(co, { entity_id: b!.id })).length).toBe(before);
    // Accounts: same type only; balances fold together.
    const accounts = await api.list(co, 'accounts');
    const expenses = accounts.filter((x) => x.account_type === 'Expense' && !x.parent_id && !accounts.some((y) => y.parent_id === x.id));
    const [e1, e2] = expenses;
    const bal = new Map((await api.balances(co)).map((x) => [x.account_id, x.debit_positive]));
    const sum = (bal.get(e1!.id) ?? 0) + (bal.get(e2!.id) ?? 0);
    await api.merge(co, { kind: 'account', from: e1!.id, into: e2!.id });
    expect(new Map((await api.balances(co)).map((x) => [x.account_id, x.debit_positive])).get(e2!.id) ?? 0).toBe(sum);
    const bank = accounts.find((x) => x.account_type === 'Bank')!;
    expect((await rejects(api.merge(co, { kind: 'account', from: bank.id, into: e2!.id }))).code).toBe('invalid');
    expect((await rejects(api.merge(co, { kind: 'entity', from: b!.id, into: b!.id }))).code).toBe('invalid');
  });

  test('name balances: open items per name, credits count against, jobs roll up to the parent', async () => {
    const api = make();
    const rows = await api.nameBalances(co);
    const entities = await api.list(co, 'entities', { include_inactive: true });
    const byId = new Map(entities.map((e) => [e.id, e]));
    const bal = new Map(rows.map((r) => [r.entity_id, r]));
    expect(rows.length).toBeGreaterThan(0);
    // Every row's totals are its own balance plus its children's totals.
    for (const r of rows) {
      const kids = entities.filter((e) => e.parent_id === r.entity_id);
      expect(r.receivable_total).toBe(r.receivable + kids.reduce((s, k) => s + (bal.get(k.id)?.receivable_total ?? 0), 0));
      expect(r.payable_total).toBe(r.payable + kids.reduce((s, k) => s + (bal.get(k.id)?.payable_total ?? 0), 0));
    }
    // A job with an open invoice carries the balance; its customer shows it in Balance Total only.
    const job = rows.find((r) => r.receivable > 0 && byId.get(r.entity_id)?.parent_id);
    expect(job).toBeDefined();
    const parent = bal.get(byId.get(job!.entity_id)!.parent_id!)!;
    expect(parent.receivable).toBe(0);
    expect(parent.receivable_total).toBeGreaterThanOrEqual(job!.receivable_total);
    // The balance is what `open` adds up to, credits against.
    const open = await api.open(co, job!.entity_id, 'receivable');
    expect(open.reduce((s, t) => s + (t.txn_type === 'CreditMemo' || t.txn_type === 'ReceivePayment' ? -1 : 1) * (t.open_balance ?? 0), 0)).toBe(job!.receivable);
    // An unpaid bill: we owe the vendor; it owes us nothing.
    const vendor = rows.find((r) => r.payable > 0)!;
    expect(byId.get(vendor.entity_id)?.kind).toBe('Vendor');
    expect(vendor.receivable).toBe(0);
    // An unapplied payment is a credit against the customer, and against the parent's total.
    const customer = byId.get(job!.entity_id)!;
    const saved = await api.createTxn(co, { ...blankTxn('ReceivePayment', today), entity_id: customer.id, amount: 1_000, applications: [] });
    const after = new Map((await api.nameBalances(co)).map((r) => [r.entity_id, r]));
    expect(after.get(customer.id)!.receivable).toBe(job!.receivable - 1_000);
    expect(after.get(parent.entity_id)!.receivable_total).toBe(parent.receivable_total - 1_000);
    // Voided, it counts for nothing again.
    await api.voidTxn(co, saved.txn.id, saved.txn.edit_seq);
    expect(new Map((await api.nameBalances(co)).map((r) => [r.entity_id, r])).get(customer.id)!.receivable).toBe(job!.receivable);
  });

  test('QuickBooks import: settings (key write-only), a sync that runs and reports, parity, Web Connector password', async () => {
    const api = make();
    const events: string[] = [];
    const sub = api.subscribe(co, { onChange: (e) => events.push(`${e.kind}:${e.id}:${e.action}`) });
    const settled = () => new Promise((r) => setTimeout(r, 40));
    const st = await api.qbdStatus(co);
    expect(st.settings.transport).toBe('bridge');
    expect(st.bridge_key_set).toBe(true);
    expect(st.running).toBeNull();
    expect(st.last_sync?.entities.length).toBeGreaterThan(5);
    // The key is write-only: it becomes a secret reference and never comes back; the .qwc ids can't be changed.
    const saved = await api.putQbdSettings(co, { ...st.settings, bridge: { ...st.settings.bridge, delay_ms: 500 }, qbwc: { ...st.settings.qbwc, owner_id: 'nope' } }, 'secret-key');
    expect(saved.settings.bridge.delay_ms).toBe(500);
    expect(saved.settings.bridge.key_ref).toBe(`secret:qbd.${co}.bridge-key`);
    expect(saved.settings.qbwc.owner_id).toBe(st.settings.qbwc.owner_id);
    expect(JSON.stringify(saved)).not.toContain('secret-key');
    const bad = await rejects(api.putQbdSettings(co, { ...st.settings, bridge: { ...st.settings.bridge, url: 'nope' } }));
    expect(bad.code).toBe('invalid');
    expect((bad.detail as { problems: string[] }).problems[0]).toContain('bridge URL');
    // Settings changes emit no change event, as on the server (only jobs do).
    await settled();
    expect(events).toEqual([]);
    // One job at a time. `?wait=` on the status answers at the next progress step (the bar is live), not only at the end…
    const sync = await api.qbdSync(co, { full: true });
    expect(sync.running?.kind).toBe('sync');
    expect((await rejects(api.qbdSync(co))).code).toBe('running');
    expect((await rejects(api.qbdParityRun(co))).code).toBe('running');
    const stepped = await api.qbdStatus(co, 10);
    expect(stepped.running?.progress).toEqual({ entity: 'Account', phase: 'Listing', done: 0, total: 56 });
    // …so a watcher asks again until the job is gone.
    let done = stepped;
    let polls = 0;
    while (done.running) {
      done = await api.qbdStatus(co, 10);
      polls += 1;
    }
    expect(polls).toBeGreaterThan(1);
    expect(done.last_sync?.full).toBe(true);
    // The events are the server's: `import`, the job kind as the id, started → progress… → finished.
    await settled();
    expect(events).toEqual(['import:sync:started', ...Array<string>(6).fill('import:sync:progress'), 'import:sync:finished']);
    events.length = 0;
    // `?wait=` on the parity route holds until the job is done.
    expect((await api.qbdParity(co)).report).toBeNull();
    const started = await api.qbdParityRun(co);
    expect(started.running?.kind).toBe('parity');
    expect((await api.qbdParity(co)).running).toBe(true);
    const parityStatus = await api.qbdParity(co, 10);
    expect(parityStatus.running).toBe(false);
    const parity = parityStatus.report;
    expect(parity?.summary.accounts_off).toBe(2);
    expect(parity?.accounts.filter((a) => a.difference !== 0).length).toBe(2);
    expect(parity?.summary.total_abs_difference).toBe(12_501);
    expect((await api.qbdStatus(co)).last_parity?.generated_at).toBe(parity!.generated_at);
    await settled();
    expect(events).toEqual(['import:parity:started', 'import:parity:finished']);
    sub.close();
    // The Web Connector password: generated and shown once, or chosen (nothing back); at least 8 characters.
    const pw = await api.qbwcPassword(co, null);
    expect(pw?.length).toBe(12);
    expect((await api.qbdStatus(co)).qbwc_password_set).toBe(true);
    expect(await api.qbwcPassword(co, 'longenough')).toBeNull();
    expect((await rejects(api.qbwcPassword(co, 'short'))).code).toBe('invalid');
    const qwc = await api.qbwcFile(co);
    expect(qwc.filename).toBe('librebooks-juniper-hill-woodworks.qwc');
    const xml = await qwc.blob.text();
    expect(xml).toContain('<QBWCXML>');
    expect(xml).toContain(`<OwnerID>{${st.settings.qbwc.owner_id.toUpperCase()}}</OwnerID>`);
    // Over the Web Connector a job waits for QuickBooks to poll.
    await api.putQbdSettings(co, { ...st.settings, transport: 'qbwc' });
    expect((await api.qbdSync(co)).running?.waiting_for_web_connector).toBe(true);
    api.dispose();
  });

  test('QuickBooks import: the sync and parity options shape the reports', async () => {
    const api = make();
    const st = await api.qbdStatus(co);
    await api.putQbdSettings(co, { ...st.settings, bridge: { ...st.settings.bridge, url: 'http://192.168.1.40:8750' } }, 'bridge-key-1');
    // A limited sync of two kinds: a partial run (watermarks untouched) reporting only those kinds.
    // The status long-poll answers on every progress step; a watcher asks again until the job is gone.
    const finished = async () => {
      let st = await api.qbdStatus(co, 10);
      while (st.running) st = await api.qbdStatus(co, 10);
      return st;
    };
    await api.qbdSync(co, { txn_limit: 50, entities: ['Invoice', 'Check'] });
    let done = await finished();
    expect(done.last_sync?.partial_run).toBe(true);
    expect(done.last_sync?.full).toBe(false);
    expect(done.last_sync?.entities.map((e) => e.entity)).toEqual(['Invoice', 'Check']);
    // Re-importing counts the unchanged as updated.
    await api.qbdSync(co, { reimport: true, txn_from: '2026-01-01' });
    done = await finished();
    expect(done.last_sync?.partial_run).toBe(true);
    expect(done.last_sync?.entities.every((e) => e.unchanged === 0)).toBe(true);
    // Parity as of a date, with a trial balance asked for (the bridge can't; the report says so).
    await api.qbdParityRun(co, { as_of: '2026-09-30', trial_balance: true });
    const parity = await api.qbdParity(co, 10);
    expect(parity.report?.summary.notes).toContain('Balances as of 2026-09-30.');
    expect(parity.report?.summary.notes.some((n) => n.includes('trial balance'))).toBe(true);
    api.dispose();
  });
});

describe('companies', () => {
  test('a new company joins the session and the picker, with its own information', async () => {
    const api = make();
    expect((await rejects(api.createCompany(blankCompanyInfo('   ')))).code).toBe('invalid');
    const { id } = await api.createCompany({ ...blankCompanyInfo('Second Shop'), fiscal_year_start_month: 7 });
    expect(id).not.toBe(co);
    expect((await api.session()).companies.map((c) => c.name)).toEqual(['Juniper Hill Woodworks', 'Second Shop']);
    expect((await api.companies()).find((c) => c.id === id)?.role).toBe('admin');
    const info = await api.companyInfo(id);
    expect(info.name).toBe('Second Shop');
    expect(info.fiscal_year_start_month).toBe(7);
    expect(info.preferences.sales_tax_enabled).toBe(false);
    // Its information is its own; the demo company's is untouched.
    await api.putCompanyInfo(id, { ...info, name: 'Second Shop LLC' });
    expect((await api.companyInfo(id)).name).toBe('Second Shop LLC');
    expect((await api.companyInfo(co)).name).toBe('Juniper Hill Woodworks');
    // Company-scoped routes answer for it (the fixture's one set of books).
    expect((await api.list(id, 'accounts')).length).toBeGreaterThan(10);
  });

  test('importing a .lbk makes a new company named by the request, else by the file', async () => {
    const api = make();
    expect((await rejects(api.importCompany(new File([], 'empty.lbk')))).code).toBe('invalid');
    const named = await api.importCompany(new File(['books'], 'juniper-hill-woodworks.lbk'), 'Juniper Hill (copy)');
    expect(named.name).toBe('Juniper Hill (copy)');
    expect(named.from).toMatchObject({ kind: 'librebooks-company', company_name: 'Juniper Hill (copy)', database_size: 5 });
    expect(named.from.database_sha256).toMatch(/^[0-9a-f]{64}$/);
    const fromFile = await api.importCompany(new File(['books'], 'Riverside Cafe.lbk'));
    expect(fromFile.name).toBe('Riverside Cafe');
    expect((await api.companies()).map((c) => c.name)).toEqual(['Juniper Hill Woodworks', 'Juniper Hill (copy)', 'Riverside Cafe']);
    expect((await api.companyInfo(fromFile.id)).name).toBe('Riverside Cafe');
  });

  test('a restored backup shows up in the company list', async () => {
    const api = make();
    const run = await api.runBackup(co);
    const restored = await api.restoreBackup(co, run.manifest.id, 0, { passphrase: 'pw', name: 'Juniper Hill (restored)' });
    expect((await api.companies()).some((c) => c.id === restored.id && c.name === 'Juniper Hill (restored)')).toBe(true);
    expect((await api.companyInfo(restored.id)).name).toBe('Juniper Hill (restored)');
  });
});

function addDaysIso(d: string, n: number): string {
  const t = new Date(`${d}T00:00:00Z`);
  t.setUTCDate(t.getUTCDate() + n);
  return t.toISOString().slice(0, 10);
}

const textOf = (row: Row) => (row.cells[0]?.value.type === 'Text' ? row.cells[0].value.value : '');
const money = (c: Cell | undefined) => (c?.value.type === 'Money' ? c.value.value : Number.NaN);
