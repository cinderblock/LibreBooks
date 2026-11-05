// The alpha-software notice: LibreBooks asks each person to accept it before
// their first change to any books (plans/alpha-notice.md).
//
// The server remembers who accepted which version (`GET /session` →
// `user.alpha_notice`, `POST /alpha-notice`), per person, or per computer for
// the desktop app's own session. This module is the page's view of that, plus
// the prompt: `ensure()` resolves once the current version is accepted, opening
// the notice and waiting for the person when it isn't. `gateWrites`
// (`~/api/access.ts`) calls it before every state-changing request, so a save
// waits for the answer instead of being lost: accept, and it carries on; cancel,
// and it fails with an `alpha_notice` error the form shows like any other.

import type { ApiClient } from '~/api/client';
import { ApiError } from '~/api/errors';
import type { NoticeAcceptance } from '~/api/types';
import { ALPHA_NOTICE_VERSION } from './version';

export { ALPHA_NOTICE_VERSION };

/** Why a held change didn't go ahead. */
export const DECLINED_MESSAGE = "Not saved: LibreBooks is alpha software, and its notice wasn't accepted. Save again to read it.";

export interface AlphaNoticeState {
  /** What's accepted, as the server last said; `undefined` until it has said. */
  accepted: NoticeAcceptance | null | undefined;
  /** `required`: a change is waiting on the answer. `view`: opened to read (the Alpha badge). */
  prompt: 'none' | 'required' | 'view';
  /** Accepting failed (the server couldn't be reached, say); the notice stays open. */
  error: string | null;
  /** The acceptance is being saved. */
  busy: boolean;
}

type Client = Pick<ApiClient, 'session' | 'acceptAlphaNotice'>;

export function isCurrent(a: NoticeAcceptance | null | undefined): a is NoticeAcceptance {
  return !!a && a.version >= ALPHA_NOTICE_VERSION;
}

export class AlphaNotice {
  private state: AlphaNoticeState = { accepted: undefined, prompt: 'none', error: null, busy: false };
  private readonly listeners = new Set<() => void>();
  /** The one question in flight: every change made meanwhile waits on the same answer. */
  private pending: Promise<void> | null = null;
  private answer: { resolve(): void; reject(e: unknown): void } | null = null;

  /** `client` is the raw API client, not the gated one (it must not wait on itself). */
  constructor(private readonly client: Client) {}

  // ---- for React (useSyncExternalStore) ----
  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  readonly getSnapshot = (): AlphaNoticeState => this.state;

  private set(patch: Partial<AlphaNoticeState>) {
    this.state = { ...this.state, ...patch };
    for (const l of this.listeners) l();
  }

  /** What the server says is accepted (every `GET /session` answer passes through here). */
  learn(accepted: NoticeAcceptance | null | undefined) {
    if (accepted === undefined) return;
    const known = this.state.accepted;
    // Never step back: a session read before an acceptance can answer after it.
    if (known && (!accepted || accepted.version < known.version)) return;
    this.set({ accepted });
    if (isCurrent(accepted) && this.state.prompt === 'required') this.settle(null);
  }

  /**
   * Resolves once the current notice is accepted; rejects with an `alpha_notice`
   * `ApiError` if the person cancels. Asks the server first when this page
   * doesn't know yet (or another window may have accepted since).
   */
  ensure(): Promise<void> {
    if (isCurrent(this.state.accepted)) return Promise.resolve();
    this.pending ??= this.ask().finally(() => {
      this.pending = null;
    });
    return this.pending;
  }

  private async ask(): Promise<void> {
    try {
      this.learn((await this.client.session()).user.alpha_notice ?? null);
    } catch {
      // Can't tell: ask. Accepting tells the server anyway.
    }
    if (isCurrent(this.state.accepted)) return;
    const answered = new Promise<void>((resolve, reject) => {
      this.answer = { resolve, reject };
    });
    this.set({ prompt: 'required', error: null });
    return answered;
  }

  /** Open the notice to read it (the Alpha badge). Doesn't replace a pending question. */
  view() {
    if (this.state.prompt === 'none') this.set({ prompt: 'view', error: null });
  }

  /** The person accepted: tell the server, then let any held changes go ahead. */
  async accept(): Promise<void> {
    if (this.state.busy) return;
    this.set({ busy: true, error: null });
    try {
      const accepted = await this.client.acceptAlphaNotice(ALPHA_NOTICE_VERSION);
      this.set({ busy: false });
      this.learn(accepted);
      if (this.state.prompt === 'view') this.set({ prompt: 'none' });
    } catch (e) {
      this.set({ busy: false, error: e instanceof Error ? e.message : String(e) });
    }
  }

  /** Closed without accepting: held changes fail; reading it changes nothing. */
  dismiss() {
    if (this.state.prompt === 'required') {
      this.settle(new ApiError(428, 'alpha_notice', DECLINED_MESSAGE));
    } else {
      this.set({ prompt: 'none', error: null });
    }
  }

  private settle(error: ApiError | null) {
    const answer = this.answer;
    this.answer = null;
    this.set({ prompt: 'none', error: null, busy: false });
    if (error) answer?.reject(error);
    else answer?.resolve();
  }
}
