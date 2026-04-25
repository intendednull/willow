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

Willow uses `w*` HRPs by convention; cross-ecosystem collisions are
mitigated by decoder strictness (HRP allow-list + bech32m checksum +
known-foreign-HRP detection in `sniff()`), not by registration in any
shared registry. Every HRP is 4–7 ASCII lowercase characters.

| HRP | Payload | Length | Shape |
|---|---|---|---|
| `wpeer` | Ed25519 public key (`EndpointId`) | 32 B | raw |
| `wserver` | genesis `EventHash` of a server (definitive) | 32 B | raw |
| `wevent` | `EventHash` + optional hints | 32 B + TLV | TLV |
| `wchan` | server id + channel id | TLV | TLV |
| `winv` | invite link pointer (link id + relay hints + UI metadata) | TLV | TLV |
| `wrelay` | relay endpoint URL | var | TLV |
| `wblob` | iroh-blobs content hash | 32 B | raw |

**HRP-length decision.** 4-char forms (`wpub`, `wsrv`, `wev`) are
~20% shorter but harder to scan. We pick 5–7 char forms because the
32-byte body already dominates length and longer HRPs read cleanly
in logs ("that's a `wserver`"). The type signal in logs is worth the
extra characters; `wpeer` (5 char) wins over `wp` (2 char).

**No `wsecret` HRP, ever.** Private keys do not get a bech32 form.
The `nsec` ↔ `npub` visual-similarity attack class — where a user
copies what they think is a public key and instead pastes a secret
key into a chat or website — is a known disaster vector in the Nostr
ecosystem. Willow secrets stay in the keystore (native) or
non-extractable WebCrypto keys (browser) and never flow through paste
buffers. This closes open question 4 permanently in the negative.

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
| 3 | — | reserved (was NIP-19 `Kind`; Willow has no analog) | — |
| 4–15 | — | reserved for future NIP-19 alignment | — |
| 16 | `Server` | 32-byte `wserver` id the entity lives under | No |
| 17 | `Channel` | blake3 channel id within a server | No |
| 18 | `ExpiresAt` | big-endian `u64` ms-since-epoch | No |
| 19 | `ServerName` | UTF-8 server display name, truncated to 64 bytes | No |
| 20 | `InviterName` | UTF-8 inviter display name, truncated to 64 bytes | No |

Types 0–2 match NIP-19 numbers for reviewer familiarity. Type 3
(NIP-19 `Kind`) has no Willow analog and is reserved rather than
repurposed, to keep parsers that recognise NIP-19 from misreading
Willow strings. Types 4–15 are also reserved for future NIP-19
alignment. Willow-specific types start at 16 to avoid any future
NIP-19 collision on low numbers. Unknown-type values are dropped;
the HRP still identifies intent and the `Special` payload is still
extractable.

## Concrete encodings

- `wpeer1<52 chars>` — 32 B key + 6-char checksum, ~62 chars total.
- `wserver1<52 chars>` — same length, different HRP. The payload is
  the genesis `EventHash` of the server (32 bytes). This is stable
  across ownership transfers and is the canonical server identifier.
- `wevent1<var>` — TLV with type-0 set to the 32-byte `EventHash`;
  optional `Relay`, `Author`, `Server` hints.
- `winv1<var>` — TLV with type-0 = UUID `link_id` (16 B), ≥1
  `Relay`, optional `ExpiresAt`, required `Author`, optional
  `ServerName` and `InviterName`. Replaces the base64-packed
  `JoinToken` for the **pointer** payload only. Encrypted-channel-key
  invite payloads (`InvitePayload` ciphertext) are NOT bech32-encoded
  — wrapping a multi-hundred-byte ciphertext blob in bech32m would
  inflate it ~5× without benefit. Those stay base64.
- `wchan1<var>` — TLV with `Server` + `Channel`; used in deep links
  like `https://willow.app/#go=wchan1…`.

### `winv` carries pre-handshake UI metadata

The current `JoinToken` carries `server_name` and `inviter_name` so
the join page can render "Alice invited you to Acme Server" before
any P2P handshake completes. The bech32 `winv` form preserves this UX
via the `ServerName` (type 19) and `InviterName` (type 20) TLVs, both
optional, both truncated to 64 bytes of UTF-8 (the bech32 length byte
caps each value at 255 bytes regardless; 64 is a sensible UI cap).

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
WASM-clean) and nothing else in the Willow graph. **Truly leaf**:
`willow-ids` imports zero other Willow crates. It operates on plain
`[u8; N]` arrays, an `Hrp` enum, and TLV byte slices — it does not
know what an `EndpointId` or `EventHash` is.

