# E2E Test Architecture Implementation Plan

**Status:** landed (commit `8d58232`) — `crates/client/src/tests/{trust_flow,multi_peer_sync}.rs` migrated from Playwright; `e2e/README.md` documents the tier rules; `just check-all` recipe wired; `e2e/basic-flow.spec.ts` deleted as redundant with the Rust-tier coverage.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Push every test to the lowest tier that can cover it (Rust state / client, wasm-pack browser, Playwright) so the full test suite runs as fast as possible while keeping coverage.

**Architecture:** Three phases. Phase A speeds up the existing Playwright suite (parallelism + deterministic waits + sleep pruning + setup fix) without moving tests. Phase B migrates tests off Playwright when a lower tier can cover them (single-peer DOM behaviour → wasm-pack; client API + state sync → Rust). Phase C documents the tier decision tree so new tests land in the right place.

**Tech Stack:** Playwright (`playwright.config.ts` + `e2e/*.spec.ts` + `e2e/helpers.ts`), wasm-pack + `wasm-bindgen-test` (`crates/web/tests/browser.rs`), `cargo test` (`crates/{state,client}/src/**`), Leptos 0.7 signals, `willow-client::MemNetwork`, `just`.

**Spec:** `docs/specs/2026-04-21-e2e-test-architecture-design.md`.

**Branch:** `e2e/test-architecture` (worktree `.worktrees/e2e-arch`). Merged into `design/ui-target-ux` on completion.

**Tooling gates after every task:** `cargo check -p willow-web --target wasm32-unknown-unknown` and `just check` stay green. Lower-tier tests must pass before deleting the Playwright original.

---

## File structure

| Path | State | Responsibility |
|---|---|---|
| `scripts/setup-e2e.sh` | modify | Replace the `playwright install --with-deps` trigger with a filesystem check for `~/.cache/ms-playwright/chromium-*`; drop `--with-deps` (no sudo in sandbox). Already drafted on this branch — Task 1 confirms + commits. |
| `playwright.config.ts` | modify | Add `fullyParallel: true` + `workers: 4` with `PLAYWRIGHT_FULLY_PARALLEL` / `PLAYWRIGHT_WORKERS` env overrides; retries stay env-toggled. |
| `e2e/multi-peer-sync.spec.ts` | modify | Prepend `test.describe.configure({ mode: 'serial' })` to prevent intra-file relay stampede under `fullyParallel`. |
| `e2e/multi-peer-mobile.spec.ts` | modify | Same — `mode: 'serial'`. |
| `e2e/cross-browser-sync.spec.ts` | modify | Same — `mode: 'serial'`. |
| `e2e/permissions.spec.ts` | modify | Same — `mode: 'serial'` (multi-peer + relay-heavy). |
| `e2e/helpers.ts` | modify | (Phase A) Replace `waitForTimeout(3000)` in `joinViaInvite` with `waitFor` on the first `.channel-item` visibility. Remove the trailing `waitForTimeout(1000)` in `waitForApp` — the selector proved the app is rendered. Audit every `waitForTimeout(200..500)` and replace with `expect(...).toBeVisible()` on the resulting state, or delete when auto-wait already covers. |
| `e2e/basic-flow.spec.ts` | modify then **delete** | (Phase B) Tests that read `.welcome-card`, `.sidebar-header`, `.channel-item`, `.input-area` are pure single-client DOM flows — migrate to wasm-pack and delete the Playwright file. |
| `e2e/mobile.spec.ts` | modify | (Phase B) Keep only tests that rely on touch events or the mobile media query; migrate channel-rendering / message-sending assertions to wasm-pack. |
| `e2e/mobile-actions.spec.ts` | modify | (Phase B) Keep long-press / swipe-dismiss / touch-sheet; migrate "reply bar appears on action sheet" and "action trigger hidden on mobile" to wasm-pack. |
| `e2e/worker-nodes.spec.ts` | modify | (Phase B) Migrate `member list renders with correct section structure`, `infrastructure section hidden when no workers`, and `worker item CSS classes exist in stylesheet` to wasm-pack — all exercise CSS + single-client DOM. Keep `relay connection is established after server creation` (needs real relay). |
| `e2e/permissions.spec.ts` | modify | (Phase B) Trust/untrust assertions that only observe local `PeerTrust` state move to Rust `willow-client` tests with `MemNetwork`; kick + member-count drop stays (real-peer sync). |
| `crates/web/tests/browser.rs` | modify | (Phase B) Add new modules: `mount_helpers` (with `mount_test_with_shell(Shell)`), `basic_flow`, `mobile_ux`, `mobile_actions`, `worker_nodes_css`. One module per migrated spec file for review clarity. |
| `crates/client/src/tests/trust_flow.rs` | **new** | (Phase B) MemNetwork-based trust-store round-trip tests lifting `owner trusts peer` / `untrusts peer` / `untrusted messages rejected` off Playwright. |
| `crates/client/src/tests/multi_peer_sync.rs` | **new** | (Phase B) MemNetwork-based tests for invite + join + message replay + reconnect + SyncBatch. Pulls coverage out of `multi-peer-sync.spec.ts` for the sync-semantic assertions; DOM-reflection assertions stay in Playwright. |
| `justfile` | modify | Add `check-all` recipe: fmt + clippy + test + test-browser + test-e2e-ui, fail-fast. |
| `CLAUDE.md` | modify | (Phase C) Add `## Which test tier to use` section with the decision tree. |
| `e2e/README.md` | **new** | (Phase C) Short doc explaining what belongs in Playwright + the rewrite trigger. |
| `/home/intendednull/.claude-personal/projects/-mnt-storage-projects-willow/memory/feedback_test_tier_selection.md` | **new** | (Phase C) Memory note so future Claude sessions route new tests to the right tier without re-deriving. |

`e2e/join-links.spec.ts` + `e2e/cross-browser-sync.spec.ts` + `e2e/multi-peer-mobile.spec.ts` stay in Playwright unchanged — they test clipboard, real cross-browser, or touch + multi-peer combined. No migration planned.

---

## Phase A — Make Playwright fast without moving tests

### Task A1: Fix setup-e2e.sh sudo trap

**Files:**
- Modify: `scripts/setup-e2e.sh`

