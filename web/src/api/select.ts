// Which API client this page load uses.
//
// Fixture mode is on with `?fixtures=1` (or `=stress` for a big data set) or
// `VITE_FIXTURES=1` at build/dev time. The query flag is remembered for the tab
// in sessionStorage, because in-app navigation drops the query string; `?fixtures=0`
// turns it back off. The fixture books load with a dynamic import, so a
// production page that never asks for them doesn't download them.

import type { ApiClient } from './client';
import type { FixtureApiClient } from './fixtures/client';
import { HttpApiClient } from './http';

const KEY = 'librebooks.fixtures';

export type FixtureMode = 'off' | 'on' | 'stress';

export function fixtureMode(): FixtureMode {
  const params = new URLSearchParams(window.location.search);
  const q = params.get('fixtures');
  if (q !== null) {
    const mode: FixtureMode = q === '0' || q === 'false' ? 'off' : q === 'stress' ? 'stress' : 'on';
    sessionStorage.setItem(KEY, mode);
    return mode;
  }
  const stored = sessionStorage.getItem(KEY) as FixtureMode | null;
  if (stored) return stored;
  return import.meta.env.VITE_FIXTURES === '1' ? 'on' : 'off';
}

/**
 * What the desktop app injects before the SPA loads: its mode, the per-launch
 * bearer token for its in-process server, and where that server is.
 */
export interface DesktopInjection {
  mode: 'desktop';
  token?: string | null;
  /** The server root (`http://127.0.0.1:8740`) or the API base (`…/api/v1`). */
  apiBase?: string | null;
}

export function desktopInjection(): DesktopInjection | null {
  const w = window as unknown as { __LIBREBOOKS__?: DesktopInjection };
  return w.__LIBREBOOKS__?.mode === 'desktop' ? w.__LIBREBOOKS__ : null;
}

function apiBaseUrl(base: string | null | undefined): string | undefined {
  if (!base) return undefined;
  const trimmed = base.replace(/\/+$/, '');
  return /\/api\/v\d+$/.test(trimmed) ? trimmed : `${trimmed}/api/v1`;
}

let singleton: ApiClient | null = null;

/**
 * Create the page's one client (call once, before rendering). A singleton
 * rather than per-render state because React may run state initializers twice
 * (StrictMode), and fixture mode must not end up with two sets of books (one in
 * the app, another on the console handle).
 */
export async function initApiClient(): Promise<ApiClient> {
  singleton ??= await createApiClient();
  return singleton;
}

export function getApiClient(): ApiClient {
  if (!singleton) throw new Error('initApiClient() must run before the app renders');
  return singleton;
}

async function createApiClient(): Promise<ApiClient> {
  const mode = fixtureMode();
  if (mode !== 'off') {
    const { FixtureApiClient } = await import('./fixtures/client');
    const latency = Number(new URLSearchParams(window.location.search).get('latency') ?? '90');
    const client = new FixtureApiClient({ stress: mode === 'stress', latencyMs: Number.isFinite(latency) ? latency : 90 });
    // Console/e2e handle: `__librebooks.simulateRemoteEdit(id)`, `__librebooks.find(…)`.
    (window as unknown as { __librebooks?: FixtureApiClient }).__librebooks = client;
    return client;
  }
  const desktop = desktopInjection();
  return new HttpApiClient({ token: desktop?.token ?? null, baseUrl: apiBaseUrl(desktop?.apiBase) });
}
