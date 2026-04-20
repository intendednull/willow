# UI Phase 0 — Foundation Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the `foundation.md` spec — new palette, typography, iconography stroke, motion, density, accent variants, states, and copy-voice infrastructure — as a standalone CSS foundation that every later spec can consume, without changing a single component's markup or behaviour in this phase.

**Architecture:** Introduce a new `crates/web/foundation.css` loaded before `style.css` that defines every foundation token from the spec. Leave existing component CSS and Leptos components unchanged. Remap the current Discord-blurple tokens in `style.css` to alias the new foundation tokens so existing components automatically reskin to willow's moss/bark palette without any Rust edits. Accent, density, and reduced-motion variants are wired so later phases can toggle them. No visible behaviour changes in Phase 0; the visual change is the whole point.

**Tech Stack:** CSS3 custom properties (variables), Google Fonts CDN (Fraunces, IBM Plex Sans, JetBrains Mono), Trunk bundler, Leptos WASM browser tests (wasm-pack + chrome), just runner.

**Scope gate:**
- In scope: all of `docs/specs/2026-04-19-ui-design/foundation.md`.
- Out of scope: component CSS migration (uses legacy token aliases), terminology copy edits, layout changes, new components, accent / density / theme toggle UI (Phase 5).
- Spec source of truth: `/mnt/storage/projects/willow/docs/specs/2026-04-19-ui-design/foundation.md`.

**Branch:** `design/ui-target-ux` already checked out. Work continues on this branch. Plan lands as one or more small commits on the branch; the branch carries the whole Phase 0 PR.

---

## File structure

| Path | State | Responsibility |
|------|-------|----------------|
| `crates/web/foundation.css` | **new** | All Phase 0 foundation tokens, keyframes, state helpers, accent variants, density classes, reduced-motion path, scrollbar / selection / focus defaults. Single source of truth for design primitives. |
| `crates/web/index.html` | modify | Insert `<link data-trunk rel="css" href="foundation.css" />` ABOVE the existing `style.css` link so foundation loads first and is the base layer. Update `data-theme` / `data-accent` / density class on root as needed. |
| `crates/web/style.css` | modify | (a) remove Google Fonts `@import` at top (moved to foundation.css so all three fonts come from one source); (b) remap `:root, [data-theme="dark"]` legacy token block so each legacy variable aliases a foundation token via `var(--new)` rather than hardcoding hex. No changes to selectors, layout rules, or `[data-theme="light"]` block (light theme is deferred per foundation.md). |
| `crates/web/src/app.rs` | modify | Set `id="app-root"` on the root Leptos container div and an initial `data-accent="moss"` + `density-balanced` class, so later phases can swap accent/density by writing to those attributes. |
| `crates/web/tests/browser.rs` | modify | Add a small test module `foundation_tokens` that asserts: (1) the `--bg-0` / `--moss-2` / `--whisper` / `--ink-on-accent` / `--font-display` custom properties are defined; (2) the legacy aliases resolve to the same computed value as their new counterparts; (3) `data-accent="ember"` swaps `--moss-2`. |

No other files change in Phase 0. Component stylesheets, `main.rs`, the service worker, manifest, icons, and every Rust `.rs` component file are untouched.

---

## Acceptance gates (before the phase PR is opened)

Each gate is a concrete check. Don't move to the next task if the current gate fails.

1. `just fmt && just clippy && just test-state && just test-client` pass on the branch (no Rust broke).
2. `just check-wasm` passes.
3. `just build-web` succeeds and produces `dist/index.html` + `dist/foundation-*.css` + `dist/style-*.css` in that order inside the HTML.
4. `wasm-pack test --headless --firefox crates/web` passes, with the new `foundation_tokens` test module green.
5. `just dev` launches, the browser loads `http://localhost:8080`, and:
   - the background reads as the deep-bark radial-gradient (no Discord blurple);
   - message text renders in IBM Plex Sans, headings in Fraunces italic, monospace as JetBrains Mono;
   - accent elements (buttons, unread badges) read as moss green, not Discord blurple;
   - no console errors, no missing-font FOUT that lasts > 500 ms;
   - every existing feature (send message, open settings, etc.) still works — this is a pure reskin.
6. `docs/specs/2026-04-19-ui-design/foundation.md` §Acceptance checklist items 1–4 visually sampled once in dev.

---

## Task 1: Create `foundation.css` skeleton and wire it into the build

**Files:**
- Create: `crates/web/foundation.css`
- Modify: `crates/web/index.html`

- [ ] **Step 1.1 — Write the file with a comment header only.**

Create `crates/web/foundation.css` with:

