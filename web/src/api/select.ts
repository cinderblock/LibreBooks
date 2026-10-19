// Which API client this page load uses.
//
// Fixture mode is on with `?fixtures=1` (or `=stress` for a big data set) or
// `VITE_FIXTURES=1` at build/dev time. The query flag is remembered for the tab
// in sessionStorage, because in-app navigation drops the query string; `?fixtures=0`
// turns it back off.

import type { ApiClient } from './client';
import { FixtureApiClient } from './fixtures/client';
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

/** Desktop local mode injects its per-launch token before the SPA loads. */
function desktopToken(): string | null {
  const w = window as unknown as { __LIBREBOOKS_TOKEN__?: string };
  return w.__LIBREBOOKS_TOKEN__ ?? null;
}

export function createApiClient(): ApiClient {
  const mode = fixtureMode();
  if (mode !== 'off') {
    const latency = Number(new URLSearchParams(window.location.search).get('latency') ?? '90');
    const client = new FixtureApiClient({ stress: mode === 'stress', latencyMs: Number.isFinite(latency) ? latency : 90 });
    // Console/e2e handle: `__librebooks.simulateRemoteEdit(id)`, `__librebooks.find(…)`.
    (window as unknown as { __librebooks?: FixtureApiClient }).__librebooks = client;
    return client;
  }
  return new HttpApiClient({ token: desktopToken() });
}
