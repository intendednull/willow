# Event-Based Waits — PR 1: test-hooks Foundation

> **2026-04-28: ERRATA APPLIES.** Read `docs/plans/2026-04-28-event-based-waits-pr1-errata.md` alongside this plan. Where they conflict, the errata wins. Several API assumptions in the original plan were wrong (no sync DAG read path, MemNetwork is native-only, `to_hex()` doesn't exist, `member_count` field doesn't exist). The errata documents the corrected pattern with file:line citations.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the cargo-feature-gated `test-hooks` infrastructure (`WillowTestHooks` WASM API + push dispatcher), the symbol-leak guard, the justfile `FEATURES` parameterisation, the ESLint rule for `waitForTimeout`, and the per-spec allowlist headers. No spec migrations yet; that work is PR 2.

**Architecture:** New module `crates/web/src/test_hooks.rs` (gated `#[cfg(feature = "test-hooks")]`) exposes `WillowTestHooks` to JS via `#[wasm_bindgen]` for pull-based snapshot/heads/event-count queries, and spawns a push dispatcher that subscribes to `ClientHandle::subscribe_events()` and forwards a stable wire-shape JSON to `window.__willowEvent` (a Playwright `exposeBinding`). Mount happens in `app.rs` after `with_trust_store`. The `test-hooks` feature is **off in production**: a CI script greps the `dist/*.js` output of the default `trunk build --release` to assert `WillowTestHooks` is absent.

**Tech Stack:** Rust + `wasm-bindgen` + `serde_wasm_bindgen`, Leptos, `wasm-pack` browser tests, justfile, ESLint.

**Spec reference:** `docs/specs/2026-04-27-event-based-waits-design.md`. The spec is the source of truth for the wire shape, the data-state lifecycle, and the rejected alternatives. This plan implements the PR-1 scope only.

---

## Pre-flight

### Task 0.1: Audit iroh for `performance.now` usage

Per spec section "iroh timer verification (PR 1 acceptance gate)". This is a one-off check: if iroh's WASM transport reads `performance.now()`, `page.clock` install would freeze JS time but not iroh, causing silent divergence in tests that install the clock.

**Files:**
- Modify: PR description (record audit result)

- [ ] **Step 1: Run the audit**

```bash
cargo metadata --format-version 1 \
  | jq -r '.packages[] | select(.name | startswith("iroh")) | .manifest_path' \
  | xargs -I{} dirname {} \
  | xargs -I{} grep -rn 'performance' {} 2>/dev/null \
  | grep -v test \
  | head -50
```

Expected: Either no matches (clean — `page.clock` covers iroh) or matches in retry/backoff code (constrain `page.clock` install to single-peer scopes only).

- [ ] **Step 2: Record the audit result in the PR description**

Paste the output (or "no matches") into the PR description under a heading "iroh `performance.now` audit". This survives review and is referenced by the spec.

### Task 0.2: Open the GitHub tracking issue

Per spec section "Tracking issue". The URL is needed by Phase 7's `eslint-disable` headers, so the issue must exist before PR 1 lands.

**Files:**
- (External: GitHub)

- [ ] **Step 1: Create the issue**

Title: `e2e: migrate remaining specs to event-based waits`

Body:
```markdown
Tracks migration of the 7 remaining Playwright spec files from time-based
to event-based waits. Spec: `docs/specs/2026-04-27-event-based-waits-design.md`.

**Sunset: 2026-09-30.** After this date the ratchet script flips to
hard-fail at 0 `waitForTimeout` calls.

- [ ] `e2e/permissions.spec.ts`
- [ ] `e2e/mobile.spec.ts`
- [ ] `e2e/mobile-actions.spec.ts`
- [ ] `e2e/multi-peer-mobile.spec.ts`
- [ ] `e2e/cross-browser-sync.spec.ts`
- [ ] `e2e/join-links.spec.ts`
- [ ] `e2e/worker-nodes.spec.ts`
```

- [ ] **Step 2: Capture the issue URL**

Save the URL (e.g. `https://github.com/intendednull/willow/issues/N`). Used in Phase 7 `eslint-disable` headers.

---

## Phase 1: Cargo feature scaffold

### Task 1.1: Add the `test-hooks` cargo feature

**Files:**
- Modify: `crates/web/Cargo.toml`

- [ ] **Step 1: Add the `[features]` section**

Insert at the bottom of the file (after `[dev-dependencies]`):

```toml
[features]
default = []
test-hooks = ["dep:serde_wasm_bindgen"]
```

And add the optional dependency to `[dependencies]`:

```toml
serde_wasm_bindgen = { version = "0.6", optional = true }
```

