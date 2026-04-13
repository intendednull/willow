import { defineConfig, devices } from '@playwright/test';

// Use WILLOW_TEST_URL to override. Defaults to local trunk serve.
// For prod: WILLOW_TEST_URL=https://willow.intendednull.com npx playwright test
const BASE_URL = process.env.WILLOW_TEST_URL || 'http://127.0.0.1:8080';

export default defineConfig({
  testDir: './e2e',
  timeout: 60_000,
  retries: 1,
  workers: 1, // Sequential — tests share server state
  use: {
    baseURL: BASE_URL,
    headless: true,
    screenshot: 'only-on-failure',
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'desktop-chrome',
      use: { ...devices['Desktop Chrome'] },
    },
    {
      name: 'mobile-chrome',
      use: { ...devices['Pixel 7'] },
    },
    {
      name: 'desktop-firefox',
      use: {
        browserName: 'firefox',
        viewport: { width: 1280, height: 720 },
        hasTouch: false,
      },
    },
    {
      name: 'mobile-firefox',
      // Firefox doesn't support isMobile, so we use a small viewport
      // and dispatch touch events manually in tests.
      use: {
        browserName: 'firefox',
        viewport: { width: 412, height: 915 },
        hasTouch: true,
      },
    },
  ],
});
