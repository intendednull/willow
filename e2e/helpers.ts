import { Page, Browser, BrowserContext, Locator, expect } from '@playwright/test';

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
 *  If `displayName` is provided, fills it into the welcome step-1 name
 *  input before advancing — required when the same page will later
 *  invoke `joinViaInvite`, since step 1 only renders once. Without
 *  this, the joiner ends up with display name "anonymous" and member-
 *  list lookups by name fail. */
export async function getPeerId(page: Page, displayName?: string): Promise<string> {
  // Welcome screen: advance past step 1, then switch to the Join tab —
  // the peer id lives inside the Join step list, hidden by default and
  // revealed by the eye-toggle icon.
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

/** Send a message in the current channel. Scopes the locator to the
 *  visible shell so it doesn't hit the hidden copy on the inactive
 *  side of the desktop / mobile split. On mobile, automatically
 *  pushes into the first channel if the composer is not mounted. */
export async function sendMessage(page: Page, text: string) {
  const scope = isMobile(page) ? '.shell-mobile' : '.shell-desktop';
  if (isMobile(page)) {
    const inPush = await page
      .locator('.shell-mobile .mobile-push--channel')
      .isVisible()
      .catch(() => false);
    if (!inPush) {
      await page.locator('.shell-mobile .mobile-home .channel-item').first().click();
      await page.waitForTimeout(400);
    }
  }
  const input = page
    .locator(`${scope} .input-area input, ${scope} .input-area textarea`)
    .first();
  await input.fill(text);
  await input.press('Enter');
  await page.locator(`${visibleShell(page)} .message .body`, { hasText: text })
    .first()
    .waitFor({ timeout: 10_000 });
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

/** Scope selector prefix for the currently-visible shell. Use to
 *  disambiguate elements that are mounted in both shells (the
 *  inactive one is hidden via `display: none`). */
export function visibleShell(page: Page): string {
  return isMobile(page) ? '.shell-mobile' : '.shell-desktop';
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
  await page
    .locator(`${visibleShell(page)} .channel-item`, { hasText: channelName })
    .first()
    .click();
}

/** Wait for a specific message to appear in the visible shell. */
export async function waitForMessage(page: Page, text: string, timeout = 20_000) {
  const scope = visibleShell(page);
  await page
    .locator(`${scope} .message .body`, { hasText: text })
    .first()
    .waitFor({ timeout });
}

/** Simulate a long-press on an element to open the mobile action sheet.
 *  Prefixes the selector with the visible-shell scope so a raw `.message`
 *  picks the mobile copy, not the hidden desktop one. */
export async function longPress(page: Page, selector: string, durationMs = 600) {
  const scoped = isMobile(page) && !selector.startsWith('.shell-')
    ? `${visibleShell(page)} ${selector}`
    : selector;
  const el = page.locator(scoped).first();
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

  const membersBtn = page.locator(`${visibleShell(page)} .action-btn[aria-label="members"]`);
  if (await membersBtn.count() > 0) {
    await membersBtn.first().click();
    await page
      .locator(`${visibleShell(page)} .right-rail[data-open="true"] .member-list`)
      .waitFor({ timeout: 3_000 })
      .catch(() => {});
  }
}

/** Closes the member list panel by toggling the same button. */
export async function closeMemberList(page: Page) {
  const openPane = page.locator(`${visibleShell(page)} .right-rail[data-open="true"] .member-list`);
  const isOpen = await openPane.isVisible().catch(() => false);
  if (!isOpen) return;

  const membersBtn = page.locator(`${visibleShell(page)} .action-btn[aria-label="members"]`);
  if (await membersBtn.count() > 0) {
    await membersBtn.first().click();
  }
}

/** Opens the sync-queue panel via the command palette.
 *
 *  The sync-queue surface (`SyncQueueView`) mounts inside the right-rail
 *  on desktop when `app.queue.open == true`. The user-facing trigger is
 *  the "open sync queue" action in the command palette (⌘K / Ctrl+K).
 *  The `OfflineStrip` is the only other in-app entry point but it gates
 *  on `peer_count > 0`, which isn't reliable immediately after server
 *  creation when no peers are queued yet.
 *
 *  Idempotent — short-circuits when the queue panel is already mounted.
 */
export async function openSyncQueue(page: Page) {
  const alreadyOpen = await page
    .locator(`${visibleShell(page)} .sync-queue-view`)
    .first()
    .isVisible()
    .catch(() => false);
  if (alreadyOpen) return;

  // Open the command palette. The global keydown listener in
  // `crates/web/src/keybindings.rs` toggles `show_palette` on Ctrl/⌘+K.
  await page.keyboard.press('Control+K');
  const row = page.locator('.palette-row', { hasText: 'open sync queue' }).first();
  await row.waitFor({ timeout: 5_000 });
  await row.click();

  // Wait for the surface to mount in the visible shell.
  await page
    .locator(`${visibleShell(page)} .sync-queue-view`)
    .first()
    .waitFor({ timeout: 5_000 });
}

// ── Invite flow ───────────────────────────────────────────────────────

/** Opens the server settings panel (opens sidebar first on mobile). */
export async function openServerSettings(page: Page) {
  if (isMobile(page)) {
    // Channel list is on the home tab; the grove header lives in the
    // sidebar rendered inside `.mobile-home`. No drawer needed.
    const backSlot = page.locator('.mobile-top-bar .top-slot-left .top-back');
    while (await backSlot.isVisible().catch(() => false)) {
      await page.locator('.mobile-top-bar .top-slot-left').click();
      await page.waitForTimeout(300);
    }
    await page.locator('.mobile-tab-bar .tab[data-tab="home"]').click();
    await page.waitForTimeout(200);
  }
  await page.locator(`${visibleShell(page)} [aria-label="grove menu"]`).first().click();
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
  // Deterministic post-join settle: wait for the sidebar + first channel
  // to materialise. Covers both shells.
  await page.locator(`${visibleShell(page)} .channel-sidebar, ${visibleShell(page)} .mobile-home`)
    .first()
    .waitFor({ timeout: 20_000 });
  await page.locator(`${visibleShell(page)} .channel-item`).first()
    .waitFor({ timeout: 20_000 });
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

  // Peer 2: Get peer ID from welcome screen. Pass peer2Name so step 1
  // commits with the correct display name — `joinViaInvite` below cannot
  // re-set it because step 1 has already been advanced.
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
    // Throw on timeout instead of swallowing — silent fallback hides
    // genuine sync regressions and produces misleading downstream
    // failures (e.g. trustPeer/kickPeer can't find the member row).
    await page1
      .locator('.member-item', { hasText: peer2Name })
      .waitFor({ timeout: 60_000 });
    await closeMemberList(page1);
  } else if (peer2Name) {
    // On mobile, just sleep a bit to let gossip propagate.
    await page1.waitForTimeout(1500);
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
  const scope = visibleShell(page);
  await page.locator(`${scope} .channel-add-btn`).first().click();
  await page.waitForTimeout(200);
  await page.locator(`${scope} .channel-create-input input`).first().fill(name);
  await page.locator(`${scope} .channel-create-input input`).first().press('Enter');
  await page.locator(`${visibleShell(page)} .channel-item`, { hasText: name })
    .waitFor({ timeout: 10_000 });
}

// ── Message actions ───────────────────────────────────────────────────

/** Performs a named action on a message (desktop: hover+dropdown, mobile: long-press+sheet).
 *  Mobile sheet copy is lowercase per `message-row.md` §Long-press
 *  action sheet — the helper matches `actionName` case-insensitively
 *  so callers can pass either `Reply` or `reply`. */
export async function messageAction(page: Page, messageText: string, actionName: string) {
  if (isMobile(page)) {
    // Mobile: long-press to open action sheet.
    await longPress(page, `.message:has-text("${messageText}")`);
    await page.locator('.shell-mobile .mobile-action-sheet.open').first()
      .waitFor({ timeout: 3000 });
    // Case-insensitive match: spec copy is lowercase `reply`, `edit`,
    // `delete`, but call-sites historically passed capitalized names.
    const actionRe = new RegExp(`^\\s*${actionName}\\s*$`, 'i');
    await page
      .locator('.shell-mobile .mobile-action-sheet.open .sheet-item', { hasText: actionRe })
      .click();
    await page.waitForTimeout(300);
  } else {
    const msg = page.locator('.shell-desktop .message', { hasText: messageText }).last();
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
  if (isMobile(page)) {
    await longPress(page, `.message:has-text("${messageText}")`);
    await page.locator('.shell-mobile .mobile-action-sheet.open').first()
      .waitFor({ timeout: 3000 });
    await page.locator('.shell-mobile .mobile-action-sheet.open .sheet-emoji-row button')
      .nth(emojiIndex).click();
    await page.waitForTimeout(500);
  } else {
    const msg = page.locator('.shell-desktop .message', { hasText: messageText }).last();
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

/** Open the compare-fingerprints dialog by clicking the trust badge
 *  next to a peer name in the member list.
 */
export async function openCompareFingerprints(page: Page, peerName: string) {
  await openMemberList(page);
  const member = page.locator(`${visibleShell(page)} .member-item`, { hasText: peerName });
  await member.waitFor({ timeout: 10_000 });
  await member.locator('.trust-badge').click();
  await page
    .locator('.add-friend__card[role="dialog"]')
    .waitFor({ timeout: 5_000 });
}

/** Click "they match" in the compare-fingerprints dialog. */
export async function markFingerprintsMatch(page: Page) {
  await page
    .locator('.add-friend__cta-primary', { hasText: 'they match' })
    .click();
  // Confirm screen appears.
  await page
    .locator('.add-friend__confirm-title', { hasText: 'verified.' })
    .waitFor({ timeout: 5_000 });
}

/** Click "they don't match" in the compare-fingerprints dialog. */
export async function markFingerprintsMismatch(page: Page) {
  await page
    .locator('.add-friend__cta-secondary', { hasText: "they don't match" })
    .click();
  await page
    .locator('.add-friend__confirm-title', { hasText: 'marked not verified.' })
    .waitFor({ timeout: 5_000 });
}

/** Long-press a peer avatar by name in the member list (mobile only). */
export async function longPressAvatar(page: Page, peerName: string) {
  await openMemberList(page);
  const member = page.locator(`${visibleShell(page)} .member-item`, { hasText: peerName });
  await member.waitFor({ timeout: 10_000 });
  const target = member.locator('.long-press-avatar, .status-dot').first();
  const box = await target.boundingBox();
  if (!box) throw new Error('avatar not measurable');
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.waitForTimeout(500);
  await page.mouse.up();
}

// ── Swipe gestures ────────────────────────────────────────────────────

/** Dispatches a horizontal swipe (touchstart → 3× touchmove → touchend)
 *  on a message row. `dx > 0` swipes right (open thread); `dx < 0`
 *  swipes left (quote reply). The four-step move path is required to
 *  cross the 8 px horizontal-dominance gate inside MessageView's
 *  touchmove handler before the row captures the gesture. */
async function dispatchSwipe(row: Locator, dx: number): Promise<void> {
  await row.evaluate((el, dx) => {
    const rect = (el as HTMLElement).getBoundingClientRect();
    // Start off-centre on the opposite side so we have room to travel
    // `dx` pixels without leaving the row's bounding box.
    const startX = dx > 0 ? rect.left + rect.width * 0.2 : rect.left + rect.width * 0.8;
    const startY = rect.top + rect.height / 2;
    const makeTouch = (x: number, y: number) => new Touch({
      identifier: 0,
      target: el as HTMLElement,
      clientX: x,
      clientY: y,
      pageX: x,
      pageY: y,
    } as TouchInit);
    const fire = (type: string, x: number) => {
      const touch = makeTouch(x, startY);
      (el as HTMLElement).dispatchEvent(new TouchEvent(type, {
        cancelable: true,
        bubbles: true,
        touches: type === 'touchend' ? [] : [touch],
        targetTouches: type === 'touchend' ? [] : [touch],
        changedTouches: [touch],
      }));
    };
    fire('touchstart', startX);
    fire('touchmove', startX + dx * 0.3);
    fire('touchmove', startX + dx * 0.7);
    fire('touchmove', startX + dx);
    fire('touchend', startX + dx);
  }, dx);
}

/** Swipe left on a message row. Populates the composer's `replying_to`
 *  context (per `message-row.md` §Swipe gestures). */
export async function swipeLeft(_page: Page, row: Locator): Promise<void> {
  return dispatchSwipe(row, -120);
}

/** Swipe right on a message row. Opens the thread pane (per
 *  `message-row.md` §Swipe gestures). */
export async function swipeRight(_page: Page, row: Locator): Promise<void> {
  return dispatchSwipe(row, 120);
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

