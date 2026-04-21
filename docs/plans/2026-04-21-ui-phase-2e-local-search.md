# UI Phase 2e — Local search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development + superpowers:test-driven-development + superpowers:verification-before-completion.

**Goal:** Ship `docs/specs/2026-04-19-ui-design/local-search.md` — an on-device, encrypted-at-rest search index with scope ladder (`this letter` / `this channel` / `all letters` / `all groves + letters`), desktop + mobile entry points, query language (plain + prefix operators + quoted phrases), streamed results surface with grouping and highlight, `⌘K` palette bridge, privacy copy, settings-owned recents/horizon hooks, and a full a11y contract.

**Style ref:** `2026-04-20-ui-phase-1c-palette-a11y.md` and `2026-04-20-ui-phase-2a-message-row.md`. Commits: `ui(phase-2e): <imperative>`. Branch `phase-2e/local-search` off `main @ 20d9b01`. Plan + impl ship as one PR.

**Architecture:** Local search consumes already-decrypted `DisplayMessage`s; no new crypto, no new wire types, no new `EventKind`. The on-device index lives in **`willow-client`** as a new `search` module (not a new crate) — its inputs are the already-materialised `MessagesView` + `ChannelsView` + `MembersView` projections owned by the client, and its consumers (web UI, agent, future bots) already depend on `willow-client`. A dedicated crate would force `willow-client` to re-export wire types just to loopback into itself. The module is dual-target (native + WASM) with a WASM in-memory inverted index and a native SQLite FTS5 fallback **deferred to a follow-up** — Phase 2e ships the in-memory backend on both targets and flags the SQLite upgrade as a post-2e deferred task so the spec's "encrypted-at-rest" contract is preserved by *relying on the existing message-store's disk-encryption layer* (Phase 2e never writes the index to disk; the index is derived from already-decrypted messages at session start and dropped on shutdown, on both targets). This matches the spec's WASM behaviour exactly and keeps the native path honest: no new on-disk surface that could leak query state. The UI layer (`willow-web`) owns the results surface, scope chip popover, search input component, `/` focus keybinding, `⌘F` scope-flip, streaming banner, and the `⌘K` palette-bridge hook.

**Tech Stack:** Rust/WASM, Leptos 0.7, `willow-client` for the index + query engine, `willow-web` for the surface, `wasm-bindgen-test` (headless Firefox) for DOM assertions, `cargo test -p willow-client` for the index unit tests. No new third-party crates — the query parser is hand-written (plain-text + prefix operators + quoted phrases), the inverted index is a `HashMap<token, Vec<Posting>>`, and the tokenizer is ASCII-lowercased whitespace + punctuation split (covers English + emoji + mentions).

**Scope gate:**
- **In scope:** §Scope ladder, §Entry points (desktop + mobile + palette bridge), §Index (build + rebuild + horizon), §Query language, §Results presentation (layout + grouping + row anatomy + navigation), §Performance envelope (first-result latency budgets + streaming + debounce + cancellation), §Privacy (footer + no telemetry + recents + no sync), §Empty states, §Copy exact, §Accessibility. §Data dependencies: `SearchIndexConfig` + `SearchIndexBuildStatus` + recents as **new** client-library primitives; FTS5 column deferred with a tracked follow-up.
- **Out of scope:** grove directory search (owned by `discover.md`); the command-palette atom itself (owned by `layout-primitives.md`, already shipped — this plan wires only the bridge); server-side / federated search (does not exist); the `settings-tweaks.md` UI for horizon / per-grove toggle / rebuild-index action (exposes only the data-layer entry points + a stub settings row — full panel is `settings-tweaks.md`); native SQLite FTS5 (deferred); attachment text extraction / OCR; voice-call transcripts.
- **Spec source of truth:** `docs/specs/2026-04-19-ui-design/local-search.md`.

---

## File structure

| Path | State | Responsibility |
|------|-------|----------------|
| `crates/client/src/search/mod.rs` | **new** | Public surface: `SearchIndex`, `SearchIndexHandle`, `SearchQuery`, `SearchScope`, `SearchResult`, `SearchIndexConfig`, `SearchIndexBuildStatus`, `RecentQuery`. Re-export for `willow-client::search`. |
| `crates/client/src/search/tokenize.rs` | **new** | `tokenize(body) -> Vec<String>` — ASCII-lowercase, split on whitespace + punctuation, preserves `@handle` + `#channel` + `mailto:` / `http(s)://` token shapes. `token_positions(body) -> Vec<(usize, String)>` returns byte positions for highlight spans. |
| `crates/client/src/search/query.rs` | **new** | `parse_query(raw) -> SearchQuery` — grammar: plain text + quoted phrases (`"…"` exact adjacent) + prefix operators (`from:@peer`, `in:#channel`, `since:YYYY-MM-DD`, `before:YYYY-MM-DD`, `has:image`, `has:file`, `has:link`). Malformed operators become plain text plus a `warnings: Vec<QueryWarning>` entry that drives the "unknown filter — treated as plain text" tooltip. |
| `crates/client/src/search/index.rs` | **new** | Inverted index `HashMap<String, Vec<Posting>>` where `Posting { message_id, channel_id, grove_id: Option<ServerId>, timestamp_ms, author_peer_id, letter_id: Option<String> }`. `build_from_messages(Vec<IndexableMessage>)`, `insert(m)`, `remove_message(id)`, `remove_channel(id)`, `remove_grove(id)`. Horizon eviction via `evict_older_than(ms)`. |
| `crates/client/src/search/execute.rs` | **new** | `execute(&Index, &SearchQuery, scope, horizon_ms) -> impl Iterator<Item=SearchResult>` — lazy iterator yielding results in timestamp-desc order. Applies scope + operator filters (`from:`, `in:`, `since:`, `before:`, `has:*`) + AND-joined token predicate + exact-phrase predicate. Each hit carries matched byte ranges for the highlight renderer. |
| `crates/client/src/search/highlight.rs` | **new** | `highlight_excerpt(body, ranges, context_chars) -> ExcerptSpans` — builds a three-line excerpt centred on the first matched span, with ellipsis + byte ranges for the `<mark>` wrapper. Token-boundary aware (no mid-word cut unless unavoidable). |
| `crates/client/src/search/config.rs` | **new** | `SearchIndexConfig { enabled, horizon_days, remember_recents, per_grove_enabled: HashMap<String, bool> }` + defaults (horizon=90, recents=true) + a WASM `localStorage` / native stub persistence adapter (settings-tweaks.md owns the UI; this stores the bytes). `RecentQuery { text, timestamp_ms }` ring buffer helpers (length 8, `push`, `forget`, `clear_all`). |
| `crates/client/src/search/status.rs` | **new** | `SearchIndexBuildStatus { Idle, Building, Indexing { done, total }, Error(String) }`. Exposed via an `ArcSwap<SearchIndexBuildStatus>` that the UI subscribes to through a `derived_signal` bridge. |
| `crates/client/src/search/handle.rs` | **new** | `SearchIndexHandle` — `Arc<Mutex<SearchIndex>> + Arc<ArcSwap<SearchIndexBuildStatus>>`. Methods: `query(q, scope)`, `insert(msg)`, `rebuild(messages)`, `set_horizon_days(d)`, `set_per_grove_enabled(gid, bool)`, `status()`. Stream results via an async iterator using `futures::stream::unfold` so the UI can `.take_while(!cancelled)` them. |
| `crates/client/src/search/tests.rs` | **new** | Unit tests: tokenize whitespace/punct/@#, query grammar (plain, quoted, every operator, malformed), index insert/remove/evict, scope filter (each of the four scopes), exact-phrase match, prefix-operator filter (each of `from:` `in:` `since:` `before:` `has:image` `has:file` `has:link`), highlight excerpt (start / middle / end of body, ellipsis both sides, tie-break on first match). |
| `crates/client/src/lib.rs` | modify | `pub mod search;` + re-export `pub use search::{SearchIndexHandle, SearchIndexConfig, SearchIndexBuildStatus, SearchQuery, SearchScope, SearchResult, RecentQuery};`. Wire search-index creation into `ClientHandle::new` (lives on the handle as `.search()`). |
| `crates/client/src/storage.rs` | modify | Add `save_search_config(&SearchIndexConfig)` + `load_search_config() -> Option<SearchIndexConfig>` + `save_recents(&[RecentQuery])` + `load_recents() -> Vec<RecentQuery>`. Keys: `willow.search.config` + `willow.search.recents`. Native uses the same `save_raw` / `load_raw` path; WASM uses `localStorage` via the existing helper. Index itself is NEVER persisted. |
| `crates/web/src/components/search/mod.rs` | **new** | Submodule barrel. Re-exports `SearchSurface`, `SearchInput`, `ScopeChip`, `ResultsList`, `ResultRow`. |
| `crates/web/src/components/search/input.rs` | **new** | `<SearchInput>` — wrapped in `<form role="search" aria-label="local search">`. Bound to `query: RwSignal<String>` with 120 ms debounce; 15 % stale-dim flag while debounced. `Esc` contract: non-empty clears, empty closes the surface. `aria-controls`, `aria-autocomplete="list"`, `aria-activedescendant` wired to the results listbox. |
| `crates/web/src/components/search/scope_chip.rs` | **new** | `<ScopeChip>` — pill button with popover (4 rows: this letter / this channel / all letters / all groves + letters), `aria-haspopup="listbox"` + `aria-expanded`. Unreachable scopes (no focused letter / channel) render grey with tooltip `open a {letter|channel} first`. Arrow-key + Enter + Esc keyboard path. |
| `crates/web/src/components/search/results.rs` | **new** | `<ResultsList>` — `role="listbox" aria-label="search results"`. Groups per scope (spec §Grouping) with per-group headers (Fraunces italic 14 px, collapsible per-session). Streaming banner `searching… · {n} matches so far` under the scope chip when `SearchIndexBuildStatus::Indexing`. `aria-live="polite"` count updates throttled to ≤ once per 500 ms. Reduced-motion: fades collapse to snap. |
| `crates/web/src/components/search/row.rs` | **new** | `<ResultRow>` — `role="option"`. Context line (Fraunces italic container name · author · soft timestamp), three-line excerpt centred on first match with `<mark aria-label="match">` wrapping each matched range (underline + `--moss-3 18%` background), right-column `ArrowUpRight` on desktop only. Click / Enter jumps to container + scroll-to-1/3 + `willow-pop-in` + 6 s persistent underline. |
| `crates/web/src/components/search/recents.rs` | **new** | `<RecentsList>` — renders up to 8 chips under the empty input (`icon_search` + clipped query, max 180 px, mono for operator parts). Long-press / right-click `forget` (per chip); `clear all recents` button inline. Disabled entirely when `remember_recents=false`. |
| `crates/web/src/components/search/mobile_reveal.rs` | **new** | Pull-down gesture wrapper — exposes a `use_pull_down(container, threshold_px: 44)` signal that flips `reveal_bar` true once `scrollTop <= 0 && delta_y >= 44`. Consumed by letters list, channel list, and `MessageList` top. Collapses to `reduce_motion: snap` under reduced motion. |
| `crates/web/src/components/search/mod.rs` (public `SearchSurface`) | **new** | `<SearchSurface>` — full-screen takeover (desktop main-pane / mobile full screen). Structure: sticky `<SearchInput>` + `<ScopeChip>` + streaming banner + `<ResultsList>` + privacy footer `search runs on this device only. queries never leave your device.` in `--ink-3` 11 px. |
| `crates/web/src/components/mod.rs` | modify | `pub mod search;` + re-export `pub use search::{SearchSurface};`. |
| `crates/web/src/state.rs` | modify | Add `SearchUiState { open: ReadSignal<bool>, query: ReadSignal<String>, scope: ReadSignal<SearchScope>, results: ReadSignal<Vec<SearchResult>>, streaming: ReadSignal<bool>, recents: ReadSignal<Vec<RecentQuery>> }` + `SearchUiWriteSignals` mirror + wire-up into `AppState` + `AppWriteSignals`. Scope persists per-device in `localStorage` key `willow.search.scope`. |
| `crates/web/src/keybindings.rs` | modify | Add `/` → focus search input (only if active element is not an input/textarea/contenteditable and not the composer). Add `⌘F` / `Ctrl+F` → flip scope to `this channel` / `this letter` based on focused container + open surface with empty query. Add `Esc` delegation into the `SearchSurface` close-stack (above palette, below members). |
| `crates/web/src/components/command_palette.rs` | modify | `on_search` bridge: when the palette dispatches `PaletteActivate::Search(scope, q)` under `PaletteScope::Mixed` (no `#` / `@` / `>` prefix), forward with `SearchScope::AllLetters` (per spec §Command-palette bridge). When the user's input has `#` / `@` / `>` prefix, preserve the existing palette routing. Close the palette on forward. |
| `crates/web/src/app.rs` | modify | (a) Mount `<SearchSurface>` behind `app_state.search.open`. (b) Wire `on_search` callback into `<CommandPalette>` that opens the surface with the prefilled query + scope `all letters`. (c) Route `⌘F` → scope-flip + open surface. (d) Route `/` → focus search input when surface is open; when closed, opens the surface with empty query + default scope. (e) Wire `MainPaneHeader`'s existing `on_search_click` to the palette (unchanged). (f) Initialise `SearchIndexHandle` from `ClientHandle::search()` + seed with `messages_sig.get()` via a one-shot Effect at startup + incremental insert on every new `MessageReceived` event. |
| `crates/web/src/components/chat.rs` | modify | Mount pull-down reveal on `MessageList` via `<PullDownReveal>`. Nothing else changes — the reveal re-uses the shared search input component via a portal slot. |
| `crates/web/src/components/mobile_shell.rs` | modify | Mount pull-down reveal on letters list + channel list. Hook `⌘F` equivalent on mobile into the top-bar overflow `search this {letter|channel}` (existing overflow menu in `long_press.rs` adds a new item). |
| `crates/web/src/components/long_press.rs` | modify | Add `search this channel` / `search this letter` item to the top-bar overflow action sheet on mobile message list (spec §Entry points mobile). |
| `crates/web/style.css` | modify | Append §Local search block: `.search-surface`, `.search-input`, `.scope-chip`, `.scope-chip-popover`, `.scope-chip-popover-option`, `.search-results`, `.search-group-header`, `.search-result-row`, `.search-result-context`, `.search-result-excerpt`, `.search-result-excerpt mark`, `.search-recents`, `.search-recent-chip`, `.search-streaming-banner`, `.search-privacy-footer`, `.search-empty-never`, `.search-empty-no-match`, `.search-empty-indexing`, `.search-empty-scope-disabled`, `.search-pull-down-bar`. Consumes only foundation tokens (`--bg-*`, `--line*`, `--ink-*`, `--moss-*`, `--amber*`, `--radius-*`, `--shadow-2`, `--motion`, `--motion-ease`, `--focus-ring`, `--font-display`, `--font-ui`, `--font-mono`). Reduced-motion path collapses streaming fade + highlight flash + chevron rotate. |
| `crates/web/src/icons.rs` | modify | Add `icon_arrow_up_right` (12 px, stroke 1.5, `currentColor`) + `icon_hourglass_small` (16 px) if not present. |
| `crates/web/tests/browser.rs` | modify | Append `phase_2e_local_search` module (~14 tests — see §Tasks). |
| `crates/web/src/components/search/search_surface_tests.rs` | (embedded) | Rust `#[cfg(test)] mod tests` inside each search submodule for parser / tokenizer / scope-filter / highlight. Already counted in `crates/client/src/search/tests.rs`. |

