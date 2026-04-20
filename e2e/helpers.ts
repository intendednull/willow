import { Page, Browser, BrowserContext, expect } from '@playwright/test';

/** Wait for the WASM app to load (loading spinner disappears). */
export async function waitForApp(page: Page) {
  // Wait for the app to render (welcome screen, chat, or join page).
  await page.waitForSelector('.welcome-screen, .app, .sidebar, .join-card', {
    timeout: 30_000,
  });
  // Give WASM a moment to stabilize.
  await page.waitForTimeout(1000);
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

  // Wait for the app to load with the new server.
  await page.waitForSelector('.channel-sidebar, .sidebar', { timeout: 10_000 });
  await page.waitForTimeout(500);
}

/** Get the full peer ID from the welcome screen or settings. */
export async function getPeerId(page: Page): Promise<string> {
  // Welcome screen: advance past step 1 (no name), then switch to the
  // Join tab — the peer id lives inside the Join step list, hidden by
  // default and revealed by the eye-toggle icon.
  if (await page.locator('.welcome-card').isVisible().catch(() => false)) {
    await advancePastNameStep(page);
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

/** Send a message in the current channel. */
export async function sendMessage(page: Page, text: string) {
  const input = page.locator('.input-area input, .input-area textarea').first();
  await input.fill(text);
  await input.press('Enter');
  await page.waitForTimeout(300);
}

/** Get all visible message bodies. */
export async function getMessages(page: Page): Promise<string[]> {
  const bodies = page.locator('.message .body');
  const count = await bodies.count();
  const texts: string[] = [];
  for (let i = 0; i < count; i++) {
    texts.push((await bodies.nth(i).textContent()) || '');
  }
  return texts;
}

/** Click a channel by name. On mobile this routes through the home
 *  tab — pop any pushed screen first so the channel list is visible,
 *  then tap the row (which pushes the chat view). */
export async function switchChannel(page: Page, channelName: string) {
  if (isMobile(page)) {
    // Pop back to home if we are currently on a pushed screen.
    const backSlot = page.locator('.mobile-top-bar .top-slot-left .top-back');
    while (await backSlot.isVisible().catch(() => false)) {
      await page.locator('.mobile-top-bar .top-slot-left').click();
      await page.waitForTimeout(300);
    }
    // Make sure we are on the home tab.
    await page.locator('.mobile-tab-bar .tab[data-tab="home"]').click();
    await page.waitForTimeout(200);
    await page
      .locator('.mobile-home .channel-item', { hasText: channelName })
      .click();
    await page.waitForTimeout(400);
    return;
  }
  await page.locator('.channel-item', { hasText: channelName }).click();
  await page.waitForTimeout(300);
}

/** Wait for a specific message to appear. */
export async function waitForMessage(page: Page, text: string, timeout = 20_000) {
  await page.locator('.message .body', { hasText: text }).waitFor({ timeout });
}

/** Simulate a long-press on an element to open the mobile action sheet. */
export async function longPress(page: Page, selector: string, durationMs = 600) {
  const el = page.locator(selector).first();
  const box = await el.boundingBox();
  if (!box) throw new Error(`Element not found: ${selector}`);

  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;

  // Dispatch real TouchEvent via page.evaluate.
  await page.evaluate(({ x, y }) => {
    const target = document.elementFromPoint(x, y);
    if (!target) return;
    const touch = new Touch({
      identifier: 1,
      target,
      clientX: x,
      clientY: y,
      pageX: x,
      pageY: y,
    });
    target.dispatchEvent(new TouchEvent('touchstart', {
      bubbles: true,
      cancelable: true,
      touches: [touch],
      targetTouches: [touch],
      changedTouches: [touch],
    }));
  }, { x, y });

  await page.waitForTimeout(durationMs);

  // Dispatch touchend.
  await page.evaluate(({ x, y }) => {
    const target = document.elementFromPoint(x, y);
    if (!target) return;
    const touch = new Touch({
      identifier: 1,
      target,
      clientX: x,
      clientY: y,
      pageX: x,
      pageY: y,
    });
    target.dispatchEvent(new TouchEvent('touchend', {
      bubbles: true,
      cancelable: true,
      touches: [],
      targetTouches: [],
      changedTouches: [touch],
    }));
  }, { x, y });

  await page.waitForTimeout(300);
}

// ── Mobile detection + navigation ─────────────────────────────────────

/** Returns true if the page viewport is narrow enough to be mobile. */
export function isMobile(page: Page): boolean {
  return (page.viewportSize()?.width ?? 1024) < 768;
}

/** Opens the grove drawer on mobile (no-op on desktop). Idempotent —
 *  won't close it if it's already open. The mobile shell top bar has
 *  a grove-glyph tile on the left that opens the drawer on the home
 *  route; on pushed screens the left slot is a back chevron.
 */
export async function openSidebar(page: Page) {
  if (!isMobile(page)) return;
  const alreadyOpen = await page.locator('.grove-drawer.open').isVisible().catch(() => false);
  if (alreadyOpen) return;
  await page.locator('.mobile-top-bar .top-slot-left').click();
  await page.waitForTimeout(500);
}

/** Closes the grove drawer on mobile by tapping the backdrop. No-op
 *  on desktop or when the drawer is already closed. */
export async function closeSidebar(page: Page) {
  if (!isMobile(page)) return;
  const drawerOpen = await page.locator('.grove-drawer.open').isVisible().catch(() => false);
  if (!drawerOpen) return;
  // Backdrop covers the full viewport; dispatch bypasses Playwright's
  // hit-test which rightly warns about overlapping layers.
  await page.locator('.grove-drawer-backdrop').dispatchEvent('click');
  await page.waitForTimeout(300);
}

/** Switch to a given mobile primary tab (home / letters / discover / you).
 *  No-op on desktop. */
export async function switchTab(
  page: Page,
  tabId: 'home' | 'letters' | 'discover' | 'you',
) {
  if (!isMobile(page)) return;
  await page.locator(`.mobile-tab-bar .tab[data-tab="${tabId}"]`).click();
  await page.waitForTimeout(200);
}

/** Opens the member list in the right rail. On desktop clicks the
 *  main-pane-header members action button; on mobile this routes
 *  into the chat push where the header lives. */
export async function openMemberList(page: Page) {
  // Already-open short-circuit — right-rail uses data-open on the aside.
  const openPane = page.locator('.right-rail[data-open="true"] .member-list');
  if (await openPane.isVisible().catch(() => false)) return;

  // On mobile the main-pane-header lives inside the channel push —
  // tap a channel to surface it first.
  if (isMobile(page)) {
    const inPush = await page.locator('.mobile-push--channel').isVisible().catch(() => false);
    if (!inPush) {
      await page.locator('.mobile-home .channel-item').first().click();
      await page.waitForTimeout(400);
    }
  }

  const membersBtn = page.locator('.action-btn[aria-label="members"]');
  if (await membersBtn.count() > 0) {
    await membersBtn.first().click();
    await page.locator('.right-rail[data-open="true"] .member-list')
      .waitFor({ timeout: 3_000 })
      .catch(() => {});
    await page.waitForTimeout(200);
  }
}

/** Closes the member list panel by toggling the same button. */
export async function closeMemberList(page: Page) {
  const openPane = page.locator('.right-rail[data-open="true"] .member-list');
  const isOpen = await openPane.isVisible().catch(() => false);
  if (!isOpen) return;

  const desktopBtn = page.locator('.action-btn[aria-label="members"]');
  if (await desktopBtn.count() > 0) {
    await desktopBtn.first().click();
    await page.waitForTimeout(200);
    return;
  }
}

// ── Invite flow ───────────────────────────────────────────────────────

/** Opens the server settings panel (opens sidebar first on mobile). */
export async function openServerSettings(page: Page) {
  await openSidebar(page);
  await page.locator('.server-gear-btn').click();
  await page.waitForTimeout(500);
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
  await page.waitForSelector('.channel-sidebar, .sidebar', { timeout: 20_000 });
  await page.waitForTimeout(3000);
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

  // Peer 2: Get peer ID from welcome screen.
  await freshStart(page2);
  const peer2Id = await getPeerId(page2);

  // Peer 1: Generate invite for peer 2.
  const inviteCode = await generateInvite(page1, peer2Id);

  // Peer 2: Join the server.
  await joinViaInvite(page2, inviteCode, peer2Name);

  // Wait for display name sync: peer2's name should appear in peer1's member list.
  if (peer2Name) {
    await openMemberList(page1);
    try {
      await page1.locator('.member-item', { hasText: peer2Name })
        .waitFor({ timeout: 20_000 });
    } catch {
      // Display name sync may be slow; proceed anyway — but warn so failures
      // here don't produce misleading timeouts in downstream assertions.
      console.warn('[setupTwoPeers] peer2 display name did not sync in time — P2P may be slow');
    }
    await closeMemberList(page1);
  }

  return { ctx1, ctx2, page1, page2 };
}

// ── Channel helpers ───────────────────────────────────────────────────

/** Creates a new text channel. On mobile the channel list is the
 *  home tab — no drawer needed to reach `.channel-add-btn`. */
export async function createChannel(page: Page, name: string) {
  if (isMobile(page)) {
    // Pop any pushed screen so the home tab is visible.
    const backSlot = page.locator('.mobile-top-bar .top-slot-left .top-back');
    while (await backSlot.isVisible().catch(() => false)) {
      await page.locator('.mobile-top-bar .top-slot-left').click();
      await page.waitForTimeout(300);
    }
    await page.locator('.mobile-tab-bar .tab[data-tab="home"]').click();
    await page.waitForTimeout(200);
  }
  await page.locator('.channel-add-btn').click();
  await page.waitForTimeout(200);
  await page.locator('.channel-create-input input').fill(name);
  await page.locator('.channel-create-input input').press('Enter');
  await page.waitForTimeout(500);
}

// ── Message actions ───────────────────────────────────────────────────

/** Performs a named action on a message (desktop: hover+dropdown, mobile: long-press+sheet). */
export async function messageAction(page: Page, messageText: string, actionName: string) {
  const msg = page.locator('.message', { hasText: messageText }).last();

  if (isMobile(page)) {
    // Mobile: long-press to open action sheet.
    await longPress(page, `.message:has-text("${messageText}")`);
    await page.locator('.mobile-action-sheet.open').waitFor({ timeout: 3000 });
    await page.locator('.sheet-item', { hasText: actionName }).click();
    await page.waitForTimeout(300);
  } else {
    // Desktop: hover to reveal action trigger, click dropdown item.
    await msg.hover();
    await page.waitForTimeout(200);
    await msg.locator('.action-trigger').click();
    await page.waitForTimeout(200);
    await page.locator('.dropdown-item', { hasText: actionName }).click();
    await page.waitForTimeout(200);
  }
}

/** Edits a message (desktop or mobile). */
export async function editMessage(page: Page, originalText: string, newText: string) {
  await messageAction(page, originalText, 'Edit');
  const input = page.locator('.input-area input, .input-area textarea').first();
  await input.fill(newText);
  await input.press('Enter');
  await page.waitForTimeout(500);
}

/** Deletes a message (desktop or mobile). */
export async function deleteMessage(page: Page, text: string) {
  await messageAction(page, text, 'Delete');
  // Confirm the deletion dialog.
  const confirmBtn = page.locator('.confirm-dialog .btn-danger', { hasText: 'Delete' });
  await confirmBtn.waitFor({ timeout: 3000 });
  await confirmBtn.click();
  await page.waitForTimeout(500);
}

/** Reacts to a message with an emoji (desktop or mobile). */
export async function reactToMessage(page: Page, messageText: string, emojiIndex = 0) {
  const msg = page.locator('.message', { hasText: messageText }).last();

  if (isMobile(page)) {
    await longPress(page, `.message:has-text("${messageText}")`);
    await page.locator('.mobile-action-sheet.open').waitFor({ timeout: 3000 });
    await page.locator('.sheet-emoji-row button').nth(emojiIndex).click();
    await page.waitForTimeout(500);
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

// ── Permission actions ────────────────────────────────────────────────

/** Trusts a peer by name from the member list. */
export async function trustPeer(page: Page, peerName: string) {
  await openMemberList(page);
  const member = page.locator('.member-item', { hasText: peerName });
  await member.waitFor({ timeout: 30_000 });
  // Hover to reveal action buttons (desktop hides them until hover).
  await member.hover();
  // Use a regex to avoid matching "Untrust" when looking for "Trust".
  await member.locator('button').filter({ hasText: /^Trust$/ }).click();
  await page.waitForTimeout(500);
  await closeMemberList(page);
}

/** Untrusts a peer by name from the member list. */
export async function untrustPeer(page: Page, peerName: string) {
  await openMemberList(page);
  const member = page.locator('.member-item', { hasText: peerName });
  await member.waitFor({ timeout: 30_000 });
  // Hover to reveal action buttons (desktop hides them until hover).
  await member.hover();
  await member.locator('button', { hasText: 'Untrust' }).click();
  await page.waitForTimeout(500);
  await closeMemberList(page);
}

/** Kicks a peer by name from the member list. */
export async function kickPeer(page: Page, peerName: string) {
  await openMemberList(page);
  const member = page.locator('.member-item', { hasText: peerName });
  await member.waitFor({ timeout: 30_000 });
  // Hover to reveal action buttons (desktop hides them until hover).
  await member.hover();
  await member.locator('.btn-danger', { hasText: 'Kick' }).click();
  await page.waitForTimeout(500);
  // Confirm the kick dialog.
  const confirmBtn = page.locator('.confirm-dialog .btn-danger', { hasText: 'Kick' });
  await confirmBtn.waitFor({ timeout: 5_000 });
  await confirmBtn.click();
  await page.waitForTimeout(500);
  await closeMemberList(page);
}