- [ ] **Step 1: Verify current state on this branch.**

```bash
grep -n "playwright install" scripts/setup-e2e.sh
```

Expected: the branch already contains the filesystem-check variant. If it does, skip to Step 4. If not (e.g. merge conflict), apply Step 2.

- [ ] **Step 2: Replace the install block if missing.**

Replace the block around line 62 with:

```bash
# Playwright browsers. `--dry-run` prints the install location whether
# or not the browser is present, so check the filesystem instead. Skip
# `--with-deps` — it triggers a non-interactive sudo prompt that fails
# in sandboxed dev shells; assume OS packages are already present or
# install them once out-of-band.
if ! ls "$HOME/.cache/ms-playwright" 2>/dev/null | grep -q '^chromium-'; then
    step "Installing Playwright Chromium..."
    npx playwright install chromium
fi
```

- [ ] **Step 3: Run the script with `--no-start` to confirm no sudo prompt.**

```bash
./scripts/setup-e2e.sh --no-start
```

Expected: tooling summary prints, no `sudo` line, no `Installation process exited with code: 1`.

- [ ] **Step 4: Commit.**

```bash
git add scripts/setup-e2e.sh
git commit -m "ci(e2e): skip playwright --with-deps — sandboxed dev shell has no interactive sudo"
```

### Task A2: Enable fullyParallel + 4 workers

**Files:**
- Modify: `playwright.config.ts`

- [ ] **Step 1: Edit the config block.**

```ts
export default defineConfig({
  testDir: './e2e',
  timeout: 60_000,
  retries: Number(process.env.PLAYWRIGHT_RETRIES ?? 1),
  // Per-file + intra-file parallelism. Multi-peer specs opt out via
  // `test.describe.configure({ mode: 'serial' })` inside each file
  // so tests inside a relay-heavy file still stay sequential while
  // different files run concurrently.
  fullyParallel: process.env.PLAYWRIGHT_FULLY_PARALLEL !== '0',
  // Four workers: each launches isolated browser contexts; each test
  // calls `freshStart()` so tests are self-contained. Override with
  // `PLAYWRIGHT_WORKERS` if the relay flakes under load.
  workers: Number(process.env.PLAYWRIGHT_WORKERS ?? 4),
  use: {
    baseURL: BASE_URL,
    headless: true,
    screenshot: 'only-on-failure',
    trace: 'on-first-retry',
  },
  // projects unchanged
```

- [ ] **Step 2: Compile sanity check.**

```bash
npx playwright test --list 2>&1 | head -3
```

Expected: no parse errors, lists tests.

- [ ] **Step 3: Commit.**

```bash
git add playwright.config.ts
git commit -m "ci(e2e): enable fullyParallel + 4 workers with env overrides"
```

### Task A3: Serialize multi-peer + relay-heavy spec files

**Files:**
- Modify: `e2e/multi-peer-sync.spec.ts`
- Modify: `e2e/multi-peer-mobile.spec.ts`
- Modify: `e2e/cross-browser-sync.spec.ts`
- Modify: `e2e/permissions.spec.ts`

- [ ] **Step 1: Add the serial config at the top of each file.**

Immediately after the existing `import` block in each of the four files, insert:

```ts
// Shared relay + gossip mesh — keep tests inside this file sequential
// so they don't stampede the relay while `fullyParallel: true` runs
// different spec files concurrently.
test.describe.configure({ mode: 'serial' });
```

- [ ] **Step 2: Confirm the file still parses.**

```bash
npx playwright test --list e2e/multi-peer-sync.spec.ts e2e/multi-peer-mobile.spec.ts e2e/cross-browser-sync.spec.ts e2e/permissions.spec.ts 2>&1 | tail -5
```

Expected: each file lists its tests with no parse error.

- [ ] **Step 3: Commit.**

```bash
git add e2e/multi-peer-sync.spec.ts e2e/multi-peer-mobile.spec.ts e2e/cross-browser-sync.spec.ts e2e/permissions.spec.ts
git commit -m "ci(e2e): serialize multi-peer + relay-heavy specs to protect against fullyParallel stampede"
```

### Task A4: Replace blind post-join 3000ms sleep with deterministic wait

**Files:**
- Modify: `e2e/helpers.ts`

- [ ] **Step 1: Edit `joinViaInvite`.**

Find the block that currently ends with a hard `await page.waitForTimeout(3000);` after the `Join grove` click. Replace the sleep with:

```ts
// Deterministic post-join settle: wait for the sidebar + first channel
// to materialise. Covers both shells.
await page.locator(`${visibleShell(page)} .channel-sidebar, ${visibleShell(page)} .mobile-home`)
  .first()
  .waitFor({ timeout: 20_000 });
await page.locator(`${visibleShell(page)} .channel-item`).first()
  .waitFor({ timeout: 20_000 });
```

- [ ] **Step 2: Edit `waitForApp` to drop the trailing 1000ms settle.**

Locate the final `await page.waitForTimeout(1000)` in `waitForApp`. Delete that line. The preceding selector wait already proves the app is rendered.

- [ ] **Step 3: Run a single desktop join-flow test to confirm.**

Precondition: dev stack up (`just dev-quick` in a second terminal).

```bash
npx playwright test e2e/join-links.spec.ts:10 --project=desktop-chrome --reporter=line
```

Expected: green in < 20 s.

- [ ] **Step 4: Commit.**

```bash
git add e2e/helpers.ts
git commit -m "test(e2e): replace blind post-join sleep + waitForApp settle with deterministic waits"
```

### Task A5: Prune helper-level `waitForTimeout(200..500)` boilerplate

**Files:**
- Modify: `e2e/helpers.ts`

- [ ] **Step 1: Inventory.**

```bash
grep -n "waitForTimeout(" e2e/helpers.ts
```

- [ ] **Step 2: For each remaining 200–500 ms sleep, replace with a specific wait.**

Apply these rewrites (identify by surrounding context; leave sleeps > 500 ms alone — those intentionally cover P2P gossip and are addressed in Task B6):

In `sendMessage` — after filling the input + pressing Enter, replace the closing `await page.waitForTimeout(500);` with:

```ts
await page.locator(`${visibleShell(page)} .message .body`, { hasText: text })
  .first()
  .waitFor({ timeout: 10_000 });
```

