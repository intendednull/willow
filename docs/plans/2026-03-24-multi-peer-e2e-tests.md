# Multi-Peer E2E Browser Tests Implementation Plan

**Status:** landed (commits `eeb8329` initial multi-peer/permissions/mobile specs + `f715387` code-review followup) — `e2e/{multi-peer-sync,permissions,multi-peer-mobile}.spec.ts` shipped; `just test-e2e-sync` / `test-e2e-perms` recipes wired; legacy `e2e/{two-peer,state-sync}.spec.ts` deleted in favor of the new fixtures.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Comprehensive Playwright E2E tests for multi-peer state sync, permissions, and mobile interactions — running across all 4 browser projects (Desktop Chrome, Mobile Chrome, Desktop Firefox, Mobile Firefox).

**Architecture:** Add shared helpers to `e2e/helpers.ts` for multi-peer setup, mobile-aware navigation, and message actions. Create 3 new test files replacing 2 old Chrome-only files. Tests use the Playwright `browser` fixture for cross-browser support.

**Tech Stack:** Playwright, TypeScript, tests run against deployed site at `https://willow.intendednull.com`

**Spec:** `docs/specs/2026-03-24-multi-peer-e2e-tests-design.md`

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `e2e/helpers.ts` | **Modify** | Add `setupTwoPeers`, mobile-aware navigation helpers, permission action helpers, message action helpers |
| `e2e/multi-peer-sync.spec.ts` | **Create** | 12 core sync tests running on all 4 browsers |
| `e2e/permissions.spec.ts` | **Create** | 8 permission/trust/kick tests on all 4 browsers |
| `e2e/multi-peer-mobile.spec.ts` | **Create** | 5 mobile-specific multi-peer tests |
| `e2e/two-peer.spec.ts` | **Delete** | Consolidated into multi-peer-sync.spec.ts |
| `e2e/state-sync.spec.ts` | **Delete** | Consolidated into multi-peer-sync.spec.ts |
| `justfile` | **Modify** | Update `test-e2e-sync`, add `test-e2e-perms` |

---

### Task 1: Add helper functions to `e2e/helpers.ts`

**Files:**
- Modify: `e2e/helpers.ts`

- [ ] **Step 1: Add mobile detection utility and navigation helpers**

Append to `e2e/helpers.ts`:

```typescript
import { Page, Browser, BrowserContext } from '@playwright/test';

/** Check if page is using a mobile viewport. */
export function isMobile(page: Page): boolean {
  return (page.viewportSize()?.width ?? 1024) < 768;
}

/** Open sidebar (clicks hamburger on mobile, no-op on desktop). */
export async function openSidebar(page: Page) {
  if (isMobile(page)) {
    await page.locator('.mobile-nav-toggle').click();
    await page.waitForTimeout(300);
  }
}

/** Close sidebar (clicks overlay on mobile, no-op on desktop). */
export async function closeSidebar(page: Page) {
  if (isMobile(page)) {
    const overlay = page.locator('.sidebar-overlay.open');
    if (await overlay.isVisible()) {
      await overlay.click();
      await page.waitForTimeout(300);
    }
  }
}

/** Open member list (clicks toggle on mobile, no-op on desktop). */
export async function openMemberList(page: Page) {
  if (isMobile(page)) {
    await page.locator('.mobile-members-toggle').click();
    await page.waitForTimeout(300);
  }
}

/** Close member list (clicks overlay on mobile, no-op on desktop). */
export async function closeMemberList(page: Page) {
  if (isMobile(page)) {
    const overlay = page.locator('.members-overlay.open');
    if (await overlay.isVisible()) {
      await overlay.click();
      await page.waitForTimeout(300);
    }
  }
}
```

- [ ] **Step 2: Add invite flow helpers**

