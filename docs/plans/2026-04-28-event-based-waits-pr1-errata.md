# PR-1 Plan Errata — 2026-04-28

> **Supersedes specific sections of `2026-04-27-event-based-waits-pr1-test-hooks-foundation.md`.** Read this file alongside the original plan. Where they conflict, this file wins.

## Why this exists

During PR-1 execution an investigation pass surfaced concrete API mismatches between the original plan and the real Willow codebase. The plan was written against speculative API signatures that don't exist. This errata records the corrections so subsequent implementer agents have one accurate source.

## Investigation findings (verified against the codebase)

1. **No synchronous DAG read path on `ClientHandle`.** Every read goes through async actor-ask via `willow_actor::state::select(&addr, |state| ...).await`. There is no `Arc<ClientShared>` or equivalent. (`crates/client/src/lib.rs:216-285`, `crates/client/src/accessors.rs` — all 23 accessors are async.)
2. **`MemNetwork` is native-only.** `crates/network/src/mem.rs:35` does `use tokio::sync::broadcast;` unconditionally, and `crates/network/Cargo.toml:28-30` gates `tokio` to `cfg(not(target_arch = "wasm32"))`. Confirmed: `cargo check --target wasm32-unknown-unknown -p willow-network --features test-utils` fails.
3. **No `EventHash::to_hex()` / `EndpointId::to_hex()`.** Both types implement `Display` producing 64-char lowercase hex. Use `.to_string()` or `format!("{}", x)`.
4. **No `ChannelView` / `channels_view()`.** Real types: `ChannelsView { channels: Vec<ChannelInfo> }` (`crates/client/src/views.rs:117-119`) and `ChannelInfo { name: String, kind: ChannelKind }` (`:122-126`). No `member_count` field exists; the plan's `ChannelDto.member_count` must be replaced with `kind` (or dropped).
5. **wasm-streams duplicate-version is not a compile blocker.** `Cargo.lock` shows `wasm-streams 0.4.2` (via `leptos→server_fn`) and `0.5.0` (via `iroh→reqwest`) coexisting. `cargo check --tests --target wasm32-unknown-unknown -p willow-web` succeeds. Whether `wasm-pack test` link-step trips a duplicate-symbol error remains unverified in this CI/sandbox env (no `wasm-pack` installed). Not blocking PR-1; out-of-scope to redesign around.
6. **`subscribe_events` signature verified correct.** `crates/client/src/accessors.rs:10` returns `EventReceiver` (defined in `lib.rs:112-166`). `try_recv()` exists at `:144` for non-blocking polling.
7. **`test-utils` feature transitively enables `MemNetwork`.** Reusing it for our purposes breaks WASM. We need a NEW `test-hooks` feature, distinct from `test-utils`.

## Section-by-section corrections

### Phase 1 — Cargo feature scaffold

**Task 1.1 corrections.** Add the feature on **two** crates (`willow-client` AND `willow-web`):

`crates/client/Cargo.toml` — append:
```toml
[features]
test-hooks = []
```
(Distinct from existing `test-utils`. `test-hooks` is narrow read-only instrumentation; `test-utils` pulls `MemNetwork`.)

`crates/web/Cargo.toml` — append:
```toml
[features]
default = []
test-hooks = ["dep:serde-wasm-bindgen", "willow-client/test-hooks"]
```

And in `[dependencies]` (place alphabetically near `serde_json`):
```toml
serde-wasm-bindgen = { version = "0.6", optional = true }
```

Verification commands stay the same:
```bash
cargo check -p willow-client
cargo check -p willow-client --features test-hooks
cargo check -p willow-web
cargo check -p willow-web --features test-hooks
```

All four must succeed, zero warnings. Commit message: `feat: add test-hooks feature on client + web` (singular commit covering both crates).

