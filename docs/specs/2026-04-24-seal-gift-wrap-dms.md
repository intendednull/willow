# Seal + Gift-Wrap DMs for Metadata Privacy

> **One-sentence summary:** Willow adopts a Nostr-style three-layer
> (rumor → seal → gift wrap) direct-message format that hides sender,
> recipient, content, and timing from passive observers and relays,
> delivered over a dedicated inbox topic and kept off the server's
> event DAG via ephemeral-author wrap events.

## Motivation

Willow's existing encryption (`crates/crypto/src/lib.rs:208–241`) is
excellent for **groups**: a shared [`ChannelKey`] and a forward-secret
[`KeyRatchet`] protect message bodies in a channel whose membership is
already public. It solves **content confidentiality** inside a known
set of participants. It does not solve **metadata privacy** for 1:1 or
small-group conversations:

- Channel topics (`crates/network/src/topics.rs:42`) leak the channel
  membership graph.
- Every `Event` (`crates/state/src/event.rs:184–204`) carries a clear-
  text `author` [`EndpointId`] and a per-author `seq`/`prev` chain,
  so anyone on the wire learns who talks, when, and how often.
- There is no DM primitive at all: users today spin up a "private
  channel" which still publishes a channel-creation event, a member
  set, and a gossip topic.

Willow therefore needs a DM format whose **on-wire shape reveals only
"someone sent someone a thing, of roughly this size, at roughly this
time."** Nostr's NIP-44/59/17 stack is a well-studied template; this
spec adapts it to Willow's signed per-author-DAG model.

## Threat model