Per-type ergonomic wrappers (`EndpointId::to_bech32()`,
`EventHash::to_bech32()`, `EndpointId::from_bech32(s)`) live in their
owning crates (`willow-identity`, `willow-state`) and call into
`willow-ids` free functions. This avoids the orphan-rule problem
(can't `impl Display for EndpointId` from a foreign crate, and we
don't want to override the upstream `iroh_base::EndpointId::Display`
hex anyway). It also keeps the arrow one-way: `identity`, `state`,
`messaging`, `network` all depend on `willow-ids`, never the reverse.

### `EndpointId::Display` policy

`EndpointId` is re-exported from `iroh_base`, whose `Display` impl is
upstream hex. We do not (and cannot) change that. Therefore:

- `EndpointId::Display` continues to emit lowercase hex.
- All user-visible code paths (UI strings, copy-id buttons, log
  lines that surface in the UI) MUST call `id.to_bech32()` (a free
  function or extension trait method living in `willow-identity`).
- `format!("{id}")` is allowed only in non-user-visible debug
  contexts (panics, internal traces).
- The implementation PR adds an audit-checklist item to grep every
  existing `EndpointId` `Display` call-site and either replace it
  with `to_bech32()` or document why hex is appropriate there.

### `willow-ids` API

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
    /// The string was a valid Willow bech32m identifier but for a
    /// different HRP than the caller asked for.
    WrongHrp { expected: Hrp, got: Hrp },
    /// The string is a valid bech32m string with a recognised
    /// non-Willow HRP (Nostr, Bitcoin, Cosmos, …). Carries the
    /// detected HRP so the UI can render a useful message.
    LooksLikeNostr { detected_hrp: String },
    /// The HRP looks like one of ours (`w*`) but the checksum does
    /// not validate — most likely a typo or truncation.
    InvalidChecksum,
    InvalidLength,
    MalformedTlv,
    UnknownVariant,
}

// Pure-bytes API — no Willow type imports.
pub fn encode_raw(hrp: Hrp, bytes: &[u8]) -> String;
pub fn decode_raw(hrp: Hrp, s: &str) -> Result<Vec<u8>, DecodeError>;

pub fn encode_tlv(hrp: Hrp, tlvs: &[(u8, &[u8])]) -> String;
pub fn decode_tlv(hrp: Hrp, s: &str) -> Result<Vec<(u8, Vec<u8>)>, DecodeError>;

/// Inspect the HRP of an unknown string without decoding the body.
/// Returns `Ok(Hrp)` for a known Willow HRP, or a `DecodeError`
/// describing the mismatch. In particular, recognised foreign HRPs
/// (`npub`, `note`, `nprofile`, `nevent`, `naddr`) return
/// `LooksLikeNostr` so paste handlers can render "That looks like a
/// Nostr key, not a Willow id" instead of the unhelpful
/// `InvalidChecksum`.
pub fn sniff(s: &str) -> Result<Hrp, DecodeError>;
```

Per-type wrappers (in their owning crates) look like:

```rust
// in willow-identity
impl EndpointId {
    pub fn to_bech32(&self) -> String {
        willow_ids::encode_raw(Hrp::Peer, self.as_bytes())
    }
    pub fn from_bech32(s: &str) -> Result<Self, DecodeError> {
        let bytes = willow_ids::decode_raw(Hrp::Peer, s)?;
        // … into [u8; 32] → EndpointId
    }
}

// in willow-state
impl EventHash {
    pub fn to_bech32(&self) -> String { … }
    pub fn from_bech32(s: &str) -> Result<Self, DecodeError> { … }
}
```

`sniff` exists so paste handlers can render "That looks like a Nostr
key, not a Willow invite" before doing any work. Recognising foreign
HRPs explicitly (rather than collapsing them into `InvalidChecksum`)
is the lesson learned from the Cosmos cross-chain phishing class of
errors, where lookalike HRPs across ecosystems led to user confusion
and lost funds.

## Interop and migration

The decoder precedence is:

1. Starts with a known Willow HRP followed by `1` → bech32m decode.
2. Starts with a recognised foreign HRP (`npub`, `note`, `nprofile`,
   `nevent`, `naddr`) → reject with `LooksLikeNostr` for a useful
   error message.
3. Pure hex, length 64 → legacy `EndpointId` / `EventHash` (accepted
   indefinitely on input via `EndpointId::from_str`; this is the
   path most external tooling will use and it costs us nothing).
4. Base64-looking → legacy invite / join token (deprecated, see
   below).
5. Otherwise: reject.

### URL embedding

Bech32 strings appear in URL fragments only:
`https://willow.app/#go=winv1…`. There is no `willow://` URL scheme.
Custom URL schemes require a desktop story (handler registration,
permissions UX, fallback for users without a desktop client) which
Willow does not have today and does not plan to grow before 1.0.
Fragment-based deep links work in any browser without OS
integration. This closes open question 5.

