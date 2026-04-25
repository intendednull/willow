# History Sync Completion Signal (EOSE-equivalent)

> **One-sentence summary:** Add an explicit `HistorySyncComplete` message
> to `willow-transport` so clients can tell — per topic, per provider —
> when backfill has finished and the gossip feed is live, eliminating
> the "is history still loading?" ambiguity that today's protocol
> silently imposes on every freshly-joined client.

## Motivation

Today a joining client subscribes to a server's gossip topic and starts
receiving events: some are historical (replayed by the replay worker
from its 1000-event ring buffer, or streamed by the storage worker from
SQLite), some are live. The client has no way to tell these apart.
The UI therefore has to guess — usually with a loading spinner that
stays up for a fixed debounce window, or that flips off the first time
the event stream goes quiet for N ms. Both heuristics are wrong on a
slow network and both waste milliseconds on a fast one.

Nostr solved this cleanly with NIP-01's `EOSE` marker: `["EOSE",
<subid>]` is a zero-payload frame the relay emits after all stored
events for a subscription have been flushed, drawing a bright line
between backfill and live tail. We want the same property in Willow.
The details differ — Willow has no subscription ids, no single
authoritative relay, and two distinct sync providers — but the user-
visible invariant we need is identical: **after the client has seen
one of these markers from a trusted provider, it knows the loading
state has resolved and the UI can commit.**

## Wire format

Introduce a new `MessageType::HistorySyncComplete = 7` variant in
`crates/transport/src/lib.rs:64` alongside the existing seven tags.
The payload is a new struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistorySyncComplete {
    /// The TopicId (blake3 of the canonical topic string) this marker applies to.
    pub topic_id: [u8; 32],
    /// Hash of the last event the provider streamed before emitting the marker,
    /// or `None` if the provider had zero stored events for this topic.
    pub last_event_hash: Option<EventHash>,
    /// Monotonically-increasing cursor scoped to (topic_id, provider).
    /// Incremented when the provider restarts and re-streams from zero.
    pub stream_generation: u64,
}
```

