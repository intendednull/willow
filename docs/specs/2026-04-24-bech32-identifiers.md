# Bech32-with-HRP User-Facing Identifiers

> **One-sentence summary:** All Willow identifiers that can appear in a
> UI, a URL, a paste buffer, or a log line are encoded as bech32m
> strings with a type-tagging human-readable prefix (HRP). The wire
> format (bincode over iroh gossip) is unchanged — bech32 is strictly
> a display-and-input boundary, inspired by Nostr's NIP-19.

## Motivation

Willow currently displays identifiers in three incompatible encodings:

| Identifier | Current encoding | Source |
|---|---|---|
| `EndpointId` (Ed25519 pubkey) | 64-char lowercase hex | `iroh_base::EndpointId` re-export (`crates/identity/src/lib.rs:39`) |
| `EventHash` (SHA-256 digest) | 64-char lowercase hex | `crates/state/src/hash.rs:52` |
| `TopicId` (blake3 of topic name) | never displayed; internal | `crates/network/src/topics.rs:12` |
| Invite code | base64 of `pack()`ed `Invite` struct | `crates/client/src/invite.rs:99` |
| Join link | base64 `JoinToken` in URL fragment | `docs/specs/2026-03-27-shareable-join-links-design.md:42` |

Three problems:

1. **No type signal.** A 64-hex string could be a pubkey, an event
   hash, a blob digest, or a topic id. Users can't tell by looking.
2. **Silent cross-paste.** An `EventHash` dropped into an invite
   field fails deep inside deserialization, with no helpful error.
3. **No forward structure.** Invite codes and join tokens carry
   ad-hoc bincode blobs; adding a relay hint or expiry bumps the
   format in a way existing clients cannot parse.

Bech32 with HRPs solves all three at the cost of ~10% longer strings
and one new dep. The HRP is a built-in type tag — clients can reject
mis-typed pastes before any crypto work. The checksum catches typos.
TLV bodies give a forward-compatible slot for hints.

## Non-goals

- **No change to the wire format.** Bincode stays on the wire.
  `willow-transport::pack/unpack`, `Event`, `WireMessage`, `Invite`
  still serialize raw bytes. Mirrors NIP-19's rule that bech32
  "MUST NOT be used in NIP-01 events".
- **No change to cryptographic primitives.** Ed25519 keys, SHA-256
  event hashes, ChaCha20-Poly1305 seals keep their byte form.
- **No key material on screen.** We do not introduce an `nsec`
  equivalent. Private keys never have a user-facing string form.

## Bech32 vs bech32m

Willow picks **bech32m** (BIP-350) for every HRP.

Bech32 (BIP-173) has a known flaw: inserting or removing `q`
characters immediately before a trailing `p` does not invalidate the
checksum. Fixed-length payloads dodge this (length alone rejects
malformation) but Willow's TLV identifiers (`winv`, `wevent`,
`wchan`) are explicitly variable length. Bech32m swaps the checksum
constant from `1` to `0x2bc830a3`, closing that class of errors
while preserving bech32's substitution detection.

Nostr chose plain bech32 before TLV variants were common and cannot
migrate without a flag day. Willow is pre-1.0; picking bech32m now
avoids the trap. Every HRP uses bech32m — consistency beats
micro-optimisation.

## HRP table

Willow reserves the `w` prefix space. Every HRP is 4–7 ASCII lowercase
characters.

| HRP | Payload | Length | Shape |
|---|---|---|---|
| `wpeer` | Ed25519 public key (`EndpointId`) | 32 B | raw |
| `wserver` | genesis `EventHash` of a server | 32 B | raw |
| `wevent` | `EventHash` + optional hints | 32 B + TLV | TLV |
| `wchan` | server id + channel id | TLV | TLV |
| `winv` | invite link pointer (link id + relay hints) | TLV | TLV |
| `wrelay` | relay endpoint URL | var | TLV |
| `wblob` | iroh-blobs content hash | 32 B | raw |

