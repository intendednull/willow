import { defineConfig, devices } from '@playwright/test';

const BASE_URL = process.env.WILLOW_TEST_URL || 'https://willow.intendednull.com';

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
  ],
});
