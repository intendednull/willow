# Long-Running Soak Swarm — Design

**Status:** Draft (brainstorm → spec). Implementation plan TBD.
**Branch:** `claude/long-running-stability-tests-ofy1S`
**Related issues:**
[#392](https://github.com/intendednull/willow/issues/392) (declarative provisioning),
[#393](https://github.com/intendednull/willow/issues/393) (version upgrade protocols).

## 1. Goal, Non-Goals, and What This Catches

### Goal

A permanently-running synthetic user community that exercises Willow under
realistic, unscripted, long-lived load. Output is two things, equally:

1. **Hardening of the app itself** — surface leaks, drift, race conditions,
   reload-state bugs, and emergent UX problems that no scripted test will find.
2. **Hardening of debugging tooling** — every bug surfaced makes the invariant
   checker, log schema, and reporting pipeline sharper. The detection layer is
   a first-class deliverable, not glue.

### Non-goals

- Not a replacement for any existing test tier. State / client / browser /
  Playwright tests stay where they are. This is a new tier *above* Playwright,
  oriented at time and emergence rather than coverage.
- Not a load test or benchmark. We don't care about throughput numbers; we
  care about whether things break.
- Not a CI gate. It runs continuously; merges don't wait on it. Findings flow
  back as GitHub issues against `main`.
- Not a security/abuse test. Bots cooperate; we're not adversarially fuzzing
  the protocol.

### What "stable over time and stable over usage" actually means here

Concrete classes of bug this should catch:

- **Resource accumulation** — memory, event log, message store, storage DB,
  indexes growing unbounded over weeks.
- **State drift** — peers' `StateHash` silently diverges due to a merge,
  dedup, ordering, or HLC bug.
- **Reload state corruption** — peer goes offline mid-something (key
  rotation, channel creation, partial sync), comes back, ends up broken.
- **Cross-session continuity** — joining peer reconstructs grove state
  correctly after the relay has accumulated 10k+ events.
- **Tail behaviors of the gossip layer** — events that take an unusually long
  path to converge, or never converge.
- **Epoch / key rotation correctness** — encryption keeps working across many
  rotations, including ones interleaved with other events.
- **Cumulative UX problems** — visible in the agent-browser canary phase.
- **Cross-grove resource isolation** on the shared prod relay (the swarm
  doubles as a continuous validation that grove isolation holds under load).

### Where this fits in the test tier hierarchy

State < client < browser < Playwright < **soak swarm** (new). Each tier above
is more expensive and less reproducible; the soak swarm is the most expensive
and least reproducible — it sits above Playwright because it tests *emergence
over time*, which lower tiers can't.

## 2. Architecture

```
┌────────────────────────── TEST VPS (small, e.g. $5/mo Linode) ─────────────────────────┐
│                                                                                         │
│   ┌──────────────────┐   ┌──────────────────────┐                                      │
│   │   swarm-runner   │   │  invariant-checker   │                                      │
│   │                  │   │  (observer +         │                                      │
│   │  ┌────────────┐  │   │   chaos-driver)      │                                      │
│   │  │ bot N task │──┼───┼─→ cross-peer hash    │                                      │
│   │  │  persona   │  │   │   comparison         │                                      │
│   │  │  Haiku loop│  │   │ - msg delivery audit │                                      │
│   │  │  ClientHnd │  │   │ - memory/event       │                                      │
│   │  └────────────┘  │   │   growth trend       │                                      │
│   │      × ~12       │   │ - panic scraping     │                                      │
│   │                  │   │ - own ClientHandle   │                                      │
│   └────────┬─────────┘   └─────────┬────────────┘                                      │
│            │                       │                                                    │
│            ▼                       ▼                                                    │
│       ┌──────────────────────────────────┐         ┌──────────────────────────────┐    │
│       │  /var/log/willow-soak/*.jsonl    │◀────────│  opus-reviewer (2× / day)    │    │
│       │  - bot.events                    │         │  - reads recent JSONL        │    │
│       │  - bot.actions                   │         │  - dedupes / clusters        │    │
│       │  - bot.self-reports              │         │  - files GitHub issues       │    │
│       │  - invariant.findings            │─────────│    via mcp__github__         │    │
│       │  - process.metrics               │         │  - writes own log entries    │    │
│       │  - opus.decisions                │         └──────────────┬───────────────┘    │
│       │  - critical-events               │                        │                    │
│       └──────────────┬───────────────────┘                        ▼                    │
│                      │                                  ┌──────────────────┐          │
│                      └─────────────────────────────────▶│  digest-writer   │          │
│                                            (weekly cron) │  → PR to repo    │          │
│                                                          └──────────────────┘          │
└─────────────────────────────────────────┬───────────────────────────────────────────────┘
                                          │  (iroh QUIC over network)
                                          ▼
                ┌──────────────────────────────────────────────┐
                │  PROD VPS — relay + storage worker (existing)│
                │  Bot grove is one of N groves on shared infra│
                └──────────────────────────────────────────────┘
```

### Components — test VPS only; prod VPS is unchanged

1. **`swarm-runner`** — single Rust binary. Holds N (~12) `willow_client::ClientHandle`
   instances in-process. Each bot is an async task running an event-driven
   loop: subscribes to its peer's event stream, on relevant events (mention,
   DM, message in watched channel) calls Anthropic Haiku with `system_prompt
   + persona + recent context + tool defs`, tool calls translate to
   `ClientHandle` method calls in the same process. Per-bot timer fires at
   the persona's reflection cadence with a low-token "anything you want to
   do?" prompt. Tool definitions are reused from the `willow-agent` crate
   (factored to expose tool schema + handlers as a library; the MCP server
   becomes one consumer, swarm-runner becomes another).

2. **`invariant-checker`** — separate Rust binary, two sub-roles in one
   process for log-writer/ClientHandle sharing:
   - **observer** — passive, runs the check catalog (Section 4) on schedule,
     writes `invariant.findings` records.
   - **chaos-driver** — actively perturbs the system on schedule (controlled
     restarts, fresh-peer joins). Logs every perturbation so observer
     anomalies can be attributed correctly (`concurrent_chaos` field).

   Owns its own `ClientHandle` with `SyncProvider` permission. Independent
   process from `swarm-runner` so a bot crash doesn't blind the checker.

3. **Log layer** — JSONL on disk under `/var/log/willow-soak/`, rotated
   daily, schema-versioned. Single source of truth for everything downstream.
   Streams: `bot.events`, `bot.actions`, `bot.self-reports`,
   `invariant.findings`, `process.metrics`, `opus.decisions`,
   `critical-events`.

4. **`opus-reviewer`** — long-running service with an internal scheduler
   (default 06:00 and 18:00 UTC; configurable). On each tick: reads JSONL
   from `(watermark - safety_overlap)` to `now`, lists open `soak`-labeled
   GitHub issues, calls Anthropic Opus with the slice + open issues + a
   triage prompt + GitHub MCP tools, lets Opus decide what (if anything) to
   file. Persists watermark only after a successful run, so coverage gaps
   are impossible by construction.

5. **`digest-writer`** — long-running service with an internal scheduler
   (Mondays 06:00 UTC). Assembles a markdown digest from the previous
   week's JSONL into `docs/reports/soak/YYYY-WW.md`, opens a PR.

6. **`critical-webhook`** *(optional)* — long-running, watches
   `critical-events.jsonl`, posts to a configured webhook (Slack/Discord) on
   `severity:critical` events. Real-time paging without waiting for the
   reviewer's next tick.

### Architectural boundaries

- **Bots have stable, on-disk identities.** Each persona's Ed25519 key
  persists in `/var/lib/willow-soak/keys/<persona>.key`. The grove's owner
  is a dedicated `operator` persona that bootstraps on first run.
  "Long-running session" only means something if peer IDs survive restarts.
- **Test VPS keeps no grove state.** The grove's events live on the prod
  storage worker (already does this for production groves). Test VPS only
  owns: bot keys, bot HLC clocks, logs, watermarks.
- **Failure isolation.** Each component is a separate Docker service with
  its own restart policy. A bot crash doesn't blind the checker; an Opus
  outage doesn't stop the swarm.
- **Single writer per log file.** Each component appends to its own JSONL
  files; opus-reviewer is the only consumer that reads across files.
- **Shared prod relay is a feature, not a hazard.** If a bot grove can
  degrade an unrelated user's grove via the shared relay, that's a
  resource-isolation bug the swarm has just usefully discovered.

## 3. Agent Loop & Personas

### Persona definition

Personas are TOML files under `/etc/willow-soak/personas/<name>.toml`,
checked into the repo so the cast is reproducible across restarts and
visible in code review:

```toml
name = "alice"
display_name = "Alice"
system_prompt = """ <static persona prose: traits, interests, voice> """
seed_permissions = ["ManageChannels", "SendMessages", "CreateInvite"]
event_triggers   = ["mention", "dm", "message_in:#events", "channel_created"]
reflection_interval_minutes = 60
max_tool_calls_per_tick = 3
```

### Initial cast (12 total)

Vary along axes that produce different bug-finding behavior:

| Persona | Role |
|---|---|
| `operator` | Grove owner. Bootstraps grove, grants roles. *Not a Haiku loop — pure scripted admin.* |
| `alice` | Organizer; creates channels, invites people, sets topics. (`ManageChannels`) |
| `bob` | Chatterbox; many small messages, reactions, frequent DMs. |
| `carol` | Lurker; mostly replies, rarely initiates. |
| `dave` | Admin-leaning; grants/revokes roles, kicks misbehaving bots. (`Administrator`) |
| `eve` | DM-heavy; one-on-one conversations across the cast. |
| `frank` | Joiner/leaver; periodically leaves and rejoins to exercise re-sync. |
| `grace`, `heidi`, `ivan`, `judy`, `mallory` | Generalists with personality variation. |

Eleven Haiku loops + one scripted operator. Adjustable via TOML.

### Loop shape (per bot)

```rust
loop {
    select! {
        event = client.events().next() => {
            if matches(persona, event) { tick(Trigger::Event(event)).await }
        }
        _ = sleep_until_next_reflection() => tick(Trigger::Reflection).await,
    }
}

async fn tick(trigger) {
    let prompt = PromptBundle {
        cached: [persona.system_prompt, tool_defs, grove_structure_snapshot],
        dynamic: [recent_events_window, trigger_context],
    };
    let response = haiku.tool_use(prompt).await;
    for call in response.tool_calls.take(persona.max_tool_calls_per_tick) {
        let result = execute_via_client_handle(call);
        log_jsonl(persona, trigger, call, result);
    }
}
```

### Why this shape works

- **Prompt caching is the cost story.** `system_prompt + tool_defs +
  grove_structure` is stable across ticks and gets cached — typically 80–95%
  of input tokens hit the cache. Without this, the test is unaffordable;
  with it, it's cheap.
- **Bounded action per tick** prevents a confused bot from spamming 50 tool
  calls. Small N (≤3) per tick.
- **Triggers are explicit.** A bot doesn't act unless mentioned, DMed, sees
  activity in a watched channel, or hits its reflection timer. Quiet groves
  stay cheap.
- **Reflection prevents stagnation.** Without the timer, a quiet grove stays
  quiet forever. Hourly low-token reflection is what produces "agents
  creating channels at their own whim."

### Tool surface

Reuses `willow-agent`'s existing MCP tool set (send message, list channels,
send DM, react, create channel, grant role, etc.). The required refactor:
factor `crates/agent/src/tools.rs` so tool defs + handlers are usable as a
library; the MCP server becomes one consumer, swarm-runner becomes another.
This is a Phase 0 prerequisite (Section 7).

