import { test, expect } from '@playwright/test';
import {
  freshStart,
  createServer,
  openMemberList,
  openServerSettings,
  visibleShell,
} from './helpers';

test.describe('Worker nodes infrastructure', () => {
  test.setTimeout(60_000);

  test('member list renders with correct section structure', async ({ page }, testInfo) => {
    // Member list is the desktop right rail now; Phase 1a put it behind
    // the members action button instead of always-visible.
    test.skip(testInfo.project.name.startsWith('mobile'), 'member list ships through the mobile drawer in a later phase');
    await freshStart(page);
    await createServer(page, 'Section Test', 'Alice');
    await page.waitForTimeout(3000);

    // Open the right-rail member list.
    await openMemberList(page);

    const memberList = page.locator(`${visibleShell(page)} .member-list`);
    await expect(memberList).toBeVisible();

    // Members heading (h3) still present per spec §Right rail.
    await expect(page.locator(`${visibleShell(page)} .member-list h3`, { hasText: 'Members' }))
      .toBeVisible();

    // Owner should have Owner badge.
    await expect(page.locator(`${visibleShell(page)} .badge.owner-badge`)).toBeVisible();
  });

  test('relay connection is established after server creation', async ({ page }, testInfo) => {
    await freshStart(page);
    await createServer(page, 'Relay Test', 'Alice');

    // Wait for relay connection to establish.
    // App indicates reachability by rendering either the desktop net
    // status footer or the mobile top bar once the client has a peer
    // id and peer count signal. `:visible` scopes to the active shell.
    await expect(
      page.locator('.net-status-footer:visible, .mobile-top-bar:visible').first()
    ).toBeVisible({ timeout: 20_000 });

    // Alice should always be in the member list on desktop. Mobile
    // defers peer member-rendering to a later phase.
    if (!testInfo.project.name.startsWith('mobile')) {
      await openMemberList(page);
      const members = page.locator(`${visibleShell(page)} .member-item`);
      await expect(members).toHaveCount(1, { timeout: 5_000 });
    }
  });

  test('infrastructure section hidden when no workers have SyncProvider', async ({ page }, testInfo) => {
    test.skip(testInfo.project.name.startsWith('mobile'), 'member list always-visible on desktop only');
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

    // Check that our CSS classes are defined (query across all loaded
    // stylesheets — foundation.css sits at index 0 after Phase 0).
    const hasWorkerStyles = await page.evaluate(() => {
      for (const sheet of Array.from(document.styleSheets)) {
        try {
          const rules = Array.from(sheet.cssRules);
          if (
            rules.some(
              (r) =>
                r instanceof CSSStyleRule &&
                (r.selectorText.includes('.worker-item') ||
                  r.selectorText.includes('.worker-icon') ||
                  r.selectorText.includes('.infra-header'))
            )
          ) {
            return true;
          }
        } catch {
          // Cross-origin stylesheets throw on cssRules access — skip.
        }
      }
      return false;
    });
    expect(hasWorkerStyles).toBe(true);
  });
});