**Task 1.2 corrections.** `lib.rs` declaration must remain alphabetically sorted (rustfmt enforces). The cfg-gated `pub mod test_hooks;` belongs **before** `pub mod trust_store;` (since `test_hooks < trust_store` lexicographically). Otherwise unchanged.

### Phase 2 — Pull API

**Task 2.1 corrections.** `ChannelDto` field rename: drop `member_count` (no source field exists), use `kind` instead.

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDto {
    pub name: String,
    pub kind: String, // ChannelKind serialised as string
}
```

`AuthorHeadDto` and `SnapshotDto` are unchanged from the plan.

**Task 2.2 corrections — failing browser test.** The plan's `fresh_client()` helper that builds a `ClientHandle<MemNetwork>` does not work (MemNetwork won't compile on wasm32). Replace with a fixture that constructs the actor system directly:

```rust
//! In-browser tests for `WillowTestHooks`.
//!
//! Run with:
//!   wasm-pack test crates/web --headless --chrome --features test-hooks
//!
//! Bypasses `ClientHandle` entirely — `MemNetwork` won't compile on
//! wasm32 (it depends on `tokio::sync::broadcast`). Constructs
//! `Addr<StateActor<DagState>>` and `Addr<StateActor<ServerState>>`
//! directly, then feeds them to `WillowTestHooks::from_actors`.

#![cfg(feature = "test-hooks")]

use wasm_bindgen_test::*;
use willow_actor::{StateActor, System};
use willow_client::state_actors::DagState;
use willow_state::ServerState;
use willow_web::test_hooks::WillowTestHooks;

wasm_bindgen_test_configure!(run_in_browser);

/// Construct a WillowTestHooks instance backed by empty actor state.
async fn empty_hooks() -> WillowTestHooks {
    let sys = System::new();
    let dag_addr = sys.spawn(StateActor::new(DagState::default()));
    let state_addr = sys.spawn(StateActor::new(ServerState::default()));
    WillowTestHooks::from_actors(dag_addr, state_addr)
}

#[wasm_bindgen_test]
async fn empty_hooks_event_count_is_zero() {
    let hooks = empty_hooks().await;
    let p = hooks.event_count();
    let count = wasm_bindgen_futures::JsFuture::from(p).await.unwrap();
    assert_eq!(count.as_f64(), Some(0.0));
}

#[wasm_bindgen_test]
async fn empty_hooks_last_event_is_null() {
    let hooks = empty_hooks().await;
    let p = hooks.last_event();
    let last = wasm_bindgen_futures::JsFuture::from(p).await.unwrap();
    assert!(last.is_null(), "last_event on empty DAG must be null, got {last:?}");
}
```

The implementer can drive a non-empty DAG fixture later (e.g., by `state::mutate(&dag_addr, |ds| ds.managed.append_local(...).unwrap())` once a signing identity is wired). For PR-1 the empty-DAG assertions are sufficient — they verify the plumbing.

**Task 2.3 corrections — `WillowTestHooks` impl.** Replace the plan's closure-erasure pattern with direct actor-address storage. `event_count`, `last_event`, `heads`, `snapshot` all return `js_sys::Promise` and use `wasm_bindgen_futures::future_to_promise` + `willow_actor::state::select`:

```rust
#![cfg(feature = "test-hooks")]

mod snapshot;
pub use snapshot::{AuthorHeadDto, ChannelDto, SnapshotDto};

use std::collections::BTreeMap;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use willow_actor::{Addr, StateActor};
use willow_client::state_actors::DagState;
use willow_client::ClientHandle;
use willow_network::Network;
use willow_state::ServerState;

#[wasm_bindgen]
pub struct WillowTestHooks {
    dag_addr: Addr<StateActor<DagState>>,
    state_addr: Addr<StateActor<ServerState>>,
}

