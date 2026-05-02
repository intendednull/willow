/* eslint-disable no-restricted-syntax -- migration tracked at https://github.com/intendednull/willow/issues/458 */
//
// Peer setup helpers. Extracted from the legacy 703-LOC e2e/helpers.ts
// per docs/plans/2026-04-29-event-based-waits-pr2-peer-wrapper.md Task 7.
// Behaviour is preserved verbatim — sleep removal is a follow-up.

import { Page, Browser, BrowserContext, expect } from '@playwright/test';
import {
  isMobile,
  visibleShell,
  openMemberList,
  closeMemberList,
} from './ui';

/** Wait for the WASM app to load (loading spinner disappears). */
export async function waitForApp(page: Page) {
  // Wait for the app to render (welcome screen, desktop shell, mobile
  // shell, or join page). `:visible` filters out the hidden sibling
  // shell on either side of the 720 px split.
  await page.waitForSelector(
    '.welcome-screen:visible, .shell-desktop .app:visible, .shell-mobile .mobile-top-bar:visible, .join-card:visible',
    { timeout: 30_000 },
  );
}

/** Clear all Willow localStorage keys and IndexedDB databases, then reload. */
export async function freshStart(page: Page) {
  await page.goto('/');
  await page.evaluate(async () => {
    const keys = Object.keys(localStorage).filter(k => k.startsWith('willow_'));
    keys.forEach(k => localStorage.removeItem(k));
    // Also clear non-prefixed keys that might be ours.
    localStorage.clear();

    // Clear Willow-related IndexedDB databases so each test starts from a
    // truly clean state. Without this, identity keys and event stores
    // persisted in IDB survive localStorage.clear() and can leak state
    // between tests running in the same browser context.
    const dbNames = await indexedDB.databases?.() ?? [];
    await Promise.all(
      dbNames
        .filter(db => db.name && (db.name.startsWith('willow') || db.name.startsWith('iroh')))
        .map(db => new Promise<void>((resolve, reject) => {
          const req = indexedDB.deleteDatabase(db.name!);
          req.onsuccess = () => resolve();
          req.onerror = () => reject(req.error);
          req.onblocked = () => resolve(); // Proceed even if blocked.
        }))
    );
  });
  await page.reload();
  await waitForApp(page);
}

/** Walk the two-step welcome flow's name step.
 *  Fills the optional display name and clicks continue to reveal the
 *  Create / Join tabs. No-op if already past step 1.
 */
async function advancePastNameStep(page: Page, displayName?: string) {
  const nameInput = page.locator('.welcome-name-input');
  if (await nameInput.isVisible().catch(() => false)) {
    if (displayName) await nameInput.fill(displayName);
    await page.locator('.welcome-continue-btn').click();
    // Wait for the tab panel to render.
    await page.locator('.welcome-tabs').waitFor({ timeout: 5_000 });
  }
}

/** Create a server from the welcome screen. Returns the server name. */
export async function createServer(page: Page, name: string, displayName?: string) {
  await expect(page.locator('.welcome-card')).toBeVisible();
  await advancePastNameStep(page, displayName);

  // Create tab is selected by default — fill the grove name and click
  // the panel's continue button to commit. Scoped to .welcome-tab-panel
  // to avoid matching step 1's continue button from earlier steps.
  await page
    .locator('.welcome-tab-panel input[placeholder="backyard"]')
    .fill(name);
  await page.locator('.welcome-tab-panel button', { hasText: 'continue' }).click();

  // Wait for the app to load with the new server. On mobile we then
  // push into the first channel (`general`) so subsequent helpers
  // (`sendMessage`, `openMemberList`, etc.) find the composer +
  // right-rail surfaces — mobile home only shows the channel list.
  if (isMobile(page)) {
    await page.waitForSelector('.mobile-top-bar', { state: 'visible', timeout: 10_000 });
    // Tap general to push the channel surface (which carries the
    // composer, message list, and main-pane-header action bar).
    const generalRow = page
      .locator(`${visibleShell(page)} .mobile-home .channel-item`, { hasText: 'general' });
    if (await generalRow.count() > 0) {
      await generalRow.first().click();
      await page.waitForSelector('.mobile-push--channel', { timeout: 10_000 });
    }
  } else {
    await page.waitForSelector('.main-pane-header, .channel-sidebar', {
      state: 'visible',
      timeout: 10_000,
    });
  }
}