```css
/* foundation.css — willow UI foundation tokens
 *
 * Source spec: docs/specs/2026-04-19-ui-design/foundation.md
 * Parent:      docs/specs/2026-04-19-ui-design/README.md
 *
 * This file is the single source of truth for willow's design primitives:
 * palette, typography, iconography rules, motion, density, accent variants,
 * states, scrollbars, selection, and focus. It is loaded BEFORE style.css;
 * style.css remaps its legacy tokens to reference the foundation below.
 *
 * Nothing in this file touches component selectors. Components keep their
 * own stylesheets; they consume foundation tokens by name.
 */
```

- [ ] **Step 1.2 — Add the Trunk link in `index.html`.**

In `crates/web/index.html`, insert the foundation link immediately before the existing `style.css` link:

```html
    <link data-trunk rel="css" href="foundation.css" />
    <link data-trunk rel="css" href="style.css" />
```

Order matters: foundation first so style.css can override or alias.

- [ ] **Step 1.3 — Build.**

Run:

```bash
just build-web
```

Expected: build succeeds. Open `dist/index.html` and confirm the emitted stylesheets include both `foundation-<hash>.css` before `style-<hash>.css`.

- [ ] **Step 1.4 — Commit.**

```bash
git add crates/web/foundation.css crates/web/index.html
git commit -m "ui(phase-0): scaffold foundation.css and wire into trunk"
```

---

## Task 2: Load Fraunces + IBM Plex Sans + JetBrains Mono via foundation.css

**Files:**
- Modify: `crates/web/foundation.css`
- Modify: `crates/web/style.css`

- [ ] **Step 2.1 — Add the single Google Fonts import at the top of `foundation.css` (after the comment header).**

```css
@import url('https://fonts.googleapis.com/css2?family=Fraunces:ital,opsz,wght@0,9..144,300;0,9..144,400;0,9..144,500;0,9..144,600;1,9..144,400&family=IBM+Plex+Sans:wght@300;400;500;600&family=JetBrains+Mono:wght@400;500&display=swap');
```

The weights match exactly what `foundation.md` §Typography scale consumes: Fraunces 300/400/500/600 + italic 400; IBM Plex Sans 300/400/500/600; JetBrains Mono 400/500.

- [ ] **Step 2.2 — Remove the legacy font `@import` from `style.css`.**

Delete lines 1 and 2 of `crates/web/style.css`:

```css
/* ── Typography: IBM Plex Sans + Mono ─────────────────────────────── */
@import url('https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;500;600&family=IBM+Plex+Mono:wght@400;500&display=swap');
```

Leave the rest of `style.css` untouched. Note: the `IBM Plex Mono` family is no longer loaded; we'll alias `font-family: 'IBM Plex Mono'` to JetBrains Mono via a legacy alias in Task 13 so any existing `code`/`pre` rules in `style.css` still resolve to a monospace font.

- [ ] **Step 2.3 — Build and visually verify fonts load.**

Run `just dev`. Open `http://localhost:8080`. In the browser devtools → Network → CSS, confirm exactly one request to `fonts.googleapis.com/css2?family=Fraunces...`. In Computed styles for `body`, confirm `font-family` still resolves (not a generic fallback). Typography may look off because `style.css` still sets `font-family: 'IBM Plex Sans'` — that's fine for this task.

- [ ] **Step 2.4 — Commit.**

```bash
git add crates/web/foundation.css crates/web/style.css
git commit -m "ui(phase-0): move font loading to foundation.css + add Fraunces + JetBrains Mono"
```

---

## Task 3: Add background + line + ink + ink-on-accent tokens

**Files:**
- Modify: `crates/web/foundation.css`

- [ ] **Step 3.1 — Open the `:root` block in `foundation.css` and add the palette tokens.**

Append below the `@import`:

```css
/* ── Background (deep bark → lit panel) ─────────────────────────── */
:root {
  --bg-0: #14130f;          /* page background, deepest */
  --bg-1: #1b1a15;          /* primary panel (sidebar, thread, popover) */
  --bg-2: #22211b;          /* raised surface (card, code, input) */
  --bg-3: #2a2822;          /* hover on --bg-1 */
  --bg-4: #34322a;          /* active / pressed */

  /* ── Line (separators, borders) ──────────────────────────────── */
  --line:      #34322a;
  --line-soft: #22211b;

  /* ── Ink (text contrast ladder) ──────────────────────────────── */
  --ink-0: #f1ede2;         /* highest contrast: display, focused read */
  --ink-1: #d9d3c2;         /* body default */
  --ink-2: #a8a290;         /* secondary / meta */
  --ink-3: #7a7463;         /* muted / hint */
  --ink-4: #504b3f;         /* disabled / divider-as-text */
  --ink-on-accent: #14130f; /* ink on filled moss / amber / err buttons */
}
```

- [ ] **Step 3.2 — Build and visually check nothing regressed.**

Run `just build-web`. Open in browser. Expected: no change yet (style.css still holds hardcoded values). Confirm devtools → Elements → Computed → `:root` shows the new `--bg-0`, `--ink-1`, `--ink-on-accent` custom properties.

- [ ] **Step 3.3 — Commit.**