### Cost order-of-magnitude

~12 bots, ~200–400 ticks/day total, with prompt caching: roughly **$1–5/day**
in Haiku spend at current pricing, plus the daily Opus reviews (~$0.50–2 per
run × 2/day). A few hundred dollars per quarter. Bounded by a hard daily cap
in `soak.toml`; if exceeded, swarm pauses Haiku calls (checker keeps running).

### Tradeoffs / runners-up

- **Time-driven loop only** (every N minutes per bot) — rejected: predictable
  cost but bursty, unnatural, and wastes tokens during quiet periods.
- **Event-driven only** (no reflection) — rejected: quiet groves stagnate
  forever; bots that aren't talked to never act.
- **Persona + planning loop with persistent memory** — rejected for now: most
  realistic but highest design complexity and highest token cost. Could
  evolve toward this in Phase 4 for specific bots.

## 4. Invariant Check Catalog

This is the section the whole project hinges on. Two sub-roles in
`invariant-checker`, separated for cleanliness: **observer** (passive, runs
checks) and **chaos-driver** (actively perturbs on schedule, logs every
perturbation as `concurrent_chaos`).

All cadences below are **defaults**; every value is overridable in
`/etc/willow-soak/soak.toml` (Section 6).

### Check catalog

| Check | Cadence | Description | Severity rules |
|---|---|---|---|
| **state-hash agreement** | every 30s | Pull `StateHash` from each bot's `ClientHandle`. All peers at equal height should agree. | Disagree <60s = `info`. Disagree 60s–5min = `warning`. Disagree >5min OR never resolves = `critical`. |
| **per-channel event-count parity** | every 5min | For each channel, every member should see the same count of events. | Mismatch persisting two windows = `warning`. Persisting an hour = `critical`. |
| **message-delivery audit** | every 5min | For each `send` in the last window, verify it appears on every member's local store within N seconds (default N=30). | Any send not delivered = `warning`. Sustained drop rate >0% = `critical`. |
| **panic / fatal log scrape** | every 30s | Tail journald (or container logs) for `panic`, `FATAL`, `unwrap` on `swarm-runner`, `invariant-checker`, the bot grove's relay channel, and storage-worker logs (filtered to bot grove's group_id). | Any match = `critical`, immediate. |
| **process liveness** | every 30s | Each bot task and the checker itself responds to a ping. | Missed ping >2 windows = `critical`. |
| **resource trends** | every 1h | Linear regression over the last 7 days of: bot-task RSS, swarm-runner total RSS, on-disk event-log bytes (storage worker, scoped to bot grove), search index size. | Slope > threshold (default >2%/day not explained by activity) = `warning`. Slope sustained 14 days = `critical`. |
| **convergence-time distribution** | every 1h | Histogram of "time from divergence detected → all peers agree." | p99 regression vs 7-day baseline = `warning`. |
| **key-rotation correctness** | every 1h | After every epoch rotation event in the window, verify the next N messages decrypted on every member. | Any decryption failure following a rotation = `critical`. |
| **disk-utilization** | every 5min | `/var/log` and `/var/lib` utilization. | 70% / 85% / 95% thresholds → `info` / `warning` / `critical`. |
| **cross-session continuity** | daily | Chaos-driver spawns a fresh peer (new keypair, given an invite). After sync settles, observer verifies its derived state hash matches established peers'. | Mismatch = `critical`. |
| **restart-survival** | every 4h | Chaos-driver picks a random bot, kills its task, restarts it, waits for sync. Observer verifies pre-restart hash == post-resync hash. | Mismatch = `critical`. State unrecoverable = `critical`. |
| **cross-grove isolation (best-effort)** | every 1h | Bot grove's CPU/bandwidth/memory share on the prod relay should not grow as a fraction of total when bot activity is steady. | Best-effort signal pending per-grove relay metrics. Unbounded growth = `warning`. |

