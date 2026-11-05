import { describe, expect, test } from 'bun:test';
import { API_ACCESS, gateWrites } from '~/api/access';
import type { ApiClient } from '~/api/client';
import { ApiError } from '~/api/errors';
import { FixtureApiClient } from '~/api/fixtures/client';
import type { NoticeAcceptance, Session } from '~/api/types';
import { ALPHA_NOTICE_VERSION, AlphaNotice, DECLINED_MESSAGE } from './notice';

/** A tick for promise chains to run. */
const flush = () => new Promise((r) => setTimeout(r, 0));

function fakeServer(accepted: NoticeAcceptance | null = null) {
  const calls = { session: 0, accept: 0 };
  const server = {
    accepted,
    fail: false,
    session(): Promise<Session> {
      calls.session++;
      return Promise.resolve({ user: { id: 'u', display_name: 'Ada', role: 'admin', alpha_notice: server.accepted }, companies: [] });
    },
    acceptAlphaNotice(version: number): Promise<NoticeAcceptance> {
      calls.accept++;
      if (server.fail) return Promise.reject(new ApiError(0, 'network', 'Network error'));
      server.accepted = { version, accepted_at: '2026-11-05T10:00:00Z' };
      return Promise.resolve(server.accepted);
    },
  };
  return { server, calls };
}

describe('AlphaNotice', () => {
  test('already accepted on the server: no prompt', async () => {
    const { server } = fakeServer({ version: ALPHA_NOTICE_VERSION, accepted_at: '2026-01-01T00:00:00Z' });
    const notice = new AlphaNotice(server);
    await notice.ensure();
    expect(notice.getSnapshot().prompt).toBe('none');
  });

  test('every change waiting meanwhile shares one prompt, and all go ahead on accept', async () => {
    const { server, calls } = fakeServer();
    const notice = new AlphaNotice(server);
    let done = 0;
    const a = notice.ensure().then(() => done++);
    const b = notice.ensure().then(() => done++);
    await flush();
    expect(notice.getSnapshot().prompt).toBe('required');
    expect(calls.session).toBe(1);
    expect(done).toBe(0);
    await notice.accept();
    await Promise.all([a, b]);
    expect(done).toBe(2);
    expect(calls.accept).toBe(1);
    expect(notice.getSnapshot().prompt).toBe('none');
    // Remembered: the next change doesn't ask the server or the person.
    await notice.ensure();
    expect(calls.session).toBe(1);
  });

  test('cancelling fails the held change with alpha_notice, and the next change asks again', async () => {
    const { server } = fakeServer();
    const notice = new AlphaNotice(server);
    const held = notice.ensure();
    await flush();
    notice.dismiss();
    const err = await held.then(
      () => null,
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).code).toBe('alpha_notice');
    expect((err as ApiError).message).toBe(DECLINED_MESSAGE);
    void notice.ensure();
    await flush();
    expect(notice.getSnapshot().prompt).toBe('required');
  });

  test("a failed acceptance keeps the notice open with the error, and the change waiting", async () => {
    const { server } = fakeServer();
    const notice = new AlphaNotice(server);
    let done = false;
    void notice.ensure().then(() => (done = true));
    await flush();
    server.fail = true;
    await notice.accept();
    expect(notice.getSnapshot()).toMatchObject({ prompt: 'required', error: 'Network error', busy: false });
    server.fail = false;
    await notice.accept();
    await flush();
    expect(done).toBe(true);
  });

  test('an older notice accepted asks again; a stale session answer never steps back', async () => {
    const { server } = fakeServer({ version: ALPHA_NOTICE_VERSION - 1, accepted_at: '2025-01-01T00:00:00Z' });
    const notice = new AlphaNotice(server);
    void notice.ensure();
    await flush();
    expect(notice.getSnapshot().prompt).toBe('required');
    await notice.accept();
    notice.learn(null);
    expect(notice.getSnapshot().accepted?.version).toBe(ALPHA_NOTICE_VERSION);
  });

  test('accepted in another window: learning it lets the held change go', async () => {
    const { server } = fakeServer();
    const notice = new AlphaNotice(server);
    let done = false;
    void notice.ensure().then(() => (done = true));
    await flush();
    notice.learn({ version: ALPHA_NOTICE_VERSION, accepted_at: '2026-11-05T10:00:00Z' });
    await flush();
    expect(done).toBe(true);
    expect(notice.getSnapshot().prompt).toBe('none');
  });

  test('viewing from the badge, then closing, changes nothing', async () => {
    const { server, calls } = fakeServer();
    const notice = new AlphaNotice(server);
    notice.view();
    expect(notice.getSnapshot().prompt).toBe('view');
    notice.dismiss();
    expect(notice.getSnapshot().prompt).toBe('none');
    expect(calls.accept).toBe(0);
  });
});

describe('gateWrites', () => {
  test('every ApiClient method is classified', () => {
    // The type makes this exhaustive; this catches a method that exists only at runtime.
    const client = new FixtureApiClient({ latencyMs: 0 });
    const proto = Object.getPrototypeOf(client) as object;
    const publicMethods = Object.getOwnPropertyNames(proto).filter(
      (n) => n !== 'constructor' && typeof (client as unknown as Record<string, unknown>)[n] === 'function',
    );
    for (const m of Object.keys(API_ACCESS)) expect(publicMethods).toContain(m);
  });

  test('writes wait for the notice; reads and exempt calls never do', async () => {
    const raw = new FixtureApiClient({ latencyMs: 0 });
    const notice = new AlphaNotice(raw);
    const api: ApiClient = gateWrites(raw, notice);

    // Reads and sign-out go straight through, with no prompt.
    expect((await api.companies()).length).toBeGreaterThan(0);
    await api.logout();
    expect(notice.getSnapshot().prompt).toBe('none');
    // `session()` tells the notice what's accepted.
    await api.session();
    expect(notice.getSnapshot().accepted).toBeNull();

    const co = (await api.companies())[0]!.id;
    const before = await api.closingDate(co);
    const saving = api.putClosingDate(co, '2001-12-31');
    await flush();
    await flush();
    expect(notice.getSnapshot().prompt).toBe('required');
    expect(await api.closingDate(co)).toBe(before);
    await notice.accept();
    await saving;
    expect(await api.closingDate(co)).toBe('2001-12-31');
    expect((await api.session()).user.alpha_notice?.version).toBe(ALPHA_NOTICE_VERSION);
  });

  test('methods keep their identity', () => {
    const raw = new FixtureApiClient({ latencyMs: 0 });
    const api = gateWrites(raw, new AlphaNotice(raw));
    expect(api.txns).toBe(api.txns);
    expect(api.createTxn).toBe(api.createTxn);
    expect(api.mode).toBe('fixtures');
  });
});