/** Get the full peer ID from the welcome screen or settings.
 *
 *  Optionally accepts a `displayName` to fill into the welcome step-1
 *  name input before advancing. Required when the caller intends to
 *  `joinViaInvite` afterward: the name input is unmounted once step 2
 *  renders, so a later `advancePastNameStep(displayName)` no-ops and
 *  the join confirm closure reads an empty `display_name` signal,
 *  broadcasting the literal "anonymous" fallback to peers.
 */
export async function getPeerId(page: Page, displayName?: string): Promise<string> {
  // Welcome screen: advance past step 1 (with optional name), then
  // switch to the Join tab — the peer id lives inside the Join step
  // list, hidden by default and revealed by the eye-toggle icon.
  if (await page.locator('.welcome-card').isVisible().catch(() => false)) {
    await advancePastNameStep(page, displayName);
    const joinTab = page.locator('.welcome-tab-btn', { hasText: 'Join' });
    if (await joinTab.isVisible().catch(() => false)) {
      await joinTab.click();
      const revealBtn = page.locator('button[aria-label="show full peer id"]');
      await revealBtn.waitFor({ timeout: 5_000 });
      await revealBtn.click();
    }
    const peerIdEl = page.locator('.welcome-join-steps__full-id').first();
    if (await peerIdEl.isVisible().catch(() => false)) {
      return (
        (await peerIdEl.getAttribute('data-full-id')) ||
        (await peerIdEl.textContent()) ||
        ''
      );
    }
  }

  // Fallback: read it from settings.
  await page.locator('text=Settings').click();
  await page.waitForTimeout(300);
  const settingsPeerId = page.locator('.peer-id-text').first();
  return (
    (await settingsPeerId.getAttribute('data-full-id')) ||
    (await settingsPeerId.textContent()) ||
    ''
  );
}

/** Opens the server settings panel (opens sidebar first on mobile). */
export async function openServerSettings(page: Page) {
  if (isMobile(page)) {
    // Channel list is on the home tab; the gear lives in the sidebar
    // header rendered inside `.mobile-home`. No drawer needed.
    const backSlot = page.locator('.mobile-top-bar .top-slot-left .top-back');
    while (await backSlot.isVisible().catch(() => false)) {
      await page.locator('.mobile-top-bar .top-slot-left').click();
      await page.waitForTimeout(300);
    }
    await page.locator('.mobile-tab-bar .tab[data-tab="home"]').click();
    await page.waitForTimeout(200);
  }
  // The grove-header button in `.channel-sidebar` is the server-settings
  // entry point on both shells (it fires `on_server_settings_click`).
  // The legacy `.server-gear-btn` was removed by the vibe-annotations
  // pass (commit 0861f26) — keep the selector aligned with the markup.
  await page.locator(`${visibleShell(page)} .channel-sidebar .grove-header`).first().click();
  await page.locator('.settings-panel, .settings-overlay').first()
    .waitFor({ timeout: 5_000 });
}

/** Generates an invite code for a given peer ID. Returns the invite code string. */
export async function generateInvite(page: Page, recipientPeerId: string): Promise<string> {
  await openServerSettings(page);
  await page.locator('input[placeholder*="12D3KooW"]').fill(recipientPeerId);
  await page.locator('button', { hasText: 'Generate Invite' }).click();
  await page.waitForTimeout(500);
  const inviteCode = await page.locator('.invite-code-display textarea').inputValue();
  await page.locator('text=Back').click();
  await page.waitForTimeout(500);
  return inviteCode;
}

/** Joins a server via invite code from the welcome screen.
 *  The welcome flow asks for the display name up-front on step 1 (before
 *  the Create / Join tabs), so displayName is consumed there.
 */