impl WillowTestHooks {
    /// Construct from a ClientHandle (production path: `app.rs` mount).
    pub fn new<N: Network + 'static>(handle: &ClientHandle<N>) -> Self {
        Self {
            dag_addr: handle.dag_addr_clone(),
            state_addr: handle.event_state_addr_clone(),
        }
    }

    /// Construct from raw actor addresses (test path: bypasses ClientHandle
    /// so wasm32 tests don't need MemNetwork).
    pub fn from_actors(
        dag_addr: Addr<StateActor<DagState>>,
        state_addr: Addr<StateActor<ServerState>>,
    ) -> Self {
        Self { dag_addr, state_addr }
    }
}

#[wasm_bindgen]
impl WillowTestHooks {
    pub fn event_count(&self) -> js_sys::Promise {
        let addr = self.dag_addr.clone();
        future_to_promise(async move {
            let n = willow_actor::state::select(&addr, |ds| ds.managed.dag().len() as u32).await;
            Ok(JsValue::from_f64(n as f64))
        })
    }

    pub fn last_event(&self) -> js_sys::Promise {
        let addr = self.dag_addr.clone();
        future_to_promise(async move {
            let hex = willow_actor::state::select(&addr, |ds| {
                ds.managed
                    .dag()
                    .topological_sort()
                    .last()
                    .map(|e| e.hash.to_string())
            })
            .await;
            Ok(match hex {
                Some(s) => JsValue::from_str(&s),
                None => JsValue::NULL,
            })
        })
    }

    pub fn heads(&self) -> js_sys::Promise {
        let addr = self.dag_addr.clone();
        future_to_promise(async move {
            let map: BTreeMap<String, AuthorHeadDto> =
                willow_actor::state::select(&addr, snapshot::build_heads).await;
            serde_wasm_bindgen::to_value(&map).map_err(Into::into)
        })
    }

    pub fn snapshot(&self) -> js_sys::Promise {
        let dag_addr = self.dag_addr.clone();
        let state_addr = self.state_addr.clone();
        future_to_promise(async move {
            let snap = snapshot::build(&dag_addr, &state_addr).await;
            serde_wasm_bindgen::to_value(&snap).map_err(Into::into)
        })
    }
}
```

The plan's closure-pattern (`Box<dyn Fn>`) is replaced because storing actor addresses is simpler, and the reads must be async anyway.

**Task 2.3 corrections — `crates/client/src/accessors.rs` additions.** The plan adds `dag_event_count` etc. as bespoke accessors. **DON'T.** Instead expose a single sync getter for the DAG actor address, gated on `test-hooks`:

```rust
// At the end of crates/client/src/accessors.rs:

#[cfg(feature = "test-hooks")]
impl<N: Network> ClientHandle<N> {
    /// Clone the DAG actor address. Test-only (read-only) access surface
    /// for `WillowTestHooks`; the address itself doesn't grant write access
    /// without an active mutator.
    pub fn dag_addr_clone(&self) -> willow_actor::Addr<willow_actor::StateActor<crate::state_actors::DagState>> {
        self.dag_addr.clone()
    }

    /// Clone the materialised ServerState actor address. Test-only.
    pub fn event_state_addr_clone(&self) -> willow_actor::Addr<willow_actor::StateActor<willow_state::ServerState>> {
        self.event_state_addr.clone()
    }
}
```

Both methods are gated, so non-test consumers (`willow-agent`, `willow-replay`, etc.) never see them.

**Task 2.3 corrections — `snapshot.rs` build helpers.**

```rust
use std::collections::BTreeMap;
use willow_actor::{Addr, StateActor};
use willow_client::state_actors::DagState;
use willow_state::ServerState;

pub(crate) fn build_heads(ds: &DagState) -> BTreeMap<String, AuthorHeadDto> {
    ds.managed
        .heads_summary()
        .heads
        .iter()
        .map(|(endpoint, head)| {
            (
                endpoint.to_string(),
                AuthorHeadDto {
                    seq: head.seq,
                    hash: head.hash.to_string(),
                },
            )
        })
        .collect()
}