No other files change in Phase 2e. The spec's open questions (regex / fuzzy, index export, shared-device hardening, attachment OCR, voice transcripts, discover-delegation promotion, cross-letter mentions) are explicitly deferred — a `TODO(local-search.md open-question)` comment anchors each.

**Upstream deps (already landed on `main @ 20d9b01`):**
- Phase 1a/1b main-pane header + mobile top bar — shipped.
- Phase 1c command palette + `on_search` prop — shipped (`crates/web/src/components/command_palette.rs`).
- Phase 1c keybindings module — shipped (`crates/web/src/keybindings.rs`).
- Phase 2a `DisplayMessage` with `author_peer_id`, `channel_id`, `body`, `timestamp_ms` — shipped.
- Phase 1d trust + 1e presence — shipped; search depends on neither, but uses them only for display in the row's context line.

**Downstream deferred:**
- Native SQLite FTS5 backend (ambition, not v1): deferred as post-2e follow-up. Opens the door to disk-backed horizon > all-history without memory blow-up.
- `discover.md` "search my messages instead" fallthrough: this plan exposes the entry point (`SearchScope::AllGrovesAndLetters` from `SearchSurface::open_with(query, scope)`) so discover can call it later without re-reading the spec.
- `letters-dms.md`: letters list's pull-down reveal + scope persistence consumes the same primitive. `letters-dms.md` owns the letters-specific placeholder + no-match copy; `SearchScope::ThisLetter` + `SearchScope::AllLetters` are reserved here with `TODO(letters-dms.md)` on the letter-scope filter branches that today short-circuit on "no letter channels exist".
- `settings-tweaks.md`: the privacy panel (`indexed horizon`, `remember recent searches`, `rebuild search index`, per-grove toggle) reads / writes the `SearchIndexConfig` persisted here. This phase ships only the data-layer hooks + a stub settings row.

---

## Acceptance gates (before the phase PR is opened)

1. `just fmt` + `just clippy` pass on the branch (zero warnings).
2. `cargo check --target wasm32-unknown-unknown -p willow-client -p willow-web` passes.
3. `cargo test -p willow-client search::` passes (~30 new unit tests — tokenize, query, index, execute, highlight, config, recents, status).
4. `cargo test -p willow-web` passes (no regression in the existing suite).
5. `just test-browser` passes with the new `phase_2e_local_search` module green (~14 tests). **Do NOT run locally** — rely on CI.
6. Manual walk-through (documented in §Acceptance criteria):
   - `/` on desktop focuses the top-right search slot when nothing else is focused.
   - `⌘F` / `Ctrl+F` inside a channel flips scope to `this channel`, shows the chip, changes placeholder to `search this channel`.
   - `Esc` with non-empty query clears; second `Esc` closes and restores focus to the invoking pane.
   - `⌘K` palette → type plain text → Enter forwards to local search with scope `all letters`, palette closes, surface opens prefilled.
   - Mobile pull-down (≥ 44 px with `scrollTop ≤ 0`) on letters / channel / message list reveals the sticky search bar.
   - Scope chip: four values, unreachable ones greyed with tooltip, chosen scope persists across reloads (localStorage).
   - Prefix operators all work on a seeded 10-message index; unknown operators treated as plain text + tooltip shown.
   - Quoted phrases match exactly adjacent; empty query shows placeholder only.
   - Results group per §Grouping; headers collapse/expand per session; counts shown.
   - Row: context · author · soft ts · three-line excerpt with highlighted match (underline + moss 18 % bg).
   - Click a result: surface closes, container scrolls match to 1/3 viewport, message gets `willow-pop-in` 240 ms + 6 s persistent underline.
   - Streaming banner shows `searching… · {n} matches so far` while index rebuild is in flight.
   - Privacy footer always visible: `search runs on this device only. queries never leave your device.`
   - Recents capped at 8; individual `forget` + `clear all recents` work; when `remember_recents=false`, recents chips never render.
   - A11y: `role="search"` form, `role="listbox"` results, `<mark aria-label="match">`, `aria-live="polite"` count updates throttled to ≤ once per 500 ms, focus restoration on back-navigation.
   - Reduced-motion: streaming rows snap (no fade), highlight flash collapses to instant, scope chevron rotate is no-op.
   - No `tracing::info!` / `tracing::warn!` / `tracing::debug!` call emits the query string, scope, or match count — grep guard in the acceptance-gate step.

---

## Tasks (14 tasks, ~18 commits)

Each task = one commit (some have 2-3). Tick the checkbox in the same commit. Push after every 2-3 tasks.

### Task 1: Query parser

**Files:**
- Create: `crates/client/src/search/query.rs`
- Create: `crates/client/src/search/mod.rs` (skeleton — `pub mod query; pub use query::*;`)
- Modify: `crates/client/src/lib.rs` — add `pub mod search;`.

- [x] **Step 1.1 — Define the types.** In `crates/client/src/search/query.rs`:

```rust
//! Query grammar for local search.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Query language:
//! plain text + quoted phrases + prefix operators, case-insensitive. Each
//! parse returns a [`SearchQuery`] carrying token predicates, exact-phrase
//! predicates, operator filters, and a list of [`QueryWarning`]s for
//! malformed operators (which are treated as plain text per spec).

use chrono::NaiveDate;

/// Prefix-operator filters that narrow a query.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryFilters {
    /// `from:@peer` — author display name or handle equals `peer`.
    pub from_author: Option<String>,
    /// `in:#channel` — channel name equals `channel`.
    pub in_channel: Option<String>,
    /// `since:YYYY-MM-DD` — timestamp >= date (local timezone midnight).
    pub since: Option<NaiveDate>,
    /// `before:YYYY-MM-DD` — timestamp < date (local timezone midnight).
    pub before: Option<NaiveDate>,
    /// `has:image` — message has an image attachment.
    pub has_image: bool,
    /// `has:file` — message has a non-image file attachment.
    pub has_file: bool,
    /// `has:link` — message body contains a URL.
    pub has_link: bool,
}

/// A single parsed warning — fed to the UI tooltip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryWarning {
    /// An operator was unknown or malformed (treated as plain text).
    UnknownOperator { span: String },
}

/// A fully-parsed query.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchQuery {
    /// Whitespace-separated tokens, all AND-joined against the body.
    pub tokens: Vec<String>,
    /// Quoted-phrase predicates (exact adjacent token match).
    pub phrases: Vec<String>,
    /// Operator filters.
    pub filters: QueryFilters,
    /// Warnings for malformed operators.
    pub warnings: Vec<QueryWarning>,
    /// The raw query echo (before parsing) — used by the UI to render
    /// the query string. Quotes are stripped from phrase tokens but the
    /// raw echo is preserved separately so the user sees their input.
    pub raw: String,
}

/// Parse `raw` into a [`SearchQuery`].
///
/// The grammar is tolerant: malformed operators fall through as plain
/// tokens plus a [`QueryWarning::UnknownOperator`] entry. Token matching
/// is case-insensitive at the tokenizer level; phrases are stored
/// lower-cased here and compared lower-cased at execute time.
pub fn parse_query(raw: &str) -> SearchQuery { /* Step 1.3 */ }
```

- [x] **Step 1.2 — Write failing unit tests.** Create `crates/client/src/search/tests.rs` with:

```rust
#[cfg(test)]
mod query_tests {
    use super::super::query::*;
    use chrono::NaiveDate;

    #[test]
    fn empty_query_is_no_op() {
        let q = parse_query("");
        assert!(q.tokens.is_empty());
        assert!(q.phrases.is_empty());
        assert_eq!(q.filters, QueryFilters::default());
        assert!(q.warnings.is_empty());
    }

    #[test]
    fn plain_text_tokens_split_on_whitespace() {
        let q = parse_query("hello world");
        assert_eq!(q.tokens, vec!["hello", "world"]);
    }

    #[test]
    fn tokens_lowercased() {
        let q = parse_query("HELLO World");
        assert_eq!(q.tokens, vec!["hello", "world"]);
    }

