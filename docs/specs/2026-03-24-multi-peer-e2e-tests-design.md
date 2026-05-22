# Multi-Peer E2E Browser Tests Design

**Date:** 2026-03-24
**Status:** landed — three new spec files + 4-browser config + shared helpers all shipped. Test enumeration in the original design body below has been intentionally *reduced* over time as the e2e-test-architecture spec migrated logic-only assertions down to the client tier — see *Realised state* below for the up-to-date catalog.
**Implementation plan:** [`docs/plans/2026-03-24-multi-peer-e2e-tests.md`](../plans/2026-03-24-multi-peer-e2e-tests.md)

## Realised state

The original 25-test enumeration in the body below was the target as of
2026-03-24. The realised state diverges in several ways because the
[`e2e-test-architecture`](2026-04-21-e2e-test-architecture-design.md) spec
that landed afterwards mandates pushing every behaviour to the lowest test
tier that can cover it. Many scenarios that were originally enumerated as
Playwright tests now live in `crates/client/src/tests/` (state + client
tier), where they run orders of magnitude faster and without browser
infrastructure. The Playwright suite retains the DOM-reflection-only
scenarios that genuinely require multi-peer browser interaction.

Current per-file enumeration:

| File | Count | Notes |
|---|---|---|
| `e2e/multi-peer-sync.spec.ts` | 6 tests | Tests 2/6/7/8/9/11/12 from the original enumeration (messages-sync, reactions, edits, deletes, refresh-persists, typing, display-names) migrated to `crates/client/src/tests/multi_peer_sync.rs` per the lowest-tier rule. The surviving Playwright tests cover DOM rendering of the joiner's view of pre-existing state. See the file's header comment for the migration log. |
| `e2e/permissions.spec.ts` | 6 tests | Tests 1-4 (trust/untrust DOM contracts) migrated to `crates/client/src/tests/trust_flow.rs` and `crates/web/tests/browser.rs`. Test 7 (role CRUD) is deferred. Test 8 morphed to "non-owner has no action buttons". Two additional Phase 1d SAS-compare trust-verification tests (`compare match flips trust badge to verified`, `compare mismatch keeps peer unverified`) were added — cross-reference [`trust-verification.md`](2026-04-19-ui-design/trust-verification.md). |
| `e2e/multi-peer-mobile.spec.ts` | 4 tests | Test 4 (member list on mobile) is deferred — the mobile shell does not yet expose member list; tracked in #595. |