`topic_id` is carried explicitly (rather than implicit in the gossip
topic) so that the marker survives relay-bridge forwarding and so a
single audit log can correlate markers across topics. The provider's
identity is deliberately **not** carried in the payload: the marker
rides the same Ed25519-wrapped envelope as ordinary events
(`crates/identity/src/lib.rs`), so the receiver derives the provider's
`EndpointId` from the verified envelope signer at unpack time. This
forecloses a class of relay-rewrite / MITM attacks where a separate
`provider_peer` field could be edited to attribute the marker to a
different trusted provider, and saves the bytes besides.
`last_event_hash` lets the client detect truncation: if the last event
the client *received* from that provider does not match the hash in
the marker, the client MUST treat the sync as incomplete (see "Sharp
edges" below). `stream_generation` exists so a provider that crashes
and restarts mid-stream cannot confuse a client still holding a stale
marker from the previous run. The semantics are inspired by Nostr's
one-EOSE-per-REQ pattern; the actual contract is adapted to Willow's
subscription-less topic gossip and is intentionally stronger than
NIP-01's (see "Sharp edges").

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

The marker is sent as a regular `TopicHandle::broadcast` frame on the
topic's gossip mesh — the same path ordinary events take. This costs
one mesh-wide message per `NeighborUp` on the provider, which is
acceptable overhead for a per-(topic, provider, generation) deduped
marker; existing peers ignore markers whose
`(provider, stream_generation)` they have already observed. Relays
forward the marker by their existing topic-routing rules: see the
`Trust model` section in the module-level documentation of
`crates/relay/src/lib.rs`, which establishes the relay as
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

A single server has, in the current architecture, three candidate
providers for history:

| Provider | Source | Completeness |
|----------|--------|--------------|
| **Replay worker** | in-memory ring buffer, last 1000 events | lossy, recent |
| **Storage worker** | SQLite archival log | authoritative, long tail |
| **Peer** | any peer with more state than us | opportunistic |

A joining client can receive up to three `HistorySyncComplete` markers
for the same topic. We define "caught up" as a **first-trusted-wins**
rule, with a pluggable policy:

1. **Default policy (fast UI):** the client considers itself caught up
   for a topic as soon as it has received a valid marker from **any
   peer granted `SyncProvider` permission** (the relay-worker role in
   the state machine). This minimises perceived load time and matches
   the UX the feature exists to deliver.
2. **Strict policy (opt-in):** wait until markers have arrived from
   **a majority of currently-connected `SyncProvider` peers**, or from
   the storage worker specifically if one is in the trust list. This
   is the right choice for clients that are about to take an offline
   snapshot or archive state.

Untrusted peers' markers are ignored entirely — accepting them would
let any peer prematurely flip the UI's loading flag off.

## Client API

Emit a new variant on `ClientEvent` in
`crates/client/src/events.rs:10`:

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
to `MessageReceived`. The natural consumer is the
`SyncCompleted { ops_applied }` variant already at
`crates/client/src/events.rs:48`: that existing event is session-wide
and is emitted after a batch of operations lands, whereas
`HistorySynced` is topic-scoped and marks the boundary, not the work.
Both are kept — they answer different questions.

## Provider-side emission

Emission is triggered by `NeighborUp` events on the provider's
`TopicEvents` stream (`crates/network/src/traits.rs:52`):

- **Replay worker**: after flushing its 1000-event ring buffer (or
  zero events if empty), emit a marker with the hash of the last
  flushed event, or `None` when empty.
- **Storage worker**: snapshot the watermark *first*, stream every
  row up to that watermark via `SELECT ... ORDER BY seq ASC`, emit
  the marker. Events that arrive at the worker after the watermark
  was taken are forwarded as ordinary gossip after the marker — they
  are live, not historical.
- **Peer-to-peer**: emit after the DAG-diff exchange from
  `2026-04-01-per-author-merkle-dag-state-design.md` completes and
  before relaying any live events to the new neighbour.

The provider tracks which neighbours it has already sent a marker to
in this `stream_generation` so a reconnect loop cannot spam the UI.

## Sharp edges

- **Silent truncation.** Nostr's EOSE deliberately does *not* guarantee
  completeness against `max_limit` truncation; clients are expected to
  consume a partial reply silently. Willow's contract is intentionally
  stronger: `last_event_hash` lets clients detect truncation against
  the 1000-event replay buffer or any other ring-buffered source. If
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
  seen since the last `NeighborUp` for that provider — where
  `provider` is the verified envelope signer.

## Testing

**State-level** (`crates/state/src/tests.rs`, `just test-state`): no
state-machine change — `HistorySyncComplete` is a transport-layer
message, not an `EventKind`. A single round-trip test in
`crates/transport/src/lib.rs` covers serde + size bounds.

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
relay forwards `MessageType::HistorySyncComplete` bytes unchanged (it
should already — the relay is opaque at the message-type level — but
the test pins that contract so a future size-bounded filter does not
silently drop the marker).

**Browser-level** (`crates/web/tests/browser.rs`, `just test-browser`):
the loading spinner component hides on receipt of `HistorySynced`
for the active channel's topic and stays hidden across subsequent
live events.

## Interaction with existing specs

- `2026-04-01-per-author-merkle-dag-state-design.md` — the DAG-diff
  protocol produces a natural "I have sent you everything in my
  frontier" moment; `HistorySyncComplete` is the wire encoding of
  that moment.
- `2026-04-12-state-authority-and-mutations.md` — markers do **not**
  flow through `apply_event`. They are not authority-bearing; they
  cannot grant, revoke, or mutate `ServerState`. A malicious marker
  is a UX bug at worst, never a state bug.
- `2026-03-27-worker-nodes-design.md` — replay and storage workers
  gain a new emission obligation documented above; no change to their
  trust status.

## Open questions

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
4. Per-channel vs per-server scope: today `TopicId` is per-channel,
   but a server-wide marker ("all channels have caught up") would
   simplify the join flow. Do we want both?
5. Is `stream_generation: u64` overkill? A simple `ChaCha20`-derived
   nonce would avoid the "did I remember to bump it?" bug class, at
   the cost of not being orderable.
6. Should markers be rate-limited at the relay? Today the relay is
   content-agnostic, but a compromised provider could emit markers
   in a tight loop and force every subscribed UI to re-render.
