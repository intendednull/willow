import { test, expect } from './test-hooks';
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
  visibleShell,
} from './helpers';

// Shared relay + gossip mesh — keep tests inside this file sequential
// so they don't stampede the relay while `fullyParallel: true` runs
// different spec files concurrently.
test.describe.configure({ mode: 'serial' });

test.describe('Permissions and trust', () => {
  // Two-peer permission tests share the setupTwoPeers + joinViaInvite
  // path with multi-peer-sync. After 7f88280 bumped joinViaInvite's
  // post-join `.channel-item` wait to 60 s for slow-CI gossip, the
  // compounded budget for setup + member-list poll + kick + re-poll
  // reliably runs past 120 s on CI under load. Match the 180 s ceiling
  // already used by multi-peer-sync.spec.ts and multi-peer-mobile.spec.ts.
  test.setTimeout(180_000);

  // Mobile member-list surface is deferred to a later phase (Phase 1b
  // shipped the mobile shell without the right-rail members pane).
  // Kick + compare-sheet tests go through `.member-item`, which only
  // renders on desktop today, so they're skipped on mobile projects.
  // Mobile long-press coverage tracked in #595.
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

  test('kicked peer messages do not reach owner', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Kick Msg', 'Alice', 'Bob');
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Alice kicks Bob.
      await kickPeer(page1, 'Bob');

      // Wait for Bob's DAG to converge with Alice's — once heads match,
      // Bob's local state has applied the kick event and any send he
      // attempts will be locally rejected (no SendMessages permission).
      await bob.waitUntilHeadsEqual(alice);

      // Bob tries to send a message that should NOT arrive. Bypass
      // `sendMessage` because, post-kick, Bob's own broadcast is
      // rejected by the local DAG, so the message body never renders
      // locally and the helper's input-clear wait would time out.
      const bobInput = page2
        .locator(`${visibleShell(page2)} .input-area input, ${visibleShell(page2)} .input-area textarea`)
        .first();
      await bobInput.fill('kicked but trying');
      await bobInput.press('Enter');

      // Sentinel: Alice sends her own message. Her own message appears locally
      // immediately, so waiting for it proves that local rendering is working
      // and that enough real time has elapsed for any P2P delivery to have
      // occurred — without relying on a fixed sleep duration.
      await sendMessage(page1, 'alice sentinel after kick');
      await waitForMessage(page1, 'alice sentinel after kick');

      // Assert that Bob's message never arrived on Alice's side.
      await expect(page1.locator('.message .body', { hasText: 'kicked but trying' }))
        .not.toBeVisible();
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

  test('non-owner has no action buttons in member list', async ({ peer, browser }, testInfo) => {
    // Skip on mobile — two-peer setup + member list toggle is flaky on narrow viewports.
    test.skip(testInfo.project.name.startsWith('mobile'), 'desktop only');

    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'NoActions', 'Alice', 'Bob');
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Wait for membership events to converge before asserting on the
      // member list (Bob's row has to be rendered for `.member-actions`
      // to mean anything).
      await bob.waitUntilHeadsEqual(alice);

      // Bob should NOT have any trust/kick/untrust action buttons.
      const actionButtons = page2.locator(`${visibleShell(page2)} .member-actions button`);
      await expect(actionButtons).toHaveCount(0);
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
    peer,
    browser,
  }, testInfo) => {
    test.skip(testInfo.project.name.startsWith('mobile'), 'desktop-chrome path');
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mismatch', 'Alice', 'Bob');
    const alice = await peer(page1, 'Alice');
    const _bob = await peer(page2, 'Bob');
    try {
      await openCompareFingerprints(page1, 'Bob');
      await markFingerprintsMismatch(page1);
      // Close the dialog via the `close` secondary action.
      await page1.locator('.add-friend__cta-secondary', { hasText: 'close' }).click();

      // Bob's row keeps the unverified/downgrade treatment.
      const bobRow = page1.locator(`${visibleShell(page1)} .member-item`, { hasText: 'Bob' });
      await expect(bobRow.locator('.trust-badge--unverified, .trust-badge--downgrade'))
        .toBeVisible();

      // Messaging is unaffected. Wait for the cross-peer
      // MessageReceived event before asserting the rendered body.
      await sendMessage(page2, 'mismatch still talks');
      await alice.nextEvent(e =>
        e.kind === 'MessageReceived' &&
        !e.isLocal
      );
      await waitForMessage(page1, 'mismatch still talks');
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });
});
