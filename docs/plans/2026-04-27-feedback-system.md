# Feedback System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship an in-app "Send Feedback" form in the Willow web UI that submits to a new `willow-feedback` worker node, which proxies user-submitted feedback to GitHub issues on `intendednull/willow`.

**Architecture:** A new native-only worker crate (`willow-feedback`) joins the existing worker pattern alongside replay and storage, reusing the gossip-based `WorkerWireMessage::Request/Response` pathway. The `WorkerRole::handle_request` trait method becomes `async` and gains a `signer: EndpointId` parameter so the role can enforce per-peer rate limits and compute the salted reporter handle. The web UI gets a new modal under Settings, configured via a `__WILLOW_FEEDBACK_PEER_ID` window global injected at container start by a new `docker/web-entrypoint.sh`.

**Tech Stack:** Rust (workspace), Leptos (web UI), `reqwest`+`rustls-tls` (GitHub API), `secrecy` (PAT handling), `blake3` (reporter-handle hash), `async-trait` (trait change), `bincode` over iroh-gossip (transport), `trunk` (web build), Docker Compose (deployment).

**Spec:** [`docs/specs/2026-04-27-feedback-system-design.md`](../specs/2026-04-27-feedback-system-design.md).

---

## File structure

**New files (created in this plan):**

```
crates/feedback/                         — new worker crate
├── Cargo.toml
├── build.rs                             — emits WILLOW_BUILD_SHA env at compile time
├── src/
│   ├── main.rs                          — CLI parsing, IrohNetwork bring-up, runtime::run
│   ├── role.rs                          — FeedbackRole : WorkerRole (sanitization, rate limits, idempotency)
│   ├── github.rs                        — reqwest-based GitHub-issues client + GithubClient trait
│   ├── handle.rs                        — salted-hash reporter handle (blake3 + BIP-39)
│   ├── wordlist.rs                      — vendored BIP-39 English wordlist (2048 entries)
│   ├── ratelimit.rs                     — token-bucket per-peer + global
│   ├── sanitize.rs                      — body-fence + title sanitization
│   ├── throttle.rs                      — startup-throttle file gating
│   └── salt.rs                          — load-or-generate 32-byte salt file
└── tests/
    └── fixtures/github/                 — recorded GitHub API JSON responses
        ├── 201-created.json
        ├── 401-unauthorized.json
        ├── 403-secondary-rate-limit.json
        ├── 404-not-found.json
        └── 422-validation.json

docker/
├── feedback.Dockerfile                  — sibling to docker/replay.Dockerfile
├── feedback-entrypoint.sh               — sibling to docker/replay-entrypoint.sh
└── web-entrypoint.sh                    — new: substitutes __INJECT_*__ placeholders in init.js

crates/web/dev_assets/                   — checked-in directory for trunk copy-dir
├── .gitignore                           — single line: feedback-peer-id.txt
└── .gitkeep                             — empty placeholder

crates/client/src/feedback.rs            — Client::submit_feedback + FeedbackError
crates/client/src/tests/feedback.rs      — client-tier integration tests via MemNetwork

crates/web/src/components/feedback.rs    — modal + failure-state copy
```

**Modified files:**

```
crates/common/src/worker_types.rs        — extend WorkerRequest/Response/RoleInfo, add wire types
crates/worker/src/actors/mod.rs          — WorkerRequestMsg gains signer
crates/worker/src/actors/state.rs        — handler awaits role; pass signer through
crates/worker/src/actors/network.rs      — pass requester (signer) into WorkerRequestMsg
crates/worker/src/actors/sync.rs         — internal Sync requests need a signer (use local peer)
crates/worker/src/actors/heartbeat.rs    — TestRole becomes async + takes _signer
crates/worker/Cargo.toml                  — add async-trait dep
crates/replay/src/role.rs                — async fn handle_request(_signer, ...)
crates/storage/src/role.rs               — async fn handle_request(_signer, ...)
crates/client/src/lib.rs                 — pub mod feedback; ClientConfig::feedback_worker
crates/client/Cargo.toml                  — add `url` dep
crates/web/src/components/settings.rs    — Help & Feedback section + button
crates/web/src/components/mod.rs         — pub mod feedback
crates/web/src/state.rs                  — read __WILLOW_FEEDBACK_PEER_ID, store in client config
crates/web/index.html                    — add `<link data-trunk rel="copy-dir" href="dev_assets/" />`
crates/web/init.js                       — placeholder substitution + dev fetch
docker/web.Dockerfile                    — COPY web-entrypoint.sh; ENTRYPOINT
docker-compose.yml                        — new `feedback` service + volume
scripts/dev.sh                           — start feedback worker, write peer ID to dev_assets
justfile                                 — test-feedback, build-feedback, docker-ids, test-workers
Cargo.toml                               — add `willow-feedback` to workspace members
```

---

## Phase 1: Wire types + async trait change

**Why first:** the trait change is foundational. Replay and storage compile and test against it. If we don't land it cleanly first, every other phase blocks. We do this with TDD: write the new wire-type round-trip tests, watch them fail, add the variants, watch them pass, then migrate the trait.

**Note on workspace registration:** the new `willow-feedback` crate is registered in the workspace `Cargo.toml` as part of Phase 2 (when the crate directory actually exists). Phase 1 only touches `willow-common`, `willow-worker`, `willow-replay`, and `willow-storage`.

### Task 1.1: Add `FeedbackCategory`, `ClientPlatform`, `FeedbackDiagnostics` types

**Files:**
- Modify: `crates/common/src/worker_types.rs`

These three types are leaf types (no dependencies on the other new variants), so we add them and their round-trip tests first.

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block at the bottom of `crates/common/src/worker_types.rs`:

```rust
#[test]
fn feedback_category_round_trips() {
    for cat in [
        FeedbackCategory::Bug,
        FeedbackCategory::Suggestion,
        FeedbackCategory::Other { detail: None },
        FeedbackCategory::Other {
            detail: Some("performance".to_string()),
        },
    ] {
        let bytes = bincode::serialize(&cat).unwrap();
        let decoded: FeedbackCategory = bincode::deserialize(&bytes).unwrap();
        assert_eq!(cat, decoded);
    }
}

#[test]
fn client_platform_round_trips() {
    for cp in [
        ClientPlatform::Web {
            ua_family: "firefox/138".to_string(),
        },
        ClientPlatform::Native {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
        },
    ] {
        let bytes = bincode::serialize(&cp).unwrap();
        let decoded: ClientPlatform = bincode::deserialize(&bytes).unwrap();
        assert_eq!(cp, decoded);
    }
}

#[test]
fn feedback_diagnostics_round_trips() {
    let diag = FeedbackDiagnostics {
        app_version: "0.1.0".to_string(),
        build_hash: Some("abc1234".to_string()),
        locale: Some("en-US".to_string()),
        client: ClientPlatform::Web {
            ua_family: "firefox/138".to_string(),
        },
    };
    let bytes = bincode::serialize(&diag).unwrap();
    let decoded: FeedbackDiagnostics = bincode::deserialize(&bytes).unwrap();
    assert_eq!(diag, decoded);
}
```

- [ ] **Step 2: Run tests, verify they fail with "not found" errors**

Run: `cargo test -p willow-common feedback_category_round_trips client_platform_round_trips feedback_diagnostics_round_trips`
Expected: COMPILE FAIL — `cannot find type FeedbackCategory in this scope` (and similar for the other two types).

- [ ] **Step 3: Add the types**

Append to `crates/common/src/worker_types.rs` *above* the `#[cfg(test)] mod tests` block:

```rust
/// Top-level category for a feedback report. Surfaced as a label and
/// title prefix on the GitHub issue.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum FeedbackCategory {
    Bug,
    Suggestion,
    /// Free-form category. `detail` is a short subcategory string the
    /// user types (e.g. "performance", "docs"); shown in the issue
    /// title prefix as `[Other:<detail>]`.
    Other {
        /// Optional, <= 60 chars. Validated by the worker.
        detail: Option<String>,
    },
}

/// The submitting client's platform — coarse-grained on purpose so
/// the issue body cannot include a fingerprintable full UA string.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum ClientPlatform {
    /// Browser submission. `ua_family` is `"<browser>/<major>"`,
    /// e.g. `"firefox/138"`. <= 40 chars.
    Web { ua_family: String },
    /// Native submission. `"linux"` / `"macos"` / `"windows"` and
    /// e.g. `"x86_64"` / `"aarch64"`.
    Native { os: String, arch: String },
}

/// Optional diagnostic info attached to a feedback report. Only
/// included when the user opts in via the UI checkbox; the disclosure
/// renders the *exact* value that will be sent.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct FeedbackDiagnostics {
    /// `CARGO_PKG_VERSION` of the submitting client.
    pub app_version: String,
    /// Short git SHA from `option_env!("WILLOW_BUILD_SHA")` injected
    /// by `build.rs`. None in dev builds.
    pub build_hash: Option<String>,
    /// IETF BCP 47 locale tag (e.g. `"en-US"`).
    pub locale: Option<String>,
    pub client: ClientPlatform,
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test -p willow-common feedback_category_round_trips client_platform_round_trips feedback_diagnostics_round_trips`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/common/src/worker_types.rs
git commit -m "feat(common): add FeedbackCategory, ClientPlatform, FeedbackDiagnostics wire types"
```

### Task 1.2: Add `FeedbackErrReason` and feedback variants on `WorkerRequest` / `WorkerResponse` / `WorkerRoleInfo`

**Files:**
- Modify: `crates/common/src/worker_types.rs`

This task extends three existing enums and adds `#[non_exhaustive]` to each as a forward-compat consumer-side guard. It also extends `WorkerRoleInfo::role_name()` so feedback identifies itself in heartbeats.

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn feedback_err_reason_variants_round_trip() {
    for r in [
        FeedbackErrReason::RateLimited { retry_after_ms: 12_345 },
        FeedbackErrReason::InvalidInput {
            field: "title".to_string(),
            message: "too long".to_string(),
        },
        FeedbackErrReason::GithubFailure {
            status: 422,
            message: Some("Validation Failed".to_string()),
        },
        FeedbackErrReason::GithubFailure { status: 0, message: None },
        FeedbackErrReason::Unconfigured,
    ] {
        let bytes = bincode::serialize(&r).unwrap();
        let decoded: FeedbackErrReason = bincode::deserialize(&bytes).unwrap();
        assert_eq!(r, decoded);
    }
}

#[test]
fn worker_request_feedback_round_trip() {
    let id = Identity::generate();
    let req = WorkerRequest::Feedback {
        dedup_id: [7u8; 16],
        title: "It crashes".to_string(),
        category: FeedbackCategory::Bug,
        body: "Steps:\n1. open the app\n2. it crashes".to_string(),
        diagnostics: Some(FeedbackDiagnostics {
            app_version: "0.1.0".to_string(),
            build_hash: Some("abc1234".to_string()),
            locale: Some("en-US".to_string()),
            client: ClientPlatform::Web {
                ua_family: "firefox/138".to_string(),
            },
        }),
    };
    let msg = WorkerWireMessage::Request {
        request_id: "rid-1".to_string(),
        target_peer: id.endpoint_id(),
        payload: req.clone(),
    };
    let decoded = worker_wire_round_trip(msg, &id);
    match decoded {
        WorkerWireMessage::Request { payload, .. } => assert_eq!(payload, req),
        _ => panic!("expected Request"),
    }
}

#[test]
fn worker_response_feedback_round_trip() {
    let id = Identity::generate();
    for resp in [
        WorkerResponse::FeedbackOk {
            issue_url: "https://github.com/x/y/issues/42".to_string(),
        },
        WorkerResponse::FeedbackErr {
            reason: FeedbackErrReason::RateLimited { retry_after_ms: 60_000 },
        },
    ] {
        let msg = WorkerWireMessage::Response {
            request_id: "rid-1".to_string(),
            target_peer: id.endpoint_id(),
            payload: Box::new(resp.clone()),
        };
        let decoded = worker_wire_round_trip(msg, &id);
        match decoded {
            WorkerWireMessage::Response { payload, .. } => assert_eq!(*payload, resp),
            _ => panic!("expected Response"),
        }
    }
}

#[test]
fn worker_role_info_feedback_round_trip_and_name() {
    let info = WorkerRoleInfo::Feedback {
        reports_accepted: 17,
        reports_rejected: 4,
        currently_rate_limited: 2,
        global_rate_limited: false,
    };
    let bytes = bincode::serialize(&info).unwrap();
    let decoded: WorkerRoleInfo = bincode::deserialize(&bytes).unwrap();
    assert_eq!(info, decoded);
    assert_eq!(info.role_name(), "feedback");
}
```

- [ ] **Step 2: Run tests, verify compile failures**

Run: `cargo test -p willow-common feedback_err_reason_variants_round_trip worker_request_feedback_round_trip worker_response_feedback_round_trip worker_role_info_feedback_round_trip_and_name`
Expected: COMPILE FAIL — missing variants on `WorkerRequest`, `WorkerResponse`, `WorkerRoleInfo`, and missing `FeedbackErrReason` type.

- [ ] **Step 3: Add the new variants and the error enum**

Edit `crates/common/src/worker_types.rs`:

1. Add `#[non_exhaustive]` to `WorkerRoleInfo`, `WorkerRequest`, and `WorkerResponse` (the existing enum declarations).

2. Add a new `Feedback` variant to `WorkerRoleInfo` (alongside `Replay` and `Storage`):

   ```rust
   Feedback {
       reports_accepted: u64,
       reports_rejected: u64,
       /// Gauge: peers currently throttled by the per-peer bucket.
       currently_rate_limited: u32,
       /// Gauge: true if the worker is hot-tripped on the global cap.
       global_rate_limited: bool,
   },
   ```

3. Extend `WorkerRoleInfo::role_name()` (currently around line 40 of the file) with a new arm:

   ```rust
   WorkerRoleInfo::Feedback { .. } => "feedback",
   ```

4. Add a new `Feedback` variant to `WorkerRequest`:

   ```rust
   Feedback {
       /// 16-byte client-generated dedup key. Worker maintains an LRU
       /// cache of (signer, dedup_id) → issue_url so retries return
       /// the original URL.
       dedup_id: [u8; 16],
       /// 1..=200 chars (worker-validated).
       title: String,
       category: FeedbackCategory,
       /// 1..=8000 chars (worker-validated). Worker wraps this
       /// verbatim in a fenced markdown code block on GitHub.
       body: String,
       diagnostics: Option<FeedbackDiagnostics>,
   },
   ```

5. Add two new variants to `WorkerResponse`:

   ```rust
   FeedbackOk { issue_url: String },
   FeedbackErr { reason: FeedbackErrReason },
   ```