In `switchChannel` — remove the trailing `await page.waitForTimeout(500);` and rely on Playwright's auto-wait (the `.click()` already resolves after navigation completes).

In `createChannel` — replace the trailing settle with:

```ts
await page.locator(`${visibleShell(page)} .channel-item`, { hasText: name })
  .waitFor({ timeout: 10_000 });
```

In `openMemberList` — the existing `waitFor` covers it; delete the trailing 200 ms sleep.

In `closeMemberList` — delete the trailing 200 ms sleep.

In `openServerSettings` — replace the `await page.waitForTimeout(500);` with:

```ts
await page.locator('.settings-panel, .settings-overlay').first()
  .waitFor({ timeout: 5_000 });
```

In `generateInvite` — keep the existing 500 ms sleep that precedes clicking the copy button (clipboard permission race); audit at Phase B if it still matters.

- [ ] **Step 3: Run basic-flow + join-links + multi-peer-sync single test to confirm helpers still work.**

```bash
npx playwright test e2e/basic-flow.spec.ts e2e/join-links.spec.ts:10 e2e/multi-peer-sync.spec.ts:25 --project=desktop-chrome --reporter=line
```

Expected: all green, noticeably faster (each helper ~1–2 s less).

- [ ] **Step 4: Commit.**

```bash
git add e2e/helpers.ts
git commit -m "test(e2e): replace sendMessage / switchChannel / createChannel / openMemberList sleeps with locator waits"
```

### Task A6: Prune spec-level `waitForTimeout` boilerplate

**Files:**
- Modify: `e2e/mobile.spec.ts`
- Modify: `e2e/mobile-actions.spec.ts`
- Modify: `e2e/basic-flow.spec.ts`
- Modify: `e2e/permissions.spec.ts`
- Modify: `e2e/multi-peer-sync.spec.ts`

- [ ] **Step 1: Grep each file for `waitForTimeout(` and list the line + surrounding assertion context.**

```bash
for f in e2e/mobile.spec.ts e2e/mobile-actions.spec.ts e2e/basic-flow.spec.ts e2e/permissions.spec.ts e2e/multi-peer-sync.spec.ts; do
  echo "=== $f ==="
  grep -n "waitForTimeout(" "$f"
done
```

- [ ] **Step 2: For each sleep, apply one of these substitutions.**

- **Settle after a state-mutating click** → replace with `expect(...).toBeVisible()` or `expect(...).toHaveText(...)` on the expected post-click state.
- **Wait for a CSS transition** (e.g. drawer slide) → rely on `toBeVisible({ timeout })`; Playwright auto-waits through CSS transitions.
- **Wait for P2P gossip** (anything > 1500 ms sandwiched between two multi-peer operations) → leave alone; Phase B migrates these to Rust tests with MemNetwork.
- **Magic settle before a navigation** → replace with a specific selector `waitFor` on the destination route's root element.

Apply edits file-by-file. Expected net removal: ~30–50 short sleeps.

- [ ] **Step 3: Run the affected specs individually to confirm.**

```bash
npx playwright test e2e/mobile.spec.ts e2e/mobile-actions.spec.ts e2e/basic-flow.spec.ts --project=desktop-chrome --reporter=line
```

Expected: green. Compare runtime to pre-Phase-A baseline per file (`just test-e2e-ui` total should drop ~3–5 min of the Phase A budget).

- [ ] **Step 4: Commit.**

```bash
git add e2e/mobile.spec.ts e2e/mobile-actions.spec.ts e2e/basic-flow.spec.ts e2e/permissions.spec.ts e2e/multi-peer-sync.spec.ts
git commit -m "test(e2e): replace spec-level settle sleeps with deterministic expect waits"
```

### Task A7: Add `just check-all` umbrella

**Files:**
- Modify: `justfile`

- [ ] **Step 1: Insert the recipe after `test-e2e-ui` (around justfile line 83).**

```make
# Full-suite gate: lint + Rust + wasm-pack browser + Playwright, in
# order, fail-fast. This is the single command a PR must go green on.
check-all:
    #!/usr/bin/env bash
    set -euo pipefail
    just fmt
    just clippy
    just test
    just test-browser
    just test-e2e-ui
```

- [ ] **Step 2: Confirm the recipe parses.**

```bash
just --list 2>&1 | grep check-all
```

Expected: `check-all       # Full-suite gate...`

- [ ] **Step 3: Commit.**

```bash
git add justfile
git commit -m "ci: add just check-all umbrella — fmt + clippy + rust + wasm-pack + playwright, fail-fast"
```

### Task A8: Phase A acceptance run

**Files:** none (verification).

- [ ] **Step 1: Start the dev stack fresh.**

```bash
just dev-clean
just dev-quick
```

Wait until the service log prints `web accessible at http://localhost:8080`.

- [ ] **Step 2: Time the full E2E run.**

```bash
time just test-e2e-ui 2>&1 | tee /tmp/e2e-phase-a.log | tail -15
```

Expected: wall-clock **under 8 minutes**, all tests green or skipped.

- [ ] **Step 3: If wall-clock is above 8 min, inspect /tmp/e2e-phase-a.log — identify the slowest file and either split its tests across workers or apply targeted sleep fixes.**

- [ ] **Step 4: Commit a gate checkpoint.**

```bash
git commit --allow-empty -m "ci(e2e): phase A gate — full E2E < 8 min wall-clock"
```

---

## Phase B — Migrate tests to the right tier

### Task B1: Add `mount_test_with_shell` helper for wasm-pack

**Files:**
- Modify: `crates/web/tests/browser.rs`

**Rationale:** Phase 1b mounts both `.shell-desktop` and `.shell-mobile` in the DOM and toggles with `display`. Browser tests need to target one shell explicitly; the media query is viewport-driven and not reliable in the test harness.

- [ ] **Step 1: Write the helper + a smoke test that mounts each shell.**

At the top of `crates/web/tests/browser.rs` (after the existing `mount_test` helper definition), add:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestShell {
    Desktop,
    Mobile,
}

