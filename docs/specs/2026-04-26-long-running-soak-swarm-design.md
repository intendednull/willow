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

### What this tier uniquely catches (vs. cheaper alternatives)

Most checks in §4 are mechanically expressible at lower tiers — a
`MemNetwork`-driven property test running for hours on CI hardware
catches `heads-summary divergence`, `per-author event-content equality`,
`HLC monotonicity`, `dedup positivity`, and `restart-survival` at $0/day
with full determinism. The soak swarm's *unique* value is a function of
two things scripted tests cannot produce:

1. **Unscripted, semantically-coherent action mixes.** A property test
   uses random or model-driven action sequences. An LLM-driven persona
   produces *narratively coherent* sequences (Alice creates an "events"
   channel, invites people, organizes movie night, kicks a misbehaving
   bot, recovers from someone's bad react bomb). Bugs that emerge from
   *plausible* action mixes — particularly UX/permission-interaction
   bugs — only surface here.
2. **Wall-clock duration on real persistence.** Bugs that need weeks
   to manifest:
   - **Per-author seq-number boundary encoding** (u32 wraparound,
     varint encoding edges).
   - **Cumulative HLC drift** between peers' system clocks.
   - **Compaction / GC interactions** on event histories of 10⁵+ events.
   - **Epoch-rotation N-of-many** — interactions between rotations and
     other events when N rotations have happened.
   - **Operating-system-level state drift** — file-handle leaks, fsync
     order assumptions, IndexedDB compaction (in the Phase 3 browser
     canaries).

   These are timing- and persistence-state-dependent in ways that don't
   compress into a CI run.

**Cheap-tier-first discipline.** Each check below is annotated in §4 with
either `[mem-net-ci]` (should also exist as a `MemNetwork` torture test
on CI; the soak swarm is corroboration, not the only line of defense) or
`[soak-only]` (genuinely needs the swarm's unscripted-coherent traffic or
wall-clock duration). When a soak finding fires for a check tagged
`[mem-net-ci]`, it's a sign the cheaper test is missing — opening the
cheaper test as a follow-up is part of the standard triage response.

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

~12 bots, ~200–400 ticks/day total. **Realistic estimate** (with the
caveats below baked in): **$5–25/day Haiku** spend, plus 2× / day Opus
reviews at $1–4 per run. **Single-digit thousands of dollars per
quarter** in Anthropic spend, plus the test VPS (~$20/month).

**Why the realistic estimate is well above the textbook one.**
Anthropic's prompt cache has a ~5-minute default TTL. The hourly
reflection trigger and event-driven ticks during quiet periods
routinely fall outside that window — every "first tick after the cache
expired" pays full input price on `system_prompt + tool_defs +
grove_structure_snapshot`. A bot that wakes once per hour will
essentially never hit cache. Effective hit rate over realistic traffic
is **30–60%, not the 80–95% an idealized stable workload would get**.

**The optimistic textbook estimate** (80–95% cache hit, ~$1–5/day,
"few hundred a quarter") is what the literature might suggest, but
it's not what this swarm will actually pay. The cap and the digest's
cost-tracker are calibrated against the realistic number, not the
optimistic one. Phase 1 exit measures the actual hit rate so this can
be re-grounded in observed data.

Mitigations the spec adopts:

- Use **1h cache TTL** (the longer Anthropic option) on the static
  prefix, not the 5-min default.
- Optionally: a low-cost cache-warming ping on a 4-minute schedule
  (cheaper than missing the cache repeatedly during active periods,
  noisier than just paying full price during quiet periods — defaults
  off, enabled by `soak.toml`).

Bounded by a hard daily cap in `soak.toml` (default $40/day, well
above the realistic top); if exceeded, swarm pauses Haiku calls
(checker keeps running).

### Self-report shape

Self-reports are the **bot-perspective lane** of the reporting pipeline,
distinct from the invariant checker's findings. They surface the kind of
bug a mechanical check can't define: "I tried to send a message and the
UI showed it but other people don't see it" (perceived delivery failure),
"this channel keeps showing 0 members but I just invited people"
(perceived membership-state weirdness), "I got an error toast I don't
understand" (unrecognized error path).

**Trigger conditions** (built into the bot loop):

- **Tool-call failure** — any `WillowToolRouter::call_by_name` returns
  an error variant (validation failure, permission denied, conflict).
  The persona's prompt asks: "you just tried X, it failed with Y. Is
  this expected given what you were doing?"
- **Perception mismatch** — bot recently called `send_message` and a
  later `list_messages` window doesn't include the message *as visible
  to that bot*. (Auto-detected before invoking Haiku.)
- **Self-flagged anomaly during reflection** — the hourly reflection
  prompt explicitly asks "anything you noticed in the last hour that
  felt wrong or surprising?" The persona either says no (no record) or
  produces a free-text report.

**Schema** (`bot.self-reports.jsonl`):

```json
{
  "schema_version": 1,
  "ts": "2026-04-26T14:02:11Z",
  "kind": "bot.self_report",
  "persona": "alice",
  "trigger": "tool_call_failure" | "perception_mismatch" | "reflection",
  "context": {
    "recent_actions": [...3 most recent tool calls...],
    "current_channel": "general"
  },
  "report_text": "...persona's free-text description...",
  "confidence": "low" | "medium" | "high"  // self-rated by persona
}
```

**Producer:** the bot loop in `swarm-runner` writes these directly when
the trigger fires; no Haiku call is needed for the auto-detected
triggers (just structured emission). The reflection-triggered ones go
through Haiku as part of the standard reflection tick; cost is folded
into normal reflection budget.

**Consumer:** opus-reviewer reads them as one of the input streams (§5).
Self-reports never auto-file GitHub issues directly — they always go
through Opus's triage rubric, which clusters them with related
invariant findings if any.

**Bounded volume.** A buggy persona could in principle emit many
reports per hour. Cap: `max_self_reports_per_persona_per_hour` (default
**5**). Over-cap reports are silently dropped with a `process.metrics`
counter increment so we know it happened. Phase 1 calibration validates
the cap is set sensibly.

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

#### Required `ClientHandle` accessor surface

The current public `ClientHandle` API exposes `state_snapshot() ->
ServerState` (`crates/client/src/accessors.rs:69`) but does **not**
expose `HeadsSummary` or per-author chains. The `Dag` is `pub(crate)
dag_addr` (`crates/client/src/lib.rs:271`). The check catalog requires
two new public accessors, added as part of Phase 0a (§7):

1. **`ClientHandle::heads_summary() -> HeadsSummary`** — wraps
   `Dag::heads_summary()`. Returns the per-author frontier. Cheap,
   immutable snapshot.
2. **`ClientHandle::author_chain(author: &EndpointId, since: Option<EventHash>)
   -> Vec<EventHash>`** — returns the author's full chain (or a
   suffix from `since`). Used by the "every position 0..=s"
   comparison in `per-author event-content equality`. Note: this is
   the *hash chain*, not full event payloads — payloads are looked up
   via existing accessors when needed (and only in the divergent case,
   keeping the steady-state cost low).

Both accessors are pure reads of the local DAG, do not require
network I/O, and are safe to call from the invariant checker on any
bot's `ClientHandle`. Phase 0a's exit gate includes these accessors.

#### Tier annotations

Each check below is annotated with `[mem-net-ci]` (also a candidate
for a deterministic `MemNetwork` torture test on CI; soak is
corroboration, not the only line of defense) or `[soak-only]`
(genuinely needs the swarm's unscripted-coherent traffic, real-relay
delivery timing, or wall-clock duration).

### Check catalog

| Check | Cadence | Description | Severity rules |
|---|---|---|---|
| **heads-summary convergence** `[mem-net-ci]` | every 30s | Pull `HeadsSummary` from each bot's `ClientHandle::heads_summary()` (Phase 0a accessor). **Head-equality check:** for every author present in ≥2 peers' summaries, those peers must agree on the head `EventHash` for that author *or* one peer's chain must be a strict prefix of the other's (allowed: lagging ingest). The disagreement case is "same author, same head-position-claim, different head EventHash." **Lag bound:** every peer's head for each author must be at most `lag_bound_seconds` behind the most-advanced peer's head; configurable in `soak.toml` (default 30s steady, 5min critical). | Heads disagree on a position both peers claim = `critical` immediately (true divergence). Lag >`lag_bound_seconds` in a steady-traffic window = `warning`; lag >`lag_bound_critical_seconds` = `critical`. |
| **per-author event-content equality** `[mem-net-ci]` | every 5min | For every author observed across ≥2 peers in the window, fetch each peer's `author_chain(author, since=last_window_head)` (Phase 0a accessor). For every position present in ≥2 peers' chains, peers must hold identical `EventHash`. Where chains diverge at position `p`, both `EventHash`es and the underlying event payloads are pulled and recorded. | Any same-position-different-hash = `critical`. |
| **message-delivery audit** `[soak-only]` | every 5min | For each `send` in the last window: at the moment of send, the bot loop snapshots **`(sender_heads_summary_at_send, sender_membership_at_send)`** into a `delivery-audits.jsonl` side-stream. The auditor reads from this side-stream rather than reconstructing post-hoc. **Intended recipients** = the channel-membership snapshot from the side-stream record (sender's local view at send-time). For each recipient peer, verify the event appears in its local event store within `delivery_deadline_seconds` (default 60s) of its system clock first observing the event. Recipients excluded if their `kick`/`leave` causally precedes the send in the receiver's HeadsSummary. | Any send not delivered to an intended recipient after `delivery_deadline × 2` = `warning`. Sustained miss rate >0% over 1h = `critical`. |
| **panic log scrape (test-VPS scope only)** `[mem-net-ci]` | every 30s | Tail Docker container logs (`docker logs --since`) for `swarm-runner`, `invariant-checker`, `opus-reviewer`, `digest-writer`, `critical-webhook`. **Pattern catalog (allowlist-driven, not substring):** matches lines that begin with `thread '...' panicked at `, lines with the literal `note: run with \`RUST_BACKTRACE=1\``, lines tagged `target=panic_hook`, and `tracing` events at `Level::ERROR` from non-allowlisted targets. **Allowlist** of expected `ERROR`/`WARN` patterns lives in `panic_allowlist.toml` (in-repo, reviewed) — `iroh_gossip::net::receiver lagged`, etc. **Out of scope:** prod relay and prod storage worker logs. Those services run on a different VPS and we have no log-shipping channel; spec §6 explicitly excludes log shipping from this work. Prod-side panics during the swarm's run are not caught by this check (see §8 risks). | Any non-allowlisted panic-pattern hit = `critical`, immediate. Any non-allowlisted `Level::ERROR` = `warning`; ≥3 in 24h = `critical`. |
| **process liveness** `[soak-only]` | every 30s | Each bot task and the checker itself responds to a ping. | Missed ping >2 windows = `critical`. |
| **HLC monotonicity** `[mem-net-ci]` | every 5min | For each peer, the per-peer HLC `(physical_ms, logical_counter)` tuple — read from `crates/messaging/src/hlc.rs::HlcTimestamp` on each event the peer emits — must be non-decreasing in lexicographic order across all events authored by that peer in the window, **including across restart boundaries** (the persisted HLC state must be loaded and respected on `ClientHandle::new`). | Any decrease (in either component) = `critical`. |
| **dedup positivity** `[mem-net-ci]` | every 1h, **enabled in Phase 1** (per its own justification) | The `apply()` path must hit its dedup branch (same event re-applied is a no-op) at least once per hour during gossip volume above `min_gossip_events_per_hour` (default 100; below this floor the check is `info`-level skip). If dedup count stays at zero in an active hour, dedup logic itself is suspect. | Zero dedup hits in a 1h window with gossip volume above floor = `warning`; sustained 4h = `critical`. |
| **resource trends** `[soak-only]` | every 1h, **disabled until day 14 of the run** | Linear regression over the last 7 days of: bot-task RSS, swarm-runner total RSS, per-bot on-disk event-store size, search index size. **Normalization:** slopes are computed *per event ingested*, not per wall-clock day, to separate organic activity-driven growth from leaks. | Per-event-ingested slope > threshold (default >0.5%/1k events) = `warning`. Sustained 14 days = `critical`. |
| **convergence-lag distribution** `[soak-only]` | every 1h. **Baseline collection starts day 1; check enabled day 14.** | Histogram of per-event "time from peer A first sees event X → peer B first sees event X" across all ordered peer pairs. | p99 regression vs 7-day baseline = `warning`. Baseline window is the median of the previous full 7-day samples; not meaningful before day 14, but baseline data is collected throughout Phase 1 so day-14 enablement has 7+ days of samples ready. |
| **key-rotation correctness** `[mem-net-ci]` | every 1h | After every epoch rotation event in the window, verify the next N messages on the same channel decrypted on every member that has ingested both the rotation and the message. | Any decryption failure following a rotation = `critical`. |
| **cross-rotation re-decryption** `[soak-only]` | daily | After a peer has been offline across ≥2 epoch rotations and rejoins, verify it can read history encrypted under the older epochs (within whatever the protocol's stated retention window is). Detects "ratchet too aggressive, can't decrypt N-2" failure mode. | Cannot decrypt a message within the documented retention window = `critical`. |
| **bot event-store integrity** `[soak-only]` | daily | For each bot, walk the local event store and verify (a) every event signature verifies, (b) every event's parent hash is present in-store, (c) the `apply()` of the store from genesis produces the same `ServerState` as the running bot's `state_snapshot()`. Detects torn writes, fsync-ordering bugs, and any silent corruption in `PersistenceActor`. | Any signature/parent/apply-equality failure = `critical`. |
| **disk-utilization** `[soak-only]` | every 5min | `/var/log` and `/var/lib` utilization on the test VPS. | 70% / 85% / 95% thresholds → `info` / `warning` / `critical`. |
| **cross-session continuity** `[soak-only]` | daily | Chaos-driver spawns a fresh peer (new keypair, given an invite). After sync settles, observer verifies that for every (author, seq) reachable from any established peer's HeadsSummary, the fresh peer holds the same `EventHash` at that position. The fresh peer's derived `ServerState` is structurally equal to established peers'. | Any (author, seq)-position the fresh peer is missing or disagrees on after 5min = `critical`. Any structural difference in derived `ServerState` = `critical`. |
| **restart-survival** `[soak-only]` | every 4h (Phase 2+); **a stripped-down "single soft-restart" form runs once during Phase 1's burn-in** to gate Phase 1 exit | Chaos-driver picks a random bot, drops its task (which gracefully drops the `ClientHandle` and its actor runtime), then re-creates a fresh `ClientHandle` for that persona using the same persistent identity and `data_dir`. Once sync settles, observer verifies the post-resync HeadsSummary is a superset of the pre-restart HeadsSummary (no events lost) and the post-resync materialized `ServerState` is structurally equal to a peer that did not restart. **Implementation detail:** with multi-tenant `ClientHandle` (Phase 0a), this is an in-process operation in `swarm-runner`, driven by an admin-channel signal from the checker — no cross-process IPC needed. | Any pre-event missing post-resync = `critical`. Any structural state mismatch with a non-restarted peer = `critical`. |
| **cross-grove isolation (best-effort, disabled by default)** `[soak-only]` | every 1h | Bot grove's CPU/bandwidth/memory share on the prod relay should not grow as a fraction of total when bot activity is steady. | Best-effort signal pending per-grove relay metrics. **Disabled** in `soak.toml` until the relay exposes per-grove counters (out of scope for this spec). When enabled: unbounded growth = `warning`. |

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

**`fingerprint`** is a **canonicalized, order-independent, container-
independent string** keyed by check name and check-specific evidence
keys. Per-check definitions:

| Check | Fingerprint keys |
|---|---|
| `heads_summary_divergence` | `check + author + position` |
| `event_content_inequality` | `check + author + seq` |
| `message_delivery_miss` | `check + send_event_hash` (independent of which recipient missed it) |
| `panic` | `check + first 80 chars of panic message` (no container — same panic in different containers is one bug) |
| `level_error` | `check + tracing_target + first 60 chars of message` |
| `hlc_decrease` | `check + author` |
| `dedup_zero` | `check` (the bug is global) |
| `resource_slope` | `check + metric_name` |
| `convergence_lag_p99` | `check + author` |
| `key_rotation_decrypt_fail` | `check + epoch + channel_id` |
| `cross_rotation_decrypt_fail` | `check + epoch_distance + channel_id` |
| `event_store_integrity` | `check + persona + failure_class` (signature/parent/apply) |
| `restart_survival_mismatch` | `check + persona` |
| `cross_session_mismatch` | `check + author` |
| `auto_resolve_pattern` | `check + underlying_check + underlying_fingerprint` |

Fingerprints are **the deduplication key** for opus-reviewer (§5) — Opus
searches `soak`-labeled issues by `fingerprint:<value>` body substring
before deciding whether to file or comment, so the same root bug
surfacing across many evidence permutations clusters into one issue.

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
   when ≥5 auto-resolves on the same fingerprint occur within 24h.
   That synthetic finding feeds the same triage rubric as any other.

   #### How auto-resolve tracking is actually wired

   - **Resolution detection.** Each check has an explicit `is_resolved(open_finding)
     -> bool` predicate. Examples: for `heads_summary_divergence`, the
     finding is resolved when the next observation tick sees the two
     peers agree at that `(author, position)`. For `level_error`, the
     finding is auto-resolved if the same fingerprint hasn't fired in
     2× the check's cadence. For `panic`, never auto-resolved (a
     panic happened — that's a fact). The predicate is part of each
     check's implementation, not a separate component.
   - **Tracking storage.** The observer maintains an
     `open_findings.jsonl` companion file (in `/var/lib/willow-soak/`,
     not `/var/log/`) that holds the live state of each unresolved
     finding (`fingerprint`, `first_observed`, `last_observed`,
     `auto_resolve_count_24h_window`). Updated atomically (write-to-tmp,
     rename) on every check tick. Survives observer restarts.
   - **Window tracking.** The `auto_resolve_count_24h_window` is a
     sliding window of resolution timestamps; the observer prunes
     entries older than 24h on every check tick. When a count reaches
     5, the synthetic `auto_resolve_pattern` finding is emitted with
     fingerprint `auto_resolve_pattern + underlying_check + underlying_fingerprint`,
     and the counter is reset (so a single underlying bug only emits
     one pattern finding per 24h).
   - **Restart durability.** Because the state lives in
     `/var/lib/willow-soak/open_findings.jsonl` (not in-memory), the
     observer can restart and resume resolution detection without
     losing partial progress.

   #### Self-test substrates

   - **State-machine checks** (heads-summary divergence, per-author
     equality, dedup positivity, HLC monotonicity, key-rotation,
     event-store integrity): self-test by constructing
     `ServerState`/`Dag` directly via `apply()` with synthetic events
     that produce a known violation. No network needed.
   - **Multi-peer convergence checks** (cross-session continuity,
     restart-survival): use `MemNetwork`
     (`crates/network/src/mem.rs`) with deterministic clock injection
     and a small (3-peer) mesh. The synthetic test stages a known
     divergence by feeding events to one peer and not another; the
     check should fire.
   - **Timing-sensitive checks** (message-delivery audit,
     convergence-lag): `MemNetwork` delivers near-instantaneously,
     which is **insufficient** for these self-tests. The spec adds a
     `DelayedMemNetwork` test wrapper that imposes per-peer
     configurable delivery delays and exposes a deterministic
     "advance time by N seconds" hook. Used only by these self-tests.
     Implementation lives in
     `crates/network/src/mem_delayed.rs` (new file, `test-utils`
     feature).
   - **Resource-trends and convergence-lag baselines** are not
     self-testable in the usual sense (they're statistical); their
     "self-test" is a unit test that feeds synthetic time-series
     data through the regression / histogram code and asserts the
     classifier output.

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

**Hypotheses are restricted to a tagged comment, not the issue body.**
Empirical weakness of "clearly labeled" hypotheses is that engineers
anchor on them anyway. The body contains evidence + log pointers only.
Opus's hypothesis (if any) goes in a comment that begins with the
literal token `<!-- auto-triage-hypothesis -->` on its own line so the
comment is identifiable regardless of comment ordering (humans may
post first). Renderers ignore the HTML comment but it's machine-
greppable.

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
5000/hr authenticated rate limit ends the spree (shared with
`digest-writer` PRs and any human use of the same token).

#### How the gate is wired (the missing implementation surface)

The upstream GitHub MCP server is third-party — opus-reviewer cannot
intercept tool calls inside it. So opus-reviewer **runs its own thin
local MCP server**, `willow-soak-gh-proxy`, which:

1. Is exposed to Anthropic's API as the `mcp__github__*` toolset that
   Opus calls.
2. Forwards every tool invocation to the *real* GitHub MCP server
   running as a sidecar in the same Docker compose service group.
3. Enforces the per-tick / per-day caps on the way through, before
   the forward.
4. Returns a synthetic error response to Opus when a cap is hit (so
   Opus stops retrying that path).
5. Logs every intercepted call (allowed or denied) to
   `opus.decisions.jsonl`.

`willow-soak-gh-proxy` is a new binary in `crates/soak/`, ~200 lines
of Rust using the rmcp server SDK. It wraps the upstream MCP server's
tool list verbatim — no schema translation needed — and only the
`issue_write`, `add_issue_comment`, and `add_reply_to_pull_request_comment`
methods carry rate-limit logic; everything else (search, read) is
forwarded unmodified.

This **closes the "asserted but not designed" gap** earlier drafts had:
the gate has a concrete implementation surface — a local MCP server
binary under our control that Opus is configured to use *instead of*
the upstream one.

#### Caps enforced by the proxy

- **Per-tick caps:** at most `max_new_issues_per_tick` (default **5**)
  new issues, `max_comments_per_tick` (default **20**) comments,
  `max_total_tool_calls_per_tick` (default **50**) GitHub MCP calls.
  Configurable in `soak.toml` (under the `[opus_reviewer]` table —
  see §6).
- **Per-day caps:** `max_new_issues_per_day` (default **15**),
  `max_comments_per_day` (default **80**).
- **Kill-switch:** if `max_new_issues_per_day` is exceeded, the proxy
  halts further GitHub writes for the day, emits a `severity:critical`
  `reviewer_runaway` record into `critical-events.jsonl`, and the
  webhook fires. Humans investigate before the next tick.
- **Self-test:** at each opus-reviewer tick start, the reviewer issues
  a synthetic over-cap probe to the proxy and verifies it returns the
  capped error. If the self-test fails, the reviewer refuses to run.

This is independent of Anthropic billing limits; it is the only
structural protection against the soft-rubric failure mode.

#### Findings-volume cap on the input side

The input side needs symmetric protection. A flapping check during
mass divergence could produce thousands of `invariant.findings` records
per hour, blowing Opus's context window before the rubric ever runs.

opus-reviewer enforces an **input-volume cap** on the slice it sends
to Opus:

- **Per-tick max:** `max_findings_per_tick` (default **500**) raw
  findings included in the slice; if the window has more, the
  reviewer pre-clusters by `fingerprint` *before* calling Opus and
  sends one representative + count + first/last timestamps for each
  cluster. This is a deterministic cluster-by-equality pass, not an
  LLM step.
- **Per-fingerprint max:** within the slice, no single fingerprint
  contributes more than `max_per_fingerprint_per_tick` (default
  **10**) records — additional are summarized as `... + N more
  identical occurrences`.

If a single tick would have shipped >10× the per-tick cap raw, the
reviewer also emits a `findings_storm` `severity:warning` record so
the storm itself becomes a triage signal.

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

### What "success at 6 months" looks like (concrete artifact picture)

If this works as designed, an engineer reading the swarm's outputs
in October 2026 has:

- **A growing catalog of fixed bugs.** Closed `soak`-labeled issues,
  each with a fingerprint that links back to the originating finding
  and the commit that fixed it. Searchable by check, severity, week.
- **A growing catalog of `[mem-net-ci]` follow-ups.** Every soak
  finding for a `[mem-net-ci]`-tagged check yields a
  cheaper-tier-test issue, so the CI tier inherits the lessons (§1
  cheap-tier-first discipline). Counted in the weekly digest.
- **Trend graphs (text-rendered) over months, not just weeks.** The
  digest writer maintains a `docs/reports/soak/long-trends.md` that
  rolls weekly numbers into a 6-month view: RSS slope, event-store
  size slope, dedup-hit rate, p99 convergence-lag, daily Anthropic
  spend, swarm-uptime. Updated every week alongside the weekly
  digest. **Limitation:** trend continuity breaks across grove
  re-bootstraps (per [#393](https://github.com/intendednull/willow/issues/393));
  the long-trend doc explicitly marks each re-bootstrap as a hard
  reset of the timeline. (See §8 risks.)
- **A coverage map.** A `docs/reports/soak/coverage.md` that lists
  every check, its tier annotation, when it was added, when its
  thresholds were last tuned, and the last 5 issues it caught.
- **A persona corpus.** The persona TOMLs have evolved (per Phase 4)
  with new behaviors and reflection prompts that emerged from
  observed bug patterns. Git history of the persona files is itself
  a record of what the swarm has learned.

What the engineer **does not** have: a leaderboard of "bugs found per
week," a trend of how many issues are open right now, or any metric
that incentivizes filing for filing's sake. The artifact picture is
deliberately about *closed* issues, not open ones — open issues are
just open.

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

**Sizing rollback path.** If Phase 1's measured RSS comes back
materially above the assumed 60–100 MB / handle (e.g. ≥150 MB), the
4GB box is undersized. Rollback steps:

1. Resize the VPS up (Linode supports in-place resize to 8GB without
   data loss — disk migration is automated). Cost goes from ~$24 to
   ~$48 per month.
2. Update `compose.yaml` `mem_limit` values from the measured
   numbers.
3. If even 8GB is undersized (≥250 MB / handle): reduce swarm size
   in `personas/` to fit, log the regression as a `severity:critical`
   `client_handle_rss_regression` finding, and open an issue against
   `crates/client/` to investigate why `ClientHandle` is so heavy.
   The whole point of soak is to find this kind of regression — if
   it's there, treat it as a finding, not a sizing failure.

The 60–100 MB / handle estimate is informed but **not measured**;
Phase 1 measures it. Spec does not pretend otherwise.

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
| `GITHUB_TOKEN` | same | **Fine-grained PAT** scoped to `intendednull/willow` only. UI labels: **Issues — Read and write**, **Pull requests — Read and write**, **Contents — Read and write** (for pushing the `soak-digest` branch). Equivalent GitHub App permissions: `issues:write`, `pull_requests:write`, `contents:write`. **Not** `admin`, **not** `workflows`, **not** `delete_repo`, **not** access to other repos. (`workflows` is intentionally excluded; the digest writer must never touch `.github/workflows/*.yml`. The build step in the writer enforces this with a path check.) | 90 days, or immediately on suspected leak. |
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
until a human inspects what happened.

#### Marker lifecycle gotcha — backups outlive `--force-new`

A `--force-new` deletes the marker on the live VPS but **does not
delete it from prior backup snapshots**. A maintainer who later
restores from a snapshot taken *before* the `--force-new` will get
the stale marker back, the live grove won't match (it's the post-`--force-new`
grove), and bootstrap will refuse to start.

This is **fail-closed and correct** (refusing to bootstrap surfaces
the situation rather than silently re-bootstrapping), but it can
look surprising to a maintainer. The runbook's restore procedure
addresses this explicitly:

1. Before restoring, confirm: "is the live VPS in a `--force-new`
   state since this backup snapshot was taken?" (Check
   `process.metrics.jsonl` for `kind=grove.bootstrap` events.)
2. If yes, the restore must either:
   - Restore to a fresh VPS (build fresh from the backup, ignore the
     marker mismatch — but only do this if the grove was wiped and is
     genuinely abandoned), or
   - Delete the marker from the restored snapshot before pushing it
     back to the VPS (`just soak-restore --no-marker`).

The marker's lifecycle is therefore: written once on first
bootstrap, lives forever in every backup, deleted from the live VPS
only by `--force-new`, and the runbook handles the asymmetry.

Wiping the grove is an explicit human action: delete
`grove-marker.json` from the restore set before restoring, or run
`just soak-bootstrap --force-new` which deletes the marker on the
live VPS.

### Configuration: `soak.toml`

Single file, all tunables, defaults match Sections 3–5:

- Per-persona reflection intervals (override the persona TOMLs if needed).
- Per-check cadences (overrides §4 defaults).
- Per-check severity thresholds (lag bounds, slope thresholds,
  dedup-volume floor, etc.) — every numeric threshold in the §4
  catalog is in this file, not hardcoded.
- Chaos cadences (overrides §4 defaults).
- opus-reviewer cadence, watermark `safety_overlap`, `max_window`.
- **`[opus_reviewer]` GitHub MCP rate-limit caps:**
  `max_new_issues_per_tick`, `max_comments_per_tick`,
  `max_total_tool_calls_per_tick`, `max_new_issues_per_day`,
  `max_comments_per_day` — defaults from §5.
- **`[opus_reviewer]` input-volume caps:**
  `max_findings_per_tick`, `max_per_fingerprint_per_tick`.
- **`[bots]` self-report caps:**
  `max_self_reports_per_persona_per_hour`.
- Log retention policy.
- **`[budgets]` Anthropic spend caps:** `daily_haiku_cap_usd`,
  `daily_opus_cap_usd` (independent — Opus billing can be paused
  without stopping bots, and vice versa). Hard caps; on exceedance
  the swarm pauses the corresponding API calls (checker keeps
  running).

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
- Thread `data_dir` through **all storage call sites**, not just
  `PersistenceActor`. The constructor reads storage *outside* the
  actor at `lib.rs:581-657`; `connect.rs`, `mutations.rs`,
  `listeners.rs`, `joining.rs`, `servers.rs`,
  `search/{handle,actor}.rs`, and `storage.rs`'s ~10 module-internal
  helpers all resolve `data_dir()` directly. The refactor either
  passes a `PathBuf` arg through every helper (~30 call sites) or
  introduces a `StorageHandle` value type owned by `ClientHandle`
  that wraps the path and exposes the helpers as methods. The
  `StorageHandle` shape is recommended — fewer signature changes
  per call site and a single place to add multi-tenant guards.
- Add `ClientHandle::heads_summary() -> HeadsSummary` and
  `ClientHandle::author_chain(author, since)` accessors (required by
  §4 checks; spec previously assumed they existed).
- **WASM single-tenancy is preserved.** The web app's
  `localStorage`-backed storage stays single-tenant; the new
  per-instance `data_dir`/`identity` config is wired but ignored on
  WASM (single-tenant by construction in the browser). This is
  acknowledged as out-of-scope for soak — the soak swarm is
  native-only.
- Existing single-tenant native call sites (agent E2E tests, dev
  binaries, `just dev` orchestration) construct `ClientConfig` from
  the same `dirs::data_dir()` they used before — preserves current
  behavior.
- Add a multi-instance test in `crates/client/tests/` that spawns 4
  in-process `ClientHandle`s with separate `data_dir`s, runs them
  through a `MemNetwork`, and asserts: isolated persistence (each
  bot's `data_dir` only contains its own state), correct
  per-instance identity (each bot signs with its own key), no
  cross-talk through any module-global state.

Exit: existing tests pass; the multi-instance test is green; the
two new accessors have unit tests; web app and existing agent tests
unchanged.

**Effort: 2–3 weeks.** Earlier "1–2 weeks" estimate was optimistic
given the 30+ call sites that bypass `PersistenceActor` and the need
to introduce `StorageHandle`. This is a careful refactor that has to
keep `just check-all` green throughout. Lands in its own PR.

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
  installed), process liveness, disk-utilization, HLC monotonicity,
  dedup positivity, bot event-store integrity** turned on in Phase 1.
  Convergence-lag baseline data **collected** during Phase 1 burn-in
  (so Phase 2 has 7+ days of samples) but the lag-regression alarm
  itself stays disabled until day 14.
- **Minimal chaos in Phase 1.** No persona-restart loop yet, but the
  chaos-driver runs a **single soft-restart probe per persona once
  during the burn-in week** so Phase 1 actually exercises the
  HLC-monotonicity-across-restart path (without a restart, the
  HLC-monotonicity check has nothing to catch). The probe is
  scheduled (not random) and logged with `concurrent_chaos`. Full
  every-4h restart-survival waits for Phase 2.
- Logging stack fully in place (JSONL streams + rotation + retention).
- **No `opus-reviewer` yet** — humans tail logs for the first week.
  Calibrating "what does good look like" before turning Opus loose.
- Deployed via Docker Compose on the test VPS, connected to prod
  relay/storage.
- **Exit:**
  - Swarm has run for 7 days uninterrupted.
  - ≥3 invariant checks have fired and been audited.
  - The Phase 1 chaos probe (single per-persona soft-restart) has
    fired for every persona and every restart re-converged with no
    pre-restart events lost (gates "bot identities survived restart"
    via the `restart-survival` check, not human eyeballing).
  - One round of "real findings vs. test-fixture noise" review done.
  - **Triage rubric written and committed.** A
    `docs/runbooks/soak-triage.md` checked into the repo, derived
    from the week's manually-reviewed findings, that becomes the
    seed for Opus's system prompt in Phase 2. Includes: false-positive
    patterns observed and how to recognize them, evidence elements
    that distinguish real bugs from fixture noise, fingerprint
    canonicalization examples from real findings. **Without this
    artifact, Phase 2 is blocked** — Opus's rubric must come from
    real calibration data, not from this spec's prose.
  - **Measured RSS report** for one steady-state `ClientHandle`
    (24h after grove bootstrap), checked into the runbook so §6's
    sizing assumptions are verified rather than inherited.
  - Anthropic prompt-cache hit rate measured against the
    `report_cache_hits` metric in `process.metrics.jsonl`, so the
    cost-model assumptions in §3 are calibrated. If the measured
    rate is below 30% during typical traffic, the cache-warming ping
    feature in §3 is enabled before Phase 2.
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
  agent-browser instead of the in-process `ClientHandle`.
- Same persona prompts; different tool surface (DOM actions instead
  of `WillowToolRouter` calls).
- Add UI-specific invariants: console errors, unhandled rejections,
  render hangs, IndexedDB growth, service-worker health.
- **Programmatic harness required.** `agent-browser` per CLAUDE.md is
  invoked as a CLI (`agent-browser open URL`, `snapshot`, `click`,
  etc.). Running 1–2 concurrent browser personas from `swarm-runner`
  needs a programmatic wrapper. Two paths:
  - (a) Drive the agent-browser CLI via subprocess from
    `swarm-runner`, parse JSON output. Simple, no upstream
    dependency. ~1 week of glue code.
  - (b) Use Playwright directly from a Rust→Node bridge. More
    flexible (full Playwright API), but adds a Node runtime to the
    Docker image and a bridge crate. ~2 weeks.
  Phase 3 starts with (a); revisit (b) only if (a) becomes a
  bottleneck. Either way the harness is a separate crate
  (`crates/soak/src/browser_harness/` or `crates/soak-browser`)
  with its own tests.
- **Exit:** at least one UI-only bug surfaced and filed.
- **Effort:** ~1–2 weeks dev for option (a). Begun only after Phase 2
  burn-in is clean. Harness can be prototyped in parallel with Phase 2
  burn-in but enabled only after.

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
| **Prod-side panics during the swarm run go uncaught.** Spec scopes the panic log scrape (§4) to test-VPS containers only — there's no log shipping from the prod relay/storage worker. A panic on the prod side is invisible to this fixture. | Acknowledged limitation. Prod-side incidents continue to rely on whatever existing monitoring covers the prod box. Adding log shipping is in [#392](https://github.com/intendednull/willow/issues/392)'s adjacency; can be added later without changing the rest of the design. |
| **Protocol-break re-bootstrap loses longitudinal data.** [#393](https://github.com/intendednull/willow/issues/393)'s wipe-and-re-bootstrap policy resets the time-series the digest plots (resource trends, convergence-lag, cumulative findings counts). Multi-month trend graphs lose continuity at every protocol-break wipe. | The 6-month long-trends doc (§5) explicitly marks each re-bootstrap as a hard reset. Within-grove trends are still meaningful per-grove-lifetime; cross-grove longitudinal claims are deliberately not made. When [#393](https://github.com/intendednull/willow/issues/393) lands rolling-upgrade tooling, this risk shrinks. |
| **Backup bus-factor of one** — `just soak-backup` runs from a maintainer's laptop, so a backup that only one human can perform won't happen on schedule. | Phase 1 burn-in includes a backup-rehearsal: a *second* maintainer must successfully run the backup-and-restore-to-test-VPS sequence end-to-end at least once before Phase 1 exit. If the spec lacks at least two humans who have demonstrated the runbook, the project halts at Phase 1 until that's resolved. |
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







