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
| `--reporter-salt-file` | — | optional, default `/etc/willow/feedback-salt` | 32-byte random salt for the reporter-handle hash (see [GitHub issue format](#github-issue-format)) |
| `--generate-identity` | — | flag | Generate keypair at `--identity-path` and exit |
| `--generate-salt` | — | flag | Write 32 random bytes to `--reporter-salt-file` if missing and exit |
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

**Back-pressure scope.** The state-actor invariant is that one
message at a time is processed per actor instance. With async
`handle_request`, a slow GitHub call blocks the *feedback role's*
mailbox specifically — this is the right behavior because
`FeedbackRole::on_event` is a no-op (the feedback worker doesn't
track DAG state) and `heads_summaries` returns empty. For the
replay and storage roles, no message is ever slow enough to matter
(everything is in-memory or a small SQLite query), so the change
is observationally identical to today. The async trait is not
proposing per-actor concurrency — only the ability to `.await`
inside a handler.

**Migration impact.** Every `impl WorkerRole` must add `async` to
`handle_request`:

- `crates/replay/src/role.rs:264` (ReplayRole)
- `crates/storage/src/role.rs:62` (StorageRole)
- `crates/worker/src/actors/state.rs:113` (TestRole, in tests)
- `crates/worker/src/actors/heartbeat.rs:124` (TestRole, in tests)
- `crates/worker/src/actors/sync.rs:108` (TestSyncRole, in tests)

No call sites change other than the actor's `.await` on the role's
return value.

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
    /// `field` is bounded to 64 chars, `message` to 200 chars.
    /// The worker enforces these caps before constructing the
    /// reply; the client also enforces them on receipt.
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
    /// The inner string is the worker-supplied URL truncated to
    /// 512 chars on receipt to bound error formatting.
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
failure-state UI. The URL is constructed in the web app from a
**hard-coded** prefix (`https://github.com/`) plus the configured
`FEEDBACK_REPO` value, plus url-encoded title and body:

```
https://github.com/{owner}/{repo}/issues/new?title={...}&body={...}
```

`{owner}/{repo}` is validated against the regex
`^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$` *both* at worker startup (the
worker refuses to start if `FEEDBACK_REPO` doesn't match) and on
the web side before constructing the URL. This prevents a
mis-configured `FEEDBACK_REPO` (e.g. `javascript:alert(1)`) from
becoming an XSS or open-redirect vector via the fallback link.
Worst case, every path to feedback works.

Below the Submit button, the modal renders a small italic note:

> Your report and any included diagnostic info are visible to
> Willow infrastructure peers in transit (the project relay and
> feedback worker) until end-to-end encryption ships. Don't include
> passwords, tokens, or other secrets.

This is the user-facing acknowledgment of the unencrypted-transport
trade-off documented in [Trade-offs](#trade-offs). It shows
unconditionally — independent of whether the feedback worker is
configured — so users always know what the privacy posture is.

#### Configuration mechanism

There is **no** existing web-app config for a worker peer ID — relay
URL is the only externally configured peer in `crates/web/init.js`
today. We add a parallel mechanism:

- New window global `__WILLOW_FEEDBACK_PEER_ID` set by
  `crates/web/init.js`. The value is the bech32 form of an
  `EndpointId` produced by `EndpointId::Display` (the format
  defined in
  [`docs/specs/2026-04-24-bech32-identifiers.md`](./2026-04-24-bech32-identifiers.md)).
  Production: from env injected at container start; dev: from the
  local-dev plumbing below. If unset, the web app is "not
  configured" and renders the `NotConfigured` state above.
- `crates/web/init.js` is already the place that picks up
  `__WILLOW_RELAY_URL` and falls back to localhost in dev; the
  feedback peer ID follows the same pattern via the production
  entrypoint substitution and the dev `dev_assets/` fetch
  described in [Local dev plumbing](#local-dev-plumbing) and
  [Production peer ID injection](#production-peer-id-injection).
- The web app reads the global at `Client` construction and stores the
  parsed `EndpointId` in `ClientConfig::feedback_worker`.

### GitHub issue format

The worker constructs the issue body deterministically. **The
user-supplied body is wrapped in a fenced markdown code block** —
this is the single most important sanitization step:

````markdown
**Reporter (salted hash):** `whisper-quiet-fern-3a9cf`
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

1. **Body wrapping.** User body is placed inside a backtick-fenced
   markdown block with the `text` info-string. The fence length is
   chosen to be longer than any backtick run inside the body, per the
   CommonMark close-fence rule (§4.5):

   - Normalize line endings: `\r\n` → `\n` first.
   - Scan body for any line matching the regex
     `` ^[ ]{0,3}`{N,}[ \t]*$ `` (where N ≥ 3) — these are valid
     closing fences for a backtick block. Track the maximum N
     observed.
   - Choose opening/closing fence length = `max(3, N_max + 1)`.
     This guarantees no line in the body can close the fence.
   - Backtick fences are *only* closed by backtick fences (and
     vice-versa for tilde), so tilde-fenced content (`~~~`) inside
     the body is inert and needs no special handling.
   - HTML entities like `&#96;` are rendered as text (not parsed)
     inside any fenced code block, so they don't escape the fence.

   This neutralizes `@mention` notification spam, autolinks,
   markdown-image exfiltration, and metadata-block spoofing — the
   core threats the round-1 review identified. A unit test exercises
   adversarial inputs: closing-fence sequences of varying length,
   indented fences (1–3 leading spaces), CRLF mix, info-string
   tricks, raw HTML payloads (`<img onerror=...>`), and HTML
   entity-encoded backticks.

2. **Title sanitization.** Strip ASCII control chars (0x00–0x1F,
   0x7F), strip Unicode bidi/RTL override codepoints (U+202A–U+202E,
   U+2066–U+2069), collapse internal whitespace to single spaces,
   escape leading `[` / `]` with backslashes. Final title is
   `[Bug] <user title>` / `[Suggestion] <user title>` /
   `[Other:<detail>] <user title>` (the `<detail>` segment goes
   through the same sanitizer). Cap final title at 250 chars (200
   user + ~50 worker overhead).

3. **Total body cap.** After assembly, the worker asserts the
   composed issue body is **≤ 65,000 chars** (GitHub's documented
   issue-body limit is 65,536 chars; we leave a small safety margin).
   Note this is a *char* count, not a byte count — GitHub counts
   Unicode codepoints. Over-cap is a worker-side assembly bug
   (worst-case input is 8,000 body + 60 detail + ~500 metadata =
   well under 65,000), not a user-facing error; over-cap rejects
   with `FeedbackErrReason::Internal` after logging.

**Reporter handle.** The peer ID is **not** posted in cleartext.
The worker computes
`hash = blake3(worker_salt || peer_id_bytes)[..8]` (8 bytes = 64
bits — bumped from 6 bytes per round-2 review to put targeted
second-preimage attacks beyond practical reach for an open-source
project's threat model) and renders it as a deterministic 4-word
phrase plus 5-hex suffix (`whisper-quiet-fern-3a9cf`):

- **Wordlist:** the BIP-39 English wordlist (2048 words, 11 bits
  each) via the existing-in-ecosystem `bip39 = "2"` crate. v1
  vendors the wordlist into `crates/feedback/src/wordlist.rs` as a
  static array to avoid pulling the full bip39 dep tree (we only
  need the words, not bip39's mnemonic checksum).
- **Mapping:** take 11 bits at a time from the 64-bit hash → 4
  words consume 44 bits; the remaining 20 bits are rendered as a
  5-hex-char suffix, *not* 4 — fix to the 4-hex suffix written in
  the example above. (Round-2 review caught a bit-arithmetic
  mismatch.) Final form is `word-word-word-word-NNNNN` (lowercase,
  hyphens, 5 hex).

This:
- Lets maintainers correlate multiple reports from the same user
  without exposing the public Ed25519 key that signs the user's
  state-DAG events on every Willow server they participate in.
- The handle is a **display** aid, not an authenticator. The spec
  is explicit that maintainers MUST NOT take punitive action based
  on handle-match alone; the salted hash is a triage ergonomics
  tool, not an identity claim.
- Rotating the salt resets all correlation. This is the
  incident-response knob.

**Salt file.** Stored at `--reporter-salt-file` (default
`/etc/willow/feedback-salt`, which lives inside the
`feedback-identity` named docker volume so it survives container
restarts). The file is 32 random bytes; `--generate-salt` (added to
the CLI flag table) writes one if missing and exits. The
`feedback-entrypoint.sh` script runs `--generate-identity` and
`--generate-salt` if the corresponding files are missing, before
starting the worker.

**Salt rotation runbook.** When malicious reports require breaking
correlation across a window (or for routine hygiene):

```sh
docker compose exec feedback rm /etc/willow/feedback-salt
docker compose restart feedback
```

The entrypoint regenerates the salt; all subsequent reports get
fresh handles. The idempotency cache (`(signer, dedup_id) → url`)
is also flushed on restart (it's in-memory) — this avoids the case
where a pre-rotation cache entry leaks the old handle in a retry
response. Maintainers lose the ability to correlate reports across
the rotation boundary; this is the documented trade-off for being
able to break a sustained abuse pattern without coordinating with
GitHub support.

**Labels.** `feedback`, plus one of `feedback:bug`,
`feedback:suggestion`, `feedback:other`, plus
`feedback:triage` (always applied). Maintainers remove
`feedback:triage` after review and apply real workflow labels.
The worker creates labels lazily on first use.

Diagnostics are included only if the user opted in (the checkbox in
the UI controls `diagnostics: Option<FeedbackDiagnostics>` directly).
With diagnostics omitted, the metadata block shows
`(diagnostics not provided)` after the category line.

**Diagnostics non-mutation invariant.** The worker MUST post
diagnostic field values byte-equal to those received in the
request — no server-side enrichment (no GeoIP, no reverse-DNS, no
synthesized fields). The disclosure-UI promise is "what you see is
what is sent"; this invariant makes that promise enforceable. The
worker's posting code is implemented as a pure render of
`(diagnostics, sanitized_body, sanitized_title, hashed_handle)` and
a unit test asserts each diagnostic field appears verbatim in the
posted markdown for several inputs.

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
- **Restart-loop hardening:** the worker writes a tiny gating file
  at a fixed path inside the named identity volume —
  specifically `<dirname(identity_path)>/.feedback-last-boot`
  (e.g. `/etc/willow/.feedback-last-boot` for the default identity
  path). On startup, the worker:

  1. Opens the file with `O_CREAT | O_EXCL` → if creation
     succeeds, this is the first boot ever, write `now()` and
     proceed.
  2. If creation fails because the file exists: read its mtime,
     compute `delta = now() - mtime`. If `delta < 15 seconds`,
     `sleep(15 - delta)`. Then bump the file's mtime via the
     standard pattern of writing fresh contents to a sibling
     tempfile and `rename()`-ing over the original (atomic on
     POSIX, the `filetime::set_file_mtime` crate works too).
     Proceed.
  3. If the file is missing at read time (unlikely race with step
     1's open), treat as `delta = 15s` and proceed.

  The gating file lives inside the docker `feedback-identity`
  volume, so it survives container restarts and is invisible from
  outside the volume mount. Operators who deliberately recreate the
  volume reset the throttle along with the identity — that's
  intended behavior. Compose is configured with
  `restart: on-failure` (not `restart: always`) and a
  `max_attempts` of 5 within a window so a wedged worker doesn't
  flap forever; once `max_attempts` is exceeded, the operator
  inspects logs and decides.
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
    restart: on-failure:5            # bounded retry, paired with the
                                     # 15-second startup throttle inside
                                     # the worker (see Abuse protection)
  volumes:
    feedback-identity:
  ```

  `GITHUB_TOKEN` is loaded from `.env` (which `docker-compose` reads
  natively); `.env` MUST be in `.gitignore` (already true for the
  repo, but a precommit / CI guard rejecting staged `.env` is added
  to keep it that way). There is no `secrets:` block because v1
  targets a single-host docker-compose deployment; promoting to a
  real secrets backend is a follow-up.

  **Blast radius if the PAT leaks** (e.g. accidental `.env`
  commit): an attacker can file or edit issues on the configured
  `FEEDBACK_REPO` until the PAT is revoked. They cannot push code,
  read private repos, or affect any other GitHub account asset
  because the PAT is fine-grained and scoped to `Issues: write` on
  one repo. **Revocation runbook**: rotate the PAT in GitHub's
  fine-grained tokens UI, update `.env`, run
  `docker compose restart feedback`. The worker will surface
  `Unconfigured` if it sees a 401 in the meantime, and recover
  automatically once the new PAT is in place.

**Justfile additions:**

- `just build-feedback` — `cargo build --release -p willow-feedback`.
- `just docker-build` — gains the feedback image.
- `just docker-ids` — prints feedback peer ID alongside replay/storage
  via the binary's `--print-peer-id` flag.
- `just test-feedback` — `cargo test -p willow-feedback`.

**Local dev plumbing.** `scripts/dev.sh` already manages relay,
replay, and storage workers under `.dev/`. The feedback worker is
added analogously, with explicit first-run idempotency:

1. **Identity keypair generation** (run unconditionally; the
   command no-ops if the file exists):

   ```sh
   if [ ! -f .dev/feedback.key ]; then
     cargo run -q -p willow-feedback -- \
       --identity-path .dev/feedback.key \
       --generate-identity
   fi
   ```

2. **Print peer ID into a file the dev web build can serve:**

   ```sh
   cargo run -q -p willow-feedback -- \
     --identity-path .dev/feedback.key --print-peer-id \
     > crates/web/dev_assets/feedback-peer-id.txt
   ```

   The `crates/web/dev_assets/` directory is checked-in (with
   `.gitignore` excluding the generated `feedback-peer-id.txt`
   contents) and referenced from `crates/web/index.html` via:

   ```html
   <link data-trunk rel="copy-dir" href="dev_assets/" />
   ```

   This causes `trunk serve` to copy the directory into `dist/`,
   making `/dev_assets/feedback-peer-id.txt` reachable at the served
   URL `http://localhost:8080/dev_assets/feedback-peer-id.txt`. This
   is the *concrete* mechanism that replaces the round-1 hand-wave
   about trunk static-file serving — `trunk serve` has no
   `--static-dir` flag, so the only supported path is via
   `data-trunk` directives in `index.html`.

   In production builds the directive is still present, but the
   directory is empty (or contains a stub), so no production peer ID
   is ever served from this path. Production injection happens via
   the entrypoint described below.

3. **Web app fetch in dev:** `crates/web/init.js` already special-cases
   localhost for the relay URL. Add an analogous fetch for the
   feedback peer ID:

   ```javascript
   if (!window.__WILLOW_FEEDBACK_PEER_ID && (h === '127.0.0.1' || h === 'localhost')) {
     fetch('/dev_assets/feedback-peer-id.txt')
       .then(r => r.ok ? r.text() : '')
       .then(s => { window.__WILLOW_FEEDBACK_PEER_ID = s.trim(); })
       .catch(() => {});
   }
   ```

   The fetch is fire-and-forget; if it fails, the app renders the
   `NotConfigured` state (with the GitHub-direct fallback link).

4. **Dev worker runs without `GITHUB_TOKEN`.** It accepts and
   validates requests fully but every successful path replies
   `FeedbackErr { reason: Unconfigured }` instead of touching
   GitHub. This exercises every UI path end-to-end (idempotency
   cache, rate limit, sanitization, error surfaces) without leaking
   a real PAT into local environments.

**Production peer ID injection.** Today's `docker/web.Dockerfile` is
stock `nginx:alpine` with no entrypoint script — and not just for
feedback: the relay URL has the same gap (it's currently set at
build time). v1 introduces `docker/web-entrypoint.sh` to fix both:

```sh
#!/bin/sh
set -e

INIT_JS=/usr/share/nginx/html/init.js

# Substitute env-injected values into the served init.js. Both vars
# are optional: if unset, the placeholder remains and the web app
# treats the corresponding feature as not-configured.
if [ -n "$WILLOW_RELAY_URL" ]; then
  sed -i "s|__INJECT_RELAY_URL__|$WILLOW_RELAY_URL|g" "$INIT_JS"
fi
if [ -n "$WILLOW_FEEDBACK_PEER_ID" ]; then
  sed -i "s|__INJECT_FEEDBACK_PEER_ID__|$WILLOW_FEEDBACK_PEER_ID|g" "$INIT_JS"
fi

exec nginx -g 'daemon off;'
```

`crates/web/init.js` is updated to use the placeholders:

```javascript
window.__WILLOW_RELAY_URL = window.__WILLOW_RELAY_URL || "__INJECT_RELAY_URL__";
window.__WILLOW_FEEDBACK_PEER_ID = window.__WILLOW_FEEDBACK_PEER_ID || "__INJECT_FEEDBACK_PEER_ID__";
// If the placeholder survived (env not set in production), null it out.
if (window.__WILLOW_RELAY_URL === "__INJECT_RELAY_URL__") delete window.__WILLOW_RELAY_URL;
if (window.__WILLOW_FEEDBACK_PEER_ID === "__INJECT_FEEDBACK_PEER_ID__") delete window.__WILLOW_FEEDBACK_PEER_ID;
```

`docker/web.Dockerfile` is updated to:

```dockerfile
COPY docker/web-entrypoint.sh /docker-entrypoint.sh
RUN chmod +x /docker-entrypoint.sh
ENTRYPOINT ["/docker-entrypoint.sh"]
```

If `WILLOW_FEEDBACK_PEER_ID` is unset at container start, the form
renders `NotConfigured`. The same mechanism cleans up the
relay-URL build-time-injection drift that exists today; the
relay-URL part is in scope for this spec because we're touching
`init.js` and `web.Dockerfile` already.

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
logged. A unit test parameterized over every reachable code path
asserts that none of those four strings (each set to a distinct
unique sentinel) appears in captured `tracing` output. Paths
covered: happy path, every `FeedbackErrReason` variant, the
secondary-rate-limit cooldown, the 401 → permanent-Unconfigured
transition, the idempotency-cache hit, and a deliberately-panicking
mock-GitHub path (panics surface as `tracing::error!`, which the
test also captures).

## Trade-offs

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
- The mirror case — a v1 client receiving a
  `WorkerResponse::FeedbackOk` it doesn't know about — is also
  benign: only feedback-aware clients send the request, so v1
  clients never receive responses for requests they didn't send.

If future variants need to coexist with strict-version peers,
`PROTOCOL_VERSION` (currently `1` in
`crates/transport/src/lib.rs:30`) gets bumped — but for v1 we
explicitly *do not* bump, since the old-worker drop behavior is
benign here and a version bump would force a coordinated upgrade of
every relay/replay/storage instance.

**Reused gossip request path vs. dedicated encrypted ALPN.** During
brainstorming we initially proposed a new `/willow/feedback/0` ALPN
with direct iroh request/response. Reusing the existing gossip
pathway (`_willow_workers` topic,
`WorkerWireMessage::Request/Response`) keeps v1 drastically simpler
— no new transport code, no new dispatcher, no parallel correlation
logic — at the cost of feedback request payloads being visible to
other peers subscribed to `_willow_workers`. Since v1's reports are
destined for a public GitHub issue anyway *and* the sensitive
header data is salted-hashed before posting, that's an acceptable
trade-off. The encrypted ALPN is on the follow-up list.

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
    cached `issue_url` without contacting the mock; two distinct
    signers with the *same* `dedup_id` get distinct issue URLs (no
    cross-signer cache poisoning — guards a future refactor that
    might accidentally drop `signer` from the cache key).
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

Add a new `test-feedback` target alongside `test-replay` /
`test-storage` and append `-p willow-feedback` to the existing
`test-workers` aggregate (current line passes
`-p willow-worker -p willow-replay -p willow-storage -p willow-common`):

```just
test-feedback:
    cargo test -p willow-feedback

test-workers:
    cargo test -p willow-worker -p willow-replay -p willow-storage -p willow-feedback -p willow-common
```

Also append the feedback service to `just docker-ids` so the script
prints all four worker peer IDs (relay/replay/storage/feedback) in
one go.

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