### Finding schema

Every check that fires writes one record to
`/var/log/willow-soak/invariant.findings.jsonl`:

```json
{
  "schema_version": 1,
  "ts": "2026-04-26T12:34:56Z",
  "kind": "invariant.finding",
  "check": "state_hash_divergence",
  "severity": "critical",
  "first_observed": "2026-04-26T12:29:11Z",
  "duration_ms": 345000,
  "evidence": {
    "peers": [
      {"persona": "alice", "peer_id": "...", "hash": "0xabc...", "height": 4823},
      {"persona": "bob",   "peer_id": "...", "hash": "0xdef...", "height": 4823}
    ]
  },
  "auto_resolved": false,
  "context_pointers": [
    {"file": "bot.events.jsonl",      "range": "2026-04-26T12:25:00Z..12:35:00Z"},
    {"file": "bot.actions.jsonl",     "range": "2026-04-26T12:25:00Z..12:35:00Z"},
    {"file": "process.metrics.jsonl", "range": "2026-04-26T12:00:00Z..12:35:00Z"}
  ],
  "concurrent_chaos": null
}
```

`context_pointers` are the link the Opus reviewer follows to pull surrounding
events without us pre-computing what's relevant. `concurrent_chaos` is
non-null when a chaos-driver action overlapped, so spurious anomalies caused
by deliberate perturbation can be attributed correctly.