6. Add the new error enum *above* the `#[cfg(test)] mod tests` block:

   ```rust
   /// Reason a feedback request was rejected. Units are MILLISECONDS to
   /// align with the broader `WireRejectReason` design
   /// ([`docs/specs/2026-04-24-error-prefixes.md`]); consolidating the
   /// two enums is a follow-up.
   #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
   #[non_exhaustive]
   pub enum FeedbackErrReason {
       RateLimited { retry_after_ms: u64 },
       /// `field` <= 64 chars; `message` <= 200 chars (worker-enforced
       /// before constructing the reply, client-enforced on receipt).
       InvalidInput { field: String, message: String },
       GithubFailure {
           status: u16,
           /// GitHub's `message` field, truncated to 200 chars.
           message: Option<String>,
       },
       /// Worker has no PAT configured, or PAT was revoked (401).
       Unconfigured,
   }
   ```

- [ ] **Step 4: Run tests, verify all pass**

Run: `cargo test -p willow-common`
Expected: all tests pass — both the four new feedback tests and every pre-existing test (the `#[non_exhaustive]` attributes don't change runtime behavior, only consumer-side compile guards).

- [ ] **Step 5: Verify nothing breaks downstream**

Run: `cargo check --workspace --all-targets 2>&1 | tail -40`
Expected: pass. The `#[non_exhaustive]` markers will only break callers that match exhaustively — workspace-internal callers all match exhaustively, so any failures here mean we forgot a `_` arm somewhere. Add `_ => unreachable!()` (or a real arm) at every match site that breaks. Likely candidates are display/log code paths in `crates/worker`, `crates/replay`, `crates/storage`.

- [ ] **Step 6: Commit**

```bash
git add crates/common/src/worker_types.rs
# Add any other files modified in step 5 above
git commit -m "feat(common): add Feedback variants to worker wire types"
```

### Task 1.3: Make `WorkerRole::handle_request` async + add `signer` parameter

**Files:**
- Modify: `crates/common/src/worker_types.rs` (the `WorkerRole` trait)
- Modify: `crates/worker/Cargo.toml` (add `async-trait` dep)
- Modify: `crates/worker/src/actors/mod.rs` (`WorkerRequestMsg` carries signer)
- Modify: `crates/worker/src/actors/state.rs` (handler awaits role, threads signer)
- Modify: `crates/worker/src/actors/network.rs` (pass requester signer into `WorkerRequestMsg`)
- Modify: `crates/worker/src/actors/sync.rs` (test role + internal sync requests pass local peer as signer)
- Modify: `crates/worker/src/actors/heartbeat.rs` (test role becomes async)
- Modify: `crates/replay/src/role.rs` (impl becomes `async fn handle_request(_signer, ...)`)
- Modify: `crates/storage/src/role.rs` (same)

This is the load-bearing change. Approach: extend the message struct first (so the field exists), then update the trait, then fix the four impl sites in lockstep. Compile-driven — `cargo check` tells us when we've covered every site.

- [ ] **Step 1: Inspect every existing `impl WorkerRole` site so the change is covered**

Run: `grep -rn "impl WorkerRole\|fn handle_request" crates/`
Expected output (the current impls — confirm these match before continuing):

- `crates/replay/src/role.rs:264` — `fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse`
- `crates/storage/src/role.rs:62` — same
- `crates/worker/src/actors/state.rs:127` — TestRole (in tests)
- `crates/worker/src/actors/sync.rs:108` — TestSyncRole (in tests)
- `crates/worker/src/actors/heartbeat.rs:124` — TestRole (in tests)

If line numbers drift, follow the names — there are exactly five impls and one trait declaration to update.

- [ ] **Step 2: Add `async-trait` to `crates/worker/Cargo.toml`**

Add under `[dependencies]`:

```toml
async-trait = "0.1"
```

Run: `cargo check -p willow-worker`
Expected: pass (just pulls the dep).

- [ ] **Step 3: Add `EndpointId` to `WorkerRequestMsg`**

In `crates/worker/src/actors/mod.rs` (around line 26), update the message:

```rust
// Before:
pub struct WorkerRequestMsg(pub WorkerRequest);

// After:
pub struct WorkerRequestMsg {
    pub req: willow_common::WorkerRequest,
    pub signer: willow_identity::EndpointId,
}
```

- [ ] **Step 4: Update the trait declaration in `willow-common`**

In `crates/common/src/worker_types.rs` (around line 131), change:

```rust
// Before:
pub trait WorkerRole: Send + 'static {
    fn role_info(&self) -> WorkerRoleInfo;
    fn on_event(&mut self, event: &Event);
    fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse;
    fn heads_summaries(&self) -> Vec<(String, HeadsSummary)> {
        vec![]
    }
}

// After:
#[async_trait::async_trait]
pub trait WorkerRole: Send + 'static {
    fn role_info(&self) -> WorkerRoleInfo;
    fn on_event(&mut self, event: &Event);
    /// Handle an inbound request from a client. `signer` is the
    /// verified Ed25519 signer of the inbound `WireMessage`; roles
    /// that don't need it (replay, storage) ignore the parameter.
    async fn handle_request(
        &mut self,
        signer: willow_identity::EndpointId,
        req: WorkerRequest,
    ) -> WorkerResponse;
    fn heads_summaries(&self) -> Vec<(String, HeadsSummary)> {
        vec![]
    }
}
```

Add `async-trait` to `crates/common/Cargo.toml` `[dependencies]`:

```toml
async-trait = "0.1"
```

- [ ] **Step 5: Run cargo check to discover every impl site that needs updating**

Run: `cargo check --workspace --all-targets 2>&1 | grep -E 'error\[|fn handle_request' | head -40`
Expected: errors at five impl sites — replay, storage, and the three test roles. Each will say "method `handle_request` has an incompatible type for trait" or similar.

- [ ] **Step 6: Update `ReplayRole` impl in `crates/replay/src/role.rs`**

Find the `impl WorkerRole for ReplayRole` block (currently around line 223 with `handle_request` at ~264). Replace the trait impl:

```rust
#[async_trait::async_trait]
impl WorkerRole for ReplayRole {
    // ... (role_info, on_event unchanged) ...

    async fn handle_request(
        &mut self,
        _signer: willow_identity::EndpointId,
        req: WorkerRequest,
    ) -> WorkerResponse {
        // existing body unchanged
    }

    // ... (heads_summaries unchanged) ...
}
```

Add `async-trait = "0.1"` to `crates/replay/Cargo.toml` `[dependencies]`.

Run: `cargo check -p willow-replay`
Expected: pass.

- [ ] **Step 7: Update `StorageRole` impl in `crates/storage/src/role.rs`**

Same shape as Step 6 — wrap the impl in `#[async_trait::async_trait]`, prepend `async`, accept `_signer: EndpointId`. Add `async-trait = "0.1"` to `crates/storage/Cargo.toml`.

Run: `cargo check -p willow-storage`
Expected: pass.

- [ ] **Step 8: Update the three test-role impls in `crates/worker/src/actors/`**

For each of `state.rs:113`, `heartbeat.rs:124`, `sync.rs:108`, wrap the impl in `#[async_trait::async_trait]`, change the signature to:

```rust
async fn handle_request(
    &mut self,
    _signer: willow_identity::EndpointId,
    req: WorkerRequest,
) -> WorkerResponse {
    // existing body
}
```

Run: `cargo check -p willow-worker --all-targets`
Expected: pass.

- [ ] **Step 9: Update the state actor's handler to `.await` the role and pass the signer**

In `crates/worker/src/actors/state.rs` (around line 52), the existing handler is:

```rust
impl Handler<WorkerRequestMsg> for StateActor {
    fn handle(
        &mut self,
        msg: WorkerRequestMsg,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = crate::types::WorkerResponse> + Send {
        let response = self.role.handle_request(msg.0);
        async move { response }
    }
}
```

Replace with:

```rust
impl Handler<WorkerRequestMsg> for StateActor {
    async fn handle(
        &mut self,
        msg: WorkerRequestMsg,
        _ctx: &mut Context<Self>,
    ) -> crate::types::WorkerResponse {
        self.role.handle_request(msg.signer, msg.req).await
    }
}
```

- [ ] **Step 10: Update internal `ask` callers in `state.rs` and `sync.rs` that construct `WorkerRequestMsg`**

Internal sync-actor code paths (like `crates/worker/src/actors/state.rs:186`, `:199`, `:244`) currently call `addr.ask(WorkerRequestMsg(WorkerRequest::Sync { ... }))`. These are *internal* synthetic requests, not from the network; they should pass the worker's own peer ID as the signer.

Find each `WorkerRequestMsg(WorkerRequest::...)` construction in `crates/worker/src/`. There's a pre-existing `local_peer_id` in scope at `network.rs`; `state.rs` and `sync.rs` need to obtain it from the actor's stored identity. If the actor doesn't currently hold the peer ID, plumb it in via the actor's constructor. Update each call site to:

```rust
addr.ask(WorkerRequestMsg {
    req: WorkerRequest::Sync { /* ... */ },
    signer: self.local_peer_id, // or whatever the actor field is named
}).await
```

Run: `cargo check -p willow-worker --all-targets`
Expected: pass.

- [ ] **Step 11: Update `network.rs` to pass the verified requester through**

In `crates/worker/src/actors/network.rs` (around line 138), the existing call is:

```rust
state_addr.ask(WorkerRequestMsg(payload)).await
```

`requester` (the verified gossip signer) is already in scope at line 133. Replace with:

```rust
state_addr.ask(WorkerRequestMsg { req: payload, signer: requester }).await
```

Run: `cargo check -p willow-worker --all-targets`
Expected: pass.

- [ ] **Step 12: Run all worker-side tests**

Run: `cargo test -p willow-common -p willow-worker -p willow-replay -p willow-storage`
Expected: all tests pass. The trait change is observationally invisible to existing roles — they ignore the new parameter and don't await anything inside `handle_request`.

- [ ] **Step 13: Commit**

```bash
git add crates/
git commit -m "refactor(worker): make WorkerRole::handle_request async + accept signer"
```

---

## Phase 2: `willow-feedback` crate

**Why now:** the trait is async and signer-aware; all the new wire types exist. We can build the worker on top.

**Approach:** TDD against the `FeedbackRole` directly. We isolate GitHub via a `GithubClient` trait so role tests don't talk to the network. The role is the integration point; the supporting modules (`sanitize`, `handle`, `ratelimit`, `salt`, `throttle`) each have their own focused unit tests.

### Task 2.1: Scaffold the crate

**Files:**
- Create: `crates/feedback/Cargo.toml`
- Create: `crates/feedback/build.rs`
- Create: `crates/feedback/src/main.rs` (placeholder so `cargo check` passes)
- Create: `crates/feedback/src/lib.rs` (empty placeholder; modules added in later tasks)
- Modify: `Cargo.toml` (root) — add `crates/feedback` to `[workspace] members`

- [ ] **Step 1: Add the workspace member**

Edit the root `Cargo.toml` `[workspace] members` array. Insert `"crates/feedback"` alphabetically.

- [ ] **Step 2: Create `crates/feedback/Cargo.toml`**

```toml
[package]
name = "willow-feedback"
version = "0.1.0"
edition.workspace = true

[[bin]]
name = "willow-feedback"
path = "src/main.rs"

[dependencies]
willow-common = { path = "../common" }
willow-identity = { path = "../identity" }
willow-network = { path = "../network" }
willow-state = { path = "../state" }
willow-worker = { path = "../worker" }

anyhow = { workspace = true }
async-trait = "0.1"
blake3 = "1"
bytes = { workspace = true }
clap = { version = "4", features = ["derive"] }
filetime = "0.2"
rand = { version = "0.8", features = ["std", "std_rng"] }
regex = "1"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
secrecy = "0.10"
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true, features = ["full"] }
tracing = { workspace = true }
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
url = "2"

[dev-dependencies]
tracing-test = "0.2"
```

- [ ] **Step 3: Create `crates/feedback/build.rs` to inject the build SHA**

```rust
//! Inject `WILLOW_BUILD_SHA` via `option_env!` so diagnostics can
//! surface the short git SHA. Best-effort: empty in dev builds.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=WILLOW_BUILD_SHA");
    if std::env::var_os("WILLOW_BUILD_SHA").is_some() {
        return; // caller already set it
    }
    if let Ok(out) = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
    {
        if out.status.success() {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !sha.is_empty() {
                println!("cargo:rustc-env=WILLOW_BUILD_SHA={sha}");
            }
        }
    }
}
```

- [ ] **Step 4: Create a placeholder `crates/feedback/src/lib.rs`**

```rust
//! Willow feedback worker library. Modules added in subsequent
//! plan tasks.

pub mod role;
pub mod github;
pub mod handle;
pub mod ratelimit;
pub mod sanitize;
pub mod salt;
pub mod throttle;
pub mod wordlist;
```

(Each `pub mod` line will fail compilation until the corresponding file exists. Add them as we go — Step 5 only stubs the binary so `cargo check` passes for now. We delete this `lib.rs` placeholder content and replace it as each module is added.)

For Step 5's check to pass right now, *temporarily* leave `lib.rs` empty:

```rust
//! Willow feedback worker library. Modules added in subsequent
//! plan tasks.
```

- [ ] **Step 5: Create a placeholder `crates/feedback/src/main.rs`**

```rust
//! Willow Feedback Node — stub.
//!
//! Filled in by Task 2.10. This stub exists so the crate compiles
//! while earlier modules are being built.

fn main() {
    eprintln!("willow-feedback: not yet implemented");
    std::process::exit(1);
}
```

- [ ] **Step 6: Verify the crate compiles**

Run: `cargo check -p willow-feedback`
Expected: pass.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/feedback/
git commit -m "feat(feedback): scaffold willow-feedback crate"
```

### Task 2.2: Body + title sanitization (`sanitize.rs`)

**Files:**
- Create: `crates/feedback/src/sanitize.rs`

This is the security boundary. Tests come first, drive the implementation.

- [ ] **Step 1: Write the failing tests**

Create `crates/feedback/src/sanitize.rs`:

```rust
//! User-supplied content sanitization for feedback issues.
//!
//! - `wrap_body_fenced` wraps the user body in a backtick code block
//!   long enough that no closing fence inside the body can escape.
//! - `sanitize_title` strips control / bidi codepoints and escapes
//!   leading brackets so the assembled title can't impersonate the
//!   metadata block.

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: assert the wrapped body is well-formed and the inner
    /// content survives byte-for-byte (modulo CRLF normalization).
    fn assert_wrap_round_trips(input: &str) {
        let wrapped = wrap_body_fenced(input);
        let normalized = input.replace("\r\n", "\n");
        assert!(wrapped.starts_with('`'), "must open with backticks");
        assert!(
            wrapped.contains(&normalized),
            "wrapped body must contain the normalized input verbatim"
        );
    }

    #[test]
    fn wraps_plain_body_with_min_three_backticks() {
        let out = wrap_body_fenced("hello world");
        assert!(out.starts_with("```text\n"));
        assert!(out.ends_with("\n```"));
    }

    #[test]
    fn escapes_body_containing_three_backticks() {
        let body = "code: ```\nrust\n```\nend";
        let out = wrap_body_fenced(body);
        // Must use at least 4 backticks since body has runs of 3.
        assert!(out.starts_with("````text\n"));
        assert!(out.ends_with("\n````"));
        assert!(out.contains(body));
    }

    #[test]
    fn handles_indented_closing_fence() {
        // Up-to-3-space indent counts as a valid close per CommonMark.
        let body = "stuff\n   ```\nmore";
        let out = wrap_body_fenced(body);
        assert!(out.starts_with("````text\n"));
    }

    #[test]
    fn ignores_four_space_indent() {
        // 4+ spaces before backticks is a code block, not a fence.
        let body = "stuff\n    ```\nmore";
        let out = wrap_body_fenced(body);
        assert!(out.starts_with("```text\n"), "no escalation needed");
    }

    #[test]
    fn handles_crlf_line_endings() {
        let body = "line1\r\n```\r\nline3";
        let out = wrap_body_fenced(body);
        assert!(out.starts_with("````text\n"));
        // Wrapped output uses LF only.
        assert!(!out.contains("\r\n"));
    }

    #[test]
    fn ignores_tilde_fences() {
        let body = "~~~\nhi\n~~~";
        let out = wrap_body_fenced(body);
        assert!(out.starts_with("```text\n"), "tildes don't close backticks");
    }

    #[test]
    fn handles_info_string_after_fence() {
        // `\`\`\`text` on its own line is a CLOSE if it's just backticks
        // and whitespace; with `text` after, it's an open. Sanitizer
        // must still escalate because the regex `^[ ]{0,3}\`{N,}[ \t]*$`
        // only matches *closing* fences.
        let body = "stuff\n```\nmore"; // bare close
        let out = wrap_body_fenced(body);
        assert!(out.starts_with("````text\n"));

        let body2 = "stuff\n```rust\nmore"; // not a close
        let out2 = wrap_body_fenced(body2);
        assert!(out2.starts_with("```text\n"));
    }

    #[test]
    fn html_entity_backticks_dont_escape() {
        // HTML entities are rendered as text inside fenced blocks, so
        // they don't escape — sanitizer doesn't need to do anything.
        let body = "&#96;&#96;&#96;\n`code`\n&#96;&#96;&#96;";
        let out = wrap_body_fenced(body);
        assert!(out.starts_with("```text\n"));
        assert!(out.contains(body));
    }

    #[test]
    fn five_backticks_in_body_escalates_to_six() {
        let body = "weird: `````".to_string();
        let out = wrap_body_fenced(&body);
        assert!(out.starts_with("``````text\n"), "got: {}", out);
    }

    #[test]
    fn wrap_round_trips_assorted_inputs() {
        for s in [
            "",
            "hello",
            "@everyone please look",
            "![pixel](https://attacker/?ip=)",
            "<img onerror=alert(1)>",
            "[link](javascript:alert(1))",
            "#1 issue cross-ref",
        ] {
            assert_wrap_round_trips(s);
        }
    }

    #[test]
    fn sanitize_title_strips_controls() {
        let raw = "hello\u{0007}world\u{0001}";
        assert_eq!(sanitize_title(raw), "helloworld");
    }

    #[test]
    fn sanitize_title_strips_bidi_overrides() {
        let raw = "hello\u{202E}evil";
        assert_eq!(sanitize_title(raw), "helloevil");
    }

    #[test]
    fn sanitize_title_collapses_internal_whitespace() {
        let raw = "hello   \tworld   bar";
        assert_eq!(sanitize_title(raw), "hello world bar");
    }

    #[test]
    fn sanitize_title_escapes_leading_brackets() {
        assert_eq!(sanitize_title("[bug] crash"), r"\[bug\] crash");
        assert_eq!(sanitize_title("]nope"), r"\]nope");
    }
}
```

- [ ] **Step 2: Wire `sanitize` into `lib.rs`**

Replace the placeholder content of `crates/feedback/src/lib.rs` with:

```rust
//! Willow feedback worker library.