### Migration policy per format

| Old format | Status on input | Hard-removal pin |
|---|---|---|
| 64-char hex `EndpointId` / `EventHash` | accepted indefinitely | never |
| base64 invite code (`Invite::pack` blob) | `#[deprecated]` on the decoder, accepted on input | removed in `v0.5.0` |
| base64 `JoinToken` URL fragment | `#[deprecated]` on the decoder, accepted on input | removed in `v0.5.0` |

`EndpointId::Display` continues to emit upstream hex (we don't own
that impl); user-visible call-sites switch to `to_bech32()`.
`EventHash::Display` switches to emit `wevent1…` immediately. Hex
stays accepted on input via `from_str` indefinitely.

Storage (SQLite, IndexedDB) keeps raw bytes — nothing on disk is
bech32 either.

## Testing

All tests live in `willow-ids/src/tests.rs` plus integration tests in
the consuming crates.

| Test | Target |
|---|---|
| Round-trip vectors | every HRP: `encode → decode → bytes equal` |
| Known-answer vectors | hard-coded `wpeer1…` / `wevent1…` strings parsed to fixed byte arrays, protects against dependency upgrades silently changing output |
| KAT cross-check | re-run the canonical bech32m fixtures from `rust-bitcoin/bech32` 0.11 against our encoder/decoder; protects against subtle constant-mismatch bugs if we ever vendor or fork the library |
| Wrong-HRP rejection | `decode_raw(Hrp::Peer, "wserver1…")` returns `WrongHrp { expected: Peer, got: Server }` without touching the body |
| Foreign-HRP detection | `sniff("npub1…")` returns `LooksLikeNostr { detected_hrp: "npub" }`; same for `note`, `nprofile`, `nevent`, `naddr` |
| Bech32 vs bech32m | a plain-bech32 string with a valid-looking body is rejected (ensures we didn't silently accept both variants) |
| TLV unknown-type ignore | `decode_tlv` with a type-99 TLV returns Ok and ignores it |
| Length bounds | >5000-char input rejected early |
| Malformed TLV — length byte overruns | a TLV where the length byte declares more bytes than remain in the payload returns `MalformedTlv`. Specifically tests a 3-byte payload `[0x00, 0xff, 0x42]` (type 0, length 255, but only 1 value byte present) — exercises the length byte itself, not just truncation |
| Legacy hex fallback | 64-char hex still decodes via `EndpointId::from_str` (no grace period, indefinite) |
| Legacy base64 deprecated | the base64 invite/join-token decoders are still callable but emit a `#[deprecated]` warning naming the `v0.5.0` removal tag |
| Property test | random bytes → encode → decode → round-trip |

Browser tests in `crates/web/tests/browser.rs` cover one end-to-end
path: paste a `winv1…` string into the join page and verify the
JoinRequest fires with the decoded link id.

## Resolved decisions

The following questions raised during design review have been
resolved in this revision:

1. **bech32m everywhere.** No mixing variants per HRP. We own the
   choice; `sniff()` explicitly recognises Nostr HRPs and returns
   `LooksLikeNostr` so paste errors get a useful message.
2. **HRP length: 5–7 chars.** `wpeer` over `wp`. Type signal in logs
   is worth the extra characters.
3. **`wserver` = genesis `EventHash`.** Definitive. The owner-pubkey
   alternative is dropped because it does not survive ownership
   transfer.
4. **No `wsecret` HRP, ever.** The `nsec` ↔ `npub` visual-similarity
   attack class is well-documented; secrets stay in keystore/files.
5. **URL embedding: fragment only.** `https://willow.app/#go=…`. No
   `willow://` URL scheme — no desktop story.
6. **Interop grace period.** 64-char hex accepted indefinitely.
   Base64 invite/join-token decoders are `#[deprecated]` and will be
   hard-removed at the `v0.5.0` git tag.

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
