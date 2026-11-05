// Smoke tests against fixture mode (no server needed). Two workers: the machine
// is shared (see the compute budget in plans/web-ui.md).

import { defineConfig, devices } from '@playwright/test';
import { ALPHA_NOTICE_VERSION, FIXTURE_ALPHA_KEY } from './src/alpha/version';

const port = 5740;

/**
 * Every test starts with the alpha notice already accepted, so a save doesn't
 * stop to ask; the notice's own test clears this.
 */
const alphaAccepted = {
  cookies: [],
  origins: [
    {
      origin: `http://localhost:${port}`,
      localStorage: [{ name: FIXTURE_ALPHA_KEY, value: JSON.stringify({ version: ALPHA_NOTICE_VERSION, accepted_at: '2026-01-01T00:00:00Z' }) }],
    },
  ],
};

export default defineConfig({
  testDir: 'e2e',
  workers: 2,
  fullyParallel: true,
  retries: 0,
  timeout: 45_000,
  expect: { timeout: 8_000 },
  reporter: [['list']],
  use: {
    baseURL: `http://localhost:${port}`,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    storageState: alphaAccepted,
  },
  webServer: {
    command: 'bun run dev',
    url: `http://localhost:${port}`,
    reuseExistingServer: true,
    timeout: 120_000,
    env: { VITE_FIXTURES: '1' },
  },
  projects: [
    {
      name: 'desktop',
      testMatch: /desktop\.spec\.ts/,
      use: { ...devices['Desktop Chrome'], viewport: { width: 1366, height: 860 } },
    },
    {
      // 375px: the narrowest phone we design for (iPhone SE / mini).
      name: 'mobile',
      testMatch: /mobile\.spec\.ts/,
      use: {
        browserName: 'chromium',
        viewport: { width: 375, height: 740 },
        deviceScaleFactor: 2,
        isMobile: true,
        hasTouch: true,
      },
    },
  ],
});