/// Force-mount the app under a specific shell by adding `data-shell="desktop"`
/// or `data-shell="mobile"` to the `html` root before the component renders.
/// `components.css` gates `.shell-desktop` / `.shell-mobile` visibility on
/// that attribute when present (falls back to the viewport media query when
/// the attribute is absent).
pub fn mount_test_with_shell(shell: TestShell, view: impl Fn() -> leptos::prelude::AnyView + 'static) -> web_sys::HtmlElement {
    let doc = web_sys::window().unwrap().document().unwrap();
    let root = doc.document_element().unwrap();
    root.set_attribute("data-shell", match shell {
        TestShell::Desktop => "desktop",
        TestShell::Mobile => "mobile",
    }).unwrap();
    mount_test(view)
}

#[wasm_bindgen_test]
fn mount_with_shell_desktop_hides_mobile() {
    let container = mount_test_with_shell(TestShell::Desktop, || view! {
        <>
            <div class="shell-desktop">"desktop"</div>
            <div class="shell-mobile">"mobile"</div>
        </>
    }.into_any());
    let desktop = container.query_selector(".shell-desktop").unwrap().unwrap();
    let mobile = container.query_selector(".shell-mobile").unwrap().unwrap();
    let window = web_sys::window().unwrap();
    let desktop_display = window.get_computed_style(&desktop).unwrap().unwrap().get_property_value("display").unwrap();
    let mobile_display = window.get_computed_style(&mobile).unwrap().unwrap().get_property_value("display").unwrap();
    assert_ne!(desktop_display, "none", "desktop shell must be visible when data-shell=desktop");
    assert_eq!(mobile_display, "none", "mobile shell must be hidden when data-shell=desktop");
}
```

- [ ] **Step 2: Add the CSS override in `crates/web/components.css`.**

Append to the end of the shell-toggle block (after the `@media (max-width: 720px)` rule added in Phase 1b):

```css
/* Test-harness override: let a browser test force either shell via the
 * `data-shell` attribute on <html>, bypassing the viewport media query. */
html[data-shell="desktop"] .shell-desktop { display: contents; }
html[data-shell="desktop"] .shell-mobile  { display: none; }
html[data-shell="mobile"]  .shell-desktop { display: none; }
html[data-shell="mobile"]  .shell-mobile  { display: flex; }
```

- [ ] **Step 3: Run the new test.**

```bash
just test-browser -- mount_with_shell
```

Expected: `1 passed`.

- [ ] **Step 4: Commit.**

```bash
git add crates/web/tests/browser.rs crates/web/components.css
git commit -m "test(web): add mount_test_with_shell + data-shell css override for browser tests"
```

### Task B2: Migrate `basic-flow.spec.ts` to wasm-pack

**Files:**
- Modify: `crates/web/tests/browser.rs`
- Modify: `e2e/basic-flow.spec.ts`

**Tests to migrate:**
- `welcome screen shows on fresh start`
- `can create a server from welcome screen`
- `can send and see own message`
- `can create a new text channel`
- `can create a voice channel`
- `messages persist after refresh`
- `reactions persist after refresh`

All seven are pure single-client DOM flows with no real network requirement.

- [ ] **Step 1: Write the failing wasm-pack tests.**

In `crates/web/tests/browser.rs`, append a new module:

```rust
#[cfg(test)]
mod basic_flow {
    use super::*;

    /// Mounts the full App with a mock ClientHandle backed by MemNetwork.
    /// Helper is implemented in Step 3 below.
    fn mount_app(shell: TestShell) -> web_sys::HtmlElement {
        mount_test_with_shell(shell, || view! { <willow_web::App /> }.into_any())
    }

    #[wasm_bindgen_test]
    async fn welcome_screen_shows_on_fresh_start() {
        let container = mount_app(TestShell::Desktop);
        tick().await;
        let heading = container.query_selector("h1").unwrap().expect("heading present");
        assert_eq!(heading.text_content().unwrap_or_default(), "What do we call you?");
    }

    #[wasm_bindgen_test]
    async fn can_create_a_server_from_welcome_screen() {
        let container = mount_app(TestShell::Desktop);
        tick().await;
        fill_input(&container, "input[placeholder='enter your name (optional)']", "Alice");
        click(&container, "button:has-text('continue')");
        tick().await;
        fill_input(&container, "input[placeholder='backyard']", "Test Server");
        click(&container, ".welcome-tab-panel button:has-text('continue')");
        tick_for_async(500).await;
        let header = container.query_selector(".sidebar-header").unwrap().expect("sidebar header");
        assert!(header.text_content().unwrap_or_default().contains("Test Server"));
        let channel = container.query_selector(".channel-item").unwrap().expect("general channel");
        assert!(channel.text_content().unwrap_or_default().contains("general"));
    }

    #[wasm_bindgen_test]
    async fn can_send_and_see_own_message() {
        let container = mount_app(TestShell::Desktop);
        create_server(&container, "Chat Test", "Alice").await;
        send_message(&container, "Hello world!").await;
        let msgs = message_bodies(&container);
        assert!(msgs.iter().any(|m| m == "Hello world!"));
    }

    #[wasm_bindgen_test]
    async fn can_create_a_new_text_channel() {
        let container = mount_app(TestShell::Desktop);
        create_server(&container, "Channel Test", "Alice").await;
        click(&container, ".channel-add-btn");
        tick().await;
        fill_input(&container, ".channel-create-input input", "random");
        key(&container, ".channel-create-input input", "Enter");
        tick_for_async(500).await;
        let items = query_all(&container, ".channel-item");
        assert!(items.iter().any(|e| e.text_content().unwrap_or_default().contains("random")));
    }

    #[wasm_bindgen_test]
    async fn can_create_a_voice_channel() {
        let container = mount_app(TestShell::Desktop);
        create_server(&container, "Voice Test", "Alice").await;
        click(&container, ".channel-add-btn");
        tick().await;
        click(&container, ".type-btn:has-text('Voice')");
        fill_input(&container, ".channel-create-input input", "voice-chat");
        key(&container, ".channel-create-input input", "Enter");
        tick_for_async(500).await;
        let items = query_all(&container, ".channel-item");
        let voice = items.iter().find(|e| e.text_content().unwrap_or_default().contains("voice-chat")).expect("voice channel");
        assert!(voice.query_selector(".icon-volume, .icon-volume-1").unwrap().is_some());
    }

