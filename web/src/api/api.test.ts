import { describe, expect, test } from 'bun:test';
import { blankLine, blankTxn } from './blank';
import { txnQueryParams } from './client';
import { ApiError, apiErrorFromBody, errorTargets } from './errors';
import { type FetchFn, HttpApiClient } from './http';
import { parseApiJson, stringifyApiJson } from './json';

describe('JSON edit_seq handling', () => {
  test('edit_seq revives to bigint and writes back as a number', () => {
    const v = parseApiJson<{ edit_seq: bigint; amount: number; lines: Array<{ edit_seq: bigint }> }>(
      '{"edit_seq":7,"amount":1234,"lines":[{"edit_seq":2}]}',
    );
    expect(v.edit_seq).toBe(7n);
    expect(v.amount).toBe(1234);
    expect(v.lines[0]!.edit_seq).toBe(2n);
    expect(stringifyApiJson(v)).toBe('{"edit_seq":7,"amount":1234,"lines":[{"edit_seq":2}]}');
  });
});

describe('txnQueryParams', () => {
  test('arrays join, booleans only when true, nulls dropped', () => {
    const p = txnQueryParams({ types: ['Check', 'Bill'], from: '2026-01-01', to: null, include_jobs: true, open_only: false, limit: 50 });
    expect(p.toString()).toBe('types=Check%2CBill&from=2026-01-01&include_jobs=true&limit=50');
  });
});

describe('error envelope', () => {
  test('parses the envelope', () => {
    const e = apiErrorFromBody(409, '', { error: { code: 'conflict', message: 'stale', detail: { expected: 3, actual: 4 } } });
    expect(e.code).toBe('conflict');
    expect(e.isStaleConflict).toBe(true);
    expect(e.isDuplicate).toBe(false);
  });
  test('falls back on status for non-JSON bodies', () => {
    const e = apiErrorFromBody(404, 'nope', undefined);
    expect(e.code).toBe('not_found');
    expect(e.message).toBe('nope');
  });
});

describe('HttpApiClient', () => {
  test('sends the CSRF header on mutations and credentials always', async () => {
    const calls: Array<{ url: string; init: RequestInit }> = [];
    const fake = (async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      if (init.method === 'DELETE') return new Response(null, { status: 204 });
      return new Response('{"txn":{"edit_seq":3},"postings":[],"total":0,"sales_tax":0,"warnings":[]}', { status: 200 });
    }) as unknown as FetchFn;
    const api = new HttpApiClient({ fetch: fake });
    const t = blankTxn('Check', '2026-10-19');
    const out = await api.updateTxn('co1', { ...t, edit_seq: 2n });
    expect(out.txn.edit_seq).toBe(3n);
    await api.deleteTxn('co1', t.id, 3n);
    await api.txns('co1', { types: ['Check'] });
    const [put, del, get] = calls;
    expect(put!.url).toBe(`/api/v1/c/co1/txns/${t.id}`);
    expect((put!.init.headers as Record<string, string>)['X-LibreBooks']).toBe('1');
    expect(JSON.parse(put!.init.body as string).edit_seq).toBe(2);
    expect(del!.url).toBe(`/api/v1/c/co1/txns/${t.id}?edit_seq=3`);
    expect((get!.init.headers as Record<string, string>)['X-LibreBooks']).toBeUndefined();
    expect(get!.init.credentials).toBe('include');
    expect(get!.url).toBe('/api/v1/c/co1/txns?types=Check');
  });

  test('the desktop app authenticates with its bearer token against its own base', async () => {
    const calls: Array<{ url: string; init: RequestInit }> = [];
    const fake = (async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      return new Response('{"user":{"id":"u","display_name":"Me","role":"admin","kind":"local"},"companies":[],"sign_in":false}', { status: 200 });
    }) as unknown as FetchFn;
    const api = new HttpApiClient({ fetch: fake, token: 's3cret', baseUrl: 'http://127.0.0.1:8740/api/v1/' });
    const s = await api.session();
    expect(s.user.kind).toBe('local');
    expect(calls[0]!.url).toBe('http://127.0.0.1:8740/api/v1/session');
    expect((calls[0]!.init.headers as Record<string, string>).Authorization).toBe('Bearer s3cret');
  });

  test('turns error envelopes into ApiError', async () => {
    const fake = (async () =>
      new Response('{"error":{"code":"posting","message":"refused","detail":{"code":"NoLines"}}}', { status: 422 })) as unknown as FetchFn;
    const api = new HttpApiClient({ fetch: fake });
    try {
      await api.createTxn('co1', blankTxn('Check', '2026-10-19'));
      throw new Error('should have thrown');
    } catch (e) {
      expect(e).toBeInstanceOf(ApiError);
      expect((e as ApiError).postingError).toEqual({ code: 'NoLines' });
    }
  });

  test('network failures are ApiError("network")', async () => {
    const fake = (async () => {
      throw new TypeError('Failed to fetch');
    }) as unknown as FetchFn;
    const api = new HttpApiClient({ fetch: fake });
    try {
      await api.health();
    } catch (e) {
      expect((e as ApiError).code).toBe('network');
    }
  });
});

describe('errorTargets', () => {
  const txn = blankTxn('Check', '2026-10-19');
  txn.account_id = 'bank';
  txn.lines = [
    { ...blankLine('Account'), account_id: 'rent', amount: 100 },
    { ...blankLine('Account'), account_id: null, amount: 50 },
  ];
  test('missing account points at the blank line', () => {
    const e = new ApiError(422, 'posting', 'x', { code: 'MissingAccount' });
    expect(errorTargets(txn, e)).toEqual([{ field: 'line', index: 1, column: 'account' }]);
  });
  test('inactive account points at the lines using it', () => {
    const e = new ApiError(422, 'posting', 'x', { code: 'InactiveAccount', detail: 'rent' });
    expect(errorTargets(txn, e)).toEqual([{ field: 'line', index: 0, column: 'account' }]);
  });
  test('wrong bank type points at the header', () => {
    const e = new ApiError(422, 'posting', 'x', { code: 'WrongAccountType', detail: { account: 'bank', expected: 'Bank' } });
    expect(errorTargets(txn, e)).toEqual([{ field: 'account' }]);
  });
  test('closed period points at the date; invalid field paths parse', () => {
    expect(errorTargets(txn, new ApiError(423, 'closed_period', 'x', { closing_date: '2026-09-30' }))).toEqual([{ field: 'date' }]);
    expect(errorTargets(txn, new ApiError(400, 'invalid', 'x', { field: 'lines[1].amount' }))).toEqual([
      { field: 'line', index: 1, column: 'amount' },
    ]);
  });
});
