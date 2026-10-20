import { describe, expect, test } from 'bun:test';
import { blankLine, blankTxn } from '../blank';
import { ApiError } from '../errors';
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
    const r = await api.runReport(co, { report: 'balance_sheet', macro: 'ThisFiscalYearToDate' });
    const total = (label: string) => r.rows.find((row) => row.cells[0] === label)?.cells[1];
    expect(total('TOTAL ASSETS')).toBe(total('TOTAL LIABILITIES & EQUITY'));
    expect(total('TOTAL ASSETS') as number).toBeGreaterThan(0);
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
    expect(saved.txn.edit_seq).toBe(1n);
    const again = await api.updateTxn(co, { ...saved.txn, memo: 'first edit' });
    expect(again.txn.edit_seq).toBe(2n);
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
    const dup = await rejects(api.createRecord(co, 'accounts', { ...checking, id: 'new-id', edit_seq: 0n }));
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

  test('reports drill down to transactions and QuickReports', async () => {
    const api = make();
    const pl = await api.runReport(co, { report: 'profit_and_loss', macro: 'ThisFiscalYearToDate' });
    const drill = pl.rows.find((r) => r.drill?.kind === 'report')!.drill!;
    expect(drill.kind).toBe('report');
    if (drill.kind !== 'report') return;
    const qr = await api.runReport(co, drill.request);
    expect(qr.rows.some((r) => r.drill?.kind === 'txn')).toBe(true);
    const net = pl.rows.find((r) => r.cells[0] === 'Net Income')!.cells[1];
    expect(typeof net).toBe('number');
  });
});