**HRP-length bikeshed.** 4-char forms (`wpub`, `wsrv`, `wev`) are
~20% shorter but harder to scan. We pick 5–7 char forms because the
32-byte body already dominates length and longer HRPs read cleanly
in logs ("that's a `wserver`").

## TLV format

Compound HRPs (`wevent`, `wchan`, `winv`, `wrelay`) use the same
Type-Length-Value convention as NIP-19: one byte type, one byte
length (0–255), then `length` bytes of value. Types repeat. Unknown
types are ignored on decode. Length-0 values are legal.

| Type | Name | Description | Repeatable |
|---|---|---|---|
| 0 | `Special` | Primary identifier bytes (e.g. event hash, invite link_id) | No |
| 1 | `Relay` | UTF-8 relay URL hint | Yes |
| 2 | `Author` | 32-byte `EndpointId` of originator | No |
| 3 | `Kind` | big-endian `u32` event kind discriminator | No |
| 4 | `Server` | 32-byte `wserver` id the entity lives under | No |
| 5 | `Channel` | blake3 channel id within a server | No |
| 6 | `ExpiresAt` | big-endian `u64` ms-since-epoch | No |

Types 0–3 match NIP-19 numbers for reviewer familiarity. Types 4–6
are Willow-specific. Unknown-type values are dropped; the HRP still
identifies intent and the `Special` payload is still extractable.

## Concrete encodings

- `wpeer1<52 chars>` — 32 B key + 6-char checksum, ~62 chars total.
- `wserver1<52 chars>` — same length, different HRP.
- `wevent1<var>` — TLV with type-0 set to the 32-byte `EventHash`;
  optional `Relay`, `Author`, `Server` hints.
- `winv1<var>` — TLV with type-0 = UUID `link_id` (16 B), ≥1
  `Relay`, optional `ExpiresAt`, required `Author`. Replaces the
  base64-packed `JoinToken`.
- `wchan1<var>` — TLV with `Server` + `Channel`; used in deep links
  like `https://willow.app/#go=wchan1…`.

## Where the boundary lives

Bech32 strings appear at four surfaces only:

| Surface | Direction | File |
|---|---|---|
| Settings "Invite Links" list | encode | `crates/web/src/components/settings.rs:69` |
| Join page fragment parser | decode | join link handler (replaces `JoinToken` base64 at `docs/specs/2026-03-27-shareable-join-links-design.md:42`) |
| Message "copy id" UI | encode | `crates/web/src/components/message.rs` |
| Debug/logging `Display` impls | encode | `crates/state/src/hash.rs:52`, `EndpointId` |

Everything behind those surfaces — `Event.author`, `Event.hash`,
gossip payloads, storage keys, the Merkle DAG — stays raw bytes.
`FromStr` impls on `EndpointId` and `EventHash` accept bech32 with
the correct HRP, and (during migration) hex as a fallback.

## API surface: the `willow-ids` crate

A new leaf crate `willow-ids` holds the encode/decode logic,
depending on `bech32 = "0.11"` (bech32m-capable, `no_std`,
WASM-clean) and nothing else in the Willow graph. A dedicated crate
— rather than extending `willow-identity` — keeps the arrow one-way:
`identity`, `state`, `messaging`, `network` all depend on
`willow-ids`, never the reverse.

```rust
pub enum Hrp {
    Peer,     // wpeer
    Server,   // wserver
    Event,    // wevent
    Channel,  // wchan
    Invite,   // winv
    Relay,    // wrelay
    Blob,     // wblob
}

pub enum DecodeError {
    WrongHrp { expected: Hrp, got: String },
    InvalidChecksum,
    InvalidLength,
    MalformedTlv,
    UnknownVariant,
}

pub fn encode_peer(id: &EndpointId) -> String;
pub fn decode_peer(s: &str) -> Result<EndpointId, DecodeError>;

pub fn encode_event(h: &EventHash, hints: &EventHints) -> String;
pub fn decode_event(s: &str) -> Result<(EventHash, EventHints), DecodeError>;

pub fn encode_invite(inv: &InvitePointer) -> String;
pub fn decode_invite(s: &str) -> Result<InvitePointer, DecodeError>;

/// Inspect the HRP of an unknown string without decoding the body.
pub fn sniff(s: &str) -> Option<Hrp>;
```