```bash
git add crates/web/foundation.css
git commit -m "ui(phase-0): add background, line, ink tokens to foundation"
```

---

## Task 4: Add accent ladder (moss default) + whisper + amber + semantic tokens

**Files:**
- Modify: `crates/web/foundation.css`

- [ ] **Step 4.1 — Append to the `:root` block.**

```css
:root {
  /* ── Accent ladder (moss, default) ───────────────────────────── */
  --moss-0: #2a3a28;        /* deep accent surface */
  --moss-1: #425c3d;        /* selection highlight, active pill bg */
  --moss-2: #6a8d5e;        /* primary interactive */
  --moss-3: #93b582;        /* hover / focus variant of primary */
  --moss-4: #c3d8b2;        /* fg on deep accent (icon on fill) */
  --willow: #b8c67a;        /* wordmark, own-avatar tint */

  /* ── Bark (amber warmth) ─────────────────────────────────────── */
  --amber:      #c99b55;    /* warm accent (queued, warn-not-error) */
  --amber-soft: #7a5a2e;    /* deep amber for borders, tag bg */

  /* ── Whisper (private side-channel) ──────────────────────────── */
  --whisper: #a88fc9;       /* violet — only for whisper surfaces */

  /* ── Semantic ─────────────────────────────────────────────────── */
  --ok:   #8fb36a;
  --warn: #d6a54a;
  --err:  #c97a5a;
}
```

- [ ] **Step 4.2 — Commit.**

```bash
git add crates/web/foundation.css
git commit -m "ui(phase-0): add accent, whisper, amber, semantic tokens"
```

---

## Task 5: Add radius + shadow + focus-ring + font stack tokens + motion variables

**Files:**
- Modify: `crates/web/foundation.css`

- [ ] **Step 5.1 — Append to the `:root` block.**

```css
:root {
  /* ── Radius ───────────────────────────────────────────────────── */
  --radius-s: 6px;          /* tags, pills, chips */
  --radius:   10px;         /* panels, cards, inputs */
  --radius-l: 16px;         /* sheets, full-card popovers */

  /* ── Shadow ──────────────────────────────────────────────────── */
  --shadow-1: 0 1px 0 rgba(255,255,255,0.02) inset, 0 1px 2px rgba(0,0,0,0.4);
  --shadow-2: 0 20px 50px -20px rgba(0,0,0,0.8), 0 4px 12px rgba(0,0,0,0.4);

  /* ── Focus ───────────────────────────────────────────────────── */
  --focus-ring: 0 0 0 2px var(--moss-1), 0 0 0 3px rgba(106, 141, 94, 0.6);

  /* ── Motion ──────────────────────────────────────────────────── */
  --motion-fast:    120ms;
  --motion:         180ms;
  --motion-slow:    240ms;
  --motion-ambient: 1200ms;
  --motion-ease:    cubic-bezier(0.2, 0.8, 0.2, 1);

  /* ── Typography stacks ───────────────────────────────────────── */
  --font-display: 'Fraunces', Georgia, serif;
  --font-ui:      'IBM Plex Sans', system-ui, sans-serif;
  --font-mono:    'JetBrains Mono', ui-monospace, monospace;
}
```

- [ ] **Step 5.2 — Commit.**

```bash
git add crates/web/foundation.css
git commit -m "ui(phase-0): add radius, shadow, focus-ring, motion, font stack tokens"
```

---

## Task 6: Body background gradient + base html/body rules

**Files:**
- Modify: `crates/web/foundation.css`

- [ ] **Step 6.1 — Append below the `:root` block.**

```css
/* ── Body background + base ─────────────────────────────────────── */

html, body {
  background: var(--bg-0);
  color: var(--ink-1);
  font-family: var(--font-ui);
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

/* Desktop body uses two softly offset radial gradients so the
   surface never reads flat. Mobile deepens the base and widens
   the gradients (applies below 720 px). */
body {
  background:
    radial-gradient(1200px 600px at 10% -10%, rgba(106,141,94,0.07), transparent 60%),
    radial-gradient(900px 500px at 110% 110%, rgba(201,155,85,0.05), transparent 60%),
    var(--bg-0);
}

@media (max-width: 720px) {
  body {
    background:
      radial-gradient(1400px 700px at 20% 0%, rgba(106,141,94,0.06), transparent 60%),
      radial-gradient(1000px 500px at 100% 100%, rgba(201,155,85,0.04), transparent 60%),
      #0c0b08;
  }
}
```

The existing `html, body` rule block in `style.css` (lines 90–103) sets `width/height/overflow/font-size/line-height` and will win for those; we only want to replace the `background` + `color` + `font-family` as a *fallback* the legacy block can still override where it needs to. That's fine because the current legacy `html, body` rule sets `background: var(--bg-main)` which we'll alias to `var(--bg-0)` in Task 13.

- [ ] **Step 6.2 — Build + visual check.**