Realised helper structure (post Phase 2's "Peer wrapper" refactor):

- `e2e/helpers.ts` is now an 11-line barrel re-exporting from
  `e2e/helpers/{peers,ui,touch}.ts` (added during
  [`event-based-waits PR 2`](../plans/2026-04-29-event-based-waits-pr2-peer-wrapper.md)).
- Selectors named in the original body have been renamed by the UI phase
  refactors: `.mobile-nav-toggle` → `.mobile-top-bar .top-slot-left`,
  `.sidebar-overlay` → `.grove-drawer-backdrop`, `.server-gear-btn` →
  `.channel-sidebar .grove-header` (replaced by the vibe-annotations pass
  in commit 0861f26), `.mobile-members-toggle` → `.action-btn[aria-label="members"]`
  in the main-pane header, `.channel-create-input` →
  `.tree-kind-picker__item` + `.tree-slot__input` (kind picker now
  precedes the name input).
- Welcome flow copy changed: `Next/Join Server` → `continue/Join grove`,
  with display name handled on step 1 via `advancePastNameStep` rather
  than mid-invite.

The original test counts and execution-multiplier math (25 tests, 90
executions) are obsolete. The actual catalog is 6 + 6 + 4 = 16 tests
with several mobile-only skips, and the cross-browser project list is
maintained in `playwright.config.ts`.

The body below is preserved as the original target for historical
context. The realised state above is authoritative; do not edit the
body to match it.

## Problem

Multi-peer E2E tests (`two-peer.spec.ts`, `state-sync.spec.ts`) only run on desktop Chrome via hardcoded `chromium.launch()`. Permission/trust/kick scenarios have zero E2E coverage. Mobile multi-peer interactions are untested.

## Scope

- **In scope:** Playwright E2E tests for multi-peer state sync, permissions, and mobile-specific multi-peer flows. All tests run across 4 browser projects (Desktop Chrome, Mobile Chrome, Desktop Firefox, Mobile Firefox).
- **Out of scope:** Single-peer tests (already covered by `basic-flow.spec.ts`, `mobile.spec.ts`). Relay/network-level tests (covered by Rust integration tests).

## Design

### File Structure

**New file: `e2e/multi-peer-sync.spec.ts`** — Core sync tests running on all 4 browsers. Replaces `two-peer.spec.ts` and `state-sync.spec.ts`.

**New file: `e2e/permissions.spec.ts`** — Permission, trust, kick tests on all 4 browsers.

**New file: `e2e/multi-peer-mobile.spec.ts`** — Mobile-specific multi-peer tests (skipped on desktop projects).

**Delete: `e2e/two-peer.spec.ts`** — Consolidated into `multi-peer-sync.spec.ts`.

**Delete: `e2e/state-sync.spec.ts`** — Consolidated into `multi-peer-sync.spec.ts`.

**Modify: `e2e/helpers.ts`** — Add shared helpers for multi-peer setup, mobile-aware interactions, permission actions.

### Cross-Browser Multi-Peer Pattern

Tests use the Playwright `browser` fixture instead of hardcoded `chromium.launch()`:

```typescript
test('messages sync between peers', async ({ browser }) => {
  const ctx1 = await browser.newContext();
  const ctx2 = await browser.newContext();
  const page1 = await ctx1.newPage();
  const page2 = await ctx2.newPage();
  // test body
  await ctx1.close();
  await ctx2.close();
});
```

When Playwright runs this against the "Mobile Chrome" project, `browser` is Chromium with Pixel 7 viewport. Against "Desktop Firefox", it's Firefox with default viewport. The test body stays the same; UI interaction helpers adapt to mobile vs desktop.

**Mobile detection**: `const isMobile = (page.viewportSize()?.width ?? 1024) < 768;` — consistent with existing mobile tests. The CSS breakpoint for mobile layout should be verified during implementation to ensure it matches this threshold.

**Mobile-aware helpers** detect viewport width and use the appropriate interaction:

```typescript
async function openSidebar(page: Page) {
  const isMobile = (page.viewportSize()?.width ?? 1024) < 768;
  if (isMobile) await page.click('.mobile-nav-toggle');
}

async function closeSidebar(page: Page) {
  const isMobile = (page.viewportSize()?.width ?? 1024) < 768;
  if (isMobile) await page.click('.sidebar-overlay.open');
}
```

### Test Scenarios

#### `multi-peer-sync.spec.ts` (all 4 browsers)

| # | Test | Validates |
|---|------|-----------|
| 1 | Invite flow: create server, generate invite, join | Full handshake, both peers see server |
| 2 | Messages sync both directions in general channel | Peer1 sends → Peer2 sees, Peer2 sends → Peer1 sees |
| 3 | Pre-existing channels visible to joining peer | Peer1 creates 2 extra channels before invite, Peer2 sees all 3 after join |
| 4 | New channel created mid-session appears on both | Peer1 creates channel after join, Peer2 sees it |
| 5 | Messages in non-general channel sync both ways | Switch to new channel, exchange messages |
| 6 | Reactions sync | Peer1 reacts → Peer2 sees reaction count |
| 7 | Edits sync | Peer1 edits → Peer2 sees updated text |
| 8 | Deletes sync | Peer1 deletes → Peer2 sees deletion |
| 9 | State persists after refresh for both peers | Refresh → messages, channels still there |
| 10 | Both peers in member list | Member list shows 2 entries |
| 11 | Typing indicator shows | Peer1 types → Peer2 sees typing indicator |
| 12 | Display names shown correctly | Peer1 sets name, Peer2 sees it in messages |

#### `permissions.spec.ts` (all 4 browsers)

| # | Test | Validates |
|---|------|-----------|
| 1 | Owner can trust a peer | Trust button in member list works |
| 2 | Trusted peer can send messages | After trust, messages appear on owner's screen |
| 3 | Owner can untrust a peer | Untrust action reflected in UI |
| 4 | Untrusted peer's messages not visible | After untrust, peer2 sends, owner does NOT see the message |
| 5 | Owner can kick a member | Peer removed from owner's member list |
| 6 | Kicked peer sees welcome/disconnected state | Kicked peer returns to welcome screen or sees kicked state — cannot send messages |
| 7 | Owner can create and assign roles | Role CRUD + permission toggle + assignment works |
| 8 | Non-owner does not see trust/kick buttons | Peer2's member list has no action buttons |

#### `multi-peer-mobile.spec.ts` (mobile projects only)

| # | Test | Validates |
|---|------|-----------|
| 1 | Invite flow through mobile UI | Hamburger → settings → invite works |
| 2 | New channels visible via hamburger menu | Open sidebar, new channel appears |
| 3 | Messages arrive while sidebar closed | Messages appear in chat view |
| 4 | Member list via mobile toggle | Both peers visible |
| 5 | Channel switch on mobile during sync | Hamburger → switch → messages load |

### Helper Additions (`e2e/helpers.ts`)

**Multi-peer setup:**
- `setupTwoPeers(browser)` — creates 2 contexts (via `freshStart` on each), server, invite, join. Returns `{ ctx1, ctx2, page1, page2 }`.
- `generateInvite(page, recipientPeerId)` — opens server settings (mobile-aware), fills recipient peer ID in `input[placeholder*="12D3KooW"]`, clicks "Generate Invite", reads code from `.invite-code-display textarea`.
- `joinViaInvite(page, inviteCode, displayName?)` — fills `.welcome-invite-input`, clicks button with text "Next", optionally sets display name, clicks "Join Server".

**Mobile-aware navigation:**
- `openSidebar(page)` — clicks `.mobile-nav-toggle` on mobile, no-op on desktop.
- `closeSidebar(page)` — clicks `.sidebar-overlay.open` on mobile, no-op on desktop.
- `openMemberList(page)` — clicks `.mobile-members-toggle` on mobile, no-op on desktop (member list always visible).
- `closeMemberList(page)` — clicks `.members-overlay.open` on mobile, no-op on desktop.
- `openServerSettings(page)` — opens sidebar if needed, clicks `.server-gear-btn`.
- `createChannel(page, name)` — opens sidebar if needed, clicks `.channel-add-btn`, fills input, submits.

**Permission actions** (operate within the member list):
- `trustPeer(page, peerDisplayName)` — opens member list if needed, finds `.member-item` containing the display name, clicks `.btn` with text "Trust" inside `.member-actions`.
- `untrustPeer(page, peerDisplayName)` — same pattern, clicks "Untrust" button.
- `kickPeer(page, peerDisplayName)` — same pattern, clicks "Kick" button (`.btn.btn-sm.btn-danger`).
- `waitForPeerCount(page, count, timeout?)` — opens member list if needed, waits for `.member-item` count to equal `count`.

**Message actions (desktop vs mobile branching):**
- `messageAction(page, messageText, actionName)` — **branches on mobile vs desktop**:
  - Desktop: hovers message → clicks `.action-trigger` → clicks `.dropdown-item` with matching text.
  - Mobile: calls `longPress` on message → clicks `.sheet-item` with matching text in the action sheet.
- `editMessage(page, originalText, newText)` — calls `messageAction(page, originalText, 'Edit')`, fills the input with newText, submits.
- `deleteMessage(page, text)` — calls `messageAction(page, text, 'Delete')`.
- `reactToMessage(page, text, emoji)` — **branches on mobile vs desktop**:
  - Desktop: hovers → `.action-trigger` → "React" dropdown item → clicks emoji in `.dropdown-emoji-row`.
  - Mobile: `longPress` → clicks emoji directly in `.sheet-emoji-row`.

### Timing

Same patterns as existing tests:
- 300-500ms between UI interactions.
- 15s timeout for P2P sync waits (`waitForMessage`).
- `waitForPeerCount` with 15s default for peer discovery.

### Justfile Updates

- Update `test-e2e-sync` to run `multi-peer-sync.spec.ts` on desktop-chrome only (quick local iteration). Full cross-browser coverage via `test-e2e-ui-all`.
- Add `test-e2e-perms` for `permissions.spec.ts` on desktop-chrome.
- `test-e2e-ui-all` already runs all projects against all spec files.

### Test Count

25 new tests across 4 browser projects. `multi-peer-sync.spec.ts` (12) and `permissions.spec.ts` (8) run on all 4 = 80 executions. `multi-peer-mobile.spec.ts` (5) runs on 2 mobile projects = 10 executions. Total: 90 test executions. Replaces 10 Chrome-only tests from the retired files.