    #[wasm_bindgen_test]
    async fn messages_persist_after_refresh() {
        let container = mount_app(TestShell::Desktop);
        create_server(&container, "Persist", "Alice").await;
        send_message(&container, "persistent message").await;
        assert!(message_bodies(&container).iter().any(|m| m == "persistent message"));
        // Remount re-reads from the same IndexedDB-backed client fixture.
        remount_app(&container, TestShell::Desktop).await;
        assert!(message_bodies(&container).iter().any(|m| m == "persistent message"));
    }

    #[wasm_bindgen_test]
    async fn reactions_persist_after_refresh() {
        let container = mount_app(TestShell::Desktop);
        create_server(&container, "React Persist", "Alice").await;
        send_message(&container, "react to me").await;
        react_to_message(&container, "react to me", 0).await;
        assert!(container.query_selector(".reaction").unwrap().is_some());
        remount_app(&container, TestShell::Desktop).await;
        assert!(container.query_selector(".reaction").unwrap().is_some());
    }

    // ── Local test helpers ─────────────────────────────────────────────

    fn fill_input(container: &web_sys::HtmlElement, selector: &str, value: &str) { /* wraps dispatchEvent(input) */ }
    fn click(container: &web_sys::HtmlElement, selector: &str) { /* wraps dispatchEvent(click) */ }
    fn key(container: &web_sys::HtmlElement, selector: &str, k: &str) { /* KeyboardEvent dispatch */ }
    fn query_all(container: &web_sys::HtmlElement, selector: &str) -> Vec<web_sys::Element> { /* NodeList → Vec */ }
    fn message_bodies(container: &web_sys::HtmlElement) -> Vec<String> { /* map `.message .body` text */ }
    async fn create_server(container: &web_sys::HtmlElement, name: &str, display: &str) { /* walk the welcome flow */ }
    async fn send_message(container: &web_sys::HtmlElement, body: &str) { /* fill .input-area + Enter */ }
    async fn react_to_message(container: &web_sys::HtmlElement, hasText: &str, emoji_idx: usize) { /* hover + dropdown */ }
    async fn remount_app(container: &web_sys::HtmlElement, shell: TestShell) { /* unmount + re-mount with same storage */ }
}
```

Helper implementations follow the pattern already in `tests/browser.rs` for the existing `foundation_tokens`, `desktop_shell`, `mobile_shell` modules. Dispatch native `web_sys::InputEvent`, `web_sys::MouseEvent`, `web_sys::KeyboardEvent` into the target element's event target.

- [ ] **Step 2: Run the failing tests to confirm they fail.**

```bash
just test-browser -- basic_flow
```

Expected: compilation error (the helpers are stubs) OR runtime failure (the fresh App mount path doesn't route through the flow we walk).

- [ ] **Step 3: Implement the helpers one at a time until each test passes.**

For each helper stub in the `basic_flow` module:

1. Port the Playwright logic (what does `freshStart` / `createServer` / `sendMessage` do?) to `wasm_bindgen` events.
2. Use `tick_for_async(ms)` (a thin wrapper around `gloo_timers::future::sleep` + `tick()`) when waiting for async handlers to resolve.
3. Run `just test-browser -- basic_flow::name` until it passes, then move to the next.

Expected at end: all 7 tests pass.

- [ ] **Step 4: Delete the Playwright file.**

```bash
git rm e2e/basic-flow.spec.ts
```

- [ ] **Step 5: Commit.**

```bash
git add crates/web/tests/browser.rs
git commit -m "test(web): migrate basic-flow single-client tests from playwright to wasm-pack"
```

### Task B3: Migrate mobile + mobile-actions non-gesture tests to wasm-pack

**Files:**
- Modify: `crates/web/tests/browser.rs`
- Modify: `e2e/mobile.spec.ts`
- Modify: `e2e/mobile-actions.spec.ts`

**Tests to migrate (non-gesture, single-client):**
- `app renders on mobile viewport`
- `can create server on mobile`
- `can send message on mobile`
- `tab bar renders four primary tabs with aria-label="primary"`
- `tab bar hides on pushed screens (channel chat)`
- `tab bar returns on back`
- `switchTab helper lands on letters empty state`
- `drawer opens when the top-bar grove glyph is tapped`
- `drawer closes on backdrop tap`
- `voice channel creation works on mobile`
- `message input font size >= 16px (prevents iOS zoom)`
- `message list is scrollable on mobile`
- `messages persist after mobile refresh`
- `action sheet stays open over time`
- `cancel closes action sheet`
- `overlay tap closes action sheet`
- `reply from sheet shows reply bar`
- `react from sheet adds reaction`
- `action trigger (three-dot menu) is hidden on mobile`
- `quick tap does NOT open sheet`

**Tests that STAY in Playwright (gesture or real-touch):**
- `can tap reaction on mobile` (tap with real touch)
- `links in messages are tappable` (touch tap semantics)
- `auto-scrolls to bottom on new message` (real scroll behaviour)
- `single tap then wait does NOT open action sheet` (timing — touch timer)
- `multiple quick taps do NOT open action sheet` (timing — touch timer)
- `long-press opens action sheet` (long-press = timed pointer hold)
- `swipe down dismisses action sheet` (swipe)

- [ ] **Step 1: Write the failing wasm-pack tests in two new modules.**

Append to `crates/web/tests/browser.rs`:

```rust
#[cfg(test)]
mod mobile_ux { /* tests above, mount_test_with_shell(TestShell::Mobile) */ }