Run `just dev`. Load the app. Expected: the deep-bark gradient shows through only if `style.css`'s legacy rule hasn't loaded yet; otherwise `var(--bg-main)` (still `#1a1a1e` at this point) wins. No break either way.

- [ ] **Step 6.3 — Commit.**

```bash
git add crates/web/foundation.css
git commit -m "ui(phase-0): add body background with radial gradients + base html/body"
```

---

## Task 7: Add named keyframes + reduced-motion overrides

**Files:**
- Modify: `crates/web/foundation.css`

- [ ] **Step 7.1 — Append keyframe definitions.**

```css
/* ── Keyframes ──────────────────────────────────────────────────── */

@keyframes willowPulse {
  0%, 100% { opacity: 0.3; transform: scale(0.8); }
  50%      { opacity: 1;   transform: scale(1.3); }
}

@keyframes willow-pop-in {
  from { opacity: 0; transform: translateY(-4px); }
  to   { opacity: 1; transform: translateY(0);    }
}

@keyframes leafFall {
  0%   { transform: translateY(-12vh) rotate(-8deg); opacity: 0; }
  20%  {                                             opacity: 1; }
  100% { transform: translateY(120vh)  rotate(24deg); opacity: 0; }
}

@keyframes shimmer {
  0%   { background-position: -200px 0; }
  100% { background-position:  200px 0; }
}
```

- [ ] **Step 7.2 — Append reduced-motion override.**

```css
/* ── Reduced motion ─────────────────────────────────────────────── */
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 0ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0ms !important;
    scroll-behavior: auto !important;
  }
  /* Opacity-only fallbacks for specific ambient animations. */
  @keyframes willowPulse { 0%, 100% { opacity: 1; transform: none; } }
  @keyframes willow-pop-in { from { opacity: 0; transform: none; } to { opacity: 1; transform: none; } }
  @keyframes leafFall { 0%, 100% { opacity: 0; transform: none; } }
}
```

- [ ] **Step 7.3 — Commit.**

```bash
git add crates/web/foundation.css
git commit -m "ui(phase-0): add named keyframes + reduced-motion override"
```

---

## Task 8: Density classes — `#app-root.density-*` swap `--msg-pad`

**Files:**
- Modify: `crates/web/foundation.css`
- Modify: `crates/web/src/app.rs`

- [ ] **Step 8.1 — Add density rules to foundation.css.**

```css
/* ── Density ────────────────────────────────────────────────────── */
:root { --msg-pad: 8px 24px; }                      /* balanced (default) */
#app-root.density-cozy    { --msg-pad: 10px 24px; } /* breathing room */
#app-root.density-balanced{ --msg-pad:  8px 24px; }
#app-root.density-dense   { --msg-pad:  4px 24px; } /* power-user scroll */
```

- [ ] **Step 8.2 — Set `id="app-root"` + default density on the Leptos root container in `app.rs`.**

Open `crates/web/src/app.rs`, locate the outermost view in the main `App` component, and add an id + class:

```rust
view! {
    <div id="app-root" class="density-balanced" data-accent="moss">
        // ... existing content
    </div>
}
```

If there are multiple top-level views (welcome / join / main / call) each rendered directly as the app root, wrap their common parent with the id + class attribute. Find by grepping `mount_to_body` or the outermost `view!` invocation in `fn App()`.

- [ ] **Step 8.3 — Run WASM build + browser test.**

```bash
just check-wasm
```

Expected: pass.

- [ ] **Step 8.4 — Commit.**

```bash
git add crates/web/foundation.css crates/web/src/app.rs
git commit -m "ui(phase-0): add density classes and id=\"app-root\" on Leptos container"
```

---

## Task 9: Accent variant overrides (moss / willow / amber / dusk / cedar / lichen / ember)

**Files:**
- Modify: `crates/web/foundation.css`

- [ ] **Step 9.1 — Append accent variant blocks.**

```css
/* ── Accent variants — overwrite --moss-*, --willow only ───────── */

[data-accent="moss"] {
  --moss-0: #2a3a28; --moss-1: #425c3d; --moss-2: #6a8d5e;
  --moss-3: #93b582; --moss-4: #c3d8b2; --willow: #b8c67a;
}
[data-accent="willow"] {
  --moss-0: #353b22; --moss-1: #5a663a; --moss-2: #a7b86a;
  --moss-3: #c1d18a; --moss-4: #e4ecb8; --willow: #b8c67a;
}
[data-accent="amber"] {
  --moss-0: #3a2c18; --moss-1: #7a5a2e; --moss-2: #c99b55;
  --moss-3: #e0b57a; --moss-4: #f2d8a8; --willow: #c99b55;
}
[data-accent="dusk"] {
  --moss-0: #2d2438; --moss-1: #5a4a72; --moss-2: #a88fc9;
  --moss-3: #c2adda; --moss-4: #e0d4ef; --willow: #c2adda;
}
[data-accent="cedar"] {
  --moss-0: #2e1e10; --moss-1: #5e3e20; --moss-2: #a47848;
  --moss-3: #c99b55; --moss-4: #e6c08a; --willow: #d0a878;
}
[data-accent="lichen"] {
  --moss-0: #18302a; --moss-1: #2e5b4c; --moss-2: #5a9580;
  --moss-3: #86c1ae; --moss-4: #b8dccc; --willow: #89b6a5;
}
[data-accent="ember"] {
  --moss-0: #3a1a10; --moss-1: #6e3722; --moss-2: #b5644a;
  --moss-3: #d98a68; --moss-4: #f2b89e; --willow: #d48a6a;
}
```

