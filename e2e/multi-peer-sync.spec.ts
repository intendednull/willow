import { test, expect } from './test-hooks';
import {
  freshStart,
  createServer,
  sendMessage,
  waitForMessage,
  getPeerId,
  switchChannel,
  setupTwoPeers,
  generateInvite,
  joinViaInvite,
  createChannel,
  openSidebar,
  openMemberList,
  visibleShell,
  waitForApp,
} from './helpers';

// Shared relay + gossip mesh — keep tests inside this file sequential
// so they don't stampede the relay while `fullyParallel: true` runs
// different spec files concurrently.
test.describe.configure({ mode: 'serial' });

test.describe('Multi-peer state synchronization', () => {
  // Two-peer tests need extra time for setup + P2P sync.
  test.setTimeout(120_000);

  // Sync-semantic tests (messages/edits/deletes/reactions/typing/display-names/
  // history-replay/reconnect-replay/persist-after-refresh) live in
  // crates/client/src/tests/multi_peer_sync.rs against MemNetwork — the
  // DAG merge path is identical and the test runs in < 200 ms.
  // Only DOM-reflection tests stay here.
  //
  // Migration to event-based waits per PR-2 (issue #458). Cross-peer
  // assertions now gate on Peer.waitUntilHeadsEqual / Peer.nextEvent;
  // DOM checks then run with the default 5s assertion timeout.

  test('invite flow — both peers see sidebar and general channel', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Both peers should converge before we assert UI state.
      await bob.waitUntilHeadsEqual(alice);

      // Both peers should see the sidebar (default 5s timeout — convergence already done).
      await expect(page1.locator(`${visibleShell(page1)} .channel-sidebar, ${visibleShell(page1)} .mobile-home`).first()).toBeVisible();
      await expect(page2.locator(`${visibleShell(page2)} .channel-sidebar, ${visibleShell(page2)} .mobile-home`).first()).toBeVisible();

      // Both peers should see the general channel.
      await expect(page1.locator(`${visibleShell(page1)} .channel-item`, { hasText: 'general' })).toBeVisible();
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'general' })).toBeVisible();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('pre-existing channels visible after join', async ({ peer, browser }) => {
    const ctx1 = await browser.newContext();
    const ctx2 = await browser.newContext();
    const page1 = await ctx1.newPage();
    const page2 = await ctx2.newPage();
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');

    try {
      // Peer 1: Create server.
      await freshStart(page1);
      await createServer(page1, 'PreChan Server', 'Alice');

      // Create 2 extra channels BEFORE invite.
      await createChannel(page1, 'announcements');
      await createChannel(page1, 'random');

      // Peer 2: Get peer ID.
      await freshStart(page2);
      const peer2Id = await getPeerId(page2);

      // Peer 1: Generate invite.
      const inviteCode = await generateInvite(page1, peer2Id);

      // Peer 2: Join.
      await joinViaInvite(page2, inviteCode, 'Bob');

      // Bob should converge to Alice's heads — including the two pre-existing channels.
      await bob.waitUntilHeadsEqual(alice);

      // Peer 2 should see all 3 channels (open sidebar on mobile).
      await openSidebar(page2);
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'general' })).toBeVisible();
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'announcements' })).toBeVisible();
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'random' })).toBeVisible();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('new channel created mid-session syncs to peer', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Alice creates a new channel after both are connected.
      await createChannel(page1, 'new-channel');

      // Wait for Bob's DAG to converge to Alice's (includes the new channel event).
      await bob.waitUntilHeadsEqual(alice);

      // Bob should see the new channel (open sidebar on mobile).
      await openSidebar(page2);
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'new-channel' })).toBeVisible();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('messages in non-general channel sync', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Alice creates a new channel.
      await createChannel(page1, 'dev');

      // Wait for Bob's DAG to include the channel.
      await bob.waitUntilHeadsEqual(alice);

      // Bob can now see the channel without padding.
      await openSidebar(page2);
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'dev' })).toBeVisible();

      // Both switch to the new channel.
      await switchChannel(page1, 'dev');
      await switchChannel(page2, 'dev');

      // Alice sends a message → wait for Bob's MessageReceived event,
      // then assert the DOM-rendered body.
      await sendMessage(page1, 'message in dev');
      await bob.nextEvent(e =>
        e.kind === 'MessageReceived' &&
        e.channel === 'dev' &&
        !e.isLocal
      );
      await waitForMessage(page2, 'message in dev');

      // Bob sends a reply, Alice consumes the event then asserts the body.
      await sendMessage(page2, 'bob in dev too');
      await alice.nextEvent(e =>
        e.kind === 'MessageReceived' &&
        e.channel === 'dev' &&
        !e.isLocal
      );
      await waitForMessage(page1, 'bob in dev too');
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('both peers appear in member list', async ({ peer, browser }, testInfo) => {
    // Mobile shell wires the members action button to a no-op callback
    // (mobile_shell.rs ~L386 `on_set_which=Callback::new(|_| ())`) so
    // there's no right-rail member pane to assert on. Re-enable when
    // mobile-shell exposes the member list (Phase 1c).
    test.skip(testInfo.project.name.startsWith('mobile'), 'mobile shell does not expose member list');

    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Wait for the membership events to converge before opening the panel.
      await bob.waitUntilHeadsEqual(alice);

      await openMemberList(page1);

      // Default expect timeout (5s) is plenty after convergence.
      const memberList = page1.locator(`${visibleShell(page1)} .member-item`);
      await expect(memberList.first()).toBeVisible();
      await expect.poll(() => memberList.count()).toBeGreaterThanOrEqual(2);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('rapid channel creation by owner — both channels propagate to peer', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Alice (owner) creates two channels back-to-back.
      await createChannel(page1, 'chan-a');
      await createChannel(page1, 'chan-b');

      // Wait for Bob's DAG to include both.
      await bob.waitUntilHeadsEqual(alice);

      // Both should appear on Bob's side after gossip delivery.
      await openSidebar(page2);
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'chan-a' })).toBeVisible();
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'chan-b' })).toBeVisible();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  // ── Relay discovery: pkarr bootstrap + mid-session migration ───────────
  //
  // Covers the two Integration rows of the outbox-relay-discovery spec
  // (docs/specs/2026-04-24-outbox-relay-discovery.md §Tests) and PR 6
  // Task 6.4 of docs/plans/2026-05-28-relay-upgrade-bundle.md. These ride
  // the real iroh/QUIC mesh + relay, so they sit at the Playwright tier
  // (not MemNetwork): pkarr address resolution and relay-mediated dialing
  // are exactly the real-network behaviour MemNetwork cannot exercise.

  test('share-link join uses bootstrap_endpoint_ids and falls back when the first bootstrap is unreachable', async ({ peer, browser, baseURL }, testInfo) => {
    // Firefox: clipboard permissions for reading the generated link are
    // unsupported (same gate as join-links.spec.ts).
    test.skip(testInfo.project.name.includes('firefox'), 'clipboard permissions not supported in Firefox');
    // Mobile P2P join over the real relay flakes past budget — the Layer-1
    // resolution + fallback logic is unit-tested in crates/web/src/app.rs
    // (bootstrap_preference_*); this row asserts the live end-to-end path.
    test.skip(testInfo.project.name.startsWith('mobile'), 'mobile P2P join-url real-network flake');
    test.setTimeout(120_000);

    // Alice owns the server and mints a share link. The inviter now ships
    // the server's live SyncProvider EndpointIds in the token
    // (bootstrap_endpoint_ids) so the joiner can resolve them via pkarr.
    const ctxA = await browser.newContext({
      permissions: ['clipboard-read', 'clipboard-write'],
    });
    const pageA = await ctxA.newPage();
    const alice = await peer(pageA, 'Alice');
    await freshStart(pageA);
    await createServer(pageA, 'Discovery Test', 'Alice');

    // Generate the join link from settings (same UI path as join-links.spec).
    await openSidebar(pageA);
    await pageA.locator(`${visibleShell(pageA)} [aria-label="grove menu"]`).first().click();
    await pageA.locator('.settings-panel, .settings-overlay').first().waitFor({ timeout: 5_000 });
    await pageA.locator('button', { hasText: 'Create Invite Link' }).click();
    await pageA.locator('.copied-tooltip', { hasText: 'Copied!' }).waitFor({ timeout: 5_000 });
    const clipboardUrl = await pageA.evaluate(() => navigator.clipboard.readText());
    expect(clipboardUrl).toContain('#join=');
    await pageA.locator('text=Back').click();

    // Rewrite the token so its bootstrap_endpoint_ids LEAD with a freshly
    // generated — and therefore unreachable — EndpointId. The join must
    // still complete: an undiallable first bootstrap is skipped and the
    // surviving bootstrap / relay fallback carries the sync (spec Layer-1
    // fallback policy, pinned decision Q2).
    const originalToken = clipboardUrl.substring(clipboardUrl.indexOf('#join=') + '#join='.length);
    const rewrittenToken = await pageA.evaluate(
      (t) => (window as unknown as {
        __willow: { prepend_unreachable_bootstrap: (s: string) => string };
      }).__willow.prepend_unreachable_bootstrap(t),
      originalToken,
    );
    expect(rewrittenToken).not.toBe(originalToken);
    const joinUrl = `${baseURL}/#join=${rewrittenToken}`;

    // Bob opens the rewritten link. The first bootstrap is dead; resolution
    // falls through. We assert convergence — not a specific transport — so
    // the test pins the spec's observable contract (the join completes),
    // not an implementation detail of which path won.
    const ctxB = await browser.newContext();
    const pageB = await ctxB.newPage();
    const bob = await peer(pageB, 'Bob');
    await pageB.goto(joinUrl);
    await waitForApp(pageB);

    await expect(pageB.locator('.join-card-server')).toContainText('Discovery Test', { timeout: 10_000 });
    await pageB.locator('.join-card-field input').fill('Bob');
    await pageB.locator('.join-card-btn').click();

    // The chat shell mounts post-join and Bob's DAG converges with Alice's,
    // proving the dead first bootstrap did not block the sync.
    await pageB.locator('.app-shell, .mobile-top-bar').first().waitFor();
    await bob.waitUntilHeadsEqual(alice);

    await openSidebar(pageB);
    await expect(pageB.locator(`${visibleShell(pageB)} .channel-item`, { hasText: 'general' })).toBeVisible();

    await ctxA.close();
    await ctxB.close();
  });

  test('authority change mid-session — existing client keeps syncing without reload', async ({ peer, browser }, testInfo) => {
    // The member-list grant UI is desktop-only (same gate the member-list
    // test uses); the mid-session re-resolution guarantee is itself
    // browser-agnostic.
    test.skip(testInfo.project.name.startsWith('mobile'), 'mobile shell does not expose the grant UI');
    test.setTimeout(120_000);

    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Baseline: both peers are connected and converged.
      await bob.waitUntilHeadsEqual(alice);

      // Owner changes the server's authority/provider set mid-session. The
      // member list's grant action ("Trust") is the UI-drivable authority
      // change available here (granting admin → implicit all-permissions,
      // which subsumes SyncProvider). A relay/worker migration is the same
      // DAG-level shape: the live authoritative-provider set changes while
      // existing sessions stay open. Clients must pick up the change by
      // re-resolving on the open mesh — pkarr resolves each EndpointId's
      // current addresses on demand — without a page reload.
      await openMemberList(page1);
      const bobMember = page1.locator('.member-item', { hasText: 'Bob' });
      await bobMember.waitFor({ timeout: 20_000 });
      // Exact match: `hasText: 'Trust'` would also match the sibling
      // "Untrust" button (substring), so anchor to the whole label.
      const trustBtn = bobMember.locator('button', { hasText: /^Trust$/ });
      await trustBtn.waitFor({ timeout: 10_000 });
      await trustBtn.click();

      // The grant is itself an event; both peers converge on it.
      await bob.waitUntilHeadsEqual(alice);

      // Crucial assertion: WITHOUT any page reload, a NEW mutation by Alice
      // after the authority change still propagates to Bob. If the session
      // had to restart to pick up the changed provider set, this channel
      // would never reach Bob. No `page.reload()` appears anywhere in this
      // test — that absence is the assertion.
      await createChannel(page1, 'post-migration');
      await bob.waitUntilHeadsEqual(alice);

      await openSidebar(page2);
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'post-migration' })).toBeVisible();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });
});