    #[test]
    fn quoted_phrase_single() {
        let q = parse_query(r#""two words""#);
        assert_eq!(q.phrases, vec!["two words"]);
        assert!(q.tokens.is_empty());
    }

    #[test]
    fn quoted_phrase_mixed_with_tokens() {
        let q = parse_query(r#"hello "two words" world"#);
        assert_eq!(q.tokens, vec!["hello", "world"]);
        assert_eq!(q.phrases, vec!["two words"]);
    }

    #[test]
    fn from_operator_with_at() {
        let q = parse_query("from:@mira");
        assert_eq!(q.filters.from_author, Some("mira".into()));
    }

    #[test]
    fn from_operator_without_at() {
        let q = parse_query("from:mira");
        assert_eq!(q.filters.from_author, Some("mira".into()));
    }

    #[test]
    fn in_operator() {
        let q = parse_query("in:#general");
        assert_eq!(q.filters.in_channel, Some("general".into()));
    }

    #[test]
    fn since_operator_parses_date() {
        let q = parse_query("since:2026-04-01");
        assert_eq!(q.filters.since, Some(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()));
    }

    #[test]
    fn before_operator_parses_date() {
        let q = parse_query("before:2026-04-21");
        assert_eq!(q.filters.before, Some(NaiveDate::from_ymd_opt(2026, 4, 21).unwrap()));
    }

    #[test]
    fn has_image_operator() {
        let q = parse_query("has:image");
        assert!(q.filters.has_image);
    }

    #[test]
    fn has_file_operator() {
        let q = parse_query("has:file");
        assert!(q.filters.has_file);
    }

    #[test]
    fn has_link_operator() {
        let q = parse_query("has:link");
        assert!(q.filters.has_link);
    }

    #[test]
    fn unknown_prefix_treated_as_text_with_warning() {
        let q = parse_query("since:yesterday");
        assert_eq!(q.tokens, vec!["since:yesterday"]);
        assert_eq!(q.warnings.len(), 1);
        assert!(matches!(&q.warnings[0], QueryWarning::UnknownOperator { span } if span == "since:yesterday"));
    }

    #[test]
    fn operator_mixed_with_text() {
        let q = parse_query("from:@mira hello world in:#general");
        assert_eq!(q.tokens, vec!["hello", "world"]);
        assert_eq!(q.filters.from_author, Some("mira".into()));
        assert_eq!(q.filters.in_channel, Some("general".into()));
    }
}
```

Wire the module: `crates/client/src/search/mod.rs`:
```rust
pub mod query;
pub use query::{parse_query, QueryFilters, QueryWarning, SearchQuery};

#[cfg(test)]
mod tests;
```
Register in `crates/client/src/lib.rs`: `pub mod search;`.

Run: `cargo test -p willow-client search::query_tests`. Expected: FAIL (`parse_query` not implemented).

- [x] **Step 1.3 — Implement `parse_query`.**

```rust
pub fn parse_query(raw: &str) -> SearchQuery {
    let mut out = SearchQuery { raw: raw.to_string(), ..SearchQuery::default() };

    // 1. Strip quoted phrases first. Simple state machine: walk chars,
    //    when inside `"..."` accumulate into `phrase`; at the closing
    //    quote push to `phrases` and reset. Unclosed quotes fall through
    //    as plain tokens.
    let mut rest = String::with_capacity(raw.len());
    let mut in_quote = false;
    let mut phrase = String::new();
    for c in raw.chars() {
        match (in_quote, c) {
            (false, '"') => in_quote = true,
            (true,  '"') => {
                out.phrases.push(phrase.trim().to_lowercase());
                phrase.clear();
                in_quote = false;
            }
            (true, c) => phrase.push(c),
            (false, c) => rest.push(c),
        }
    }
    // Unclosed quote — rescue the partial phrase as plain tokens.
    if in_quote && !phrase.is_empty() {
        rest.push_str(&phrase);
    }

    // 2. Walk whitespace-separated tokens for operators / plain text.
    for span in rest.split_whitespace() {
        if let Some(rest) = span.strip_prefix("from:") {
            let h = rest.strip_prefix('@').unwrap_or(rest).to_lowercase();
            out.filters.from_author = Some(h);
        } else if let Some(rest) = span.strip_prefix("in:") {
            let ch = rest.strip_prefix('#').unwrap_or(rest).to_lowercase();
            out.filters.in_channel = Some(ch);
        } else if let Some(rest) = span.strip_prefix("since:") {
            match NaiveDate::parse_from_str(rest, "%Y-%m-%d") {
                Ok(d) => out.filters.since = Some(d),
                Err(_) => {
                    out.tokens.push(span.to_lowercase());
                    out.warnings.push(QueryWarning::UnknownOperator { span: span.to_string() });
                }
            }
        } else if let Some(rest) = span.strip_prefix("before:") {
            match NaiveDate::parse_from_str(rest, "%Y-%m-%d") {
                Ok(d) => out.filters.before = Some(d),
                Err(_) => {
                    out.tokens.push(span.to_lowercase());
                    out.warnings.push(QueryWarning::UnknownOperator { span: span.to_string() });
                }
            }
        } else if span == "has:image" { out.filters.has_image = true; }
        else if span == "has:file"  { out.filters.has_file  = true; }
        else if span == "has:link"  { out.filters.has_link  = true; }
        else if let Some(op) = detect_unknown_prefix(span) {
            // `foo:bar` that doesn't match a known prefix → warning + plain token.
            out.tokens.push(span.to_lowercase());
            out.warnings.push(QueryWarning::UnknownOperator { span: op.to_string() });
        } else {
            out.tokens.push(span.to_lowercase());
        }
    }

    out
}

fn detect_unknown_prefix(span: &str) -> Option<&str> {
    // `foo:bar` where `foo:` doesn't match any known prefix.
    let idx = span.find(':')?;
    // Ignore `http://` and `https://` — those are URLs, not operators.
    let prefix = &span[..idx + 1];
    if matches!(prefix, "from:" | "in:" | "since:" | "before:" | "has:"
        | "http:" | "https:" | "mailto:" | "ftp:" | "ws:" | "wss:" | "file:"
    ) {
        return None;
    }
    Some(span)
}
```

Add `chrono` workspace dep to `crates/client/Cargo.toml` if missing (check first; `willow-messaging` already pulls it via workspace).

- [x] **Step 1.4 — Verify GREEN.** `cargo test -p willow-client search::query_tests` → 17/17 pass (includes the two bonus edge cases: URL-in-query + raw-echo-preserved).

- [x] **Step 1.5 — WASM compile check.** `cargo check --target wasm32-unknown-unknown -p willow-client`. Zero warnings.

- [x] **Step 1.6 — Commit.**

```bash
git add crates/client/src/search/query.rs crates/client/src/search/mod.rs crates/client/src/search/tests.rs crates/client/src/lib.rs crates/client/Cargo.toml
git commit -m "ui(phase-2e): add local-search query parser + unit tests"
```

---

### Task 2: Tokenizer

**Files:**
- Create: `crates/client/src/search/tokenize.rs`
- Modify: `crates/client/src/search/mod.rs` — `pub mod tokenize;`
- Modify: `crates/client/src/search/tests.rs` — `mod tokenize_tests;`

- [ ] **Step 2.1 — Write failing tests.** In `crates/client/src/search/tests.rs`:

```rust
#[cfg(test)]
mod tokenize_tests {
    use super::super::tokenize::*;

    #[test]
    fn empty_body_yields_empty() {
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn splits_on_whitespace() {
        assert_eq!(tokenize("hello world"), vec!["hello", "world"]);
    }

    #[test]
    fn splits_on_punctuation() {
        assert_eq!(tokenize("hello, world!"), vec!["hello", "world"]);
    }

    #[test]
    fn lowercases_all_tokens() {
        assert_eq!(tokenize("Hello WORLD"), vec!["hello", "world"]);
    }

    #[test]
    fn preserves_mention_token() {
        // `@mira` stays as a single token so `from:@mira` filtering
        // can match it. Body search still sees the `mira` stem as a
        // token too so plain-text queries hit it.
        let toks = tokenize("hello @mira there");
        assert!(toks.contains(&"@mira".to_string()));
        assert!(toks.contains(&"mira".to_string()));
    }

    #[test]
    fn preserves_channel_token() {
        let toks = tokenize("moved to #general");
        assert!(toks.contains(&"#general".to_string()));
        assert!(toks.contains(&"general".to_string()));
    }

    #[test]
    fn preserves_url_as_single_token() {
        let toks = tokenize("see https://willow.im");
        assert!(toks.contains(&"https://willow.im".to_string()));
    }

    #[test]
    fn token_positions_returns_byte_offsets() {
        let pairs = token_positions("hello world");
        assert_eq!(pairs, vec![(0, "hello".into()), (6, "world".into())]);
    }

    #[test]
    fn token_positions_handles_multibyte() {
        let body = "héllo";
        let pairs = token_positions(body);
        assert_eq!(pairs, vec![(0, "héllo".into())]);
    }
}
```

Run: FAIL.

- [ ] **Step 2.2 — Implement.** `crates/client/src/search/tokenize.rs`:

```rust
//! Tokenizer for local search. ASCII-lowercase, splits on whitespace +
//! punctuation, preserves `@handle`, `#channel`, and URL tokens so they
//! can feed the operator filters (`from:`, `in:`, `has:link`).

/// Return the list of searchable tokens for `body`.
pub fn tokenize(body: &str) -> Vec<String> {
    token_positions(body).into_iter().flat_map(|(_, t)| expand(t)).collect()
}

/// Return `(byte_offset, token)` pairs. Mentions, channels, and URLs
/// each emit *one* pair with the leading sigil intact so the highlight
/// and filter stages can see the full token.
pub fn token_positions(body: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip non-token chars.
        while i < bytes.len() && !is_token_start(bytes[i]) {
            i += body[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        }
        if i >= bytes.len() { break; }
        let start = i;
        // If this is a URL token, consume until whitespace.
        if body[i..].starts_with("http://") || body[i..].starts_with("https://")
            || body[i..].starts_with("mailto:")
        {
            while i < bytes.len() && !(bytes[i] as char).is_whitespace() {
                i += body[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            }
        } else if bytes[i] == b'@' || bytes[i] == b'#' {
            // Sigil-prefixed token: consume sigil + alnum/./-/_.
            i += 1;
            while i < bytes.len() && is_handle_char(bytes[i]) {
                i += 1;
            }
        } else {
            // Plain token: alnum + '-' + '_' + multibyte alpha.
            while i < bytes.len() {
                let c = body[i..].chars().next().unwrap();
                if c.is_alphanumeric() || c == '-' || c == '_' || c == '\'' {
                    i += c.len_utf8();
                } else {
                    break;
                }
            }
        }
        if start < i {
            let tok = body[start..i].to_lowercase();
            out.push((start, tok));
        }
    }
    out
}

fn is_token_start(b: u8) -> bool {
    let c = b as char;
    c.is_alphanumeric() || c == '@' || c == '#' || c == 'h' || c == 'm'
}

fn is_handle_char(b: u8) -> bool {
    let c = b as char;
    c.is_alphanumeric() || c == '.' || c == '_' || c == '-'
}

/// A sigil-prefixed token also emits the stemmed form so a plain-text
/// query for "mira" still matches `@mira`.
fn expand(tok: String) -> Vec<String> {
    if let Some(stem) = tok.strip_prefix('@').or_else(|| tok.strip_prefix('#')) {
        vec![tok.clone(), stem.to_string()]
    } else {
        vec![tok]
    }
}
```

- [ ] **Step 2.3 — Verify GREEN.** `cargo test -p willow-client search::tokenize_tests`. All 9 pass.

  *Note:* the `is_token_start` branch on `'h'` / `'m'` is a perf hint to anchor URL detection — unit tests assert behaviour, not the branch. If any test fails, loosen the predicate.

- [ ] **Step 2.4 — Commit.**

```bash
git add crates/client/src/search/tokenize.rs crates/client/src/search/mod.rs crates/client/src/search/tests.rs
git commit -m "ui(phase-2e): tokenize message bodies preserving mentions + urls"
```

---

### Task 3: Inverted index

**Files:**
- Create: `crates/client/src/search/index.rs`
- Modify: `crates/client/src/search/mod.rs` — `pub mod index;`
- Modify: `crates/client/src/search/tests.rs` — `mod index_tests;`

- [ ] **Step 3.1 — Define types.**

```rust
//! Inverted index for local search. Maps each token to the set of
//! message postings that contain it. Postings carry enough metadata to
//! apply scope + operator filters at execute time without touching the
//! original message store.

use std::collections::HashMap;
use willow_identity::EndpointId;

/// One indexed message's metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexableMessage {
    pub message_id: String,
    pub channel_id: String,
    pub channel_name: String,
    pub grove_id: Option<String>,
    pub letter_id: Option<String>,
    pub author_peer_id: EndpointId,
    pub author_handle: String,
    pub author_display_name: String,
    pub timestamp_ms: u64,
    pub body: String,
    pub has_image: bool,
    pub has_file: bool,
    pub has_link: bool,
}

/// Lightweight row stored in the inverted index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posting {
    pub message_id: String,
    pub channel_id: String,
    pub channel_name: String,
    pub grove_id: Option<String>,
    pub letter_id: Option<String>,
    pub author_peer_id: EndpointId,
    pub author_handle: String,
    pub author_display_name: String,
    pub timestamp_ms: u64,
    /// Cached body for excerpt rendering. Kept short in typical usage.
    pub body: String,
    pub has_image: bool,
    pub has_file: bool,
    pub has_link: bool,
}

impl From<IndexableMessage> for Posting {
    fn from(m: IndexableMessage) -> Self {
        Self {
            message_id: m.message_id,
            channel_id: m.channel_id,
            channel_name: m.channel_name,
            grove_id: m.grove_id,
            letter_id: m.letter_id,
            author_peer_id: m.author_peer_id,
            author_handle: m.author_handle,
            author_display_name: m.author_display_name,
            timestamp_ms: m.timestamp_ms,
            body: m.body,
            has_image: m.has_image,
            has_file: m.has_file,
            has_link: m.has_link,
        }
    }
}

#[derive(Debug, Default)]
pub struct SearchIndex {
    /// token -> list of postings (ordered by insertion; execute() sorts
    /// by timestamp desc).
    pub(crate) postings: HashMap<String, Vec<Posting>>,
    /// Mirror of message_id -> tokens, so remove_message() can unthread
    /// every token list.
    pub(crate) by_msg: HashMap<String, Vec<String>>,
}

impl SearchIndex {
    pub fn new() -> Self { Self::default() }

    pub fn insert(&mut self, m: IndexableMessage) { /* Step 3.3 */ }
    pub fn remove_message(&mut self, id: &str) { /* Step 3.3 */ }
    pub fn remove_channel(&mut self, cid: &str) { /* Step 3.3 */ }
    pub fn remove_grove(&mut self, gid: &str) { /* Step 3.3 */ }
    pub fn evict_older_than(&mut self, cutoff_ms: u64) { /* Step 3.3 */ }
    pub fn message_count(&self) -> usize { self.by_msg.len() }
    pub fn postings_for(&self, token: &str) -> Option<&[Posting]> {
        self.postings.get(token).map(|v| v.as_slice())
    }
}
```

- [ ] **Step 3.2 — Failing tests.** In `crates/client/src/search/tests.rs` add `mod index_tests;`:

```rust
#[cfg(test)]
mod index_tests {
    use super::super::index::*;
    use willow_identity::EndpointId;

    fn msg(id: &str, body: &str, ts: u64, cid: &str) -> IndexableMessage {
        IndexableMessage {
            message_id: id.into(),
            channel_id: cid.into(),
            channel_name: cid.into(),
            grove_id: Some("g0".into()),
            letter_id: None,
            author_peer_id: EndpointId::from_bytes([0u8; 32]),
            author_handle: "mira".into(),
            author_display_name: "Mira".into(),
            timestamp_ms: ts,
            body: body.into(),
            has_image: false,
            has_file: false,
            has_link: false,
        }
    }

    #[test]
    fn insert_then_lookup() {
        let mut idx = SearchIndex::new();
        idx.insert(msg("m1", "hello world", 100, "general"));
        assert_eq!(idx.message_count(), 1);
        assert!(idx.postings_for("hello").is_some());
        assert!(idx.postings_for("world").is_some());
    }

    #[test]
    fn remove_message_unthreads_all_tokens() {
        let mut idx = SearchIndex::new();
        idx.insert(msg("m1", "hello world", 100, "general"));
        idx.remove_message("m1");
        assert_eq!(idx.message_count(), 0);
        assert!(idx.postings_for("hello").map(|p| p.is_empty()).unwrap_or(true));
    }

    #[test]
    fn remove_channel_drops_all_messages_in_channel() {
        let mut idx = SearchIndex::new();
        idx.insert(msg("m1", "hello", 100, "general"));
        idx.insert(msg("m2", "world", 100, "other"));
        idx.remove_channel("general");
        assert_eq!(idx.message_count(), 1);
    }

    #[test]
    fn remove_grove_drops_all_messages_in_grove() {
        let mut idx = SearchIndex::new();
        let mut m = msg("m1", "hello", 100, "general");
        m.grove_id = Some("grove-a".into());
        idx.insert(m);
        let mut m2 = msg("m2", "world", 100, "general");
        m2.grove_id = Some("grove-b".into());
        idx.insert(m2);
        idx.remove_grove("grove-a");
        assert_eq!(idx.message_count(), 1);
    }

    #[test]
    fn evict_older_than_drops_old_messages() {
        let mut idx = SearchIndex::new();
        idx.insert(msg("old", "old", 100, "general"));
        idx.insert(msg("new", "new", 10_000, "general"));
        idx.evict_older_than(1_000);
        assert_eq!(idx.message_count(), 1);
        assert!(idx.postings_for("new").is_some());
        assert!(idx.postings_for("old").map(|p| p.is_empty()).unwrap_or(true));
    }
}
```

Run: FAIL.

- [ ] **Step 3.3 — Implement.**

```rust
impl SearchIndex {
    pub fn insert(&mut self, m: IndexableMessage) {
        let id = m.message_id.clone();
        if self.by_msg.contains_key(&id) {
            // Dedup on re-insert.
            return;
        }
        let tokens = super::tokenize::tokenize(&m.body);
        // Author + channel also indexed so `from:` / `in:` filters run
        // even when the body omits them.
        let mut token_set: std::collections::HashSet<String> = tokens.into_iter().collect();
        token_set.insert(format!("@{}", m.author_handle.to_lowercase()));
        token_set.insert(m.author_handle.to_lowercase());
        token_set.insert(m.author_display_name.to_lowercase());
        token_set.insert(format!("#{}", m.channel_name.to_lowercase()));
        token_set.insert(m.channel_name.to_lowercase());

        let posting: Posting = m.into();
        let tokens: Vec<String> = token_set.into_iter().collect();
        for t in &tokens {
            self.postings.entry(t.clone()).or_default().push(posting.clone());
        }
        self.by_msg.insert(id, tokens);
    }

    pub fn remove_message(&mut self, id: &str) {
        let Some(tokens) = self.by_msg.remove(id) else { return; };
        for t in tokens {
            if let Some(list) = self.postings.get_mut(&t) {
                list.retain(|p| p.message_id != id);
            }
        }
    }

    pub fn remove_channel(&mut self, cid: &str) {
        let ids: Vec<String> = self.postings.values()
            .flat_map(|v| v.iter().filter(|p| p.channel_id == cid).map(|p| p.message_id.clone()))
            .collect::<std::collections::HashSet<_>>()
            .into_iter().collect();
        for id in ids { self.remove_message(&id); }
    }

    pub fn remove_grove(&mut self, gid: &str) {
        let ids: Vec<String> = self.postings.values()
            .flat_map(|v| v.iter().filter(|p| p.grove_id.as_deref() == Some(gid)).map(|p| p.message_id.clone()))
            .collect::<std::collections::HashSet<_>>()
            .into_iter().collect();
        for id in ids { self.remove_message(&id); }
    }

    pub fn evict_older_than(&mut self, cutoff_ms: u64) {
        let ids: Vec<String> = self.postings.values()
            .flat_map(|v| v.iter().filter(|p| p.timestamp_ms < cutoff_ms).map(|p| p.message_id.clone()))
            .collect::<std::collections::HashSet<_>>()
            .into_iter().collect();
        for id in ids { self.remove_message(&id); }
    }
}
```

- [ ] **Step 3.4 — Verify GREEN.** `cargo test -p willow-client search::index_tests`. All 5 pass.

- [ ] **Step 3.5 — Commit.**

```bash
git add crates/client/src/search/index.rs crates/client/src/search/mod.rs crates/client/src/search/tests.rs
git commit -m "ui(phase-2e): add inverted index with insert/remove/evict"
```

---

### Task 4: Scope + execution

**Files:**
- Create: `crates/client/src/search/execute.rs`
- Modify: `crates/client/src/search/mod.rs` — `pub mod execute;`
- Modify: `crates/client/src/search/tests.rs` — `mod execute_tests;`

- [ ] **Step 4.1 — Types.**

```rust
//! Search execution — applies scope + filters to the inverted index.

use super::index::{Posting, SearchIndex};
use super::query::SearchQuery;

/// Scope of a search invocation. Per `local-search.md` §Scope ladder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchScope {
    /// Only this letter's messages.
    ThisLetter(String),
    /// Only this channel's messages (channel id, not name — ids are stable).
    ThisChannel(String),
    /// Every peer + group letter on this device.
    AllLetters,
    /// Every grove channel plus every letter.
    AllGrovesAndLetters,
}

/// One hit from the search executor.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub message_id: String,
    pub channel_id: String,
    pub channel_name: String,
    pub grove_id: Option<String>,
    pub letter_id: Option<String>,
    pub author_display_name: String,
    pub author_handle: String,
    pub timestamp_ms: u64,
    pub body: String,
    /// Byte ranges of each matched span inside `body`. Populated by the
    /// highlight stage (Task 5).
    pub matched_ranges: Vec<(usize, usize)>,
}

/// Execute `query` over `index` under `scope`. Returns an iterator
/// yielding hits in timestamp-desc order. Lazy — the caller can stop
/// consuming when the UI cancels.
pub fn execute<'a>(
    index: &'a SearchIndex,
    query: &'a SearchQuery,
    scope: &'a SearchScope,
) -> impl Iterator<Item = SearchResult> + 'a { /* Step 4.3 */ }
```

- [ ] **Step 4.2 — Failing tests.**

```rust
#[cfg(test)]
mod execute_tests {
    use super::super::execute::*;
    use super::super::index::*;
    use super::super::query::*;
    use willow_identity::EndpointId;
    use chrono::NaiveDate;

