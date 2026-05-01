/* eslint-disable no-restricted-syntax -- migration tracked at https://github.com/intendednull/willow/issues/458 */
//
// UI navigation + message-action helpers. Extracted from legacy
// e2e/helpers.ts per docs/plans/2026-04-29-event-based-waits-pr2-peer-wrapper.md
// Task 8. Behaviour preserved verbatim.

import { Page } from '@playwright/test';
import { longPress } from './touch';

/** Returns true if the page viewport is narrow enough to be mobile. */
export function isMobile(page: Page): boolean {
  return (page.viewportSize()?.width ?? 1024) < 768;
}

/** Scope selector prefix for the currently-visible shell. Use to
 *  disambiguate elements that are mounted in both shells (the
 *  inactive one is hidden via `display: none`). */
export function visibleShell(page: Page): string {
  return isMobile(page) ? '.shell-mobile' : '.shell-desktop';
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
  // The "new" button now opens a kind picker (text / voice / temp)
  // before the name input renders; the name input itself is
  // `.tree-slot__input`. The previous `.channel-create-input` selector
  // and one-shot fill no longer apply (channel_sidebar.rs:317-384).
  await page.locator(`${scope} .channel-add-btn`).first().click();
  await page
    .locator(`${scope} .tree-kind-picker__item`, { hasText: 'text' })
    .first()
    .click();
  const nameInput = page.locator(`${scope} .tree-slot__input`).first();
  await nameInput.waitFor({ timeout: 5_000 });
  await nameInput.fill(name);
  await nameInput.press('Enter');
  await page.locator(`${visibleShell(page)} .channel-item`, { hasText: name })
    .waitFor({ timeout: 10_000 });
}

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