pub(crate) async fn build(
    dag_addr: &Addr<StateActor<DagState>>,
    state_addr: &Addr<StateActor<ServerState>>,
) -> SnapshotDto {
    // Two actor-asks (cheap, sub-ms each on local mailbox dispatch).
    let (event_count, heads, last_event) =
        willow_actor::state::select(dag_addr, |ds| {
            (
                ds.managed.dag().len() as u32,
                build_heads(ds),
                ds.managed
                    .dag()
                    .topological_sort()
                    .last()
                    .map(|e| e.hash.to_string()),
            )
        })
        .await;
    let channels = willow_actor::state::select(state_addr, |ss| {
        // ServerState's channels live on `ss.channels` (HashMap<String, ChannelInfo>)
        // — the implementer must verify the exact accessor; falling back to
        // computing via crates/client/src/views.rs::compute_channels_view if needed.
        ss.channels
            .iter()
            .map(|(name, info)| ChannelDto {
                name: name.clone(),
                kind: format!("{:?}", info.kind),
            })
            .collect::<Vec<_>>()
    })
    .await;
    SnapshotDto {
        event_count,
        heads,
        last_event,
        channels,
    }
}
```

The implementer should verify the exact `ServerState.channels` field access — if it's not a direct `HashMap`, reuse `compute_channels_view` from `crates/client/src/views.rs:656-674`. The shape `Vec<ChannelDto { name, kind }>` is what matters.

**Task 2.4-2.7 corrections.** The browser tests for `heads()` and `snapshot()` use the same `from_actors` pattern. Since methods return `js_sys::Promise`, tests must `JsFuture::from(p).await`. Drop any `serde_wasm_bindgen::from_value::<Snapshot>` that assumes a sync return.

### Phase 3 — Wire-shape conversion

No corrections needed. `ClientEvent` enum reference (`crates/client/src/events.rs:19`) and `EndpointId.to_hex()` are the same shape — except: replace `id.to_hex()` with `id.to_string()` in the `to_wire` body.

### Phase 4 — Push dispatcher

No structural corrections. `subscribe_events()` is async; the dispatcher already runs inside `wasm_bindgen_futures::spawn_local` so `let mut rx = handle.subscribe_events().await;` works. The plan's code is correct.

`install_push_dispatcher` signature stays generic over `N: Network` and returns `DispatcherHandle`.

### Phase 5 — Mount in `app.rs`

Mount block stays mostly the same. Two small changes:

1. `WillowTestHooks::new(&handle)` takes `&ClientHandle<N>` (borrow) not by value. Update accordingly:
   ```rust
   let hooks = crate::test_hooks::WillowTestHooks::new(&inner_for_hooks);
   ```
2. `install_push_dispatcher(handle.clone())` still takes ownership of a clone (the dispatcher needs a long-lived clone for the spawn_local task).

### Phases 6, 7, 8

No corrections.

## What stays unchanged

- Buffer drain on three edges (init / per-dispatch / read-side) — Phase 4 logic is correct.
- Symbol-leak guard in Phase 6 (`grep WillowTestHooks dist/*.js`).
- ESLint `no-restricted-syntax` rule and per-file disable headers.
- TDD red-green flow.
- Commit boundary structure (one commit per Task or TDD pair).
- The whole of Phase 8 verification.

## Browser-test environment caveat

`wasm-pack` and `just` are not available in the Claude Code sandbox. The implementer should still attempt `cargo check --tests --target wasm32-unknown-unknown -p willow-web --features test-hooks` to verify compile. Actual `wasm-pack test` runs must happen on the developer's machine or in CI; this PR's acceptance gate notes this explicitly so reviewers know to run tests locally before approving.

## Tracking

The tracking issue created in plan Task 0.2 is **#458** — `https://github.com/intendednull/willow/issues/458` — already exists. Don't recreate.
