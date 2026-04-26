# Direct Messages — design notes and deferral to MLS-over-Willow

> **One-sentence summary:** This spec captures lessons learned from a
> Nostr-NIP-17/44/59-inspired investigation of Willow DMs. The
> conclusion is that we should **NOT** ship the seal+gift-wrap design
> directly. Instead we plan to specify **MLS-over-Willow (RFC 9420)**
> in a follow-up, retaining NIP-44's metadata-hiding patterns as a
> transport encoding *on top of* MLS application messages.

## Status

**Deferred.** No `EventKind` variants are added by this spec. No code
lands as a result of this spec. The wire-format work below is preserved
in Appendix A as research notes for the future MLS-over-Willow spec.

## Why we are deferring

After a Round 2 review of the seal+gift-wrap design against the wider
secure-messaging landscape (Signal, Matrix, XMTP, Bluesky, Nostr's own
NIP-EE / Marmot), the clean-architecture answer is to defer first-class
DM implementation in favor of MLS-over-Willow.

- **NIP-59 is a privacy envelope without a forward-secrecy layer
  underneath.** Signal's Sealed Sender works because it sits on top of
  the Double Ratchet. Nostr's NIP-17 explicitly lacks FS / PCS — and
  the Nostr ecosystem itself has moved on to NIP-EE / Marmot, both
  MLS-based. Shipping seal+gift-wrap on Willow would be repeating
  Nostr's known-bad starting point.

- **Matrix Megolm is the lived warning of group-chat-over-gossip
  without MLS.** Roughly seven years of UTD ("Unable to Decrypt")
  production bugs — sender goes offline mid-key-share, partial
  delivery, session corruption, device-list races — all live exactly
  in the seam this spec was creating between "DM rumor", "per-author
  seal DAG", and "ephemeral wrap on inbox topic". MLS removes the
  seam.

- **MLS solves these problems.** RFC 9420's TreeKEM gives O(log N)
  group rotation (vs O(N) gift wraps in this spec). Welcome messages
  atomically bind admit-and-key-distribution. RFC 9750 specifies the
  architecture for deployment. XMTP, Bluesky, and Cisco Webex have
  all shipped against RFC 9420; the convergence is industry-wide.

- **DAG concurrency.** Willow's event DAG tolerates concurrent
  membership events; MLS assumes serialized Commits. Channels must
  linearize to fit MLS, but channel-level linearization is a strictly
  smaller scope than full server-state linearization, and is a
  tractable design problem for the follow-up spec.

The seal+gift-wrap design captured here would solve a metadata-hiding
problem while leaving forward secrecy, post-compromise security,
multi-device, and group-DM scaling unsolved — and would have to be
ripped out the moment we adopted MLS for groups. That is not a
sequence we want to commit to.

## Crypto lessons captured for the MLS spec

The investigation surfaced specific findings that the future
MLS-over-Willow spec must absorb.

### Deniability claim was structurally false

The original seal layer used the real author's Ed25519 signature over
the encrypted rumor. Once a recipient (or a future device-compromise)
recovers the rumor plaintext, that signature **non-repudiably binds the
author to the rumor**. Calling this "deniable" because the seal was
encrypted to one recipient was sleight-of-hand: the cryptographic
binding survives plaintext recovery.

The future spec must be honest about this. Either:

- Drop the deniability claim entirely; or
- Use a designated-verifier MAC (e.g. an HMAC keyed from the X25519
  shared secret) instead of a signature, so the recipient cannot
  prove authorship to a third party.

### Per-recipient inbox topic leaks the active-DM-recipient graph

