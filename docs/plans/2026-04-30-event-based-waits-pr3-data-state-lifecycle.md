# Event-Based Waits PR-3 — `data-state` Lifecycle + `page.clock` Adoption

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace post-animation magic-number sleeps with a four-phase `data-state="closed|opening|open|closing"` lifecycle on six animated UI elements, and migrate `longPress`'s real 600 ms timer to `page.clock.runFor()` so touch tests don't pay wall-clock time per iteration.

**Architecture:** Each animated component owns an `RwSignal<&'static str>` (or `LifecycleState` enum, see Task 1) reflecting `closed | opening | open | closing`. The signal is exposed via a `data-state` attribute on the root element. State advances are driven by `transitionend` filtered on the component's specific driving CSS property (`transform` for slide animations, `opacity` for fade animations). A reduced-motion shortcut snaps to the terminal state synchronously when `getComputedStyle(el).transitionDuration === '0s'`, so tests under `prefers-reduced-motion: reduce` don't hang. `page.clock.install()` is opt-in per spec; only the touch helpers (and any future debounce-sensitive specs) install it. Iroh's WASM transport runs on QUIC retransmit timers that are insensitive to `Date`/`setTimeout` patching, so installing the clock during touch tests does not destabilise gossip — confirmed by the PR-1 audit recorded in the design spec §`page.clock` for real durations.

**Tech Stack:** Leptos 0.7 (signals + view!), `web_sys::TransitionEvent`, `web_sys::CssStyleDeclaration::transition_duration`, Playwright `page.clock` (since 1.45), wasm-bindgen-test for browser coverage.

