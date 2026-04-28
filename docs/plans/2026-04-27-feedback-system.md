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