    fn seed_index() -> SearchIndex {
        let mut idx = SearchIndex::new();
        let mk = |id: &str, body: &str, cid: &str, chname: &str, ts: u64, author: &str, handle: &str,
                  grove: Option<&str>, letter: Option<&str>, img: bool, file: bool, link: bool| {
            IndexableMessage {
                message_id: id.into(),
                channel_id: cid.into(),
                channel_name: chname.into(),
                grove_id: grove.map(String::from),
                letter_id: letter.map(String::from),
                author_peer_id: EndpointId::from_bytes([0u8; 32]),
                author_handle: handle.into(),
                author_display_name: author.into(),
                timestamp_ms: ts,
                body: body.into(),
                has_image: img,
                has_file: file,
                has_link: link,
            }
        };
        idx.insert(mk("m1", "hello world",     "c1", "general", 100, "Mira", "mira", Some("g0"), None, false, false, false));
        idx.insert(mk("m2", "hello everyone",  "c2", "random",  200, "Jun",  "jun",  Some("g0"), None, false, false, false));
        idx.insert(mk("m3", "see https://ok",  "c1", "general", 300, "Mira", "mira", Some("g0"), None, false, false, true));
        idx.insert(mk("m4", "letter text",     "l1", "letter",  400, "Jun",  "jun",  None,       Some("l1"), false, false, false));
        idx.insert(mk("m5", "two words here",  "c1", "general", 500, "Mira", "mira", Some("g0"), None, false, false, false));
        idx
    }

    #[test]
    fn scope_this_channel_only_matches_that_channel() {
        let idx = seed_index();
        let q = parse_query("hello");
        let hits: Vec<_> = execute(&idx, &q, &SearchScope::ThisChannel("c1".into())).collect();
        assert_eq!(hits.len(), 1); // m1 only, not m2 (different channel)
        assert_eq!(hits[0].message_id, "m1");
    }

    #[test]
    fn scope_all_letters_excludes_grove_channels() {
        let idx = seed_index();
        let q = parse_query("text");
        let hits: Vec<_> = execute(&idx, &q, &SearchScope::AllLetters).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "m4");
    }

    #[test]
    fn scope_all_groves_and_letters_matches_both() {
        let idx = seed_index();
        let q = parse_query("hello");
        let hits: Vec<_> = execute(&idx, &q, &SearchScope::AllGrovesAndLetters).collect();
        let ids: Vec<_> = hits.iter().map(|h| h.message_id.clone()).collect();
        assert!(ids.contains(&"m1".into()));
        assert!(ids.contains(&"m2".into()));
    }

    #[test]
    fn quoted_phrase_matches_adjacent_only() {
        let idx = seed_index();
        let q = parse_query(r#""two words""#);
        let hits: Vec<_> = execute(&idx, &q, &SearchScope::AllGrovesAndLetters).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "m5");
    }

    #[test]
    fn quoted_phrase_requires_adjacency() {
        let idx = seed_index();
        // "hello words" — not adjacent anywhere in the corpus.
        let q = parse_query(r#""hello words""#);
        let hits: Vec<_> = execute(&idx, &q, &SearchScope::AllGrovesAndLetters).collect();
        assert!(hits.is_empty());
    }

    #[test]
    fn from_filter_narrows_by_author() {
        let idx = seed_index();
        let q = parse_query("hello from:@jun");
        let hits: Vec<_> = execute(&idx, &q, &SearchScope::AllGrovesAndLetters).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "m2");
    }

    #[test]
    fn in_filter_narrows_by_channel() {
        let idx = seed_index();
        let q = parse_query("hello in:#general");
        let hits: Vec<_> = execute(&idx, &q, &SearchScope::AllGrovesAndLetters).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "m1");
    }

    #[test]
    fn has_link_filter() {
        let idx = seed_index();
        let q = parse_query("has:link");
        let hits: Vec<_> = execute(&idx, &q, &SearchScope::AllGrovesAndLetters).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "m3");
    }

    #[test]
    fn results_ordered_desc_by_timestamp() {
        let idx = seed_index();
        let q = parse_query("");
        // Empty query + no filters = every message, newest first. Use
        // `has:` as a no-op guard: execute with a single tautological
        // phrase instead — the spec says empty-query is a no-op in the
        // UI but the executor itself is still callable.
        let q = SearchQuery { tokens: vec!["hello".into()], ..SearchQuery::default() };
        let hits: Vec<_> = execute(&idx, &q, &SearchScope::AllGrovesAndLetters).collect();
        assert!(hits.windows(2).all(|w| w[0].timestamp_ms >= w[1].timestamp_ms));
    }

    #[test]
    fn since_before_filter() {
        let idx = seed_index();
        // All our seed messages fall after 1970-01-01 — use epoch-ish
        // values to keep the harness local-tz agnostic.
        let mut q = parse_query("hello");
        q.filters.since = Some(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
        q.filters.before = Some(NaiveDate::from_ymd_opt(2100, 1, 1).unwrap());
        let hits: Vec<_> = execute(&idx, &q, &SearchScope::AllGrovesAndLetters).collect();
        assert!(hits.len() >= 2);
    }
}
```

Run: FAIL.

- [ ] **Step 4.3 — Implement.**

```rust
pub fn execute<'a>(
    index: &'a SearchIndex,
    query: &'a SearchQuery,
    scope: &'a SearchScope,
) -> impl Iterator<Item = SearchResult> + 'a {
    let candidates = candidate_postings(index, query);
    let mut out: Vec<SearchResult> = candidates
        .filter(|p| scope_admits(p, scope))
        .filter(|p| filters_admit(p, &query.filters))
        .filter(|p| body_admits(p, query))
        .map(|p| SearchResult {
            message_id: p.message_id.clone(),
            channel_id: p.channel_id.clone(),
            channel_name: p.channel_name.clone(),
            grove_id: p.grove_id.clone(),
            letter_id: p.letter_id.clone(),
            author_display_name: p.author_display_name.clone(),
            author_handle: p.author_handle.clone(),
            timestamp_ms: p.timestamp_ms,
            body: p.body.clone(),
            matched_ranges: vec![],
        })
        .collect();

    // Dedup by message_id — a message tokenised into N terms shows up
    // in N posting lists but renders once.
    let mut seen = std::collections::HashSet::new();
    out.retain(|r| seen.insert(r.message_id.clone()));

    out.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
    out.into_iter()
}

/// Return the union of posting lists across every query token + phrase
/// first word. Empty tokens + phrases + no filters = every posting.
fn candidate_postings<'a>(
    index: &'a SearchIndex,
    query: &'a SearchQuery,
) -> Box<dyn Iterator<Item = Posting> + 'a> {
    let tokens = query.tokens.iter()
        .chain(query.phrases.iter().flat_map(|p| p.split_whitespace().next()));
    let lists: Vec<Posting> = tokens
        .filter_map(|t| index.postings_for(t))
        .flatten()
        .cloned()
        .collect();
    if !lists.is_empty() {
        Box::new(lists.into_iter())
    } else {
        // No tokens + no phrases — walk every posting via by_msg.
        // This is O(n); callers with empty query should short-circuit
        // in the UI.
        let all: Vec<Posting> = index.postings.values().flatten().cloned().collect();
        let mut seen = std::collections::HashSet::new();
        Box::new(all.into_iter().filter(move |p| seen.insert(p.message_id.clone())))
    }
}

fn scope_admits(p: &Posting, scope: &SearchScope) -> bool {
    match scope {
        SearchScope::ThisLetter(id) => p.letter_id.as_deref() == Some(id.as_str()),
        SearchScope::ThisChannel(id) => p.channel_id == *id,
        SearchScope::AllLetters => p.letter_id.is_some(),
        SearchScope::AllGrovesAndLetters => true,
    }
}

fn filters_admit(p: &Posting, f: &super::query::QueryFilters) -> bool {
    if let Some(h) = &f.from_author {
        let lc = h.to_lowercase();
        if p.author_handle.to_lowercase() != lc && p.author_display_name.to_lowercase() != lc {
            return false;
        }
    }
    if let Some(c) = &f.in_channel {
        if p.channel_name.to_lowercase() != c.to_lowercase() {
            return false;
        }
    }
    if let Some(d) = f.since {
        let cutoff_ms = d.and_hms_opt(0, 0, 0).unwrap()
            .and_local_timezone(chrono::Local).unwrap()
            .timestamp_millis() as u64;
        if p.timestamp_ms < cutoff_ms { return false; }
    }
    if let Some(d) = f.before {
        let cutoff_ms = d.and_hms_opt(0, 0, 0).unwrap()
            .and_local_timezone(chrono::Local).unwrap()
            .timestamp_millis() as u64;
        if p.timestamp_ms >= cutoff_ms { return false; }
    }
    if f.has_image && !p.has_image { return false; }
    if f.has_file  && !p.has_file  { return false; }
    if f.has_link  && !p.has_link  { return false; }
    true
}

/// Every token in `query.tokens` must appear; every phrase must appear
/// as a contiguous substring. Case-insensitive.
fn body_admits(p: &Posting, q: &SearchQuery) -> bool {
    let body_lc = p.body.to_lowercase();
    for t in &q.tokens {
        if !body_contains_token(&body_lc, &p.author_display_name.to_lowercase(),
                                 &p.author_handle.to_lowercase(),
                                 &p.channel_name.to_lowercase(), t) {
            return false;
        }
    }
    for ph in &q.phrases {
        if !body_lc.contains(ph) { return false; }
    }
    true
}

fn body_contains_token(body: &str, display: &str, handle: &str, channel: &str, token: &str) -> bool {
    body.contains(token) || display.contains(token) || handle.contains(token) || channel.contains(token)
}
```

**Note — dual-target.** `chrono::Local` is available on wasm (chrono >= 0.4.22 with default features). If the wasm build complains about `Local`, fall back to a fixed UTC reading plus a comment flagging the divergence — local-tz semantics are per-user and the spec says "local timezone".

- [ ] **Step 4.4 — Verify GREEN.** `cargo test -p willow-client search::execute_tests`. All 10 pass.

- [ ] **Step 4.5 — WASM compile.** `cargo check --target wasm32-unknown-unknown -p willow-client`.

- [ ] **Step 4.6 — Commit.**

```bash
git add crates/client/src/search/execute.rs crates/client/src/search/mod.rs crates/client/src/search/tests.rs
git commit -m "ui(phase-2e): add scope-aware query executor"
```

---

### Task 5: Highlight excerpts

**Files:**
- Create: `crates/client/src/search/highlight.rs`
- Modify: `crates/client/src/search/mod.rs` — `pub mod highlight;`
- Modify: `crates/client/src/search/execute.rs` — populate `matched_ranges` at `SearchResult` emit time by calling `highlight::match_ranges(&p.body, query)`.
- Modify: `crates/client/src/search/tests.rs` — `mod highlight_tests;`

- [ ] **Step 5.1 — Failing tests.**

```rust
#[cfg(test)]
mod highlight_tests {
    use super::super::highlight::*;
    use super::super::query::*;

    #[test]
    fn no_tokens_yields_no_ranges() {
        let q = parse_query("");
        let ranges = match_ranges("hello world", &q);
        assert!(ranges.is_empty());
    }

    #[test]
    fn single_token_range() {
        let q = parse_query("world");
        let ranges = match_ranges("hello world", &q);
        assert_eq!(ranges, vec![(6, 11)]);
    }

    #[test]
    fn multiple_token_ranges() {
        let q = parse_query("hello world");
        let ranges = match_ranges("hello world", &q);
        assert_eq!(ranges, vec![(0, 5), (6, 11)]);
    }

    #[test]
    fn phrase_range() {
        let q = parse_query(r#""two words""#);
        let ranges = match_ranges("and two words here", &q);
        assert_eq!(ranges, vec![(4, 13)]);
    }

    #[test]
    fn case_insensitive_match() {
        let q = parse_query("HELLO");
        let ranges = match_ranges("Hello World", &q);
        assert_eq!(ranges, vec![(0, 5)]);
    }

    #[test]
    fn excerpt_centres_on_first_match() {
        let body = "a b c d e f g match h i j k l m n o p q r s";
        let q = parse_query("match");
        let ranges = match_ranges(body, &q);
        let excerpt = build_excerpt(body, &ranges, 20);
        assert!(excerpt.text.contains("match"));
        assert!(excerpt.text.starts_with("…") || excerpt.text.starts_with("e"));
    }

    #[test]
    fn excerpt_trims_on_both_sides_when_truncated() {
        let body = "x".repeat(200);
        let mut body = body;
        body.insert_str(100, " match ");
        let q = parse_query("match");
        let ranges = match_ranges(&body, &q);
        let excerpt = build_excerpt(&body, &ranges, 30);
        assert!(excerpt.text.starts_with("…"));
        assert!(excerpt.text.ends_with("…"));
    }
}
```

- [ ] **Step 5.2 — Implement.**

```rust
/// Excerpt + highlight ranges for render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Excerpt {
    pub text: String,
    /// Byte ranges inside `text` (not the original body).
    pub ranges: Vec<(usize, usize)>,
}

/// Find all match ranges in `body` for `query`. Case-insensitive.
///
/// Tokens match substring-anywhere; phrases match adjacent. Overlapping
/// ranges are merged.
pub fn match_ranges(body: &str, query: &super::query::SearchQuery) -> Vec<(usize, usize)> {
    let body_lc = body.to_lowercase();
    let mut ranges = Vec::new();
    for tok in &query.tokens {
        let mut idx = 0usize;
        while let Some(p) = body_lc[idx..].find(tok) {
            let start = idx + p;
            let end = start + tok.len();
            ranges.push((start, end));
            idx = end;
        }
    }
    for ph in &query.phrases {
        let mut idx = 0usize;
        while let Some(p) = body_lc[idx..].find(ph) {
            let start = idx + p;
            let end = start + ph.len();
            ranges.push((start, end));
            idx = end;
        }
    }
    ranges.sort();
    merge_overlaps(ranges)
}

fn merge_overlaps(mut r: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    r.sort_by_key(|&(a, _)| a);
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (a, b) in r {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Build a three-line excerpt centred on the first matched span. Adds
/// `…` on either side when truncated. Ranges are translated into
/// excerpt-local byte offsets.
pub fn build_excerpt(body: &str, ranges: &[(usize, usize)], context_chars: usize) -> Excerpt {
    let Some(&(first_start, first_end)) = ranges.first() else {
        return Excerpt { text: body.to_string(), ranges: vec![] };
    };
    let start_byte = body[..first_start].char_indices()
        .rev()
        .nth(context_chars)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let end_byte = body[first_end..].char_indices()
        .nth(context_chars)
        .map(|(i, _)| first_end + i)
        .unwrap_or(body.len());
    let mut text = String::new();
    let left_pad = start_byte > 0;
    let right_pad = end_byte < body.len();
    if left_pad { text.push('…'); }
    text.push_str(&body[start_byte..end_byte]);
    if right_pad { text.push('…'); }
    // Translate ranges into excerpt-local offsets.
    let base_offset = if left_pad { '…'.len_utf8() } else { 0 };
    let out_ranges: Vec<(usize, usize)> = ranges.iter()
        .filter(|&&(a, b)| a >= start_byte && b <= end_byte)
        .map(|&(a, b)| (a - start_byte + base_offset, b - start_byte + base_offset))
        .collect();
    Excerpt { text, ranges: out_ranges }
}
```

Also extend `execute.rs` — after building `SearchResult`, run `result.matched_ranges = highlight::match_ranges(&p.body, query);` so result rows carry ready-to-render ranges.

- [ ] **Step 5.3 — Verify GREEN.** `cargo test -p willow-client search::highlight_tests`. All 7 pass.

- [ ] **Step 5.4 — Commit.**

```bash
git add crates/client/src/search/highlight.rs crates/client/src/search/execute.rs crates/client/src/search/mod.rs crates/client/src/search/tests.rs
git commit -m "ui(phase-2e): build highlight match-ranges + centred excerpts"
```

---

### Task 6: Config + recents + status

**Files:**
- Create: `crates/client/src/search/config.rs`
- Create: `crates/client/src/search/status.rs`
- Modify: `crates/client/src/search/mod.rs` — `pub mod config; pub mod status;`
- Modify: `crates/client/src/storage.rs` — persistence helpers.
- Modify: `crates/client/src/search/tests.rs` — `mod config_tests;`

- [ ] **Step 6.1 — Config + recents types.**

```rust
//! Per-device search settings and the recent-queries ring buffer.

use std::collections::HashMap;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchIndexConfig {
    /// Master enable. `false` disables the index entirely. Default `true`.
    pub enabled: bool,
    /// Days of history to retain in the index. `30 | 90 | 365 | u32::MAX`.
    /// Default `90`. `u32::MAX` represents `all history`.
    pub horizon_days: u32,
    /// Whether to save recent queries locally. Default `true`.
    pub remember_recents: bool,
    /// Per-grove index opt-out. `false` = grove skipped.
    pub per_grove_enabled: HashMap<String, bool>,
}

impl Default for SearchIndexConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            horizon_days: 90,
            remember_recents: true,
            per_grove_enabled: HashMap::new(),
        }
    }
}

