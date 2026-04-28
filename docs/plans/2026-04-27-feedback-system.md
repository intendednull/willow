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