### Failure-attribution discipline

The hardest part of soak testing is telling real bugs from test-fixture
bugs. Three guardrails:

1. **The checker checks itself.** Every check has a self-test that runs at
   startup against synthetic state to confirm it can detect a known
   violation. If a self-test ever fails, the checker refuses to run and
   emits a `critical` (the test fixture is broken before we trust its
   output).
2. **Chaos events are first-class log entries.** Anomalies during a chaos
   window get a non-null `concurrent_chaos` pointer; Opus is instructed to
   weight those down unless the anomaly *was the very thing being chaosed*.
3. **No anomaly is silently suppressed.** "Auto-resolved" findings still
   get logged at lower severity. The pattern of self-healing matters — five
   auto-resolves in a day on the same check is itself a finding.

### Deliberately deferred

- **Performance/throughput regression.** Out of scope per Section 1.
- **Network partition simulation.** Real partitioning needs iroh-level
  cooperation we don't have a clean API for yet. Controlled restarts are
  the accessible proxy; revisit when iroh exposes one.
- **Adversarial bot behavior.** Bots cooperate. Byzantine-resistance
  testing is its own project.
- **UI rendering checks.** Land in the agent-browser canary phase
  (Phase 3, Section 7), not here.

## 5. Reporting Pipeline

### opus-reviewer

Long-running service with an internal scheduler. Default tick: **2× / day**
(06:00 and 18:00 UTC). Configurable in `soak.toml`.

On each tick:

1. Reads JSONL from `(opus.watermark.last_reviewed_ts - safety_overlap)` to
   `now`. Default `safety_overlap = 1h`. Persists watermark only after
   successful run, so coverage is gap-free by construction.
2. Lists open `soak`-labeled GitHub issues — these are the *known
   fingerprint*.
3. Calls Anthropic Opus with: log slice + open issues + triage prompt +
   GitHub MCP tools (`add_issue_comment`, `issue_write`, `search_issues`).
4. Lets Opus decide what to do per finding cluster.
5. Logs every Opus decision to `opus.decisions.jsonl` — including
   "no action and why."