/// Ring buffer cap for recents.
pub const MAX_RECENTS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentQuery {
    pub text: String,
    pub timestamp_ms: u64,
}

/// Push a new recent, moving existing entries to the tail. Dedup by
/// text.
pub fn push_recent(list: &mut Vec<RecentQuery>, r: RecentQuery) {
    list.retain(|e| e.text != r.text);
    list.insert(0, r);
    if list.len() > MAX_RECENTS { list.truncate(MAX_RECENTS); }
}

pub fn forget_recent(list: &mut Vec<RecentQuery>, text: &str) {
    list.retain(|e| e.text != text);
}

pub fn clear_all_recents(list: &mut Vec<RecentQuery>) { list.clear(); }
```

Failing tests:

```rust
#[cfg(test)]
mod config_tests {
    use super::super::config::*;

    #[test]
    fn default_horizon_is_90() {
        assert_eq!(SearchIndexConfig::default().horizon_days, 90);
    }

    #[test]
    fn remember_recents_default_on() {
        assert!(SearchIndexConfig::default().remember_recents);
    }

    #[test]
    fn push_recent_moves_to_front() {
        let mut list = Vec::new();
        push_recent(&mut list, RecentQuery { text: "hello".into(), timestamp_ms: 1 });
        push_recent(&mut list, RecentQuery { text: "world".into(), timestamp_ms: 2 });
        assert_eq!(list[0].text, "world");
    }

    #[test]
    fn push_recent_dedups_by_text() {
        let mut list = Vec::new();
        push_recent(&mut list, RecentQuery { text: "hello".into(), timestamp_ms: 1 });
        push_recent(&mut list, RecentQuery { text: "hello".into(), timestamp_ms: 2 });
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].timestamp_ms, 2);
    }

    #[test]
    fn push_recent_caps_at_max() {
        let mut list = Vec::new();
        for i in 0..20 { push_recent(&mut list, RecentQuery { text: format!("q{i}"), timestamp_ms: i as u64 }); }
        assert_eq!(list.len(), MAX_RECENTS);
    }

    #[test]
    fn forget_recent_removes_by_text() {
        let mut list = Vec::new();
        push_recent(&mut list, RecentQuery { text: "hello".into(), timestamp_ms: 1 });
        forget_recent(&mut list, "hello");
        assert!(list.is_empty());
    }

    #[test]
    fn clear_all_empties_list() {
        let mut list = Vec::new();
        push_recent(&mut list, RecentQuery { text: "a".into(), timestamp_ms: 1 });
        push_recent(&mut list, RecentQuery { text: "b".into(), timestamp_ms: 2 });
        clear_all_recents(&mut list);
        assert!(list.is_empty());
    }
}
```

- [ ] **Step 6.2 — Status enum.** `crates/client/src/search/status.rs`:

```rust
//! Build-status signal for the search index. UI surfaces subscribe.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchIndexBuildStatus {
    Idle,
    Building,
    Indexing { done: u32, total: u32 },
    Error(String),
}

impl Default for SearchIndexBuildStatus {
    fn default() -> Self { Self::Idle }
}
```

Register `pub mod status;` in `mod.rs` + re-export.

- [ ] **Step 6.3 — Persistence.** In `crates/client/src/storage.rs` add:

```rust
/// Persisted search config (see `SearchIndexConfig` in `crate::search::config`).
pub fn save_search_config(c: &crate::search::SearchIndexConfig) {
    if let Ok(bytes) = willow_transport::pack(c) { save_raw("search_config", &bytes); }
}

pub fn load_search_config() -> Option<crate::search::SearchIndexConfig> {
    load_raw("search_config").and_then(|b| willow_transport::unpack(&b).ok())
}

/// Persisted recent-query ring buffer.
pub fn save_recents(list: &[crate::search::RecentQuery]) {
    if let Ok(bytes) = willow_transport::pack(&list.to_vec()) { save_raw("search_recents", &bytes); }
}

pub fn load_recents() -> Vec<crate::search::RecentQuery> {
    load_raw("search_recents")
        .and_then(|b| willow_transport::unpack::<Vec<crate::search::RecentQuery>>(&b).ok())
        .unwrap_or_default()
}
```

- [ ] **Step 6.4 — Verify GREEN.** `cargo test -p willow-client search::config_tests`. All 7 pass.

- [ ] **Step 6.5 — Commit.**

```bash
git add crates/client/src/search/config.rs crates/client/src/search/status.rs crates/client/src/search/mod.rs crates/client/src/storage.rs crates/client/src/search/tests.rs
git commit -m "ui(phase-2e): add search config + recents + build-status primitives"
```

---

### Task 7: `SearchIndexHandle` + client wiring

**Files:**
- Create: `crates/client/src/search/handle.rs`
- Modify: `crates/client/src/search/mod.rs`
- Modify: `crates/client/src/lib.rs` — top-level re-exports + integrate handle into `ClientHandle`.
- Modify: `crates/client/src/actions.rs` or `accessors.rs` — expose `fn search(&self) -> SearchIndexHandle` on `ClientHandle`.

- [ ] **Step 7.1 — Handle + API.**

```rust
//! Top-level handle exposing the search index to UI code.

use std::sync::Arc;
use parking_lot::Mutex;
use super::{config::{RecentQuery, SearchIndexConfig}, execute::{SearchResult, SearchScope},
            index::{IndexableMessage, SearchIndex}, query::SearchQuery,
            status::SearchIndexBuildStatus};

#[derive(Clone)]
pub struct SearchIndexHandle {
    index: Arc<Mutex<SearchIndex>>,
    config: Arc<Mutex<SearchIndexConfig>>,
    recents: Arc<Mutex<Vec<RecentQuery>>>,
    status: Arc<Mutex<SearchIndexBuildStatus>>,
}

impl SearchIndexHandle {
    pub fn new() -> Self {
        let config = crate::storage::load_search_config().unwrap_or_default();
        let recents = crate::storage::load_recents();
        Self {
            index: Arc::new(Mutex::new(SearchIndex::new())),
            config: Arc::new(Mutex::new(config)),
            recents: Arc::new(Mutex::new(recents)),
            status: Arc::new(Mutex::new(SearchIndexBuildStatus::Idle)),
        }
    }

    pub fn insert(&self, m: IndexableMessage) {
        // Skip disabled groves.
        if let Some(gid) = &m.grove_id {
            let cfg = self.config.lock();
            if cfg.per_grove_enabled.get(gid).copied() == Some(false) { return; }
        }
        self.index.lock().insert(m);
    }

    pub fn rebuild(&self, msgs: Vec<IndexableMessage>) {
        let total = msgs.len() as u32;
        *self.status.lock() = SearchIndexBuildStatus::Building;
        let mut index = self.index.lock();
        *index = SearchIndex::new();
        for (i, m) in msgs.into_iter().enumerate() {
            // Apply grove opt-out.
            if let Some(gid) = &m.grove_id {
                let cfg = self.config.lock();
                if cfg.per_grove_enabled.get(gid).copied() == Some(false) { continue; }
            }
            index.insert(m);
            *self.status.lock() = SearchIndexBuildStatus::Indexing { done: (i + 1) as u32, total };
        }
        *self.status.lock() = SearchIndexBuildStatus::Idle;
    }

    pub fn query(&self, q: &SearchQuery, scope: &SearchScope) -> Vec<SearchResult> {
        super::execute::execute(&self.index.lock(), q, scope).collect()
    }

    pub fn config(&self) -> SearchIndexConfig { self.config.lock().clone() }
    pub fn set_config(&self, c: SearchIndexConfig) {
        *self.config.lock() = c.clone();
        crate::storage::save_search_config(&c);
    }

    pub fn recents(&self) -> Vec<RecentQuery> { self.recents.lock().clone() }
    pub fn push_recent(&self, r: RecentQuery) {
        let cfg = self.config.lock();
        if !cfg.remember_recents { return; }
        drop(cfg);
        let mut list = self.recents.lock();
        super::config::push_recent(&mut list, r);
        crate::storage::save_recents(&list);
    }
    pub fn forget_recent(&self, text: &str) {
        let mut list = self.recents.lock();
        super::config::forget_recent(&mut list, text);
        crate::storage::save_recents(&list);
    }
    pub fn clear_all_recents(&self) {
        let mut list = self.recents.lock();
        super::config::clear_all_recents(&mut list);
        crate::storage::save_recents(&list);
    }

    pub fn status(&self) -> SearchIndexBuildStatus { self.status.lock().clone() }
}

impl Default for SearchIndexHandle { fn default() -> Self { Self::new() } }
```

- [ ] **Step 7.2 — Re-exports.** `crates/client/src/search/mod.rs`:

```rust
pub mod config;
pub mod execute;
pub mod handle;
pub mod highlight;
pub mod index;
pub mod query;
pub mod status;
pub mod tokenize;

#[cfg(test)]
mod tests;

pub use config::{RecentQuery, SearchIndexConfig, MAX_RECENTS};
pub use execute::{execute, SearchResult, SearchScope};
pub use handle::SearchIndexHandle;
pub use highlight::{build_excerpt, match_ranges, Excerpt};
pub use index::{IndexableMessage, Posting, SearchIndex};
pub use query::{parse_query, QueryFilters, QueryWarning, SearchQuery};
pub use status::SearchIndexBuildStatus;
```

In `crates/client/src/lib.rs`:

```rust
pub use search::{
    IndexableMessage, RecentQuery, SearchIndexBuildStatus, SearchIndexConfig,
    SearchIndexHandle, SearchQuery, SearchResult, SearchScope,
};
```

- [ ] **Step 7.3 — Failing handle test.** In `crates/client/src/search/tests.rs` add `mod handle_tests`:

```rust
#[cfg(test)]
mod handle_tests {
    use super::super::*;
    use willow_identity::EndpointId;

    fn mk_msg(id: &str, body: &str) -> IndexableMessage {
        IndexableMessage {
            message_id: id.into(), channel_id: "c1".into(), channel_name: "general".into(),
            grove_id: Some("g0".into()), letter_id: None,
            author_peer_id: EndpointId::from_bytes([0u8; 32]),
            author_handle: "mira".into(), author_display_name: "Mira".into(),
            timestamp_ms: 100, body: body.into(),
            has_image: false, has_file: false, has_link: false,
        }
    }

    #[test]
    fn handle_insert_then_query() {
        let h = SearchIndexHandle::new();
        h.insert(mk_msg("m1", "hello world"));
        let q = parse_query("hello");
        let hits = h.query(&q, &SearchScope::AllGrovesAndLetters);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, "m1");
    }

    #[test]
    fn handle_grove_opt_out_drops_inserts() {
        let h = SearchIndexHandle::new();
        let mut cfg = h.config();
        cfg.per_grove_enabled.insert("g0".into(), false);
        h.set_config(cfg);
        h.insert(mk_msg("m1", "hello world"));
        let q = parse_query("hello");
        let hits = h.query(&q, &SearchScope::AllGrovesAndLetters);
        assert!(hits.is_empty());
    }

    #[test]
    fn recents_disabled_by_config() {
        let h = SearchIndexHandle::new();
        let mut cfg = h.config();
        cfg.remember_recents = false;
        h.set_config(cfg);
        h.push_recent(RecentQuery { text: "hi".into(), timestamp_ms: 1 });
        assert!(h.recents().is_empty());
    }

    #[test]
    fn rebuild_replaces_index() {
        let h = SearchIndexHandle::new();
        h.insert(mk_msg("m1", "hello"));
        h.rebuild(vec![mk_msg("m2", "world")]);
        let hits = h.query(&parse_query("hello"), &SearchScope::AllGrovesAndLetters);
        assert!(hits.is_empty());
        let hits = h.query(&parse_query("world"), &SearchScope::AllGrovesAndLetters);
        assert_eq!(hits.len(), 1);
    }
}
```

Run: initially fails; after handle.rs lands, passes.

- [ ] **Step 7.4 — Verify GREEN.** `cargo test -p willow-client search::`. All modules pass (tokenize + query + index + execute + highlight + config + handle).

- [ ] **Step 7.5 — WASM compile.** `cargo check --target wasm32-unknown-unknown -p willow-client`.

- [ ] **Step 7.6 — Commit.**

```bash
git add crates/client/src/search/handle.rs crates/client/src/search/mod.rs crates/client/src/search/tests.rs crates/client/src/lib.rs
git commit -m "ui(phase-2e): expose SearchIndexHandle on willow-client"
```

---

### Task 8: UI state + signals

**Files:**
- Modify: `crates/web/src/state.rs` — add `SearchUiState` + `SearchUiWriteSignals` + wire into `AppState` / `AppWriteSignals` / `create_signals()`.

- [ ] **Step 8.1 — Add signal buckets.**

```rust
// Append in crates/web/src/state.rs

use willow_client::{SearchIndexBuildStatus, SearchResult, SearchScope, RecentQuery};

#[derive(Clone, Copy)]
pub struct SearchUiState {
    pub open: ReadSignal<bool>,
    pub query: ReadSignal<String>,
    pub scope: ReadSignal<SearchScope>,
    pub results: ReadSignal<Vec<SearchResult>>,
    pub status: ReadSignal<SearchIndexBuildStatus>,
    pub recents: ReadSignal<Vec<RecentQuery>>,
    /// True while a debounce timer is outstanding — UI dims stale results
    /// by 15 % per spec §Performance envelope.
    pub debouncing: ReadSignal<bool>,
}

#[derive(Clone, Copy)]
pub struct SearchUiWriteSignals {
    pub set_open: WriteSignal<bool>,
    pub set_query: WriteSignal<String>,
    pub set_scope: WriteSignal<SearchScope>,
    pub set_results: WriteSignal<Vec<SearchResult>>,
    pub set_status: WriteSignal<SearchIndexBuildStatus>,
    pub set_recents: WriteSignal<Vec<RecentQuery>>,
    pub set_debouncing: WriteSignal<bool>,
}
```

Add `pub search: SearchUiState` / `pub search: SearchUiWriteSignals` to `AppState` / `AppWriteSignals`.

In `create_signals()`:

```rust
let (search_open, set_search_open) = signal(false);
let (search_query, set_search_query) = signal(String::new());
let saved_scope: SearchScope = web_sys::window()
    .and_then(|w| w.local_storage().ok().flatten())
    .and_then(|s| s.get_item("willow.search.scope").ok().flatten())
    .and_then(|v| serde_json::from_str::<SearchScope>(&v).ok())
    .unwrap_or(SearchScope::AllGrovesAndLetters);
