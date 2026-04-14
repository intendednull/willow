import { test, expect } from '@playwright/test';
import {
  sendMessage,
  waitForMessage,
  setupTwoPeers,
  trustPeer,
  untrustPeer,
  kickPeer,
  openServerSettings,
  openMemberList,
  closeMemberList,
} from './helpers';

test.describe('Permissions and trust', () => {
  // Two-peer permission tests need extra time for setup + P2P sync.
  test.setTimeout(120_000);

  test('owner trusts peer — trusted badge appears', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Trust Server', 'Alice', 'Bob');
    try {
      // Alice trusts Bob.
      await trustPeer(page1, 'Bob');

      // Trusted badge should appear on Bob's member entry.
      await expect(page1.locator('.member-item', { hasText: 'Bob' }).locator('.trusted-badge'))
        .toBeVisible({ timeout: 10_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('trusted peer messages are visible', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Trusted Msg', 'Alice', 'Bob');
    try {
      // Alice trusts Bob.
      await trustPeer(page1, 'Bob');
      await page1.waitForTimeout(1000);

      // Bob sends a message.
      await sendMessage(page2, 'trusted message');

      // Alice should see it.
      await waitForMessage(page1, 'trusted message', 15_000);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('owner untrusts peer — trusted badge hidden', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Untrust Server', 'Alice', 'Bob');
    try {
      // Alice trusts then untrusts Bob.
      await trustPeer(page1, 'Bob');
      await page1.waitForTimeout(1000);
      await untrustPeer(page1, 'Bob');

      // Trusted badge should be hidden.
      await expect(page1.locator('.member-item', { hasText: 'Bob' }).locator('.trusted-badge'))
        .toBeHidden({ timeout: 10_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('untrusted messages rejected after untrust', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Reject Msg', 'Alice', 'Bob');
    try {
      // Trust Bob first and verify messaging works (establishes delivery baseline).
      await trustPeer(page1, 'Bob');
      await page1.waitForTimeout(1000);

      await sendMessage(page2, 'before untrust');
      await waitForMessage(page1, 'before untrust', 15_000);

      // Now untrust Bob.
      await untrustPeer(page1, 'Bob');
      await page1.waitForTimeout(1000);

      // Bob sends a message that should NOT arrive on Alice's side.
      await sendMessage(page2, 'after untrust secret');

      // Sentinel: Alice sends a message from her own side. Since Alice is the
      // owner, her own message always appears locally immediately. We wait for
      // it to confirm enough real time has elapsed for P2P delivery to have
      // occurred if it were going to — without this we'd just be racing a
      // fixed timeout against an unknown network delay.
      await sendMessage(page1, 'alice sentinel');
      await waitForMessage(page1, 'alice sentinel', 10_000);

      // Now we have proof that local rendering is working and time has passed.
      // Assert that the rejected message never arrived.
      await expect(page1.locator('.message .body', { hasText: 'after untrust secret' }))
        .not.toBeVisible({ timeout: 5000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('owner kicks member — member count drops', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Kick Server', 'Alice', 'Bob');
    try {
      // Record initial member count (includes relay + peers).
      await page1.waitForTimeout(1000);
      const initialCount = await page1.locator('.member-item').count();
      expect(initialCount).toBeGreaterThanOrEqual(2);

      // Alice kicks Bob.
      await kickPeer(page1, 'Bob');

      // Member count should drop by 1.
      await expect(page1.locator('.member-item'))
        .toHaveCount(initialCount - 1, { timeout: 15_000 });
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
      await expect(page1.locator('.sidebar')).toBeVisible({ timeout: 5000 });
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
      const actionButtons = page2.locator('.member-actions button');
      await expect(actionButtons).toHaveCount(0, { timeout: 5000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });
});