export async function joinViaInvite(page: Page, inviteCode: string, displayName?: string) {
  await advancePastNameStep(page, displayName);
  // Switch to the Join tab.
  await page.locator('.welcome-tab-btn', { hasText: 'Join' }).click();
  await page.locator('.welcome-invite-input').waitFor({ timeout: 5_000 });
  await page.locator('.welcome-invite-input').fill(inviteCode);
  await page.locator('.welcome-tab-panel button', { hasText: 'continue' }).click();
  // Wait for the confirmation step ("Join grove") to appear.
  await page.locator('button', { hasText: 'Join grove' }).waitFor({ timeout: 5_000 });
  await page.locator('button', { hasText: 'Join grove' }).click();
  if (isMobile(page)) {
    await page.waitForSelector('.mobile-top-bar', { state: 'visible', timeout: 20_000 });
  } else {
    await page.waitForSelector('.main-pane-header, .channel-sidebar', {
      state: 'visible',
      timeout: 20_000,
    });
  }
  // Deterministic post-join settle: wait for the sidebar + first
  // channel to materialise. Covers both shells. The channel-item
  // wait depends on the SyncBatch round-trip landing — when two
  // workers are bootstrapping their relay handshake at the same
  // time the first pair can take 30–50 s before iroh-gossip dials
  // through. 60 s mirrors `waitUntilHeadsEqual`'s default cold-start
  // budget; warm tests still settle in <5 s.
  await page.locator(`${visibleShell(page)} .channel-sidebar, ${visibleShell(page)} .mobile-home`)
    .first()
    .waitFor({ timeout: 20_000 });
  await page.locator(`${visibleShell(page)} .channel-item`).first()
    .waitFor({ timeout: 60_000 });
}

/** Sets up two peers: peer1 creates a server, peer2 joins via invite. */
export async function setupTwoPeers(
  browser: Browser,
  serverName = 'Test Server',
  peer1Name = 'Alice',
  peer2Name = 'Bob',
): Promise<{ ctx1: BrowserContext; ctx2: BrowserContext; page1: Page; page2: Page }> {
  const ctx1 = await browser.newContext();
  const ctx2 = await browser.newContext();
  const page1 = await ctx1.newPage();
  const page2 = await ctx2.newPage();

  // Peer 1: Create server.
  await freshStart(page1);
  await createServer(page1, serverName, peer1Name);

  // Peer 2: Get peer ID from welcome screen. Pass `peer2Name` so step 1
  // captures the display name BEFORE the input unmounts — otherwise
  // `joinViaInvite`'s own `advancePastNameStep` no-ops, leaving the
  // welcome `display_name` signal empty and the join broadcasts the
  // literal "anonymous" fallback (add_server.rs:163-167) to peer 1.
  await freshStart(page2);
  const peer2Id = await getPeerId(page2, peer2Name);

  // Peer 1: Generate invite for peer 2.
  const inviteCode = await generateInvite(page1, peer2Id);

  // Peer 2: Join the server.
  await joinViaInvite(page2, inviteCode, peer2Name);

  // Wait for display name sync: peer2's name should appear in peer1's member list.
  // Only do this on desktop — Phase 1b's mobile shell does not yet
  // surface the member list, and the display-name sync completes
  // just as reliably via gossip events consumed by other helpers.
  if (peer2Name && !isMobile(page1)) {
    await openMemberList(page1);
    try {
      await page1
        .locator('.member-item', { hasText: peer2Name })
        .waitFor({ timeout: 20_000 });
    } catch {
      // Display name sync may be slow; proceed anyway — but warn so failures
      // here don't produce misleading timeouts in downstream assertions.
      console.warn('[setupTwoPeers] peer2 display name did not sync in time — P2P may be slow');
    }
    await closeMemberList(page1);
  } else if (peer2Name) {
    // On mobile, just sleep a bit to let gossip propagate.
    await page1.waitForTimeout(1500);
  }

  return { ctx1, ctx2, page1, page2 };
}
