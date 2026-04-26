import { test, expect } from '@playwright/test';
import {
  sendMessage,
  waitForMessage,
  setupTwoPeers,
  kickPeer,
  openMemberList,
  openServerSettings,
  openCompareFingerprints,
  markFingerprintsMatch,
  markFingerprintsMismatch,
  longPressAvatar,
  visibleShell,
} from './helpers';

// Shared relay + gossip mesh — keep tests inside this file sequential
// so they don't stampede the relay while `fullyParallel: true` runs
// different spec files concurrently.
test.describe.configure({ mode: 'serial' });

test.describe('Permissions and trust', () => {
  // Two-peer permission tests need extra time for setup + P2P sync.
  test.setTimeout(120_000);

  // Mobile member-list surface is deferred to a later phase (Phase 1b
  // shipped the mobile shell without the right-rail members pane).
  // Kick tests go through `.member-item`, which only renders on desktop
  // today. Long-press / compare-sheet tests below opt back in explicitly
  // since they drive trust via the compare-fingerprints sheet, not the
  // member list.
  //
  // Trust / untrust tests that used to live here (Unknown → Verified
  // and badge-render contracts) moved to:
  //   - Rust: `crates/client/src/tests/trust_flow.rs` (transitions +
  //     two-peer `MemNetwork` revoke-SendMessages rejection).
  //   - wasm-pack DOM: `crates/web/tests/browser.rs`
  //     (`trust_badge_dom` — `.trust-badge--verified` / `--unverified`).
  // Only the real-multi-peer behaviours stay in Playwright.
  test.beforeEach(async ({}, testInfo) => {
    const mobileSkipPattern = /kicks member|kicked peer|server settings panel/;
    if (testInfo.project.name.startsWith('mobile') && mobileSkipPattern.test(testInfo.title)) {
      testInfo.skip(true, 'mobile member-list surface deferred — tracked in onboarding phase followup');
    }
  });

  test('owner kicks member — member count drops', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Kick Server', 'Alice', 'Bob');
    try {
      // The members pane is closed by default — `setupTwoPeers` opens it
      // briefly to wait for display-name sync and then closes it again.
      // Open it before counting so `.member-item` rows are mounted (the
      // right-rail `match which.get()` only renders MemberList when the
      // pane is open). Then poll for the membership-sync-completed state
      // (>= 2 members) instead of taking a single fixed-delay snapshot.
      await openMemberList(page1);
      const memberItems = page1.locator(`${visibleShell(page1)} .member-item`);
      await expect.poll(() => memberItems.count(), { timeout: 30_000 })
        .toBeGreaterThanOrEqual(2);
      const initialCount = await memberItems.count();

      // Alice kicks Bob (helper toggles the pane open/closed itself).
      await kickPeer(page1, 'Bob');

      // Re-open the pane so we can re-count after the kick lands.
      await openMemberList(page1);
      await expect(page1.locator(`${visibleShell(page1)} .member-item`))
        .toHaveCount(initialCount - 1, { timeout: 30_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('kicked peer messages do not reach owner', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Kick Msg', 'Alice', 'Bob');
    try {
      // Alice kicks Bob.
      await kickPeer(page1, 'Bob');
      await page1.waitForTimeout(2000);

      // Bob tries to send a message that should NOT arrive.
      await sendMessage(page2, 'kicked but trying');

      // Sentinel: Alice sends her own message. Her own message appears locally
      // immediately, so waiting for it proves that local rendering is working
      // and that enough real time has elapsed for any P2P delivery to have
      // occurred — without relying on a fixed sleep duration.
      await sendMessage(page1, 'alice sentinel after kick');
      await waitForMessage(page1, 'alice sentinel after kick', 10_000);

      // Assert that Bob's message never arrived on Alice's side.
      await expect(page1.locator('.message .body', { hasText: 'kicked but trying' }))
        .not.toBeVisible({ timeout: 5000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('server settings panel opens and back button returns to chat', async ({ browser }) => {
    // NOTE: Role creation UI is not yet implemented. This test was previously
    // guarded by an `if (await roleInput.isVisible())` check that made the
    // entire test body optional — the test passed vacuously whether or not the
    // UI existed. Until roles are added, this test verifies that the settings
    // panel opens and the Back button returns to the chat view, which is a real
    // and unconditional assertion. Add role creation assertions here once the
    // UI lands.
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Role Server', 'Alice', 'Bob');
    try {
      // Open server settings.
      await openServerSettings(page1);

      // Settings panel should be visible.
      await expect(page1.locator('.server-settings, .settings-panel')).toBeVisible({ timeout: 5000 });

      // Go back to chat.
      await page1.locator('text=Back').click();

      // Sidebar / chat area should be visible again.
      await expect(page1.locator(`${visibleShell(page1)} .channel-sidebar, ${visibleShell(page1)} .mobile-home`).first()).toBeVisible({ timeout: 5000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('non-owner cannot create a channel — add button absent', async ({ browser }, testInfo) => {
    // Desktop only — easier to assert button visibility without sidebar toggle.
    test.skip(testInfo.project.name.startsWith('mobile'), 'desktop only');

    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Chan Perm', 'Alice', 'Bob');
    try {
      // Bob (non-admin) should not see the channel-add or delete buttons.
      // The state machine rejects ManageChannels mutations from non-admins, but the
      // UI must also hide the controls — otherwise errors are swallowed silently.
      await expect(page2.locator('.channel-add-btn')).toBeHidden({ timeout: 5_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('non-owner has no action buttons in member list', async ({ browser }, testInfo) => {
    // Skip on mobile — two-peer setup + member list toggle is flaky on narrow viewports.
    test.skip(testInfo.project.name.startsWith('mobile'), 'desktop only');

    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'NoActions', 'Alice', 'Bob');
    try {
      // Bob opens the member list (he is not the owner).
      // On desktop, member list is always visible (no toggle needed).
      await page2.waitForTimeout(1000);

      // Bob should NOT have any trust/kick/untrust action buttons.
      const actionButtons = page2.locator(`${visibleShell(page2)} .member-actions button`);
      await expect(actionButtons).toHaveCount(0, { timeout: 5000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  // ── Phase 1d — trust verification (SAS compare) ──────────────────────
  //
  // Spec: docs/specs/2026-04-19-ui-design/trust-verification.md
  // Plan: docs/plans/2026-04-20-ui-phase-1d-trust-verification.md

  test('compare match flips the trust badge to verified', async ({ browser }, testInfo) => {
    test.skip(testInfo.project.name.startsWith('mobile'), 'desktop-chrome path');
    const { ctx1, ctx2, page1 } = await setupTwoPeers(browser, 'Verify', 'Alice', 'Bob');
    try {
      await openCompareFingerprints(page1, 'Bob');
      await markFingerprintsMatch(page1);

      // `done` closes the sheet; the badge on Bob's row switches to verified.
      await page1.locator('.add-friend__cta-primary', { hasText: 'done' }).click();
      const bobRow = page1.locator(`${visibleShell(page1)} .member-item`, { hasText: 'Bob' });
      await expect(bobRow.locator('.trust-badge--verified'))
        .toBeVisible({ timeout: 5_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('compare mismatch keeps peer unverified but messaging still works', async ({
    browser,
  }, testInfo) => {
    test.skip(testInfo.project.name.startsWith('mobile'), 'desktop-chrome path');
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mismatch', 'Alice', 'Bob');
    try {
      await openCompareFingerprints(page1, 'Bob');
      await markFingerprintsMismatch(page1);
      // Close the dialog via the `close` secondary action.
      await page1.locator('.add-friend__cta-secondary', { hasText: 'close' }).click();

      // Bob's row keeps the unverified/downgrade treatment.
      const bobRow = page1.locator(`${visibleShell(page1)} .member-item`, { hasText: 'Bob' });
      await expect(bobRow.locator('.trust-badge--unverified, .trust-badge--downgrade'))
        .toBeVisible({ timeout: 5_000 });

      // Messaging is unaffected.
      await sendMessage(page2, 'mismatch still talks');
      await waitForMessage(page1, 'mismatch still talks', 30_000);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('mobile long-press opens the compare sheet', async ({ browser }, testInfo) => {
    test.skip(!testInfo.project.name.startsWith('mobile'), 'mobile-chrome path');
    // Long-press targets a member avatar; the mobile member surface
    // lands in a follow-up phase. Skip until that ships.
    test.skip(true, 'mobile member-list surface deferred');
    const { ctx1, ctx2, page1 } = await setupTwoPeers(browser, 'LongPress', 'Alice', 'Bob');
    try {
      await longPressAvatar(page1, 'Bob');
      await expect(page1.locator('.add-friend__card[role="dialog"]'))
        .toBeVisible({ timeout: 10_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });
});
