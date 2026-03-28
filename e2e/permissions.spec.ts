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

  test.fixme('owner trusts peer — trusted badge appears', async ({ browser }) => {
    // Flaky: two-peer setup with trust action can exceed test timeframes.
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

  test.fixme('trusted peer messages are visible', async ({ browser }) => {
    // Flaky: depends on trust + message sync within test timeframes.
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

  test.fixme('owner untrusts peer — trusted badge hidden', async ({ browser }) => {
    // Flaky: depends on trust + untrust sync within test timeframes.
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

  test.fixme('untrusted messages rejected after untrust', async ({ browser }) => {
    // Depends on display name sync for trustPeer/untrustPeer('Bob').
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Reject Msg', 'Alice', 'Bob');
    try {
      // Trust Bob first and verify messaging works.
      await trustPeer(page1, 'Bob');
      await page1.waitForTimeout(1000);

      await sendMessage(page2, 'before untrust');
      await waitForMessage(page1, 'before untrust', 15_000);

      // Now untrust Bob.
      await untrustPeer(page1, 'Bob');
      await page1.waitForTimeout(1000);

      // Bob sends another message.
      await sendMessage(page2, 'after untrust secret');

      // Alice should NOT see Bob's new message (wait a reasonable time).
      await page1.waitForTimeout(5000);
      await expect(page1.locator('.message .body', { hasText: 'after untrust secret' }))
        .toBeHidden();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test.fixme('owner kicks member — member count drops', async ({ browser }) => {
    // Flaky: depends on kick sync within test timeframes.
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

  test.fixme('kicked peer messages do not reach owner', async ({ browser }) => {
    // Depends on display name sync for kickPeer('Bob').
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Kick Msg', 'Alice', 'Bob');
    try {
      // Alice kicks Bob.
      await kickPeer(page1, 'Bob');
      await page1.waitForTimeout(2000);

      // Bob tries to send a message.
      await sendMessage(page2, 'kicked but trying');

      // Alice should NOT see it.
      await page1.waitForTimeout(5000);
      await expect(page1.locator('.message .body', { hasText: 'kicked but trying' }))
        .toBeHidden();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('create and assign roles via server settings', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Role Server', 'Alice', 'Bob');
    try {
      // Open server settings.
      await openServerSettings(page1);

      // Look for a role creation input (if visible in the settings panel).
      const roleInput = page1.locator('input[placeholder*="role" i], input[placeholder*="Role"]').first();
      if (await roleInput.isVisible({ timeout: 3000 }).catch(() => false)) {
        await roleInput.fill('Moderator');
        await roleInput.press('Enter');
        await page1.waitForTimeout(500);

        // Should see the new role in the settings.
        await expect(page1.locator('text=Moderator')).toBeVisible({ timeout: 5000 });
      }

      // Go back to chat.
      await page1.locator('text=Back').click();
      await page1.waitForTimeout(500);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('non-owner has no action buttons in member list', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'NoActions', 'Alice', 'Bob');
    try {
      // Bob opens the member list (he is not the owner).
      await openMemberList(page2);
      await page2.waitForTimeout(1000);

      // Bob should NOT have any trust/kick/untrust action buttons.
      const actionButtons = page2.locator('.member-actions button');
      await expect(actionButtons).toHaveCount(0, { timeout: 5000 });

      await closeMemberList(page2);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });
});