Note: `--whisper`, `--ok`, `--warn`, `--err`, `--amber`, `--amber-soft`, all `--ink-*`, all `--bg-*`, and `--line*` are **not** accent-swappable.

- [ ] **Step 9.2 — Commit.**

```bash
git add crates/web/foundation.css
git commit -m "ui(phase-0): add accent variant overrides (moss/willow/amber/dusk/cedar/lichen/ember)"
```

---

## Task 10: Scrollbar + selection + focus-visible defaults

**Files:**
- Modify: `crates/web/foundation.css`

- [ ] **Step 10.1 — Append scrollbar + selection + focus rules.**

```css
/* ── Scrollbar (Firefox + WebKit) ───────────────────────────────── */
.scroll {
  overflow-y: auto;
  scrollbar-width: thin;
  scrollbar-color: var(--bg-3) transparent;
}
.scroll::-webkit-scrollbar       { width: 8px; }
.scroll::-webkit-scrollbar-thumb { background: var(--bg-3); border-radius: 4px; }
.scroll::-webkit-scrollbar-track { background: transparent; }

@media (max-width: 720px) {
  .scroll::-webkit-scrollbar { width: 6px; }
}

.noscroll::-webkit-scrollbar { width: 0; height: 0; }
.noscroll                    { scrollbar-width: none; }

/* ── Selection ──────────────────────────────────────────────────── */
::selection { background: var(--moss-1); color: var(--ink-0); }

/* ── Focus-visible (accessibility baseline) ─────────────────────── */
:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}

/* Composer + message bodies never trap focus. */
textarea:focus-visible,
[contenteditable]:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}
```

- [ ] **Step 10.2 — Commit.**

```bash
git add crates/web/foundation.css
git commit -m "ui(phase-0): add scrollbar, selection, focus-visible defaults"
```

---

## Task 11: State helpers — empty, loading (shimmer skeleton), error

**Files:**
- Modify: `crates/web/foundation.css`

- [ ] **Step 11.1 — Append reusable state classes that components can opt in to.**

```css
/* ── States: empty / loading / error / skeleton ─────────────────── */

/* Empty state shell. Components fill their own copy; this just
   spaces the two-line pattern (italic Fraunces + Plex Sans hint). */
.state-empty {
  display: flex; flex-direction: column;
  align-items: center; justify-content: center;
  gap: 6px;
  padding: 32px 24px;
  color: var(--ink-1);
  font-family: var(--font-ui);
}
.state-empty__headline {
  font-family: var(--font-display);
  font-style: italic;
  font-size: 17px;
  color: var(--ink-1);
}
.state-empty__hint {
  font-size: 13px;
  color: var(--ink-3);
  max-width: 36ch;
  text-align: center;
}

/* Skeleton block — structural preview of content-about-to-render.
   Uses shimmer keyframe from Task 7. Reduced-motion path auto
   disables via the media query in Task 7. */
.state-skeleton {
  background:
    linear-gradient(90deg,
      var(--bg-2) 0%,
      var(--bg-3) 40%,
      var(--bg-2) 80%);
  background-size: 400px 100%;
  background-repeat: no-repeat;
  animation: shimmer 1.6s linear infinite;
  border-radius: var(--radius-s);
  min-height: 12px;
}

/* Inline loading hint (use where a skeleton would be visually noisy). */
.state-loading-inline {
  font-family: var(--font-mono);
  font-size: 12px;
  font-style: italic;
  color: var(--ink-3);
}
.state-loading-inline::after {
  content: '…';
}

/* Error card — small, scoped, carries a recovery action. */
.state-error {
  display: inline-flex; align-items: flex-start; gap: 10px;
  padding: 12px 14px;
  background: color-mix(in oklab, var(--err) 12%, var(--bg-1));
  border: 1px solid color-mix(in oklab, var(--err) 40%, var(--line));
  border-radius: var(--radius);
  color: var(--ink-1);
  font-family: var(--font-ui);
  font-size: 13px;
}
.state-error__icon { color: var(--err); flex: none; }
```

- [ ] **Step 11.2 — Commit.**

```bash
git add crates/web/foundation.css
git commit -m "ui(phase-0): add state helpers (empty / loading / skeleton / error)"
```