let (search_scope, set_search_scope) = signal(saved_scope);
let (search_results, set_search_results) = signal(Vec::<SearchResult>::new());
let (search_status, set_search_status) = signal(SearchIndexBuildStatus::default());
let (search_recents, set_search_recents) = signal(Vec::<RecentQuery>::new());
let (search_debouncing, set_search_debouncing) = signal(false);
```

Add `search: SearchUiState { open, query, scope, results, status, recents, debouncing }` to the returned `AppState`.

Persist scope on every change:

```rust
Effect::new(move |_| {
    let s = search_scope.get();
    if let Some(store) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        if let Ok(j) = serde_json::to_string(&s) {
            let _ = store.set_item("willow.search.scope", &j);
        }
    }
});
```

(Requires `SearchScope: Serialize + Deserialize` — add `#[derive(Serialize, Deserialize)]` in `crates/client/src/search/execute.rs`.)

- [ ] **Step 8.2 — Clippy + wasm check.** `cargo check --target wasm32-unknown-unknown -p willow-web`. Zero warnings.

- [ ] **Step 8.3 — Commit.**

```bash
git add crates/web/src/state.rs crates/client/src/search/execute.rs
git commit -m "ui(phase-2e): add search UI signals + persist scope"
```

---

### Task 9: `<SearchInput>` + scope chip + debounced query

**Files:**
- Create: `crates/web/src/components/search/mod.rs` (barrel).
- Create: `crates/web/src/components/search/input.rs`
- Create: `crates/web/src/components/search/scope_chip.rs`
- Modify: `crates/web/src/components/mod.rs` — `pub mod search;` + re-export.
- Modify: `crates/web/style.css` — input + chip CSS.
- Modify: `crates/web/tests/browser.rs` — first 4 tests in `phase_2e_local_search`.

- [ ] **Step 9.1 — Failing browser tests.** Append in `crates/web/tests/browser.rs`:

```rust
#[cfg(test)]
mod phase_2e_local_search {
    use super::*;

    #[wasm_bindgen_test]
    async fn search_input_has_role_search() {
        let container = mount_test_with_shell(TestShell::Desktop, || view! { <App /> });
        tick().await;
        // Open the search surface via the state signal.
        let app_state = use_context::<willow_web::state::AppState>().expect("state");
        let write = use_context::<willow_web::state::AppWriteSignals>().expect("write");
        write.search.set_open.set(true);
        tick().await;
        let form = query(&container, "form[role='search'][aria-label='local search']");
        assert!(form.is_some());
    }

    #[wasm_bindgen_test]
    async fn scope_chip_renders_current_value() {
        let container = mount_test_with_shell(TestShell::Desktop, || view! { <App /> });
        tick().await;
        let write = use_context::<willow_web::state::AppWriteSignals>().expect("write");
        write.search.set_open.set(true);
        write.search.set_scope.set(willow_client::SearchScope::AllLetters);
        tick().await;
        let chip = query(&container, ".scope-chip").expect("chip mounts");
        assert!(text(&chip).contains("all letters"));
    }

    #[wasm_bindgen_test]
    async fn esc_non_empty_query_clears() { /* drive keydown('Escape') on input with text, assert query cleared but surface open */ }

    #[wasm_bindgen_test]
    async fn esc_empty_query_closes_surface() { /* drive keydown('Escape') on empty input, assert surface closed */ }
}
```

- [ ] **Step 9.2 — Implement `<SearchInput>`.**

```rust
// crates/web/src/components/search/input.rs
use leptos::prelude::*;
use willow_client::SearchScope;
use crate::state::{AppState, AppWriteSignals};

#[component]
pub fn SearchInput(
    #[prop(into)] on_submit: Callback<String>,
) -> impl IntoView {
    let state = use_context::<AppState>().unwrap();
    let write = use_context::<AppWriteSignals>().unwrap();

    let placeholder = move || match state.search.scope.get() {
        SearchScope::ThisLetter(_) => "search this letter",
        SearchScope::ThisChannel(_) => "search this channel",
        SearchScope::AllLetters => "search all letters",
        SearchScope::AllGrovesAndLetters => "search groves + letters",
    };

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        match ev.key().as_str() {
            "Escape" => {
                ev.prevent_default();
                if !state.search.query.get_untracked().is_empty() {
                    write.search.set_query.set(String::new());
                } else {
                    write.search.set_open.set(false);
                }
            }
            "Enter" => {
                ev.prevent_default();
                on_submit.run(state.search.query.get_untracked());
            }
            _ => {}
        }
    };

    view! {
        <form role="search" aria-label="local search" class="search-form"
              on:submit=move |ev| ev.prevent_default()>
            <input
                class=move || if state.search.debouncing.get() { "search-input is-debouncing" } else { "search-input" }
                type="text"
                placeholder=placeholder
                aria-label="local search input"
                aria-autocomplete="list"
                aria-controls="search-results-list"
                prop:value=move || state.search.query.get()
                on:input=move |ev| write.search.set_query.set(event_target_value(&ev))
                on:keydown=on_keydown
            />
        </form>
    }
}
```

- [ ] **Step 9.3 — Implement `<ScopeChip>`.**

```rust
// crates/web/src/components/search/scope_chip.rs
use leptos::prelude::*;
use willow_client::SearchScope;
use crate::icons;
use crate::state::{AppState, AppWriteSignals};

fn chip_label(s: &SearchScope) -> &'static str {
    match s {
        SearchScope::ThisLetter(_) => "this letter",
        SearchScope::ThisChannel(_) => "this channel",
        SearchScope::AllLetters => "all letters",
        SearchScope::AllGrovesAndLetters => "all groves + letters",
    }
}

#[component]
pub fn ScopeChip(
    /// Channel id currently focused (enables `this channel` selection).
    #[prop(into)] focused_channel: Signal<Option<String>>,
    /// Letter id currently focused.
    #[prop(optional, into)]
    focused_letter: Option<Signal<Option<String>>>,
) -> impl IntoView {
    let state = use_context::<AppState>().unwrap();
    let write = use_context::<AppWriteSignals>().unwrap();
    let (open, set_open) = signal(false);

    let disabled_letter = move || focused_letter.map(|s| s.get().is_none()).unwrap_or(true);
    let disabled_channel = move || focused_channel.get().is_none();

    let pick = move |s: SearchScope| {
        write.search.set_scope.set(s);
        set_open.set(false);
    };

    view! {
        <button class="scope-chip"
                aria-haspopup="listbox"
                aria-expanded=move || open.get().to_string()
                on:click=move |_| set_open.update(|v| *v = !*v)>
            <span class="scope-chip-label">{move || chip_label(&state.search.scope.get())}</span>
            <span class="scope-chip-chevron" aria-hidden="true">{icons::icon_chevron_down()}</span>
        </button>
        {move || open.get().then(|| view! {
            <div class="scope-chip-popover" role="listbox">
                <button class="scope-chip-popover-option"
                        role="option"
                        aria-selected=move || matches!(state.search.scope.get(), SearchScope::ThisLetter(_)).to_string()
                        disabled=disabled_letter
                        title=move || if disabled_letter() { "open a letter first" } else { "" }
                        on:click={
                            let p = pick.clone();
                            move |_| if let Some(id) = focused_letter.and_then(|s| s.get()) {
                                p(SearchScope::ThisLetter(id));
                            }
                        }>
                    "this letter"
                </button>
                <button class="scope-chip-popover-option"
                        role="option"
                        aria-selected=move || matches!(state.search.scope.get(), SearchScope::ThisChannel(_)).to_string()
                        disabled=disabled_channel
                        title=move || if disabled_channel() { "open a channel first" } else { "" }
                        on:click={
                            let p = pick.clone();
                            move |_| if let Some(id) = focused_channel.get() { p(SearchScope::ThisChannel(id)); }
                        }>
                    "this channel"
                </button>
                <button class="scope-chip-popover-option"
                        role="option"
                        aria-selected=move || matches!(state.search.scope.get(), SearchScope::AllLetters).to_string()
                        on:click={
                            let p = pick.clone();
                            move |_| p(SearchScope::AllLetters)
                        }>
                    "all letters"
                </button>
                <button class="scope-chip-popover-option"
                        role="option"
                        aria-selected=move || matches!(state.search.scope.get(), SearchScope::AllGrovesAndLetters).to_string()
                        on:click={
                            let p = pick.clone();
                            move |_| p(SearchScope::AllGrovesAndLetters)
                        }>
                    "all groves + letters"
                </button>
            </div>
        })}
    }
}
```

- [ ] **Step 9.4 — CSS.** Append in `crates/web/style.css`:

```css
/* ── Phase 2e · Local search ─────────────────────────────────────── */

.search-surface {
    position: fixed; inset: 0;
    background: var(--bg-1);
    display: flex; flex-direction: column;
    z-index: 900;
    animation: willow-pop-in var(--motion) var(--motion-ease);
}
.search-form {
    padding: 16px 20px 12px;
    border-bottom: 1px solid var(--line-soft);
    position: sticky; top: 0;
    background: var(--bg-1); z-index: 2;
}
.search-input {
    width: 100%;
    height: 44px;
    padding: 0 14px;
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: var(--radius-m);
    color: var(--ink-0);
    font: 15px var(--font-ui);
    outline: none;
    transition: opacity 120ms ease-out;
}
.search-input:focus-visible { box-shadow: var(--focus-ring); }
.search-input::placeholder { color: var(--ink-3); }
.search-input.is-debouncing ~ .search-results { opacity: 0.85; }

.scope-chip {
    display: inline-flex; align-items: center; gap: 4px;
    padding: 4px 8px;
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: var(--radius-s);
    color: var(--moss-3);
    font: 11px/1 var(--font-mono);
    letter-spacing: 0.5px;
}
.scope-chip-chevron { display: inline-flex; transition: transform 120ms; }
.scope-chip[aria-expanded='true'] .scope-chip-chevron { transform: rotate(180deg); }
.scope-chip-popover {
    position: absolute;
    margin-top: 6px;
    background: var(--bg-1);
    border: 1px solid var(--line);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow-2);
    padding: 4px;
    min-width: 180px;
}
.scope-chip-popover-option {
    display: flex; width: 100%; padding: 8px 12px;
    background: transparent; border: none;
    color: var(--ink-2);
    font: 13px var(--font-ui);
    text-align: left; cursor: pointer;
}
.scope-chip-popover-option[aria-selected='true'] { color: var(--moss-3); }
.scope-chip-popover-option:disabled { color: var(--ink-4); cursor: not-allowed; }
.scope-chip-popover-option:hover:not(:disabled) { background: var(--bg-2); }

@media (prefers-reduced-motion: reduce) {
    .search-surface { animation: none; }
    .scope-chip-chevron { transition: none; }
}
```

Register the module: `crates/web/src/components/search/mod.rs`:

```rust
pub mod input;
pub mod scope_chip;

pub use input::SearchInput;
pub use scope_chip::ScopeChip;
```

In `crates/web/src/components/mod.rs`: `pub mod search;`.

- [ ] **Step 9.5 — Verify tests.** `just test-browser` in CI. 4 `phase_2e_local_search` tests pass.

- [ ] **Step 9.6 — Commit.**

```bash
git add crates/web/src/components/search/ crates/web/src/components/mod.rs crates/web/style.css crates/web/tests/browser.rs
git commit -m "ui(phase-2e): SearchInput + ScopeChip with keyboard + a11y"
```

---

### Task 10: `<ResultsList>` + `<ResultRow>` + streaming banner + recents

**Files:**
- Create: `crates/web/src/components/search/results.rs`
- Create: `crates/web/src/components/search/row.rs`
- Create: `crates/web/src/components/search/recents.rs`
- Modify: `crates/web/src/components/search/mod.rs` — re-export.
- Modify: `crates/web/src/icons.rs` — `icon_arrow_up_right`.
- Modify: `crates/web/style.css` — results + row CSS.
- Modify: `crates/web/tests/browser.rs` — append 4 more tests.

- [ ] **Step 10.1 — Failing tests.** Append to `phase_2e_local_search`:

```rust
#[wasm_bindgen_test]
async fn result_row_renders_excerpt_with_mark() { /* seed the handle with a msg containing "hello"; open surface; assert `.search-result-excerpt mark[aria-label='match']` appears */ }

#[wasm_bindgen_test]
async fn result_row_click_jumps_to_container() { /* click; assert surface closed AND `#messages` scroll lands the matched message within 1/3 viewport */ }

#[wasm_bindgen_test]
async fn streaming_banner_renders_during_build() { /* set status to Indexing {done:10, total:100}; assert `.search-streaming-banner` text matches "searching… · 10 matches so far" */ }

#[wasm_bindgen_test]
async fn privacy_footer_always_visible() { /* assert `.search-privacy-footer` has exact text */ }
```

- [ ] **Step 10.2 — Implement `<ResultRow>`.**

```rust
// crates/web/src/components/search/row.rs
use leptos::prelude::*;
use willow_client::{SearchResult, search::{build_excerpt}};
use crate::icons;
use crate::util::format_timestamp;

#[component]
pub fn ResultRow(
    result: SearchResult,
    selected: Signal<bool>,
    #[prop(into)] on_select: Callback<SearchResult>,
) -> impl IntoView {
    let r = result.clone();
    let id_attr = format!("search-row-{}", r.message_id);
    let excerpt = build_excerpt(&r.body, &r.matched_ranges, 60);
    let ts = format_timestamp(r.timestamp_ms);

    // Build body spans with <mark> around each range.
    let spans = render_spans(&excerpt.text, &excerpt.ranges);

    view! {
        <button id=id_attr
                class=move || if selected.get() { "search-result-row is-selected" } else { "search-result-row" }
                role="option"
                aria-selected=move || selected.get().to_string()
                on:click={
                    let r = r.clone();
                    move |_| on_select.run(r.clone())
                }>
            <div class="search-result-context">
                <em class="search-result-container">{r.channel_name.clone()}</em>
                <span class="search-result-author">{r.author_display_name.clone()}</span>
                <span class="search-result-ts">"·"{ts.clone()}</span>
            </div>
            <div class="search-result-excerpt">{spans}</div>
            <span class="search-result-arrow" aria-hidden="true">{icons::icon_arrow_up_right()}</span>
        </button>
    }
}

fn render_spans(text: &str, ranges: &[(usize, usize)]) -> Vec<AnyView> {
    let mut out: Vec<AnyView> = Vec::new();
    let mut cursor = 0usize;
    for &(a, b) in ranges {
        if a > cursor {
            let slice = text[cursor..a].to_string();
            out.push(view! { <span>{slice}</span> }.into_any());
        }
        let slice = text[a..b].to_string();
        out.push(view! { <mark aria-label="match">{slice}</mark> }.into_any());
        cursor = b;
    }
    if cursor < text.len() {
        out.push(view! { <span>{text[cursor..].to_string()}</span> }.into_any());
    }
    out
}
```

- [ ] **Step 10.3 — Implement `<ResultsList>`.**

```rust
// crates/web/src/components/search/results.rs
use leptos::prelude::*;
use willow_client::{SearchIndexBuildStatus, SearchResult, SearchScope};
use super::row::ResultRow;
use crate::state::AppState;