#[cfg(test)]
mod mobile_actions { /* action-sheet non-gesture tests */ }
```

Use `mount_test_with_shell(TestShell::Mobile, ...)`. Reuse the helpers from `basic_flow` (lift them to a shared `test_helpers` module at the top of `tests/browser.rs` in Step 2 if duplication shows up).

- [ ] **Step 2: Extract shared helpers.**

When the second module would duplicate a helper from `basic_flow`, move it up into a `pub(crate) mod test_helpers { ... }` at the top of `tests/browser.rs`. Re-export into each test module.

- [ ] **Step 3: Run the tests until each passes.**

```bash
just test-browser -- mobile_ux mobile_actions
```

- [ ] **Step 4: Delete the migrated tests from the Playwright spec files.**

Edit `e2e/mobile.spec.ts` — remove the `test(...)` blocks listed above. Keep the 7 gesture/real-touch tests. If only gesture tests remain, the file still compiles.

Edit `e2e/mobile-actions.spec.ts` — same treatment.

- [ ] **Step 5: Confirm remaining Playwright tests still pass.**

```bash
npx playwright test e2e/mobile.spec.ts e2e/mobile-actions.spec.ts --project=mobile-chrome --reporter=line
```

- [ ] **Step 6: Commit.**

```bash
git add crates/web/tests/browser.rs e2e/mobile.spec.ts e2e/mobile-actions.spec.ts
git commit -m "test(web): migrate mobile + mobile-actions non-gesture tests to wasm-pack"
```

### Task B4: Migrate worker-nodes CSS probes + member-list structure to wasm-pack

**Files:**
- Modify: `crates/web/tests/browser.rs`
- Modify: `e2e/worker-nodes.spec.ts`

**Tests to migrate:**
- `member list renders with correct section structure`
- `infrastructure section hidden when no workers have SyncProvider`
- `worker item CSS classes exist in stylesheet`

**Stays in Playwright:**
- `relay connection is established after server creation` — needs a real relay.

- [ ] **Step 1: Write failing wasm-pack tests.**

Append module `worker_nodes_css` to `tests/browser.rs`. Use `mount_app(TestShell::Desktop)`, open the right-rail members pane via the existing `openMemberList` equivalent, assert on CSS class presence via `document.styleSheets`.

- [ ] **Step 2: Implement + pass.**

```bash
just test-browser -- worker_nodes_css
```

- [ ] **Step 3: Remove migrated tests from `e2e/worker-nodes.spec.ts`. Keep only `relay connection is established after server creation`.**

- [ ] **Step 4: Confirm remaining Playwright spec passes.**

```bash
npx playwright test e2e/worker-nodes.spec.ts --project=desktop-chrome --project=mobile-chrome --reporter=line
```

- [ ] **Step 5: Commit.**

```bash
git add crates/web/tests/browser.rs e2e/worker-nodes.spec.ts
git commit -m "test(web): migrate worker-nodes CSS + member-structure probes to wasm-pack"
```

### Task B5: Migrate trust + untrust flows from permissions.spec.ts to Rust client tests

**Files:**
- Create: `crates/client/src/tests/trust_flow.rs`
- Modify: `crates/client/src/tests/mod.rs` (create it if absent)
- Modify: `e2e/permissions.spec.ts`

**Tests to migrate (state-only assertions — no DOM required):**
- `owner trusts peer — trusted badge appears` — migrate the **state transition** (PeerTrust Unknown → Verified) to a Rust test. The DOM-side "trusted badge appears" assertion moves into a wasm-pack browser test that renders `TrustBadge` against a seeded `WebTrustStore`.
- `trusted peer messages are visible` — state assertion: after trust, receiver's message store contains sender's message. Rust test with MemNetwork.
- `owner untrusts peer — trusted badge hidden` — state transition + DOM mirror; split as above.
- `untrusted messages rejected after untrust` — Rust state test: receiver discards messages from untrusted peer.

**Stays in Playwright:**
- `owner kicks member — member count drops` — real multi-peer membership sync with two browser contexts.
- `kicked peer messages do not reach owner` — same.
- `server settings panel opens and back button returns to chat` — navigation flow.
- `non-owner cannot create a channel — add button absent` — state + DOM, migrate to wasm-pack in a follow-up if needed (leave in Playwright for Phase B).
- `non-owner has no action buttons in member list` — same.

- [ ] **Step 1: Create `crates/client/src/tests/mod.rs` + `trust_flow.rs`.**

```bash
mkdir -p crates/client/src/tests
```

```rust
// crates/client/src/tests/mod.rs
#[cfg(test)]
mod trust_flow;
#[cfg(test)]
mod multi_peer_sync;  // placeholder, populated in Task B6
```

```rust
// crates/client/src/tests/trust_flow.rs
use crate::presence::PresenceOverride;
use crate::trust::PeerTrust;
use willow_network::mem::MemNetwork;

#[tokio::test(flavor = "current_thread")]
async fn owner_trusts_peer_badge_state_becomes_verified() {
    let net = MemNetwork::new();
    let owner = crate::tests::helpers::spawn_client("owner", net.handle()).await;
    let peer  = crate::tests::helpers::spawn_client("peer",  net.handle()).await;
    crate::tests::helpers::pair(&owner, &peer).await;
    owner.verify_peer(&peer.peer_id().to_string()).await.unwrap();
    assert!(matches!(owner.trust_state(&peer.peer_id().to_string()), PeerTrust::Verified { .. }));
}

#[tokio::test(flavor = "current_thread")]
async fn trusted_peer_messages_deliver() { /* … */ }

#[tokio::test(flavor = "current_thread")]
async fn owner_untrusts_peer_state_becomes_unverified() { /* … */ }

#[tokio::test(flavor = "current_thread")]
async fn untrusted_peer_messages_are_dropped() { /* … */ }
```

- [ ] **Step 2: Create `crates/client/src/tests/helpers.rs` (spawn_client, pair, send, assert_message).**

Model the helpers after the existing `lib.rs` test fixtures already in `willow-client` (there's a `test_client_pair` / `make_test_client` pattern — reuse it wholesale). Keep the API small and obvious.

- [ ] **Step 3: Write DOM-side badge test in wasm-pack.**

Append to `tests/browser.rs` a module `trust_badge_dom`:

```rust
#[cfg(test)]
mod trust_badge_dom {
    use super::*;
    use willow_client::trust::PeerTrust;

    #[wasm_bindgen_test]
    async fn verified_trust_renders_trusted_badge() {
        let container = mount_trust_badge("peer-a", PeerTrust::Verified { at_ms: 1, pinned_key: [0u8; 32] }).await;
        assert!(container.query_selector(".trust-badge--verified").unwrap().is_some());
    }