---

## Task 12: Universal reset + base typography

**Files:**
- Modify: `crates/web/foundation.css`

- [ ] **Step 12.1 — Append a minimal reset + default typography in foundation.css.**

```css
/* ── Universal reset ─────────────────────────────────────────────── */
* { box-sizing: border-box; }

/* Inherit on form controls so type stacks are consistent. */
button, input, textarea, select {
  font: inherit;
  color: inherit;
}
button { background: none; border: none; padding: 0; cursor: pointer; }
a { color: inherit; }

/* Exclamation marks are prohibited in copy per foundation §Copy voice —
   no enforcement in CSS, but noted here for lint tooling. */
```

`style.css` already has `* { margin: 0; padding: 0; box-sizing: border-box; }`; leaving that in place is fine. The rules above are additive and non-conflicting.

- [ ] **Step 12.2 — Commit.**

```bash
git add crates/web/foundation.css
git commit -m "ui(phase-0): add universal reset + form-control inherit"
```

---

## Task 13: Remap `style.css` legacy tokens to alias foundation tokens

**Files:**
- Modify: `crates/web/style.css`

**Purpose:** every existing component keeps using `--bg-main`, `--accent`, etc. without touching its Rust. After this task, those names resolve to the foundation palette.

- [ ] **Step 13.1 — Rewrite the `:root, [data-theme="dark"]` block to alias foundation tokens.**

Replace lines 6–43 of `crates/web/style.css` with:

```css
:root, [data-theme="dark"] {
    /* Every legacy token below aliases a foundation token from
       foundation.css. This is a one-way migration step: components
       keep their old names; the palette moves under them. */
    --bg-main:           var(--bg-0);
    --bg-sidebar:        var(--bg-1);
    --bg-input:          var(--bg-2);
    --bg-message-hover:  var(--bg-2);
    --bg-server-rail:    var(--bg-0);
    --bg-elevated:       var(--bg-1);

    --text-primary:      var(--ink-1);
    --text-secondary:    var(--ink-2);
    --text-muted:        var(--ink-3);
    --text-placeholder:  var(--ink-4);

    --accent:            var(--moss-2);
    --accent-hover:      var(--moss-3);
    --accent-glow:       color-mix(in oklab, var(--moss-2) 18%, transparent);
    --accent-subtle:     color-mix(in oklab, var(--moss-2) 8%,  transparent);

    --accent-green:      var(--ok);
    --accent-green-hover:color-mix(in oklab, var(--ok) 80%, black);
    --accent-green-glow: color-mix(in oklab, var(--ok) 18%, transparent);

    --danger:            var(--err);
    --danger-hover:      color-mix(in oklab, var(--err) 80%, black);
    --danger-glow:       color-mix(in oklab, var(--err) 18%, transparent);

    --online:            var(--moss-2);
    --unread:            var(--amber);

    --border:            var(--line);
    --border-subtle:     var(--line-soft);

    --scrollbar-thumb:   var(--bg-3);
    --scrollbar-track:   transparent;

    --shadow-sm:         0 1px 2px rgba(0, 0, 0, 0.2);
    --shadow-md:         0 4px 12px rgba(0, 0, 0, 0.3);
    --shadow-lg:         0 8px 32px rgba(0, 0, 0, 0.4);
    --shadow-inset:      inset 0 1px 3px rgba(0, 0, 0, 0.2);

    /* Keep legacy focus-ring shape but re-seat on the foundation token. */
    --focus-ring:        var(--focus-ring, 0 0 0 2px var(--moss-1), 0 0 0 3px rgba(106, 141, 94, 0.6));

    --overlay-bg:        rgba(0, 0, 0, 0.6);
    --backdrop-blur:     blur(4px);

    --transition-fast:   var(--motion-fast);
    --transition-normal: var(--motion);
    --transition-smooth: var(--motion-slow) var(--motion-ease);
}
```

Leave the `[data-theme="light"]` block (lines 47–84) unchanged — light theme is deferred per foundation.md; its hardcoded values remain in place and will be reworked when light theme ships.

- [ ] **Step 13.2 — Add a monospace alias so legacy `'IBM Plex Mono'` references resolve.**

After the `:root, [data-theme="dark"]` block, add a helper stanza that reroutes legacy monospace usage to the now-loaded JetBrains Mono:

```css
/* Legacy alias — IBM Plex Mono is no longer loaded; route to JetBrains Mono
   until components migrate to --font-mono directly. */
code, pre, .peer-id-text, .invite-code-display textarea, .welcome-invite-input {
    font-family: var(--font-mono) !important;
}
```

- [ ] **Step 13.3 — Build and visually verify the reskin.**

Run `just dev`. Open the app. Expected:
- backgrounds shift to the deep-bark palette (`--bg-0` / `--bg-1` hexes);
- primary buttons and unread badges read as moss green, not Discord blurple;
- "danger" / destructive confirms read as `--err` (softer red-orange);
- typography unchanged (still IBM Plex Sans because `style.css` sets that directly);
- every existing interaction (send a message, open settings, toggle theme) still works — this is a pure reskin.