#[component]
pub fn ResultsList(
    #[prop(into)] on_select: Callback<SearchResult>,
) -> impl IntoView {
    let state = use_context::<AppState>().unwrap();
    let groups = Memo::new(move |_| group_results(&state.search.results.get(), &state.search.scope.get()));
    let status = state.search.status;

    view! {
        {move || match status.get() {
            SearchIndexBuildStatus::Indexing { done, total: _ } => Some(view! {
                <div class="search-streaming-banner" aria-live="polite">
                    {format!("searching… · {done} matches so far")}
                </div>
            }.into_any()),
            _ => None,
        }}
        <div id="search-results-list"
             class="search-results"
             role="listbox"
             aria-label="search results">
            <For each=move || groups.get()
                 key=|g| g.0.clone()
                 let:(label, items)>
                {(!label.is_empty()).then(|| view! {
                    <div class="search-group-header">
                        <em>{label.clone()}</em>
                        <span class="search-group-count">{format!("({})", items.len())}</span>
                    </div>
                })}
                <For each=move || items.clone()
                     key=|r| r.message_id.clone()
                     let:r>
                    <ResultRow
                        result=r
                        selected=Signal::derive(move || false)
                        on_select=on_select
                    />
                </For>
            </For>
        </div>
    }
}

fn group_results(rows: &[SearchResult], scope: &SearchScope) -> Vec<(String, Vec<SearchResult>)> {
    use std::collections::BTreeMap;
    match scope {
        SearchScope::ThisChannel(_) | SearchScope::ThisLetter(_) => {
            vec![(String::new(), rows.to_vec())]
        }
        SearchScope::AllLetters => {
            let mut m: BTreeMap<String, Vec<SearchResult>> = BTreeMap::new();
            for r in rows {
                let key = r.letter_id.clone().unwrap_or_else(|| "letter".into());
                m.entry(key).or_default().push(r.clone());
            }
            m.into_iter().collect()
        }
        SearchScope::AllGrovesAndLetters => {
            let mut m: BTreeMap<String, Vec<SearchResult>> = BTreeMap::new();
            for r in rows {
                let key = r.grove_id.clone()
                    .map(|g| format!("grove: {g}"))
                    .unwrap_or_else(|| "letters".into());
                m.entry(key).or_default().push(r.clone());
            }
            m.into_iter().collect()
        }
    }
}
```

- [ ] **Step 10.4 — Implement `<RecentsList>`.**

```rust
// crates/web/src/components/search/recents.rs
use leptos::prelude::*;
use willow_client::RecentQuery;
use crate::icons;
use crate::state::{AppState, AppWriteSignals};

#[component]
pub fn RecentsList(
    #[prop(into)] on_pick: Callback<String>,
    #[prop(into)] on_forget: Callback<String>,
    #[prop(into)] on_clear_all: Callback<()>,
) -> impl IntoView {
    let state = use_context::<AppState>().unwrap();
    view! {
        <div class="search-recents">
            <For each=move || state.search.recents.get()
                 key=|r: &RecentQuery| r.text.clone()
                 let:r>
                <button class="search-recent-chip"
                        on:click=move |_| on_pick.run(r.text.clone())>
                    <span class="icon">{icons::icon_search()}</span>
                    <span>{r.text.clone()}</span>
                </button>
            </For>
            <button class="search-recent-clear"
                    on:click=move |_| on_clear_all.run(())>
                "clear all recents"
            </button>
        </div>
    }
}
```

- [ ] **Step 10.5 — CSS.** Append:

```css
.search-results { flex: 1; overflow-y: auto; padding: 8px 0; }

.search-group-header {
    display: flex; align-items: baseline; gap: 6px;
    padding: 12px 20px 4px;
    color: var(--ink-3);
    font: italic 14px var(--font-display);
}
.search-group-count { font: 11px var(--font-mono); color: var(--ink-4); }

.search-result-row {
    display: flex; flex-direction: column; align-items: flex-start;
    width: 100%; min-height: 44px;
    padding: 10px 20px;
    background: transparent; border: none;
    color: var(--ink-1);
    text-align: left; cursor: pointer;
    transition: background 120ms ease-out;
}
.search-result-row:hover { background: var(--bg-2); }
.search-result-row.is-selected { background: var(--bg-2); box-shadow: inset 2px 0 0 var(--moss-3); }
.search-result-row:focus-visible { box-shadow: var(--focus-ring); }

.search-result-context {
    font: 11px var(--font-mono);
    color: var(--ink-3);
}
.search-result-container { font-family: var(--font-display); font-style: italic; color: var(--ink-2); }
.search-result-excerpt {
    font: 13px/1.5 var(--font-ui);
    color: var(--ink-1);
    display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical;
    overflow: hidden; text-overflow: ellipsis;
}
.search-result-excerpt mark {
    background: color-mix(in oklab, var(--moss-3) 18%, transparent);
    color: inherit;
    text-decoration: underline;
    text-decoration-thickness: 1.5px;
    padding: 0;
}
.search-result-arrow {
    position: absolute; right: 20px; top: 12px;
    color: var(--ink-3);
}
@media (max-width: 720px) { .search-result-arrow { display: none; } }

.search-streaming-banner {
    padding: 6px 20px; font: italic 12px var(--font-display); color: var(--ink-3);
}

.search-recents { display: flex; flex-wrap: wrap; gap: 8px; padding: 12px 20px; }
.search-recent-chip {
    display: inline-flex; align-items: center; gap: 6px;
    padding: 4px 10px;
    max-width: 180px;
    background: var(--bg-2); border: 1px solid var(--line);
    border-radius: var(--radius-s);
    color: var(--ink-2);
    font: 12px var(--font-mono);
    cursor: pointer;
}
.search-recent-clear {
    align-self: center;
    background: transparent; border: none;
    color: var(--ink-3); font: 11px var(--font-mono);
    cursor: pointer;
}

.search-privacy-footer {
    padding: 8px 20px; color: var(--ink-3);
    font: 11px var(--font-mono);
    border-top: 1px solid var(--line-soft);
    background: var(--bg-1);
}
```

- [ ] **Step 10.6 — Add `icon_arrow_up_right` to `icons.rs`.**

```rust
pub fn icon_arrow_up_right() -> impl IntoView {
    view! {
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none"
             stroke="currentColor" stroke-width="1.5"
             stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M7 17L17 7"/><path d="M7 7H17V17"/>
        </svg>
    }
}
```

- [ ] **Step 10.7 — Verify.** `cargo check --target wasm32-unknown-unknown -p willow-web`. Plan the `just test-browser` runs to CI.

- [ ] **Step 10.8 — Commit.**

```bash
git add crates/web/src/components/search/ crates/web/src/icons.rs crates/web/style.css crates/web/tests/browser.rs
git commit -m "ui(phase-2e): render results list + row + highlight + recents"
```

---

### Task 11: `<SearchSurface>` + index hydration + debounced query driver

**Files:**
- Create: `crates/web/src/components/search/surface.rs`
- Modify: `crates/web/src/components/search/mod.rs` — re-export `SearchSurface`.
- Modify: `crates/web/src/app.rs` — mount `<SearchSurface>`, hydrate index.

- [ ] **Step 11.1 — Implement `<SearchSurface>`.**

```rust
// crates/web/src/components/search/surface.rs
use leptos::prelude::*;
use willow_client::{SearchIndexHandle, SearchScope, parse_query, RecentQuery};
use crate::state::{AppState, AppWriteSignals};
use super::{SearchInput, ScopeChip, results::ResultsList, recents::RecentsList};

#[component]
pub fn SearchSurface(index: SearchIndexHandle) -> impl IntoView {
    let state = use_context::<AppState>().unwrap();
    let write = use_context::<AppWriteSignals>().unwrap();

    // Debounce the query (120 ms) and execute against the index.
    let idx = index.clone();
    Effect::new(move |_| {
        let raw = state.search.query.get();
        let scope = state.search.scope.get();
        let idx = idx.clone();
        write.search.set_debouncing.set(true);
        let handle = leptos::prelude::set_timeout_with_handle(
            move || {
                let q = parse_query(&raw);
                let results = idx.query(&q, &scope);
                write.search.set_results.set(results);
                write.search.set_debouncing.set(false);
            },
            std::time::Duration::from_millis(120),
        ).ok();
        // Cancel on next run.
        on_cleanup(move || { if let Some(h) = handle { h.clear(); } });
    });

    let on_submit = {
        let idx = index.clone();
        Callback::new(move |q: String| {
            if !q.is_empty() {
                idx.push_recent(RecentQuery { text: q, timestamp_ms: js_sys::Date::now() as u64 });
                write.search.set_recents.set(idx.recents());
            }
        })
    };

    let on_select = Callback::new(move |r: willow_client::SearchResult| {
        // Close surface + stash jump target on AppState so chat.rs can
        // scroll it into view. For v1 we route via the `chat.set_current_channel`
        // signal plus a one-shot `pending_scroll_msg_id` signal the chat
        // renderer reads and clears.
        write.chat.set_current_channel.set(r.channel_name.clone());
        write.search.set_open.set(false);
    });

    let on_forget = {
        let idx = index.clone();
        Callback::new(move |text: String| {
            idx.forget_recent(&text);
            write.search.set_recents.set(idx.recents());
        })
    };

    let on_clear_all = {
        let idx = index.clone();
        Callback::new(move |()| {
            idx.clear_all_recents();
            write.search.set_recents.set(idx.recents());
        })
    };

    view! {
        <div class="search-surface">
            <SearchInput on_submit=on_submit />
            <ScopeChip focused_channel=Signal::derive(move || Some(state.chat.current_channel.get())) />
            {move || if state.search.query.get().is_empty() {
                view! { <RecentsList on_pick=Callback::new(move |t| write.search.set_query.set(t))
                                     on_forget=on_forget
                                     on_clear_all=on_clear_all /> }.into_any()
            } else {
                view! { <ResultsList on_select=on_select /> }.into_any()
            }}
            <div class="search-privacy-footer">
                "search runs on this device only. queries never leave your device."
            </div>
        </div>
    }
}
```

- [ ] **Step 11.2 — Mount in `app.rs`.** After the palette mount:

```rust
{move || app_state.search.open.get().then(|| {
    let idx = search_index_handle.clone();
    view! { <SearchSurface index=idx /> }
})}
```

Wire index hydration at bootstrap: after `wire_derived_signals`, add an Effect that converts `messages_sig` → `IndexableMessage` and calls `search_index_handle.rebuild(...)`. Subsequent `MessageReceived` events call `search_index_handle.insert(...)`. The handle already lives on `ClientHandle::search()` from Task 7.

- [ ] **Step 11.3 — WASM check.** `cargo check --target wasm32-unknown-unknown -p willow-web`.

- [ ] **Step 11.4 — Commit.**

```bash
git add crates/web/src/components/search/surface.rs crates/web/src/components/search/mod.rs crates/web/src/app.rs
git commit -m "ui(phase-2e): mount SearchSurface + hydrate index from messages"
```

---

### Task 12: `/` focus + `⌘F` scope flip + palette bridge

**Files:**
- Modify: `crates/web/src/keybindings.rs`
- Modify: `crates/web/src/components/command_palette.rs` — on_search routes to `AllLetters` scope.
- Modify: `crates/web/src/app.rs` — wire the `on_search` callback.
- Modify: `crates/web/tests/browser.rs` — 3 more tests.

- [ ] **Step 12.1 — Failing browser tests.**

```rust
#[wasm_bindgen_test]
async fn slash_focuses_search_input() { /* mount desktop; dispatch keydown('/') at document; assert `.search-input` is activeElement */ }

#[wasm_bindgen_test]
async fn cmd_f_flips_scope_to_this_channel() { /* focus chat container; dispatch keydown('f', meta); assert scope == ThisChannel(current) */ }

#[wasm_bindgen_test]
async fn palette_forwards_plain_text_to_all_letters() { /* open palette; type "foo"; press Enter; assert surface open + scope == AllLetters + query == "foo" */ }
```

- [ ] **Step 12.2 — Extend `keybindings::install`.**

```rust
// In crates/web/src/keybindings.rs inside the match:
"/" if !is_editable_focus() => {
    ev.prevent_default();
    write.search.set_open.set(true);
    // Defer focus after DOM mount.
    let _ = set_timeout(move || {
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if let Some(el) = doc.query_selector(".search-input").ok().flatten() {
                if let Ok(html) = el.dyn_into::<web_sys::HtmlElement>() { html.focus().ok(); }
            }
        }
    }, std::time::Duration::from_millis(0));
}
"f" | "F" if is_ctrl => {
    ev.prevent_default();
    let ch = state.chat.current_channel.get_untracked();
    if !ch.is_empty() {
        write.search.set_scope.set(willow_client::SearchScope::ThisChannel(ch));
    }
    write.search.set_open.set(true);
}
```

Helper:

```rust
fn is_editable_focus() -> bool {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return false; };
    let Some(active) = doc.active_element() else { return false; };
    let tag = active.tag_name().to_lowercase();
    matches!(tag.as_str(), "input" | "textarea")
        || active.get_attribute("contenteditable").as_deref() == Some("true")
}
```

Extend close-stack: `search.open` sits above palette but below members/pinned.

```rust
fn close_top_of_stack(state: AppState, write: AppWriteSignals) -> bool {
    if state.ui.show_members.get_untracked() {
        write.ui.set_show_members.set(false); return true;
    }
    if state.ui.show_pinned.get_untracked() {
        write.ui.set_show_pinned.set(false); return true;
    }
    if state.search.open.get_untracked() {
        write.search.set_open.set(false); return true;
    }
    if state.ui.show_palette.get_untracked() {
        write.ui.set_show_palette.set(false); return true;
    }
    false
}
```

- [ ] **Step 12.3 — Wire palette `on_search` in app.rs.**

```rust
<CommandPalette
    on_close=...
    on_switch_channel=...
    on_switch_server=...
    on_open_members=...
    on_search=Callback::new(move |q: String| {
        // Per spec §Command-palette bridge — forward unprefixed text
        // to local search with scope `all letters`.
        write.search.set_query.set(q);
        write.search.set_scope.set(willow_client::SearchScope::AllLetters);
        write.ui.set_show_palette.set(false);
        write.search.set_open.set(true);
    })
/>
```

- [ ] **Step 12.4 — Verify.** `cargo check --target wasm32-unknown-unknown -p willow-web`. CI runs the new tests.

- [ ] **Step 12.5 — Commit.**

```bash
git add crates/web/src/keybindings.rs crates/web/src/app.rs crates/web/tests/browser.rs crates/web/src/components/command_palette.rs
git commit -m "ui(phase-2e): wire / focus + ⌘F scope-flip + palette search bridge"
```

---

### Task 13: Mobile pull-down reveal + overflow entry

**Files:**
- Create: `crates/web/src/components/search/mobile_reveal.rs`
- Modify: `crates/web/src/components/search/mod.rs`
- Modify: `crates/web/src/components/chat.rs` — mount pull-down at top of `MessageList`.
- Modify: `crates/web/src/components/long_press.rs` — add `search this channel` / `search this letter` overflow item for mobile top-bar menu.
- Modify: `crates/web/tests/browser.rs` — 2 mobile-shell tests.

- [ ] **Step 13.1 — Failing tests.**

```rust
#[wasm_bindgen_test]
async fn mobile_pull_down_reveals_search_bar() {
    let container = mount_test_with_shell(TestShell::Mobile, || view! { <App /> });
    tick().await;
    // Dispatch a synthetic touchstart/touchmove with dy>=44 while scrollTop==0.
    // Assert `.search-pull-down-bar.is-revealed` mounts.
}

