# In-App Feedback via GitHub-Proxy Worker

> **One-sentence summary:** add an in-app "Send Feedback" form in the web
> UI that submits to a new `willow-feedback` worker node, which holds a
> GitHub PAT and creates issues on `intendednull/willow` on the user's
> behalf — so users can report bugs, suggestions, or other issues without
> needing a GitHub account.

## Motivation

Willow has no in-app channel for user feedback. Users hitting a bug
either silently churn, find the GitHub repo on their own, or page the
maintainers out-of-band. We want a one-click path from "something is
wrong" to "the maintainer sees a structured report" without:

- Requiring users to have a GitHub account (most won't).
- Standing up new external infra to maintain (no email forwarder, no
  Cloudflare Worker, no third-party tracker).
- Polluting the event-sourced state DAG with reports that aren't shared
  state and shouldn't replicate to every peer.

A worker node is the right shape: it's the existing pattern for
project-run services that hold privileged credentials (relay,
replay, storage), and it keeps the feedback path off the per-server
state DAG.

## Scope

**In scope (v1):**

- New `willow-feedback` worker binary (alongside `willow-replay` and
  `willow-storage`), with sibling `docker/feedback.Dockerfile` +
  `docker/feedback-entrypoint.sh`.
- Settings → "Help & Feedback" UI in the Leptos web app: title,
  category, description, "include diagnostic info" checkbox.
- A `Feedback` request/response variant added to the existing
  `WorkerRequest` / `WorkerResponse` enums in `willow-common`, plus a
  `Feedback` variant in `WorkerRoleInfo`.
- A cross-cutting change to `WorkerRole::handle_request` to make it
  `async` (replay and storage become async fns that don't `.await`
  anything — see [Async trait change](#async-trait-change)).
- Per-peer **and** worker-wide rate limiting + length caps on the
  feedback worker. User-supplied content sanitized via mandatory
  fenced-code-block wrapping.
- A `Client::submit_feedback(...)` method on `willow-client`, with
  the feedback worker peer ID resolved from client config (not a
  caller parameter).
- A new `__WILLOW_FEEDBACK_PEER_ID` window global wired through
  `crates/web/init.js` for the web app, plus `.dev/feedback-peer-id`
  plumbing through `scripts/dev.sh` for local development.
- A `just test-feedback` target and a `feedback` service in
  `docker-compose.yml`.

**Out of scope (deferred to a larger redesign):**

- Attachments (logs, screenshots).
- Threaded replies / two-way conversation with the reporter.
- Per-server feedback workers (each server admin running their own).
- Feedback that lives in the per-server DAG.
- Encrypted-to-worker feedback over a dedicated ALPN. V1 reuses the
  existing gossip-based worker request pathway — see
  [Trade-offs](#trade-offs).
- Persistent rate-limit buckets across worker restarts.
- Consolidating `FeedbackErrReason` into the broader `WireRejectReason`
  proposed in [`2026-04-24-error-prefixes.md`](./2026-04-24-error-prefixes.md).
  V1 ships a feedback-local error enum with units aligned to
  `WireRejectReason` (ms, not secs); migration is a pure refactor
  once that spec lands.
- Real-time content moderation. v1 relies on pre-flight sanitization
  + GitHub-side moderation tools — see [Trade-offs](#trade-offs).

## Architecture

### New crate: `willow-feedback`

Native-only worker binary. Mirrors `willow-replay` and `willow-storage`
both in crate layout and in the surrounding docker/dev plumbing:

```
crates/feedback/
├── Cargo.toml
└── src/
    ├── main.rs       — CLI parsing, identity load, IrohNetwork bring-up
    ├── role.rs       — FeedbackRole : WorkerRole
    └── github.rs     — Thin reqwest-based client around POST /repos/:owner/:repo/issues

docker/
├── feedback.Dockerfile        — sibling to replay.Dockerfile / storage.Dockerfile
└── feedback-entrypoint.sh     — sibling to replay-entrypoint.sh
```

Built on `willow-worker`'s actor runtime: the role implements
`WorkerRole::handle_request`, the runtime handles identity, networking,
heartbeat, and request routing.

**HTTP client choice.** `reqwest` with `rustls-tls` (matches iroh's TLS
stack — no native OpenSSL dep). The client lives **only** in
`willow-feedback`'s `Cargo.toml`; `willow-common` stays dual-target,
so the new wire types added there (see below) must remain WASM-clean.

**CLI flags + env:**

| Flag | Env | Required | Notes |
| --- | --- | --- | --- |
| `--identity-path` | — | yes | Ed25519 keypair for the worker peer |
| `--relay-url` | — | optional | Iroh relay to connect through |
| `--github-token` | `GITHUB_TOKEN` | yes | Fine-grained PAT, `Issues: write` on the target repo only |
| `--github-repo` | `FEEDBACK_REPO` | yes (default: `intendednull/willow`) | `owner/repo` to file issues against |
| `--rate-limit-per-hour` | — | optional, default 5 | Per-peer cap |
| `--global-rate-limit-per-hour` | — | optional, default 50 | Worker-wide ceiling (see [Abuse](#abuse-protection-on-the-worker)) |
| `--generate-identity` | — | flag | Generate keypair at `--identity-path` and exit |
| `--print-peer-id` | — | flag | Print the bech32 peer ID for `--identity-path` and exit (used by `just docker-ids`) |

**PAT handling.** The PAT is wrapped in
`secrecy::SecretString` at load time and stored only inside
`FeedbackRole`. The role struct **must not** derive `Debug`; a clippy
`missing_debug_implementations` allow with a `// security: PAT` comment
makes the intent explicit. A unit test asserts the role does not
`Debug`-format. A misconfigured worker (missing token, unreachable
repo, wrong scope) fails closed and replies
`FeedbackErr { reason: Unconfigured }` to every request without
contacting GitHub.

### Async trait change

`WorkerRole::handle_request` is currently a synchronous method
(`crates/common/src/worker_types.rs:139`), called synchronously from
the state actor's message handler
(`crates/worker/src/actors/state.rs`). Replay and storage compute
their responses entirely in memory, so sync was sufficient.

A feedback worker must perform an HTTP call to GitHub before it can
return a response. We change the trait to:

```rust
#[async_trait::async_trait]
pub trait WorkerRole: Send + 'static {
    fn role_info(&self) -> WorkerRoleInfo;
    fn on_event(&mut self, event: &Event);
    async fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse;
    fn heads_summaries(&self) -> Vec<(String, HeadsSummary)> { vec![] }
}
```

Replay and storage become trivial converts — `async fn handle_request`
that doesn't `.await` anything. The state actor's
`Handler<WorkerRequestMsg>` is already `async fn handle(...)`, so it
just needs to `.await` the role's response. While `handle_request` is
running on the state actor, no other message is processed for that
role — this is the existing concurrency invariant and we preserve it.
A slow GitHub call therefore back-pressures further feedback requests
on the same worker, which is the right behavior (and combined with
the rate limits, bounds the worst case).

**Trade-off considered:** alternatively, `FeedbackRole::handle_request`
could spawn a tokio task that owns the HTTP work and signals back via a
oneshot channel, leaving the trait sync. Rejected because it requires
either (a) extending the actor's request/response correlation to handle
late replies, or (b) blocking inside `handle_request` on a spawned
task, which is the same back-pressure as async with extra plumbing.
Async-trait is the simpler, smaller diff. The cost — pulling in
`async-trait` (or using a manually-written `Pin<Box<dyn Future>>`
return type) — is acceptable.

**Migration impact.** Every `impl WorkerRole` must add `async` to
`handle_request` (replay, storage, the in-test `TestRole` in
`crates/worker/src/actors/state.rs:113`, and the in-test `TestSyncRole`
in `crates/worker/src/actors/sync.rs:108`). No call sites change other
than the actor's `.await`.

### Wire types in `willow-common`

Extend the existing `WorkerRequest` / `WorkerResponse` enums and
`WorkerRoleInfo` rather than introducing parallel types. This keeps the
worker dispatch path unchanged and lets the feedback role plug into the
same actor runtime as replay and storage.

All three enums also gain `#[non_exhaustive]` so future variants don't
silently break consumers that match exhaustively. The annotation is a
consumer-side compile guard; **bincode forward compatibility is
discussed in [Trade-offs](#trade-offs)**.

```rust
// Added to WorkerRoleInfo (#[non_exhaustive] applied to the enum).
WorkerRoleInfo::Feedback {
    reports_accepted: u64,
    reports_rejected: u64,
    /// Gauge: peers currently throttled by the per-peer bucket.
    currently_rate_limited: u32,
    /// Gauge: 1 if the worker is hot-tripped on the global cap, else 0.
    global_rate_limited: bool,
}
// AND: the existing match in `WorkerRoleInfo::role_name()`
// (crates/common/src/worker_types.rs:40-45) gains
//     WorkerRoleInfo::Feedback { .. } => "feedback",

// Added to WorkerRequest (#[non_exhaustive] applied to the enum).
WorkerRequest::Feedback {
    /// 16-byte client-generated dedup key. Worker maintains an LRU
    /// cache of (signer, dedup_id) → issue_url so retries after a
    /// network blip return the original URL instead of opening a
    /// duplicate issue.
    dedup_id: [u8; 16],
    /// 1..=200 chars. Bytes-counted, not graphemes.
    title: String,
    category: FeedbackCategory,
    /// 1..=8000 chars. The worker wraps this verbatim in a fenced
    /// markdown code block when posting to GitHub (see
    /// "GitHub issue format"); clients MUST NOT pre-format it.
    body: String,
    diagnostics: Option<FeedbackDiagnostics>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum FeedbackCategory {
    Bug,
    Suggestion,
    Other {
        /// Optional free-form subcategory. <= 60 chars. Surfaced as
        /// part of the issue title so triage isn't a black hole.
        detail: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct FeedbackDiagnostics {
    /// CARGO_PKG_VERSION of the submitting client.
    pub app_version: String,
    /// Short git SHA from `option_env!("WILLOW_BUILD_SHA")` injected
    /// by `build.rs`. None in dev builds.
    pub build_hash: Option<String>,
    /// IETF BCP 47 locale tag (e.g. "en-US"); helps triage RTL /
    /// date-format / pluralisation bugs.
    pub locale: Option<String>,
    /// Coarse-grained UA: browser family + major version only
    /// ("firefox/138", "chrome/130"). Full UA strings are *not*
    /// shipped — see "Privacy" in Trade-offs.
    pub client: ClientPlatform,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum ClientPlatform {
    Web {
        /// e.g. "firefox/138". <= 40 chars.
        ua_family: String,
    },
    Native {
        /// "linux" | "macos" | "windows".
        os: String,
        /// "x86_64" | "aarch64" | etc.
        arch: String,
    },
}

// Added to WorkerResponse (#[non_exhaustive] applied to the enum).
WorkerResponse::FeedbackOk { issue_url: String }
WorkerResponse::FeedbackErr { reason: FeedbackErrReason }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum FeedbackErrReason {
    /// Units are MILLISECONDS to align with the broader
    /// WireRejectReason design (docs/specs/2026-04-24-error-prefixes.md).
    /// Consolidating with that enum is on the follow-up list.
    RateLimited { retry_after_ms: u64 },
    InvalidInput { field: String, message: String },
    GithubFailure {
        status: u16,
        /// GitHub's `message` field, truncated to 200 chars. Useful
        /// for surfacing 422 validator failures in maintainer logs
        /// without re-running the failing call.
        message: Option<String>,
    },
    /// Worker has no PAT configured, or PAT was revoked (401) —
    /// see [Abuse](#abuse-protection-on-the-worker).
    Unconfigured,
}
```

**Reporter peer ID.** The reporter's peer ID is **not** carried in
`FeedbackRequest`. The worker recovers it from the `WireMessage`
envelope's verified signer via the existing `unpack_wire` path
(`crates/transport/src/lib.rs`, returning `(WireMessage, EndpointId)`).
A forged peer ID in any payload would be ignored. A unit test asserts
this invariant so a future refactor can't silently regress it.

**Per-variant size cap.** `WireMessage::Worker(_)` currently inherits
the 256 KB envelope cap (`crates/common/src/wire.rs:144`). Worst-case
feedback envelope is 200 (title) + 8000 (body) + 60 (detail) +
diagnostics ≈ 9 KB, well under cap. Decoder-side validation happens
*before* the worker accepts the request: title/body length, dedup
shape, diagnostics field lengths.

### Client API in `willow-client`

The client knows the feedback worker's peer ID from configuration set
at construction time (mirroring how relay/replay/storage workers are
discovered today). The public method does NOT take an `EndpointId`
parameter — exposing it would leak bootstrap config into every UI
caller.

```rust
// In willow-client config (e.g. ClientConfig):
pub feedback_worker: Option<EndpointId>,

// In willow-client::error (or wherever client error types live;
// match the existing pattern):
#[derive(Debug, thiserror::Error)]
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
    GithubFailure { status: u16, message: Option<String> },
    #[error("worker returned a malformed issue url: {0}")]
    BadIssueUrl(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl Client {
    /// Submit feedback to the configured feedback worker.
    /// Returns the GitHub issue URL on success.
    /// Returns NotConfigured if no feedback worker is set.
    pub async fn submit_feedback(
        &self,
        title: String,
        category: FeedbackCategory,
        body: String,
        include_diagnostics: bool,
    ) -> Result<url::Url, FeedbackError>;
}
```

Implementation:

1. The client generates a random `[u8; 16]` `dedup_id` per call.
2. Builds `FeedbackDiagnostics` from compile-time constants
   (`CARGO_PKG_VERSION`, `WILLOW_BUILD_SHA`) and runtime sniffing
   (`window.navigator` on web, `std::env::consts::{OS, ARCH}` on
   native) — but only if `include_diagnostics` is true. Otherwise
   `diagnostics: None`.
3. Wraps in `WorkerRequest::Feedback`, sends through the same
   `WorkerWireMessage::Request` gossip path used by replay/history
   requests, awaits the matching `WorkerResponse` (correlated by
   `request_id`).
4. On `FeedbackOk { issue_url }`, parses with `url::Url::parse`. A
   parse failure returns `FeedbackError::BadIssueUrl(...)` rather than
   handing a malformed string to the UI.
5. Maps `FeedbackErrReason` variants to `FeedbackError` variants
   one-to-one, preserving units (ms).

A 30-second total timeout (gossip round-trip + GitHub API) covers the
client side; longer than that surfaces as `FeedbackError::Timeout`.

**Idempotency**: if the user retries after a network blip, the same
`dedup_id` MUST be reused. The web UI keeps `dedup_id` in component
state between attempts and only regenerates it when the form is
cleared / "Send another" is clicked.

### Web UI in `willow-web`

- Entry point: **Settings page → "Help & Feedback" section**, with a
  "Send Feedback" button that opens a modal. (No top-bar icon, no
  command palette entry in v1 — settings only, per the explicit scope
  decision during brainstorming. Adding more entry points later is
  trivial once `Client::submit_feedback` exists.)
- Modal contents:
  - Title input (single line, `<= 200` chars; counter visible past 150).
  - Category dropdown: Bug / Suggestion / Other.
    Selecting "Other" reveals a `detail` text input (<= 60 chars).
    Default: Bug.
  - Body textarea (`<= 8000` chars; counter visible past 7500).
  - "Include diagnostic info" checkbox, default **checked**, with an
    expandable disclosure that renders the **exact**
    `FeedbackDiagnostics` value that will be sent (app version, build
    hash if any, locale, and either `Web { ua_family: "firefox/138" }`
    or `Native { os, arch }`). What you see is what is sent — no UA
    string surprises.
  - Submit / Cancel buttons.
- States: `Idle → Submitting → Success(issue_url) | Failure(reason)`.
  - Success shows the issue URL with a "Open issue" link and a "Send
    another" button. "Send another" clears the form *and* the
    component-local `dedup_id`.
  - Failure renders a human-friendly mapping of `FeedbackError` and
    keeps both the form and `dedup_id` populated so the user can
    retry idempotently. Retry of the same `dedup_id` returns the
    original issue URL if GitHub already received it.

#### Failure-state copy

| `FeedbackError` | UI surface |
| --- | --- |
| `NotConnected` | Inline error: "You're offline. Reconnect and try again." Form stays open. |
| `NotConfigured` | Form is disabled at mount; settings panel renders "Feedback is not configured for this build. <a href='https://github.com/intendednull/willow/issues/new'>File an issue on GitHub</a> instead." |
| `WorkerUnreachable` | Inline error: "The feedback service is currently unavailable. Please try again later, or <a>file an issue on GitHub directly</a>." with a fallback link. |
| `Timeout` | Inline error: "Submitting timed out. Check your connection and retry." |
| `RateLimited { retry_after_ms }` | Inline error: "You've sent too many reports. Try again in {N} minutes." |
| `InvalidInput { field, message }` | Field-level error highlighting the offending field. |
| `GithubFailure { status, message }` | Inline error: "GitHub rejected the report ({status}). {message}" — direct user to file directly on GitHub if persistent. |
| `BadIssueUrl(_)` | Inline error: "Submitted but the response was malformed. Check the GitHub issues page for your report." |
| `Internal(_)` | Inline error: "Something went wrong. Please retry." Logs full string to console. |

The fallback "file directly on GitHub" link is always present in the
failure-state UI, hand-built with the user's title and body
url-encoded (`https://github.com/intendednull/willow/issues/new?title=...&body=...`).
Worst case, every path to feedback works.

#### Configuration mechanism

There is **no** existing web-app config for a worker peer ID — relay
URL is the only externally configured peer in `crates/web/init.js`
today. We add a parallel mechanism:

- New window global `__WILLOW_FEEDBACK_PEER_ID` (bech32 string), set by
  `crates/web/init.js` (production: from env injected at container
  start; dev: from the local-dev plumbing below). If unset, the web
  app is "not configured" and renders the `NotConfigured` state above.
- `crates/web/init.js` is already the place that picks up
  `__WILLOW_RELAY_URL` and falls back to localhost in dev; the
  feedback peer ID follows the same pattern. For dev, when
  `location.hostname` is `127.0.0.1` or `localhost`, init.js attempts
  to fetch `/.dev/feedback-peer-id` (a static file served by
  `trunk serve`) and assigns the result to
  `__WILLOW_FEEDBACK_PEER_ID`.
- The web app reads the global at `Client` construction and stores the
  parsed `EndpointId` in `ClientConfig::feedback_worker`.

### GitHub issue format

The worker constructs the issue body deterministically. **The
user-supplied body is wrapped in a fenced markdown code block** —
this is the single most important sanitization step:

````markdown
**Reporter (salted hash):** `whisper-quiet-fern-3a9c`
**Category:** Bug
**App version:** 0.1.0
**Build:** abc1234
**Locale:** en-US
**Client:** Web { ua_family: "firefox/138" }

> Submitted via willow-feedback. The reporter's body is rendered
> verbatim in the fenced block below; @mentions, links, and image
> syntax inside it are **not** processed by GitHub.

```text
<user-supplied body>
```
````

Sanitization rules applied by the worker before assembly:

1. **Body wrapping.** User body is placed inside a `​```text` fenced
   block. Before insertion, the worker verifies the body does not
   contain a closing ` ``` ` sequence; if it does, the worker swaps
   the fence to a longer backtick run (` ```` ` → ` ````` ` etc.)
   that doesn't appear in the body. This neutralizes `@mention`
   notification spam, autolinks, image-pixel exfiltration, and
   metadata-block spoofing.
2. **Title sanitization.** Strip control chars, collapse internal
   whitespace, escape leading `[` / `]`. Final title is
   `[Bug] <user title>` / `[Suggestion] <user title>` /
   `[Other:<detail>] <user title>` (the `<detail>` segment is also
   sanitized).
3. **Total body cap.** After assembly, the worker asserts the
   composed issue body is `<= 60 KB` (well under GitHub's 65 KB
   limit). Over-cap is a worker-side bug, not a user-facing error;
   reject with `Internal`.

**Reporter handle.** The peer ID is **not** posted in cleartext.
The worker computes
`hash = blake3(worker_salt || peer_id_bytes)[..6]` and renders it
as a [bip39-style] human-friendly four-word phrase + 4-hex suffix
(`whisper-quiet-fern-3a9c`). The salt is loaded from
`--reporter-salt-file` (default: `/etc/willow/feedback-salt`,
generated on first run with `--generate-salt`). This:
- Lets maintainers correlate multiple reports from the same user
  without exposing the public Ed25519 key that signs the user's
  state-DAG events on every Willow server they participate in.
- Rotating the salt resets all correlation, which is the right
  knob for incident response.

**Labels.** `feedback`, plus one of `feedback:bug`,
`feedback:suggestion`, `feedback:other`, plus
`feedback:triage` (always applied). Maintainers remove
`feedback:triage` after review and apply real workflow labels.
The worker creates labels lazily on first use.

Diagnostics are included only if the user opted in (the checkbox in
the UI controls `diagnostics: Option<FeedbackDiagnostics>` directly).
With diagnostics omitted, the metadata block shows
`(diagnostics not provided)` after the category line.

### Abuse protection on the worker

- **Per-peer rate limit:** in-memory token bucket keyed by signer
  peer ID; default 5 requests / hour with a 1-hour refill.
- **Global rate limit:** in-memory token bucket worker-wide; default
  50 requests / hour with a 1-hour refill. **This is the main
  abuse-bound** because Ed25519 keys are free to generate (per-peer
  buckets do not constrain a determined attacker rotating
  identities). The global cap turns the attack from "unbounded
  spam" into "saturate the worker's bucket and stop." Operators can
  raise/lower at runtime via the CLI flags.
- **Restart-loop hardening:** worker process supervisor (Docker
  restart policy) uses exponential-backoff restart, and the worker
  refuses to start more than once per 15 seconds (a tiny gating
  file under the identity dir, touched on startup). This bounds
  rate-limit reset abuse without needing persistent buckets.
- **Length and shape validation:** title 1..=200, body 1..=8000,
  detail 0..=60, dedup_id length, diagnostics field caps. Rejection
  is `InvalidInput` *before* contacting GitHub.
- **Signature verification:** already enforced by `unpack_wire` on
  the inbound gossip path, so the worker only sees signed messages
  with a verified signer.
- **GitHub API failures:** non-2xx responses surface as
  `GithubFailure { status, message }`; the worker does not retry
  individual requests. The user can retry idempotently via the same
  `dedup_id`.
- **Secondary-rate-limit detection.** A `403` with
  `x-ratelimit-remaining: 0` (GitHub's secondary rate limit, signal
  of abuse-throttling) trips a worker-wide cooldown for the
  duration in `retry-after`. While cooled down, the worker replies
  `RateLimited { retry_after_ms }` to all callers.
- **PAT revocation.** A `401` response transitions the worker to
  the `Unconfigured` state for the rest of the process lifetime;
  all subsequent requests reply `Unconfigured`. Operator restart
  with a fresh PAT is required.
- **Idempotency cache.** An LRU of
  `(signer_peer_id, dedup_id) → issue_url` (capacity 4096 entries)
  short-circuits retries that arrive within the cache window; the
  worker returns the original `FeedbackOk { issue_url }` without
  contacting GitHub.

The worker does **not** moderate content beyond sanitization.
Issues are filed into the configured public repository and rely on
GitHub's own moderation tooling. Trade-offs and the residual abuse
risk are documented explicitly in [Trade-offs](#trade-offs).

### Deployment

**Docker.** Sibling files alongside replay and storage:

- `docker/feedback.Dockerfile` (mirrors `docker/replay.Dockerfile`).
- `docker/feedback-entrypoint.sh` (mirrors `docker/replay-entrypoint.sh`).
- New `feedback` service in `docker-compose.yml`:

  ```yaml
  feedback:
    build:
      context: .
      dockerfile: docker/feedback.Dockerfile
    depends_on:
      - relay
    environment:
      - GITHUB_TOKEN=${GITHUB_TOKEN}        # from .env, NOT committed
      - FEEDBACK_REPO=${FEEDBACK_REPO:-intendednull/willow}
      - RUST_LOG=info,willow_feedback=debug
    volumes:
      - feedback-identity:/etc/willow
  volumes:
    feedback-identity:
  ```

  `GITHUB_TOKEN` is loaded from `.env` (which `docker-compose` reads
  natively); `.env` is `.gitignore`-d. There is no `secrets:` block
  because v1 targets a single-host docker-compose deployment; promoting
  to a real secrets backend is a follow-up.

**Justfile additions:**

- `just build-feedback` — `cargo build --release -p willow-feedback`.
- `just docker-build` — gains the feedback image.
- `just docker-ids` — prints feedback peer ID alongside replay/storage
  via the binary's `--print-peer-id` flag.
- `just test-feedback` — `cargo test -p willow-feedback`.

**Local dev plumbing.** `scripts/dev.sh` already manages relay,
replay, and storage workers under `.dev/`. The feedback worker is
added analogously:

1. On first run, `scripts/dev.sh` invokes
   `cargo run -p willow-feedback -- --identity-path .dev/feedback.key
    --generate-identity` if the keypair is absent.
2. Then `cargo run -p willow-feedback -- --identity-path
    .dev/feedback.key --print-peer-id > .dev/feedback-peer-id`.
3. The dev web app is served from a directory that includes
   `.dev/feedback-peer-id` as `/.dev/feedback-peer-id` (configure
   trunk's `--ignore` / static dir as needed; the simplest option is
   to add a `dev_assets/` symlink). `crates/web/init.js` fetches it
   on dev hostnames and assigns to `__WILLOW_FEEDBACK_PEER_ID`.
4. The dev feedback worker runs **without** a `GITHUB_TOKEN`. It
   accepts and validates requests fully, but every successful path
   replies `FeedbackErr { reason: Unconfigured }` instead of touching
   GitHub. This exercises every UI path end-to-end (idempotency
   cache, rate limit, sanitization, error surfaces) without leaking
   a real PAT into local environments.

**Production peer ID injection.** `docker/web.Dockerfile` does not
own the feedback peer ID — the deployment's web container entrypoint
reads `WILLOW_FEEDBACK_PEER_ID` from the environment and injects it
into the served `init.js` at startup (a tiny `sed` step in the
existing entrypoint, mirroring how the relay URL is injected today).
If unset, the form renders `NotConfigured`.

### Observability

Per-request logging is critical for debugging "user X says they
submitted at 14:02 but no issue exists." Each `handle_request` invocation
emits exactly one structured log line:

```
INFO feedback_request id=<request_id> signer=<bech32 prefix>…
     category=Bug body_len=243 dedup=<hex8> github_status=201
     issue=<url> latency_ms=412
```

Log fields:

- `id` — `WorkerWireMessage::Request::request_id`.
- `signer` — first 8 chars of the bech32 peer ID (full ID at debug
  level only; salted hash at info to limit cross-server correlation
  surface in operator logs).
- `category` — Bug / Suggestion / Other(detail).
- `body_len` — bytes (not chars).
- `dedup` — first 8 hex chars of `dedup_id`.
- `github_status` — HTTP status from GitHub, or `cache` if served
  from the idempotency cache, or `rate-limited` / `invalid` /
  `unconfigured` / `cooldown` for non-GitHub paths.
- `issue` — GitHub issue URL on success, omitted otherwise.
- `latency_ms` — total request latency.

The PAT, the salt, the user body, and the user title are **never**
logged. A unit test asserts none of these strings appear in any log
output the role emits during a happy-path or error-path call.

## Trade-offs

**Reused gossip request path vs. dedicated encrypted ALPN.** During
brainstorming we initially proposed a new `/willow/feedback/0` ALPN
with direct iroh request/response. Inspecting the existing worker
infrastructure showed that replay and storage already share a single
gossip-based request/response pathway (`_willow_workers` topic,
`WorkerWireMessage::Request/Response`). Reusing that pathway keeps v1
drastically simpler — no new transport code, no new dispatcher, no
parallel correlation logic — at the cost of feedback request payloads
being visible to other peers subscribed to `_willow_workers`. Since
v1's reports are destined for a public GitHub issue anyway, that's an
acceptable trade-off for the initial cut. A dedicated encrypted ALPN
is on the follow-up list with the broader feedback redesign.

**In-memory rate limit vs. persistent.** A restart resets every
peer's bucket. For a single instance with light load this is fine —
combined with the global cap and the 15-second restart-throttle,
worst-case abuse is "saturate the worker, restart, saturate again
once per 15 seconds." Persistent buckets (SQLite or piggyback on
storage worker) are deferred until we see actual abuse.

**Diagnostics opt-in default checked.** Defaulting to **checked**
trades a little user privacy for dramatically more useful reports.
The disclosure renders the *exact* `FeedbackDiagnostics` value that
will be sent — no UA string surprises — and the user can opt out
per-report. Reports without version/build info are nearly useless for
triage and require a maintainer round-trip to ask for them.

**Hard-coded repo target.** Configurable via env so a fork can point
at its own repo, but there is no per-server / per-user override. v1
is for the upstream project; multi-tenant routing belongs to the
larger redesign.

**Forward compatibility.** Adding `WorkerRequest::Feedback` /
`WorkerResponse::FeedbackOk` / `WorkerRoleInfo::Feedback` to the
existing enums means a v1 peer (e.g. an old replay or storage worker
running an older binary) bincode-deserializing a `WireMessage::Worker`
that wraps the new variant will fail decode and **drop the entire
envelope**. Since `WorkerWireMessage::Request` is gossiped on
`_willow_workers` and addressed by `target_peer`, every subscribed
worker attempts decode — old workers simply log a warn and drop. This
is acceptable because:

- Old workers wouldn't respond to a `Feedback` request anyway.
- Drops surface as warn-level decode errors, not outages.
- The new variants are addressed only at the feedback worker via
  `target_peer`; cross-version chatter that's not feedback isn't
  affected.

If future variants need to coexist with strict-version peers,
`PROTOCOL_VERSION` (currently `1` in
`crates/transport/src/lib.rs:30`) gets bumped — but for v1 we
explicitly *do not* bump, since the old-worker drop behavior is
benign here and a version bump would force a coordinated upgrade of
every relay/replay/storage instance.

**Reused gossip request path vs. dedicated encrypted ALPN.** During
brainstorming we initially proposed a new `/willow/feedback/0` ALPN
with direct iroh request/response. Reusing the gossip pathway keeps
v1 drastically simpler at the cost of feedback request payloads
being visible to other peers subscribed to `_willow_workers`. Since
v1's reports are destined for a public GitHub issue anyway *and* the
sensitive header data is salted-hashed before posting, that's an
acceptable trade-off. The encrypted ALPN is on the follow-up list.

**Content moderation: pre-flight sanitization, not real-time
moderation.** Issues are filed into a public repository. The worker
applies fenced-code-block wrapping (defeats `@mentions`, autolinks,
markdown-image exfiltration, metadata-block spoofing), title
sanitization, length caps, per-peer + global rate limits, and
restart-throttling — but it does NOT run content classifiers, NOT
diff against an abuse blocklist, and NOT review submissions before
posting. The residual risk: a user can still post abusive prose that
GitHub's own policies might flag. Mitigations:

- The `feedback:triage` label keeps reports out of the default
  issue triage view until a maintainer has reviewed them.
- Maintainers can lock or delete issues via standard GitHub tools.
- The salted reporter handle (rotateable salt) supports incident
  response without de-anonymizing legitimate reporters.
- The global rate limit (50/hour) bounds the rate of abusive
  postings.

If even one report ends up being a serious GitHub-ToS violation, the
operator can rotate the salt (resetting correlation), lower the
global cap, or take the worker offline (the UI then shows
`WorkerUnreachable` with the GitHub-direct fallback link).
Real-time moderation (private triage repo with manual promotion,
or model-based content filter) is on the follow-up list.

**Identity rotation defeats per-peer rate limit.** Ed25519 keypairs
are free to generate; an attacker rotating identities bypasses the
per-peer 5/hour bucket. The global 50/hour cap is the real
abuse-bound. The per-peer bucket is defense-in-depth against a
single legitimate user accidentally over-submitting, not a
protection against motivated abuse. This is documented honestly
rather than overclaimed.

**`FeedbackErrReason` reinvents `WireRejectReason`.** The existing
spec
[`2026-04-24-error-prefixes.md`](./2026-04-24-error-prefixes.md)
proposes a typed `WireRejectReason` enum (RateLimited, Invalid,
PermissionDenied, …) with the same semantics. We use a feedback-local
enum here to ship without depending on that spec landing first, but
the units and shape (`retry_after_ms`, `InvalidInput { field, message }`)
are aligned so consolidation is a pure refactor. Tracked in
[Follow-ups](#follow-ups).

## Testing

Per CLAUDE.md's "Which test tier to use" decision tree, push each
behavior to the lowest tier that covers it. Concretely:

**`willow-common` (`cargo test -p willow-common`):**
- Round-trip `WorkerRequest::Feedback` (with and without
  diagnostics; `Other { detail }` populated and `None`),
  `WorkerResponse::FeedbackOk`, `WorkerResponse::FeedbackErr` (each
  `FeedbackErrReason` variant), and `WorkerRoleInfo::Feedback`
  through bincode AND through the full `pack_wire`/`unpack_wire`
  envelope path. Mirrors the existing worker round-trip tests.
- Assert `WorkerRoleInfo::role_name()` returns `"feedback"` for the
  new variant.

**`willow-feedback` role tests (`just test-feedback`):**
- Exercise `FeedbackRole` directly with a mock GitHub-client trait.
  No live HTTP. Cases:
  - Happy path → `FeedbackOk { issue_url }`.
  - Per-peer rate limit trips at the 6th request from the same peer
    within the window; `retry_after_ms` returned.
  - Global rate limit trips at the 51st request across distinct
    peers; persists until the bucket refills.
  - Length validation: title >200, body >8000, detail >60.
  - Missing-token startup path returns `Unconfigured` for every
    request.
  - 401 from mock GitHub transitions the role to `Unconfigured`
    permanently for the rest of the process.
  - 403 with `x-ratelimit-remaining: 0` trips secondary-rate-limit
    cooldown; subsequent requests get `RateLimited` with the
    advertised `retry-after`.
  - Idempotency cache: the same `(signer, dedup_id)` returns the
    cached `issue_url` without contacting the mock.
  - Sanitization: bodies containing closing fence sequences switch
    to a longer fence; `@mentions` and image-exfiltration syntax
    survive intact inside the fence (verified by string assertion).
  - Reporter handle is salted hash, never raw peer ID.
  - Logging: PAT, salt, user title, and user body do not appear in
    captured `tracing` output for any path.
  - Role does not implement `Debug`-printing the PAT (compile-time
    plus a runtime test that calls a custom `Display` if one
    exists).

**`willow-feedback` GitHub client unit tests:**
- Parse representative GitHub API responses (201 created, 422
  validation, 401 unauthorized, 403 secondary-rate-limit, 404 repo
  not found) into `FeedbackErrReason`.
- Verify the assembled issue body satisfies all sanitization
  invariants for several adversarial inputs (closing fences,
  Unicode chars, leading `[`).

**`willow-client` tests (`just test-client`, in
`crates/client/src/tests/feedback.rs`):**
- Per CLAUDE.md, "Client API + derivation, no DOM" lives at the
  client tier. Stand up a mock feedback worker via `MemNetwork`,
  call `Client::submit_feedback`, and assert:
  - Happy path resolves to a parsed `url::Url`.
  - `dedup_id` is generated per call and reused on retry within the
    same UI submission flow (verified by ferrying state through the
    test fixture).
  - `RateLimited` maps to `FeedbackError::RateLimited` with units
    preserved (ms).
  - `WorkerUnreachable` surfaces when the worker peer has no
    listener.
  - `NotConfigured` returned when `feedback_worker` is `None`.
  - `BadIssueUrl` returned when the worker replies with a malformed
    URL.

**`willow-web` browser tests (`just test-browser`, in
`crates/web/tests/browser.rs`):**
- Per CLAUDE.md, "DOM rendering or event dispatch" lives at the
  browser tier. Mount the settings page; verify:
  - Feedback button renders inside the "Help & Feedback" section.
  - Modal opens, validates input, disables submit when empty.
  - "Other" category reveals the `detail` input.
  - Diagnostics disclosure renders the exact `FeedbackDiagnostics`
    that will be submitted.
  - Each `FeedbackError` variant maps to the documented
    failure-state copy (mock the client).
  - "Send another" clears form *and* `dedup_id`; "Retry" preserves
    both.
  - With `__WILLOW_FEEDBACK_PEER_ID` unset, the form is disabled
    and shows the GitHub-direct fallback link.

**No Playwright E2E for v1.** The multi-peer scenario (peer A
submits, worker B forwards to GitHub) is covered at the client tier
against `MemNetwork`. The only thing Playwright would add is "real
iroh transport across two browsers," which is already exercised for
replay/storage in existing e2e specs and doesn't need to be
re-covered here. A *deferred* Playwright test that sends a feedback
report through the docker-compose stack with a stubbed GitHub server
is on the follow-up list — useful for catching docker/dev-plumbing
regressions, but not blocking v1.

**Test commands (justfile):**

```just
test-feedback:
    cargo test -p willow-feedback

# Folded into existing aggregate targets:
test-workers: test-replay test-storage test-feedback ...
```

## Follow-ups

These are explicitly **not** part of this spec but should land in the
follow-ups list when v1 ships:

- Larger feedback redesign (maintainer has ideas requiring major
  refactoring — capture in a separate spec when ready).
- Dedicated encrypted ALPN so feedback bodies are not visible to other
  workers-topic subscribers.
- Consolidate `FeedbackErrReason` into `WireRejectReason` once
  [`2026-04-24-error-prefixes.md`](./2026-04-24-error-prefixes.md)
  lands. Units are pre-aligned (ms); the migration is mechanical.
- Persistent rate-limit buckets (small JSON file or shared with
  storage).
- Real-time content moderation: private triage repo with manual
  promotion, or a model-based content filter, or an explicit
  blocklist. Today's approach (sanitization + rate limit + GitHub
  moderation) is honest defense-in-depth, not a moderation story.
- Attachment support (recent log buffer, screenshot capture).
- Two-way replies — the worker writes a comment-back path so a
  maintainer's GitHub reply lands in the reporter's UI.
- Per-server feedback workers and per-server routing.
- A "Send feedback" command-palette entry and a "?" top-bar shortcut
  once the form is proven.
- Playwright E2E covering the docker-compose feedback stack with a
  stubbed GitHub server (catches dev-plumbing regressions).
- Promote `GITHUB_TOKEN` from compose `.env` to a real secrets
  backend when v1 deployment moves beyond a single host.

## Open questions

None at spec time. Brainstorming + round-1 review resolved:

- Delivery target → GitHub issues via a project-run worker.
- Transport → existing worker gossip request/response pathway (not a
  new ALPN); forward-compat behavior with v1 workers documented in
  Trade-offs.
- UI entry point → Settings page only, in v1.
- Per-peer rate limit → 5/hour; **global** rate limit → 50/hour
  (the real abuse bound, given free identity rotation).
- Anonymity → signed by the user's identity, but the issue posts a
  salted-hash reporter handle (rotateable salt for incident
  response). No display name, no raw peer ID, no full UA string.
- Sanitization → user body wrapped in fenced code block (defeats
  `@mention` spam, autolinks, image exfil, metadata spoofing).
- Idempotency → 16-byte client-generated `dedup_id` + worker LRU
  cache.
- Trait change → `WorkerRole::handle_request` becomes async.
  Migration impact: replay, storage, in-test roles each gain
  `async fn`; no call sites change other than `.await`ing.
