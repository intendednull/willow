# History Sync Completion Signal (EOSE-equivalent)

> **One-sentence summary:** Add an explicit `HistorySyncComplete`
> variant to `WireMessage` in `willow-common` so clients can tell —
> per topic, per provider — when backfill has finished and the gossip
> feed is live, eliminating the "is history still loading?" ambiguity
> that today's protocol silently imposes on every freshly-joined client.

## Motivation

Today a joining client picks up history through a single client-visible
path: a `WireMessage::SyncRequest` / `SyncBatch` exchange on
`_willow_server_ops` (the SERVER_OPS topic), where peers and the replay
worker stream missing events back to the joiner. (A second path exists
strictly between workers — `WorkerRequest::Sync` / `WorkerResponse::SyncBatch`
on `_willow_workers` — but clients neither send nor subscribe to those
messages today; see the explicit TODO at
`crates/client/src/listeners.rs:294-295` to migrate clients onto the
worker heads-based protocol. Until that lands, history reaches the
client only via SERVER_OPS.)

Today's history path piggybacks on the SERVER_OPS state-sync exchange
(SERVER_OPS carries server-level mutations + `SyncRequest`/`SyncBatch`,
not live chat — chat events flow on per-channel topics). Because chat
events live in the same DAG as those server mutations, they ride back
to the joiner inside the same `SyncBatch` payload as the historical
state — interleaved with whatever new server-state events ordinary
peers happen to be publishing on `_willow_server_ops` right now. The
client has no way to tell historical from live. The UI therefore has
to guess — usually with a loading spinner that stays up for a fixed
debounce window, or that flips off the first time the event stream
goes quiet for N ms. Both heuristics are wrong on a slow network and
both waste milliseconds on a fast one.