(`serde_wasm_bindgen` is gated as an optional dep so the prod build doesn't pay for it.)

- [ ] **Step 2: Verify both build configurations compile**

Run:
```bash
cargo check -p willow-web
cargo check -p willow-web --features test-hooks
```

Expected: both succeed with zero warnings.

- [ ] **Step 3: Commit**

```bash
git add crates/web/Cargo.toml
git commit -m "feat(web): add test-hooks cargo feature"
```

### Task 1.2: Create the gated module skeleton

**Files:**
- Create: `crates/web/src/test_hooks.rs`
- Modify: `crates/web/src/lib.rs`

- [ ] **Step 1: Write `crates/web/src/test_hooks.rs`**

```rust
//! Test instrumentation for the Willow web UI.
//!
//! This module is gated behind the `test-hooks` cargo feature and is
//! **never compiled into production builds**. It exposes
//! `WillowTestHooks` to JavaScript via `wasm_bindgen` so Playwright
//! e2e tests can synchronise on real signals (applied events, DAG
//! heads, snapshot fields) instead of arbitrary `waitForTimeout`s.
//!
//! See `docs/specs/2026-04-27-event-based-waits-design.md`.

#![cfg(feature = "test-hooks")]

use wasm_bindgen::prelude::*;

/// Read-only test instrumentation handle exposed to JS as `window.__willow`.
#[wasm_bindgen]
pub struct WillowTestHooks {
    // ClientHandle field added in Task 2.2.
    _placeholder: (),
}
```

- [ ] **Step 2: Wire the module into `crates/web/src/lib.rs`**

Find the existing `pub mod` declarations and add:

```rust
#[cfg(feature = "test-hooks")]
pub mod test_hooks;
```

- [ ] **Step 3: Verify both build configurations still compile**

```bash
cargo check -p willow-web
cargo check -p willow-web --features test-hooks
```

Expected: both pass.

- [ ] **Step 4: Commit**

```bash
git add crates/web/src/test_hooks.rs crates/web/src/lib.rs
git commit -m "feat(web): add gated test_hooks module skeleton"
```

---

## Phase 2: Pull API — `WillowTestHooks` snapshot / heads / event_count / last_event

The pull API serializes a `SnapshotDto` (defined in this phase) plus the existing `HeadsSummary` from `crates/state/src/sync.rs:22`. The DTO uses `#[serde(rename_all = "camelCase")]` so the JS-side field names match the spec's TypeScript `Snapshot` interface without modifying the state crate.

### Task 2.1: Define the snapshot DTO

**Files:**
- Create: `crates/web/src/test_hooks/mod.rs` (renamed from `test_hooks.rs`)
- Create: `crates/web/src/test_hooks/snapshot.rs`
- Modify: `crates/web/src/test_hooks.rs` → move to `crates/web/src/test_hooks/mod.rs`

- [ ] **Step 1: Convert `test_hooks.rs` to a module directory**

```bash
mkdir -p crates/web/src/test_hooks
git mv crates/web/src/test_hooks.rs crates/web/src/test_hooks/mod.rs
```

- [ ] **Step 2: Add the snapshot DTO file**

Write `crates/web/src/test_hooks/snapshot.rs`:

```rust
//! DTOs for the `WillowTestHooks` pull API.
//!
//! These mirror the TypeScript `Snapshot` / `AuthorHead` types defined
//! in `e2e/test-hooks.ts`. Field names are camelCase to match TS
//! convention; the kind discriminator on `ClientEvent` (a separate
//! module) stays PascalCase.

use serde::Serialize;

/// One author's DAG head, as exposed to JS.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorHeadDto {
    pub seq: u64,
    pub hash: String,
}

/// One channel's summary, as exposed to JS.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDto {
    pub name: String,
    pub member_count: u32,
}

/// Aggregated state snapshot for `expect.poll` matchers.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDto {
    pub event_count: u32,
    /// Per-author DAG heads, keyed by `EndpointId` hex string.
    pub heads: std::collections::BTreeMap<String, AuthorHeadDto>,
    pub last_event: Option<String>,
    pub channels: Vec<ChannelDto>,
}
```

- [ ] **Step 3: Wire the submodule into `crates/web/src/test_hooks/mod.rs`**

At the top of `mod.rs`:

```rust
mod snapshot;
pub use snapshot::{AuthorHeadDto, ChannelDto, SnapshotDto};
```

- [ ] **Step 4: Verify it compiles under both configurations**

```bash
cargo check -p willow-web
cargo check -p willow-web --features test-hooks
```

Expected: both pass.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src/test_hooks/
git commit -m "feat(web): add SnapshotDto / AuthorHeadDto / ChannelDto"
```

### Task 2.2: Failing browser test for `event_count` and `last_event`

The pull API's first observable behaviour: after the client applies its `CreateServer` genesis event, `event_count == 1` and `last_event == Some(<hash>)`.

**Files:**
- Create: `crates/web/tests/test_hooks_browser.rs`

- [ ] **Step 1: Write the test file**

```rust
//! In-browser tests for `WillowTestHooks`.
//!
//! Run with:
//!   wasm-pack test crates/web --headless --chrome --features test-hooks
//!
//! These tests construct a real `ClientHandle` (no networking — uses
//! `MemNetwork`) and assert the test-hooks API observes the expected
//! shape after applying known events.

#![cfg(feature = "test-hooks")]

use wasm_bindgen_test::*;
use willow_client::ClientHandle;
use willow_network::mem::MemNetwork;
use willow_web::test_hooks::WillowTestHooks;

wasm_bindgen_test_configure!(run_in_browser);

async fn fresh_client() -> ClientHandle<MemNetwork> {
    // Helper: spin up a ClientHandle backed by MemNetwork and apply
    // CreateServer so the DAG is non-empty. Returns the handle.
    let network = MemNetwork::new();
    let config = willow_client::ClientConfig::ephemeral_with_network(network);
    let (handle, _event_loop) = ClientHandle::new(config);
    handle.create_server("test-server").await.unwrap();
    handle
}

#[wasm_bindgen_test]
async fn snapshot_event_count_and_last_event_after_create_server() {
    let handle = fresh_client().await;
    let hooks = WillowTestHooks::new(handle);

    assert_eq!(hooks.event_count(), 1, "CreateServer should be event #1");
    assert!(
        hooks.last_event().is_some(),
        "last_event should be Some after CreateServer"
    );
}
```

- [ ] **Step 2: Run the test to confirm it fails**

```bash
wasm-pack test crates/web --headless --chrome --features test-hooks --test test_hooks_browser
```

Expected: compile error or test failure — `WillowTestHooks::new` does not yet accept a `ClientHandle`.

### Task 2.3: Implement `event_count` and `last_event`

**Files:**
- Modify: `crates/web/src/test_hooks/mod.rs`

- [ ] **Step 1: Replace the placeholder struct**

Update `crates/web/src/test_hooks/mod.rs`:

```rust
#![cfg(feature = "test-hooks")]

mod snapshot;
pub use snapshot::{AuthorHeadDto, ChannelDto, SnapshotDto};

use wasm_bindgen::prelude::*;
use willow_client::ClientHandle;
use willow_network::Network;

/// Read-only test instrumentation handle exposed to JS as `window.__willow`.
///
/// Generic over `Network` so unit tests can construct it with `MemNetwork`
/// and the production mount in `app.rs` uses `IrohNetwork`.
#[wasm_bindgen]
pub struct WillowTestHooks {
    // SendWrapper not needed here: WillowTestHooks is only ever held on
    // the main browser thread (single-threaded WASM). The handle is a
    // pure data accessor; no async work runs through this type directly.
    inner: WillowTestHooksInner,
}

struct WillowTestHooksInner {
    /// Type-erased handle. Only the methods used by the pull API are
    /// invoked; the dispatcher (Phase 4) holds its own clone of the
    /// concrete-typed handle.
    event_count_fn: Box<dyn Fn() -> u32>,
    last_event_fn: Box<dyn Fn() -> Option<String>>,
    snapshot_fn: Box<dyn Fn() -> SnapshotDto>,
    heads_fn: Box<dyn Fn() -> std::collections::BTreeMap<String, AuthorHeadDto>>,
}

impl WillowTestHooks {
    /// Construct from any `ClientHandle<N>`. Captures the handle in
    /// closures so the wasm_bindgen-exposed methods can stay
    /// monomorphic.
    pub fn new<N: Network + 'static>(handle: ClientHandle<N>) -> Self {
        let h_event_count = handle.clone();
        let h_last_event = handle.clone();
        let h_snapshot = handle.clone();
        let h_heads = handle.clone();
        Self {
            inner: WillowTestHooksInner {
                event_count_fn: Box::new(move || {
                    h_event_count.dag_event_count() as u32
                }),
                last_event_fn: Box::new(move || {
                    h_last_event.dag_last_event_hash().map(|h| h.to_hex())
                }),
                snapshot_fn: Box::new(move || snapshot::build(&h_snapshot)),
                heads_fn: Box::new(move || snapshot::build_heads(&h_heads)),
            },
        }
    }
}

#[wasm_bindgen]
impl WillowTestHooks {
    /// Total events applied to the local DAG.
    pub fn event_count(&self) -> u32 {
        (self.inner.event_count_fn)()
    }

    /// Hex-encoded hash of the most recently applied event, or `None`.
    pub fn last_event(&self) -> Option<String> {
        (self.inner.last_event_fn)()
    }
}
```

- [ ] **Step 2: Add the supporting accessors `dag_event_count` + `dag_last_event_hash` to `ClientHandle`**

These are pure read accessors over the existing DAG. Add to `crates/client/src/accessors.rs`:

```rust
impl<N: Network> ClientHandle<N> {
    /// Total events applied to the local DAG. Used by test-hooks; cheap
    /// O(1) read of an actor-held counter.
    pub fn dag_event_count(&self) -> usize {
        self.shared.dag_event_count()
    }

    /// Hash of the most recently applied event across all authors, or
    /// `None` if the DAG is empty.
    pub fn dag_last_event_hash(&self) -> Option<willow_state::EventHash> {
        self.shared.dag_last_event_hash()
    }
}
```

(The `shared` field is the existing `Arc<ClientShared>`; it already exposes the DAG via the actor read-side. Mirror an existing accessor pattern from the same file for the underlying impl.)

- [ ] **Step 3: Add the two minimal `snapshot::build` / `build_heads` stubs that the test in Task 2.2 will exercise**

Append to `crates/web/src/test_hooks/snapshot.rs`:

```rust
use willow_client::ClientHandle;
use willow_network::Network;

pub(crate) fn build<N: Network>(handle: &ClientHandle<N>) -> SnapshotDto {
    SnapshotDto {
        event_count: handle.dag_event_count() as u32,
        heads: build_heads(handle),
        last_event: handle.dag_last_event_hash().map(|h| h.to_hex()),
        channels: handle
            .channels_view()
            .into_iter()
            .map(|ch| ChannelDto {
                name: ch.name,
                member_count: ch.member_count as u32,
            })
            .collect(),
    }
}

