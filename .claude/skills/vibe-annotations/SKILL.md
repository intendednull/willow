---
name: vibe-annotations
description: Drive the vibe-annotations feedback loop on a running web UI. Use when the user asks to "check annotations", "do a vibe pass", "read annotations", or hands you a running dev server + MCP vibe-annotations endpoint. Reads pending annotations, triages them, implements mechanical fixes, proposes options for structural ones, opens issues for changes that need specs, and deletes annotations once addressed.
---

# Vibe-annotations workflow

Drive a tight feedback loop: annotation in Chrome → code change → rebuild visible → annotation deleted. Keep it terse. Work autonomously on mechanical asks. Ask before destructive/structural work.

## REQUIRED: invoke frontend-design skill first

**Before reading annotations or writing any UI/CSS code, invoke `frontend-design:frontend-design` via the `Skill` tool.** It loads design-system knowledge (tokens, spacing, typography, component patterns) needed to make annotation fixes cohere with the target UX. Re-invoke it if the session compacts the skill content out of context.

Applies to every vibe-annotations session — mechanical fixes included. Copy changes without visual impact are the only exception.

## Self-improvement note

**This skill is iteratively evolving.** If you notice a workflow friction point (a decision you made that could be systematized, a command sequence worth memorizing, a missing tool permission, a heuristic that failed), record it in a dedicated `## Lessons` section at the bottom of this file before ending the session. Don't edit the top of the file without user approval — append to `## Lessons`, and the user will fold good lessons upward over time.

## Preconditions to verify on entry

1. Dev stack running somewhere (relay + workers + web UI on a known port).
2. MCP server `vibe-annotations` reachable (annotations are surfaced via the `mcp__vibe-annotations__*` tools).
3. You are in a git worktree isolated from any concurrent dev session. If the main worktree's `trunk serve` owns port 8080, start your own trunk on a free port (e.g. 8081) pointed at the shared relay. One-line pattern:
   ```
   cd <worktree>/crates/web && trunk serve --port 8081 --address 127.0.0.1 > /tmp/trunk-8081.log 2>&1 &
   ```
   Then arm a Monitor on the log so build failures surface automatically.

## Reading annotations

- First call: `read_annotations` with `url` filter matching the port you're serving (e.g. `http://localhost:8081/*`). Without the filter you get cross-project noise.
- `element_context` + `parent_chain` + `selector` are usually enough. Call `get_annotation_screenshot` only when styling/layout/visual-hierarchy asks need pixel context that text can't convey.
- Treat terse or incomplete comments charitably. Read the element path + surrounding UI to infer intent. If ambiguous after that, ask briefly.

## Verifying visually with `agent-browser`

Use the `agent-browser` CLI to drive the live UI and confirm fixes land. It's a Chromium-based headed browser with an accessibility-tree snapshot for element refs. Key commands:

- `agent-browser open <url>` — navigate.
- `agent-browser snapshot` — get interactive element tree with `ref=e{N}` handles.
- `agent-browser click e3` / `fill e4 "text"` / `key "Enter"` — interact by ref.
- `agent-browser screenshot /tmp/shot.png` — capture for before/after comparison.
- `agent-browser eval "<js>"` — run JS in page context. Invaluable for measuring computed styles / `offsetHeight` when diagnosing why a CSS change didn't land the expected effect.
- `agent-browser close` — release the browser.

When a visual fix "looks wrong," pull computed dimensions via `eval` before guessing at CSS tweaks. Typical pattern:
```
agent-browser eval "JSON.stringify(Array.from(document.querySelectorAll('.foo')).map(el => ({h: el.offsetHeight, mt: getComputedStyle(el).marginTop})))"
```
Reveals hidden line-boxes, min-heights, and inline-block ghosts that the static CSS read can't.

## Triage — three buckets

| Bucket | Examples | Response |
|---|---|---|
| **Mechanical** | copy change, remove a button, rename a label, add an icon, tweak spacing, swap one class | Implement immediately. |
| **Structural** | restructure a pane, change interaction flow, combine two UI concerns | Reply with 2–3 options + tradeoffs in ≤3 sentences. Wait for direction unless the path is obvious. |
| **State / spec-level** | new `EventKind`, new permission, anything touching `willow-state` authority | Open a GitHub issue via `gh issue create` with context, proposed implementation, and out-of-scope list. Defer. Do NOT implement in the same session unless explicitly asked. |