`sniff` exists so paste handlers can render "That looks like a peer
id, not an invite" before doing any work.

## Interop and migration

The existing invite-code (base64) and join-link (base64 JSON)
formats stay parseable by decoders for two release cycles. New code
only emits bech32. The decoder precedence is:

1. Starts with a known Willow HRP followed by `1` → bech32m decode.
2. Pure hex, length 64 → legacy `EndpointId` / `EventHash`.
3. Base64-looking → legacy invite / join token.
4. Otherwise: reject.

`EndpointId::Display` and `EventHash::Display` switch to emit
`wpeer1…` / `wevent1…` immediately; hex stays accepted on input.
After the grace period, legacy decoders are deleted.

Storage (SQLite, IndexedDB) keeps raw bytes — nothing on disk is
bech32 either.

## Testing

All tests live in `willow-ids/src/tests.rs` plus integration tests in
the consuming crates.

| Test | Target |
|---|---|
| Round-trip vectors | every HRP: `encode → decode → bytes equal` |
| Known-answer vectors | hard-coded `wpeer1…` / `wevent1…` strings parsed to fixed byte arrays, protects against dependency upgrades silently changing output |
| Wrong-HRP rejection | `decode_peer("wserver1…")` returns `WrongHrp` without touching the body |
| Bech32 vs bech32m | a plain-bech32 string with a valid-looking body is rejected (ensures we didn't silently accept both variants) |
| TLV unknown-type ignore | `decode_event` with a type-99 TLV returns Ok and ignores it |
| Length bounds | >5000-char input rejected early |
| Malformed TLV | length byte overruns the payload → `MalformedTlv` |
| Legacy fallback | during migration: 64-char hex still decodes via `EndpointId::from_str` |
| Property test | random bytes → encode → decode → round-trip |

Browser tests in `crates/web/tests/browser.rs` cover one end-to-end
path: paste a `winv1…` string into the join page and verify the
JoinRequest fires with the decoded link id.

## Open questions

1. **bech32m everywhere, or plain bech32 on `wpeer` for Nostr-style
   interop?** For: reuses `npub1…` tooling. Against: mixing variants
   per HRP is a footgun for implementers.
2. **HRP length — `wpeer` or `wpub`?** Shorter is denser, longer is
   more readable. Pick one and stay consistent.
3. **`wserver` = genesis `EventHash` or owner `EndpointId`?**
   Genesis is stable across ownership changes but unknown until the
   server exists; owner is known up front but not invariant.
4. **Add `wsecret`/`nsec`-equivalent for key backup/export flows, or
   keep private keys off-screen forever?** (Current stance: off.)
5. **URL embedding — fragment (`#go=wevent1…`) or path
   (`/e/wevent1…`)?** Fragment is fully client-side; path allows
   server-rendered previews.
6. **Interop grace period length.** Two releases is a guess; depends
   on whether any external tool consumes the base64 invite format.

## Checklists

### Adding a new HRP

1. Add a variant to `willow_ids::Hrp` and the HRP string constant.
2. Write an `encode_X` / `decode_X` pair, with TLV types documented
   in a comment above the function.
3. Add round-trip, known-answer, and wrong-HRP tests.
4. Update the HRP table in this spec.
5. If the new id appears in a URL, extend the `#go=` parser.

### Adding a new TLV type

1. Assign the next unused number in the TLV table.
2. Implement encode/decode; decoders ignore unknowns.
3. Forward-compat test: old clients decode new strings.