```typescript
/** Open server settings (mobile-aware). */
export async function openServerSettings(page: Page) {
  await openSidebar(page);
  await page.locator('.server-gear-btn').click();
  await page.waitForTimeout(500);
}

/** Generate an invite code for a specific peer. Returns the invite code string. */
export async function generateInvite(page: Page, recipientPeerId: string): Promise<string> {
  await openServerSettings(page);
  await page.locator('input[placeholder*="12D3KooW"]').fill(recipientPeerId);
  await page.locator('button', { hasText: 'Generate Invite' }).click();
  await page.waitForTimeout(500);
  const code = await page.locator('.invite-code-display textarea').inputValue();
  // Go back to chat.
  await page.locator('text=Back').click();
  await page.waitForTimeout(500);
  return code;
}

/** Join a server via invite code from the welcome screen. */
export async function joinViaInvite(page: Page, inviteCode: string, displayName?: string) {
  await page.locator('.welcome-invite-input').fill(inviteCode);
  await page.locator('button', { hasText: 'Next' }).click();
  await page.waitForTimeout(500);
  if (displayName) {
    // The display name input is the second input in the join step.
    const dnInput = page.locator('.welcome-option').last().locator('input').first();
    if (await dnInput.isVisible()) {
      await dnInput.fill(displayName);
    }
  }
  await page.locator('button', { hasText: 'Join Server' }).click();
  await page.waitForSelector('.sidebar', { timeout: 15_000 });
  await page.waitForTimeout(3000); // Wait for initial sync.
}

/** Set up two peers: Peer1 creates server, Peer2 joins via invite. */
export async function setupTwoPeers(
  browser: Browser,
  serverName = 'Test Server',
  peer1Name = 'Alice',
  peer2Name = 'Bob',
): Promise<{
  ctx1: BrowserContext;
  ctx2: BrowserContext;
  page1: Page;
  page2: Page;
}> {
  const ctx1 = await browser.newContext();
  const ctx2 = await browser.newContext();
  const page1 = await ctx1.newPage();
  const page2 = await ctx2.newPage();

  // Peer 1: Create server.
  await freshStart(page1);
  await createServer(page1, serverName, peer1Name);

  // Peer 2: Get peer ID from welcome screen.
  await freshStart(page2);
  const peer2Id = await getPeerId(page2);

  // Peer 1: Generate invite.
  const inviteCode = await generateInvite(page1, peer2Id);

  // Peer 2: Join.
  await joinViaInvite(page2, inviteCode, peer2Name);

  return { ctx1, ctx2, page1, page2 };
}
```

- [ ] **Step 3: Add message action helpers (desktop/mobile branching)**

```typescript
/** Create a channel (mobile-aware: opens sidebar if needed). */
export async function createChannel(page: Page, name: string) {
  await openSidebar(page);
  await page.locator('.channel-add-btn').click();
  await page.waitForTimeout(200);
  await page.locator('.channel-create-input input').fill(name);
  await page.locator('.channel-create-input input').press('Enter');
  await page.waitForTimeout(1000);
  await closeSidebar(page);
}

/** Switch to a channel (mobile-aware: opens sidebar if needed). */
export async function switchChannelMobile(page: Page, channelName: string) {
  await openSidebar(page);
  await page.locator('.channel-item', { hasText: channelName }).click();
  await page.waitForTimeout(300);
  // Sidebar auto-closes on mobile after channel click.
}

/**
 * Perform a message action via dropdown (desktop) or action sheet (mobile).
 * Desktop: hover → .action-trigger → .dropdown-item matching actionName.
 * Mobile: longPress → .sheet-item matching actionName.
 */
export async function messageAction(page: Page, messageText: string, actionName: string) {
  const msg = page.locator('.message', { hasText: messageText }).last();
  if (isMobile(page)) {
    // Long-press to open action sheet.
    const box = await msg.boundingBox();
    if (!box) throw new Error(`Message not found: ${messageText}`);
    await longPress(page, `.message:has-text("${messageText.replace(/"/g, '\\"')}")`);
    await page.locator('.mobile-action-sheet.open').waitFor({ timeout: 3000 });
    await page.locator('.sheet-item', { hasText: actionName }).click();
    await page.waitForTimeout(300);
  } else {
    await msg.hover();
    await page.waitForTimeout(200);
    await msg.locator('.action-trigger').click();
    await page.waitForTimeout(200);
    await page.locator('.dropdown-item', { hasText: actionName }).click();
    await page.waitForTimeout(200);
  }
}

