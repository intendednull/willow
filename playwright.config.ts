import { defineConfig, devices } from '@playwright/test';

// Use WILLOW_TEST_URL to override. Defaults to local trunk serve.
// For prod: WILLOW_TEST_URL=https://willow.intendednull.com npx playwright test
const BASE_URL = process.env.WILLOW_TEST_URL || 'http://127.0.0.1:8080';

export default defineConfig({
  testDir: './e2e',
  timeout: 60_000,
  retries: Number(process.env.PLAYWRIGHT_RETRIES ?? 1),
  // Per-file + intra-file parallelism. Multi-peer specs opt out via
  // `test.describe.configure({ mode: 'serial' })` inside each file
  // so tests inside a relay-heavy file still stay sequential while
  // different files run concurrently.
  fullyParallel: process.env.PLAYWRIGHT_FULLY_PARALLEL !== '0',
  // Four workers: each launches isolated browser contexts; each test
  // calls `freshStart()` so tests are self-contained. Override with
  // `PLAYWRIGHT_WORKERS` if the relay flakes under load.
  workers: Number(process.env.PLAYWRIGHT_WORKERS ?? 4),
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
      // Dedicated project for WebRTC voice/video media tests. The fake-media
      // flags give Chromium a synthetic mic/camera (no hardware) and
      // auto-grant the getUserMedia/getDisplayMedia permission prompt, so two
      // browser contexts on this machine can establish a *real* peer
      // connection over loopback host candidates (no STUN/TURN needed
      // same-host) and actually exchange media. Used by e2e/voice-video.spec.ts.
      name: 'voice-chrome',
      testMatch: /voice-video\.spec\.ts/,
      use: {
        ...devices['Desktop Chrome'],
        launchOptions: {
          args: [
            '--use-fake-device-for-media-stream',
            '--use-fake-ui-for-media-stream',
            '--autoplay-policy=no-user-gesture-required',
          ],
        },
        permissions: ['microphone', 'camera'],
      },
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
