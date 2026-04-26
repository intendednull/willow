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
- **State drift** — peers' per-author DAG heads silently diverge or fail to
  converge due to a merge, dedup, ordering, or HLC bug. Detected via
  cross-peer comparison of `HeadsSummary` (`crates/state/src/sync.rs:22`),
  not a global state hash. (Earlier drafts of this spec assumed a single
  `StateHash` digest existed; the actual model is per-author Merkle DAG with
  `heads: HashMap<EndpointId, EventHash>` from `dag.rs:267`.)
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
│   │  │ bot N task │──┼───┼─→ HeadsSummary       │                                      │
│   │  │  persona   │  │   │   convergence        │                                      │
│   │  │  Haiku loop│  │   │ - msg delivery audit │                                      │
│   │  │ ClientHnd  │  │   │ - memory/event       │                                      │
│   │  │ (per-inst. │  │   │   growth trend       │                                      │
│   │  │  identity  │  │   │ - panic scraping     │                                      │
│   │  │  + dir)    │  │   │ - own ClientHandle   │                                      │
│   │  └────────────┘  │   │                      │                                      │
│   │      × ~12       │   │                      │                                      │
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

1. **`swarm-runner`** — single Rust binary. Holds N (~12)
   `willow_client::ClientHandle` instances in-process. Each bot is an async
   task running an event-driven loop: subscribes to its peer's event
   stream, on relevant events (mention, DM, message in watched channel)
   calls Anthropic Haiku with `system_prompt + persona + recent context +
   tool defs`, tool calls translate to `ClientHandle` method calls in the
   same process. Per-bot timer fires at the persona's reflection cadence
   with a low-token "anything you want to do?" prompt.

   **Multi-tenant `ClientHandle` is a Phase 0 prerequisite**, not a free
   property. As of this spec, `ClientHandle::new` resolves identity and
   data directory through process-global helpers (`load_identity()` in
   `crates/client/src/lib.rs`, `data_dir()` in `crates/client/src/storage.rs`),
   which makes 12 in-process instances impossible without changes — they
   would all clobber the same `identity.bin` and per-server event stores.
   Phase 0 adds explicit per-instance `identity` and `data_dir` parameters
   to `ClientHandle::new` (and through `ClientConfig`, `PersistenceActor`,
   and event-store paths). See §7 Phase 0 for the scope and effort.

   Tool definitions are reused from the `willow-agent` crate. `tools.rs` is
   already library-shaped (`WillowToolRouter::new`, `tool_list()`, `call()`
   are public); the gap is that the call entry point is rmcp-typed
   (`CallToolRequestParams`). Phase 0 adds a thin non-MCP entry point so
   swarm-runner can dispatch directly without re-encoding through rmcp.

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
- **Test VPS owns per-bot client state, not authoritative grove state.**
  The grove's archival history lives on the prod storage worker (as for
  any production grove). Each bot, however, persists its own
  materialized event store and `ClientHandle` state under its
  per-instance `data_dir` (via `PersistenceActor`,
  `crates/client/src/storage.rs`). Test VPS therefore owns: bot keys,
  per-bot event stores, per-bot HLC clocks, logs, watermarks. The bot
  event stores are themselves a soak target — the persistence layer is
  exercised at scale across 12 long-lived clients.
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

Reuses `willow-agent`'s existing tool set (send message, list channels,
send DM, react, create channel, grant role, etc.). `tools.rs` is already
library-shaped (`WillowToolRouter::new`, `tool_list()`, `call()` are
public — verified against `crates/agent/src/lib.rs`,
`crates/agent/src/tools.rs:276`); the public entry point is rmcp-typed
(`CallToolRequestParams`/`CallToolResult`). Phase 0 adds a thin non-MCP
dispatch so swarm-runner can call tools by name without re-encoding
through rmcp. This is **separate from and smaller than** the multi-tenant
`ClientHandle` refactor that is Phase 0's larger half.

### Cost order-of-magnitude

~12 bots, ~200–400 ticks/day total. Order-of-magnitude with prompt
caching: roughly **$1–5/day** Haiku spend at current pricing, plus 2× / day
Opus reviews (~$0.50–2 per run). A few hundred dollars per quarter.

