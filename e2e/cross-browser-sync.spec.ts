import { existsSync } from 'node:fs';
import { chromium, firefox, devices } from '@playwright/test';
import { test, expect } from './test-hooks';
import { freshStart, createServer, sendMessage, waitForMessage, getPeerId, openSidebar, closeSidebar, openServerSettings, joinViaInvite, visibleShell } from './helpers';

// Custom Firefox context options — avoids flakiness seen with the full
// devices['Desktop Firefox'] preset (which sets a Windows UA + specific screen
// dimensions that appear to slow gossip mesh formation, cause unknown).
// Using a plain viewport gives consistent behaviour.
const desktopFirefoxContext = {
  viewport: { width: 1280, height: 720 },
  hasTouch: false,
};

// Probe whether the Firefox browser binary Playwright expects is actually
// installed. `firefox.executablePath()` always returns the expected path
// string even when the binary hasn't been downloaded — so we have to stat
// the file to confirm it's actually present. `scripts/setup-e2e.sh` only
// installs Chromium; without this guard these tests fail in ~200ms instead
// of skipping cleanly. See issue #103.
function firefoxAvailable(): boolean {
  try {
    const p = firefox.executablePath();
    return p !== '' && existsSync(p);
  } catch {
    return false;
  }
}
const FIREFOX_SKIP_REASON = 'Firefox not installed — install via `npx playwright install firefox` to enable';

// Shared relay + gossip mesh — keep tests inside this file sequential
// so they don't stampede the relay while `fullyParallel: true` runs
// different spec files concurrently.
test.describe.configure({ mode: 'serial' });

/**
 * Cross-browser sync tests.
 *
 * These tests launch DIFFERENT browser types (e.g., mobile Chrome + desktop Firefox)
 * to replicate real-world cross-browser P2P connectivity.
 * They do NOT use the Playwright project's browser fixture — they launch browsers directly.
 */
