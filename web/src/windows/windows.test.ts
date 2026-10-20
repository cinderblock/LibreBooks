import { describe, expect, test } from 'bun:test';
import { EMPTY_WINDOWS, type WindowsAction, type WindowsState, windowsReducer } from './model';
import { parseWindowPath, paths } from './paths';

const run = (...actions: WindowsAction[]): WindowsState => actions.reduce(windowsReducer, EMPTY_WINDOWS);
const open = (path: string): WindowsAction => ({ type: 'open', desc: parseWindowPath(path)! });

describe('window paths', () => {
  test('known paths', () => {
    expect(parseWindowPath('/')?.key).toBe('home');
    expect(parseWindowPath('/accounts')?.kind).toBe('accounts');
    expect(parseWindowPath('/accounts/a1/register')).toMatchObject({ key: 'register:a1', params: { accountId: 'a1' } });
    expect(parseWindowPath('/txns/t9')).toMatchObject({ key: 'txn:t9', kind: 'txn' });
    expect(parseWindowPath('/checks/new')?.key).toBe('check:new');
    expect(parseWindowPath('/customers?select=e1')).toMatchObject({ key: 'customers', params: { select: 'e1' } });
    expect(parseWindowPath('/forms/invoice/new')).toMatchObject({ key: 'form:invoice', params: { form: 'invoice' } });
  });
  test('reports are distinct per subject; dates change in place', () => {
    const a = parseWindowPath('/reports/AccountQuickReport?acct=a1&dates=ThisMonth');
    const b = parseWindowPath('/reports/AccountQuickReport?dates=LastMonth&acct=a1&cols=Month');
    const c = parseWindowPath('/reports/AccountQuickReport?acct=a2');
    const d = parseWindowPath('/reports/AccountQuickReport?acct=a1&exact=1');
    expect(a?.key).toBe(b!.key);
    expect(a?.key).not.toBe(c!.key);
    expect(a?.key).not.toBe(d!.key);
    expect(b?.params.dates).toBe('LastMonth');
  });
  test('unknown paths', () => {
    expect(parseWindowPath('/nope')).toBeNull();
    expect(parseWindowPath('/accounts/a1/ledger')).toBeNull();
  });
  test('builders round-trip', () => {
    expect(parseWindowPath(paths.register('x y'))?.params.accountId).toBe('x y');
    expect(parseWindowPath(paths.report('balance_sheet', { macro: 'ThisMonth', account: null }))?.params).toEqual({
      macro: 'ThisMonth',
      report: 'balance_sheet',
    });
  });
});

describe('open window list', () => {
  test('opening appends and activates; reopening just activates', () => {
    const s = run(open('/home'), open('/accounts'), open('/home'));
    expect(s.windows.map((w) => w.key)).toEqual(['home', 'accounts']);
    expect(s.active).toBe('home');
    expect(s.mru).toEqual(['home', 'accounts']);
  });
  test('reopening updates params (e.g. a centre selecting another name)', () => {
    const s = run(open('/customers?select=a'), open('/customers?select=b'));
    expect(s.windows).toHaveLength(1);
    expect(s.windows[0]!.params.select).toBe('b');
  });
  test('Ctrl+Tab cycles in opening order and wraps', () => {
    let s = run(open('/home'), open('/accounts'), open('/checks/new'));
    s = windowsReducer(s, { type: 'cycle', delta: 1 });
    expect(s.active).toBe('home');
    s = windowsReducer(s, { type: 'cycle', delta: -1 });
    expect(s.active).toBe('check:new');
  });
  test('closing the active window returns to the most recently used one', () => {
    let s = run(open('/home'), open('/accounts'), open('/checks/new'), { type: 'activate', key: 'home' }, open('/find'));
    s = windowsReducer(s, { type: 'close', key: 'find' });
    expect(s.active).toBe('home');
    s = windowsReducer(s, { type: 'close', key: 'home' });
    expect(s.active).toBe('check:new');
  });
  test('dirty and title bookkeeping is dropped on close', () => {
    let s = run(open('/checks/new'), { type: 'dirty', key: 'check:new', dirty: true }, { type: 'title', key: 'check:new', title: 'Write Checks' });
    expect(s.dirty['check:new']).toBe(true);
    s = windowsReducer(s, { type: 'close', key: 'check:new' });
    expect(s.dirty).toEqual({});
    expect(s.titles).toEqual({});
    expect(s.active).toBeNull();
  });
  test('no-op actions return the same state object (cheap re-renders)', () => {
    const s = run(open('/home'));
    expect(windowsReducer(s, { type: 'activate', key: 'home' })).toBe(s);
    expect(windowsReducer(s, { type: 'dirty', key: 'home', dirty: false })).toBe(s);
    expect(windowsReducer(s, { type: 'cycle', delta: 1 })).toBe(s);
  });
});