Nostr solved this cleanly with NIP-01's `EOSE` marker: `["EOSE",
<subid>]` is a zero-payload frame the relay emits after all stored
events for a subscription have been flushed, drawing a bright line
between backfill and live tail. We want the same property in Willow.
The details differ — Willow has no subscription ids, no single
authoritative relay, and history is (or will be) served by multiple
provider classes (see "Multiple providers" below) — but the user-
visible invariant we need is identical: **after the client has seen
one of these markers from a trusted provider, it knows the loading
state has resolved and the UI can commit.**

## Wire format

The existing transport-layer enum `MessageType` in
`crates/transport/src/lib.rs` (declared on line 64; variants `Chat=0`
… `Ping=6` on lines 66-78) is bypassed by every production code path:
all current
gossipsub traffic is dispatched through the single
`MessageType::Channel` envelope and routed by the inner
`WireMessage` enum at `crates/common/src/wire.rs:13`. Adding a new
`MessageType` variant for this signal would diverge from that
established convention for no benefit, and adding a payload type at
the transport layer would create a `willow-transport → willow-state`
dependency cycle (transport currently has no `EventHash` access; the
dependency graph in `CLAUDE.md` runs `client → state` and
`state → identity → transport` (via willow-identity)).

We therefore add a new variant to `WireMessage` in `willow-common`
(which already depends on `willow-state`):

```rust
/// Signals that a sync provider has finished streaming the historical
/// portion of its store for a topic. Subsequent events on the same
/// topic from this provider are live, not backfill.
HistorySyncComplete {
    /// The TopicId (blake3 of the canonical topic string) this marker
    /// applies to. Carried explicitly — see rationale below.
    topic_id: [u8; 32],
    /// Hash of the last event the provider streamed before emitting the
    /// marker, or `None` if the provider had zero stored events for
    /// this topic.
    last_event_hash: Option<willow_state::EventHash>,
    /// Monotonically-increasing cursor scoped to (topic_id, provider).
    /// Incremented when the provider restarts and re-streams from zero.
    stream_generation: u64,
},
```

`topic_id` is carried explicitly (rather than implicit in the gossip
topic) so that the marker survives relay-bridge forwarding and so a
single audit log can correlate markers across topics. The provider's
identity is deliberately **not** carried in the payload: like every
other `WireMessage`, the marker rides the same Ed25519-signed
envelope built by `pack_wire` / verified by `unpack_wire`
(`crates/common/src/wire.rs:105-120`), so the receiver derives the
provider's `EndpointId` from the verified envelope signer at unpack
time. This forecloses a class of relay-rewrite / MITM attacks where a
separate `provider_peer` field could be edited to attribute the
marker to a different trusted provider, and saves the bytes besides.
`last_event_hash` lets the client detect truncation: if the last
event the client *received* from that provider does not match the
hash in the marker, the client MUST treat the sync as incomplete (see
"Sharp edges" below). `stream_generation` exists so a provider that
crashes and restarts mid-stream cannot confuse a client still holding
a stale marker from the previous run. The semantics are inspired by
Nostr's one-EOSE-per-REQ pattern; the actual contract is adapted to
Willow's subscription-less topic gossip and is intentionally stronger
than NIP-01's (see "Sharp edges").

The struct serialises well under the existing 256 KB
`MAX_DESER_SIZE` ceiling in `crates/transport/src/lib.rs:36` — it is
~80 bytes on the wire and imposes no new size class.

## Per-topic vs per-request

Nostr's EOSE is scoped to a subscription id because the client
explicitly requested a filtered view. Willow has no such structure:
`Network::subscribe()` in `crates/network/src/traits.rs:132` joins a
gossip mesh by `TopicId`, and from that point every peer in the mesh
sees every event. There is nothing to "close."

We therefore scope the marker to `(topic_id, provider, stream_generation)`,
where `provider` is recovered from the verified envelope signer at
unpack time:

- **Topic** is the unit of subscription a client actually has.
- **Provider** is needed because, unlike Nostr, Willow has multiple
  concurrent providers per topic (see next section).
- **Stream generation** distinguishes fresh streams from a restarted
  provider.

### Which gossip topic carries the marker

In current code the workers do **not** subscribe to per-channel
topics: `crates/worker/src/runtime.rs:30-34` joins only
`WORKERS_TOPIC` (`_willow_workers`) and `SERVER_OPS_TOPIC`
(`_willow_server_ops`). Per-channel topics are joined only by
clients. **Worker → SERVER_OPS is currently ingest-only.** The
runtime binds the SERVER_OPS subscription handle as
`let (_ops_sender, ops_events) = ...` (note the underscore), and all
worker outbound traffic (`HeartbeatActor`, `SyncActor`, the request
reply path in `crates/worker/src/actors/network.rs:141-149`) goes out
on `workers_sender`, the WORKERS topic. Worker replies are
`WorkerWireMessage::Response` frames on WORKERS — they are not
re-broadcast onto SERVER_OPS today.

This spec selects, of the two transports a worker-emitted marker
could ride, **option (a): extend the worker runtime to broadcast on
SERVER_OPS**. The alternative (b) — ride the existing
`WorkerWireMessage::Response` path on WORKERS and require clients to
subscribe to that topic — was rejected because it inverts the topic's
semantics (WORKERS today carries worker-to-worker control traffic
only; clients are intentionally absent), it forces every client onto
a mesh sized for workers, and it couples the marker's lifetime to a
specific `WorkerRequest`/`Response` correlation rather than a
broadcast event any subscriber on the channel/server topic can see.

**Prerequisite (in scope of the implementing PR, not this spec):**
both the replay worker and the storage worker must add an outbound
broadcast handle for `_willow_server_ops`. Concretely, change the
runtime binding from `let (_ops_sender, ops_events) = ...` to a named
sender and thread it into whichever actor emits the marker. No new
network primitive is required — `TopicHandle::broadcast` already
exists; only the binding does not.

With that prerequisite in place, the marker travels on
`_willow_server_ops` for server-state backfill (where the joining
client is already subscribed alongside the workers) and on the
per-channel topic for any peer-to-peer history that streams directly
between clients on that channel topic. A worker that did not
previously broadcast on the channel topic is **not** required to do
so by this spec; if a future worker grows direct channel-topic
backfill it must subscribe to that topic first.

The marker is sent as a regular `TopicHandle::broadcast` frame on the
chosen topic — the same path ordinary events take. This costs one
mesh-wide message per `NeighborUp` on the provider, which is
acceptable overhead for a per-(topic, provider, generation) deduped
marker; existing peers ignore markers whose
`(provider, stream_generation)` they have already observed. Relays
forward the marker by their existing topic-routing rules: see the
`Scope: transport only` section in the module-level documentation of
`crates/relay/src/lib.rs:8-22`, which establishes the relay as
content-agnostic at the wire level.

Using `broadcast` rather than `broadcast_neighbors` is a deliberate
trade. `broadcast_neighbors` (`crates/network/src/traits.rs:72`) is
documented as not forwarded by gossip relays, and there is no per-peer
direct-send primitive in the `Network` trait — so a "send only to the
new joiner" frame would either fail to traverse the relay bridge or
require new transport plumbing. Mesh broadcast plus
`(provider, stream_generation)` deduplication at receivers gives the
same effect with no new primitives.

## Multiple providers

A single server has, in the architecture this spec targets, three
candidate provider classes for history. The first two exist today;
the third is **conditional on the per-author Merkle-DAG sync protocol
landing** (see "Interaction with existing specs"):

| Provider | Source | Completeness | Status |
|----------|--------|--------------|--------|
| **Replay worker** | per-author DAG with `max_events_per_author` cap (default 1000, see `crates/replay/src/role.rs:64`); LRU-evicted at `MAX_SERVERS = 1000` per node (`crates/replay/src/role.rs:18`) | lossy, recent | exists today |
| **Storage worker** | SQLite archival log | authoritative, long tail | exists today |
| **Peer** | any peer with more state than us | opportunistic | **conditional** — requires `SyncMessage` wire plumbing (see below) |

A joining client can receive up to three `HistorySyncComplete`
markers for the same topic (one per provider class). We define
"caught up" as a **first-trusted-wins** rule, with a pluggable
policy:

1. **Default policy (fast UI):** the client considers itself caught up
   for a topic as soon as it has received a valid marker from **any
   peer granted `SyncProvider` permission**
   (`crates/state/src/event.rs:23` — `SyncProvider` is a permission
   that any peer can hold, granted by the owner via
   `EventKind::GrantPermission`; it is not a role). This minimises
   perceived load time and matches the UX the feature exists to
   deliver.
2. **Strict policy (opt-in):** wait until markers have arrived from
   **a majority of currently-connected `SyncProvider` peers**, or from
   the storage worker specifically if one is in the trust list. This
   is the right choice for clients that are about to take an offline
   snapshot or archive state.

Untrusted peers' markers are ignored entirely — accepting them would
let any peer prematurely flip the UI's loading flag off.

## Client API

Emit a new variant on `ClientEvent` in
`crates/client/src/events.rs:19`:

```rust
/// Backfill from at least one trusted SyncProvider has finished for this topic.
HistorySynced {
    topic: String,
    provider: EndpointId,
    /// Number of additional trusted providers still streaming history
    /// for this topic. `0` means no providers are still streaming.
    still_pending: usize,
},
```

The UI subscribes to this event exactly the way it already subscribes
to `MessageReceived`. The natural counterpart is the
`SyncCompleted { ops_applied }` variant already at
`crates/client/src/events.rs:57`: that existing event is session-wide
and is emitted after a batch of operations lands, whereas
`HistorySynced` is topic-scoped and marks the boundary, not the work.
Both are kept — they answer different questions.

## Provider-side emission

Emission is triggered by `NeighborUp` events on the provider's
`TopicEvents` stream (`GossipEvent::NeighborUp` variant at
`crates/network/src/traits.rs:56`; the `TopicEvents` trait at
`crates/network/src/traits.rs:84`). All worker-side emissions
described below are gated on the broadcast-handle prerequisite
recorded in "Which gossip topic carries the marker" — neither the
replay nor the storage worker can broadcast on `_willow_server_ops`
in the current code.

- **Replay worker** (requires SERVER_OPS broadcast handle): after
  responding to a peer's `WorkerRequest::Sync` with the computed
  delta (or `Snapshot`), emit a marker on `_willow_server_ops` with
  the hash of the last event in that response, or `None` when it had
  nothing to send. The reply itself still goes back as
  `WorkerWireMessage::Response` on `_willow_workers` (worker-to-worker
  RPC); only the `HistorySyncComplete` marker is broadcast on
  SERVER_OPS, where the joining client is subscribed. The replay
  worker today uses a per-author DAG (max-events-per-author default
  1000) and an LRU server cap of 1000 — see
  `crates/replay/src/role.rs:18,64`.
- **Storage worker** (requires SERVER_OPS broadcast handle):
  snapshot the watermark *first*, then send every row matching the
  watermark in one `WorkerResponse::SyncBatch` reply on `_willow_workers`
  via the `sync_since` path (`crates/storage/src/store.rs:289-347`,
  `ORDER BY seq ASC`, capped at `SYNC_BATCH_LIMIT = 10_000` rows per
  reply) — not the paginated `history()` path
  (`store.rs:184-238`, `ORDER BY seq DESC`), which is for explicit
  user-initiated history fetches. The marker is emitted on
  `_willow_server_ops` after the SyncBatch reply is sent so any client
  subscribed to SERVER_OPS sees it. Events that arrive at the worker
  after the watermark was taken are not part of this reply; once the
  worker grows a path to forward them as ordinary gossip after the
  marker (forward-looking — no such path exists in storage today),
  they will be live, not historical.
- **Peer-to-peer** (**conditional** on the per-author DAG sync wire
  protocol landing): `SyncMessage` is defined in
  `crates/state/src/sync.rs:37-44` but has zero producers or
  consumers anywhere on the network today — there is no
  `WireMessage::Advertise`/`Request`/`Response` variant, no handler.
  When that protocol is wired up per
  `2026-04-01-per-author-merkle-dag-state-design.md`, peer providers
  emit a marker after the
  `SyncMessage::Advertise(HeadsSummary)` →
  `SyncMessage::Request(Vec<AuthorRequest>)` →
  `SyncMessage::Response(Vec<Event>)` exchange completes and before
  relaying any live events to the new neighbour. Until then, only
  the worker-emitted markers exist.

  > **Resolved 2026-05-30** (PR #664): peer-to-peer marker emission
  > **landed** — but via the gossip `SyncRequestV2` responder (the
  > heads-based protocol from `2026-04-24-negentropy-sync.md`, plan PR 4),
  > not the never-wired `SyncMessage` variant above. The gossip responder in
  > `crates/client/src/listeners.rs` now broadcasts a `HistorySyncComplete`
  > on SERVER_OPS after a successful serve, with `last_event_hash` = the hash
  > of the last streamed event and `stream_generation` = a stable
  > per-session value (`ListenerCtx::history_stream_generation`, generated
  > once per `connect()`). This is **required**, not optional: worker-only
  > emission is unobservable by gossip clients (web clients backfill over
  > SyncRequestV2 and never touch the worker ALPN `WorkerRequest::Sync`
  > path), so the EOSE feature was dead end-to-end for clients until the
  > responder emitted the marker. The receiver's trust gate honors the
  > owner/admins (see the gate note in `2026-04-24-negentropy-sync.md`), so
  > an owner-served backfill produces an observable `HistorySynced`. The
  > stable `stream_generation` lets the receiver's `(provider,
  > stream_generation)` dedup make repeated serves on reconnect idempotent.
  > See `docs/reports/2026-05-30-heads-sync-owner-serve-and-eose-emission.md`.

The provider tracks which neighbours it has already sent a marker to
in this `stream_generation` so a reconnect loop cannot spam the UI.

## Sharp edges

- **Silent truncation.** Nostr's EOSE deliberately does *not* guarantee
  completeness against `max_limit` truncation; clients are expected to
  consume a partial reply silently. Willow's contract is intentionally
  stronger: `last_event_hash` lets clients detect truncation against
  the per-author 1000-event chain cap or any other ring-buffered
  source. If
  the DAG cursor the client was tracking does not link to
  `last_event_hash` through a known parent chain, the client SHOULD
  fall back to the storage worker or a peer with a deeper history
  before flipping the UI flag.
- **Provider lies.** A compromised `SyncProvider` can emit a marker
  before actually flushing its history. The worst-case effect is a
  stale UI; it cannot forge events (signatures still verify). Clients
  MAY set a floor (e.g. "wait at least 200 ms and see at least one
  event *or* one marker") to cap the damage.
- **Filter-change analogue.** Nostr has to define what EOSE means
  when the filter changes mid-subscription; Willow does not, because
  a topic change is literally a new subscription. Joining a new
  channel = new `TopicId` = fresh marker cycle. This is cleaner
  precisely *because* Willow is topic-scoped.
- **Offline peers and resumption.** A client that disconnects and
  reconnects requests history again; providers emit a new marker with
  a new `stream_generation`. The client SHOULD discard markers whose
  `(provider, stream_generation)` is older than one it has already
  seen since the last `NeighborUp` for that provider.

## Testing

**State-level** (`crates/state/src/tests.rs`, `just test-state`): no
state-machine change — `HistorySyncComplete` is a wire-layer
`WireMessage` variant, not an `EventKind`. A round-trip test added
to `crates/common/src/wire.rs`'s test module (alongside the existing
`pack_unpack_*_round_trip` cases) covers serde + envelope signing +
size bounds.

**Client-level** (`crates/client/src/lib.rs` test module,
`just test-client`):

- emitting a marker from `MemNetwork` produces exactly one
  `ClientEvent::HistorySynced` per `(topic, provider)` pair;
- a marker from an untrusted peer produces **no** event;
- reconnect with a new `stream_generation` re-emits; reconnect with
  the same `stream_generation` does not;
- a `last_event_hash` mismatch between the marker and the last
  received event produces a `HistorySynced { still_pending: _ }`
  with a warning logged but no false-positive completion.

**Relay-level** (`crates/relay/tests/`, `just test-relay`): verify the
relay forwards `WireMessage::HistorySyncComplete`-bearing envelopes
unchanged (it should already — the relay is opaque at the wire level,
see `crates/relay/src/lib.rs:8-22` — but the test pins that contract
so a future size-bounded filter does not silently drop the marker).

**Browser-level** (`crates/web/tests/browser.rs`, `just test-browser`):
the loading spinner component hides on receipt of `HistorySynced`
for the active channel's topic and stays hidden across subsequent
live events.

## Interaction with existing specs

- `2026-04-01-per-author-merkle-dag-state-design.md` — the
  `SyncMessage::Advertise` / `Request` / `Response` exchange (over
  per-author `HeadsSummary`) produces a natural "I have sent you
  everything in my frontier" moment; `HistorySyncComplete` is the
  wire encoding of that moment. **Blocks-on:** the peer-to-peer
  provider class enumerated in "Multiple providers" cannot ship until
  `2026-04-01-per-author-merkle-dag-state-design.md` lands a
  `WireMessage` variant for `SyncMessage` and the per-channel-topic
  handshake to drive it. (This spec lands only
  `WireMessage::HistorySyncComplete`; the `SyncMessage` wire-plumbing
  is owned by the per-author DAG spec.) The replay/storage worker
  provider classes do **not** depend on that work and can ship first.
- `2026-04-12-state-authority-and-mutations.md` — markers do **not**
  flow through `apply_event`. They are not authority-bearing; they
  cannot grant, revoke, or mutate `ServerState`. A malicious marker
  is a UX bug at worst, never a state bug.
- `2026-03-27-worker-nodes-design.md` — replay and storage workers
  gain a new emission obligation documented above plus a new outbound
  broadcast handle for `_willow_server_ops` (see "Which gossip topic
  carries the marker"); no change to their trust status.

## Open questions

> **Resolved 2026-05-28** (plan `2026-05-28-relay-upgrade-bundle.md`):
> - **Q1 (`last_event_hash` mandatory?)** → keep **optional**; `None`
>   cleanly encodes the empty-store case without a sentinel hash.
> - **Q5 (`stream_generation` counter vs random?)** → **random `u64`**
>   (from the existing `rand`/`ChaCha20Rng` dep); equality-based dedup
>   needs no ordering, and randomness avoids the "did I bump it?" bug
>   class and counter persistence across restarts.
> - **`SyncCompleted` vs `HistorySynced` reconciliation** → **Option B
>   (additive)**: introduce `ClientEvent::HistorySynced`, keep
>   `SyncCompleted` as session-wide progress.
> - **Peer-to-peer provider class** → ~~deferred~~ **landed 2026-05-30
>   (PR #664)**. Originally deferred to a follow-up after the heads-sync
>   responder/receiver landed (plan PR 4); the replay + storage worker
>   provider classes shipped first (plan PR 5). It is now in: the gossip
>   `SyncRequestV2` responder emits the marker after a successful serve
>   (owner/admin/granted providers), because worker-only emission turned out
>   **unobservable** by gossip clients — the EOSE feature was dead end-to-end
>   for clients without it. See the "Provider-side emission" note above and
>   `docs/reports/2026-05-30-heads-sync-owner-serve-and-eose-emission.md`.

1. Should `last_event_hash` be mandatory rather than optional? Making
   it required forces providers to decide "am I empty?" vs "am I done
   streaming?" explicitly, at the cost of one more type-level state
   in the provider.
2. Do we want a separate `HistorySyncFailed { topic, provider,
   reason }` message, or is absence-of-marker + a client-side timeout
   sufficient? Nostr gets away with absence; Willow has richer error
   information available at the provider.
3. Should the marker carry the provider's current state hash, so
   clients can cross-check against the "majority-agreed state"
   mechanism from `CLAUDE.md`'s trust model? This would unify history
   completion and state convergence into one signal.
4. Per-channel vs per-server scope: today the per-channel `TopicId`
   is the unit of subscription a joining client sees historical chat
   on (the same `TopicId` type is also used for SERVER_OPS, WORKERS,
   PROFILES, and voice topics, but those are not what the UI's
   "loading channel history" spinner gates on). A server-wide marker
   ("all channels have caught up") would simplify the join flow. Do
   we want both?
5. Is `stream_generation: u64` overkill? A random `u64` (e.g. from
   `ChaCha20Rng`) would avoid the "did I remember to bump it?" bug
   class, at the cost of not being orderable.
6. Should markers be rate-limited at the relay? Today the relay is
   content-agnostic, but a compromised provider could emit markers
   in a tight loop and force every subscribed UI to re-render.