Capture a quick mental snapshot. If a component looks visibly broken (huge contrast failure, missing colour), note it in the PR description under "Phase 2 migration targets" — do not fix it in Phase 0.

- [ ] **Step 13.4 — Commit.**

```bash
git add crates/web/style.css
git commit -m "ui(phase-0): remap legacy style.css tokens to alias foundation palette"
```

---

## Task 14: Browser test — assert foundation tokens + accent swap

**Files:**
- Modify: `crates/web/tests/browser.rs`

- [ ] **Step 14.1 — Append a new test module at the bottom of `crates/web/tests/browser.rs`.**

```rust
// ── Foundation tokens (Phase 0) ─────────────────────────────────────────────

#[cfg(test)]
mod foundation_tokens {
    use super::*;

    fn computed_root_prop(prop: &str) -> String {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let root: web_sys::Element = document.document_element().unwrap();
        let style = window.get_computed_style(&root).unwrap().unwrap();
        style.get_property_value(prop).unwrap_or_default()
    }

    fn set_data_accent(value: &str) {
        let document = web_sys::window().unwrap().document().unwrap();
        let root: web_sys::Element = document.document_element().unwrap();
        root.set_attribute("data-accent", value).unwrap();
    }

    #[wasm_bindgen_test]
    fn foundation_palette_tokens_defined() {
        // Sanity — foundation.css is loaded and the core palette resolves.
        for var in [
            "--bg-0", "--bg-1", "--bg-2",
            "--ink-0", "--ink-1", "--ink-on-accent",
            "--moss-2", "--willow", "--whisper",
            "--amber", "--ok", "--warn", "--err",
            "--radius", "--shadow-2", "--focus-ring",
            "--font-display", "--font-ui", "--font-mono",
            "--motion", "--motion-ease",
        ] {
            let v = computed_root_prop(var);
            assert!(!v.trim().is_empty(), "foundation token {var} not defined");
        }
    }

    #[wasm_bindgen_test]
    fn legacy_bg_main_aliases_bg_0() {
        // style.css remaps --bg-main to var(--bg-0). Both must resolve to
        // the same computed hex.
        let bg_main = computed_root_prop("--bg-main");
        let bg_0    = computed_root_prop("--bg-0");
        assert_eq!(bg_main.trim(), bg_0.trim(), "legacy --bg-main drifted from --bg-0");
    }

    #[wasm_bindgen_test]
    fn data_accent_swap_changes_moss_2() {
        // Swap the accent attribute on document element and verify --moss-2
        // updates synchronously (CSS-only, no Rust side effects).
        set_data_accent("moss");
        let moss_default = computed_root_prop("--moss-2");
        set_data_accent("ember");
        let moss_ember = computed_root_prop("--moss-2");
        assert_ne!(
            moss_default.trim(), moss_ember.trim(),
            "accent swap did not change --moss-2"
        );
        // Restore moss so later tests are unaffected.
        set_data_accent("moss");
    }
}
```

- [ ] **Step 14.2 — Run the browser test suite.**

```bash
just test-browser
```

Expected: all existing tests still pass, plus three new tests in the `foundation_tokens` module pass.

If `just test-browser` isn't available in this environment, run the underlying command:

```bash
wasm-pack test --headless --firefox crates/web
```

- [ ] **Step 14.3 — Commit.**

```bash
git add crates/web/tests/browser.rs
git commit -m "ui(phase-0): browser test — foundation tokens + legacy alias + accent swap"
```

---

## Task 15: Full `just check` + visual smoke

**Files:** none (verification only)

- [ ] **Step 15.1 — Run the full check suite.**

```bash
just check
```

Expected: fmt clean, clippy clean (workspace-wide, warnings as errors), all Rust unit + integration tests pass, WASM build succeeds.

If anything fails, stop and fix. Do not proceed to PR open.

- [ ] **Step 15.2 — Launch the local dev stack.**

```bash
just dev
```

- [ ] **Step 15.3 — Visual smoke list.**

In a real browser at `http://localhost:8080`, walk through:

- [ ] Welcome / join-link screen renders; backgrounds read as deep bark, not grey-blue.
- [ ] After joining, server rail renders; active server tile uses moss accent, not blurple.
- [ ] Sidebar header + channel list legible; font families as expected (Fraunces not yet used by components — that's fine; only headings in future phases switch to Fraunces).
- [ ] Send a message; message body renders in IBM Plex Sans; timestamp resolves in computed style to a mono font (JetBrains Mono fallback).
- [ ] Open settings; toggle the theme (light remains blurple — expected, deferred).
- [ ] DevTools console is clean: no 404s, no missing-font warnings after first load.
- [ ] Test `prefers-reduced-motion`: toggle in devtools → Rendering → emulate prefers-reduced-motion: reduce. Presence dots and pulse animations freeze.
- [ ] Set `document.documentElement.setAttribute('data-accent', 'ember')` in devtools. Unread badges and buttons shift to ember orange. Revert with `'moss'`.

If any of the above fails, capture the issue (class name, screenshot, DOM) and log it under "Known Phase 2 targets" in the PR description. Phase 0 does not block on component-level regressions — it blocks only on build / test / cascading breakage.

- [ ] **Step 15.4 — If everything passes, commit the checkpoint.**

No content change; use an empty commit to mark the gate:

```bash
git commit --allow-empty -m "ui(phase-0): gate passed — foundation shell in place"
```

---

## Task 16: Phase 0 PR

**Files:** none (process task)

- [ ] **Step 16.1 — Push the branch.**

```bash
git push
```

(Branch already tracks `origin/design/ui-target-ux`.)

- [ ] **Step 16.2 — Open the PR.**

```bash
gh pr create --title "ui(phase-0): foundation shell — palette, typography, motion, density, states" --body "$(cat <<'EOF'
## Summary

- New `crates/web/foundation.css` defining every token, keyframe, and state helper from `docs/specs/2026-04-19-ui-design/foundation.md`.
- Loads Fraunces + IBM Plex Sans + JetBrains Mono from a single `@import`.
- Accent variants (moss/willow/amber/dusk/cedar/lichen/ember) wired via `[data-accent]`; swappable at runtime.
- Density classes (`cozy`/`balanced`/`dense`) wired via `#app-root.density-*`.
- Reduced-motion path + state helpers (empty / loading / skeleton / error).
- `style.css` legacy tokens remapped to alias foundation tokens — every existing component automatically reskins to the new moss/bark palette without any Rust edits.
- `#app-root` + default `data-accent="moss"` + `density-balanced` class on the Leptos root container so later phases can toggle accent / density.

## Scope

- In scope: `foundation.md` only.
- Out of scope: component CSS migration (uses legacy aliases), terminology edits, layout changes, new components, accent / density user-facing UI (Phase 5).

## Test plan

- [x] `just check` green (fmt + clippy + unit + integration + WASM).
- [x] `just test-browser` green, including the new `foundation_tokens` module.
- [x] `just dev` smoke: every screen renders, moss palette visible, no console errors.
- [x] Reduced-motion emulation pauses ambient animations.
- [x] `data-accent="ember"` swap visibly changes accent-coloured UI; reverting to `"moss"` restores default.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 16.3 — Record the PR URL.**

Paste the URL into the task tracker and wait for review. Do **not** begin Phase 1 work until the Phase 0 PR is merged.

---

## Post-phase notes for later phases

- A two-step welcome screen (name → tabbed create/join) shipped on top of Phase 0 on the `design/ui-target-ux` branch after Phase 0 closed. It is a subset of `docs/specs/2026-04-19-ui-design/onboarding.md` — see the `Shipped state — design/ui-target-ux` section at the top of that spec for the shipped scope, terminology, and gap vs the full six-step flow. When Phase 5 (onboarding) begins, read that section first; the full onboarding flow layers on top of what is already in `welcome.rs` / `add_server.rs`.
- Phase 2 work will migrate component CSS (sidebar, chat, settings, etc.) from `--bg-main` / `--accent` names to the foundation tokens directly, then delete the legacy alias block added in Task 13.
- The `[data-theme="light"]` block in `style.css` remains hardcoded. Rework in a dedicated "light theme" phase; not tracked here.
- Typography in components still uses hardcoded font-family strings in many selectors inside `style.css`. Phase 2 sweeps these to `var(--font-ui)` / `var(--font-display)` / `var(--font-mono)`.
- Copy-voice lint tooling (detect pejorative offline strings, uppercase chrome labels, stray exclamation marks) is an `Open question` in `foundation.md`; build it in its own phase when we have enough component-level copy to lint.

---

## Self-review checklist

Before starting implementation, scan this plan once more:

- [x] Every token in `foundation.md` §CSS variable registry appears in a task (Tasks 3–5 + 9).
- [x] Radial gradient body background: Task 6.
- [x] Named keyframes: Task 7.
- [x] Density: Task 8.
- [x] Accent variants: Task 9.
- [x] Scrollbar, selection, focus-visible: Task 10.
- [x] States (empty / loading / error / skeleton): Task 11.
- [x] Reset + form-control inherit: Task 12.
- [x] Legacy token alias: Task 13.
- [x] Test coverage: Task 14.
- [x] `just check` gate: Task 15.
- [x] PR open: Task 16.

No placeholders, no "TBD", no "similar to". Every step has exact paths, exact code, exact commands. Types are consistent: `foundation.css` defines `--moss-2`; the accent-swap test reads `--moss-2` by the same name; style.css remap aliases via `var(--moss-2)`.
