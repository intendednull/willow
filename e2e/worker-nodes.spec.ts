import { test, expect } from '@playwright/test';
import {
  freshStart,
  createServer,
  openMemberList,
  visibleShell,
} from './helpers';

// Member-list section structure, the hidden-infrastructure section, and
// the stylesheet scan all migrated to wasm-pack
// (`crates/web/tests/browser.rs::mod worker_nodes_css`) because they are
// pure-DOM / pure-CSS checks that a single client can verify. Only the
// relay-connection test stays here — it asserts that a real relay is
// reachable after server creation, which requires the Playwright
// transport stack.

test.describe('Worker nodes infrastructure', () => {
  test.setTimeout(60_000);

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
});