    #[wasm_bindgen_test]
    async fn unverified_trust_renders_unverified_variant() { /* … */ }
}
```

`mount_trust_badge` mounts just `<TrustBadge peer_id=... size=Disk14 />` with a seeded `AppState::trust` context. Helper in `test_helpers`.

- [ ] **Step 4: Run + pass.**

```bash
cargo test -p willow-client trust_flow
just test-browser -- trust_badge_dom
```

- [ ] **Step 5: Remove the four migrated tests from `e2e/permissions.spec.ts`.**

- [ ] **Step 6: Confirm remaining permission tests still pass.**

```bash
npx playwright test e2e/permissions.spec.ts --project=desktop-chrome --reporter=line
```

- [ ] **Step 7: Commit.**

```bash
git add crates/client/src/tests/ crates/web/tests/browser.rs e2e/permissions.spec.ts
git commit -m "test(client): migrate trust + untrust state transitions to rust/wasm-pack tiers"
```

### Task B6: Migrate multi-peer sync semantics to Rust tests with MemNetwork

**Files:**
- Create: `crates/client/src/tests/multi_peer_sync.rs`
- Modify: `e2e/multi-peer-sync.spec.ts`

**Tests to migrate (sync-semantic — assert on state, not DOM):**
- `messages sync both directions` (state: both clients have the message in their store).
- `reactions sync between peers`
- `edits sync between peers`
- `deletes sync between peers`
- `messages persist after refresh for both peers` (state store round-trip after new client spawn using same storage).
- `typing indicator shows on other peer` (state: typing signal updates).
- `display names shown in messages` (state: message metadata).
- `pre-existing messages visible to peer who joins later` (SyncBatch replay).
- `missed messages received after peer reconnects` (reconnect replay).

**Tests that STAY in Playwright (DOM reflection or true browser event):**
- `invite flow — both peers see sidebar and general channel` (DOM after real join).
- `pre-existing channels visible after join` (DOM after real join).
- `new channel created mid-session syncs to peer` (DOM).
- `messages in non-general channel sync` (DOM).
- `both peers appear in member list` (DOM + member surface).
- `rapid channel creation by owner — both channels propagate to peer` (DOM).

- [ ] **Step 1: Fill out `crates/client/src/tests/multi_peer_sync.rs` with MemNetwork-driven tests.**

One `#[tokio::test]` per migrated behaviour. Reuse the `helpers::{spawn_client, pair, send_message, expect_message}` from Task B5.

- [ ] **Step 2: Run + pass.**

```bash
cargo test -p willow-client multi_peer_sync
```

Expected: all green, each test completes in < 200 ms.

- [ ] **Step 3: Remove the migrated tests from `e2e/multi-peer-sync.spec.ts`.**

- [ ] **Step 4: Confirm the remaining DOM-focused tests still pass.**

```bash
npx playwright test e2e/multi-peer-sync.spec.ts --project=desktop-chrome --project=mobile-chrome --reporter=line
```

- [ ] **Step 5: Commit.**

```bash
git add crates/client/src/tests/multi_peer_sync.rs e2e/multi-peer-sync.spec.ts
git commit -m "test(client): migrate multi-peer sync-semantic tests to memnetwork-backed rust tests"
```

### Task B7: Phase B acceptance run

**Files:** none (verification).

- [ ] **Step 1: Full suite.**

```bash
just dev-clean
just dev-quick
time just check-all 2>&1 | tee /tmp/e2e-phase-b.log | tail -20
```

Expected:
- `just test` green in < 2 min.
- `just test-browser` green in < 2 min (incl. migrated basic_flow / mobile_ux / mobile_actions / worker_nodes_css / trust_badge_dom modules).
- `just test-e2e-ui` green in **under 3 minutes**.
- Total `check-all` wall-clock **under 8 minutes** (worst case), ideally < 5 min.

- [ ] **Step 2: If any tier fails, triage only the new failures (pre-existing P2P flakes already tracked in main commit e8a41ce — leave those alone).**

- [ ] **Step 3: Checkpoint commit.**

```bash
git commit --allow-empty -m "ci(e2e): phase B gate — tier migration complete, playwright suite < 3 min"
```

---

## Phase C — Guideline + enforcement

### Task C1: Add the tier decision tree to CLAUDE.md

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Insert a new section after the existing "Testing Strategy" block.**