test.describe('Cross-browser peer sync', () => {
  // These tests are slow — they launch two separate browser engines.
  // Per-test deadline (not a sleep) — Firefox's iroh bootstrap is
  // measurably slower than Chromium on a freshly-spun mesh, so the
  // join-side `joinViaInvite.channel-item` wait commonly runs past the
  // legacy 120 s budget before SyncBatch lands.
  test.setTimeout(180_000);

  // Only run from one project to avoid duplicating (each test launches its own browsers).
  test.beforeEach(({}, testInfo) => {
    test.skip(testInfo.project.name !== 'desktop-chrome', 'cross-browser tests run once from desktop-chrome');
    test.skip(!firefoxAvailable(), FIREFOX_SKIP_REASON);
  });

  test('mobile Chrome to desktop Firefox — invite + messaging', async ({ peer }) => {
    // Launch mobile Chrome (Pixel 7 viewport).
    const mobileBrowser = await chromium.launch();
    const mobileCtx = await mobileBrowser.newContext({
      ...devices['Pixel 7'],
    });
    const mobilePage = await mobileCtx.newPage();

    // Launch desktop Firefox.
    const desktopBrowser = await firefox.launch();
    const desktopCtx = await desktopBrowser.newContext({
      ...desktopFirefoxContext,
    });
    const desktopPage = await desktopCtx.newPage();

    // Wire test-hooks BEFORE the first goto on each page (addInitScript
    // only takes effect on subsequent loads).
    const mobile = await peer(mobilePage, 'Mobile');
    const desktop = await peer(desktopPage, 'Desktop');

    try {
      // Desktop Firefox: create server.
      await freshStart(desktopPage);
      await createServer(desktopPage, 'CrossBrowser Test', 'DesktopUser');

      // Mobile Chrome: get peer ID from welcome screen. Pass the
      // display name so step 1 captures it before the input unmounts —
      // otherwise the join broadcasts the literal "anonymous" fallback.
      await freshStart(mobilePage);
      const mobilePeerId = await getPeerId(mobilePage, 'MobileUser');
      expect(mobilePeerId).toBeTruthy();

      // Desktop Firefox: open the grove menu (scoped to the visible
      // desktop shell so the duplicate `.shell-mobile` button doesn't
      // trigger Playwright's strict-mode duplicate-match guard).
      await desktopPage.locator('.shell-desktop [aria-label="grove menu"]').first().click();
      // Settings panel mounts after the click.
      await desktopPage.locator('input[placeholder*="12D3KooW"]')
        .waitFor({ timeout: 10_000 });
      await desktopPage.locator('input[placeholder*="12D3KooW"]').fill(mobilePeerId);
      await desktopPage.locator('button', { hasText: 'Generate Invite' }).click();
      // Wait for the invite-code field to mount with a value.
      const inviteField = desktopPage.locator('.invite-code-display textarea');
      await expect(inviteField).not.toHaveValue('');
      const inviteCode = await inviteField.inputValue();
      expect(inviteCode).toBeTruthy();

      // Desktop Firefox: go back to chat.
      await desktopPage.locator('text=Back').click();
      // Wait for the channel sidebar to mount before continuing.
      await desktopPage.locator(`${visibleShell(desktopPage)} .channel-sidebar`)
        .first().waitFor({ timeout: 10_000 });

      // Mobile Chrome: join via invite.
      await joinViaInvite(mobilePage, inviteCode, 'MobileUser');

      // Wait for Mobile's DAG to converge with Desktop's — the
      // post-join initial sync delivers the channel events. After
      // convergence, every channel from Desktop's state is in Mobile's
      // local DAG and DOM checks run with the default 5s timeout.
      await mobile.waitUntilHeadsEqual(desktop);

      // Open the grove drawer briefly to confirm the channel is
      // visible, then close it (`sendMessage` below pushes into the
      // channel via the home tab, which the drawer overlay would
      // otherwise sit on top of and block). Use the helper so the
      // close path is the same backdrop-click `closeSidebar` uses
      // elsewhere — the previous `.grove-drawer__close, .top-slot-left`
      // composite locator picked up `.top-slot-left` (which OPENS
      // the drawer further) when the dedicated close button was
      // missing, hanging until the test deadline.
      await openSidebar(mobilePage);
      await expect(mobilePage.locator(`${visibleShell(mobilePage)} .channel-item`, { hasText: 'general' }))
        .toBeVisible();
      await closeSidebar(mobilePage);

      // Establish bidirectional gossip mesh: Mobile → Desktop is the
      // reliable direction. Waiting for Desktop's MessageReceived event
      // proves both gossip paths are open.
      await sendMessage(mobilePage, 'warmup');
      await desktop.nextEvent(e =>
        e.kind === 'MessageReceived' &&
        !e.isLocal,
        { timeout: 30_000 },
      );

      // Desktop Firefox: send a message; mobile waits for the event.
      await sendMessage(desktopPage, 'Hello from Firefox desktop');
      await mobile.nextEvent(e =>
        e.kind === 'MessageReceived' &&
        !e.isLocal,
        { timeout: 30_000 },
      );
      await waitForMessage(mobilePage, 'Hello from Firefox desktop');

      // Mobile Chrome: send a reply; desktop waits for the event.
      await sendMessage(mobilePage, 'Hello from Chrome mobile');
      await desktop.nextEvent(e =>
        e.kind === 'MessageReceived' &&
        !e.isLocal,
        { timeout: 30_000 },
      );
      await waitForMessage(desktopPage, 'Hello from Chrome mobile');

    } finally {
      await mobileCtx.close();
      await mobileBrowser.close();
      await desktopCtx.close();
      await desktopBrowser.close();
    }
  });

  test('mobile Chrome to desktop Firefox — server owner sends, joiner receives', async ({ peer }) => {
    const mobileBrowser = await chromium.launch();
    const mobileCtx = await mobileBrowser.newContext({
      ...devices['Pixel 7'],
    });
    const mobilePage = await mobileCtx.newPage();

    const desktopBrowser = await firefox.launch();
    const desktopCtx = await desktopBrowser.newContext({
      ...desktopFirefoxContext,
    });
    const desktopPage = await desktopCtx.newPage();

    const mobile = await peer(mobilePage, 'Mobile');
    const desktop = await peer(desktopPage, 'Desktop');

    try {
      // Mobile Chrome creates the server this time.
      await freshStart(mobilePage);
      await createServer(mobilePage, 'Mobile Server', 'MobileUser');

      // Desktop Firefox gets peer ID. Pass the display name so step 1
      // captures it before the input unmounts.
      await freshStart(desktopPage);
      const desktopPeerId = await getPeerId(desktopPage, 'DesktopUser');
      expect(desktopPeerId).toBeTruthy();

      // Mobile Chrome: open the server settings via the grove menu.
      // The grove menu button lives in `.channel-sidebar .grove-header`
      // which is mounted on the home tab — pop any push first and tap
      // home so the click doesn't fall behind the drawer overlay.
      await openServerSettings(mobilePage);
      await mobilePage.locator('input[placeholder*="12D3KooW"]')
        .waitFor({ timeout: 10_000 });
      await mobilePage.locator('input[placeholder*="12D3KooW"]').fill(desktopPeerId);
      await mobilePage.locator('button', { hasText: 'Generate Invite' }).click();
      const inviteField = mobilePage.locator('.invite-code-display textarea');
      await expect(inviteField).not.toHaveValue('');
      const inviteCode = await inviteField.inputValue();
      expect(inviteCode).toBeTruthy();

      // Mobile Chrome: go back.
      await mobilePage.locator('text=Back').click();
      // Wait for the home tab to be visible again before continuing.
      await mobilePage.locator(`${visibleShell(mobilePage)} .mobile-home`)
        .first().waitFor({ timeout: 10_000 });

      // Desktop Firefox: join via invite.
      await joinViaInvite(desktopPage, inviteCode, 'DesktopUser');

      // Wait for Desktop's DAG to converge with Mobile's.
      await desktop.waitUntilHeadsEqual(mobile);
      await expect(desktopPage.locator(`${visibleShell(desktopPage)} .channel-item`, { hasText: 'general' }))
        .toBeVisible();

      // Mobile sends a message; desktop waits for the event.
      await sendMessage(mobilePage, 'Cross browser works!');
      await desktop.nextEvent(e =>
        e.kind === 'MessageReceived' &&
        !e.isLocal,
        { timeout: 30_000 },
      );
      await waitForMessage(desktopPage, 'Cross browser works!');

    } finally {
      await mobileCtx.close();
      await mobileBrowser.close();
      await desktopCtx.close();
      await desktopBrowser.close();
    }
  });
});