### Triage rubric (in Opus's system prompt)

A finding gets a **new** GitHub issue only if all hold:

1. Severity is `critical`, OR same `warning` check has fired ≥3 times in
   the last 24h.
2. There is no existing open `soak` issue covering the same `check` with
   overlapping evidence (search by check name + check-specific keys, e.g.
   divergent peer pair).
3. The evidence is concrete enough that an engineer reading the issue could
   reproduce or investigate (specific event IDs, peer IDs, state hashes,
   log pointers).

Otherwise: comment on the existing issue with the new occurrence (counts go
up, log pointers extend), or do nothing (logged as "below threshold").

### Issue body template

```
**Check:** state_hash_divergence  **Severity:** critical
**First observed:** 2026-04-26T12:29:11Z
**Duration:** 5m45s, did not auto-resolve

**Evidence**
- alice (peer 0x...) hash 0xabc... at height 4823
- bob   (peer 0x...) hash 0xdef... at height 4823
- divergent event range: 4801–4823

**Reproduction context**
- swarm-runner commit: <sha>
- relay commit:        <sha>
- last chaos event:    none in window

**Log pointers** (test VPS, /var/log/willow-soak/)
- bot.events.jsonl:  2026-04-26T12:25Z..12:35Z
- bot.actions.jsonl: 2026-04-26T12:25Z..12:35Z

**Hypothesis** (Opus, clearly labeled as hypothesis): <brief, hedged take>

---
Auto-filed by opus-reviewer. Severity-1 incidents get paged out-of-band via
critical-webhook (if configured); lower severities get logged here.
```

Labels applied: `soak`, plus per-check label (`state-divergence`, `memory`,
`delivery`, `key-rotation`, `panic`, `restart-survival`, …), plus
`severity:critical` | `severity:warning`.

### Auditing the auditor

Every Opus invocation logs the prompt slice, the model output, and every
tool call to `opus.decisions.jsonl`. If Opus files a garbage issue, we see
exactly why. The weekly digest includes an "opus-reviewer self-audit"
section with: issues filed, comments added, no-action counts, and any Opus
decisions a human reviewer flagged in the previous week (via a
`bad-triage` label on a soak issue — Opus reads those next run as
negative-context for the rubric).

### Don't ship raw logs to Opus

opus-reviewer's input per run is sparse:

- All `invariant.findings` in window (KB-scale).
- All `bot.self-reports` in window (KB-scale).
- Summary stats from the bulky streams (event count, action count, panic
  count, RSS trend, top channels by activity).
- A grep-like log-range tool Opus can call to pull specific slices when
  investigating a finding.

Daily Opus invocation reads tens of KB from disk and ships a few thousand
input tokens (mostly cached system+rubric prompt). Cost order:
**single-digit cents per run.**

### Critical paging

For `severity:critical` invariant violations specifically, the checker
writes an extra record to `critical-events.jsonl` and `critical-webhook`
(if configured) posts to a webhook. opus-reviewer still triages and files
the GitHub issue on its normal cadence, but humans are notified in real
time without needing the reviewer to run.

### digest-writer

Long-running service with internal scheduler. Default: **Mondays 06:00
UTC**. Reads the last 7 days of JSONL, writes
`docs/reports/soak/YYYY-WW.md`, opens a PR. Sections:

- Counts (findings by check × severity, opus actions, chaos events).
- Resource trends (ASCII sparklines of RSS, event log size, storage DB
  size — text-renderable, no graphics deps).
- Top open soak issues (oldest, most recurring).
- New vs. resolved soak issues this week.
- opus-reviewer self-audit.
- Notable patterns Opus surfaced (clustering across checks).
- Daily-cost summary (Anthropic spend by service).

Two outputs, one source of truth: **GitHub issues are the call-to-action
layer**; the **weekly digest is the trend / pattern layer**. The same
JSONL feeds both.

### Log volume & retention

| Stream | Per-day estimate | Retention |
|---|---|---|
| `bot.events`        | ~3 MB    | hot 7d, gz 90d, then weekly-aggregate then drop raw |
| `bot.actions`       | ~1 MB    | hot 7d, gz 90d, then weekly-aggregate then drop raw |
| `bot.self-reports`  | <50 KB   | hot 7d, gz 90d, then weekly-aggregate then drop raw |
| `invariant.findings`| <50 KB   | **kept indefinitely** |
| `process.metrics`   | ~600 KB  | hot 7d, gz 90d, then weekly-aggregate then drop raw |
| `opus.decisions`    | ~100 KB  | **kept indefinitely** |
| `critical-events`   | <10 KB   | **kept indefinitely** |

**~5 MB/day raw** total, ~1–2 MB compressed. 6 months ≈ 300–600 MB. Fits
comfortably on a small VPS.