pub(crate) fn build_heads<N: Network>(
    handle: &ClientHandle<N>,
) -> std::collections::BTreeMap<String, AuthorHeadDto> {
    handle
        .dag_heads_summary()
        .heads
        .into_iter()
        .map(|(endpoint, head)| {
            (
                endpoint.to_hex(),
                AuthorHeadDto {
                    seq: head.seq,
                    hash: head.hash.to_hex(),
                },
            )
        })
        .collect()
}
```

- [ ] **Step 4: Add `dag_heads_summary()` and `channels_view()` accessors if not already public**

Check `crates/client/src/accessors.rs`. If `heads_summary` and a channel-list accessor are not already public, expose them now (mirror the spec's intent — the data is already computed; this just publishes it).

- [ ] **Step 5: Run the test from Task 2.2 again**

```bash
wasm-pack test crates/web --headless --chrome --features test-hooks --test test_hooks_browser
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/web/src/test_hooks/ crates/web/tests/test_hooks_browser.rs crates/client/src/accessors.rs
git commit -m "feat(web): add WillowTestHooks event_count + last_event"
```

### Task 2.4: Failing test for `heads()`

**Files:**
- Modify: `crates/web/tests/test_hooks_browser.rs`

- [ ] **Step 1: Add the test**

```rust
#[wasm_bindgen_test]
async fn heads_returns_one_author_after_create_server() {
    let handle = fresh_client().await;
    let hooks = WillowTestHooks::new(handle);

    let heads_js = hooks.heads().expect("heads should serialize");
    let heads: std::collections::BTreeMap<String, willow_web::test_hooks::AuthorHeadDto> =
        serde_wasm_bindgen::from_value(heads_js).expect("deserialize");

    assert_eq!(heads.len(), 1, "exactly one author after CreateServer");
    let (_id, head) = heads.iter().next().unwrap();
    assert_eq!(head.seq, 0, "genesis seq is 0");
    assert!(!head.hash.is_empty(), "hash is set");
}
```

- [ ] **Step 2: Run, expect compile error / fail**

```bash
wasm-pack test crates/web --headless --chrome --features test-hooks --test test_hooks_browser
```

Expected: FAIL — `WillowTestHooks::heads` is not yet exposed to JS.

### Task 2.5: Implement `heads()` on the JS surface

**Files:**
- Modify: `crates/web/src/test_hooks/mod.rs`

- [ ] **Step 1: Add the method to the `#[wasm_bindgen] impl` block**

```rust
#[wasm_bindgen]
impl WillowTestHooks {
    // …existing methods…

    /// Per-author DAG heads. Stable across calls when the DAG is unchanged.
    pub fn heads(&self) -> Result<JsValue, JsValue> {
        let heads = (self.inner.heads_fn)();
        serde_wasm_bindgen::to_value(&heads).map_err(Into::into)
    }
}
```

- [ ] **Step 2: Run the test, expect PASS**

```bash
wasm-pack test crates/web --headless --chrome --features test-hooks --test test_hooks_browser
```

Expected: PASS for both `snapshot_event_count_and_last_event_after_create_server` and `heads_returns_one_author_after_create_server`.

### Task 2.6: Failing test for full `snapshot()`

**Files:**
- Modify: `crates/web/tests/test_hooks_browser.rs`

- [ ] **Step 1: Add the test**

```rust
#[wasm_bindgen_test]
async fn snapshot_returns_full_dto_with_one_channel() {
    let handle = fresh_client().await;
    handle.create_channel("general").await.unwrap();
    let hooks = WillowTestHooks::new(handle);

    let snap_js = hooks.snapshot().expect("snapshot should serialize");
    let snap: willow_web::test_hooks::SnapshotDto =
        serde_wasm_bindgen::from_value(snap_js).expect("deserialize");

    assert_eq!(snap.event_count, 2, "CreateServer + CreateChannel = 2");
    assert_eq!(snap.heads.len(), 1, "still one author");
    assert!(snap.last_event.is_some());
    assert_eq!(snap.channels.len(), 1);
    assert_eq!(snap.channels[0].name, "general");
}
```

- [ ] **Step 2: Run, expect FAIL** — `snapshot()` not yet exposed.

### Task 2.7: Implement `snapshot()` on the JS surface

**Files:**
- Modify: `crates/web/src/test_hooks/mod.rs`

- [ ] **Step 1: Add the method**

```rust
#[wasm_bindgen]
impl WillowTestHooks {
    // …existing methods…

    /// Aggregated state snapshot for `expect.poll` matchers.
    pub fn snapshot(&self) -> Result<JsValue, JsValue> {
        let snap = (self.inner.snapshot_fn)();
        serde_wasm_bindgen::to_value(&snap).map_err(Into::into)
    }
}
```

- [ ] **Step 2: Run, expect PASS**

- [ ] **Step 3: Commit**

```bash
git add crates/web/src/test_hooks/ crates/web/tests/test_hooks_browser.rs
git commit -m "feat(web): add WillowTestHooks heads + snapshot pull API"
```

---

## Phase 3: `ClientEvent` wire-shape conversion

Per spec section "Stable JSON wire shape for `ClientEvent`", `test_hooks` defines a hand-written conversion from the Rust `ClientEvent` enum to a stable `{kind: <PascalCase>, ...flat camelCase fields}` JSON shape. Internal-only variants (`QueueChanged`, `VoiceSignal`, etc.) are filtered out — `to_wire()` returns `None` for them.

### Task 3.1: Failing unit test for `SyncCompleted` conversion

**Files:**
- Create: `crates/web/src/test_hooks/wire.rs`
- Modify: `crates/web/src/test_hooks/mod.rs`

- [ ] **Step 1: Add `pub mod wire;` to `crates/web/src/test_hooks/mod.rs`**

Insert near the top:
```rust
mod wire;
pub use wire::{to_wire, WireEvent};
```

- [ ] **Step 2: Write `crates/web/src/test_hooks/wire.rs` with a stub + a unit test**

```rust
//! Stable JSON wire shape for `ClientEvent`.
//!
//! `to_wire(event)` returns `Some(WireEvent)` for variants exposed to
//! e2e tests, and `None` for internal-only variants. The `WireEvent`
//! shape is `{kind: <PascalCase>, ...camelCase fields}` per the spec.

use serde::Serialize;
use willow_client::events::ClientEvent;

/// JSON-stable representation of a `ClientEvent` for the test surface.
///
/// Each variant flattens into `{kind, ...fields}`. The `kind`
/// discriminator is PascalCase (matches Rust variant names); other
/// fields are camelCase (matches TypeScript convention).
#[derive(Serialize)]
#[serde(tag = "kind")]
pub enum WireEvent {
    SyncCompleted {
        #[serde(rename = "opsApplied")]
        ops_applied: u32,
    },
    // …other variants added in Task 3.3…
}

/// Convert a `ClientEvent` to its wire shape, or `None` if the variant
/// is internal-only.
pub fn to_wire(event: &ClientEvent) -> Option<WireEvent> {
    match event {
        ClientEvent::SyncCompleted { ops_applied } => Some(WireEvent::SyncCompleted {
            ops_applied: *ops_applied as u32,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_completed_serializes_to_stable_shape() {
        let ev = ClientEvent::SyncCompleted { ops_applied: 5 };
        let wire = to_wire(&ev).expect("SyncCompleted must convert");
        let json = serde_json::to_string(&wire).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"SyncCompleted","opsApplied":5}"#,
        );
    }
}
```

- [ ] **Step 3: Run the test**

```bash
cargo test -p willow-web --features test-hooks --lib test_hooks::wire
```

Expected: PASS (this task already includes the implementation; the test verifies the shape).

### Task 3.2: Failing tests for the remaining 9 wire-visible variants