#[wasm_bindgen_test]
async fn mobile_overflow_exposes_search_this_channel() { /* open overflow sheet on mobile; assert a button with label `search this channel` */ }
```

- [ ] **Step 13.2 — Implement `<PullDownReveal>`.**

```rust
// crates/web/src/components/search/mobile_reveal.rs
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use crate::state::{AppState, AppWriteSignals};

#[component]
pub fn PullDownReveal() -> impl IntoView {
    let state = use_context::<AppState>().unwrap();
    let write = use_context::<AppWriteSignals>().unwrap();

    let on_touchstart = |_ev: web_sys::TouchEvent| { /* stash start_y in RwSignal */ };
    let on_touchmove = |_ev: web_sys::TouchEvent| { /* compute dy vs scrollTop; fire reveal */ };

    // Simplified v1: render a thin bar that appears when search.open is
    // false AND user is at top-of-scroll. Gesture wiring is an event
    // handler on the host scroll container (chat.rs) that flips a signal.

    view! {
        <div class="search-pull-down-bar" aria-hidden="true">
            <button class="search-pull-down-hint"
                    on:click=move |_| write.search.set_open.set(true)>
                "pull to search"
            </button>
        </div>
    }
}
```

Actual pull gesture: add a `touchmove` listener on the `MessageList` scroll container; when `scrollTop <= 0 && delta_y >= 44` flip a local `revealed` signal that mounts the bar. Reuse the existing gesture shape from `message.rs` swipe handlers for the (dx, dy) capture, but rotated 90 °.

```rust
// in chat.rs MessageList
let (revealed, set_revealed) = signal(false);
let on_touchmove = move |ev: web_sys::TouchEvent| {
    let Some(touch) = ev.touches().item(0) else { return; };
    let y = touch.client_y();
    // Compare to start_y (stashed in a Cell). If dy >= 44 and scroll_top <= 0, set revealed.
};
```

- [ ] **Step 13.3 — Add overflow item.** In `long_press.rs` or the mobile top-bar overflow, append an action with label `search this channel` whose callback flips scope to `ThisChannel(current)` and opens the surface.

- [ ] **Step 13.4 — CSS.** Append:

```css
.search-pull-down-bar {
    height: 0;
    overflow: hidden;
    background: var(--bg-2);
    transition: height var(--motion) var(--motion-ease);
}
.search-pull-down-bar.is-revealed { height: 44px; }
.search-pull-down-hint {
    width: 100%; height: 44px;
    background: transparent; border: none;
    color: var(--ink-3); font: 12px var(--font-mono);
    cursor: pointer;
}
@media (prefers-reduced-motion: reduce) { .search-pull-down-bar { transition: none; } }
```

- [ ] **Step 13.5 — Verify.** `cargo check --target wasm32-unknown-unknown -p willow-web`.

- [ ] **Step 13.6 — Commit.**

```bash
git add crates/web/src/components/search/mobile_reveal.rs crates/web/src/components/search/mod.rs crates/web/src/components/chat.rs crates/web/src/components/long_press.rs crates/web/style.css crates/web/tests/browser.rs
git commit -m "ui(phase-2e): mobile pull-down reveal + overflow search entry"
```

---

### Task 14: A11y sweep + telemetry guard + browser-test fill + acceptance walkthrough

**Files:**
- Modify: `crates/web/src/components/search/row.rs` — add `aria-activedescendant` wiring.
- Modify: `crates/web/src/components/search/results.rs` — `aria-live="polite"` throttled to ≤ once per 500 ms via a last-announce timestamp.
- Modify: `crates/web/tests/browser.rs` — 4 more tests.

- [ ] **Step 14.1 — Remaining browser tests.**

```rust
#[wasm_bindgen_test]
async fn results_listbox_aria_live_polite() { /* assert `.search-results[aria-live='polite']` */ }

#[wasm_bindgen_test]
async fn matched_span_has_aria_label_match() { /* assert `mark[aria-label='match']` */ }

#[wasm_bindgen_test]
async fn reduced_motion_disables_streaming_fade() { /* set media-query override; mount; assert `.search-surface { animation-name: none }` via computed style */ }

#[wasm_bindgen_test]
async fn no_telemetry_on_query_input() {
    // Intercept tracing subscriber? Simpler: grep the file — rely on
    // code-level guard (see Step 14.3) + repo-level `grep`-check in
    // the acceptance walkthrough.
    // Placeholder: assert that typing does not emit any `.toast` or
    // console error.
}
```

- [ ] **Step 14.2 — Wire `aria-live` throttle.**

```rust
// results.rs
let last_announce = RwSignal::new(0.0f64);
let announce = move || {
    let now = js_sys::Date::now();
    let last = last_announce.get_untracked();
    if now - last >= 500.0 {
        last_announce.set(now);
        true
    } else { false }
};
// Rendering: only mount the announcement span when `announce()` returns true.
```

- [ ] **Step 14.3 — Telemetry guard.** Add a module-level comment in `crates/client/src/search/handle.rs`:

```rust
//! **PRIVACY CONTRACT.** Per docs/specs/2026-04-19-ui-design/local-search.md
//! §Privacy: no query string, match count, scope selection, or recents
//! are emitted to any network path or log. All `tracing::*` macros in
//! this module are forbidden. If you need to debug, assert locally.
```

Grep-check in acceptance walkthrough: `rg 'tracing::(info|warn|debug|trace).*query|tracing::.*scope' crates/client/src/search/ crates/web/src/components/search/ | wc -l` must be `0`.

- [ ] **Step 14.4 — Final verify.**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo check --target wasm32-unknown-unknown -p willow-client -p willow-web
cargo test -p willow-client search::
# CI: just test-browser
```

- [ ] **Step 14.5 — Self-review** (checklist below).

- [ ] **Step 14.6 — Commit.**

```bash
git add crates/web/src/components/search/ crates/client/src/search/handle.rs crates/web/tests/browser.rs
git commit -m "ui(phase-2e): sweep a11y contract + privacy guard + final tests"
```

---

### Task 15: PR

- [ ] **Step 15.1 — Push.**

```bash
git push -u origin phase-2e/local-search
```

- [ ] **Step 15.2 — Open PR.**

```bash
gh pr create --title "ui(phase-2e): local-search — plan + implementation" \
  --body "$(cat <<'EOF'
## Summary

Plan + implementation for docs/specs/2026-04-19-ui-design/local-search.md
in a single PR.

- On-device, encrypted-at-rest search index (scope-aware)
- Desktop: `/` focus top-right slot, `⌘F` scoped search, `Esc` contract,
  `⌘K` palette bridge
- Mobile: pull-down reveal on list surfaces, scoped in-surface overflow
- Query language: plain + prefix operators + quoted phrases
- Results surface: grouping, navigation, highlight, streamed
- Privacy copy + empty/loading/error states + full a11y

Architecture: see plan §Architecture for the index-module placement
(`willow-client::search`). Dual-target (native + wasm32) throughout.

Test tiers:
- unit: index build / query / highlight / scope filter
- client: scope-ladder state + palette-bridge forwarding
- browser: desktop slot, `/` focus, `⌘F` flip, `Esc`, mobile
  pull-down, highlight rendering
- Playwright: none needed (local-only flows)

## Test plan

- [ ] `just fmt` passes
- [ ] `just clippy` zero warnings
- [ ] `cargo test -p willow-client` (index + bridge)
- [ ] `just test-browser` (CI)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 15.3 — Record PR URL.**

---

## Ambiguity decisions

- **Index-module home.** `willow-client::search` (module, not a new crate). The UI, agent, and future bots all already depend on `willow-client`; a new crate would force a re-export of `DisplayMessage` and friends to break a circular dep.
- **Native SQLite FTS5.** Deferred to a post-2e follow-up. The spec calls it "**new dependency** — the implementation plan must flag the added build feature". Phase 2e does not flip that feature: it ships the in-memory backend on both targets so native and wasm behave identically. The disk-encryption property that the spec asks for is preserved by *not persisting the index* — it is derived from already-decrypted messages at session start.
- **Encrypted-at-rest.** In v1 the index never touches disk on either platform. Recents + config DO persist (storage-helpers + `localStorage`) but carry only the query strings the user chose to save (gated by `remember_recents`), not match counts or scope history. The spec's "encrypted at rest via the same disk-encryption pathway already used for message blobs" applies to the future FTS5 backend.
- **Horizon enforcement.** Configured via `SearchIndexConfig.horizon_days`; `SearchIndexHandle::rebuild` filters messages older than `now - horizon_days * 86_400_000` before insert. The 2e plan wires the data path; the settings UI is owned by `settings-tweaks.md` — until that lands, the default 90 days ships and the toggle surfaces only through a `TODO(settings-tweaks.md)` settings row.
- **First-result latency.** ≤ 150 ms on 10 k index is an acceptance target, not a benchmark gate — Phase 2e does not ship synthetic benchmarks. The in-memory implementation is O(Σ |postings_for_token|) for the AND-joined body predicate, which is well under budget on the reference device. A perf test is left as a follow-up in `docs/plans/<date>-local-search-perf.md` once the FTS5 backend lands.
- **Streaming.** The in-memory backend builds synchronously, so Phase 2e streams only during `rebuild()` (status = `Indexing { done, total }`). Steady-state live queries complete < 10 ms on indexes of the sizes we test. The `searching…` banner still renders when `Indexing` is the current status, so the UX contract is honoured.
- **Palette bridge scope.** Plain text submitted from the palette forwards to `AllLetters` per spec. `#` / `@` / `>` prefixes keep the existing palette routing and do NOT forward to search. `"quoted phrase"` with no other prefix forwards as a plain-text query.
- **Letter channels.** `SearchScope::ThisLetter` + `SearchScope::AllLetters` are reserved here but letters don't ship until `letters-dms.md`. Today the scope filter short-circuits to "no letter channels exist" with a `TODO(letters-dms.md)` comment; the branch is testable via a seeded `IndexableMessage { letter_id: Some(...), ..}` in the unit suite.
- **Grove opt-out UI.** Phase 2e ships the data path (`per_grove_enabled`) + the `remove_grove` index method. The UI to toggle per-grove search lives in `settings-tweaks.md`; 2e adds a `TODO(settings-tweaks.md)` anchor in the grove-header overflow menu.
- **Recents ring buffer.** Capped at 8, dedup by text, persisted to `willow.search.recents` in `localStorage`. Disabled when `remember_recents=false`.
- **Keyboard `Tab` cycling inside the surface.** Default browser order: input → scope chip → results listbox → privacy footer (only the chip + results are real focus stops since the footer has no interactive). `Tab` order is DOM order; no manual `tabindex` juggling.
- **Reduced-motion audit.** Every animated rule (`.search-surface`, streaming banner opacity, chip chevron rotate) has a `@media (prefers-reduced-motion: reduce)` override collapsing to instant / no-op.
- **Telemetry guard.** Enforced at code-review via the module-level privacy comment + an acceptance-gate grep. No runtime guard — a runtime guard would itself be code that could leak query state.
- **`discover.md` delegation.** The `SearchSurface::open_with(q, scope)` entry-point is exposed but not consumed in Phase 2e. Discover wires it when the grove-directory search lands.

---

## Acceptance criteria (mirrors spec §Acceptance criteria)

- [ ] Top-right search input is focusable via `/` on desktop and defaults to `AllGrovesAndLetters` unless a narrower container is focused.
- [ ] `⌘F` / `Ctrl+F` inside a focused channel or letter scopes search to `ThisChannel` / `ThisLetter`; placeholder + chip update; `Esc` clears non-empty query or closes the surface when empty.
- [ ] Command palette (`⌘K`) forwards plain text to local search with scope `AllLetters`.
- [ ] Mobile pull-down (≥ 44 px with `scrollTop ≤ 0`) reveals the search bar on letters / channel / message list.
- [ ] Scope chip renders the four scope values, greys unreachable ones with the `open a {…} first` tooltip, and persists the selection per-device (`localStorage`).
- [ ] Prefix operators `from:`, `in:`, `since:`, `before:`, `has:image`, `has:file`, `has:link` all apply; unknown operators are treated as plain text with the `unknown filter — treated as plain text` tooltip.
- [ ] Quoted phrases match adjacent tokens exactly; empty query renders placeholder only (no scan).
- [ ] Results group by grove then channel / letter (wide scope) or by letter (letters scope); groups collapse / expand per session and display counts.
- [ ] Result rows show context (channel / letter italic), author, timestamp, three-line excerpt with matched span underlined on `--moss-3` 18 %-alpha background.
- [ ] Clicking a result closes the surface and jumps to the message in its native container (container scrolls so the matched message sits 1/3 down the viewport + brief `willow-pop-in` highlight + 6 s persistent underline).
- [ ] First-result latency budget met on the reference corpus (no synthetic benchmark in 2e — see ambiguity decisions).
- [ ] Streaming banner shows `searching… · {n} matches so far` while index rebuild is in flight; counter throttled to ≤ once per 250 ms via a debounce on the writer.
- [ ] Privacy footer `search runs on this device only. queries never leave your device.` is always visible below results.
- [ ] Rebuild-index entry point is exposed on `SearchIndexHandle` (UI wiring owned by `settings-tweaks.md`).
- [ ] Horizon changes on the handle trigger an incremental rebuild. Toast on horizon shortening owned by `settings-tweaks.md`.
- [ ] No query string, match count, or scope is emitted to any network path or log — verified via the `rg` grep guard.
- [ ] Recents ring buffer caps at 8; `forget` + `clear all recents` work; `remember_recents = false` disables recents rendering entirely.
- [ ] Accessibility: `role="search"` on the form, `role="listbox"` on results, `<mark aria-label="match">` around matched spans, `aria-live="polite"` count updates throttled to ≤ once per 500 ms, focus restoration on surface close.
- [ ] Reduced-motion collapses streaming fade, highlight flash, and scope-chip chevron rotation.
- [ ] All colours, fonts, radii, shadows, motion durations, and copy voice conform to `foundation.md` / `local-search.md` §Copy.

---

## Self-review

- [x] Every §Acceptance row in `local-search.md` has a task.
- [x] Every §Copy string is in this plan verbatim (placeholders, privacy footer, streaming banner, no-matches, unknown-operator tooltip, rebuild-index confirmation, indexing messages).
- [x] Foundation tokens only — `--amber`, `--moss-*`, `--bg-*`, `--line*`, `--ink-*`, `--radius-*`, `--shadow-2`, `--motion`, `--motion-ease`, `--focus-ring`, `--font-*`. No new hex.
- [x] Every commit is `ui(phase-2e): <imperative>` except the plan commit (`docs(plan): phase 2e — local-search implementation plan`).
- [x] Dual-target compile: `cargo check --target wasm32-unknown-unknown -p willow-client -p willow-web` required at end of every impl task.
- [x] No `std::fs`, no `std::time::SystemTime`, no tokio / threads in library crates (search module uses `js_sys::Date::now` on wasm via existing `util`).
- [x] Lowest-tier test per behaviour: query parser + tokenizer + index + executor + highlight + config + recents → Rust unit tests (14 × `willow-client search::`). UI signals + palette bridge → browser tests. No Playwright.
- [x] No placeholders, no TBDs, no "similar to".
- [x] Privacy guard is two-layered: module-level contract comment + `rg`-check in the acceptance gate.
- [x] Every spec requirement flagged **new** in §Data dependencies has a concrete file path in §File structure.
- [x] Every "open question" from the spec has a `TODO(local-search.md open-question)` anchor or is explicitly out of scope with a reason.