**Caveat: prompt-cache TTL realism.** Anthropic's prompt cache has a
~5-minute default TTL. The hourly reflection trigger and event-driven
ticks during quiet periods routinely fall outside that window — every
"first tick after the cache expired" pays full input price on
`system_prompt + tool_defs + grove_structure_snapshot`. A bot that wakes
once per hour will essentially never hit cache. Realistic effective hit
rate is probably **30–60%, not 80–95%**, so the worst case is **2–5×
the order-of-magnitude above**. Mitigations:

- Use **1h cache TTL** (the longer Anthropic option) on the static
  prefix, not the 5-min default.
- Optionally: a low-cost cache-warming ping on a 4-minute schedule
  (cheaper than missing the cache repeatedly during active periods,
  noisier than just paying full price during quiet periods — defaults
  off, enabled by `soak.toml`).

Bounded by a hard daily cap in `soak.toml`; if exceeded, swarm pauses
Haiku calls (checker keeps running). Cap accounts for the realistic
not the optimistic estimate.

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

### Data-model alignment (read this before the table)

Willow's state is a **per-author Merkle DAG**, not a globally-hashed
materialized state. The cross-peer comparison primitive is
`HeadsSummary` (`crates/state/src/sync.rs:22`) — a per-author map of
`EndpointId → EventHash` derived from `Dag::heads_summary()`
(`crates/state/src/dag.rs:267`). There is no global `StateHash` and no
global `height`. "Convergence" therefore means: for each author, every
peer that has ingested an event holds the same `EventHash` at the same
position in that author's chain, and every peer's `HeadsSummary` is a
subset of the union of all peers' summaries (with a bounded lag).

A naive "do all peers' state hashes match?" check would either fire
constantly (peers always lag by a few events during gossip) or never
(any "they're catching up" excuse hides real bugs). The checks below
are framed against the actual model.

### Check catalog