pub mod sanitize;
```

- [ ] **Step 3: Run tests, verify failure**

Run: `cargo test -p willow-feedback --lib sanitize::tests`
Expected: COMPILE FAIL — `cannot find function wrap_body_fenced` and `sanitize_title`.

- [ ] **Step 4: Implement the sanitizers**

Append to `crates/feedback/src/sanitize.rs` *above* the test module:

```rust
use regex::Regex;
use std::sync::OnceLock;

/// Match a CommonMark closing-fence line for backtick fences:
/// 0–3 leading spaces, three or more backticks, optional trailing
/// whitespace, end of line.
fn close_fence_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[ ]{0,3}(`{3,})[ \t]*$").unwrap())
}

/// Wrap `body` in a backtick fenced markdown block with the `text`
/// info-string. Fence length is the smallest N ≥ 3 such that no line
/// in the body is `^[ ]{0,3}` `` ` ``{N,}` `[ \t]*$` — guaranteeing
/// no body line can close our fence.
///
/// CRLF line endings are normalized to LF before scanning and in the
/// output.
pub fn wrap_body_fenced(body: &str) -> String {
    let body = body.replace("\r\n", "\n");
    let mut max_run: usize = 0;
    for line in body.split('\n') {
        if let Some(c) = close_fence_re().captures(line) {
            let n = c.get(1).unwrap().as_str().len();
            if n > max_run {
                max_run = n;
            }
        }
    }
    let fence_len = std::cmp::max(3, max_run + 1);
    let fence = "`".repeat(fence_len);
    format!("{fence}text\n{body}\n{fence}")
}

/// Sanitize a feedback title. Strips ASCII control codepoints
/// (0x00–0x1F, 0x7F) and Unicode bidi/RTL override codepoints
/// (U+202A..=U+202E, U+2066..=U+2069). Collapses internal
/// runs of whitespace to single spaces. Escapes leading `[` / `]`
/// with a backslash so the assembled title can't impersonate the
/// metadata-block prefix.
pub fn sanitize_title(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_ws = false;
    for ch in raw.chars() {
        let c = ch as u32;
        let is_ascii_control = c <= 0x1F || c == 0x7F;
        let is_bidi_override = matches!(c, 0x202A..=0x202E | 0x2066..=0x2069);
        if is_ascii_control || is_bidi_override {
            continue;
        }
        if ch.is_whitespace() {
            if !last_was_ws && !out.is_empty() {
                out.push(' ');
            }
            last_was_ws = true;
        } else {
            last_was_ws = false;
            out.push(ch);
        }
    }
    let trimmed = out.trim_end().to_string();
    // Escape leading [ or ] so a user title can't fake the worker prefix.
    if trimmed.starts_with('[') {
        format!(r"\[{}", &trimmed[1..])
    } else if trimmed.starts_with(']') {
        format!(r"\]{}", &trimmed[1..])
    } else {
        trimmed
    }
}
```

- [ ] **Step 5: Run tests, verify all pass**

Run: `cargo test -p willow-feedback --lib sanitize::tests`
Expected: all 14 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/feedback/src/lib.rs crates/feedback/src/sanitize.rs
git commit -m "feat(feedback): add body + title sanitization"
```

### Task 2.3: BIP-39 wordlist + salted reporter handle (`wordlist.rs`, `handle.rs`)

**Files:**
- Create: `crates/feedback/src/wordlist.rs`
- Create: `crates/feedback/src/handle.rs`
- Modify: `crates/feedback/src/lib.rs`

The reporter handle is `blake3(salt || peer_id)[..8]` rendered as 4 BIP-39 English words (44 bits) plus a 5-hex suffix (20 bits).

- [ ] **Step 1: Vendor the BIP-39 English wordlist**

The official BIP-39 English wordlist is 2048 words, ordered, all lowercase. The canonical source is https://github.com/bitcoin/bips/blob/master/bip-0039/english.txt. Implementer downloads the raw file and writes it as a Rust array literal. Approach:

1. Fetch the wordlist (one-time):

   ```bash
   curl -sSf https://raw.githubusercontent.com/bitcoin/bips/master/bip-0039/english.txt > /tmp/bip39-english.txt
   wc -l /tmp/bip39-english.txt
   # expect: 2048
   ```

2. Generate the Rust file:

   ```bash
   {
     echo '//! Vendored BIP-39 English wordlist (2048 words). Source:'
     echo '//! https://github.com/bitcoin/bips/blob/master/bip-0039/english.txt'
     echo
     echo 'pub const WORDS: [&str; 2048] = ['
     awk '{ printf "    \"%s\",\n", $1 }' /tmp/bip39-english.txt
     echo '];'
   } > crates/feedback/src/wordlist.rs
   ```

3. Spot-check a couple of canonical entries (the first word is `abandon`, the last is `zoo`).

- [ ] **Step 2: Add a sanity test for the wordlist**

Append to `crates/feedback/src/wordlist.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wordlist_has_2048_entries() {
        assert_eq!(WORDS.len(), 2048);
    }

    #[test]
    fn first_and_last_words_are_canonical() {
        assert_eq!(WORDS[0], "abandon");
        assert_eq!(WORDS[2047], "zoo");
    }

    #[test]
    fn all_words_lowercase_and_nonempty() {
        for w in WORDS {
            assert!(!w.is_empty());
            assert_eq!(w, w.to_lowercase());
        }
    }
}
```

- [ ] **Step 3: Wire into `lib.rs`**

Edit `crates/feedback/src/lib.rs`:

```rust
pub mod sanitize;
pub mod wordlist;
pub mod handle;
```

- [ ] **Step 4: Write the failing handle tests**

Create `crates/feedback/src/handle.rs`:

```rust
//! Salted reporter handle. Renders an opaque human-friendly string
//! from `(salt, peer_id_bytes)` so maintainers can correlate reports
//! from the same user without exposing the raw Ed25519 public key.

use willow_identity::EndpointId;

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;

    #[test]
    fn handle_is_deterministic_for_same_inputs() {
        let id = Identity::generate().endpoint_id();
        let salt = [0xABu8; 32];
        let h1 = compute_handle(&salt, &id);
        let h2 = compute_handle(&salt, &id);
        assert_eq!(h1, h2);
    }

    #[test]
    fn handle_changes_when_salt_rotates() {
        let id = Identity::generate().endpoint_id();
        let h1 = compute_handle(&[0u8; 32], &id);
        let h2 = compute_handle(&[1u8; 32], &id);
        assert_ne!(h1, h2);
    }

    #[test]
    fn handle_distinguishes_distinct_peers() {
        let salt = [0u8; 32];
        let id1 = Identity::generate().endpoint_id();
        let id2 = Identity::generate().endpoint_id();
        assert_ne!(compute_handle(&salt, &id1), compute_handle(&salt, &id2));
    }

    #[test]
    fn handle_format_is_four_words_dash_five_hex() {
        let id = Identity::generate().endpoint_id();
        let h = compute_handle(&[0u8; 32], &id);
        let parts: Vec<&str> = h.split('-').collect();
        assert_eq!(parts.len(), 5, "expected 4 words + 5-hex suffix, got {h}");
        for word in &parts[..4] {
            assert!(crate::wordlist::WORDS.contains(word), "{word} not in wordlist");
        }
        assert_eq!(parts[4].len(), 5);
        assert!(parts[4].chars().all(|c| c.is_ascii_hexdigit()));
        assert!(parts[4].chars().all(|c| !c.is_ascii_uppercase()));
    }
}
```

- [ ] **Step 5: Run tests, expect compile failure**

Run: `cargo test -p willow-feedback --lib handle::tests`
Expected: COMPILE FAIL — `cannot find function compute_handle`.

- [ ] **Step 6: Implement `compute_handle`**

Append to `crates/feedback/src/handle.rs` *above* the test module:

```rust
use crate::wordlist::WORDS;

/// Compute the salted-hash reporter handle. Layout:
/// - `blake3(salt || peer_id_bytes)[..8]` = 64 bits.
/// - First 44 bits → 4 BIP-39 English words (11 bits each).
/// - Last 20 bits → 5 lowercase hex chars.
/// Final form: `word-word-word-word-NNNNN`.
pub fn compute_handle(salt: &[u8; 32], peer_id: &EndpointId) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(salt);
    hasher.update(peer_id.as_bytes());
    let digest = hasher.finalize();
    let bytes = digest.as_bytes();
    // Pull the first 8 bytes into a u64 (big-endian).
    let mut h64 = 0u64;
    for &b in &bytes[..8] {
        h64 = (h64 << 8) | (b as u64);
    }
    // Top 44 bits: four 11-bit words.
    let mut words: [&'static str; 4] = ["", "", "", ""];
    for i in 0..4 {
        let shift = 64 - (i + 1) * 11;
        let idx = ((h64 >> shift) & 0x7FF) as usize;
        words[i] = WORDS[idx];
    }
    // Bottom 20 bits → 5 hex chars.
    let suffix_bits = (h64 & 0xF_FFFF) as u32; // 20 bits
    format!(
        "{}-{}-{}-{}-{:05x}",
        words[0], words[1], words[2], words[3], suffix_bits,
    )
}
```

This depends on `EndpointId::as_bytes()`. Verify that method exists:

```bash
grep -n "fn as_bytes" crates/identity/src/lib.rs
```

If `as_bytes()` returns `&[u8; N]` for some `N`, the call works as-is. If it returns `Vec<u8>`, replace `peer_id.as_bytes()` with `&peer_id.as_bytes()[..]`. If neither method exists (unlikely), use `peer_id.to_string().as_bytes()` — bech32 form is also fine for this purpose.

- [ ] **Step 7: Run tests, verify all pass**

Run: `cargo test -p willow-feedback --lib handle::tests wordlist::tests`
Expected: 7 tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/feedback/src/lib.rs crates/feedback/src/handle.rs crates/feedback/src/wordlist.rs
git commit -m "feat(feedback): salted-hash reporter handle (blake3 + BIP-39)"
```

### Task 2.4: Token-bucket rate limiter (`ratelimit.rs`)

**Files:**
- Create: `crates/feedback/src/ratelimit.rs`
- Modify: `crates/feedback/src/lib.rs`

A continuous-refill token bucket. We use `Instant` so tests can drive time deterministically via an injected clock — but for v1 simplicity we drive the clock through a small trait `Clock` with a real `SystemClock` and a test `MockClock`.

- [ ] **Step 1: Write the failing tests**

Create `crates/feedback/src/ratelimit.rs`:

```rust
//! Continuous-refill token-bucket rate limiter.
//!
//! - Per-peer buckets keyed by `EndpointId`.
//! - One worker-wide global bucket.
//! - On rejection, returns the exact wait time to the next available
//!   token in milliseconds.