## 6. Infrastructure & Deployment

### Test VPS sizing

Small Linode (or equivalent): **2 GB RAM, 1 vCPU, 50 GB disk, Ubuntu 24.04
LTS.** Sized for ~12 in-process bot ClientHandles plus a checker
ClientHandle, with comfortable headroom on memory and disk. Network
bandwidth is the only thing worth watching — bots talk to the prod relay
over QUIC.

### Runtime: Docker Compose with internal schedulers

Adopted over rsync-binaries-to-systemd for reproducibility. Adopted over
NixOS / Pulumi / Ansible for incremental fit with the project's existing
`docker-up`/`docker-down` and `crates/relay/Dockerfile` patterns. See
[#392](https://github.com/intendednull/willow/issues/392) for the broader
declarative-provisioning conversation deferred until after this lands.

- Single `crates/soak/Dockerfile` with a Rust multi-stage build producing
  all four binaries (`swarm-runner`, `invariant-checker`, `opus-reviewer`,
  `digest-writer`, plus optional `critical-webhook`).
- `compose.yaml` declares one service per binary.
- **Long-running services with internal schedulers**, not cron-in-container.
  `opus-reviewer` sleeps until next 06:00/18:00 UTC, runs, sleeps.
  `digest-writer` sleeps until next Monday. ~20 lines of Rust each.
- Images tagged with `${GIT_SHA}` in CI, pushed to GitHub Container Registry
  (ghcr.io). Free for the repo, integrates with GitHub Actions.
- Deploy = `docker compose pull && docker compose up -d`. Rollback = pin a
  previous SHA tag.
- Pinning everywhere: `rust-toolchain.toml` (project already has this),
  `Cargo.lock --locked`, base image pinned by digest
  (`debian:trixie-slim@sha256:...`).
- Volumes: `/var/lib/willow-soak` and `/var/log/willow-soak` are bind-mounts
  on the host so identities, watermarks, and logs survive image swaps.
- Log rotation: Docker JSON-file driver with `--log-opt max-size` and
  `max-file`. JSONL streams are app-managed (the app rotates them itself
  for retention-policy reasons).

### Service inventory

| Service | Type | Restart policy | Schedule |
|---|---|---|---|
| `swarm-runner` | long-running | `unless-stopped` | continuous |
| `invariant-checker` | long-running | `unless-stopped` | continuous (observer + chaos in one process) |
| `opus-reviewer` | long-running | `unless-stopped` | internal scheduler — 06:00, 18:00 UTC |
| `digest-writer` | long-running | `unless-stopped` | internal scheduler — Mon 06:00 UTC |
| `critical-webhook` *(optional)* | long-running | `unless-stopped` | watches `critical-events.jsonl`, posts on append |

### Filesystem layout

```
/etc/willow-soak/
  soak.toml                    # all cadences/thresholds (in-repo, deployed on update)
  personas/                    # persona TOMLs (in-repo)
  secrets.env                  # ANTHROPIC_API_KEY, GITHUB_TOKEN, CRITICAL_WEBHOOK_URL
                               # provisioned out-of-band, never committed
/var/lib/willow-soak/
  keys/                        # operator + persona Ed25519 keys, mode 0400
  state/                       # ClientHandle local state (event store, etc.)
  opus.watermark               # JSON {last_reviewed_ts, last_run_ts}
  digest.watermark
  bootstrap.lock               # presence = grove already bootstrapped
/var/log/willow-soak/
  *.jsonl                      # live streams
  archive/                     # rotated, gzipped
```

All paths are **bind-mounted** into the appropriate Docker services.

### Identities

- **`operator`** is the grove owner. Its key is the single most critical
  secret on the VPS — losing it bricks the grove permanently.
- **Personas** each have their own keypair, generated on first run,
  persisted indefinitely. Bot peer IDs stay stable across restarts (this is
  what makes "long-running session" actually mean something).
- **Backup discipline:** `just soak-backup` (added recipe) tars
  `/var/lib/willow-soak/keys/`, encrypts with age, uploads to a backup
  bucket. Run from a maintainer's laptop, not the VPS itself. Documented in
  `docs/runbooks/soak.md`.

### Bootstrapping the grove

Idempotent first-run logic in `swarm-runner`:

1. If `bootstrap.lock` exists, skip and start normal loop.
2. Else: generate operator key (or fail if a key was provisioned from
   backup), generate persona keys, create grove via operator's
   `ClientHandle`, grant `seed_permissions` per persona TOML, generate
   invites, have each persona join, write `bootstrap.lock`.
3. Bootstrap is a logged event (`process.metrics.jsonl`,
   `kind=grove.bootstrap`).

If the operator key is restored from backup but `bootstrap.lock` is
missing, the system detects the existing grove via the storage worker's
history sync and writes the lock without re-bootstrapping.

### Configuration: `soak.toml`

Single file, all tunables, defaults match Sections 3–5:

- Per-persona reflection intervals (override the persona TOMLs if needed).
- Per-check cadences (overrides Section 4 defaults).
- Chaos cadences (overrides Section 4 defaults).
- opus-reviewer cadence and watermark safety overlap.
- Severity escalation thresholds.
- Log retention policy.
- Daily Anthropic spend cap (hard cap; swarm pauses Haiku calls if exceeded,
  checker keeps running).

### Deployment recipes (added)

| Recipe | What it does |
|---|---|
| `just soak-build` | CI multi-stage Docker build, tag with `${GIT_SHA}`, push to ghcr.io |
| `just soak-deploy` | SSH to VPS, `docker compose pull && docker compose up -d` |
| `just soak-status` | SSH + `docker compose ps`, plus latest entries from each JSONL stream |
| `just soak-logs <stream>` | SSH + `tail -f` on one of the JSONL streams |
| `just soak-bootstrap` | Manual/recovery bootstrap (one-off; normally automatic on first start) |
| `just soak-backup` | Pull encrypted key backup from VPS to local |
| `just soak-restore` | Push restored keys back to VPS (rare, recovery only) |

CI does not run any of this. The test VPS is the only environment.

### Observability of the test fixture itself

(Distinct from invariant findings, which are about *Willow*.)

- Docker logs go to journald via the JSON-file driver.
- `process.metrics.jsonl` records start/stop events, restart counts,
  bootstrap events, chaos events.
- Disk-fill check (Section 4) + a basic CPU/memory sanity check on the VPS
  cover "is the test fixture itself healthy?"

### What's NOT in scope here (deferred)

- Multi-VPS / regional distribution.
- HA failover (single VPS is fine; keys are backed up off-box).
- Declarative provisioning of the VPS itself (Ansible/Pulumi/NixOS) —
  tracked in [#392](https://github.com/intendednull/willow/issues/392).
- Migration of `just deploy` (prod) to the same Compose pattern — also
  [#392](https://github.com/intendednull/willow/issues/392).

## 7. Phased Rollout

Phases run **sequentially**, not in parallel. Each has explicit exit
criteria so we don't paper over flakiness by moving on.

### Phase 0 — `willow-agent` tool refactor *(prerequisite)*

- Factor `crates/agent/src/tools.rs` so tool definitions and handlers are
  usable as a library.
- The existing MCP server becomes one consumer; the swarm-runner will
  become the second.
- No behavior change.
- **Exit:** existing `willow-agent` tests pass unchanged; library API has
  at least two callers (MCP + a smoke test).
- **Effort:** ~2–3 days.
- **Lands in its own PR** before any swarm code so the diff stays
  reviewable.

### Phase 1 — MVP swarm: 3 bots, no chaos, manual review

- `operator` + `alice`, `bob`, `carol` only. Simple personas, event-driven
  loop, reflection at 4h.
- Tool subset: send message, list channels, send DM, react. (Not yet:
  create channel, grant role.)
- Invariant checker: **state-hash agreement, panic scrape, process
  liveness, disk fill** only. Other checks deferred.
- Logging stack fully in place (JSONL streams + rotation + retention).
- **No `opus-reviewer` yet** — humans tail logs for the first week.
  Calibrating "what does good look like" before turning Opus loose.
- Deployed via Docker Compose on the test VPS, connected to prod
  relay/storage.
- **Exit:** swarm has run for 7 days uninterrupted; ≥3 invariant checks
  have fired and been audited; bot identities have survived a service
  restart and re-converged correctly; one round of "real findings vs.
  test-fixture noise" review done.
- **Effort:** ~1–2 weeks dev + 1 week burn-in.

### Phase 2 — Full swarm + full checks + opus-reviewer

- Expand to all 11 personas (full cast). Per-persona reflection cadences.
- Tool surface widens: create channel, grant/revoke role, kick, invite.
  Grove can structurally evolve.
- Full check catalog from Section 4 turned on.
- Chaos-driver enabled.
- `opus-reviewer` runs 2× / day. `digest-writer` weekly.
  `critical-webhook` optional.
- **Exit:** system runs 14 days uninterrupted; ≥1 real bug has been
  found, triaged through Opus, filed as a GitHub issue, and fixed; weekly
  digest has been generated and is readable.
- **Effort:** ~2–3 weeks dev + 2 weeks burn-in.

### Phase 3 — Browser canaries (Option C end state)

- 1–2 personas (e.g. `bob`, `eve`) drive the actual web UI via
  agent-browser instead of MCP.
- Same persona prompts, different tool surface (DOM actions instead of
  `ClientHandle` calls).
- Add UI-specific invariants: console errors, unhandled rejections, render
  hangs, IndexedDB growth, service-worker health.
- **Exit:** at least one UI-only bug surfaced and filed.
- **Effort:** ~1–2 weeks dev. Begun only after Phase 2 burn-in is clean.
  Browser tooling can be prototyped in parallel with Phase 2 burn-in but
  enabled only after.

### Phase 4 — Continuous hardening *(no end)*

- Every real bug found feeds back as a new check or a tightened threshold.
- Persona TOMLs evolve (new behaviors, new mischief).
- Retention / cost / cadence tuned based on what actually fires.
- Steady state. Never "done."

## 8. Out of Scope, Risks, Open Questions

### Out of scope (recap)

- Performance / throughput benchmarking. We're catching breakage, not
  measuring speed.
- Adversarial / Byzantine bot behavior. Bots cooperate.
- Network-level chaos (partition simulation, packet loss). Deferred until
  iroh exposes a cleaner API; controlled restarts are the accessible
  proxy.
- Multi-grove and multi-region testing. One grove on one test VPS for
  now.
- Changes to prod's `just deploy` pattern. Tracked in
  [#392](https://github.com/intendednull/willow/issues/392).
- Security / abuse / spam testing. Different project.

### Risks

| Risk | Mitigation |
|---|---|
| **LLM cost overrun** (cache hit rate worse than estimated, Opus filing aggressively, Haiku price changes). | Daily cost tracker in the digest. Hard daily cap in `soak.toml` — if exceeded, swarm pauses Haiku calls (checker keeps running). |
| **Test-fixture bugs masquerading as Willow bugs** (worst failure mode — engineers chase phantoms). | Checker self-tests at startup, refuse to run if any fail. `concurrent_chaos` on every finding. Phase 1's manual-review burn-in. `bad-triage` label as feedback. |
| **Operator key loss** (catastrophic — bricks the grove). | `just soak-backup` runbook. Documented as a P0 operational responsibility. |
| **Opus filing low-quality issues at scale.** | Strict triage rubric. `bad-triage` label fed back as negative context next run. Weekly digest's "self-audit" section. |
| **Bot persona drift** toward uninteresting equilibria (bots stop doing anything novel). | Reflection prompt rotation; periodic persona-prompt review as part of weekly digest. |
| **Anthropic API outages** stalling the swarm. | Checker is independent of Anthropic. Bots back off and retry; outage windows are logged but not flagged as Willow bugs. |
| **Test VPS resource exhaustion** (RAM, disk, file handles). | Disk-fill check (Section 4). Memory limits per Docker service. Process-level metrics in `process.metrics.jsonl`. |
| **JSONL schema evolution** breaking opus-reviewer. | `schema_version` field on every record. Reviewer reads multiple schema versions; a schema bump is a coordinated change. |
| **Bot grove load affecting prod relay** — *not a risk to the soak test, but a real bug if it happens*. | Cross-grove isolation check (Section 4). If isolation breaks, this is the test working as designed. |

### Open questions (acknowledged, deferred)

- **Swarm size of 12** — gut number. Re-evaluate after Phase 2 burn-in
  based on findings rate. Could double if too quiet.
- **Mid-run client / protocol upgrades.** Until Willow has formal version
  upgrade and rolling-upgrade tooling, the soak grove is **wiped and
  re-bootstrapped on breaking protocol changes** (`bootstrap.lock`
  removal + key wipe + `docker compose up -d`). This is documented in the
  runbook. Tracked at
  [#393](https://github.com/intendednull/willow/issues/393); when that
  work lands, a soak-specific rolling-upgrade playbook becomes part of it.
- **Should `operator` ever be a Haiku loop too?** Currently scripted. An
  LLM operator would be more chaotic but could reproduce admin-level
  confusion bugs. Consider in Phase 4 as an optional persona type.

## 9. References

- `docs/specs/2026-04-21-e2e-test-architecture-design.md` — existing
  multi-tier test architecture this layers on top of.
- `docs/specs/2026-03-29-agentic-peer-api-design.md` — `willow-agent`
  MCP server prior art; the tools-as-library refactor in Phase 0 builds
  on this.
- `docs/specs/2026-04-26-state-management-model-design.md` — actor /
  state-management model that bot ClientHandles and the checker rely on.
- `docs/specs/2026-04-12-state-authority-and-mutations.md` — permission
  model the operator persona uses to grant `seed_permissions`.
- `crates/agent/` — the MCP server to be refactored in Phase 0.
- [#392](https://github.com/intendednull/willow/issues/392) — declarative
  provisioning + unified deployment pattern (deferred follow-up).
- [#393](https://github.com/intendednull/willow/issues/393) — version
  upgrade protocols + rolling-upgrade tooling (deferred follow-up).







