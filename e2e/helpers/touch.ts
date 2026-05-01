/* eslint-disable no-restricted-syntax -- migration tracked at https://github.com/intendednull/willow/issues/458 */
//
// Touch + gesture helpers. Extracted from legacy e2e/helpers.ts per
// docs/plans/2026-04-29-event-based-waits-pr2-peer-wrapper.md Task 9.
// Behaviour preserved verbatim — page.clock migration is a follow-up.

import { Page, Locator } from '@playwright/test';
import { isMobile, visibleShell, openMemberList } from './ui';

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

/**
 * Like `longPress`, but uses `page.clock.runFor(durationMs)` instead of
 * a real-time wait. Caller must have invoked `installPageClock(page)`
 * earlier in the test (see `helpers/clock.ts`).
 *
 * Use this in tests where the clock is already installed for other
 * reasons; otherwise prefer the real-time `longPress` to avoid having
 * to install the clock just for one helper.
 *
 * Per docs/specs/2026-04-27-event-based-waits-design.md §`page.clock`.
 */
export async function longPressWithClock(page: Page, selector: string, durationMs = 600) {
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

  await page.clock.runFor(durationMs);

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

  await page.clock.runFor(300);
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