use std::collections::HashMap;
use std::time::Duration;

use willow_identity::EndpointId;

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;

    fn peer() -> EndpointId {
        Identity::generate().endpoint_id()
    }

    #[test]
    fn fresh_bucket_allows_burst_capacity() {
        let mut clock = MockClock::new();
        let mut rl = RateLimiter::new(5, 50, &mut clock);
        let p = peer();
        for _ in 0..5 {
            assert!(rl.try_take(&p, &mut clock).is_ok());
        }
    }

    #[test]
    fn per_peer_limit_returns_retry_after() {
        let mut clock = MockClock::new();
        let mut rl = RateLimiter::new(5, 50, &mut clock);
        let p = peer();
        for _ in 0..5 {
            rl.try_take(&p, &mut clock).unwrap();
        }
        match rl.try_take(&p, &mut clock) {
            Err(RateLimited::PerPeer { retry_after_ms }) => {
                // Refill rate is 5/3600s ≈ 1 per 720s.
                let expected_ms = 3600 * 1000 / 5;
                let lo = expected_ms as i64 - 50;
                let hi = expected_ms as i64 + 50;
                assert!(
                    (retry_after_ms as i64) >= lo && (retry_after_ms as i64) <= hi,
                    "got {retry_after_ms}, expected near {expected_ms}"
                );
            }
            other => panic!("expected PerPeer, got {other:?}"),
        }
    }

    #[test]
    fn distinct_peers_have_independent_buckets() {
        let mut clock = MockClock::new();
        let mut rl = RateLimiter::new(2, 50, &mut clock);
        let a = peer();
        let b = peer();
        rl.try_take(&a, &mut clock).unwrap();
        rl.try_take(&a, &mut clock).unwrap();
        assert!(matches!(rl.try_take(&a, &mut clock), Err(RateLimited::PerPeer { .. })));
        // b is unaffected.
        rl.try_take(&b, &mut clock).unwrap();
        rl.try_take(&b, &mut clock).unwrap();
    }

    #[test]
    fn global_limit_trips_across_distinct_peers() {
        let mut clock = MockClock::new();
        // Per-peer 100 (won't trip), global 3.
        let mut rl = RateLimiter::new(100, 3, &mut clock);
        for _ in 0..3 {
            let p = peer();
            rl.try_take(&p, &mut clock).unwrap();
        }
        let p4 = peer();
        match rl.try_take(&p4, &mut clock) {
            Err(RateLimited::Global { retry_after_ms }) => {
                let expected_ms = 3600 * 1000 / 3;
                assert!(retry_after_ms >= (expected_ms as u64).saturating_sub(50));
            }
            other => panic!("expected Global, got {other:?}"),
        }
    }

    #[test]
    fn refill_replenishes_a_token_after_the_advertised_wait() {
        let mut clock = MockClock::new();
        let mut rl = RateLimiter::new(2, 50, &mut clock);
        let p = peer();
        rl.try_take(&p, &mut clock).unwrap();
        rl.try_take(&p, &mut clock).unwrap();
        let err = rl.try_take(&p, &mut clock).unwrap_err();
        let wait_ms = match err {
            RateLimited::PerPeer { retry_after_ms } => retry_after_ms,
            _ => panic!("expected PerPeer"),
        };
        clock.advance(Duration::from_millis(wait_ms));
        rl.try_take(&p, &mut clock).unwrap();
    }

    #[test]
    fn currently_rate_limited_count_reflects_throttled_peers() {
        let mut clock = MockClock::new();
        let mut rl = RateLimiter::new(1, 50, &mut clock);
        let a = peer();
        let b = peer();
        rl.try_take(&a, &mut clock).unwrap();
        let _ = rl.try_take(&a, &mut clock); // a is now throttled
        rl.try_take(&b, &mut clock).unwrap();
        // a is throttled (saturated bucket); b is not.
        assert_eq!(rl.currently_rate_limited(&clock), 1);
    }
}
```

- [ ] **Step 2: Run tests, expect compile failure**

Run: `cargo test -p willow-feedback --lib ratelimit::tests`
Expected: COMPILE FAIL — types don't exist yet.

- [ ] **Step 3: Implement the limiter**

Append to `crates/feedback/src/ratelimit.rs` *above* the test module:

```rust
/// Abstract clock so tests can drive time without sleeping.
pub trait Clock {
    fn now(&self) -> std::time::Instant;
}

#[derive(Default)]
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }
}

#[cfg(test)]
pub struct MockClock {
    base: std::time::Instant,
    offset: Duration,
}
#[cfg(test)]
impl MockClock {
    pub fn new() -> Self {
        Self {
            base: std::time::Instant::now(),
            offset: Duration::ZERO,
        }
    }
    pub fn advance(&mut self, by: Duration) {
        self.offset += by;
    }
}
#[cfg(test)]
impl Clock for MockClock {
    fn now(&self) -> std::time::Instant {
        self.base + self.offset
    }
}

#[derive(Debug)]
pub enum RateLimited {
    PerPeer { retry_after_ms: u64 },
    Global { retry_after_ms: u64 },
}

#[derive(Clone, Copy)]
struct Bucket {
    /// Tokens available, fractional (refills smoothly between integers).
    tokens: f64,
    /// Last time we updated `tokens`.
    last: std::time::Instant,
}

impl Bucket {
    fn fresh(capacity: u32, now: std::time::Instant) -> Self {
        Self {
            tokens: capacity as f64,
            last: now,
        }
    }
    fn refill(&mut self, capacity: u32, refill_per_sec: f64, now: std::time::Instant) {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * refill_per_sec).min(capacity as f64);
        self.last = now;
    }
    fn try_take(&mut self, capacity: u32, refill_per_sec: f64, now: std::time::Instant) -> Result<(), u64> {
        self.refill(capacity, refill_per_sec, now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            // Time to next full token.
            let need = 1.0 - self.tokens;
            let secs = need / refill_per_sec;
            Err((secs * 1000.0).ceil() as u64)
        }
    }
    fn is_throttled(&self, capacity: u32, refill_per_sec: f64, now: std::time::Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        let projected = (self.tokens + elapsed * refill_per_sec).min(capacity as f64);
        projected < 1.0
    }
}

pub struct RateLimiter {
    per_peer_capacity: u32,
    per_peer_refill_per_sec: f64,
    global_capacity: u32,
    global_refill_per_sec: f64,
    per_peer: HashMap<EndpointId, Bucket>,
    global: Bucket,
}

impl RateLimiter {
    pub fn new(per_peer_per_hour: u32, global_per_hour: u32, clock: &mut impl Clock) -> Self {
        let now = clock.now();
        Self {
            per_peer_capacity: per_peer_per_hour,
            per_peer_refill_per_sec: per_peer_per_hour as f64 / 3600.0,
            global_capacity: global_per_hour,
            global_refill_per_sec: global_per_hour as f64 / 3600.0,
            per_peer: HashMap::new(),
            global: Bucket::fresh(global_per_hour, now),
        }
    }

    pub fn try_take(
        &mut self,
        peer: &EndpointId,
        clock: &mut impl Clock,
    ) -> Result<(), RateLimited> {
        let now = clock.now();
        let bucket = self
            .per_peer
            .entry(*peer)
            .or_insert_with(|| Bucket::fresh(self.per_peer_capacity, now));
        if let Err(retry_after_ms) =
            bucket.try_take(self.per_peer_capacity, self.per_peer_refill_per_sec, now)
        {
            return Err(RateLimited::PerPeer { retry_after_ms });
        }
        if let Err(retry_after_ms) =
            self.global
                .try_take(self.global_capacity, self.global_refill_per_sec, now)
        {
            // Refund the per-peer token we just took: the request didn't
            // really happen.
            let bucket = self.per_peer.get_mut(peer).unwrap();
            bucket.tokens += 1.0;
            return Err(RateLimited::Global { retry_after_ms });
        }
        Ok(())
    }

    /// Number of peers whose per-peer bucket is currently throttled
    /// (would deny a `try_take` right now without waiting).
    pub fn currently_rate_limited(&self, clock: &impl Clock) -> u32 {
        let now = clock.now();
        self.per_peer
            .values()
            .filter(|b| {
                b.is_throttled(self.per_peer_capacity, self.per_peer_refill_per_sec, now)
            })
            .count() as u32
    }

    pub fn global_is_throttled(&self, clock: &impl Clock) -> bool {
        let now = clock.now();
        self.global
            .is_throttled(self.global_capacity, self.global_refill_per_sec, now)
    }
}
```

- [ ] **Step 4: Wire into `lib.rs`**

```rust
pub mod sanitize;
pub mod wordlist;
pub mod handle;
pub mod ratelimit;
```

- [ ] **Step 5: Run tests, verify all pass**

Run: `cargo test -p willow-feedback --lib ratelimit::tests`
Expected: 6 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/feedback/src/lib.rs crates/feedback/src/ratelimit.rs
git commit -m "feat(feedback): continuous-refill token-bucket rate limiter"
```

### Task 2.5: Salt file load-or-generate (`salt.rs`)

**Files:**
- Create: `crates/feedback/src/salt.rs`
- Modify: `crates/feedback/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Create `crates/feedback/src/salt.rs`:

```rust
//! 32-byte salt file used by the reporter-handle hash. Loaded at
//! startup; regenerated on demand via the `--generate-salt` CLI
//! flag (which writes a fresh salt and exits if the file is missing).

use std::path::Path;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn load_salt_returns_32_bytes_for_valid_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("salt");
        fs::write(&path, [0u8; 32]).unwrap();
        let salt = load_salt(&path).unwrap();
        assert_eq!(salt, [0u8; 32]);
    }

    #[test]
    fn load_salt_errors_on_missing_file() {
        let dir = tempdir().unwrap();
        let err = load_salt(&dir.path().join("missing")).unwrap_err();
        assert!(err.to_string().contains("not found") || err.to_string().contains("No such"));
    }

    #[test]
    fn load_salt_errors_on_wrong_length() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("short");
        fs::write(&path, [0u8; 16]).unwrap();
        let err = load_salt(&path).unwrap_err();
        assert!(err.to_string().contains("expected 32"));
    }

    #[test]
    fn generate_salt_creates_32_random_bytes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("salt");
        generate_salt(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 32);
        // Two consecutive generations should differ (extremely high prob).
        let path2 = dir.path().join("salt2");
        generate_salt(&path2).unwrap();
        let bytes2 = fs::read(&path2).unwrap();
        assert_ne!(bytes, bytes2);
    }

    #[test]
    fn generate_salt_refuses_to_overwrite_existing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("salt");
        fs::write(&path, [0u8; 32]).unwrap();
        let err = generate_salt(&path).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }
}
```

Add `tempfile = "3"` to `crates/feedback/Cargo.toml` `[dev-dependencies]`.

- [ ] **Step 2: Run tests, expect compile failure**

Run: `cargo test -p willow-feedback --lib salt::tests`
Expected: COMPILE FAIL — functions don't exist.

- [ ] **Step 3: Implement load + generate**

Append to `crates/feedback/src/salt.rs` *above* the test module:

```rust
use std::fs;

use anyhow::{anyhow, Context, Result};
use rand::RngCore;