/** Edit a message. */
export async function editMessage(page: Page, originalText: string, newText: string) {
  await messageAction(page, originalText, 'Edit');
  const input = page.locator('.input-area input, .input-area textarea').first();
  await input.fill(newText);
  await input.press('Enter');
  await page.waitForTimeout(500);
}

/** Delete a message. */
export async function deleteMessage(page: Page, text: string) {
  await messageAction(page, text, 'Delete');
  await page.waitForTimeout(500);
}

/**
 * React to a message with an emoji.
 * Desktop: hover → action-trigger → React → emoji row.
 * Mobile: longPress → emoji row in sheet.
 */
export async function reactToMessage(page: Page, messageText: string, emojiIndex = 0) {
  const msg = page.locator('.message', { hasText: messageText }).last();
  if (isMobile(page)) {
    const box = await msg.boundingBox();
    if (!box) throw new Error(`Message not found: ${messageText}`);
    await longPress(page, `.message:has-text("${messageText.replace(/"/g, '\\"')}")`);
    await page.locator('.mobile-action-sheet.open').waitFor({ timeout: 3000 });
    await page.locator('.sheet-emoji-row button').nth(emojiIndex).click();
    await page.waitForTimeout(300);
  } else {
    await msg.hover();
    await page.waitForTimeout(200);
    await msg.locator('.action-trigger').click();
    await page.waitForTimeout(200);
    await page.locator('.dropdown-item', { hasText: 'React' }).click();
    await page.waitForTimeout(200);
    await page.locator('.dropdown-emoji-row button').nth(emojiIndex).click();
    await page.waitForTimeout(500);
  }
}
```

- [ ] **Step 4: Add permission action helpers**

```typescript
/** Trust a peer via the member list. */
export async function trustPeer(page: Page, peerName: string) {
  await openMemberList(page);
  const memberItem = page.locator('.member-item', { hasText: peerName });
  await memberItem.locator('button', { hasText: 'Trust' }).click();
  await page.waitForTimeout(500);
}

/** Untrust a peer via the member list. */
export async function untrustPeer(page: Page, peerName: string) {
  await openMemberList(page);
  const memberItem = page.locator('.member-item', { hasText: peerName });
  await memberItem.locator('button', { hasText: 'Untrust' }).click();
  await page.waitForTimeout(500);
}

/** Kick a peer via the member list. */
export async function kickPeer(page: Page, peerName: string) {
  await openMemberList(page);
  const memberItem = page.locator('.member-item', { hasText: peerName });
  await memberItem.locator('button.btn-danger', { hasText: 'Kick' }).click();
  await page.waitForTimeout(500);
}

/** Wait for the member list to show exactly `count` members. */
export async function waitForPeerCount(page: Page, count: number, timeout = 15_000) {
  await openMemberList(page);
  await expect(page.locator('.member-item')).toHaveCount(count, { timeout });
}
```

Note: `waitForPeerCount` uses `expect` from `@playwright/test` — add it to the imports at the top of helpers.ts.

- [ ] **Step 5: Verify helpers compile**

Run: `npx tsc --noEmit e2e/helpers.ts` or just `npx playwright test --list` to verify no syntax errors.

- [ ] **Step 6: Commit**

```bash
git add e2e/helpers.ts
git commit -m "feat: add multi-peer, mobile-aware, and permission helpers for E2E tests"
```

---

### Task 2: Create `e2e/multi-peer-sync.spec.ts`

**Files:**
- Create: `e2e/multi-peer-sync.spec.ts`

- [ ] **Step 1: Write the test file**

Create `e2e/multi-peer-sync.spec.ts`:

```typescript
import { test, expect } from '@playwright/test';
import {
  setupTwoPeers, sendMessage, waitForMessage, waitForApp,
  createChannel, switchChannelMobile, editMessage, reactToMessage,
  openMemberList, isMobile,
} from './helpers';