| Adversary | Sees today | Sees after this spec |
|-----------|-----------|----------------------|
| Passive network observer | Sender, recipient channel, timing, size | Ephemeral sender only, size bucket, approximate timing (±2d) |
| Relay / worker (`crates/relay`) | Full channel history, clear authors | Ciphertext blobs addressed to a recipient hint; cannot link to real sender |
| Curious peer not in the DM | Full channel history | Nothing (gift wrap ignored; can't decrypt seal) |
| Recipient's device post-compromise | — | Historical plaintext (no PCS — see non-goals) |
| Quantum adversary | — | Eventually everything (X25519 is not PQ — see non-goals) |

**This hides:** the real sender's `EndpointId`, the content, the
recipient set for group DMs, and correlation between DM events
and the sender's main author chain.

**This does NOT hide:** the fact that *someone* sent *a* DM of roughly
some size to *some* recipient, the recipient's pubkey on the
delivery topic, IP addresses at the transport layer, precise timing
against an adversary that logs every gift-wrap event, or the
existence of a conversation once a key is later compromised.

## Layer structure

Three layers, mirroring NIP-59 but re-shaped for Willow's event model:

```
   DMRumor  ── no signature, carries the real author & content
      │
      ▼ NIP-44-style encrypt to recipient pubkey, sign with real author
   EventKind::DMSeal { ciphertext }        (real author's DAG, seq++)
      │
      ▼ NIP-44-style encrypt to recipient pubkey, sign with EPHEMERAL key
   EventKind::DMGiftWrap { ciphertext, recipient_hint }
                                              (ephemeral author's DAG, seq=1)
```

### Rumor (unsigned, off-DAG)

```rust
// crates/messaging/src/dm.rs (new)
pub struct DMRumor {
    pub author_peer_id: EndpointId,   // real author
    pub timestamp_hint_ms: u64,       // jittered up to 2d in the past
    pub content: Content,             // reuse willow_messaging::Content
}
```

`DMRumor` is deliberately **not** a full `Event`
(`crates/state/src/event.rs:184`): an `Event` carries `seq`, `prev`,
`deps`, and `sig` that either tie the rumor to the real author's
chain (defeating the point) or force a parallel chain (adds
complexity without benefit). A rumor is serialized with `bincode`
via `willow-transport` and encrypted raw.

**No signature.** This preserves deniability: a leaked rumor cannot
be cryptographically attributed to its author.

### Seal (`EventKind::DMSeal`)

A new `EventKind` variant on the real author's DAG:

```rust
EventKind::DMSeal {
    ciphertext: Vec<u8>,           // NIP-44 v1 payload over DMRumor
}
```

Rules:

- Signed by the **real author** and inserted at the next `seq` on
  their chain, like any other event.
- `deps` MUST be empty (no cross-author dependency leaks a link to
  the recipient).
- No field names the recipient; the seal's only public metadata is
  "author X produced a DMSeal at seq N."
- `required_permission()` in `crates/state/src/materialize.rs` maps
  this variant to `None` (unrestricted — DMs aren't server actions).
- `apply_mutation()` treats it as **inert**: the seal exists on the
  author's DAG for local recall but does **not** change
  `ServerState`.

Verifier requirement: a recipient that decrypts a `DMSeal` MUST
check that the decrypted `DMRumor.author_peer_id` equals the seal
event's signed `author`. If it doesn't, the rumor is discarded. This
prevents "pubkey swap" impersonation where an attacker replaces the
rumor author field.

### Gift wrap (`EventKind::DMGiftWrap`)

A new `EventKind` variant authored by a **fresh ephemeral Ed25519
identity generated per recipient per message**:

```rust
EventKind::DMGiftWrap {
    ciphertext: Vec<u8>,           // NIP-44 v1 payload over the Seal event bytes
    recipient_hint: EndpointId,    // recipient pubkey (routing only)
}
```

Rules:

- The wrapping `Event.author` is the ephemeral key, not the real
  sender. `seq = 1`, `prev = EventHash::ZERO`, `deps = []`.
- The ephemeral key is used exactly once and then discarded.
- `timestamp_hint_ms` is independently jittered up to 2 days in the
  past (the seal's timestamp is also jittered — independently).
- `recipient_hint` is the only metadata needed for delivery routing.
  It does **not** prove the recipient was actually addressed (an
  attacker can forge a hint); it's a fast filter.

Because the gift wrap lives on a single-event ephemeral-author DAG,
it does not interfere with any real peer's `seq`/`prev` chain and
does not need DAG-level merge (`crates/state/src/materialize.rs`).

## Payload format (NIP-44 v1, Willow-flavored)

Mirrors NIP-44 v2 byte-for-byte except for the HKDF labels; version
byte is `0x01` to reserve future migration independently of Nostr.

| Field | Size | Notes |
|-------|------|-------|
| `version` | 1 B | `0x01` |
| `nonce` | 32 B | CSPRNG, also used as HKDF-expand `info` |
| `ciphertext` | variable | ChaCha20 (counter=0) output, input is padded plaintext |
| `mac` | 32 B | HMAC-SHA256(hmac_key, nonce ‖ ciphertext) |

Key derivation:

```
shared     = X25519(ed25519_to_x25519(sender_sk), ed25519_to_x25519(recipient_pk))
conv_key   = HKDF-Extract(salt = "willow-dm-v1",        ikm = shared)
expanded   = HKDF-Expand(prk  = conv_key, info = nonce, L = 76)
chacha_key = expanded[0..32]
chacha_iv  = expanded[32..44]           // 12 bytes
hmac_key   = expanded[44..76]
```

Ed25519→X25519 reuses `identity_to_x25519` /
`ed25519_public_to_x25519` (`crates/crypto/src/lib.rs:318–343`). The
HKDF label `"willow-dm-v1"` distinguishes this derivation from the
existing `"willow-channel-key-wrap"` label
(`crates/crypto/src/lib.rs:360`).

Padded plaintext layout matches NIP-44:

```
[u16 BE length][plaintext][zero padding]
```

with power-of-two bucket sizes (min 32 B):

```
if len ≤ 32: bucket = 32
else:        next = 2^(floor(log2(len-1)) + 1)
             chunk = max(32, next / 8)
             bucket = chunk * (ceil(len / chunk))
```

MAC intentionally uses encrypt-then-HMAC (not Poly1305) so the
construction mirrors NIP-44 exactly, keeping test vectors portable.

## Delivery topic

Two candidates, both with leaks:

| Option | Pro | Con |
|--------|-----|-----|
| Per-recipient topic `_willow_inbox/<blake3(recipient_pk)>` | Small fan-out; recipient only subscribes to their own inbox | Observer learns which pubkeys are active DM recipients |
| Shared `_willow_inbox` topic | No per-pubkey topic-id leak | Every wrap floods every peer; DoS/bandwidth cost |

**This spec specifies the per-recipient topic.** The topic id is
`topic_id(&format!("_willow_inbox/{}", hex(blake3(recipient_pk))))`
(`crates/network/src/topics.rs:12`). Recipients subscribe to their
own inbox on startup. Senders subscribe briefly to publish, then
unsubscribe. The recipient-pubkey leak is judged acceptable because
the pubkey is already public-key material; the shared-inbox DoS is
not.

Future optimization: workers can serve as inbox aggregators that
pull on behalf of offline peers (see *Open questions*).

## State-machine integration

Gift wraps and seals both deserialize to `Event`s and ride the same
wire pipeline (`identity::pack` / `unpack`). The state machine:

1. `dag.insert()` accepts the wrap like any other event (signature
   verifies against the ephemeral author).
2. `apply_event()` in `crates/state/src/materialize.rs` recognises
   `DMGiftWrap` / `DMSeal` and short-circuits: it does **not**
   mutate `ServerState`. Both return `None` from
   `required_permission()` and are listed in the catch-all comment
   alongside `SetProfile` (see
   `docs/specs/2026-04-12-state-authority-and-mutations.md:96–107`).
3. A separate `DMInbox` side-state (local, per-client, not shared)
   collects decrypted rumors, keyed by `(participant_set, rumor_id)`
   where `rumor_id = blake3(bincode(DMRumor))`.
4. **Dedup** happens at the rumor layer: two different ephemeral
   wraps of the same rumor produce identical `rumor_id`s after
   decryption.

Because the wrap's DAG has only one event, `ManagedDag::create_event`
for DM send bypasses the normal per-author chain and instead
instantiates a throwaway identity, produces the seal on the real
chain first, packs it, then constructs the wrap event.

## Multi-recipient DMs

Group DMs produce **N independent gift wraps**, one per recipient
(including the sender themselves so their other devices see a copy,
matching NIP-17's rule). There is **no shared identifier on the
wire**: each wrap has a distinct ephemeral author, distinct
ciphertext, distinct timestamp. Clients reconstruct a "room" locally
by hashing the sorted participant set from the decrypted rumor's
`Content`-level metadata (to be defined in a follow-up — for now
group DM is just "DM to these N pubkeys").

A shared room identifier on-wire would defeat the metadata-hiding
goal; the cost is O(N) wrap events per group message.

## Timestamp jitter

Each layer independently jitters `timestamp_hint_ms`:

```
t_wrap = now_ms - rand(0, 2 * 86_400_000)
t_seal = now_ms - rand(0, 2 * 86_400_000)
```

Independent jitter breaks the obvious "wrap.ts == seal.ts" linkage a
relay could exploit. `HLC` (`crates/messaging/src/hlc.rs`) is **not**
used for DMs — HLC is designed for totally-ordered channel history
and would leak real sender clocks.

## Tests

| # | Property | Location |
|---|----------|----------|
| 1 | NIP-44 KAT round-trip (seal + open a `DMRumor`) | `crates/crypto/src/dm.rs` tests |
| 2 | Pubkey-swap imposter detected (rumor.author ≠ seal.author → rejected) | `crates/crypto/src/dm.rs` tests |
| 3 | Ephemeral key single-use (two wraps of the same seal have distinct authors) | `crates/client/src/dm.rs` tests |
| 4 | Wrap/seal timestamps uncorrelated across 1000 samples (χ² over pairing) | `crates/crypto/src/dm.rs` tests |
| 5 | `DMSeal`/`DMGiftWrap` leave `ServerState` unchanged | `crates/state/src/tests.rs` |
| 6 | Wrong recipient cannot decrypt (AEAD fails) | `crates/crypto/src/dm.rs` tests |
| 7 | Padding buckets match NIP-44 vectors for sizes 1, 32, 33, 145, 600 | `crates/crypto/src/dm.rs` tests |
| 8 | Tampered MAC rejected before ChaCha20 runs | `crates/crypto/src/dm.rs` tests |

## Non-goals

| Property | Why deferred |
|----------|--------------|
| Forward secrecy within a conversation | Would need per-session ratchet (Double Ratchet / MLS). The existing `KeyRatchet` (`crates/crypto/src/lib.rs:102–165`) is group-keyed and not suitable for 1:1 over independent wrap events. |
| Post-compromise security | Same — requires a ratchet with key update primitives. |
| Length hiding beyond bucket size | Requires per-message padding to a global ceiling; prohibitive for long messages. |
| IP-address hiding | Transport concern; iroh relays see endpoint IPs. Out of scope; see transport-privacy work (TBD spec). |
| Precise-timing hiding | Requires cover traffic; not in this spec. |
| Post-quantum confidentiality | Willow uses X25519/Ed25519 throughout; a PQ migration is a whole-codebase spec. |

## Open questions

1. **Per-recipient vs shared inbox topic.** This spec picks
   per-recipient. Is the pubkey-activity leak acceptable long-term,
   or should workers proxy to hide it?
2. **MLS for group DMs.** For groups > ~8, N wraps scale linearly.
   Should a future spec adopt MLS (RFC 9420) and treat this spec as
   the 1:1 fallback?
3. **Worker/relay TTL.** Gift wraps bypass `ServerState` so workers
   (`crates/relay`) have no natural retention signal. Should wraps
   expire after e.g. 30 days, or should each recipient ack a wrap
   to unlock deletion? Current replay-node 1000-event cap would
   drop DMs unpredictably.
4. **Forward secrecy / epoch rotation.** A sibling epoch-key-
   rotation spec would give DMs a sliding-window FS property by
   re-deriving the conversation key on a timer. Prerequisite: a DM
   session abstraction that doesn't exist yet.
5. **Identity-key vs session-key separation.** Willow today uses one
   Ed25519 key for signing, DH (via conversion), and peer ID. For
   DMs we may want a per-device session key bound to the identity
   — both to enable FS and to limit blast radius of a device
   compromise. Should this spec ship with session keys, or land in
   a follow-up?
6. **Ephemeral-author DAG pollution.** Every DM spawns a one-event
   DAG. Do we prune ephemeral-author DAGs after the receiver has
   processed the wrap, or let them accumulate indefinitely at
   workers?

## Checklists

### Adding the `DMSeal` and `DMGiftWrap` variants

1. Add two variants to `EventKind` in `crates/state/src/event.rs`.
2. Leave both out of `required_permission()`; add them to the
   catch-all comment listing unrestricted variants.
3. In `apply_mutation()`, match both variants and return without
   mutating `ServerState`.
4. Add state-machine tests (#5 above) verifying inertness.
5. Add a `willow-crypto` module implementing the NIP-44 v1 payload
   (`seal_dm`, `open_dm`).
6. Add a `willow-client` API: `send_dm(recipients: &[EndpointId], content: Content)`.
7. Add an inbox subscription in `willow-client` startup.

### Adding a new DM content type

Reuse `willow_messaging::Content` — no new types needed. If a DM-
only content variant appears later, add it to `Content` and treat
it identically on the rumor path.