/// Load a 32-byte salt from `path`. Errors if the file is missing,
/// the wrong length, or unreadable.
pub fn load_salt(path: &Path) -> Result<[u8; 32]> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read salt file at {}", path.display()))?;
    if bytes.len() != 32 {
        return Err(anyhow!(
            "salt file at {} is {} bytes, expected 32",
            path.display(),
            bytes.len()
        ));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// Generate a fresh 32-byte salt and write it to `path`. Errors if
/// the file already exists (caller is expected to delete it first
/// for rotation).
pub fn generate_salt(path: &Path) -> Result<()> {
    if path.exists() {
        return Err(anyhow!(
            "salt file at {} already exists; delete it first to rotate",
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).ok();
        }
    }
    let mut salt = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut salt);
    fs::write(path, salt)
        .with_context(|| format!("failed to write salt file to {}", path.display()))?;
    // Restrict permissions on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}
```

- [ ] **Step 4: Wire into `lib.rs`**

```rust
pub mod sanitize;
pub mod wordlist;
pub mod handle;
pub mod ratelimit;
pub mod salt;
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p willow-feedback --lib salt::tests`
Expected: 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/feedback/Cargo.toml crates/feedback/src/lib.rs crates/feedback/src/salt.rs
git commit -m "feat(feedback): salt file load + generate"
```

### Task 2.6: Startup-throttle gating (`throttle.rs`)

**Files:**
- Create: `crates/feedback/src/throttle.rs`
- Modify: `crates/feedback/src/lib.rs`

15-second startup throttle: atomic `O_CREAT | O_EXCL` on first boot; on subsequent boots, read mtime and sleep to enforce the gap. Bumps mtime via tempfile + rename.

- [ ] **Step 1: Write the failing tests**

Create `crates/feedback/src/throttle.rs`:

```rust
//! Startup throttle. Enforces a minimum 15-second gap between
//! consecutive worker starts so a crash-loop attacker can't reset
//! rate-limit buckets unbounded times.

use std::path::Path;
use std::time::Duration;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn first_boot_creates_file_and_does_not_sleep() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".feedback-last-boot");
        let elapsed = enforce_throttle(&path, Duration::from_secs(15)).unwrap();
        assert!(path.exists());
        assert!(elapsed < Duration::from_millis(500), "first boot should not sleep");
    }

    #[test]
    fn second_boot_within_window_sleeps_remainder() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".feedback-last-boot");
        // First call.
        enforce_throttle(&path, Duration::from_millis(200)).unwrap();
        // Second call immediately — should sleep ≈ 200ms.
        let elapsed = enforce_throttle(&path, Duration::from_millis(200)).unwrap();
        assert!(elapsed >= Duration::from_millis(150), "got {elapsed:?}");
        assert!(elapsed < Duration::from_millis(500));
    }

    #[test]
    fn second_boot_outside_window_does_not_sleep() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".feedback-last-boot");
        enforce_throttle(&path, Duration::from_millis(50)).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        let elapsed = enforce_throttle(&path, Duration::from_millis(50)).unwrap();
        assert!(elapsed < Duration::from_millis(20), "got {elapsed:?}");
    }
}
```

- [ ] **Step 2: Run tests, expect compile failure**

Run: `cargo test -p willow-feedback --lib throttle::tests`
Expected: COMPILE FAIL — `enforce_throttle` undefined.

- [ ] **Step 3: Implement throttle**

Append to `crates/feedback/src/throttle.rs` *above* the test module:

```rust
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result};
use filetime::{set_file_mtime, FileTime};

/// Enforce a minimum gap between consecutive boots by reading and
/// updating the mtime of `gate_path`. Returns the time spent inside
/// this call (mostly sleeping or zero).
///
/// On first boot, atomically creates the file via `O_CREAT|O_EXCL`.
/// On subsequent boots, reads mtime; if `delta < window`, sleeps
/// `window - delta`; then bumps mtime via tempfile+rename.
pub fn enforce_throttle(gate_path: &Path, window: Duration) -> Result<Duration> {
    let start = Instant::now();
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(gate_path)
    {
        Ok(mut f) => {
            // First boot — write timestamp, no sleep.
            let now = SystemTime::now();
            let secs = now
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs();
            f.write_all(secs.to_string().as_bytes()).ok();
            return Ok(start.elapsed());
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Fall through to mtime check.
        }
        Err(e) => return Err(e).context("opening gate file"),
    }

    // Read existing mtime.
    let metadata = std::fs::metadata(gate_path).context("stat gate file")?;
    let mtime = metadata.modified().context("read mtime")?;
    let delta = SystemTime::now()
        .duration_since(mtime)
        .unwrap_or(Duration::ZERO);
    if delta < window {
        std::thread::sleep(window - delta);
    }

    // Bump mtime atomically: write fresh contents to a sibling tempfile, rename over.
    let parent = gate_path.parent().unwrap_or_else(|| Path::new("."));
    let tempfile = parent.join(format!(
        ".{}.tmp",
        gate_path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let mut f = File::create(&tempfile).context("create tempfile")?;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    f.write_all(secs.to_string().as_bytes()).ok();
    drop(f);
    std::fs::rename(&tempfile, gate_path).context("atomic rename")?;
    let now = FileTime::from_system_time(SystemTime::now());
    set_file_mtime(gate_path, now).ok();

    Ok(start.elapsed())
}
```

- [ ] **Step 4: Wire into `lib.rs`**

```rust
pub mod sanitize;
pub mod wordlist;
pub mod handle;
pub mod ratelimit;
pub mod salt;
pub mod throttle;
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p willow-feedback --lib throttle::tests`
Expected: 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/feedback/src/lib.rs crates/feedback/src/throttle.rs
git commit -m "feat(feedback): startup-throttle gating with O_CREAT|O_EXCL"
```

### Task 2.7: GitHub client trait + reqwest impl + fixtures (`github.rs`)

**Files:**
- Create: `crates/feedback/src/github.rs`
- Create: `crates/feedback/tests/fixtures/github/201-created.json`
- Create: `crates/feedback/tests/fixtures/github/401-unauthorized.json`
- Create: `crates/feedback/tests/fixtures/github/403-secondary-rate-limit.json`
- Create: `crates/feedback/tests/fixtures/github/404-not-found.json`
- Create: `crates/feedback/tests/fixtures/github/422-validation.json`
- Modify: `crates/feedback/src/lib.rs`

We isolate GitHub behind a trait so the role can be tested with a mock. The reqwest impl is thin — POST + parse response.

- [ ] **Step 1: Capture the JSON fixtures**

Capture representative responses. Each fixture is a static JSON document recorded once and committed; tests parse them as if they came back from `reqwest`. The shapes below are recorded from GitHub's REST API documentation (`docs.github.com/en/rest/issues/issues#create-an-issue`) — adjust if the live API drift requires it.

`crates/feedback/tests/fixtures/github/201-created.json`:

```json
{
  "url": "https://api.github.com/repos/intendednull/willow/issues/42",
  "html_url": "https://github.com/intendednull/willow/issues/42",
  "number": 42,
  "state": "open",
  "title": "[Bug] It crashes",
  "body": "..."
}
```

`crates/feedback/tests/fixtures/github/422-validation.json`:

```json
{
  "message": "Validation Failed",
  "errors": [
    { "resource": "Issue", "code": "missing_field", "field": "title" }
  ],
  "documentation_url": "https://docs.github.com/rest/reference/issues#create-an-issue"
}
```

`crates/feedback/tests/fixtures/github/401-unauthorized.json`:

```json
{
  "message": "Bad credentials",
  "documentation_url": "https://docs.github.com/rest"
}
```

`crates/feedback/tests/fixtures/github/403-secondary-rate-limit.json`:

```json
{
  "message": "You have exceeded a secondary rate limit. Please wait a few minutes before you try again.",
  "documentation_url": "https://docs.github.com/rest/overview/resources-in-the-rest-api#secondary-rate-limits"
}
```

`crates/feedback/tests/fixtures/github/404-not-found.json`:

```json
{
  "message": "Not Found",
  "documentation_url": "https://docs.github.com/rest/reference/issues#create-an-issue"
}
```

- [ ] **Step 2: Write the failing tests**

Create `crates/feedback/src/github.rs`:

```rust
//! GitHub Issues API client.
//!
//! `GithubClient` is a trait so the role can be tested with a mock.
//! `ReqwestGithubClient` is the production impl.

use willow_common::FeedbackErrReason;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/github")
            .join(name);
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
        serde_json::from_str(&raw).unwrap()
    }

    #[test]
    fn parse_201_extracts_html_url() {
        let body = fixture("201-created.json");
        let url = parse_201_html_url(&body).unwrap();
        assert_eq!(url, "https://github.com/intendednull/willow/issues/42");
    }

    #[test]
    fn parse_422_returns_invalid_input() {
        let body = fixture("422-validation.json");
        match map_failure(422, None, Some(&body)) {
            FeedbackErrReason::GithubFailure { status, message } => {
                assert_eq!(status, 422);
                assert_eq!(message.as_deref(), Some("Validation Failed"));
            }
            other => panic!("expected GithubFailure, got {other:?}"),
        }
    }

    #[test]
    fn parse_401_returns_unconfigured() {
        let body = fixture("401-unauthorized.json");
        // 401 is the role's signal to transition to Unconfigured; the
        // GithubClient layer surfaces it as GithubFailure { status: 401 }
        // and the role decides what to do with it.
        match map_failure(401, None, Some(&body)) {
            FeedbackErrReason::GithubFailure { status, .. } => assert_eq!(status, 401),
            other => panic!("expected GithubFailure, got {other:?}"),
        }
    }

    #[test]
    fn parse_403_with_zero_remaining_returns_rate_limited() {
        // Headers carry the secondary-rate-limit signal.
        let body = fixture("403-secondary-rate-limit.json");
        let headers = vec![
            ("x-ratelimit-remaining".to_string(), "0".to_string()),
            ("retry-after".to_string(), "60".to_string()),
        ];
        match map_failure(403, Some(&headers), Some(&body)) {
            FeedbackErrReason::RateLimited { retry_after_ms } => {
                assert_eq!(retry_after_ms, 60_000);
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[test]
    fn parse_403_without_secondary_signal_is_just_failure() {
        // A 403 *without* x-ratelimit-remaining: 0 is a generic 403,
        // not the secondary-rate-limit signal.
        let body = fixture("403-secondary-rate-limit.json");
        let headers = vec![("x-ratelimit-remaining".to_string(), "47".to_string())];
        match map_failure(403, Some(&headers), Some(&body)) {
            FeedbackErrReason::GithubFailure { status, .. } => assert_eq!(status, 403),
            other => panic!("expected GithubFailure, got {other:?}"),
        }
    }

    #[test]
    fn message_truncation_caps_at_200_chars() {
        let big = "x".repeat(500);
        let body = serde_json::json!({ "message": big });
        match map_failure(500, None, Some(&body)) {
            FeedbackErrReason::GithubFailure { message: Some(m), .. } => {
                assert_eq!(m.chars().count(), 200);
            }
            other => panic!("expected GithubFailure, got {other:?}"),
        }
    }
}
```

- [ ] **Step 3: Run tests, expect compile failure**

Run: `cargo test -p willow-feedback --lib github::tests`
Expected: COMPILE FAIL — `parse_201_html_url`, `map_failure` undefined.

- [ ] **Step 4: Implement parsing helpers + the trait + reqwest impl**

Append to `crates/feedback/src/github.rs` *above* the test module:

```rust
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use secrecy::ExposeSecret;
use secrecy::SecretString;
use serde::Serialize;

/// Result of a `create_issue` call. Successful path carries the GitHub
/// `html_url` (the user-facing issue URL); failure carries a typed
/// reason matching `FeedbackErrReason` plus `secondary_rate_limit`
/// flag so the role can trip the worker-wide cooldown.
pub enum CreateIssueOutcome {
    Created { html_url: String },
    Failed { reason: FeedbackErrReason },
}

#[async_trait]
pub trait GithubClient: Send + Sync {
    /// Create an issue on the configured `owner/repo`. Returns the
    /// resulting issue's `html_url` on success, or a typed error.
    async fn create_issue(&self, body: IssueBody<'_>) -> CreateIssueOutcome;
}

#[derive(Serialize)]
pub struct IssueBody<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub labels: &'a [&'a str],
}

/// Production GitHub client.
pub struct ReqwestGithubClient {
    client: reqwest::Client,
    repo: String, // "owner/repo"
    token: SecretString,
}

impl ReqwestGithubClient {
    pub fn new(repo: String, token: SecretString) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("willow-feedback/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .build()?;
        Ok(Self { client, repo, token })
    }
}

#[async_trait]
impl GithubClient for ReqwestGithubClient {
    async fn create_issue(&self, body: IssueBody<'_>) -> CreateIssueOutcome {
        let url = format!("https://api.github.com/repos/{}/issues", self.repo);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(self.token.expose_secret())
            .header("Accept", "application/vnd.github+json")
            .json(&body)
            .send()
            .await;

        let resp = match resp {
            Ok(r) => r,
            Err(e) if e.is_timeout() => {
                return CreateIssueOutcome::Failed {
                    reason: FeedbackErrReason::GithubFailure {
                        status: 0,
                        message: Some("timeout".to_string()),
                    },
                };
            }
            Err(e) => {
                return CreateIssueOutcome::Failed {
                    reason: FeedbackErrReason::GithubFailure {
                        status: 0,
                        message: Some(format!("transport: {}", truncate(&e.to_string(), 200))),
                    },
                };
            }
        };

        let status = resp.status().as_u16();
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_lowercase(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let body_json: Option<serde_json::Value> = resp.json().await.ok();

        if status == 201 {
            if let Some(b) = body_json.as_ref() {
                if let Some(url) = parse_201_html_url(b) {
                    return CreateIssueOutcome::Created { html_url: url };
                }
            }
            return CreateIssueOutcome::Failed {
                reason: FeedbackErrReason::GithubFailure {
                    status,
                    message: Some("missing html_url".to_string()),
                },
            };
        }

        CreateIssueOutcome::Failed {
            reason: map_failure(status, Some(&headers), body_json.as_ref()),
        }
    }
}

/// Extract the `html_url` from a 201 response body.
pub fn parse_201_html_url(body: &serde_json::Value) -> Option<String> {
    body.get("html_url")?.as_str().map(|s| s.to_string())
}

/// Map a non-201 GitHub response to a `FeedbackErrReason`.
///
/// Special cases:
/// - 403 with `x-ratelimit-remaining: 0` → `RateLimited` with
///   `retry-after` (default 60s if header missing).
/// - All other non-2xx → `GithubFailure { status, message }` with
///   the message truncated to 200 chars.
pub fn map_failure(
    status: u16,
    headers: Option<&[(String, String)]>,
    body: Option<&serde_json::Value>,
) -> FeedbackErrReason {
    if status == 403 {
        if let Some(headers) = headers {
            let remaining_zero = headers
                .iter()
                .any(|(k, v)| k == "x-ratelimit-remaining" && v == "0");
            if remaining_zero {
                let retry_secs: u64 = headers
                    .iter()
                    .find(|(k, _)| k == "retry-after")
                    .and_then(|(_, v)| v.parse().ok())
                    .unwrap_or(60);
                return FeedbackErrReason::RateLimited {
                    retry_after_ms: retry_secs * 1000,
                };
            }
        }
    }
    let message = body
        .and_then(|b| b.get("message").and_then(|m| m.as_str()))
        .map(|s| truncate(s, 200));
    FeedbackErrReason::GithubFailure { status, message }
}

fn truncate(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

// Helper to make `anyhow!` resolve in the file even though we don't
// use it directly above (keeps the import non-noisy if `parse_*` is
// extended later).
#[allow(dead_code)]
fn _unused_anyhow() -> Result<()> {
    Err(anyhow!("placeholder"))
}
```

- [ ] **Step 5: Wire into `lib.rs`**

```rust
pub mod sanitize;
pub mod wordlist;
pub mod handle;
pub mod ratelimit;
pub mod salt;
pub mod throttle;
pub mod github;
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p willow-feedback --lib github::tests`
Expected: 6 tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/feedback/
git commit -m "feat(feedback): GitHub client trait + reqwest impl + fixtures"
```

### Task 2.8: `FeedbackRole` (the integration piece) — `role.rs`

**Files:**
- Create: `crates/feedback/src/role.rs`
- Modify: `crates/feedback/src/lib.rs`

This is where everything composes. The role:

1. Validates request shape (length caps).
2. Checks idempotency cache; if hit, returns cached URL.
3. Checks rate limits.
4. Computes salted handle.
5. Wraps body, sanitizes title, builds metadata block.
6. Calls `GithubClient::create_issue`.
7. Updates state machine (Unconfigured/cooldown transitions on 401/403).
8. Updates idempotency cache + counters.
9. Emits one structured log line.

Tests inject a `MockGithubClient` so no live HTTP is hit.

- [ ] **Step 1: Write the failing test scaffold**

Create `crates/feedback/src/role.rs`:

```rust
//! `FeedbackRole` — the integration glue. Implements `WorkerRole`.

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use secrecy::SecretString;
use tokio::sync::Mutex;
use willow_common::{
    FeedbackCategory, FeedbackDiagnostics, FeedbackErrReason, WorkerRequest, WorkerResponse,
    WorkerRole, WorkerRoleInfo,
};
use willow_identity::EndpointId;
use willow_state::{Event, HeadsSummary};

use crate::github::{CreateIssueOutcome, GithubClient, IssueBody};
use crate::handle::compute_handle;
use crate::ratelimit::{Clock, RateLimited, RateLimiter, SystemClock};
use crate::sanitize::{sanitize_title, wrap_body_fenced};

#[cfg(test)]
mod tests;
```

- [ ] **Step 2: Create the test module skeleton with the failing tests**

Create `crates/feedback/src/role/tests.rs` (Rust module-style — alternative is to keep tests in `role.rs`; we split into a sibling file because the test set is large). Actually, since `mod tests` is declared inline above, let's keep it as a test child module file. Use `#[path]` if needed.

Replace the `#[cfg(test)] mod tests;` line with an inline `#[cfg(test)] mod tests {` block. Append at the bottom of `role.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use willow_identity::Identity;

    /// Test mock that returns a scripted response.
    struct MockGithub {
        outcomes: Mutex<VecDeque<CreateIssueOutcome>>,
        calls: AtomicUsize,
    }
    impl MockGithub {
        fn new(outcomes: Vec<CreateIssueOutcome>) -> Arc<Self> {
            Arc::new(Self {
                outcomes: Mutex::new(outcomes.into()),
                calls: AtomicUsize::new(0),
            })
        }
        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }
    #[async_trait]
    impl GithubClient for MockGithub {
        async fn create_issue(&self, _body: IssueBody<'_>) -> CreateIssueOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.outcomes
                .lock()
                .await
                .pop_front()
                .unwrap_or(CreateIssueOutcome::Failed {
                    reason: FeedbackErrReason::Unconfigured,
                })
        }
    }

    fn ok_outcome(url: &str) -> CreateIssueOutcome {
        CreateIssueOutcome::Created {
            html_url: url.to_string(),
        }
    }

    fn req(dedup: u8, body: &str) -> WorkerRequest {
        WorkerRequest::Feedback {
            dedup_id: [dedup; 16],
            title: "title".to_string(),
            category: FeedbackCategory::Bug,
            body: body.to_string(),
            diagnostics: None,
        }
    }

    fn role_with(github: Arc<dyn GithubClient>) -> FeedbackRole {
        FeedbackRole::new_for_test(FeedbackRoleConfig {
            github,
            salt: [0u8; 32],
            per_peer_per_hour: 5,
            global_per_hour: 50,
            repo: "intendednull/willow".to_string(),
        })
    }

    fn signer() -> EndpointId {
        Identity::generate().endpoint_id()
    }

    #[tokio::test]
    async fn happy_path_returns_feedback_ok() {
        let mock = MockGithub::new(vec![ok_outcome("https://github.com/x/y/issues/1")]);
        let mut role = role_with(mock.clone());
        let resp = role.handle_request(signer(), req(1, "hi")).await;
        assert!(matches!(
            resp,
            WorkerResponse::FeedbackOk { issue_url } if issue_url == "https://github.com/x/y/issues/1"
        ));
        assert_eq!(mock.call_count(), 1);
    }

    #[tokio::test]
    async fn validates_title_length() {
        let mock = MockGithub::new(vec![]);
        let mut role = role_with(mock.clone());
        let req = WorkerRequest::Feedback {
            dedup_id: [0u8; 16],
            title: "x".repeat(201),
            category: FeedbackCategory::Bug,
            body: "ok".to_string(),
            diagnostics: None,
        };
        match role.handle_request(signer(), req).await {
            WorkerResponse::FeedbackErr {
                reason: FeedbackErrReason::InvalidInput { field, .. },
            } => assert_eq!(field, "title"),
            other => panic!("expected InvalidInput(title), got {other:?}"),
        }
        assert_eq!(mock.call_count(), 0);
    }

    #[tokio::test]
    async fn validates_body_length() {
        let mock = MockGithub::new(vec![]);
        let mut role = role_with(mock.clone());
        match role
            .handle_request(signer(), req(0, &"x".repeat(8001)))
            .await
        {
            WorkerResponse::FeedbackErr {
                reason: FeedbackErrReason::InvalidInput { field, .. },
            } => assert_eq!(field, "body"),
            other => panic!("expected InvalidInput(body), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn validates_other_detail_length() {
        let mock = MockGithub::new(vec![]);
        let mut role = role_with(mock.clone());
        let req = WorkerRequest::Feedback {
            dedup_id: [0u8; 16],
            title: "t".to_string(),
            category: FeedbackCategory::Other {
                detail: Some("x".repeat(61)),
            },
            body: "ok".to_string(),
            diagnostics: None,
        };
        match role.handle_request(signer(), req).await {
            WorkerResponse::FeedbackErr {
                reason: FeedbackErrReason::InvalidInput { field, .. },
            } => assert_eq!(field, "category.detail"),
            other => panic!("expected InvalidInput(category.detail), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unconfigured_when_no_github_client() {
        let mut role = FeedbackRole::new_unconfigured(FeedbackRoleConfig {
            github: MockGithub::new(vec![]),
            salt: [0u8; 32],
            per_peer_per_hour: 5,
            global_per_hour: 50,
            repo: "intendednull/willow".to_string(),
        });
        assert!(matches!(
            role.handle_request(signer(), req(0, "hi")).await,
            WorkerResponse::FeedbackErr {
                reason: FeedbackErrReason::Unconfigured
            }
        ));
    }

    #[tokio::test]
    async fn per_peer_rate_limit_kicks_in() {
        let outs = (0..5).map(|i| ok_outcome(&format!("https://x/{i}"))).collect();
        let mock = MockGithub::new(outs);
        let mut role = role_with(mock.clone());
        let p = signer();
        for i in 0..5 {
            let r = role.handle_request(p, req(i as u8, "ok")).await;
            assert!(matches!(r, WorkerResponse::FeedbackOk { .. }));
        }
        match role.handle_request(p, req(255, "ok")).await {
            WorkerResponse::FeedbackErr {
                reason: FeedbackErrReason::RateLimited { .. },
            } => {}
            other => panic!("expected RateLimited, got {other:?}"),
        }
        assert_eq!(mock.call_count(), 5);
    }

    #[tokio::test]
    async fn idempotency_cache_returns_cached_url() {
        let mock = MockGithub::new(vec![ok_outcome("https://github.com/x/y/issues/9")]);
        let mut role = role_with(mock.clone());
        let p = signer();
        let r1 = role.handle_request(p, req(7, "hi")).await;
        let r2 = role.handle_request(p, req(7, "hi")).await;
        assert_eq!(format!("{r1:?}"), format!("{r2:?}"));
        assert_eq!(mock.call_count(), 1);
    }

    #[tokio::test]
    async fn distinct_signers_with_same_dedup_get_distinct_urls() {
        let mock = MockGithub::new(vec![
            ok_outcome("https://github.com/x/y/issues/1"),
            ok_outcome("https://github.com/x/y/issues/2"),
        ]);
        let mut role = role_with(mock.clone());
        let r1 = role.handle_request(signer(), req(7, "hi")).await;
        let r2 = role.handle_request(signer(), req(7, "hi")).await;
        match (&r1, &r2) {
            (
                WorkerResponse::FeedbackOk { issue_url: u1 },
                WorkerResponse::FeedbackOk { issue_url: u2 },
            ) => assert_ne!(u1, u2),
            _ => panic!("expected two FeedbackOk, got {r1:?} / {r2:?}"),
        }
        assert_eq!(mock.call_count(), 2);
    }

    #[tokio::test]
    async fn fourzeroone_transitions_to_unconfigured() {
        let mock = MockGithub::new(vec![CreateIssueOutcome::Failed {
            reason: FeedbackErrReason::GithubFailure {
                status: 401,
                message: Some("Bad credentials".to_string()),
            },
        }]);
        let mut role = role_with(mock.clone());
        let p = signer();
        // First call surfaces the 401.
        let r1 = role.handle_request(p, req(0, "hi")).await;
        assert!(matches!(
            r1,
            WorkerResponse::FeedbackErr {
                reason: FeedbackErrReason::Unconfigured
            }
        ));
        // Subsequent calls are also Unconfigured without contacting the mock.
        let r2 = role.handle_request(p, req(1, "hi")).await;
        assert!(matches!(
            r2,
            WorkerResponse::FeedbackErr {
                reason: FeedbackErrReason::Unconfigured
            }
        ));
        assert_eq!(mock.call_count(), 1);
    }

    #[tokio::test]
    async fn fourohthree_secondary_trips_cooldown() {
        let mock = MockGithub::new(vec![
            CreateIssueOutcome::Failed {
                reason: FeedbackErrReason::RateLimited {
                    retry_after_ms: 60_000,
                },
            },
            ok_outcome("https://x/1"), // wouldn't be called during cooldown
        ]);
        let mut role = role_with(mock.clone());
        let r1 = role.handle_request(signer(), req(0, "hi")).await;
        assert!(matches!(
            r1,
            WorkerResponse::FeedbackErr {
                reason: FeedbackErrReason::RateLimited { .. }
            }
        ));
        // Second call returns RateLimited from the cooldown without contacting the mock.
        let r2 = role.handle_request(signer(), req(1, "hi")).await;
        assert!(matches!(
            r2,
            WorkerResponse::FeedbackErr {
                reason: FeedbackErrReason::RateLimited { .. }
            }
        ));
        assert_eq!(mock.call_count(), 1);
    }

    #[tokio::test]
    async fn role_info_reports_feedback_with_counters() {
        let mock = MockGithub::new(vec![ok_outcome("https://x/1")]);
        let mut role = role_with(mock);
        role.handle_request(signer(), req(0, "hi")).await;
        match role.role_info() {
            WorkerRoleInfo::Feedback {
                reports_accepted,
                reports_rejected,
                ..
            } => {
                assert_eq!(reports_accepted, 1);
                assert_eq!(reports_rejected, 0);
            }
            other => panic!("expected Feedback, got {other:?}"),
        }
    }
}
```

- [ ] **Step 3: Implement `FeedbackRole`**

Append to `crates/feedback/src/role.rs` *between* the `use` block and the `#[cfg(test)] mod tests` block:

```rust
const TITLE_MAX: usize = 200;
const BODY_MAX: usize = 8000;
const DETAIL_MAX: usize = 60;
const DEDUP_CACHE_CAPACITY: usize = 4096;

pub struct FeedbackRoleConfig {
    pub github: Arc<dyn GithubClient>,
    pub salt: [u8; 32],
    pub per_peer_per_hour: u32,
    pub global_per_hour: u32,
    pub repo: String,
}

/// Internal state machine for the role's GitHub-side health.
enum GithubState {
    Configured,
    /// 401 transitions here permanently for the rest of the process.
    Unconfigured,
    /// 403 secondary-rate-limit; while in cooldown, all requests
    /// reply RateLimited with this `retry_after_ms` (decremented by
    /// elapsed time on each request).
    Cooldown {
        until: std::time::Instant,
        retry_after_ms: u64,
    },
}

pub struct FeedbackRole {
    github: Arc<dyn GithubClient>,
    salt: [u8; 32],
    repo: String,
    state: GithubState,
    rate_limiter: RateLimiter,
    clock: SystemClock,
    /// LRU-ish: deque + companion vec. Capacity 4096 entries.
    /// Each entry: (signer, dedup_id, issue_url).
    dedup_cache: VecDeque<((EndpointId, [u8; 16]), String)>,
    reports_accepted: u64,
    reports_rejected: u64,
}

impl FeedbackRole {
    pub fn new(config: FeedbackRoleConfig) -> Self {
        let mut clock = SystemClock;
        let rate_limiter = RateLimiter::new(
            config.per_peer_per_hour,
            config.global_per_hour,
            &mut clock,
        );
        Self {
            github: config.github,
            salt: config.salt,
            repo: config.repo,
            state: GithubState::Configured,
            rate_limiter,
            clock,
            dedup_cache: VecDeque::with_capacity(DEDUP_CACHE_CAPACITY),
            reports_accepted: 0,
            reports_rejected: 0,
        }
    }

    /// Construct a role that's permanently `Unconfigured` (used by
    /// the dev stack when no GITHUB_TOKEN is set).
    pub fn new_unconfigured(config: FeedbackRoleConfig) -> Self {
        let mut role = Self::new(config);
        role.state = GithubState::Unconfigured;
        role
    }

    #[cfg(test)]
    pub fn new_for_test(config: FeedbackRoleConfig) -> Self {
        Self::new(config)
    }

    fn validate_request(req: &WorkerRequest) -> Result<(), FeedbackErrReason> {
        let WorkerRequest::Feedback {
            title,
            body,
            category,
            ..
        } = req
        else {
            return Err(FeedbackErrReason::InvalidInput {
                field: "request".to_string(),
                message: "not a feedback request".to_string(),
            });
        };
        if title.is_empty() || title.len() > TITLE_MAX {
            return Err(FeedbackErrReason::InvalidInput {
                field: "title".to_string(),
                message: format!("must be 1..={TITLE_MAX} bytes"),
            });
        }
        if body.is_empty() || body.len() > BODY_MAX {
            return Err(FeedbackErrReason::InvalidInput {
                field: "body".to_string(),
                message: format!("must be 1..={BODY_MAX} bytes"),
            });
        }
        if let FeedbackCategory::Other {
            detail: Some(detail),
        } = category
        {
            if detail.len() > DETAIL_MAX {
                return Err(FeedbackErrReason::InvalidInput {
                    field: "category.detail".to_string(),
                    message: format!("must be 0..={DETAIL_MAX} bytes"),
                });
            }
        }
        Ok(())
    }

    fn lookup_cache(&self, key: &(EndpointId, [u8; 16])) -> Option<String> {
        self.dedup_cache
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }

    fn record_cache(&mut self, key: (EndpointId, [u8; 16]), url: String) {
        if self.dedup_cache.len() >= DEDUP_CACHE_CAPACITY {
            self.dedup_cache.pop_front();
        }
        self.dedup_cache.push_back((key, url));
    }

    fn assemble_issue<'a>(
        &self,
        signer: &EndpointId,
        title: &'a str,
        category: &FeedbackCategory,
        body: &str,
        diagnostics: Option<&FeedbackDiagnostics>,
    ) -> (String, String, Vec<&'static str>) {
        let handle = compute_handle(&self.salt, signer);
        let title_clean = sanitize_title(title);
        let prefix = match category {
            FeedbackCategory::Bug => "[Bug] ".to_string(),
            FeedbackCategory::Suggestion => "[Suggestion] ".to_string(),
            FeedbackCategory::Other { detail: None } => "[Other] ".to_string(),
            FeedbackCategory::Other { detail: Some(d) } => {
                format!("[Other:{}] ", sanitize_title(d))
            }
        };
        let mut full_title = format!("{prefix}{title_clean}");
        if full_title.chars().count() > 256 {
            // Truncate to 255 chars + ellipsis (256 total).
            let mut t: String = full_title.chars().take(255).collect();
            t.push('…');
            full_title = t;
        }

        let category_str = match category {
            FeedbackCategory::Bug => "Bug".to_string(),
            FeedbackCategory::Suggestion => "Suggestion".to_string(),
            FeedbackCategory::Other { detail: None } => "Other".to_string(),
            FeedbackCategory::Other { detail: Some(d) } => format!("Other ({d})"),
        };
        let mut header = format!(
            "**Reporter (salted hash):** `{handle}`\n**Category:** {category_str}\n",
        );
        if let Some(d) = diagnostics {
            header.push_str(&format!("**App version:** {}\n", d.app_version));
            if let Some(b) = &d.build_hash {
                header.push_str(&format!("**Build:** {b}\n"));
            }
            if let Some(l) = &d.locale {
                header.push_str(&format!("**Locale:** {l}\n"));
            }
            header.push_str(&format!("**Client:** {:?}\n", d.client));
        } else {
            header.push_str("(diagnostics not provided)\n");
        }
        let preamble = "\n> Submitted via willow-feedback. The reporter's body is rendered \n> verbatim in the fenced block below; @mentions, links, and image\n> syntax inside it are **not** processed by GitHub.\n\n";
        let body_block = wrap_body_fenced(body);
        let full_body = format!("{header}{preamble}{body_block}");

        let labels: Vec<&'static str> = match category {
            FeedbackCategory::Bug => vec!["feedback", "feedback:bug", "feedback:triage"],
            FeedbackCategory::Suggestion => {
                vec!["feedback", "feedback:suggestion", "feedback:triage"]
            }
            FeedbackCategory::Other { .. } => vec!["feedback", "feedback:other", "feedback:triage"],
        };
        (full_title, full_body, labels)
    }
}

#[async_trait]
impl WorkerRole for FeedbackRole {
    fn role_info(&self) -> WorkerRoleInfo {
        WorkerRoleInfo::Feedback {
            reports_accepted: self.reports_accepted,
            reports_rejected: self.reports_rejected,
            currently_rate_limited: self.rate_limiter.currently_rate_limited(&self.clock),
            global_rate_limited: self.rate_limiter.global_is_throttled(&self.clock),
        }
    }

    fn on_event(&mut self, _event: &Event) {
        // Feedback role doesn't track DAG events.
    }

    async fn handle_request(
        &mut self,
        signer: EndpointId,
        req: WorkerRequest,
    ) -> WorkerResponse {
        // 1. Unconfigured short-circuit.
        if matches!(self.state, GithubState::Unconfigured) {
            self.reports_rejected += 1;
            return WorkerResponse::FeedbackErr {
                reason: FeedbackErrReason::Unconfigured,
            };
        }

        // 2. Cooldown short-circuit.
        if let GithubState::Cooldown { until, retry_after_ms } = self.state {
            if std::time::Instant::now() < until {
                self.reports_rejected += 1;
                let remaining = until.saturating_duration_since(std::time::Instant::now());
                return WorkerResponse::FeedbackErr {
                    reason: FeedbackErrReason::RateLimited {
                        retry_after_ms: remaining.as_millis() as u64,
                    },
                };
            } else {
                self.state = GithubState::Configured;
                let _ = retry_after_ms; // silence unused warning
            }
        }

        // 3. Validate shape.
        if let Err(reason) = Self::validate_request(&req) {
            self.reports_rejected += 1;
            return WorkerResponse::FeedbackErr { reason };
        }
        let WorkerRequest::Feedback {
            dedup_id,
            title,
            category,
            body,
            diagnostics,
        } = req
        else {
            unreachable!("validated above");
        };

        // 4. Idempotency cache.
        let cache_key = (signer, dedup_id);
        if let Some(url) = self.lookup_cache(&cache_key) {
            return WorkerResponse::FeedbackOk { issue_url: url };
        }

        // 5. Rate limit.
        match self.rate_limiter.try_take(&signer, &mut self.clock) {
            Ok(()) => {}
            Err(RateLimited::PerPeer { retry_after_ms })
            | Err(RateLimited::Global { retry_after_ms }) => {
                self.reports_rejected += 1;
                return WorkerResponse::FeedbackErr {
                    reason: FeedbackErrReason::RateLimited { retry_after_ms },
                };
            }
        }

        // 6. Assemble + post.
        let (full_title, full_body, labels) =
            self.assemble_issue(&signer, &title, &category, &body, diagnostics.as_ref());
        let outcome = self
            .github
            .create_issue(IssueBody {
                title: &full_title,
                body: &full_body,
                labels: &labels,
            })
            .await;

        match outcome {
            CreateIssueOutcome::Created { html_url } => {
                self.record_cache(cache_key, html_url.clone());
                self.reports_accepted += 1;
                WorkerResponse::FeedbackOk {
                    issue_url: html_url,
                }
            }
            CreateIssueOutcome::Failed {
                reason: FeedbackErrReason::GithubFailure { status: 401, .. },
            } => {
                self.state = GithubState::Unconfigured;
                self.reports_rejected += 1;
                WorkerResponse::FeedbackErr {
                    reason: FeedbackErrReason::Unconfigured,
                }
            }
            CreateIssueOutcome::Failed {
                reason: FeedbackErrReason::RateLimited { retry_after_ms },
            } => {
                self.state = GithubState::Cooldown {
                    until: std::time::Instant::now() + std::time::Duration::from_millis(retry_after_ms),
                    retry_after_ms,
                };
                self.reports_rejected += 1;
                WorkerResponse::FeedbackErr {
                    reason: FeedbackErrReason::RateLimited { retry_after_ms },
                }
            }
            CreateIssueOutcome::Failed { reason } => {
                self.reports_rejected += 1;
                WorkerResponse::FeedbackErr { reason }
            }
        }
    }

    fn heads_summaries(&self) -> Vec<(String, HeadsSummary)> {
        vec![]
    }
}
```

- [ ] **Step 4: Wire into `lib.rs`**

```rust
pub mod sanitize;
pub mod wordlist;
pub mod handle;
pub mod ratelimit;
pub mod salt;
pub mod throttle;
pub mod github;
pub mod role;
```

- [ ] **Step 5: Run all tests**

Run: `cargo test -p willow-feedback --lib`
Expected: every module's tests pass — sanitize, wordlist, handle, ratelimit, salt, throttle, github, role.

- [ ] **Step 6: Commit**

```bash
git add crates/feedback/
git commit -m "feat(feedback): FeedbackRole — sanitization, rate limits, idempotency, cooldown"
```

### Task 2.9: `main.rs` — CLI, repo validation, IrohNetwork, runtime

**Files:**
- Modify: `crates/feedback/src/main.rs`

Closely mirrors `crates/storage/src/main.rs`'s shape, with extra steps for salt and the `--generate-salt` CLI flag.

- [ ] **Step 1: Replace the placeholder `main.rs` with the real binary**

Replace `crates/feedback/src/main.rs`:

```rust
//! Willow Feedback Node — proxies user feedback to GitHub issues.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use regex::Regex;
use secrecy::SecretString;
use willow_feedback::github::{GithubClient, ReqwestGithubClient};
use willow_feedback::role::{FeedbackRole, FeedbackRoleConfig};
use willow_feedback::salt::{generate_salt, load_salt};
use willow_feedback::throttle::enforce_throttle;

#[derive(Parser)]
#[command(name = "willow-feedback", about = "Willow feedback worker node")]
struct Cli {
    /// Path to the Ed25519 identity keypair file.
    #[arg(long, default_value = "/etc/willow/feedback.key")]
    identity_path: String,

    /// Iroh relay URL to connect through.
    #[arg(long)]
    relay_url: Option<String>,

    /// GitHub PAT (`Issues: write` scope, fine-grained, target repo only).
    /// Can also be supplied via the `GITHUB_TOKEN` env var.
    #[arg(long, env = "GITHUB_TOKEN")]
    github_token: Option<String>,

    /// `owner/repo` to file issues against.
    #[arg(long, env = "FEEDBACK_REPO", default_value = "intendednull/willow")]
    github_repo: String,

    /// Per-peer rate limit (requests / hour).
    #[arg(long, default_value = "5")]
    rate_limit_per_hour: u32,

    /// Worker-wide rate limit (requests / hour).
    #[arg(long, default_value = "50")]
    global_rate_limit_per_hour: u32,

    /// Path to the 32-byte reporter-handle salt file.
    #[arg(long, default_value = "/etc/willow/feedback-salt")]
    reporter_salt_file: PathBuf,

    /// Generate a new identity at `--identity-path` and exit.
    #[arg(long)]
    generate_identity: bool,

    /// Write 32 random bytes to `--reporter-salt-file` if missing and exit.
    #[arg(long)]
    generate_salt: bool,

    /// Print the bech32 peer ID for `--identity-path` and exit.
    #[arg(long)]
    print_peer_id: bool,
}

fn validate_repo(repo: &str) -> Result<()> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$").unwrap());
    if !re.is_match(repo) {
        return Err(anyhow!(
            "invalid FEEDBACK_REPO {:?}: must match owner/repo (alphanumeric, dot, underscore, hyphen)",
            repo
        ));
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    if cli.generate_identity {
        willow_worker::identity::generate_identity(&cli.identity_path)?;
        tracing::info!("identity generated at {}", cli.identity_path);
        return Ok(());
    }

    if cli.generate_salt {
        generate_salt(&cli.reporter_salt_file)?;
        tracing::info!("salt generated at {}", cli.reporter_salt_file.display());
        return Ok(());
    }

    if cli.print_peer_id {
        return willow_worker::identity::print_peer_id(&cli.identity_path);
    }

    validate_repo(&cli.github_repo)?;

    // Enforce 15-second startup throttle.
    let identity_dir = std::path::Path::new(&cli.identity_path)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/etc/willow"))
        .to_path_buf();
    std::fs::create_dir_all(&identity_dir).ok();
    let gate = identity_dir.join(".feedback-last-boot");
    let throttled = enforce_throttle(&gate, Duration::from_secs(15))
        .context("startup throttle")?;
    if !throttled.is_zero() {
        tracing::info!("startup throttle slept for {:?}", throttled);
    }

    // Load identity.
    let identity = willow_worker::identity::load_or_generate(&cli.identity_path)?;

    // Load salt (auto-generate if missing — entrypoint may not have run --generate-salt).
    if !cli.reporter_salt_file.exists() {
        generate_salt(&cli.reporter_salt_file)
            .with_context(|| format!("generating salt at {}", cli.reporter_salt_file.display()))?;
        tracing::info!("salt generated at {}", cli.reporter_salt_file.display());
    }
    let salt = load_salt(&cli.reporter_salt_file)?;

    // Resolve relay URL.
    let relay_url = cli.relay_url.as_deref().map(|url| {
        url.parse::<willow_network::iroh::RelayUrl>()
            .expect("invalid relay URL")
    });

    let iroh_config = willow_network::iroh::Config {
        secret_key: identity.secret_key().clone(),
        relay_url,
        bootstrap_peers: vec![],
        mdns: false,
    };
    let network = willow_network::iroh::IrohNetwork::new(iroh_config).await?;

    // Build the role. If no GITHUB_TOKEN, run permanently Unconfigured
    // (dev path: every UI flow exercised, no GitHub calls).
    let role = match cli.github_token.as_deref() {
        Some(token) if !token.is_empty() => {
            let client: Arc<dyn GithubClient> = Arc::new(ReqwestGithubClient::new(
                cli.github_repo.clone(),
                SecretString::from(token.to_string()),
            )?);
            FeedbackRole::new(FeedbackRoleConfig {
                github: client,
                salt,
                per_peer_per_hour: cli.rate_limit_per_hour,
                global_per_hour: cli.global_rate_limit_per_hour,
                repo: cli.github_repo.clone(),
            })
        }
        _ => {
            tracing::warn!("no GITHUB_TOKEN set; running permanently Unconfigured");
            FeedbackRole::new_unconfigured(FeedbackRoleConfig {
                github: Arc::new(NullGithubClient) as Arc<dyn GithubClient>,
                salt,
                per_peer_per_hour: cli.rate_limit_per_hour,
                global_per_hour: cli.global_rate_limit_per_hour,
                repo: cli.github_repo.clone(),
            })
        }
    };

    let config = willow_worker::WorkerConfig {
        identity_path: cli.identity_path,
        relay_url: cli.relay_url,
        sync_interval_secs: 60,
        allocation: willow_worker::AllocationStrategy::Global,
    };

    willow_worker::runtime::run(Box::new(role), config, network).await
}

/// Stub GithubClient that's never called (used when running Unconfigured).
struct NullGithubClient;
#[async_trait::async_trait]
impl GithubClient for NullGithubClient {
    async fn create_issue(
        &self,
        _body: willow_feedback::github::IssueBody<'_>,
    ) -> willow_feedback::github::CreateIssueOutcome {
        willow_feedback::github::CreateIssueOutcome::Failed {
            reason: willow_common::FeedbackErrReason::Unconfigured,
        }
    }
}
```

- [ ] **Step 2: Add `willow-common` to `dev-dependencies` if needed**

`crates/feedback/Cargo.toml` already lists `willow-common`. The `NullGithubClient` impl uses `willow_common::FeedbackErrReason`; that's already accessible.

- [ ] **Step 3: Verify the binary compiles**

Run: `cargo build -p willow-feedback`
Expected: pass.

- [ ] **Step 4: Sanity test the CLI subcommands**

Run: `cargo run -p willow-feedback -- --help 2>&1 | head -30`
Expected: prints help text including `--generate-identity`, `--generate-salt`, `--print-peer-id`, `--github-token`, `--github-repo`, `--reporter-salt-file`.

Run: `cargo run -p willow-feedback -- --github-repo "javascript:alert(1)" --github-token x 2>&1 | head -5`
Expected: errors with `invalid FEEDBACK_REPO`. Confirms repo validation.

- [ ] **Step 5: Run the full crate test suite + workspace check**

Run: `cargo test -p willow-feedback`
Expected: every test passes.

Run: `just check-native 2>&1 | tail -10`
Expected: workspace-wide cargo check passes.

- [ ] **Step 6: Commit**

```bash
git add crates/feedback/src/main.rs
git commit -m "feat(feedback): main binary — CLI, repo validation, runtime bring-up"
```

---

## Phase 3: `willow-client` API

**Why now:** the worker is reachable via the existing gossip request/response pathway. We need a typed client API that constructs `WorkerRequest::Feedback` and parses `WorkerResponse::FeedbackOk/FeedbackErr`.

### Task 3.1: Add `feedback_worker` to `ClientConfig` + `FeedbackError` enum

**Files:**
- Modify: `crates/client/src/lib.rs` (around line 189 — `ClientConfig`)
- Create: `crates/client/src/feedback.rs`
- Modify: `crates/client/Cargo.toml` (add `url`, `rand`)

- [ ] **Step 1: Add the config field**

Edit `crates/client/src/lib.rs` — extend `ClientConfig` (currently around line 189) and the `Default` impl:

```rust
pub struct ClientConfig {
    pub relay_addr: Option<String>,
    pub display_name: Option<String>,
    pub persistence: bool,
    pub bootstrap_peers: Vec<willow_identity::EndpointId>,
    /// Project-run feedback worker peer ID. If `None`, the
    /// in-app feedback form is disabled and renders a
    /// "Feedback is not configured for this build" state.
    pub feedback_worker: Option<willow_identity::EndpointId>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            relay_addr: None,
            display_name: None,
            persistence: true,
            bootstrap_peers: vec![],
            feedback_worker: None,
        }
    }
}
```

- [ ] **Step 2: Add `url` and `rand` to client deps**

Add to `crates/client/Cargo.toml` `[dependencies]`:

```toml
url = "2"
rand = { version = "0.8", features = ["std", "std_rng"] }
```

(Workspace may already pin `rand`; use the workspace version if so.)

- [ ] **Step 3: Create the empty `feedback.rs` module**

Create `crates/client/src/feedback.rs` with just the error enum + skeleton, so it compiles before tests are added:

```rust
//! `Client::submit_feedback` and the `FeedbackError` enum.

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum FeedbackError {
    #[error("client is not connected to a network")]
    NotConnected,
    #[error("feedback worker is not configured for this build")]
    NotConfigured,
    #[error("feedback worker is unreachable")]
    WorkerUnreachable,
    #[error("request timed out")]
    Timeout,
    #[error("rate limited; retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },
    #[error("invalid input in field {field}: {message}")]
    InvalidInput { field: String, message: String },
    #[error("github returned {status}: {message:?}")]
    GithubFailure {
        status: u16,
        message: Option<String>,
    },
    /// Inner string is the worker-supplied URL, truncated to 512
    /// chars on receipt to bound error formatting.
    #[error("worker returned a malformed issue url: {0}")]
    BadIssueUrl(String),
    #[error("internal: {0}")]
    Internal(String),
}
```

- [ ] **Step 4: Wire the module into `lib.rs`**

Add to the `pub mod ...` declarations (alphabetically):

```rust
pub mod feedback;
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p willow-client`
Expected: pass.

- [ ] **Step 6: Commit**

```bash
git add crates/client/Cargo.toml crates/client/src/lib.rs crates/client/src/feedback.rs
git commit -m "feat(client): add feedback_worker config + FeedbackError"
```

### Task 3.2: `Client::submit_feedback` (TDD with `MemNetwork`)

**Files:**
- Modify: `crates/client/src/feedback.rs` (add `submit_feedback` method on `ClientHandle`)
- Create: `crates/client/src/tests/feedback.rs`
- Modify: `crates/client/src/tests/mod.rs` (or wherever the test module list lives)

- [ ] **Step 1: Inspect the existing pattern for sending a `WorkerRequest`**

Run: `grep -rn "WorkerRequest::Sync\|WorkerWireMessage::Request" crates/client/src/ | head -10`
Expected: existing `submit_sync_request` / `submit_history_request` methods (or similar) that build `WorkerRequest`, wrap in `WorkerWireMessage::Request`, gossip on `_willow_workers`, await a matching response. The implementer follows that exact shape.

If no such pattern exists in `willow-client` (the request path may live in worker_cache or another module), look at `crates/client/src/worker_cache.rs` and follow its conventions.

- [ ] **Step 2: Write the failing test**

Create `crates/client/src/tests/feedback.rs`:

```rust
//! Client-tier integration tests for Client::submit_feedback.
//! Uses MemNetwork to stand up a mock feedback worker.

use std::time::Duration;
use willow_client::feedback::FeedbackError;
use willow_client::{ClientConfig, ClientHandle};
use willow_common::{
    FeedbackCategory, FeedbackErrReason, WorkerRequest, WorkerResponse, WorkerWireMessage,
};
use willow_network::mem::MemNetwork;

// The exact spawn helper depends on the existing test infra; reuse
// the helper that other client tests use (e.g. `test_client()` from
// crates/client/src/tests/multi_peer_sync.rs or similar).

#[tokio::test]
async fn submit_feedback_returns_not_configured_when_unset() {
    let (client, _evloop) = make_client(None).await;
    let err = client
        .submit_feedback(
            "title".to_string(),
            FeedbackCategory::Bug,
            "body".to_string(),
            false,
        )
        .await
        .unwrap_err();
    assert_eq!(err, FeedbackError::NotConfigured);
}

#[tokio::test]
async fn submit_feedback_happy_path_returns_parsed_url() {
    let (client, worker_peer_id, _evloop_c, _evloop_w, _net) =
        spawn_client_and_mock_worker(|req| match req {
            WorkerRequest::Feedback { .. } => WorkerResponse::FeedbackOk {
                issue_url: "https://github.com/x/y/issues/42".to_string(),
            },
            _ => panic!("unexpected request"),
        })
        .await;
    let url = client
        .submit_feedback(
            "title".to_string(),
            FeedbackCategory::Bug,
            "body".to_string(),
            false,
        )
        .await
        .unwrap();
    assert_eq!(url.as_str(), "https://github.com/x/y/issues/42");
    let _ = worker_peer_id;
}

#[tokio::test]
async fn submit_feedback_maps_rate_limited() {
    let (client, _peer, _e1, _e2, _n) = spawn_client_and_mock_worker(|_req| {
        WorkerResponse::FeedbackErr {
            reason: FeedbackErrReason::RateLimited { retry_after_ms: 12_345 },
        }
    })
    .await;
    let err = client
        .submit_feedback(
            "t".to_string(),
            FeedbackCategory::Bug,
            "b".to_string(),
            false,
        )
        .await
        .unwrap_err();
    assert_eq!(err, FeedbackError::RateLimited { retry_after_ms: 12_345 });
}

#[tokio::test]
async fn submit_feedback_maps_bad_issue_url() {
    let (client, _peer, _e1, _e2, _n) = spawn_client_and_mock_worker(|_req| {
        WorkerResponse::FeedbackOk {
            issue_url: "not a url".to_string(),
        }
    })
    .await;
    let err = client
        .submit_feedback(
            "t".to_string(),
            FeedbackCategory::Bug,
            "b".to_string(),
            false,
        )
        .await
        .unwrap_err();
    matches!(err, FeedbackError::BadIssueUrl(_));
}

#[tokio::test]
async fn submit_feedback_returns_worker_unreachable_when_no_listener() {
    let fake_peer = willow_identity::Identity::generate().endpoint_id();
    let (client, _ev) = make_client(Some(fake_peer)).await;
    // No worker spawned; the request times out / returns unreachable.
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        client.submit_feedback(
            "t".to_string(),
            FeedbackCategory::Bug,
            "b".to_string(),
            false,
        ),
    )
    .await;
    assert!(result.is_ok(), "client must surface error within 5s");
    let err = result.unwrap().unwrap_err();
    assert!(
        matches!(
            err,
            FeedbackError::WorkerUnreachable | FeedbackError::Timeout
        ),
        "got {err:?}"
    );
}

// --- helpers ---------------------------------------------------------------

async fn make_client(
    feedback_worker: Option<willow_identity::EndpointId>,
) -> (ClientHandle<MemNetwork>, /* event loop join handle */ tokio::task::JoinHandle<()>) {
    let cfg = ClientConfig {
        feedback_worker,
        persistence: false,
        ..Default::default()
    };
    let identity = willow_identity::Identity::generate();
    // The exact API for spawning a ClientHandle in tests is project-specific —
    // the implementer follows the pattern in crates/client/src/tests/*.rs,
    // typically `ClientHandle::<MemNetwork>::new_with(cfg, identity, network)`
    // or similar. Returns (client, event_loop_handle).
    todo!("follow existing test_client() helper from crates/client/src/tests/")
}

async fn spawn_client_and_mock_worker(
    handler: impl Fn(&WorkerRequest) -> WorkerResponse + Send + Sync + 'static,
) -> (
    ClientHandle<MemNetwork>,
    willow_identity::EndpointId,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
    std::sync::Arc<MemNetwork>,
) {
    todo!("reuse the multi-peer test harness from crates/client/src/tests/multi_peer_sync.rs")
}
```

The two `todo!()` helpers MUST be filled in by following the existing client test harness. The plan can't write them here because the project's helper signatures are project-internal. **The implementer's first concrete action under Step 3 is to read `crates/client/src/tests/multi_peer_sync.rs` (or whichever sibling file currently contains the helpers) and copy-adapt the spawn pattern.**

- [ ] **Step 3: Wire the test module into the test list**

Edit `crates/client/src/tests/mod.rs` (or `crates/client/src/lib.rs`'s `#[cfg(test)] mod tests` declaration if that's the convention) to add `mod feedback;`.

- [ ] **Step 4: Run the tests, expect compile failure on `submit_feedback`**

Run: `cargo test -p willow-client --test feedback 2>&1 | head -20` (or whatever the test invocation pattern is — `cargo test -p willow-client feedback::` if the test module is inline).
Expected: COMPILE FAIL — `submit_feedback` undefined on `ClientHandle`.

- [ ] **Step 5: Implement `Client::submit_feedback`**

Append to `crates/client/src/feedback.rs`:

```rust
use std::time::Duration;

use rand::RngCore;
use willow_common::{
    FeedbackCategory, FeedbackDiagnostics, FeedbackErrReason, WorkerRequest, WorkerResponse,
    WorkerWireMessage,
};
use willow_identity::EndpointId;

const SUBMIT_TIMEOUT: Duration = Duration::from_secs(30);

impl<N: willow_network::Network> crate::ClientHandle<N> {
    /// Submit feedback to the configured feedback worker.
    pub async fn submit_feedback(
        &self,
        title: String,
        category: FeedbackCategory,
        body: String,
        include_diagnostics: bool,
    ) -> Result<url::Url, FeedbackError> {
        let Some(worker) = self.feedback_worker_peer() else {
            return Err(FeedbackError::NotConfigured);
        };

        // Generate a fresh dedup_id per call. Callers that need to
        // retry idempotently can use submit_feedback_with_dedup_id.
        let mut dedup_id = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut dedup_id);

        let diagnostics = if include_diagnostics {
            Some(build_diagnostics())
        } else {
            None
        };

        let req = WorkerRequest::Feedback {
            dedup_id,
            title,
            category,
            body,
            diagnostics,
        };

        // Submit through the same worker request/response path used
        // by replay/history requests. The exact helper name varies;
        // the pattern is `self.send_worker_request(worker, req).await`.
        let resp = match tokio::time::timeout(
            SUBMIT_TIMEOUT,
            self.send_worker_request(worker, req),
        )
        .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(map_send_err(e)),
            Err(_) => return Err(FeedbackError::Timeout),
        };

        match resp {
            WorkerResponse::FeedbackOk { issue_url } => {
                let truncated = if issue_url.chars().count() > 512 {
                    issue_url.chars().take(512).collect()
                } else {
                    issue_url
                };
                url::Url::parse(&truncated).map_err(|_| FeedbackError::BadIssueUrl(truncated))
            }
            WorkerResponse::FeedbackErr { reason } => Err(map_reason(reason)),
            other => Err(FeedbackError::Internal(format!(
                "unexpected response shape: {other:?}"
            ))),
        }
    }

    fn feedback_worker_peer(&self) -> Option<EndpointId> {
        // Implementer wires this to the field stored from
        // ClientConfig::feedback_worker at construction.
        self.config.feedback_worker
    }
}

fn map_reason(r: FeedbackErrReason) -> FeedbackError {
    match r {
        FeedbackErrReason::RateLimited { retry_after_ms } => {
            FeedbackError::RateLimited { retry_after_ms }
        }
        FeedbackErrReason::InvalidInput { field, message } => FeedbackError::InvalidInput {
            field: truncate(&field, 64),
            message: truncate(&message, 200),
        },
        FeedbackErrReason::GithubFailure { status, message } => FeedbackError::GithubFailure {
            status,
            message: message.map(|m| truncate(&m, 200)),
        },
        FeedbackErrReason::Unconfigured => FeedbackError::NotConfigured,
    }
}

fn map_send_err(e: impl std::fmt::Display) -> FeedbackError {
    let s = e.to_string();
    if s.contains("unreachable") || s.contains("no listener") {
        FeedbackError::WorkerUnreachable
    } else {
        FeedbackError::Internal(truncate(&s, 200))
    }
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn build_diagnostics() -> FeedbackDiagnostics {
    use willow_common::ClientPlatform;
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let build_hash = option_env!("WILLOW_BUILD_SHA").map(|s| s.to_string());
    #[cfg(target_arch = "wasm32")]
    let (locale, client) = wasm_diagnostics();
    #[cfg(not(target_arch = "wasm32"))]
    let (locale, client) = native_diagnostics();
    FeedbackDiagnostics {
        app_version,
        build_hash,
        locale,
        client,
    }
}

#[cfg(target_arch = "wasm32")]
fn wasm_diagnostics() -> (Option<String>, willow_common::ClientPlatform) {
    use willow_common::ClientPlatform;
    // Implementer wires this to web_sys / js_sys to read
    // `navigator.language` and parse the UA string into a coarse
    // family/major-version. See spec §"FeedbackDiagnostics".
    let locale = web_sys::window()
        .and_then(|w| w.navigator().language());
    let ua = web_sys::window()
        .map(|w| w.navigator().user_agent().unwrap_or_default())
        .unwrap_or_default();
    let ua_family = parse_ua_family(&ua);
    (locale, ClientPlatform::Web { ua_family })
}

#[cfg(target_arch = "wasm32")]
fn parse_ua_family(ua: &str) -> String {
    // Coarse parser: pick `firefox/<major>` etc. Implementer keeps
    // it conservative; falls back to "unknown/0" if no match.
    for (needle, name) in [
        ("Firefox/", "firefox"),
        ("Chrome/", "chrome"),
        ("Safari/", "safari"),
        ("Edge/", "edge"),
    ] {
        if let Some(idx) = ua.find(needle) {
            let rest = &ua[idx + needle.len()..];
            let major: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !major.is_empty() {
                return format!("{name}/{major}");
            }
        }
    }
    "unknown/0".to_string()
}

#[cfg(not(target_arch = "wasm32"))]
fn native_diagnostics() -> (Option<String>, willow_common::ClientPlatform) {
    use willow_common::ClientPlatform;
    let locale = std::env::var("LANG").ok().and_then(|l| {
        // Strip ".UTF-8" or "@variant" suffixes to leave bare BCP 47.
        l.split(['.', '@']).next().map(|s| s.replace('_', "-"))
    });
    (
        locale,
        ClientPlatform::Native {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        },
    )
}
```

The implementer adapts `self.send_worker_request(...)` to the existing project helper for sending a worker request and awaiting a typed response. If no such helper exists, use the same pattern as `crates/client/src/worker_cache.rs` — wrap in `WorkerWireMessage::Request { request_id, target_peer, payload }`, broadcast, listen for the matching `Response`.

- [ ] **Step 6: Run tests**

Run: `cargo test -p willow-client feedback`
Expected: 4 tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/client/
git commit -m "feat(client): Client::submit_feedback + FeedbackError mapping"
```