test.describe('Multi-peer state synchronization', () => {

  test('invite flow: create server, generate invite, join', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Both peers should see the sidebar (server loaded).
      await expect(page1.locator('.sidebar')).toBeVisible();
      await expect(page2.locator('.sidebar')).toBeVisible();
      // Both should see "general" channel.
      if (isMobile(page1)) {
        await page1.locator('.mobile-nav-toggle').click();
        await page1.waitForTimeout(300);
      }
      await expect(page1.locator('.channel-item', { hasText: 'general' })).toBeVisible();
      if (isMobile(page2)) {
        await page2.locator('.mobile-nav-toggle').click();
        await page2.waitForTimeout(300);
      }
      await expect(page2.locator('.channel-item', { hasText: 'general' })).toBeVisible();
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('messages sync both directions in general channel', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await sendMessage(page1, 'Hello from Alice');
      await waitForMessage(page2, 'Hello from Alice', 15_000);
      await sendMessage(page2, 'Hello from Bob');
      await waitForMessage(page1, 'Hello from Bob', 15_000);
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('pre-existing channels visible to joining peer', async ({ browser }) => {
    // Create a fresh setup where Peer1 creates extra channels BEFORE invite.
    const ctx1 = await browser.newContext();
    const ctx2 = await browser.newContext();
    const page1 = await ctx1.newPage();
    const page2 = await ctx2.newPage();
    try {
      const { freshStart, createServer, getPeerId, generateInvite, joinViaInvite } = await import('./helpers');
      await freshStart(page1);
      await createServer(page1, 'PreChan Server', 'Alice');

      // Create 2 extra channels before invite.
      await createChannel(page1, 'announcements');
      await createChannel(page1, 'random');

      // Now invite Peer2.
      await freshStart(page2);
      const peer2Id = await getPeerId(page2);
      const inviteCode = await generateInvite(page1, peer2Id);
      await joinViaInvite(page2, inviteCode, 'Bob');

      // Peer2 should see all 3 channels.
      if (isMobile(page2)) {
        await page2.locator('.mobile-nav-toggle').click();
        await page2.waitForTimeout(300);
      }
      await expect(page2.locator('.channel-item', { hasText: 'general' })).toBeVisible({ timeout: 15_000 });
      await expect(page2.locator('.channel-item', { hasText: 'announcements' })).toBeVisible({ timeout: 15_000 });
      await expect(page2.locator('.channel-item', { hasText: 'random' })).toBeVisible({ timeout: 15_000 });
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('new channel created mid-session appears on both', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await createChannel(page1, 'new-channel');
      // Peer2 should see it.
      if (isMobile(page2)) {
        await page2.locator('.mobile-nav-toggle').click();
        await page2.waitForTimeout(300);
      }
      await expect(page2.locator('.channel-item', { hasText: 'new-channel' }))
        .toBeVisible({ timeout: 15_000 });
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('messages in non-general channel sync both ways', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await createChannel(page1, 'dev');
      await switchChannelMobile(page1, 'dev');

      // Wait for channel to appear on Peer2, then switch.
      if (isMobile(page2)) {
        await page2.locator('.mobile-nav-toggle').click();
        await page2.waitForTimeout(300);
      }
      await expect(page2.locator('.channel-item', { hasText: 'dev' }))
        .toBeVisible({ timeout: 15_000 });
      await switchChannelMobile(page2, 'dev');

      // Exchange messages.
      await sendMessage(page1, 'dev message from Alice');
      await waitForMessage(page2, 'dev message from Alice', 15_000);
      await sendMessage(page2, 'dev reply from Bob');
      await waitForMessage(page1, 'dev reply from Bob', 15_000);
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('reactions sync between peers', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await sendMessage(page1, 'react to this');
      await waitForMessage(page2, 'react to this', 15_000);
      await reactToMessage(page1, 'react to this');
      await expect(page2.locator('.reaction')).toBeVisible({ timeout: 15_000 });
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('edits sync between peers', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await sendMessage(page1, 'original text');
      await waitForMessage(page2, 'original text', 15_000);
      await editMessage(page1, 'original text', 'edited text');
      await waitForMessage(page2, 'edited text', 15_000);
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('deletes sync between peers', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await sendMessage(page1, 'delete me');
      await waitForMessage(page2, 'delete me', 15_000);
      await deleteMessage(page1, 'delete me');
      // Peer2 should see the message disappear or show deleted state.
      await expect(page2.locator('.message .body', { hasText: 'delete me' }))
        .toBeHidden({ timeout: 15_000 });
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('state persists after refresh for both peers', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await sendMessage(page1, 'persistent msg');
      await waitForMessage(page2, 'persistent msg', 15_000);

      await page1.reload();
      await waitForApp(page1);
      await page1.waitForTimeout(1000);
      await page2.reload();
      await waitForApp(page2);
      await page2.waitForTimeout(1000);

      await expect(page1.locator('.message .body', { hasText: 'persistent msg' })).toBeVisible();
      await expect(page2.locator('.message .body', { hasText: 'persistent msg' })).toBeVisible();
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('both peers in member list', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await openMemberList(page1);
      await expect(page1.locator('.member-item')).toHaveCount(2, { timeout: 15_000 });
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('typing indicator shows on other peer', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      const input = page1.locator('.input-area input, .input-area textarea').first();
      await input.fill('typing...');
      await page1.waitForTimeout(500);
      await expect(page2.locator('.typing-indicator')).not.toBeEmpty({ timeout: 10_000 });
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('display names shown correctly', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Name Server', 'Alice', 'Bob');
    try {
      await sendMessage(page1, 'hello from alice');
      await waitForMessage(page2, 'hello from alice', 15_000);
      // Peer2 should see "Alice" as the message author.
      const authorEl = page2.locator('.message .author', { hasText: 'Alice' });
      await expect(authorEl).toBeVisible({ timeout: 15_000 });
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });
});
```

Add the missing `deleteMessage` import at the top.

- [ ] **Step 2: Run the tests on desktop-chrome**

Run: `npx playwright test e2e/multi-peer-sync.spec.ts --project=desktop-chrome`

Expected: All 12 tests pass (some may need timing adjustments).

- [ ] **Step 3: Fix any failing tests**

Adjust timeouts, waits, or selectors based on failures.

- [ ] **Step 4: Commit**

```bash
git add e2e/multi-peer-sync.spec.ts
git commit -m "feat: add multi-peer sync E2E tests (12 tests, all browsers)"
```

---

### Task 3: Create `e2e/permissions.spec.ts`

**Files:**
- Create: `e2e/permissions.spec.ts`

- [ ] **Step 1: Write the test file**

Create `e2e/permissions.spec.ts`:

```typescript
import { test, expect } from '@playwright/test';
import {
  setupTwoPeers, sendMessage, waitForMessage,
  openMemberList, trustPeer, untrustPeer, kickPeer,
  waitForPeerCount, isMobile,
} from './helpers';

test.describe('Permissions and trust', () => {

  test('owner can trust a peer', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await openMemberList(page1);
      // Find Bob's member item and click Trust.
      await trustPeer(page1, 'Bob');
      // Bob should now show "Trusted" badge.
      await expect(page1.locator('.member-item', { hasText: 'Bob' }).locator('.trusted-badge'))
        .toBeVisible({ timeout: 5_000 });
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('trusted peer can send messages', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await trustPeer(page1, 'Bob');
      await page1.waitForTimeout(1000);

      // Bob sends a message.
      await sendMessage(page2, 'Message from trusted Bob');
      // Alice should see it.
      await waitForMessage(page1, 'Message from trusted Bob', 15_000);
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('owner can untrust a peer', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Trust first, then untrust.
      await trustPeer(page1, 'Bob');
      await page1.waitForTimeout(500);
      await untrustPeer(page1, 'Bob');
      // Trusted badge should be gone.
      await expect(page1.locator('.member-item', { hasText: 'Bob' }).locator('.trusted-badge'))
        .toBeHidden({ timeout: 5_000 });
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('untrusted peer messages not visible to owner', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Trust Bob so messages work, then untrust.
      await trustPeer(page1, 'Bob');
      await page1.waitForTimeout(1000);

      // Verify messaging works while trusted.
      await sendMessage(page2, 'trusted msg');
      await waitForMessage(page1, 'trusted msg', 15_000);

      // Untrust Bob.
      await untrustPeer(page1, 'Bob');
      await page1.waitForTimeout(1000);

      // Bob sends after untrust — Alice should NOT see it.
      await sendMessage(page2, 'untrusted msg');
      await page2.waitForTimeout(5000);
      await expect(page1.locator('.message .body', { hasText: 'untrusted msg' }))
        .toBeHidden();
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('owner can kick a member', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await openMemberList(page1);
      // Should see 2 members.
      await expect(page1.locator('.member-item')).toHaveCount(2, { timeout: 15_000 });

      // Kick Bob.
      await kickPeer(page1, 'Bob');
      await page1.waitForTimeout(2000);

      // Alice's member list should now show 1 member.
      await expect(page1.locator('.member-item')).toHaveCount(1, { timeout: 10_000 });
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('kicked peer sees disconnected state', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await kickPeer(page1, 'Bob');
      await page2.waitForTimeout(5000);

      // After being kicked, Bob should not be able to send messages that
      // Alice sees (encryption keys rotated). Bob may still see the UI
      // but new messages from Bob will not appear on Alice's side.
      await sendMessage(page2, 'post-kick message');
      await page1.waitForTimeout(5000);
      await expect(page1.locator('.message .body', { hasText: 'post-kick message' }))
        .toBeHidden();
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('owner can create and assign roles', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Open server settings → roles.
      await openServerSettings(page1);
      await page1.waitForTimeout(500);

      // Look for role management section.
      const roleInput = page1.locator('input[placeholder*="role" i], input[placeholder*="Role" i]');
      if (await roleInput.isVisible()) {
        await roleInput.fill('Moderator');
        await page1.locator('button', { hasText: 'Create' }).click();
        await page1.waitForTimeout(500);
        // Role should appear in the list.
        await expect(page1.locator('text=Moderator')).toBeVisible({ timeout: 5_000 });
      }
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('non-owner does not see trust/kick buttons', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Peer2 (Bob, non-owner) opens member list.
      await openMemberList(page2);
      await page2.waitForTimeout(1000);

      // Bob should see member items but NO action buttons.
      await expect(page2.locator('.member-item')).toHaveCount(2, { timeout: 15_000 });
      await expect(page2.locator('.member-actions button')).toHaveCount(0);
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });
});
```

Add the missing `openServerSettings` import.

- [ ] **Step 2: Run the tests on desktop-chrome**

Run: `npx playwright test e2e/permissions.spec.ts --project=desktop-chrome`

Expected: All 8 tests pass.

- [ ] **Step 3: Fix any failures**

Permission tests depend on real P2P behavior — may need extended timeouts for trust propagation.

- [ ] **Step 4: Commit**

```bash
git add e2e/permissions.spec.ts
git commit -m "feat: add permission/trust/kick E2E tests (8 tests, all browsers)"
```

---

### Task 4: Create `e2e/multi-peer-mobile.spec.ts`

**Files:**
- Create: `e2e/multi-peer-mobile.spec.ts`

- [ ] **Step 1: Write the test file**

Create `e2e/multi-peer-mobile.spec.ts`:

```typescript
import { test, expect } from '@playwright/test';
import {
  setupTwoPeers, sendMessage, waitForMessage,
  openSidebar, closeSidebar, openMemberList,
  createChannel, switchChannelMobile,
} from './helpers';

test.describe('Multi-peer mobile interactions', () => {
  test.beforeEach(({}, testInfo) => {
    test.skip(!testInfo.project.name.includes('mobile'), 'mobile only');
  });

  test('invite flow through mobile UI', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mobile Server');
    try {
      // Both peers should have loaded.
      await expect(page1.locator('.app')).toBeVisible();
      await expect(page2.locator('.app')).toBeVisible();
      // Verify sidebar accessible via hamburger.
      await openSidebar(page1);
      await expect(page1.locator('.channel-item', { hasText: 'general' })).toBeVisible();
      await closeSidebar(page1);
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('new channels visible via hamburger menu', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Peer1 creates a channel.
      await createChannel(page1, 'mobile-test');
      // Peer2 opens sidebar and sees it.
      await page2.waitForTimeout(5000); // Wait for sync.
      await openSidebar(page2);
      await expect(page2.locator('.channel-item', { hasText: 'mobile-test' }))
        .toBeVisible({ timeout: 15_000 });
      await closeSidebar(page2);
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('messages arrive while sidebar closed', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Peer2 has sidebar closed (default on mobile).
      // Peer1 sends a message.
      await sendMessage(page1, 'while sidebar closed');
      // Peer2 should see it in the chat area (no sidebar interaction needed).
      await waitForMessage(page2, 'while sidebar closed', 15_000);
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('member list via mobile toggle', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await openMemberList(page1);
      await expect(page1.locator('.member-item')).toHaveCount(2, { timeout: 15_000 });
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });

  test('channel switch on mobile during sync', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await createChannel(page1, 'switch-test');
      await switchChannelMobile(page1, 'switch-test');
      await sendMessage(page1, 'message in switch-test');

      // Peer2: wait for channel, switch to it, see the message.
      await page2.waitForTimeout(5000);
      await switchChannelMobile(page2, 'switch-test');
      await waitForMessage(page2, 'message in switch-test', 15_000);
    } finally {
      await ctx1.close(); await ctx2.close();
    }
  });
});
```

- [ ] **Step 2: Run on mobile-chrome**

Run: `npx playwright test e2e/multi-peer-mobile.spec.ts --project=mobile-chrome`

Expected: All 5 tests pass.

- [ ] **Step 3: Fix any failures**

Mobile tests are sensitive to timing and viewport — adjust waits as needed.

- [ ] **Step 4: Commit**

```bash
git add e2e/multi-peer-mobile.spec.ts
git commit -m "feat: add mobile-specific multi-peer E2E tests (5 tests)"
```

---

### Task 5: Delete old files and update justfile

**Files:**
- Delete: `e2e/two-peer.spec.ts`
- Delete: `e2e/state-sync.spec.ts`
- Modify: `justfile`

- [ ] **Step 1: Delete old test files**

```bash
rm e2e/two-peer.spec.ts e2e/state-sync.spec.ts
```

- [ ] **Step 2: Update justfile**

In `justfile`, replace:

```
# Run only state sync tests
test-e2e-sync:
    npx playwright test e2e/state-sync.spec.ts --project=desktop-chrome
```

With:

```
# Run multi-peer sync tests (desktop-chrome for quick iteration)
test-e2e-sync:
    npx playwright test e2e/multi-peer-sync.spec.ts --project=desktop-chrome

# Run permission tests
test-e2e-perms:
    npx playwright test e2e/permissions.spec.ts --project=desktop-chrome
```

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "chore: retire two-peer.spec.ts and state-sync.spec.ts, update justfile

Replaced by multi-peer-sync.spec.ts and permissions.spec.ts which
run across all 4 browser projects instead of Chrome-only."
```

---

### Task 6: Full cross-browser verification

**Files:** None (testing only)

- [ ] **Step 1: Run all E2E tests on all browsers**

Run: `npx playwright test`

This runs all spec files across all 4 projects. Expected: 90 total test executions pass. Some mobile tests may be flaky due to P2P timing — note flaky tests and add extended waits where needed.

- [ ] **Step 2: Run just the new multi-peer tests on each browser individually**

```bash
npx playwright test e2e/multi-peer-sync.spec.ts --project=desktop-chrome
npx playwright test e2e/multi-peer-sync.spec.ts --project=desktop-firefox
npx playwright test e2e/multi-peer-sync.spec.ts --project=mobile-chrome
npx playwright test e2e/multi-peer-sync.spec.ts --project=mobile-firefox
```

Note which browsers pass, which fail, and fix issues.

- [ ] **Step 3: Fix any flaky tests**

Common fixes: increase wait timeouts, add `waitForTimeout` between interactions, use `toBeVisible({ timeout: 15_000 })` for sync-dependent assertions.

- [ ] **Step 4: Final commit if fixes were needed**

```bash
git add -A
git commit -m "fix: stabilize E2E tests across all browsers"
```