The inbox topic `_willow_inbox/<blake3(recipient_pk)>` lets workers
and any subscribed observer enumerate which pubkeys are *currently
receiving* DM traffic, by watching subscription patterns. The pubkey
itself is public, but the **subscription graph** ("which pubkeys are
DM-active right now") is new metadata.

The future spec must address this — bloom-filter or k-anonymity
buckets (multiple recipients share a bucket id), worker-mediated
fetch (recipients pull from an aggregator), or explicit acceptance
of the leak. It must not be silently inherited.

### Per-author DAG pollution from one-shot ephemeral chains

Each gift wrap in the original design spawned a single-event
ephemeral-author DAG. Workers retain these forever (no natural
retention signal) and the per-author DAG count grows linearly with
DM volume across the network. This is an existing data-model
problem the spec made worse, not better.

**MLS application messages should NOT enter the per-author DAG.**
They belong on a separate transport path — e.g. an inbox topic with
worker-bounded retention, or a fetch-on-demand store — explicitly
outside the event-sourced state machine.

### NIP-44 v2 payload format is reusable, but must be used verbatim

The NIP-44 v2 AEAD construction — ChaCha20 + HMAC-SHA256, 76-byte
HKDF-Expand split into 32 / 12 / 32 (chacha key / iv / hmac key),
length-prefixed power-of-two padding — IS a reasonable AEAD primitive
for MLS application-message ciphertexts at the framing layer.

It must be used **verbatim**: no `"willow-dm-v1"` HKDF salt fork, no
version-byte renumbering. Preserving identical KAT vectors with
upstream NIP-44 keeps cross-implementation interop and lets us reuse
the existing test corpus. A custom salt buys nothing and breaks every
external test vector.

### Multi-device must be designed in from day one

Willow currently uses one Ed25519 key for signing, endpoint ID, and
(via conversion) DH. The future MLS spec must split:

- A long-term **identity key** that names the user across devices.
- Per-device **session keys** that participate in MLS group state.

This is the Sesame-class design. Adding multi-device after the fact
(as Signal and Matrix both learned) is dramatically harder than
designing it in. It cannot be a v2 feature.

## Non-goals (for the future MLS spec)

The future MLS-over-Willow spec MUST satisfy:

- **MUST provide forward secrecy.** A device compromise today does not
  reveal yesterday's plaintext.
- **MUST provide post-compromise security.** Recovery from a device
  compromise via key rotation, without re-establishing the group out
  of band.
- **MUST handle multi-device.** Identity key separated from session
  key; new devices join via a user-scoped enrollment flow.
- **MUST avoid DAG pollution.** MLS application messages live on a
  transport path that is not the event-sourced per-author DAG.
- **MUST hide metadata at least as well as NIP-59.** Sender, content,
  and recipient set are not visible to passive observers or workers
  beyond a coarse routing hint.

These are non-negotiable preconditions for the follow-up spec — not
items to be deferred again.

## Open questions

1. **When do we start the MLS-over-Willow spec?** A draft should
   begin once the channel-linearization design (a prerequisite for
   serialized Commits) is sketched.

2. **Who owns it?** The follow-up spec spans `willow-state` (channel
   linearization), `willow-crypto` (MLS ciphersuite glue),
   `willow-network` (transport path that bypasses the per-author
   DAG), and `willow-client` (multi-device enrollment).

3. **Library choice.** `openmls` (Rust, RFC 9420 conformant, used by
   several production deployments) is the leading candidate. Open
   questions: WASM compatibility, ciphersuite selection, storage
   trait fit with our `EventDag` / `ManagedDag` abstractions (the
   legacy `EventStore` trait has been removed).

4. **Ciphersuite.** RFC 9420 mandates X25519 + Ed25519 + ChaCha20-
   Poly1305 + SHA-256 as one valid option, which aligns with
   Willow's existing primitives.

5. **Channel linearization scope.** What exactly must serialize for
   Commits? Only membership-changing events, or all channel events?

6. **Inbox-topic privacy.** Bloom buckets vs worker-mediated fetch
   vs accepted leak — to be decided in the MLS spec, not here.

## Sources

- RFC 9420 — *The Messaging Layer Security (MLS) Protocol*.
- RFC 9750 — *The Messaging Layer Security (MLS) Architecture*.
- Signal blog — *Sealed Sender for Signal* (technical preview).
- Matrix.org — *MatrixConf 2024: Unable To Decrypt — A Postmortem*.
- Marmot Protocol — MLS-over-Nostr specification (NIP-EE precursor).
- XMTP — *Why XMTP chose MLS* (engineering rationale).
- Bluesky — MLS direct-messaging design notes.

---

## Appendix A: Investigated wire format (not adopted)

> **DEPRECATED — DO NOT IMPLEMENT.** The material in this appendix
> describes a Nostr-NIP-17/44/59-inspired design that was investigated
> and **rejected** in favor of MLS-over-Willow (see body of spec).
> It is preserved only as research notes for the future MLS spec
> author. **No `EventKind` variants are added. No code lands.**

### A.1 Layer structure (investigated, rejected)

Three layers, mirroring NIP-59:

```
   DMRumor  ── no signature, carries the real author & content
      │
      ▼ NIP-44-style encrypt to recipient pubkey, sign with real author
   [Seal payload]                          (would have been on real author's DAG)
      │
      ▼ NIP-44-style encrypt to recipient pubkey, sign with EPHEMERAL key
   [Gift-wrap payload]                     (would have been on ephemeral author DAG)
```

The rumor carried `author_endpoint_id` (Willow uses `EndpointId`, not
`PeerId`), a jittered timestamp hint, and a `willow_messaging::Content`.
The seal wrapped the rumor under an X25519 shared secret to the
recipient. The gift wrap wrapped the seal under a fresh single-use
ephemeral key.

This appendix omits the originally-proposed `EventKind::DMSeal` and
`EventKind::DMGiftWrap` variants from any implementation framing.
**This spec does NOT add new `EventKind` variants. Implementation is
deferred to the MLS-over-Willow follow-up.**

### A.2 Payload format (NIP-44 v2, investigated)

Mirrors NIP-44 v2:

| Field | Size | Notes |
|-------|------|-------|
| `version` | 1 B | `0x02` (do NOT fork to `0x01` — keep KAT compatibility) |
| `nonce` | 32 B | CSPRNG, also used as HKDF-expand `info` |
| `ciphertext` | variable | ChaCha20 (counter=0) output, input is padded plaintext |
| `mac` | 32 B | HMAC-SHA256(hmac_key, nonce ‖ ciphertext) |

Key derivation (verbatim NIP-44 v2; **do not** introduce a Willow-
specific salt):

```
shared     = X25519(ed25519_to_x25519(sender_sk), ed25519_to_x25519(recipient_pk))
conv_key   = HKDF-Extract(salt = "nip44-v2",         ikm = shared)
expanded   = HKDF-Expand(prk  = conv_key, info = nonce, L = 76)
chacha_key = expanded[0..32]
chacha_iv  = expanded[32..44]            // 12 bytes
hmac_key   = expanded[44..76]
```

> Note: `ed25519_to_x25519` above is generic NIP-44 pseudocode. In
> the current Willow codebase the corresponding helpers are
> `willow_crypto::identity_to_x25519` (for an `Identity`'s secret
> key) and `willow_crypto::ed25519_public_to_x25519` (for a public
> key); a future MLS spec should call these by their real names.

Padded plaintext layout:

```
[u16 BE length][plaintext][zero padding]
```

Power-of-two bucket sizes (min 32 B):

```
if len ≤ 32: bucket = 32
else:        next  = 2^(floor(log2(len-1)) + 1)
             chunk = max(32, next / 8)
             bucket = chunk * (ceil(len / chunk))
```

Encrypt-then-HMAC (not Poly1305) preserves NIP-44 KAT portability.
The future MLS spec should reuse this construction at the framing
layer **without** modification.

### A.3 Delivery topic (investigated)

Two candidates were considered:

| Option | Pro | Con |
|--------|-----|-----|
| Per-recipient `_willow_inbox/<blake3(recipient_pk)>` | Small fan-out | Leaks DM-recipient activity graph via subscriptions |
| Shared `_willow_inbox` | No per-pubkey topic-id leak | Every wrap floods every peer; DoS |

Neither is acceptable as-is. The MLS spec must address the
subscription-graph leak explicitly (see "Crypto lessons" above).

### A.4 Multi-recipient (investigated)

Group DMs would have produced **N independent gift wraps**, one per
recipient (including the sender's own other devices). This is O(N) per
message — exactly the cost MLS's TreeKEM amortizes to O(log N), and a
direct motivator for moving to MLS for any group of more than ~8.

### A.5 Timestamp jitter (investigated)

Each layer independently jittered `timestamp_hint_ms` up to 2 days
into the past, breaking the obvious `wrap.ts == seal.ts` linkage.
HLC was deliberately not used (it would leak real sender clocks).
The MLS spec inherits the same constraint.

### A.6 Threat model (investigated, summary only)

The original threat-model table is omitted from this revision — it
applies to a design we are not shipping. The MLS spec will produce
its own threat model. Key carry-overs: passive observers must learn
no more than "someone sent a DM, of roughly this size, at roughly
this time", and workers must not be able to link wraps to real
sender identities.
