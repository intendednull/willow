import { Page, expect } from '@playwright/test';

/** Wait for the WASM app to load (loading spinner disappears). */
export async function waitForApp(page: Page) {
  // Wait for the app to render (either welcome screen or chat).
  await page.waitForSelector('.welcome-screen, .app, .sidebar', {
    timeout: 30_000,
  });
  // Give WASM a moment to stabilize.
  await page.waitForTimeout(1000);
}

/** Clear all Willow localStorage keys and reload. */
export async function freshStart(page: Page) {
  await page.goto('/');
  await page.evaluate(() => {
    const keys = Object.keys(localStorage).filter(k => k.startsWith('willow_'));
    keys.forEach(k => localStorage.removeItem(k));
    // Also clear non-prefixed keys that might be ours.
    localStorage.clear();
  });
  await page.reload();
  await waitForApp(page);
}

/** Create a server from the welcome screen. Returns the server name. */
export async function createServer(page: Page, name: string, displayName?: string) {
  // Should be on welcome screen.
  await expect(page.locator('.welcome-card')).toBeVisible();

  // Fill server name.
  const serverInput = page.locator('.welcome-option').first().locator('input').first();
  await serverInput.fill(name);

  // Optional display name.
  if (displayName) {
    const dnInput = page.locator('.welcome-option').first().locator('input').nth(1);
    await dnInput.fill(displayName);
  }

  // Click Create Server.
  await page.locator('.welcome-option').first().locator('button.btn-primary').click();

  // Wait for the app to load with the new server.
  await page.waitForSelector('.sidebar', { timeout: 10_000 });
  await page.waitForTimeout(500);
}

/** Get the peer ID from the welcome screen or settings. */
export async function getPeerId(page: Page): Promise<string> {
  // Check welcome screen first.
  const peerIdEl = page.locator('.peer-id-text').first();
  if (await peerIdEl.isVisible()) {
    return (await peerIdEl.textContent()) || '';
  }
  // Try settings.
  await page.locator('text=Settings').click();
  await page.waitForTimeout(300);
  const settingsPeerId = page.locator('.peer-id-text').first();
  const id = (await settingsPeerId.textContent()) || '';
  return id;
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

/** Click a channel in the sidebar. */
export async function switchChannel(page: Page, channelName: string) {
  await page.locator('.channel-item', { hasText: channelName }).click();
  await page.waitForTimeout(300);
}

/** Wait for a specific message to appear. */
export async function waitForMessage(page: Page, text: string, timeout = 15_000) {
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