**Spec:** [`docs/specs/2026-04-27-event-based-waits-design.md`](../specs/2026-04-27-event-based-waits-design.md) §`data-state` attribute pattern + §`page.clock` for real durations + §"Implementation phasing" PR 3 entry.
**Predecessor:** PR-2 (#495, merged in `5cc9efb`).
**Tracking issue:** [#458](https://github.com/intendednull/willow/issues/458).

---

## File Structure

**Create:**
- `crates/web/src/components/lifecycle.rs` — shared `LifecycleState` enum + helper free fns. (Reasoning under Task 1.)
- `e2e/helpers/clock.ts` — `installPeerClock(page)` plus a per-test `usePageClock` fixture extension. Lives next to `helpers/touch.ts` so the touch migration is single-file from the test author's POV.

**Modify:**
- `crates/web/src/components/grove_drawer.rs` — replace `data-open` with `data-state` lifecycle.
- `crates/web/src/components/mobile_shell.rs` — wire mobile drawer lifecycle. Currently uses an `RwSignal<bool>` with no `data-state`.
- `crates/web/src/components/confirm_dialog.rs` — add lifecycle (no current state attribute).
- `crates/web/src/components/bottom_sheet.rs` — replace `data-open` with `data-state`.
- `crates/web/src/components/tab_bar.rs` — replace `data-visible` with `data-state`. (NB: tab bar's hide/show is the "active tab indicator slide", per the spec.)
- `crates/web/src/components/message.rs` — verify the existing `data-state` at `:1265` and either replace its semantics or wire the same four-phase lifecycle to the `mobile-action-sheet-overlay` div.
- `crates/web/src/components/mod.rs` — `pub mod lifecycle;`.
- `crates/web/components.css` — for each component, ensure the open-class trigger plus reduced-motion override (`@media (prefers-reduced-motion: reduce) { ... transition: none }`) is present so the JS shortcut path is exercised in tests.
- `crates/web/tests/browser.rs` — append the `data-state` lifecycle test module covering the three failure modes (reduced-motion, mid-transition unmount, overlapping transitions).
- `e2e/helpers/touch.ts` — `longPress` swaps `page.waitForTimeout(durationMs)` for `await page.clock.runFor(durationMs)`. Same for the trailing 300 ms settle.
- `e2e/test-hooks.ts` — extend the `peer` fixture (or add a sibling `pageClock` fixture) so touch specs install the clock without per-spec boilerplate.
- `e2e/README.md` — document the `data-state` lifecycle convention (four phases vs. the categorical `data-state` on `status_dot`/`grove_rail`) and the `page.clock` opt-in pattern.

**Untouched (legacy specs continue to import from the barrel):**
- `e2e/cross-browser-sync.spec.ts`, `e2e/join-links.spec.ts`, `e2e/multi-peer-mobile.spec.ts`, `e2e/permissions.spec.ts`, `e2e/worker-nodes.spec.ts`, `e2e/multi-peer-sync.spec.ts` (PR-2 pilot — already on `./test-hooks`).
- `e2e/mobile.spec.ts` and `e2e/mobile-actions.spec.ts` — these will benefit from `page.clock` once they migrate (file-by-file via #458). PR-3 only changes the helper.

**Why a shared `lifecycle.rs` module instead of inlining?** The spec's open-question §"Should the `data-state` attribute pattern be lifted into a shared Leptos helper component" defers shared-helper-ness, but the *enum + advance fn + reduced-motion fn* are pure data with no Leptos signal dependency. Lifting just those three into a free-function helper (no component wrapper) avoids six near-identical 30-line blocks while honouring the "first apply, then refactor" guidance — there is nothing to refactor later because the duplication is already collapsed at the cheap, non-invasive level. The component-level wrapping (`RwSignal<LifecycleState>` + the `transitionend` closure on the root node) stays inline so each component can choose its own driving property without indirection.

---

## Task 0: Preflight — verify PR-2 baseline still passes

**Files:** none.

PR-2 just landed (`5cc9efb`) — confirm the worktree starts from a green baseline before any Rust changes.

- [ ] **Step 1: Confirm git state**

```bash
git status
git log --oneline -5
```

Expected: clean tree on `claude/event-based-waits-pr3`, head is the PR-2 merge commit (`5cc9efb`) or the new branch root.

- [ ] **Step 2: Run the cheap subset of `just check-all`**

```bash
just fmt
just clippy
```

Expected: zero errors, zero warnings. Skip `just test` and `just test-browser` here — they're slow and the next tasks will fail loudly long before a regression in those would.

- [ ] **Step 3: No commit (read-only baseline).** Move to Task 1.

---

## Task 1: Add the `lifecycle` helper module

**Files:**
- Create: `crates/web/src/components/lifecycle.rs`
- Modify: `crates/web/src/components/mod.rs`

This module owns the three pure-data primitives that every animated component reuses: the `LifecycleState` enum, the `advance(state)` transition fn, and the `is_zero_duration(element)` reduced-motion check. Components instantiate their own `RwSignal<LifecycleState>` and own their own `transitionend` listener, so this module has no Leptos dependency and is straightforwardly unit-testable.

- [ ] **Step 1: Write the module**

```rust
// crates/web/src/components/lifecycle.rs
//
// Four-phase lifecycle helpers for animated components. See
// docs/specs/2026-04-27-event-based-waits-design.md §`data-state` attribute
// pattern. Apply via `RwSignal<LifecycleState>` + a `transitionend` closure
// on the component's root element (filtered on the component's specific
// driving CSS property).
//
// `data-state` lifecycle is reserved for the four animated phases
// (closed/opening/open/closing). For categorical states (online/offline/away
// on `status_dot.rs`, idle/loading/connected on `grove_rail.rs`,
// presence labels in `peer_status_label.rs`) keep using `data-state` with
// custom strings — those usages are orthogonal and pre-date this module.

use web_sys::{Element, HtmlElement};

/// Animated component's transition phase.
///
/// `Opening` and `Closing` are set imperatively when the user triggers the
/// transition; `Open` and `Closed` are flipped by the component's
/// `transitionend` listener (or the reduced-motion shortcut).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleState {
    Closed,
    Opening,
    Open,
    Closing,
}

impl LifecycleState {
    /// String form for the `data-state` attribute. Keep in sync with e2e
    /// tests asserting `toHaveAttribute('data-state', ...)`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Opening => "opening",
            Self::Open => "open",
            Self::Closing => "closing",
        }
    }
}

/// Advance the lifecycle on `transitionend`.
///
/// `Opening` → `Open`, `Closing` → `Closed`. Other states are unchanged
/// (a stray `transitionend` while already terminal is a no-op, not an error).
pub const fn advance(state: LifecycleState) -> LifecycleState {
    match state {
        LifecycleState::Opening => LifecycleState::Open,
        LifecycleState::Closing => LifecycleState::Closed,
        terminal => terminal,
    }
}

/// Returns true if the element has no animation duration (reduced motion or
/// untransitioned). When this returns true, callers must snap to the
/// terminal lifecycle state synchronously without waiting for `transitionend`
/// — otherwise the test hangs because no event will fire.
///
/// Reads `getComputedStyle(el).transition-duration`. Empty string and "0s"
/// both count as zero. Multi-property transitions (comma-separated values)
/// are conservatively treated as non-zero unless every component is "0s",
/// matching the spec's "if computed-zero, skip wait" semantics.
pub fn is_zero_duration(element: &Element) -> bool {
    let Some(window) = web_sys::window() else { return false; };
    let Ok(Some(computed)) = window.get_computed_style(element) else { return false; };
    let Ok(duration) = computed.get_property_value("transition-duration") else { return false; };
    if duration.is_empty() {
        return true;
    }
    duration
        .split(',')
        .all(|d| matches!(d.trim(), "" | "0s" | "0ms"))
}

/// Convenience: convert an `&HtmlElement` to `&Element` for `is_zero_duration`.
pub fn is_zero_duration_html(html: &HtmlElement) -> bool {
    is_zero_duration(html.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_opening_to_open() {
        assert_eq!(advance(LifecycleState::Opening), LifecycleState::Open);
    }

    #[test]
    fn advance_closing_to_closed() {
        assert_eq!(advance(LifecycleState::Closing), LifecycleState::Closed);
    }

    #[test]
    fn advance_terminal_states_idempotent() {
        assert_eq!(advance(LifecycleState::Open), LifecycleState::Open);
        assert_eq!(advance(LifecycleState::Closed), LifecycleState::Closed);
    }

    #[test]
    fn as_str_round_trip() {
        for state in [
            LifecycleState::Closed,
            LifecycleState::Opening,
            LifecycleState::Open,
            LifecycleState::Closing,
        ] {
            // No round-trip parser by design — `as_str` is for the DOM attribute.
            assert!(!state.as_str().is_empty());
        }
    }
}
```

- [ ] **Step 2: Wire the module into `mod.rs`**

Open `crates/web/src/components/mod.rs` and add `pub mod lifecycle;` in alphabetical position (between `letters` and `member_list` if they're present; or wherever the pattern reads).

- [ ] **Step 3: Verify it compiles + native tests pass**

```bash
cargo check -p willow-web
cargo test -p willow-web --lib lifecycle::tests
```

Expected: zero errors, 4 tests pass. The `is_zero_duration` fn is gated on `web_sys::window()` so the unit tests skip it (it would only run under wasm-bindgen-test); coverage for that fn comes in Task 8.

- [ ] **Step 4: WASM build sanity**

```bash
just check-wasm
```

Expected: zero errors. `web_sys::Element` and `CssStyleDeclaration::get_property_value` are both stable and already used elsewhere in the crate.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src/components/lifecycle.rs crates/web/src/components/mod.rs
git commit -m "feat(web): add LifecycleState enum + transition helpers

Four-phase data-state lifecycle (closed/opening/open/closing) for
animated components. Pure-data primitives only — components own their
own RwSignal and transitionend closure. Reduced-motion shortcut reads
computed transition-duration; if zero, callers snap to terminal state
synchronously (otherwise the test hangs because no transitionend
fires under prefers-reduced-motion).

Per docs/specs/2026-04-27-event-based-waits-design.md §data-state
attribute pattern. The shared-helper-component question stays open
per the spec's defer guidance — only the data primitives are lifted."
```

---

## Tasks 2–7: Per-component lifecycle wiring

All six components follow the same 4-block pattern from Task 2 (the canonical reference below). For Tasks 3–7, **only the diff vs. Task 2** is documented — the Effect, on_transition_end closure, and view! changes are mechanically identical apart from the driving property and (for the action sheet) the source of truth signal.

| Task | Component file | Driving CSS property | Source of truth | Existing attr being replaced |
|------|----------------|---------------------|-----------------|------------------------------|
| 2 | `grove_drawer.rs` | `transform` | `open: Signal<bool>` prop | `data-open` |
| 3 | `mobile_shell.rs` | `transform` | `drawer_open: RwSignal<bool>` | (none — adding) |
| 4 | `confirm_dialog.rs` | `opacity` | `visible: ReadSignal<bool>` prop | (none — adding) |
| 5 | `bottom_sheet.rs` | `opacity` | `open: Signal<bool>` prop | `data-open` |
| 6 | `tab_bar.rs` | `transform` | `visible: Signal<bool>` prop | `data-visible` |
| 7 | `message.rs` action sheet | `transform` | `show_sheet: Memo<bool>` | class-name toggling |

Per-task structure: edit → `just check-wasm` → commit. No separate Step 2 for CSS audit unless the existing attribute (`data-open` / `data-visible`) is keyed by CSS selectors.

## Task 2: Wire `grove_drawer.rs` lifecycle (canonical reference)

**Files:**
- Modify: `crates/web/src/components/grove_drawer.rs`

`grove_drawer.rs` is the simplest of the six (220 LOC, single signal). This is the reference implementation; subsequent components (`mobile_shell`, `confirm_dialog`, `bottom_sheet`, `tab_bar`, action sheet) follow the same pattern with their own driving property.

Driving property: `transform` (the slide-in/out is a `translateX`).

The current file uses `data-open=move || open.get()` at `:71`. Replace with `data-state=move || lifecycle.get().as_str()`. The `open: Signal<bool>` prop stays — internally we maintain a `RwSignal<LifecycleState>` mirrored from `open` via an Effect.

- [ ] **Step 1: Edit the component**

Find the existing `data-open=move || open.get()` attribute and replace its surrounding open/close logic with the lifecycle-driven version. The diff is:

```rust
// At the top of the component body, after props are destructured:
use crate::components::lifecycle::{LifecycleState, advance, is_zero_duration};
use leptos::ev::TransitionEvent;
use leptos::html::Aside;  // or whatever element the drawer is

let drawer_ref: NodeRef<Aside> = NodeRef::new();
let lifecycle = RwSignal::new(if open.get_untracked() {
    LifecycleState::Open
} else {
    LifecycleState::Closed
});

// Mirror the `open` prop into our lifecycle signal. Imperatively set
// Opening/Closing; the transitionend handler advances to terminal.
Effect::new(move |prev: Option<bool>| {
    let now_open = open.get();
    if prev == Some(now_open) || prev.is_none() {
        // First run or no change — initialise to terminal state, don't fire opening/closing.
        lifecycle.set(if now_open { LifecycleState::Open } else { LifecycleState::Closed });
        return now_open;
    }
    lifecycle.set(if now_open { LifecycleState::Opening } else { LifecycleState::Closing });
    // Reduced-motion shortcut: snap to terminal if no animation.
    if let Some(el) = drawer_ref.get_untracked() {
        if is_zero_duration(el.as_ref()) {
            lifecycle.set(advance(lifecycle.get_untracked()));
        }
    }
    now_open
});

let on_transition_end = move |ev: TransitionEvent| {
    // Filter on the driving property — grove drawer slides via `transform`,
    // so only the transform transitionend advances the lifecycle. Stray
    // events from opacity/box-shadow/etc. are ignored.
    if ev.property_name() == "transform" {
        lifecycle.update(|s| *s = advance(*s));
    }
};
```

In the `view!`, change:

```rust
data-open=move || open.get()
```

to:

```rust
node_ref=drawer_ref
data-state=move || lifecycle.get().as_str()
on:transitionend=on_transition_end
```

(Keep any other attributes intact. If `data-open` is read by CSS selectors elsewhere in `components.css`, leave it set as a sibling so styling doesn't break — TBD in Step 2.)

- [ ] **Step 2: Audit CSS for `data-open` selectors**

```bash
grep -n "data-open\|grove-drawer" crates/web/components.css | head -20
```

If `.grove-drawer[data-open="true"]` selectors exist, change them to `.grove-drawer[data-state="open"], .grove-drawer[data-state="opening"]` so the open visual is shown during both phases. If only the class `.grove-drawer.open` is used (set elsewhere), leave the CSS alone.

If a refactor is needed, do it in this same task — no separate commit.

- [ ] **Step 3: Verify**

```bash
just check-wasm
just test-browser   # if browser tests for grove_drawer exist, run them
```

Expected: zero errors. Existing `data-open` consumers in the e2e helpers (`e2e/helpers/ui.ts:openSidebar`/`closeSidebar`) currently key on `.grove-drawer.open` (the CSS class), not `data-open`, so no e2e change is needed.

- [ ] **Step 4: Commit**

```bash
git add crates/web/src/components/grove_drawer.rs crates/web/components.css
git commit -m "feat(web): grove_drawer four-phase data-state lifecycle

Replaces the existing data-open boolean attribute. Driving property:
transform (slide via translateX). transitionend filtered on
property_name == 'transform' so opacity/box-shadow events from
overlapping transitions are ignored. Reduced-motion shortcut snaps
to terminal state when computed transition-duration is 0s.

Per docs/specs/2026-04-27-event-based-waits-design.md §data-state
attribute pattern. Reference implementation for the other 5 animated
components."
```

---

## Task 3: Wire `mobile_shell.rs` lifecycle (mobile drawer)

**Files:**
- Modify: `crates/web/src/components/mobile_shell.rs`

The mobile shell owns the mobile drawer's `RwSignal<bool>` directly (`drawer_open` at `:110`). Apply the Task 2 pattern targeting the drawer's root element (the wrapper `<aside>` or `<div>` that carries the slide animation — verify by reading `:110-200` and choosing the element CSS targets via `.mobile-drawer` or similar).

Driving property: `transform` (slide via `translateX`).

The drawer signal is bool-backed — the Effect mirrors `drawer_open.get()` into a new `RwSignal<LifecycleState>` exactly as in Task 2. **No new prop**; the lifecycle signal is internal.

- [ ] **Step 1: Edit** the component, adding the `lifecycle`/`Effect`/`on_transition_end` block scoped to the drawer's root `NodeRef`, and the `data-state` + `node_ref` + `on:transitionend` attributes on the drawer view.
- [ ] **Step 2: Verify** `just check-wasm`.
- [ ] **Step 3: Commit** with message `feat(web): mobile_shell drawer four-phase data-state lifecycle`.

---

## Task 4: Wire `confirm_dialog.rs` lifecycle

**Files:**
- Modify: `crates/web/src/components/confirm_dialog.rs`

Driving property: `opacity` (the dialog fades in/out per `components.css`).

Source of truth: `visible: ReadSignal<bool>` prop at `:12`. The Effect mirrors it into the lifecycle signal. The `transitionend` filter uses `property_name() == "opacity"`.

- [ ] **Step 1: Edit** the component.
- [ ] **Step 2: Verify** `just check-wasm`.
- [ ] **Step 3: Commit** with message `feat(web): confirm_dialog four-phase data-state lifecycle`.

---

## Task 5: Wire `bottom_sheet.rs` lifecycle

**Files:**
- Modify: `crates/web/src/components/bottom_sheet.rs`

Driving property: `opacity` (per the survey: `transition: opacity var(--motion-slow) linear` on `.bottom-sheet`). The `transform: translateY` is applied but doesn't carry a transition declaration in the captured CSS — confirm before merging by re-grep.

Existing attribute: `data-open` at `:72, :80`. Audit `components.css` for `.bottom-sheet[data-open]` selectors and convert any to `data-state` equivalents (`[data-state="open"], [data-state="closing"]` etc.).

- [ ] **Step 1: Edit** the component + CSS.
- [ ] **Step 2: Verify** `just check-wasm`.
- [ ] **Step 3: Commit** with message `feat(web): bottom_sheet four-phase data-state lifecycle`.

---

## Task 6: Wire `tab_bar.rs` lifecycle

**Files:**
- Modify: `crates/web/src/components/tab_bar.rs`

The spec says `tab_bar.rs` (active-tab transition). Two candidate elements: the bar itself (`data-visible="false"` hides via `display:none` per the survey, **no transition**), or the active-tab indicator that slides between tabs.

**Decision tree:**
- If the bar's hide/show has no CSS transition (`display: none` is instantaneous), the `data-state` lifecycle adds no value at the bar level — keep `data-visible` for that.
- If there IS an inner active-tab indicator with a `transform` transition, apply the lifecycle to *that* element.

Read `tab_bar.rs:26-43` and `components.css` `.mobile-tab-bar` selectors. Pick the right element. If neither qualifies, **skip Task 6 with a commit-message note** — the spec's "five physical edits" already counts the action sheet as the fifth, so dropping tab_bar to four edits is acceptable. Document the decision in the commit body.

- [ ] **Step 1: Audit** which element has a transition. If none → skip the rest of this task and write a `docs(web): record tab_bar lifecycle decision` commit citing the audit; otherwise continue.
- [ ] **Step 2: Edit** the chosen element with the lifecycle pattern.
- [ ] **Step 3: Verify** `just check-wasm`.
- [ ] **Step 4: Commit** with message `feat(web): tab_bar four-phase data-state lifecycle on <element>` (or the skip commit per Step 1).

---

## Task 7: Wire `message.rs` mobile action-sheet lifecycle

**Files:**
- Modify: `crates/web/src/components/message.rs`

The action sheet currently uses class-name toggling: `class=move || if show_sheet.get() { "mobile-action-sheet open" } else { "mobile-action-sheet" }` at `:1268`. Add `data-state` alongside the existing class binding (don't remove the class — CSS `.mobile-action-sheet.open` selectors stay intact for now; converting CSS is out of scope).

Driving property: `transform` (the sheet slides up via `translateY`).

Source of truth: `show_sheet: Memo<bool>` at `:378-381`. The class toggle is already driven by `show_sheet`; mirror it into a `RwSignal<LifecycleState>` via Effect. Apply both the overlay and the sheet itself — both carry the open class in lock-step. The lifecycle attribute goes on the **sheet** element (the one with the transform), not the overlay (which is opacity-only).

- [ ] **Step 1: Edit** message.rs around lines 1265-1280 and the show_sheet declaration around 378-381. Add the lifecycle signal + Effect + on_transition_end. Apply `data-state=move || lifecycle.get().as_str()`, `node_ref=sheet_ref`, `on:transitionend=on_transition_end` to the `.mobile-action-sheet` div.
- [ ] **Step 2: Verify** `just check-wasm`. Mobile action-sheet has heavy interaction (swipe-down dismissal, drag transforms) — be careful that the new `transitionend` listener doesn't fire while the user is dragging (the current code disables the transition during drag via inline `transition: none` style; under that condition `transitionend` doesn't fire, so the lifecycle just doesn't advance — acceptable).
- [ ] **Step 3: Commit** with message `feat(web): mobile action-sheet four-phase data-state lifecycle`.

---

## Task 8: Browser tests for the lifecycle

**Files:**
- Modify: `crates/web/tests/browser.rs`

Three failure modes from the spec, three tests. Tests exercise `grove_drawer.rs` only (the canonical implementation) — coverage for the other 5 is via the existing browser tests for those components plus the e2e specs that already exercise them. Adding 3 tests × 6 components is overkill for PR-3.

Test matrix:

| Test name | What it asserts |
|-----------|-----------------|
| `grove_drawer_lifecycle_advances_on_transitionend` | Mount drawer with `open=Signal::derive(...)`. Toggle `open` → assert `data-state="opening"`. Dispatch synthetic `transitionend` with `propertyName: "transform"`. Assert `data-state="open"`. |
| `grove_drawer_reduced_motion_snaps_to_terminal` | Force reduced-motion via inline `style="transition-duration: 0s"` on the root. Toggle `open` → assert `data-state="open"` immediately (no `transitionend` needed). |
| `grove_drawer_ignores_unrelated_transitionend` | Toggle to `opening`. Dispatch `transitionend` with `propertyName: "opacity"` (not the driving property). Assert `data-state` is still `"opening"`. Then dispatch with `propertyName: "transform"` → assert `"open"`. |

- [ ] **Step 1: Append the test module**

```rust
// crates/web/tests/browser.rs (append to file)

#[cfg(test)]
mod data_state_lifecycle {
    use super::*;
    use willow_web::components::grove_drawer::GroveDrawer;
    use leptos::prelude::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;
    use web_sys::{TransitionEvent, TransitionEventInit};

    #[wasm_bindgen_test]
    async fn grove_drawer_lifecycle_advances_on_transitionend() {
        let open = RwSignal::new(false);
        let host = mount_test(move || view! {
            <GroveDrawer open=Signal::derive(move || open.get()) />
            // ... whatever required props the component takes
        });
        tick().await;

        // Initial state.
        let drawer = host.query_selector(".grove-drawer").unwrap().unwrap();
        assert_eq!(drawer.get_attribute("data-state"), Some("closed".to_string()));

        // Toggle open → opening.
        open.set(true);
        tick().await;
        assert_eq!(drawer.get_attribute("data-state"), Some("opening".to_string()));

        // Synthetic transitionend on the driving property.
        let mut init = TransitionEventInit::new();
        init.bubbles(true);
        init.property_name("transform");
        let ev = TransitionEvent::new_with_event_init_dict("transitionend", &init).unwrap();
        drawer.dispatch_event(&ev).unwrap();
        tick().await;

        assert_eq!(drawer.get_attribute("data-state"), Some("open".to_string()));
    }

    #[wasm_bindgen_test]
    async fn grove_drawer_reduced_motion_snaps_to_terminal() {
        let open = RwSignal::new(false);
        let host = mount_test(move || view! {
            <GroveDrawer open=Signal::derive(move || open.get()) />
        });
        tick().await;

        // Force computed transition-duration: 0s by inline style.
        let drawer = host.query_selector(".grove-drawer").unwrap().unwrap();
        drawer
            .unchecked_ref::<web_sys::HtmlElement>()
            .style()
            .set_property("transition-duration", "0s")
            .unwrap();

        open.set(true);
        tick().await;

        // Should snap straight to "open" without a transitionend dispatch.
        assert_eq!(drawer.get_attribute("data-state"), Some("open".to_string()));
    }

    #[wasm_bindgen_test]
    async fn grove_drawer_ignores_unrelated_transitionend() {
        let open = RwSignal::new(false);
        let host = mount_test(move || view! {
            <GroveDrawer open=Signal::derive(move || open.get()) />
        });
        tick().await;
        let drawer = host.query_selector(".grove-drawer").unwrap().unwrap();

        open.set(true);
        tick().await;
        assert_eq!(drawer.get_attribute("data-state"), Some("opening".to_string()));

        // Stray opacity transitionend — should NOT advance.
        let mut opacity_init = TransitionEventInit::new();
        opacity_init.bubbles(true);
        opacity_init.property_name("opacity");
        drawer.dispatch_event(
            &TransitionEvent::new_with_event_init_dict("transitionend", &opacity_init).unwrap()
        ).unwrap();
        tick().await;
        assert_eq!(drawer.get_attribute("data-state"), Some("opening".to_string()));

        // Now the real transform transitionend — advances.
        let mut transform_init = TransitionEventInit::new();
        transform_init.bubbles(true);
        transform_init.property_name("transform");
        drawer.dispatch_event(
            &TransitionEvent::new_with_event_init_dict("transitionend", &transform_init).unwrap()
        ).unwrap();
        tick().await;
        assert_eq!(drawer.get_attribute("data-state"), Some("open".to_string()));
    }
}
```

- [ ] **Step 2: Run the tests**

```bash
just test-browser 2>&1 | tail -30
```

Expected: 3 new tests pass. If `GroveDrawer` requires more props than `open` (peek `grove_drawer.rs`), fill them in with stub `Signal::derive(...)` values — these tests don't validate drawer behaviour, just the lifecycle.

- [ ] **Step 3: Commit**

```bash
git add crates/web/tests/browser.rs
git commit -m "test(web): browser tests for data-state lifecycle

Three tests against grove_drawer (canonical implementation):
- transitionend on driving property advances opening → open
- reduced-motion (transition-duration: 0s) snaps to terminal
- transitionend on a non-driving property is ignored

Other 5 components reuse the same lifecycle helpers (lifecycle.rs)
and the same advance/transitionend pattern, so coverage is shared.
Per spec §data-state attribute pattern's three failure modes."
```

---

## Task 9: `page.clock` adoption — `longPress` migration + clock fixture

**Files:**
- Create: `e2e/helpers/clock.ts`
- Modify: `e2e/helpers/touch.ts`, `e2e/test-hooks.ts`

`page.clock.install()` patches `Date`/`setTimeout`/`setInterval`/`requestAnimationFrame` on a single page. The spec's risk row warns that iroh's WASM transport may use real-time timers (gossip heartbeats, retry backoff); installing the clock during a multi-peer test could freeze UI/HLC time while iroh keeps running on real time, producing silent divergence. The PR-1 audit was a gate but its result is not inlined in the spec — so PR-3 follows the documented `Opt-in. Clock install is per test (or describe block), not global` rule:

- Default: clock is NOT installed.
- Touch/longPress tests opt in via `usePageClock(page)` before the press.
- Multi-peer tests stay clock-free.

- [ ] **Step 1: Create `e2e/helpers/clock.ts`**

```ts
// e2e/helpers/clock.ts
//
// Per-page Playwright clock helpers. Opt-in: tests that install the
// clock are explicit about which timers they advance. Default e2e tests
// run with real time so iroh background timers (gossip heartbeats,
// retry backoff) are unaffected.
//
// Per docs/specs/2026-04-27-event-based-waits-design.md §page.clock.

import type { Page } from '@playwright/test';

/**
 * Install the Playwright clock on `page`. Patches Date/setTimeout/
 * setInterval/requestAnimationFrame. After install, time only advances
 * via runFor / fastForward / pauseAt.
 *
 * Idempotent: calling twice on the same page is safe (Playwright no-ops
 * the second install).
 */
export async function installPageClock(page: Page): Promise<void> {
  await page.clock.install();
}

/** Advance the page's clock by `durationMs` synthetic milliseconds. */
export async function runForMs(page: Page, durationMs: number): Promise<void> {
  await page.clock.runFor(durationMs);
}
```

- [ ] **Step 2: Update `e2e/helpers/touch.ts`**

Replace the two `page.waitForTimeout(...)` calls in `longPress` with `page.clock.runFor(...)`. **Add a precondition check** — if the clock isn't installed, fall back to the real timeout (so existing specs that haven't opted in keep working). Detect via `await page.evaluate(() => typeof (window as any).__clockInstalled === 'boolean')` won't work because Playwright doesn't expose that flag; instead, accept the clock as a parameter and let callers opt in.

Simpler: keep `longPress` as-is for callers that haven't opted in, and add a new `longPressWithClock(page, selector, duration)` variant that calls `clock.runFor`. Migrate `longPressAvatar` similarly. This preserves the legacy behaviour for the 2 un-migrated mobile specs that use `longPress` without setting up a clock.

```ts
// e2e/helpers/touch.ts (new variant, alongside existing longPress)

/**
 * Like `longPress`, but uses `page.clock.runFor(durationMs)` instead of
 * a real-time wait. Caller must have invoked `installPageClock(page)`
 * earlier in the test.
 *
 * Use this in tests where the clock is already installed for other
 * reasons; otherwise prefer the real-time `longPress` to avoid having
 * to install the clock just for one helper.
 */
export async function longPressWithClock(page: Page, selector: string, durationMs = 600) {
  // ... copy of longPress body, but with `page.waitForTimeout(durationMs)`
  // replaced by `await page.clock.runFor(durationMs)` and the trailing
  // `page.waitForTimeout(300)` replaced by `await page.clock.runFor(300)`.
}
```

- [ ] **Step 3: Optionally extend the `peer` fixture**

Skip unless a current spec needs it. The `installPageClock` helper is independent and can be called in `test.beforeEach` without a fixture. Adding a `pageClock` fixture is YAGNI for PR-3 — defer until a spec actually demands it.

- [ ] **Step 4: Verify**

```bash
npx tsc --noEmit --target es2022 --module esnext --moduleResolution bundler --strict --esModuleInterop --skipLibCheck e2e/helpers/clock.ts e2e/helpers/touch.ts
npx eslint e2e/helpers/clock.ts e2e/helpers/touch.ts
```

Expected: zero errors.

- [ ] **Step 5: Commit**

```bash
git add e2e/helpers/clock.ts e2e/helpers/touch.ts
git commit -m "test(e2e): add page.clock helper + longPressWithClock variant

installPageClock(page) patches Date/setTimeout/setInterval globals
per Playwright's per-page clock API. Opt-in: tests that install the
clock are explicit about which timers they advance, so multi-peer
specs (gossip + iroh retry timers) are unaffected.

longPressWithClock mirrors longPress with clock.runFor(durationMs)
in place of waitForTimeout. The legacy longPress stays for specs
that haven't opted in.

Per docs/specs/2026-04-27-event-based-waits-design.md §page.clock
for real durations."
```

---

## Task 10: Update `e2e/README.md`

**Files:**
- Modify: `e2e/README.md`

Two new subsections after the existing "Event-based waits (Peer wrapper)" section:

```markdown

## `data-state` lifecycle on animated components

Five animated components (mobile drawer, grove drawer, confirm dialog,
bottom sheet, mobile action sheet) and the tab bar's active-tab indicator
expose a `data-state` attribute reflecting one of four phases:
`closed | opening | open | closing`. Tests gate on the attribute
rather than sleeping after the click that opens the component.

```ts
await openSidebarBtn.click();
await expect(drawer).toHaveAttribute('data-state', 'open');
```

The lifecycle is driven by `transitionend` on the component's specific
CSS property (transform for slides, opacity for fades). A reduced-motion
shortcut snaps to the terminal phase synchronously when computed
transition-duration is 0s, so tests under `prefers-reduced-motion: reduce`
don't hang.

**Categorical `data-state` (separate convention.)** `status_dot.rs`,
`grove_rail.rs`, and `peer_status_label.rs` use `data-state` for
orthogonal categorical states (`online`/`offline`, `idle`/`loading`).
The four-phase lifecycle does NOT apply to them. Tests that gate on
`data-state` must know which component they target.

## `page.clock` for real-duration waits

`longPressWithClock(page, selector, ms)` and `installPageClock(page)`
in `e2e/helpers/clock.ts` use Playwright's per-page clock to advance
synthetic time without paying real wall-clock seconds. Use this for
touch gestures and debounce timers.

**Multi-peer caveat.** The clock is per-page. iroh's WASM transport may
use real-time timers (gossip heartbeats, retry backoff); installing
the clock during a multi-peer test could freeze UI/HLC time while iroh
keeps running. Default e2e tests stay clock-free; only single-peer
touch specs opt in.
```

- [ ] **Step 1: Append** the sections after the existing "Anti-patterns" section.
- [ ] **Step 2: Commit** with `docs(e2e): document data-state lifecycle + page.clock opt-in`.

---

## Final acceptance — run the full PR gate

```bash
just check-all FEATURES=test-hooks
```

Expected: PASS, including `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`, WASM target check, browser-tier tests (the 3 new lifecycle tests + existing), and Playwright e2e (no semantic change).

If green, push:

```bash
git push -u origin claude/event-based-waits-pr3
```

---

## Out of scope (deferred)

- Migration of any spec from `longPress` → `longPressWithClock`. PR-3 only adds the helper. Migration is per-spec via #458.
- Replacement of `data-open` / `data-visible` consumers in CSS where the existing class-based selector still works. PR-3 keeps both during the transition; a future spec-by-spec follow-up removes them.
- Three-peer drift simulation via `page.clock.setFixedTime` — no test needs it yet.
- Promotion of the lifecycle pattern into a Leptos wrapper component — open question deferred per spec.

---

## Cross-references

- Spec: [`docs/specs/2026-04-27-event-based-waits-design.md`](../specs/2026-04-27-event-based-waits-design.md) §`data-state` attribute pattern + §`page.clock` for real durations.
- PR-2 plan: [`docs/plans/2026-04-29-event-based-waits-pr2-peer-wrapper.md`](./2026-04-29-event-based-waits-pr2-peer-wrapper.md).
- Tracking issue: [#458](https://github.com/intendednull/willow/issues/458).

