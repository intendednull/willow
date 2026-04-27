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
  `willow-storage`).
- Settings → "Help & Feedback" UI in the Leptos web app: title,
  category, description, "include diagnostic info" checkbox.
- A `Feedback` request/response variant added to the existing
  `WorkerRequest`/`WorkerResponse` enums in `willow-common`.
- Per-peer rate limiting (5 reports/hour) and length caps on the
  feedback worker.
- A `Client::submit_feedback(...)` method on `willow-client`.
- The project-run feedback worker peer ID configurable via env/config
  (same shape as relay/replay/storage configuration today).

**Out of scope (deferred to a larger redesign):**

- Attachments (logs, screenshots).
- Threaded replies / two-way conversation with the reporter.
- Per-server feedback workers (each server admin running their own).
- Feedback that lives in the per-server DAG.
- Encrypted-to-worker feedback over a dedicated ALPN. V1 reuses the
  existing gossip-based worker request pathway — see
  [Trade-offs](#trade-offs).

## Architecture

### New crate: `willow-feedback`

Native-only worker binary. Mirrors the structure of `willow-replay` and
`willow-storage`:

```
crates/feedback/
├── Cargo.toml
└── src/
    ├── main.rs       — CLI parsing, identity load, IrohNetwork bring-up
    ├── role.rs       — FeedbackRole : WorkerRole
    └── github.rs     — Thin HTTP client around POST /repos/:owner/:repo/issues
```

Built on `willow-worker`'s actor runtime: the role implements
`WorkerRole::handle_request`, the runtime handles identity, networking,
heartbeat, and request routing.

**Configuration (CLI flags + env):**

| Flag | Env | Required | Notes |
| --- | --- | --- | --- |
| `--identity-path` | — | yes | Ed25519 keypair for the worker peer |
| `--relay-url` | — | optional | Iroh relay to connect through |
| `--github-token` | `GITHUB_TOKEN` | yes | GitHub PAT with `issues:write` |
| `--github-repo` | `FEEDBACK_REPO` | yes (default: `intendednull/willow`) | `owner/repo` to file issues against |
| `--rate-limit-per-hour` | — | optional, default 5 | Per-peer cap |

The PAT is read once at startup; never logged; stored only in the
role's memory. A misconfigured worker (missing token, unreachable repo)
fails closed and replies `Denied` to every request.

### Wire types in `willow-common`

Extend the existing `WorkerRequest` / `WorkerResponse` enums and
`WorkerRoleInfo` rather than introducing parallel types. This keeps the
worker dispatch path unchanged and lets the feedback role plug into the
same actor runtime as replay and storage.

```rust
// Added to WorkerRoleInfo
WorkerRoleInfo::Feedback {
    reports_accepted: u64,
    reports_rejected: u64,
    rate_limited_peers: u32,
}

// Added to WorkerRequest
WorkerRequest::Feedback {
    title: String,                   // <= 200 chars
    category: FeedbackCategory,
    body: String,                    // <= 8000 chars
    diagnostics: Option<FeedbackDiagnostics>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum FeedbackCategory { Bug, Suggestion, Other }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FeedbackDiagnostics {
    pub app_version: String,         // CARGO_PKG_VERSION
    pub build_hash: Option<String>,  // git short SHA at build time, if present
    pub user_agent: String,          // browser UA (web only) or "native"
    pub platform: String,            // "wasm32" | "linux" | "macos" | "windows"
}

// Added to WorkerResponse
WorkerResponse::FeedbackOk { issue_url: String }
WorkerResponse::FeedbackErr { reason: FeedbackErrReason }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum FeedbackErrReason {
    RateLimited { retry_after_secs: u32 },
    InvalidInput { field: String, message: String },
    GithubFailure { status: u16 },
    Unconfigured,
}
```

Note the reporter's peer ID is **not** in `FeedbackRequest` — it's
recovered from the `WireMessage` envelope's signer at the worker side
via the existing `unpack_wire` path. That guarantees the reported peer
ID matches the actual signer; a forged ID in the payload would be
ignored.

### Client API in `willow-client`

```rust
impl Client {
    pub async fn submit_feedback(
        &self,
        feedback_worker: EndpointId,
        title: String,
        category: FeedbackCategory,
        body: String,
        include_diagnostics: bool,
    ) -> Result<String, FeedbackError>;
}

pub enum FeedbackError {
    NotConnected,
    WorkerUnreachable,
    Timeout,
    RateLimited { retry_after_secs: u32 },
    InvalidInput { field: String, message: String },
    GithubFailure { status: u16 },
    Unconfigured,
    Internal(String),
}
```

The client serializes the request through the same `WorkerRequest`
gossip path used by replay/history requests, awaits the matching
`WorkerResponse` (correlated by `request_id`), and maps it to
`FeedbackError`.

### Web UI in `willow-web`

- Entry point: **Settings page → "Help & Feedback" section**, with a
  "Send Feedback" button that opens a modal. (No top-bar icon, no
  command palette entry in v1 — settings only, per the explicit scope
  decision during brainstorming. Adding more entry points later is
  trivial once `Client::submit_feedback` exists.)
- Modal contents:
  - Title input (single line, `<= 200` chars; counter visible past 150).
  - Category dropdown: Bug / Suggestion / Other (default: Bug).
  - Body textarea (`<= 8000` chars; counter visible past 7500).
  - "Include diagnostic info" checkbox, default **checked**, with a
    disclosure showing exactly what would be attached (app version,
    build hash, user agent, platform). Diagnostics are visible to the
    user before submission — no surprises.
  - Submit / Cancel buttons.
- States: `Idle → Submitting → Success(issue_url) | Failure(reason)`.
  - Success shows the issue URL with a "Open issue" link and a "Send
    another" button.
  - Failure renders a human-friendly mapping of `FeedbackError` and
    keeps the form populated so the user can retry.
- Configuration: the feedback worker peer ID is loaded from web-app
  configuration alongside the existing relay/worker bootstrap config.
  If unset, the form is disabled with a clear "Feedback is not
  configured for this build" message.

### GitHub issue format

The worker constructs the issue body deterministically:

```markdown
**Reported by peer:** `<bech32-prefixed peer id>`
**Category:** Bug
**App version:** 0.1.0
**Build:** abc1234
**Platform:** wasm32 (Mozilla/5.0 ...)

---

<user-supplied body>
```

Title is the user-supplied title with a `[Bug]` / `[Suggestion]` /
`[Other]` prefix added by the worker so issues are scannable in the
GitHub UI. Labels: `feedback`, plus one of `feedback:bug`,
`feedback:suggestion`, `feedback:other`. The worker creates these
labels lazily on first use.

The peer ID is included as a stable pseudonymous handle so maintainers
can correlate multiple reports from the same user without exposing a
display name. Diagnostics are included only if the user opted in.

### Abuse protection on the worker

- **Rate limit:** in-memory token bucket keyed by signer peer ID;
  default 5 requests / hour with a 1-hour refill. Resets on worker
  restart (acceptable for v1 — see [Follow-ups](#follow-ups)).
- **Length validation:** title `<= 200`, body `<= 8000`; reject with
  `InvalidInput` before contacting GitHub.
- **Signature verification:** already enforced by `unpack_wire` on the
  inbound gossip path, so the worker only ever sees signed messages
  with a verified signer.
- **GitHub API failures:** non-2xx responses surface as
  `GithubFailure { status }`; the worker does not retry. The user can
  retry from the UI.

The worker does **not** moderate content. Issues are filed verbatim
into a public repository, so abuse is bounded by the rate limit and by
GitHub's own moderation tooling.

### Deployment

- Add a `Dockerfile` for the feedback worker mirroring
  `crates/replay/Dockerfile` / `crates/storage/Dockerfile`.
- Add a `feedback` service to `docker-compose.yml` and to `just dev`
  alongside the existing replay/storage services.
- Add `just build-feedback` and integrate into `just docker-build`.
- The local `just dev` stack runs the feedback worker with a *blank*
  GitHub token by default — it accepts requests, validates them, and
  returns `Unconfigured`. This lets developers exercise the UI flow
  end-to-end without leaking a real PAT into local environments.

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
the worst case is "abuser restarts the worker and reports 5 more
times before the next refill," which is bounded. Persistent buckets
add a SQLite dependency (or piggyback on storage worker) for marginal
benefit; defer until we see actual abuse.

**Diagnostics opt-in default checked.** Defaulting to **checked**
trades a little user privacy for dramatically more useful reports.
The disclosure makes the contents explicit, and the user can opt out
per-report. We considered defaulting unchecked, but reports without
version/build info are nearly useless for triage and require a
maintainer round-trip to ask for them.

**Hard-coded repo target.** Configurable via env so a fork can point
at its own repo, but there is no per-server / per-user override. v1
is for the upstream project; multi-tenant routing belongs to the
larger redesign.

## Testing

- **`willow-common` unit tests:** round-trip the new
  `WorkerRequest::Feedback`, `WorkerResponse::FeedbackOk` /
  `FeedbackErr`, and `WorkerRoleInfo::Feedback` variants through
  bincode and through the full `pack_wire` / `unpack_wire` path
  (alongside the existing worker round-trip tests).
- **`willow-feedback` role tests:** exercise `FeedbackRole` directly
  with a mock GitHub client trait — happy path, rate limit
  enforcement, length validation, GitHub failure mapping, missing-token
  `Unconfigured` path. No live HTTP.
- **`willow-feedback` HTTP unit tests:** the GitHub client module
  parses representative GitHub API responses (success, 422 validation
  error, 403 abuse detection, 404 repo not found) into
  `FeedbackErrReason`.
- **`willow-client` test:** add to `crates/client/src/tests/`. Stand
  up a mock worker via `MemNetwork`, send a feedback request, assert
  the `Client::submit_feedback` future resolves to the issue URL.
  Includes a rate-limit-mapping test and a worker-unreachable test.
- **`willow-web` browser test:** in `crates/web/tests/browser.rs`,
  mount the settings page, open the feedback modal, fill the form,
  assert the submit handler is called with the right `FeedbackRequest`
  and that success/failure UI states render correctly. Use a stubbed
  client (no real network).
- **No Playwright E2E for v1.** Per the project's testing policy, the
  multi-peer scenarios this would cover (peer A submits, worker B
  forwards) are exercised by the client-tier test against
  `MemNetwork`. The only thing Playwright would add is "real iroh
  transport," which we already cover for replay/storage and don't
  need to re-cover here.

## Follow-ups

These are explicitly **not** part of this spec but should land in the
follow-ups list when v1 ships:

- Larger feedback redesign (the user has ideas requiring major
  refactoring — capture in a separate spec when ready).
- Dedicated encrypted ALPN so feedback bodies are not visible to other
  workers-topic subscribers.
- Persistent rate-limit buckets (SQLite or shared with storage).
- Attachment support (recent log buffer, screenshot capture).
- Two-way replies — the worker writes a comment-back path so a
  maintainer's GitHub reply lands in the reporter's UI.
- Per-server feedback workers and per-server routing.
- A "Send feedback" command-palette entry and a "?" top-bar shortcut
  once the form is proven.

## Open questions

None at spec time. Brainstorming resolved:

- Delivery target → GitHub issues via a project-run worker.
- Transport → existing worker gossip request/response pathway (not a
  new ALPN).
- UI entry point → Settings page only, in v1.
- Rate limit → 5 / hour / peer.
- Anonymity → signed by the user's identity, peer ID included in the
  issue as a pseudonymous handle; no display name.