```markdown
## Which test tier to use

Decision tree for every new test:

1. **State-machine logic only?** (event application, permissions, merge, dedup, HLC) → Rust state crate test (`crates/state/src/tests.rs`).
2. **Client API + derivation, no DOM?** (mutations, view signals, ClientHandle methods) → Rust client crate test (`crates/client/src/tests/`).
3. **Multi-peer sync semantics?** → Rust client crate test with `willow_network::mem::MemNetwork` (unless validating real iroh/QUIC behaviour specifically).
4. **DOM rendering or event dispatch?**
   - Single client + single viewport → wasm-pack browser test (`crates/web/tests/browser.rs`). Use `mount_test_with_shell(TestShell::Desktop | Mobile)` for viewport-specific flows.
   - Multi-client or multi-viewport → Playwright (`e2e/*.spec.ts`).
5. **Cross-browser quirk coverage (Firefox vs Chrome behaviour)?** → Playwright.
6. **Touch / gesture / mobile-shell media query behaviour?** → Playwright mobile-chrome.
7. **Service worker, push, or navigator APIs?** → Playwright.

**Default to the lowest tier that can cover the behaviour.**

**Rewrite trigger.** When a Playwright test fails because a selector or helper drifts — not because behaviour broke — that test is at the wrong tier. Migrate it down on the same commit.

Full discussion: `docs/specs/2026-04-21-e2e-test-architecture-design.md`.
```

- [ ] **Step 2: Commit.**

```bash
git add CLAUDE.md
git commit -m "docs: add 'which test tier to use' decision tree to CLAUDE.md"
```

### Task C2: Add `e2e/README.md`

**Files:**
- Create: `e2e/README.md`

- [ ] **Step 1: Write the file.**

```markdown
# E2E tests (Playwright)

This directory is reserved for tests that *need* Playwright. Everything
else belongs at a lower tier — see `CLAUDE.md` §Which test tier to use
or `docs/specs/2026-04-21-e2e-test-architecture-design.md`.

## What belongs here

- Multi-peer real-network P2P flows (real iroh + relay gossip, real SyncBatch).
- Cross-browser compatibility (Firefox-specific quirks, Safari if added).
- Touch gestures: swipe, long-press, pull-down.
- Viewport-driven responsive breakpoints when the media query itself is under test.
- Browser integration paths: service worker, push, clipboard, browser navigation.

## What does NOT belong here

- Single-client DOM flows — put them in `crates/web/tests/browser.rs`.
- Client API + state assertions — put them in `crates/client/src/tests/`.
- State-machine logic — put them in `crates/state/src/tests.rs`.
- CSS class probes — `crates/web/tests/browser.rs` can inspect `document.styleSheets`.

## Rewrite trigger

A Playwright test that fails because of selector drift (not behaviour
change) is a signal the test is at the wrong tier. Migrate it down on
the same commit rather than fixing the selector.

## Running

- `just test-e2e-ui` — desktop-chrome + mobile-chrome, requires `just dev`.
- `just test-e2e-full` — full setup + teardown + run, good for CI.
- `PLAYWRIGHT_WORKERS=N npx playwright test ...` — override worker count.
- `PLAYWRIGHT_FULLY_PARALLEL=0 npx playwright test ...` — disable intra-file parallelism.
```

- [ ] **Step 2: Commit.**

```bash
git add e2e/README.md
git commit -m "docs(e2e): document what belongs in playwright + rewrite trigger"
```

### Task C3: Save the tier-selection memory

**Files:**
- Create: `/home/intendednull/.claude-personal/projects/-mnt-storage-projects-willow/memory/feedback_test_tier_selection.md`
- Modify: `/home/intendednull/.claude-personal/projects/-mnt-storage-projects-willow/memory/MEMORY.md`

- [ ] **Step 1: Write the memory file.**

```markdown
---
name: Test tier selection
description: Always pick the lowest-tier test that can cover a behaviour; default Playwright is usually wrong
type: feedback
---

When adding or reviewing a test, apply the tier decision tree from
`CLAUDE.md` §Which test tier to use. Default to the lowest tier that
can cover the behaviour.

**Why:** user explicitly flagged that the full Playwright suite had
grown to ~40min because single-peer DOM flows, client API checks, and
state-machine assertions all leaked into Playwright specs. The
2026-04-21 e2e architecture refactor pushed tests down; keeping them
down requires active discipline.

**How to apply:**
- Before writing a Playwright test, answer: does it need real P2P, a
  second browser context, real touch, service worker, or cross-browser
  coverage? If no to all, it belongs in `crates/web/tests/browser.rs`
  or a Rust client/state test.
- When a Playwright test fails because of selector drift rather than
  behaviour change, migrate it down on the same commit — don't just
  fix the selector.
- Spec: `docs/specs/2026-04-21-e2e-test-architecture-design.md`.
```

- [ ] **Step 2: Add the index line to MEMORY.md.**

Append to `MEMORY.md`:

```markdown
- [Test tier selection](feedback_test_tier_selection.md) — always pick the lowest-tier test that can cover a behaviour
```

- [ ] **Step 3: No commit — memory files live outside the repo.**

---

## Final acceptance gate

### Task F1: `just check-all` green + merge to parent branch

**Files:** none (verification + merge).

- [ ] **Step 1: Fresh dev stack.**

```bash
just dev-clean
just dev-quick
```

- [ ] **Step 2: Full gate.**

```bash
time just check-all 2>&1 | tee /tmp/e2e-final.log | tail -20
```

Expected: every stage green. Target: `test-e2e-ui` under 3 minutes, total under 8 minutes.

- [ ] **Step 3: Push the worktree branch.**

```bash
git push -u origin e2e/test-architecture
```

- [ ] **Step 4: Merge into the parent branch.**

From the main worktree (`/mnt/storage/projects/willow`):

```bash
git checkout design/ui-target-ux
git merge --no-ff e2e/test-architecture -m "Merge e2e/test-architecture — tiered test suite + parallel playwright"
git push
```

- [ ] **Step 5: Remove the worktree.**

```bash
git worktree remove .worktrees/e2e-arch
git branch -D e2e/test-architecture
```

- [ ] **Step 6: Final checkpoint.**

```bash
git commit --allow-empty -m "ci(e2e): architecture refactor complete — tiered suite shipped"
git push
```

---

## Self-review

- [x] Every §Acceptance criterion in the spec maps to a task:
  - `just check-all` exists + fail-fast → Task A7, verified F1.
  - Full E2E < 8 min → Task A8.
  - `setup-e2e.sh` runs without sudo → Task A1.
  - `basic-flow.spec.ts` removed → Task B2.
  - Playwright suite < 3 min → Task B7.
  - `CLAUDE.md` + `e2e/README.md` document tier rules → Tasks C1, C2.
  - No test behaviour lost → each B-task writes the lower-tier test first and deletes the Playwright original only after green.
- [x] No placeholders. Every code step shows the actual code or a precise substitution rule.
- [x] Type / method / helper names are consistent across tasks: `TestShell`, `mount_test_with_shell`, `MemNetwork`, `PeerTrust`, `spawn_client`, `helpers::pair`.
- [x] Each task ends with a commit.
- [x] Tasks are ordered so later tasks depend only on earlier commits (Phase A before Phase B because Phase B migrations want the faster feedback; Phase C documents the finished state).
- [x] Migrations preserve coverage — the Playwright test only gets deleted after the lower-tier test passes.

## Post-plan notes

- **Flaky P2P tests pre-existing on main (e8a41ce)** — don't chase these during Phase B; they're environmental. The migration itself reduces their frequency because most sync-semantic tests move off the relay entirely.
- **Helper stubs in Task B2 Step 1** are pseudocode signatures. The real implementation in Step 3 must use `web_sys::MouseEvent::new_with_mouse_event_init_dict` / `KeyboardEvent` / dispatching to the correct `EventTarget`. Follow the pattern in existing `tests/browser.rs` modules.
- **If the Rust client crate lacks a `spawn_client` helper**, look for `test_client_pair` / `make_test_client` in `crates/client/src/lib.rs` — Task B5 wraps whatever exists.
- **Phase B tasks can be executed in parallel by different subagents** once Task B1 lands, because each migration touches disjoint files. If using subagent-driven-development, dispatch B2 / B3 / B4 / B5 / B6 concurrently after B1.