The spec lists 10 variants total. We just covered `SyncCompleted`; this task adds tests for the other 9, all expected to fail until Task 3.3 implements them.

**Files:**
- Modify: `crates/web/src/test_hooks/wire.rs`

- [ ] **Step 1: Add 9 failing tests in the existing `mod tests`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::EndpointId;

    fn endpoint_a() -> EndpointId {
        // 32-byte all-ones for a stable test fixture.
        EndpointId::from_bytes([1u8; 32])
    }

    #[test]
    fn message_received() {
        let ev = ClientEvent::MessageReceived {
            channel: "general".into(),
            message_id: "m1".into(),
            is_local: false,
        };
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"MessageReceived","channel":"general","messageId":"m1","isLocal":false}"#,
        );
    }

    #[test]
    fn peer_connected() {
        let ev = ClientEvent::PeerConnected(endpoint_a());
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert!(json.starts_with(r#"{"kind":"PeerConnected","peerId":"#));
    }

    #[test]
    fn peer_disconnected() {
        let ev = ClientEvent::PeerDisconnected(endpoint_a());
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert!(json.starts_with(r#"{"kind":"PeerDisconnected","peerId":"#));
    }

    #[test]
    fn channel_created() {
        let ev = ClientEvent::ChannelCreated("general".into());
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert_eq!(json, r#"{"kind":"ChannelCreated","name":"general"}"#);
    }

    #[test]
    fn channel_deleted() {
        let ev = ClientEvent::ChannelDeleted("general".into());
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert_eq!(json, r#"{"kind":"ChannelDeleted","name":"general"}"#);
    }

    #[test]
    fn peer_trusted() {
        let ev = ClientEvent::PeerTrusted(endpoint_a());
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert!(json.starts_with(r#"{"kind":"PeerTrusted","peerId":"#));
    }

    #[test]
    fn peer_untrusted() {
        let ev = ClientEvent::PeerUntrusted(endpoint_a());
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert!(json.starts_with(r#"{"kind":"PeerUntrusted","peerId":"#));
    }

    #[test]
    fn profile_updated() {
        let ev = ClientEvent::ProfileUpdated {
            peer_id: endpoint_a(),
            display_name: "alice".into(),
        };
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert!(json.contains(r#""kind":"ProfileUpdated""#));
        assert!(json.contains(r#""displayName":"alice""#));
    }

    #[test]
    fn role_created() {
        let ev = ClientEvent::RoleCreated {
            name: "moderator".into(),
            role_id: "r1".into(),
        };
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"RoleCreated","roleId":"r1","name":"moderator"}"#,
        );
    }

    // Existing test — keep it.
    #[test]
    fn sync_completed_serializes_to_stable_shape() {
        let ev = ClientEvent::SyncCompleted { ops_applied: 5 };
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"SyncCompleted","opsApplied":5}"#,
        );
    }
}
```

- [ ] **Step 2: Run, expect 9 failures**

```bash
cargo test -p willow-web --features test-hooks --lib test_hooks::wire
```

Expected: 9 of 10 tests fail because the variants are not yet implemented.

### Task 3.3: Implement the 9 remaining wire variants

**Files:**
- Modify: `crates/web/src/test_hooks/wire.rs`

- [ ] **Step 1: Replace the `WireEvent` enum and `to_wire` body**

```rust
#[derive(Serialize)]
#[serde(tag = "kind")]
pub enum WireEvent {
    SyncCompleted {
        #[serde(rename = "opsApplied")]
        ops_applied: u32,
    },
    MessageReceived {
        channel: String,
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(rename = "isLocal")]
        is_local: bool,
    },
    PeerConnected {
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    PeerDisconnected {
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    ChannelCreated {
        name: String,
    },
    ChannelDeleted {
        name: String,
    },
    PeerTrusted {
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    PeerUntrusted {
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    ProfileUpdated {
        #[serde(rename = "peerId")]
        peer_id: String,
        #[serde(rename = "displayName")]
        display_name: String,
    },
    RoleCreated {
        #[serde(rename = "roleId")]
        role_id: String,
        name: String,
    },
}

pub fn to_wire(event: &ClientEvent) -> Option<WireEvent> {
    match event {
        ClientEvent::SyncCompleted { ops_applied } => Some(WireEvent::SyncCompleted {
            ops_applied: *ops_applied as u32,
        }),
        ClientEvent::MessageReceived {
            channel,
            message_id,
            is_local,
        } => Some(WireEvent::MessageReceived {
            channel: channel.clone(),
            message_id: message_id.clone(),
            is_local: *is_local,
        }),
        ClientEvent::PeerConnected(id) => Some(WireEvent::PeerConnected {
            peer_id: id.to_hex(),
        }),
        ClientEvent::PeerDisconnected(id) => Some(WireEvent::PeerDisconnected {
            peer_id: id.to_hex(),
        }),
        ClientEvent::ChannelCreated(name) => Some(WireEvent::ChannelCreated {
            name: name.clone(),
        }),
        ClientEvent::ChannelDeleted(name) => Some(WireEvent::ChannelDeleted {
            name: name.clone(),
        }),
        ClientEvent::PeerTrusted(id) => Some(WireEvent::PeerTrusted {
            peer_id: id.to_hex(),
        }),
        ClientEvent::PeerUntrusted(id) => Some(WireEvent::PeerUntrusted {
            peer_id: id.to_hex(),
        }),
        ClientEvent::ProfileUpdated {
            peer_id,
            display_name,
        } => Some(WireEvent::ProfileUpdated {
            peer_id: peer_id.to_hex(),
            display_name: display_name.clone(),
        }),
        ClientEvent::RoleCreated { name, role_id } => Some(WireEvent::RoleCreated {
            role_id: role_id.clone(),
            name: name.clone(),
        }),
        // Internal-only variants are filtered out.
        _ => None,
    }
}
```

- [ ] **Step 2: Run all wire tests, expect PASS**

```bash
cargo test -p willow-web --features test-hooks --lib test_hooks::wire
```

Expected: 10 of 10 pass.

### Task 3.4: Failing test that internal variants are filtered

**Files:**
- Modify: `crates/web/src/test_hooks/wire.rs`

- [ ] **Step 1: Add the test**

```rust
#[test]
fn internal_variants_are_filtered() {
    use willow_client::queue::RelayStatus;
    let ev = ClientEvent::RelayStatusChanged(RelayStatus::Connected);
    assert!(
        to_wire(&ev).is_none(),
        "internal-only variants must not leak to the wire"
    );
}
```

- [ ] **Step 2: Run, expect PASS** (the catch-all `_ => None` arm already covers this; the test guards against future regressions where someone accidentally adds a wire-visible arm for an internal variant).

- [ ] **Step 3: Commit**

```bash
git add crates/web/src/test_hooks/
git commit -m "feat(web): add ClientEvent wire-shape conversion"
```

---

## Phase 4: Push dispatcher

The dispatcher subscribes to `ClientHandle::subscribe_events()` (`crates/client/src/accessors.rs:10`, returns `EventReceiver` from `crates/client/src/lib.rs:120`), converts each `ClientEvent` to `WireEvent`, and forwards to `window.__willowEvent`. Buffer with capacity 65 536; overflow calls `window.__willowOverflow(droppedCount)`. Drain on three edges per spec: dispatcher init, every dispatch, and the Playwright fixture's read-side binding.

### Task 4.1: Failing browser test that the dispatcher emits to a JS callback

**Files:**
- Modify: `crates/web/tests/test_hooks_browser.rs`

- [ ] **Step 1: Add the test**

```rust
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsValue;

#[wasm_bindgen_test]
async fn dispatcher_emits_sync_completed_to_window_callback() {
    let captured: Rc<RefCell<Vec<JsValue>>> = Rc::new(RefCell::new(Vec::new()));
    let captured_clone = captured.clone();

    let cb = Closure::wrap(Box::new(move |ev: JsValue| {
        captured_clone.borrow_mut().push(ev);
    }) as Box<dyn FnMut(JsValue)>);

    let window = web_sys::window().unwrap();
    js_sys::Reflect::set(&window, &"__willowEvent".into(), cb.as_ref()).unwrap();
    cb.forget();

    let handle = fresh_client().await;
    let _dispatcher =
        willow_web::test_hooks::install_push_dispatcher(handle.clone());

    // Trigger a SyncCompleted by applying another event.
    handle.create_channel("general").await.unwrap();

    // Yield to let the dispatcher loop run.
    gloo_timers::future::TimeoutFuture::new(50).await;

    let events = captured.borrow();
    assert!(
        events.iter().any(|ev| {
            let s = js_sys::JSON::stringify(ev).unwrap().as_string().unwrap();
            s.contains(r#""kind":"SyncCompleted""#)
        }),
        "expected at least one SyncCompleted event; got {:?}",
        events
            .iter()
            .map(|ev| js_sys::JSON::stringify(ev).unwrap().as_string().unwrap())
            .collect::<Vec<_>>()
    );

    // Cleanup.
    js_sys::Reflect::delete_property(&window, &"__willowEvent".into()).unwrap();
}
```

- [ ] **Step 2: Run, expect FAIL** (compile error — `install_push_dispatcher` does not exist).

```bash
wasm-pack test crates/web --headless --chrome --features test-hooks --test test_hooks_browser
```

### Task 4.2: Implement the dispatcher with init + per-dispatch drain

**Files:**
- Create: `crates/web/src/test_hooks/dispatcher.rs`
- Modify: `crates/web/src/test_hooks/mod.rs`

- [ ] **Step 1: Add the dispatcher module**

Write `crates/web/src/test_hooks/dispatcher.rs`:

```rust
//! Push dispatcher for `WillowTestHooks`.
//!
//! Subscribes to `ClientHandle::subscribe_events()`, converts each
//! `ClientEvent` to its wire shape, and forwards to
//! `window.__willowEvent` (a Playwright `exposeBinding`). On overflow
//! calls `window.__willowOverflow(droppedCount)` so the test fixture
//! can fail the test immediately.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;
use willow_client::ClientHandle;
use willow_network::Network;

use super::wire::to_wire;

const BUFFER_CAPACITY: usize = 65_536;

/// Returned from `install_push_dispatcher`. Dropping aborts the loop.
pub struct DispatcherHandle {
    abort: Rc<RefCell<bool>>,
}

impl Drop for DispatcherHandle {
    fn drop(&mut self) {
        *self.abort.borrow_mut() = true;
    }
}

/// Install the push dispatcher. Spawns a `wasm_bindgen_futures` task
/// that loops on the broker `recv()`, converts events, and forwards
/// them to `window.__willowEvent`.
pub fn install_push_dispatcher<N: Network + 'static>(
    handle: ClientHandle<N>,
) -> DispatcherHandle {
    let abort = Rc::new(RefCell::new(false));
    let abort_clone = abort.clone();

    spawn_local(async move {
        let mut rx = handle.subscribe_events().await;

        // Drain on dispatcher init: covers the case where a previous
        // dispatcher buffered events into __willowEventBuffer before
        // being aborted (hot reload, auth re-init).
        drain_buffer_into_callback();

        while !*abort_clone.borrow() {
            let Some(event) = rx.recv().await else { break };
            let Some(wire) = to_wire(&event) else { continue };

            let js = match serde_wasm_bindgen::to_value(&wire) {
                Ok(v) => v,
                Err(e) => {
                    web_sys::console::error_1(
                        &format!("test-hooks: serialize failed: {e:?}").into(),
                    );
                    continue;
                }
            };

            // Drain on every dispatch: covers the case where the
            // binding became available after some events were already
            // buffered.
            drain_buffer_into_callback();
            dispatch_or_buffer(js);
        }
    });

    DispatcherHandle { abort }
}

/// Try to call `window.__willowEvent(js)`. If the binding is absent,
/// push the value into `window.__willowEventBuffer` so a future drain
/// can deliver it.
fn dispatch_or_buffer(js: JsValue) {
    let Some(window) = web_sys::window() else { return };

    if let Ok(callback) = js_sys::Reflect::get(&window, &"__willowEvent".into()) {
        if let Some(func) = callback.dyn_ref::<js_sys::Function>() {
            let _ = func.call1(&JsValue::NULL, &js);
            return;
        }
    }

    push_into_buffer(&window, js);
}

fn drain_buffer_into_callback() {
    let Some(window) = web_sys::window() else { return };

    let Ok(callback) = js_sys::Reflect::get(&window, &"__willowEvent".into()) else {
        return;
    };
    let Some(func) = callback.dyn_ref::<js_sys::Function>() else {
        return;
    };

    let Ok(buffer) = js_sys::Reflect::get(&window, &"__willowEventBuffer".into()) else {
        return;
    };
    let Some(arr) = buffer.dyn_ref::<js_sys::Array>() else {
        return;
    };

    while arr.length() > 0 {
        let item = arr.shift();
        let _ = func.call1(&JsValue::NULL, &item);
    }
}

fn push_into_buffer(window: &web_sys::Window, js: JsValue) {
    let buffer = match js_sys::Reflect::get(window, &"__willowEventBuffer".into()) {
        Ok(b) if b.is_object() => b,
        _ => {
            let arr = js_sys::Array::new();
            let _ = js_sys::Reflect::set(window, &"__willowEventBuffer".into(), &arr);
            arr.into()
        }
    };

    let arr: js_sys::Array = buffer.unchecked_into();

    if arr.length() as usize >= BUFFER_CAPACITY {
        // Overflow: drop oldest, signal the test fixture.
        arr.shift();
        signal_overflow(window, 1);
    }

    arr.push(&js);
}

fn signal_overflow(window: &web_sys::Window, dropped: u32) {
    if let Ok(cb) = js_sys::Reflect::get(window, &"__willowOverflow".into()) {
        if let Some(func) = cb.dyn_ref::<js_sys::Function>() {
            let _ = func.call1(&JsValue::NULL, &JsValue::from_f64(dropped as f64));
        }
    }
    web_sys::console::error_1(
        &format!("test-hooks: __willow buffer overflow ({dropped} dropped)").into(),
    );
}
```

- [ ] **Step 2: Re-export from `crates/web/src/test_hooks/mod.rs`**

```rust
mod dispatcher;
pub use dispatcher::{install_push_dispatcher, DispatcherHandle};
```

- [ ] **Step 3: Run the test from Task 4.1, expect PASS**

```bash
wasm-pack test crates/web --headless --chrome --features test-hooks --test test_hooks_browser
```

Expected: PASS.

### Task 4.3: Failing test that dispatcher abort halts the loop

**Files:**
- Modify: `crates/web/tests/test_hooks_browser.rs`

- [ ] **Step 1: Add the test**

```rust
#[wasm_bindgen_test]
async fn dropping_dispatcher_handle_stops_emissions() {
    let captured: Rc<RefCell<Vec<JsValue>>> = Rc::new(RefCell::new(Vec::new()));
    let captured_clone = captured.clone();

    let cb = Closure::wrap(Box::new(move |ev: JsValue| {
        captured_clone.borrow_mut().push(ev);
    }) as Box<dyn FnMut(JsValue)>);

    let window = web_sys::window().unwrap();
    js_sys::Reflect::set(&window, &"__willowEvent".into(), cb.as_ref()).unwrap();
    cb.forget();

    let handle = fresh_client().await;
    {
        let _dispatcher =
            willow_web::test_hooks::install_push_dispatcher(handle.clone());
        handle.create_channel("a").await.unwrap();
        gloo_timers::future::TimeoutFuture::new(50).await;
    } // <- DispatcherHandle dropped here.

    let count_after_drop = captured.borrow().len();
    handle.create_channel("b").await.unwrap();
    gloo_timers::future::TimeoutFuture::new(50).await;
    let count_after_post_drop_event = captured.borrow().len();

    assert!(
        count_after_post_drop_event <= count_after_drop + 1,
        "dispatcher should not deliver events after handle drop \
         (got {count_after_post_drop_event} after drop, was {count_after_drop} at drop)"
    );

    js_sys::Reflect::delete_property(&window, &"__willowEvent".into()).unwrap();
}
```

(The `<= count_after_drop + 1` allows for one in-flight event delivered between the create_channel("b") triggering a SyncCompleted and the Drop's `*abort.borrow_mut() = true` taking effect on the next loop iteration. If you see > +1, the abort is not being checked.)

- [ ] **Step 2: Run, expect PASS** (Drop already implemented in Task 4.2).

### Task 4.4: Failing test for buffer drain on dispatch

Verifies the spec's "drain on every dispatch" edge: events buffered before the binding was registered get flushed when the next event triggers a dispatch.

**Files:**
- Modify: `crates/web/tests/test_hooks_browser.rs`

- [ ] **Step 1: Add the test**

```rust
#[wasm_bindgen_test]
async fn buffer_drains_on_first_dispatch_after_binding_appears() {
    let window = web_sys::window().unwrap();

    // Pre-seed the buffer as if a dispatcher had run before the
    // binding existed.
    let pre_buffer = js_sys::Array::new();
    pre_buffer.push(&JsValue::from_str("PREEXISTING"));
    js_sys::Reflect::set(&window, &"__willowEventBuffer".into(), &pre_buffer).unwrap();

    let captured: Rc<RefCell<Vec<JsValue>>> = Rc::new(RefCell::new(Vec::new()));
    let captured_clone = captured.clone();

    let cb = Closure::wrap(Box::new(move |ev: JsValue| {
        captured_clone.borrow_mut().push(ev);
    }) as Box<dyn FnMut(JsValue)>);
    js_sys::Reflect::set(&window, &"__willowEvent".into(), cb.as_ref()).unwrap();
    cb.forget();

    let handle = fresh_client().await;
    let _dispatcher =
        willow_web::test_hooks::install_push_dispatcher(handle.clone());

    handle.create_channel("trigger-drain").await.unwrap();
    gloo_timers::future::TimeoutFuture::new(50).await;

    let events = captured.borrow();
    let strs: Vec<String> = events
        .iter()
        .map(|ev| ev.as_string().unwrap_or_default())
        .collect();
    assert!(
        strs.contains(&"PREEXISTING".to_string()),
        "buffered pre-existing event should be drained; got {strs:?}"
    );

    js_sys::Reflect::delete_property(&window, &"__willowEvent".into()).unwrap();
    js_sys::Reflect::delete_property(&window, &"__willowEventBuffer".into()).unwrap();
}
```

- [ ] **Step 2: Run, expect PASS** (drain logic already in Task 4.2).

### Task 4.5: Failing test for buffer overflow signalling

**Files:**
- Modify: `crates/web/tests/test_hooks_browser.rs`

- [ ] **Step 1: Add the test**

```rust
#[wasm_bindgen_test]
async fn buffer_overflow_calls_willow_overflow_callback() {
    let window = web_sys::window().unwrap();

    // Set up the overflow hook FIRST.
    let overflow_count: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
    let overflow_clone = overflow_count.clone();
    let overflow_cb = Closure::wrap(Box::new(move |dropped: f64| {
        *overflow_clone.borrow_mut() += dropped as u32;
    }) as Box<dyn FnMut(f64)>);
    js_sys::Reflect::set(&window, &"__willowOverflow".into(), overflow_cb.as_ref())
        .unwrap();
    overflow_cb.forget();

    // Pre-fill the buffer past capacity. Use 65_537 entries: capacity
    // + 1 forces exactly one overflow drop.
    let pre_buffer = js_sys::Array::new();
    for i in 0..65_537u32 {
        pre_buffer.push(&JsValue::from_f64(i as f64));
    }
    js_sys::Reflect::set(&window, &"__willowEventBuffer".into(), &pre_buffer).unwrap();

    // Do NOT bind __willowEvent — we want push_into_buffer to be the
    // path under test.
    js_sys::Reflect::delete_property(&window, &"__willowEvent".into()).unwrap();

    let handle = fresh_client().await;
    let _dispatcher =
        willow_web::test_hooks::install_push_dispatcher(handle.clone());

    // Triggering a new event causes the dispatcher to push into a
    // full buffer.
    handle.create_channel("overflow-trigger").await.unwrap();
    gloo_timers::future::TimeoutFuture::new(50).await;

    assert!(
        *overflow_count.borrow() >= 1,
        "expected at least one overflow signal, got {}",
        overflow_count.borrow()
    );

    js_sys::Reflect::delete_property(&window, &"__willowOverflow".into()).unwrap();
    js_sys::Reflect::delete_property(&window, &"__willowEventBuffer".into()).unwrap();
}
```

- [ ] **Step 2: Run, expect PASS** (overflow signalling already in Task 4.2).

- [ ] **Step 3: Commit**

```bash
git add crates/web/src/test_hooks/ crates/web/tests/test_hooks_browser.rs
git commit -m "feat(web): add push dispatcher with three-edge buffer drain"
```

---

## Phase 5: Mount `WillowTestHooks` in `app.rs`

Per spec section "Mounted from `app.rs`": mount must happen **after** `with_trust_store` (so the same handle the UI uses is captured) and the dispatcher handle must be bound (not discarded) so the loop survives the function scope.

### Task 5.1: Add the cfg-gated mount block

**Files:**
- Modify: `crates/web/src/app.rs`

- [ ] **Step 1: Insert the mount block after `with_trust_store` and before `provide_context`**

Find the line in `crates/web/src/app.rs` (around line 161-165) that reads:

```rust
let handle_inner = (*handle).clone().with_trust_store(trust_store.clone());
let handle: WebClientHandle = SendWrapper::new(handle_inner);

// Provide context so child components can access the handle and state.
provide_context(handle.clone());
```

Insert the mount block immediately between the `SendWrapper::new(handle_inner)` line and the `provide_context` call:

```rust
let handle_inner = (*handle).clone().with_trust_store(trust_store.clone());
let handle: WebClientHandle = SendWrapper::new(handle_inner);

#[cfg(feature = "test-hooks")]
{
    use wasm_bindgen::JsCast;
    let inner_for_hooks = (*handle).clone();
    let hooks = crate::test_hooks::WillowTestHooks::new(inner_for_hooks.clone());
    if let Some(window) = web_sys::window() {
        let _ = js_sys::Reflect::set(
            &window,
            &"__willow".into(),
            &wasm_bindgen::JsValue::from(hooks),
        );
    }
    let dispatcher = crate::test_hooks::install_push_dispatcher(inner_for_hooks);
    // Bind the handle so it lives for the App component's scope. The
    // StoredValue does not drop until the owning Leptos scope is
    // disposed; binding (rather than discarding) keeps the dispatcher
    // loop alive for the app's lifetime.
    let _dispatcher_handle = leptos::StoredValue::new(send_wrapper::SendWrapper::new(dispatcher));
}

// Provide context so child components can access the handle and state.
provide_context(handle.clone());
```

- [ ] **Step 2: Verify both build configurations compile**

```bash
cargo check -p willow-web
cargo check -p willow-web --features test-hooks
```

Expected: both pass with zero warnings.

### Task 5.2: Browser test that `window.__willow` exists under feature

**Files:**
- Modify: `crates/web/tests/browser.rs`

- [ ] **Step 1: Add the test**

```rust
#[wasm_bindgen_test]
#[cfg(feature = "test-hooks")]
fn window_willow_is_mounted_under_test_hooks_feature() {
    use willow_web::App;

    let _container = mount_test(|| leptos::view! { <App/> });

    let window = web_sys::window().unwrap();
    let willow = js_sys::Reflect::get(&window, &"__willow".into()).unwrap();
    assert!(
        !willow.is_undefined(),
        "window.__willow must be present when test-hooks feature is on"
    );
}
```

- [ ] **Step 2: Run, expect PASS**

```bash
wasm-pack test crates/web --headless --chrome --features test-hooks
```

Expected: PASS (along with all earlier tests).

### Task 5.3: Verify default build does not export `__willow`

This is a compile-time absence check rather than a run-time test (the default-feature browser test cannot reference `WillowTestHooks` because the symbol does not exist). The CI symbol-leak script in Phase 6 covers the production-build case more authoritatively.

- [ ] **Step 1: Run the standard browser tests without `--features test-hooks`**

```bash
wasm-pack test crates/web --headless --chrome
```

Expected: existing tests pass; the test-hooks-gated test from Task 5.2 is excluded by `cfg`.

- [ ] **Step 2: Commit**

```bash
git add crates/web/src/app.rs crates/web/tests/browser.rs
git commit -m "feat(web): mount WillowTestHooks under test-hooks feature"
```

---

## Phase 6: Symbol-leak guard + justfile `FEATURES` forwarding

### Task 6.1: Add `FEATURES` variable to relevant justfile recipes

Per spec section "`test-hooks` cargo feature": `dev`, `setup-e2e`, `test-e2e-*`, and `check-all` recipes accept a `FEATURES` variable forwarded to `trunk build`. E2e recipes hardcode `FEATURES=test-hooks` internally; everything else defaults to empty.

**Files:**
- Modify: `justfile`

- [ ] **Step 1: Add the `FEATURES` parameter to existing recipes**

Identify the recipes (search the justfile for `dev:`, `setup-e2e:`, `test-e2e-ui:`, `test-e2e-sync:`, `test-e2e-perms:`, `test-e2e-full:`, `check-all:`). For each, add a `FEATURES=""` parameter and forward it to `trunk build` calls.

Example for `dev`:

```just
# Start full local dev stack (relay + workers + web)
dev FEATURES="":
    # ...existing recipe body...
    # Wherever it calls `trunk build` or `trunk serve`, change to:
    trunk serve {{ if FEATURES != "" { "--features " + FEATURES } else { "" } }}
```

Example for an e2e recipe that hardcodes the feature:

```just
test-e2e-ui:
    @just setup-e2e FEATURES=test-hooks
    npx playwright test e2e/
```

- [ ] **Step 2: Verify the recipes still parse**

```bash
just --list
```

Expected: all recipes listed; no parse errors.

- [ ] **Step 3: Verify a feature build runs**

```bash
just dev FEATURES=test-hooks --help 2>&1 | head -5
```

(If `dev` runs a long-lived server, smoke-test by ctrl-C'ing after seeing the build succeed.)

- [ ] **Step 4: Commit**

```bash
git add justfile
git commit -m "build: add FEATURES variable to dev / e2e / check-all recipes"
```

### Task 6.2: Failing test (a script that should fail without the symbol-leak guard, then pass with it)

**Files:**
- Create: `scripts/check-no-test-hooks-in-prod.sh`

- [ ] **Step 1: Create the script**

```bash
#!/usr/bin/env bash
# Asserts that a default `trunk build --release` does NOT include the
# WillowTestHooks symbol. Run as part of `just check-all` to catch
# accidental `default = ["test-hooks"]` regressions.
#
# Per docs/specs/2026-04-27-event-based-waits-design.md: the test-hooks
# feature must remain off in production for privacy reasons (third-party
# JS in prod could otherwise read DAG heads).

set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> Building release (no features) ..."
(cd crates/web && trunk build --release --offline 2>&1) > /dev/null

DIST=crates/web/dist
if grep -q "WillowTestHooks" "$DIST"/*.js 2>/dev/null; then
    echo "FAIL: WillowTestHooks symbol leaked into prod JS shim:"
    grep -l "WillowTestHooks" "$DIST"/*.js
    exit 1
fi

# Defence-in-depth: check the wasm name section if wasm-objdump is
# available. If not present, skip (CI image may install on demand).
if command -v wasm-objdump >/dev/null 2>&1; then
    if wasm-objdump --section=name "$DIST"/*.wasm 2>/dev/null | grep -q "WillowTestHooks"; then
        echo "FAIL: WillowTestHooks symbol leaked into prod wasm name section"
        exit 1
    fi
fi

echo "==> Building with --features test-hooks (sanity check) ..."
(cd crates/web && trunk build --release --features test-hooks --offline 2>&1) > /dev/null

if ! grep -q "WillowTestHooks" "$DIST"/*.js 2>/dev/null; then
    echo "FAIL: WillowTestHooks symbol absent from feature build — gating broken?"
    exit 1
fi

echo "PASS: test-hooks gating verified."
```

- [ ] **Step 2: Make it executable**

```bash
chmod +x scripts/check-no-test-hooks-in-prod.sh
```

- [ ] **Step 3: Run it**

```bash
./scripts/check-no-test-hooks-in-prod.sh
```

Expected: PASS (`==> Building release ...` then `==> Building with --features ...` then `PASS: test-hooks gating verified.`).

If FAIL on the first build — `WillowTestHooks` is leaking. Investigate `crates/web/Cargo.toml` `default = []` line and the `#[cfg(feature = "test-hooks")]` gates; one of them is wrong.

### Task 6.3: Wire the script into `just check-all`

**Files:**
- Modify: `justfile`

- [ ] **Step 1: Add the script invocation to `check-all`**

Find the `check-all` recipe and append:

```just
check-all: fmt-check clippy test check-wasm
    # ...existing body...
    ./scripts/check-no-test-hooks-in-prod.sh
```

- [ ] **Step 2: Run `just check-all`**

```bash
just check-all
```

Expected: passes (slow — full builds — but green).

- [ ] **Step 3: Commit**

```bash
git add scripts/check-no-test-hooks-in-prod.sh justfile
git commit -m "ci: add test-hooks symbol-leak guard to check-all"
```

---

## Phase 7: ESLint rule + per-spec `eslint-disable` headers

Per spec section "Lint window note": the ESLint rule lands in PR 1 (this PR), and every existing spec gets a per-file disable header referencing the tracking issue. PR 4 lands the count-based ratchet; PR 1 only enforces "no new offences" via the rule + headers.

### Task 7.1: Verify the e2e directory has an ESLint config

- [ ] **Step 1: Inspect for existing config**

```bash
ls -la /home/user/willow/e2e/.eslintrc* /home/user/willow/.eslintrc* /home/user/willow/eslint.config.* 2>/dev/null
```

If a config already exists at the repo root or in `e2e/`, the rule will be added there (Step 2 of Task 7.2). If none exists, the new file is created in this task (continue to Step 2 of Task 7.1).

- [ ] **Step 2: If no config exists, install ESLint and create a base config**

```bash
cd /home/user/willow
npm install --save-dev --no-save eslint @typescript-eslint/parser @typescript-eslint/eslint-plugin
```

(Use `--no-save` only if no `package.json` exists. If one exists, drop `--no-save`.)

### Task 7.2: Add the `no-restricted-syntax` rule

**Files:**
- Create or modify: `e2e/.eslintrc.cjs` (the most local config takes precedence)

- [ ] **Step 1: Write `e2e/.eslintrc.cjs`**

```js
// ESLint configuration for Playwright e2e tests.
//
// Bans `page.waitForTimeout(...)` (and any `*.waitForTimeout(...)` call)
// in favour of event-based waits. See:
// docs/specs/2026-04-27-event-based-waits-design.md
//
// Existing un-migrated specs carry per-file `eslint-disable` headers
// referencing the tracking issue. Those headers are removed file by
// file as each spec migrates to event-based waits.

module.exports = {
  parser: '@typescript-eslint/parser',
  plugins: ['@typescript-eslint'],
  rules: {
    'no-restricted-syntax': [
      'error',
      {
        selector: "CallExpression[callee.property.name='waitForTimeout']",
        message:
          'Use event-based waits (Peer.nextEvent / waitUntilHeadsEqual / data-state). See docs/specs/2026-04-27-event-based-waits-design.md.',
      },
    ],
  },
};
```

- [ ] **Step 2: Verify the rule fires on existing code**

```bash
npx eslint e2e/multi-peer-sync.spec.ts
```

Expected: errors on every `waitForTimeout` line in that file.

### Task 7.3: Add the per-spec disable headers

The headers point at the tracking-issue URL captured in Task 0.2. Each header is a single line at the top of the file (after the `import` block if any, but per-line ESLint disable headers must be at file top to suppress on all subsequent lines).

**Files:**
- Modify: `e2e/cross-browser-sync.spec.ts`
- Modify: `e2e/join-links.spec.ts`
- Modify: `e2e/mobile-actions.spec.ts`
- Modify: `e2e/mobile.spec.ts`
- Modify: `e2e/multi-peer-mobile.spec.ts`
- Modify: `e2e/multi-peer-sync.spec.ts`
- Modify: `e2e/permissions.spec.ts`
- Modify: `e2e/worker-nodes.spec.ts`
- Modify: `e2e/helpers.ts`

- [ ] **Step 1: Add the disable header to each file**

Substitute `<ISSUE_URL>` with the URL from Task 0.2.

For each of the 9 files, add as the very first line:

```ts
/* eslint-disable no-restricted-syntax -- migration tracked at <ISSUE_URL> */
```

- [ ] **Step 2: Run ESLint over all e2e files**

```bash
npx eslint 'e2e/**/*.ts'
```

Expected: zero errors. The disable headers suppress all current `waitForTimeout` offences; new calls in any other location would fail.

- [ ] **Step 3: Verify the rule still bites new offences**

Add a temporary `await page.waitForTimeout(100);` to a fresh file (e.g. a new throwaway `e2e/_lint-probe.spec.ts`) and re-run lint — must FAIL. Then delete the probe file. This step proves the disable headers do not silently disable the rule globally.

```bash
# Create the probe.
cat > e2e/_lint-probe.spec.ts <<'EOF'
import { test } from '@playwright/test';
test('probe', async ({ page }) => {
  await page.waitForTimeout(100);
});
EOF

npx eslint e2e/_lint-probe.spec.ts && {
  echo "FAIL: probe should have errored"; exit 1;
} || echo "PASS: rule fires on new offences"

rm e2e/_lint-probe.spec.ts
```

- [ ] **Step 4: Commit**

```bash
git add e2e/.eslintrc.cjs e2e/*.spec.ts e2e/helpers.ts
git commit -m "ci(e2e): forbid new waitForTimeout calls; allowlist existing"
```

---

## Phase 8: Final verification + push

### Task 8.1: Run `just check`

- [ ] **Step 1: Run**

```bash
just check
```

Expected: zero warnings across fmt, clippy, test, and WASM check.

If clippy fires on the new code, fix inline before committing. The bar is "zero warnings", per CLAUDE.md.

### Task 8.2: Run `just test-browser` for both feature configurations

- [ ] **Step 1: Default features**

```bash
just test-browser
```

Expected: existing browser tests pass.

- [ ] **Step 2: With `test-hooks`**

```bash
wasm-pack test crates/web --headless --chrome --features test-hooks
```

Expected: existing tests + 6 new test-hooks tests pass (snapshot, heads, event_count/last_event, dispatcher emission, dispatcher abort, buffer drain on dispatch, buffer overflow signalling).

### Task 8.3: Run `just check-all`

- [ ] **Step 1: Run**

```bash
just check-all
```

Expected: includes the symbol-leak guard from Phase 6; passes end-to-end.

### Task 8.4: Push the branch

- [ ] **Step 1: Verify branch**

```bash
git branch --show-current
```

Expected: `claude/event-based-waits-RNFZ9`.

- [ ] **Step 2: Push**

```bash
git push -u origin claude/event-based-waits-RNFZ9
```

Expected: push succeeds.

- [ ] **Step 3: Open the PR**

PR title: `feat: test-hooks foundation for event-based Playwright waits`

PR body should include:
- Link to spec: `docs/specs/2026-04-27-event-based-waits-design.md`
- Link to tracking issue (from Task 0.2)
- iroh `performance.now` audit result (from Task 0.1)
- Note that this is PR 1 of 4: `Peer` wrapper + helpers split + first pilot land in PR 2.

---

## Plan self-review

This section is run **once**, by the implementer (or you if you stayed inline). Compare against `docs/specs/2026-04-27-event-based-waits-design.md`.

- [ ] **Spec coverage check.** Walk the "In scope" bullets in the spec. Each must map to at least one task above. Specifically:
  - Cargo feature `test-hooks` off in production → Task 1.1 + Task 6.2 (symbol-leak guard).
  - WASM-exported `WillowTestHooks` API (snapshot, heads, event count, last event) → Phase 2 tasks.
  - Push instrumentation via `exposeBinding` → Phase 4 (the WASM dispatcher; the Playwright-side fixture lands in PR 2).
  - TypeScript wrapper `Peer` → **NOT in PR 1** (PR 2 scope, called out in Phase 8 PR body).
  - `data-state` attribute pattern → **NOT in PR 1** (PR 3 scope).
  - `page.clock` adoption → **NOT in PR 1** (PR 2 scope).
  - Helpers split → **NOT in PR 1** (PR 2 scope).
  - Pilot conversions → **NOT in PR 1** (PR 2 scope).
  - ESLint rule blocking new `page.waitForTimeout` calls + per-file allowlist → Phase 7.
  - CI symbol-leak check + flake harness → Symbol-leak in Phase 6; flake harness in PR 4.
  - GitHub tracking issue → Task 0.2.

- [ ] **Placeholder scan.** Search the plan above for "TBD", "TODO", "fill in", "similar to", "appropriate". No matches expected. Code blocks must be complete (no `// ...` ellipses representing un-shown code).

- [ ] **Type / signature consistency.** `WillowTestHooks::new` signature is referenced from Tasks 2.2, 2.3, 4.1, 5.1 — all use `<N: Network + 'static>`. `install_push_dispatcher` returns `DispatcherHandle` (Tasks 4.2, 5.1). `to_wire` returns `Option<WireEvent>` (Tasks 3.1, 3.2, 3.3). `WireEvent` is `#[serde(tag = "kind")]` (Task 3.1, 3.3).

- [ ] **Acceptance verification.** Phase 8's `just check`, `just test-browser`, `wasm-pack test --features test-hooks`, `just check-all` cover compile + test for both feature configurations + symbol-leak gating.

If any of the above fails, fix inline and re-run the corresponding tasks.

---

## Out-of-scope (subsequent PRs)

- **PR 2 — Playwright `Peer` wrapper + helpers split + first pilot.** Plan: `docs/plans/2026-04-27-event-based-waits-pr2-playwright-wrapper.md` (to be written when PR 1 lands).
- **PR 3 — `data-state` lifecycle on five animated components.** Plan: `docs/plans/2026-04-27-event-based-waits-pr3-data-state-lifecycle.md`.
- **PR 4 — Ratchet script + flake harness + cleanup.** Plan: `docs/plans/2026-04-27-event-based-waits-pr4-ratchet-and-cleanup.md`.

The 7 remaining spec migrations are tracked in the GitHub issue from Task 0.2. Each gets its own small PR; ESLint disable headers are removed in those PRs.