## Implementation rules

1. **Find the source**: grep for the class name or an excerpt of the annotation's visible text. Identify the Leptos component file.
2. **Edit components + CSS together.** When you remove a class, remove its CSS. When you add a new class, add rules in the same pass. Leave unused rules only if removing them would ripple (note it in `## Lessons` if it does).
3. **Tests at the lowest tier that can cover the change.** For UI markup: a `crates/web/tests/browser.rs` static-markup contract test. For state changes: a state crate test. Never add a Playwright test unless the behavior genuinely requires multi-peer or cross-browser coverage.
4. **`query()` helper takes `&HtmlElement`, not `&Element`.** When asserting inside a queried element, requery from `container` with a compound selector (e.g. `query(&container, "button.foo .bar")`) instead of passing the intermediate element.
5. **Thread props through parents** when splitting UI concerns (e.g. `on_close` from `RightRail` → `MemberList`). Update every call site; remove unused props when you're sure they're no longer needed (check `app.rs` + `mobile_shell.rs` at minimum).
6. **Delete each annotation via `mcp__vibe-annotations__delete_annotation` as soon as its fix is written.** Don't batch to end-of-session. (Memory: `feedback_delete_annotations_when_addressed.md`.)
7. **Commit + push after each annotation lands.** One annotation → one commit → one push. Do NOT batch multiple annotations into one commit unless they're truly inseparable (e.g. a CSS rule + its matching component edit for the same ask). Sequence:
   ```
   just fmt && just clippy
   git add -A
   git commit -m "ui(vibe): <terse what> — <why/annotation gist>"
   git push
   ```
   Commit message stays normal prose (caveman doesn't apply). Never `--no-verify`; if a hook fails, fix the root cause.
8. **Verify compile after each batch** — no tests:
   ```
   cargo check -p willow-web --target wasm32-unknown-unknown
   cargo check --workspace --tests
   ```
   If the other session holds the `target/` lock, expect `Blocking waiting for file lock` — wait it out, don't clean.

## Handling build failures

- **Incremental compiler ICE** (`rust_begin_unwind` / `incremental_verify_ich_failed`): almost always a shared `target/` corruption from parallel cargo runs. Ask the user for permission to run `cargo clean -p willow-web --target wasm32-unknown-unknown` (narrow, preserves worker/relay builds). Don't run it unauthorized.
- **Dead trunk watcher** (log shows old error, no rebuild after touching files): ask the user to `pkill -f "trunk serve --port <port>"` then re-spawn trunk yourself in the background.
- **Leptos closure errors** (`FnMut` vs `FnOnce`, moved-variable): usually you cloned a non-trivially-cloneable closure inside a `.map(|kind| ...)` block. Lift the clone to the enclosing `.then(|| { let foo = outer_foo.clone(); ... })` scope so the inner closure only sees already-owned values.
- **`web_sys::ScrollToOptions` / missing methods in LSP diagnostics for files you didn't touch**: likely a stale LSP against a concurrently-edited file from another session. Ignore if `cargo check` is green.

## Structural-option framing

When replying with 2–3 options for a structural ask:
- Lead with the tradeoff, not the mechanics.
- State cost crudely (`~30 min client-local`, `~2h state-synced`).
- Recommend one + say why in one sentence. Don't pick for the user — let them redirect.

## GitHub issue template for state-level defers

```
## Context
<one paragraph — what the annotation asked + where the UI lives>

## Scope
<why it belongs in state vs. client-local>

## Proposed implementation
1. `crates/state/...` — <new EventKind + permission + apply>
2. `crates/client/...` — <method + bridge>
3. `crates/web/...` — <signal + render + test>

## Out of scope
- <adjacent things this issue does NOT cover>

## Links
- Source: <path>
- Spec: <docs link>
- Authority model: docs/specs/2026-04-12-state-authority-and-mutations.md
```

## Caveman / output discipline

- Caveman mode (if active) applies to chat output, not commit messages, PR bodies, issue bodies, or code comments.
- After each batch of annotations: one line per annotation, then "ready for next cycle" or the compile status. Skip summaries longer than a screen.
- Don't narrate unless blocked.

## Worklog at end of session

When the user stops handing you annotations, **before signing off**:
1. List remaining pending annotations (unaddressed or parked with an issue link).
2. Note any failed experiments or gotchas in `## Lessons` below.
3. Don't offer to commit / PR / merge unless the user asks.

## Lessons

<!-- Append new lessons below as `- YYYY-MM-DD — lesson`. Keep each under ~3 lines.
     Good lessons are specific, tie to a concrete failure, and name the file / symptom. -->

- 2026-04-21 — `query()` in `crates/web/tests/browser.rs` takes `&HtmlElement`, not `&Element`. After `query(...).expect(...)` the bound value is an `Element`; compound selectors from `container` avoid the cast dance.
- 2026-04-21 — Leptos `Effect::new(move |prev: Option<T>| ...)` with a transition check (`prev.unwrap_or(false)` → `is_some`) is the right pattern for "fire once on open", not a raw unconditional focus. An unconditional Effect inside a block that remounts will steal focus on every keystroke.
- 2026-04-21 — Shared `target/` across worktrees invites incremental-cache ICEs under concurrent `cargo check` / `trunk serve`. If the user is running `just dev` in another session, expect it and plan for `cargo clean -p willow-web --target wasm32-unknown-unknown` as the first remediation.
- 2026-04-21 — When removing a prop from a component (e.g. dropped `connection_status` + `peer_count` from `ChannelSidebar`), grep every caller — `app.rs` and `mobile_shell.rs` both wire the desktop + mobile shells, and leaving a stale `x=x` arg silently compiles into an unused-arg warning or an error depending on the prop macro.
- 2026-04-21 — Terse annotation comments ("looks good, lets have button for new row") often mean "ship what's on screen + add one thing". Read the attached element + the latest edit you made to that area; don't over-interpret.
- 2026-04-23 — "Defocus" annotations: suspect silent remounts before blaming a focus-stealer. If no `blur()`/`focusout` fires on the input and yet focus is lost, attach a `MutationObserver` on the parent subtree — paired DIV add/remove entries mean Leptos is swapping the view. Look upstream at which reactive closure re-ran.
- 2026-04-23 — `crates/web/src/state_bridge.rs` `DerivedStateActor.cached` must be updated after every successful `write.set()` (wrapped in `Arc<Mutex<Option<U>>>` so the async handler can mutate across `.await`). Without it, every upstream `Notify` fires downstream signals regardless of value equality — 27 derived_signals across the UI remount views on every gossip tick (~2s). If a post-fix UI still re-renders on that cadence, check this first.
- 2026-04-23 — Leptos 0.8 default `signal()` notifies on every `set()` regardless of `PartialEq`. Memos check equality. When a `move ||` closure reads a signal that may be written with the same value repeatedly, wrap in `Memo::new(|_| sig.get())` to gate propagation — otherwise the whole subtree rebuilds.
- 2026-04-23 — `agent-browser` `.click()` fires only `click`, not `mousedown`. Components using `on:mousedown` with `preventDefault` (e.g. `.tree-slot__save` / `.tree-slot__cancel` in `channel_sidebar.rs`) ignore `.click()`. Automate via `el.dispatchEvent(new MouseEvent('mousedown',{bubbles:true}))`.
- 2026-04-23 — `agent-browser eval` state persists across CLI invocations via `window`. Each `eval` is a fresh CDP `Runtime.evaluate`, but the page's `window` is the same. Register listeners once (`document.addEventListener(...)`) and `window.__log = []` between steps to reset. Lets you trace events that happen between two `eval` calls.
- 2026-04-23 — Relay binds IPv4-only (`Ipv4Addr::UNSPECIFIED` at `crates/relay/src/main.rs:128`). Chrome resolves `localhost` to `::1` first and reports `ERR_FAILED` on the bootstrap-id fetch before Happy-Eyeballs falls back to 127.0.0.1. Console noise, not broken — don't chase it unless the fetch is functionally failing.