| Check | Cadence | Description | Severity rules |
|---|---|---|---|
| **heads-summary convergence** | every 30s | Pull `HeadsSummary` from each bot's `ClientHandle::state_snapshot()`. **Per-author chain check:** for every author present in any peer's summary, every peer that has ingested up to seq `s` for that author must hold the same `EventHash` at every position `0..=s`. **Lag bound:** every peer's summary head must be at most `lag_bound_seconds` (default 30s) behind the union frontier; a sustained gap is treated as a separate `convergence-lag` finding (below). | Same-position-different-hash = `critical` immediately (this is true divergence, not lag). Lag >30s in a steady-traffic window = `warning`; lag >5min = `critical`. |
| **per-author event-content equality** | every 5min | For every `(author, seq)` tuple seen by ≥2 peers in the window, peers agree on the resulting `EventHash` and on the materialized effect (channel state, role state, etc.) after `apply()`. Failures are recorded with the divergent `EventHash`es and the (author, seq) tuple. | Any disagreement = `critical`. |
| **message-delivery audit** | every 5min | For each `send` in the last window: define **"intended recipients"** as the set of channel members in the *sender's* `ServerState` *at the moment immediately after the send event is applied* — this is the receiver-independent causal frontier the spec uses. For each recipient peer, verify the event appears in its local event store within `delivery_deadline` seconds (default 60s) of its system clock first observing the event. Recipients added/removed by concurrent membership events that race the send are excluded from this window's audit and re-evaluated next window. | Any send not delivered to an intended recipient still in the channel after `delivery_deadline` × 2 = `warning`. Sustained miss rate >0% over 1h = `critical`. |
| **panic log scrape (test-VPS scope only)** | every 30s | Tail Docker container logs (`docker logs --since`) for `swarm-runner`, `invariant-checker`, `opus-reviewer`, `digest-writer`, `critical-webhook`. **Pattern catalog (allowlist-driven, not substring):** matches lines that begin with `thread '...' panicked at `, lines with the literal `note: run with \`RUST_BACKTRACE=1\``, lines tagged `target=panic_hook`, and `tracing` events at `Level::ERROR` from non-allowlisted targets. **Allowlist** of expected `ERROR`/`WARN` patterns lives in `panic_allowlist.toml` (in-repo, reviewed) — `iroh_gossip::net::receiver lagged`, etc. **Out of scope:** prod relay and prod storage worker logs. Those services run on a different VPS and we have no log-shipping channel; spec §6 explicitly excludes log shipping from this work. Prod-side panics during the swarm's run are not caught by this check. | Any non-allowlisted panic-pattern hit = `critical`, immediate. Any non-allowlisted `Level::ERROR` = `warning`; ≥3 in 24h = `critical`. |
| **process liveness** | every 30s | Each bot task and the checker itself responds to a ping. | Missed ping >2 windows = `critical`. |
| **HLC monotonicity** | every 5min | For each peer, the per-peer HLC must be non-decreasing across all events the peer has emitted in the window (and across the persistence boundary if the peer was restarted). Detects a peer whose HLC went backwards after restart — exactly the "reload state corruption" §1 promises to catch. | Any decrease = `critical`. |
| **dedup positivity** | every 1h | The `apply()` path must hit its dedup branch (same event re-applied is a no-op) at least once per hour during normal gossip overlap. If dedup count stays at zero for an hour, the dedup logic itself is suspect (a regression that breaks dedup would silently re-apply events and only show up later as event-content disagreement). | Zero dedup hits in a 1h window of nonzero gossip volume = `warning`; sustained 4h = `critical`. |
| **resource trends** | every 1h, **disabled until day 14 of the run** | Linear regression over the last 7 days of: bot-task RSS, swarm-runner total RSS, per-bot on-disk event-store size, search index size. **Normalization:** slopes are computed *per event ingested*, not per wall-clock day, to separate organic activity-driven growth from leaks. | Per-event-ingested slope > threshold (default >0.5%/1k events) = `warning`. Sustained 14 days = `critical`. |
| **convergence-lag distribution** | every 1h, **disabled until day 14 of the run** | Histogram of per-event "time from peer A first sees event X → peer B first sees event X" across all ordered peer pairs. | p99 regression vs 7-day baseline = `warning`. Baseline window is the median of the previous full 7-day samples; not meaningful before day 14. |
| **key-rotation correctness** | every 1h | After every epoch rotation event in the window, verify the next N messages on the same channel decrypted on every member that has ingested both the rotation and the message. | Any decryption failure following a rotation = `critical`. |
| **cross-rotation re-decryption** | daily | After a peer has been offline across ≥2 epoch rotations and rejoins, verify it can read history encrypted under the older epochs (within whatever the protocol's stated retention window is). Detects "ratchet too aggressive, can't decrypt N-2" failure mode. | Cannot decrypt a message within the documented retention window = `critical`. |
| **disk-utilization** | every 5min | `/var/log` and `/var/lib` utilization on the test VPS. | 70% / 85% / 95% thresholds → `info` / `warning` / `critical`. |
| **cross-session continuity** | daily | Chaos-driver spawns a fresh peer (new keypair, given an invite). After sync settles, observer verifies the fresh peer's `HeadsSummary` is a superset of all known events for every author, and its derived `ServerState` is structurally equal to established peers'. | Any author missing from the fresh peer's summary after 5min = `critical`. Any structural difference in derived `ServerState` = `critical`. |
| **restart-survival** | every 4h | Chaos-driver picks a random bot, drops its task (which gracefully drops the `ClientHandle` and its actor runtime), then re-creates a fresh `ClientHandle` for that persona using the same persistent identity and `data_dir`. Once sync settles, observer verifies the post-resync `HeadsSummary` is a superset of the pre-restart `HeadsSummary` (no events lost) and that the post-resync materialized `ServerState` is structurally equal to a peer that did not restart. **Implementation detail:** with multi-tenant `ClientHandle` (Phase 0), this is an in-process operation in `swarm-runner`, driven by an admin-channel signal from the checker — no cross-process IPC needed. | Any pre-event missing post-resync = `critical`. Any structural state mismatch with a non-restarted peer = `critical`. |
| **cross-grove isolation (best-effort, disabled by default)** | every 1h | Bot grove's CPU/bandwidth/memory share on the prod relay should not grow as a fraction of total when bot activity is steady. | Best-effort signal pending per-grove relay metrics. **Disabled** in `soak.toml` until the relay exposes per-grove counters (out of scope for this spec). When enabled: unbounded growth = `warning`. |

### Finding schema

Every check that fires writes one record to
`/var/log/willow-soak/invariant.findings.jsonl`:

```json
{
  "schema_version": 2,
  "ts": "2026-04-26T12:34:56Z",
  "kind": "invariant.finding",
  "check": "heads_summary_divergence",
  "severity": "critical",
  "first_observed": "2026-04-26T12:29:11Z",
  "duration_ms": 345000,
  "fingerprint": "heads_summary_divergence:author=ed25519:abc123:seq=4823",
  "evidence": {
    "author": "ed25519:abc123...",
    "seq": 4823,
    "peers": [
      {"persona": "alice", "peer_id": "...", "event_hash_at_seq": "0xabc..."},
      {"persona": "bob",   "peer_id": "...", "event_hash_at_seq": "0xdef..."}
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

**`fingerprint`** is a **canonicalized, order-independent string** keyed
by check name and check-specific evidence keys (e.g. for
`heads_summary_divergence`: `check + author + seq`; for
`message_delivery_miss`: `check + send_event_hash`; for `panic`:
`check + container + first 80 chars of panic message`). Fingerprints
are **the deduplication key** for opus-reviewer (Section 5) — Opus
searches `soak`-labeled issues by fingerprint before deciding whether
to file or comment, so the same root bug surfacing across many evidence
permutations clusters into one issue.

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
   get logged at lower severity. The pattern of self-healing matters,
   so the observer emits a synthetic **`auto_resolve_pattern` finding**
   when ≥5 auto-resolves on the same fingerprint occur within 24h. That
   synthetic finding feeds the same triage rubric as any other. (Earlier
   draft asserted this without wiring it; the synthetic-finding mechanism
   is the wire.) Self-tests: `MemNetwork`-based for state-machine
   checks (state divergence, dedup, HLC); the multi-peer checks
   (delivery audit, convergence-lag, cross-rotation) self-test against
   a small in-process `MemNetwork` mesh (`crates/network/src/mem.rs`)
   with synthetic events that produce a known violation.

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
   `now`, **bounded to a maximum window** (default 48h) so that a
   prolonged outage does not produce a 5-day, multi-MB log slice on
   recovery. If the watermark is older than the cap, the reviewer emits a
   `coverage_gap` finding for the missing window and proceeds from
   `(now - max_window)`. Default `safety_overlap = 1h`.
2. Lists open `soak`-labeled GitHub issues — these are the *known
   fingerprints*. Cross-references against the new findings'
   `fingerprint` field (Section 4) for deduplication.
3. Calls Anthropic Opus with: log slice + open issues + triage prompt +
   constrained GitHub MCP tools (see "Hard rate limits" below).
4. Lets Opus decide what to do per finding cluster, **subject to the
   per-tick caps**.
5. Logs every Opus decision to `opus.decisions.jsonl` — including
   "no action and why."
6. **Idempotency on partial-success.** Every `mcp__github__issue_write`
   call from Opus is preceded by an issue-search by `fingerprint:` body
   substring; if a matching issue exists, Opus is required to comment
   instead of file. So a partial run (some issues filed, then crash)
   followed by a full re-run on the same window does not double-file.
   Watermark advance happens only on full success, but the idempotency
   check holds independently.

### Triage rubric (in Opus's system prompt)

A finding gets a **new** GitHub issue only if all hold:

1. Severity is `critical`, OR same `warning` fingerprint has fired ≥3
   times in the last 24h.
2. There is no existing open `soak` issue whose body contains
   `fingerprint:<this fingerprint>` (Opus runs `search_issues`
   `repo:intendednull/willow is:issue is:open label:soak
   "fingerprint:<value>"` first; if anything matches, comment instead).
3. The evidence is concrete enough that an engineer reading the issue
   could reproduce or investigate (specific event IDs, peer IDs,
   `(author, seq)` tuples, log pointers).

Otherwise: comment on the existing issue with the new occurrence (counts
go up, log pointers extend), or do nothing (logged as "below threshold").

**Hypotheses are restricted to a comment, not the issue body.** Empirical
weakness of "clearly labeled" hypotheses is that engineers anchor on them
anyway. The body contains evidence + log pointers only; Opus's hypothesis
goes in the first comment so that a human reviewer can dismiss it without
its phrasing dominating the issue's framing.

### Issue body template

```
**Check:** heads_summary_divergence  **Severity:** critical
**Fingerprint:** heads_summary_divergence:author=ed25519:abc123:seq=4823
**First observed:** 2026-04-26T12:29:11Z
**Duration:** 5m45s, did not auto-resolve

**Evidence**
- author:    ed25519:abc123...
- seq:       4823
- alice (peer 0x...) event_hash_at_seq: 0xabc...
- bob   (peer 0x...) event_hash_at_seq: 0xdef...
- divergent (author, seq) range: (ed25519:abc123, 4801..=4823)

**Reproduction context**
- swarm-runner image:  ghcr.io/.../soak:<sha>
- relay commit:        <sha>
- last chaos event:    none in window

**Log pointers** (test VPS, /var/log/willow-soak/)
- bot.events.jsonl:  2026-04-26T12:25Z..12:35Z
- bot.actions.jsonl: 2026-04-26T12:25Z..12:35Z

---
Auto-filed by opus-reviewer. Hypothesis (if any) is in the first comment,
not the body. Severity-1 incidents are paged out-of-band via
critical-webhook (if configured).
```

Labels applied: `soak`, plus per-check label (`state-divergence`, `memory`,
`delivery`, `key-rotation`, `panic`, `restart-survival`, …), plus
`severity:critical` | `severity:warning`.

### Hard rate limits and kill-switch on Opus's GitHub MCP path

The triage rubric is a soft constraint inside Opus's prompt. If Opus
regresses, hallucinates, or its self-test degrades, soft constraints
will not stop it from filing thousands of issues until GitHub's
5000/hr authenticated rate limit ends the spree (which is shared with
`digest-writer` PRs and any human use of the same token).

opus-reviewer therefore enforces hard limits **out-of-band**, in the
process around Opus rather than in the prompt:

- **Per-tick caps:** at most `max_new_issues_per_tick` (default **5**)
  new issues, `max_comments_per_tick` (default **20**) comments,
  `max_total_tool_calls_per_tick` (default **50**) GitHub MCP calls.
  Configurable in `soak.toml`.
- **Per-day caps:** `max_new_issues_per_day` (default **15**),
  `max_comments_per_day` (default **80**).
- **Pre-call gate:** every `mcp__github__issue_write` /
  `add_issue_comment` invocation is intercepted by the reviewer
  process; if a cap is exceeded, the call returns a synthetic error to
  Opus (so Opus stops trying that path) and the reviewer logs a
  `rate_limit_hit` record.
- **Kill-switch:** if `max_new_issues_per_day` is exceeded, the
  reviewer halts further GitHub writes for the day and emits a
  `severity:critical` `reviewer_runaway` record into
  `critical-events.jsonl`. The webhook fires; humans investigate
  before the next tick.
- **Self-test on the cap:** at each tick start, the reviewer
  pre-validates that the rate-limit gate fires when invoked with a
  synthetic over-cap call. If the self-test fails, the reviewer
  refuses to run.

This is independent of whatever Anthropic billing limits exist; it is
the only structural protection against the soft-rubric failure mode.

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

Daily Opus invocation reads tens of KB from disk in the steady state
and ships a few thousand input tokens (mostly cached system+rubric
prompt). Cost order: **single-digit cents per run.**

The 48h max-window cap above bounds the worst case: even after a long
outage, the input is `(48h × ~5 MB/day raw)` summarized down to
findings/self-reports/stats — never more than ~200 KB of structured
input plus the cached prompt. If Opus needs more, it pulls specific
log ranges via the grep tool, with each grep call further bounded.

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

**Linode 4GB (or equivalent): 4 GB RAM, 2 vCPU, 80 GB disk, Ubuntu
24.04 LTS.** Earlier draft proposed 2GB / 1 vCPU based on hand-waving;
that's optimistic given each `ClientHandle` brings up its own iroh
`Endpoint` (full QUIC stack, gossip, blob store, relay connection)
plus ~10 actors per handle (`ProfileState`, `NetworkMeta`,
`PersistenceActor`, `EventState`, `Broker`, `dag`, etc. — see
`crates/client/src/lib.rs:665-723`). At a conservative 60–100 MB RSS
per handle, 13 handles × ~80 MB ≈ 1+ GB *before* any work, plus
swarm-runner overhead, checker, opus-reviewer (which holds log slices
in memory during the tick), digest-writer, critical-webhook, the
Docker daemon, and journald.

**Phase 1 exit criterion explicitly includes a measured RSS report**
for one `ClientHandle` (steady-state, 24h after grove bootstrap), so
the 4GB sizing is verified rather than assumed. Per-service
`mem_limit` in `compose.yaml` is set conservatively from the measured
numbers.

Network bandwidth is the only other thing worth watching — bots talk
to the prod relay over QUIC.

**Per-service Docker resource limits** (declared in `compose.yaml`,
verified in Phase 1):

| Service | `mem_limit` (default) | `cpus` |
|---|---|---|
| `swarm-runner` | 2.5 GB | 1.5 |
| `invariant-checker` | 512 MB | 0.3 |
| `opus-reviewer` | 512 MB | 0.2 (idle), bursts during tick |
| `digest-writer` | 256 MB | 0.1 (idle), bursts during tick |
| `critical-webhook` | 64 MB | 0.05 |

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

### Identities and secrets

- **`operator`** is the grove owner. Its key is the single most
  critical secret on the VPS — losing it bricks the grove permanently.
- **Personas** each have their own keypair, generated on first run,
  persisted indefinitely. Bot peer IDs stay stable across restarts
  (this is what makes "long-running session" actually mean something).
- **Backup discipline:** `just soak-backup` (added recipe) tars
  `/var/lib/willow-soak/keys/` **plus the bootstrap-integrity marker
  (below)**, encrypts with age, uploads to a backup bucket. Run from a
  maintainer's laptop, not the VPS itself. Documented in
  `docs/runbooks/soak.md`.

**API tokens — least privilege and rotation:**

| Secret | Storage | Required scope | Rotation cadence |
|---|---|---|---|
| `ANTHROPIC_API_KEY` | `/etc/willow-soak/secrets.env` (root-readable, mode 0600) | Per-key Anthropic budget cap configured in the Anthropic console = the daily cap in `soak.toml` × 1.5. Independent budget for the Opus key vs Haiku key — separate keys recommended so Opus billing can be paused without stopping bots. | 90 days, or immediately on suspected leak. |
| `GITHUB_TOKEN` | same | **Fine-grained PAT or GitHub App** scoped to `intendednull/willow` only. Permissions: `issues:write`, `pull_requests:write`, `contents:write` (for the `soak-digest` branch). **Not** `admin`, **not** `delete_repo`, **not** other repos. | 90 days, or immediately on suspected leak. |
| `CRITICAL_WEBHOOK_URL` | same | Webhook URL only; no auth secret if the receiver supports IP-allowlist, otherwise an HMAC shared secret. | On receiver change; 180 days otherwise. |

**Rotation runbook** (in `docs/runbooks/soak.md`):

1. Generate new secret in the provider's console.
2. Update `/etc/willow-soak/secrets.env` on the VPS via SSH.
3. `docker compose up -d` (services pick up the new env on next
   container start).
4. Verify next opus-reviewer tick succeeds (`just soak-status`).
5. Revoke the old secret in the provider's console.
6. Log the rotation in `process.metrics.jsonl` via a one-shot tool.

**Leak response runbook** (also in `docs/runbooks/soak.md`):

1. Revoke the leaked secret in the provider console immediately
   (before rotating).
2. Audit recent activity (Anthropic usage logs, GitHub audit log,
   webhook receiver logs).
3. Rotate per the steps above.
4. File a `security-incident` issue with the audit findings.

### Bootstrapping the grove

Idempotent first-run logic in `swarm-runner`:

1. If `bootstrap.lock` exists, skip and start normal loop.
2. Else if a **`grove-marker.json`** is present in
   `/var/lib/willow-soak/keys/` (see "Bootstrap integrity marker"
   below), the system **must** find the existing grove via the prod
   storage worker's history sync. If the storage worker cannot serve a
   grove matching the marker's owner-signed genesis hash within
   `bootstrap_recovery_timeout` (default 5min), `swarm-runner`
   **refuses to bootstrap** and emits a `severity:critical`
   `bootstrap_grove_missing` finding. A human decides whether to
   restore from a deeper backup, accept that the grove was wiped (and
   delete the marker explicitly), or wait for the storage worker.
3. Else (no `bootstrap.lock`, no `grove-marker.json` — true fresh
   start): generate operator key, generate persona keys, create grove
   via operator's `ClientHandle`, grant `seed_permissions` per persona
   TOML, generate invites, have each persona join, **write
   `grove-marker.json` containing the owner-signed genesis-event hash
   plus the operator's public key**, then write `bootstrap.lock`.
4. Bootstrap is a logged event (`process.metrics.jsonl`,
   `kind=grove.bootstrap`).

#### Bootstrap integrity marker

The marker is a small JSON file written alongside the operator key on
first bootstrap, included in `just soak-backup`, and required to be
present before any "operator key restored from backup" recovery can
proceed:

```json
{
  "schema_version": 1,
  "operator_pubkey": "ed25519:...",
  "genesis_event_hash": "blake3:...",
  "bootstrap_ts": "2026-05-03T11:14:00Z",
  "signature": "ed25519:...(operator's signature over the above fields)..."
}
```

This closes the silent-data-loss path the spec previously had: if the
prod storage worker GC'd the bot grove (per the
[#393](https://github.com/intendednull/willow/issues/393)
re-bootstrap-on-protocol-break policy, or for any other reason) and a
maintainer restores keys from backup, **the system does not re-create
a divergent grove with the same operator key**. It refuses to start
until a human inspects what happened. The "we wiped the grove" path is
now an explicit human action: delete `grove-marker.json` from the
restore set before restoring, or run `just soak-bootstrap --force-new`
which deletes the marker on the VPS first.

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

### Phase 0 — `ClientHandle` multi-tenancy + tool dispatch *(prerequisite)*

Two coupled refactors that must land before any swarm code can run.
They split into two independent PRs.

**Phase 0a — Multi-tenant `ClientHandle`** *(load-bearing)*

The earlier draft of this spec assumed `ClientHandle` could be
instantiated multiple times in one process. It cannot today:
`ClientHandle::new` resolves identity through a process-global
`load_identity()` (`crates/client/src/lib.rs:571`) and storage paths
through a process-global `data_dir()`
(`crates/client/src/storage.rs:246`). Twelve in-process bots would all
clobber the same `identity.bin`, the same `server.bin`, and the same
per-server event stores.

Scope:
- Add explicit `identity: Ed25519Keypair` and `data_dir: PathBuf`
  fields to `ClientConfig`; remove process-global resolution from
  `ClientHandle::new` (replaced with explicit args, not env).
- Thread `data_dir` through `PersistenceActor`, the per-server event
  store path computation in `storage.rs`, and any other persistence
  consumers.
- Existing single-tenant call sites (web app via wasm, agent E2E
  tests, dev binaries) construct `ClientConfig` from the same
  `dirs::data_dir()` they used before — preserves current behavior.
- Add a multi-instance test in `crates/client/tests/` that spawns 4
  in-process `ClientHandle`s with separate `data_dir`s, runs them
  through a `MemNetwork`, and asserts isolated persistence + correct
  per-instance identity.

Exit: existing tests pass; the multi-instance test is green; web app
and existing agent tests unchanged.

**Effort:** ~1 week, possibly 2 if persistence threading is gnarlier
than it looks. Lands in its own PR.

**Phase 0b — Non-MCP tool dispatch on `WillowToolRouter`**

`crates/agent/src/tools.rs` is already library-shaped
(`WillowToolRouter::new`, `tool_list()`, `call()` are public). The gap
is that `call()` takes `rmcp::CallToolRequestParams`. Add a thin
non-MCP dispatch (e.g. `call_by_name(name: &str, args: serde_json::Value)`)
so swarm-runner can invoke tools without the rmcp encoding round-trip.

Exit: existing MCP tests pass; new direct-dispatch test green; agent
crate compiles unchanged otherwise.

**Effort:** ~2 days. Lands in its own PR after Phase 0a is in.

Total Phase 0 effort: ~1.5–2.5 weeks across two PRs.

### Phase 1 — MVP swarm: 3 bots, no chaos, manual review

- `operator` + `alice`, `bob`, `carol` only. Simple personas, event-driven
  loop, reflection at 4h.
- Tool subset: send message, list channels, send DM, react. (Not yet:
  create channel, grant role.)
- Invariant checker: **heads-summary convergence, per-author
  event-content equality, panic log scrape (test-VPS scope, allowlist
  installed), process liveness, disk-utilization, HLC monotonicity**
  only. Other checks deferred until Phase 2.
- Logging stack fully in place (JSONL streams + rotation + retention).
- **No `opus-reviewer` yet** — humans tail logs for the first week.
  Calibrating "what does good look like" before turning Opus loose.
- Deployed via Docker Compose on the test VPS, connected to prod
  relay/storage.
- **Exit:**
  - Swarm has run for 7 days uninterrupted.
  - ≥3 invariant checks have fired and been audited.
  - Bot identities have survived a service restart and re-converged
    correctly.
  - One round of "real findings vs. test-fixture noise" review done.
  - **Measured RSS report** for one steady-state `ClientHandle`
    (24h after grove bootstrap), checked into the runbook so §6's
    sizing assumptions are verified rather than inherited.
  - Anthropic prompt-cache hit rate measured against the
    `report_cache_hits` metric in `process.metrics.jsonl`, so the
    cost-model assumptions in §3 are calibrated.
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
| **LLM cost overrun** (cache hit rate worse than estimated, Opus filing aggressively, Haiku price changes). | Daily cost tracker in the digest. Hard daily cap in `soak.toml` — if exceeded, swarm pauses Haiku calls (checker keeps running). Cost cap is set against the *realistic* (30–60% cache hit) estimate, not the *optimistic* one. Separate Anthropic budget caps per key (Haiku vs Opus) — see §6 secrets. |
| **Prompt-cache TTL miss** — 5-min default vs hourly reflection cadence means quiet bots routinely miss cache. | Use 1h cache TTL on the static prefix. Optional cache-warming ping every 4 minutes (defaults off; enabled only if active-period cost dominates). Phase 1 exit measures actual hit rate. |
| **Opus / opus-reviewer runaway** (regression or hallucination causes mass issue creation). | Hard out-of-band caps on `mcp__github__issue_write`/`add_issue_comment` per tick and per day (see §5 "Hard rate limits"). Pre-call gate intercepts every GitHub MCP call. Kill-switch trips a `severity:critical` `reviewer_runaway` event. Independent of Anthropic billing; structural protection. |
| **Test-fixture bugs masquerading as Willow bugs** (worst failure mode — engineers chase phantoms). | Checker self-tests at startup, refuse to run if any fail. `concurrent_chaos` on every finding. Phase 1's manual-review burn-in. `bad-triage` label as feedback. |
| **Operator key loss** (catastrophic — bricks the grove). | `just soak-backup` runbook (now includes `grove-marker.json`). Documented as a P0 operational responsibility. |
| **Bootstrap-from-backup creates a divergent grove** (operator key restored, but storage worker had GC'd the old grove → re-bootstrap with same identity → silent drift). | `grove-marker.json` integrity marker (§6) is required-present on any "operator key from backup" path. If marker present and grove not findable in storage worker within timeout, system refuses to bootstrap and emits a `severity:critical` finding. Wiping the grove becomes an explicit human action (`--force-new` or marker deletion). |
| **API token leak / over-privilege.** | Fine-grained PAT or GitHub App scoped to this repo only with least-privilege scopes (`issues:write`, `pull_requests:write`, `contents:write`). Anthropic per-key budget caps. 90-day rotation. Documented leak-response runbook. |
| **Opus filing low-quality issues at scale.** | Strict triage rubric. `bad-triage` label fed back as negative context next run. Weekly digest's "self-audit" section. |
| **Bot persona drift** toward uninteresting equilibria (bots stop doing anything novel). | Reflection prompt rotation; periodic persona-prompt review as part of weekly digest. |
| **Anthropic API outages** stalling the swarm. | Checker is independent of Anthropic. Bots back off and retry; outage windows are logged but not flagged as Willow bugs. |
| **Test VPS resource exhaustion** (RAM, disk, file handles). | Disk-fill check (§4). Per-service `mem_limit` (§6). Phase 1 measures actual `ClientHandle` RSS so the 4 GB sizing is verified, not assumed. `ulimit -n` raised in container, journald `SystemMaxUse` capped, `docker system prune` weekly cron. |
| **Multi-tenant `ClientHandle` refactor (Phase 0a) lands buggy.** | Phase 0a's exit gate is a green multi-instance test asserting per-instance identity + isolated persistence. Existing single-tenant call sites continue to use `dirs::data_dir()` so the refactor is backward-compatible. |
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
  MCP server prior art; Phase 0b's non-MCP tool dispatch builds on
  this.
- `crates/state/src/dag.rs` and `crates/state/src/sync.rs` — the
  `HeadsSummary` API the convergence checks are framed against.
- `crates/network/src/mem.rs` — `MemNetwork` test substrate used by
  the multi-peer self-tests in §4.
- `crates/client/src/lib.rs` and `crates/client/src/storage.rs` — the
  process-global identity/data-dir resolution that Phase 0a replaces
  with explicit per-instance config.
- `docs/specs/2026-04-26-state-management-model-design.md` — actor /
  state-management model that bot ClientHandles and the checker rely on.
- `docs/specs/2026-04-12-state-authority-and-mutations.md` — permission
  model the operator persona uses to grant `seed_permissions`.
- `crates/agent/` — the MCP server to be refactored in Phase 0.
- [#392](https://github.com/intendednull/willow/issues/392) — declarative
  provisioning + unified deployment pattern (deferred follow-up).
- [#393](https://github.com/intendednull/willow/issues/393) — version
  upgrade protocols + rolling-upgrade tooling (deferred follow-up).







