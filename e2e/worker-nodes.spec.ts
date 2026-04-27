import { test, expect } from '@playwright/test';
import {
  freshStart,
  createServer,
  openMemberList,
  openSyncQueue,
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

    // Smoke check: app shell mounted. Proves the WASM client booted,
    // joined a server, and rendered the channel surface. Desktop
    // renders `.main-pane-header`; mobile renders `.mobile-top-bar`.
    // `:visible` scopes to the active shell so we don't match the
    // hidden inactive copy on the other side of the 720 px split.
    // On its own this only proves the UI mounted — the actual relay
    // reachability assertion is below.
    await expect(
      page.locator('.main-pane-header:visible, .mobile-top-bar:visible').first()
    ).toBeVisible({ timeout: 20_000 });

    // Mobile shell does not yet expose the sync-queue surface or the
    // command palette — the relay-reachability assertion runs on
    // desktop only. The shell smoke check above still runs in both
    // projects.
    if (!testInfo.project.name.startsWith('mobile')) {
      // Real relay-reachability assertion. The `--ok` modifier on
      // `.relay-signal-button` is set ONLY when the live iroh handshake
      // reports `RelayStatus::Reachable` (data flow:
      // `Network::relay_status` → `state_actors.rs` →
      // `mutations.rs` → `RelaySignalButton::class_for`). The button
      // is mounted only inside `<SyncQueueView>` (gated on
      // `app.queue.open == true`), so we open the panel first.
      // 30 s ceiling covers CI cold-start: trunk-served WASM load +
      // iroh handshake to a freshly-spawned relay can comfortably
      // exceed Playwright's default 5 s.
      await openSyncQueue(page);
      await expect(
        page.locator('.relay-signal-button.relay-signal-button--ok:visible').first()
      ).toBeVisible({ timeout: 30_000 });

      // Alice is the sole member on desktop after server creation.
      // Mobile defers peer member-rendering to a later phase.
      await openMemberList(page);
      const members = page.locator(`${visibleShell(page)} .member-item`);
      await expect(members).toHaveCount(1, { timeout: 5_000 });
    }
  });
});
