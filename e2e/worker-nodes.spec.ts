import { test, expect } from '@playwright/test';
import {
  freshStart,
  createServer,
  openServerSettings,
} from './helpers';

test.describe('Worker nodes infrastructure', () => {
  test.setTimeout(60_000);

  // Skip on mobile — member list toggle behavior differs.
  test.beforeEach(({}, testInfo) => {
    test.skip(testInfo.project.name.startsWith('mobile'), 'desktop only');
  });

  test('member list renders with correct section structure', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Section Test', 'Alice');
    await page.waitForTimeout(3000);

    // On desktop, member list is always visible.
    const memberList = page.locator('.member-list');
    await expect(memberList).toBeVisible();

    // "Members" header should always be present.
    await expect(page.locator('.member-list h3', { hasText: 'Members' }))
      .toBeVisible();

    // Owner should have Owner badge.
    await expect(page.locator('.badge.owner-badge')).toBeVisible();
  });

  test('relay peer visible in member list', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Relay Test', 'Alice');

    // Wait for relay connection.
    await page.waitForTimeout(5000);

    // Member list should show at least Alice + relay (workers may also appear).
    const members = page.locator('.member-item');
    const count = await members.count();
    // Wait for at least 2 members (Alice + relay); workers may add more.
    await expect(async () => {
      expect(await members.count()).toBeGreaterThanOrEqual(2);
    }).toPass({ timeout: 15_000 });
  });

  test('infrastructure section hidden when no workers have SyncProvider', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'No Workers', 'Alice');
    await page.waitForTimeout(3000);

    // No infra section should be visible (no workers authorized yet).
    const infraHeader = page.locator('.infra-header');
    await expect(infraHeader).toBeHidden();

    // Workers section CSS classes should not exist in the DOM.
    const workerItems = page.locator('.worker-item');
    await expect(workerItems).toHaveCount(0);
  });

  test('worker item CSS classes exist in stylesheet', async ({ page }) => {
    // Verify the worker node styles are loaded correctly.
    await freshStart(page);
    await createServer(page, 'CSS Test', 'Alice');

    // Check that our CSS classes are defined (query computed styles).
    const hasWorkerStyles = await page.evaluate(() => {
      const sheet = document.styleSheets[0];
      if (!sheet) return false;
      try {
        const rules = Array.from(sheet.cssRules);
        return rules.some(
          (r) =>
            r instanceof CSSStyleRule &&
            (r.selectorText.includes('.worker-item') ||
              r.selectorText.includes('.worker-icon') ||
              r.selectorText.includes('.infra-header'))
        );
      } catch {
        return false;
      }
    });
    expect(hasWorkerStyles).toBe(true);
  });
});
