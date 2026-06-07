# Dependency-bump triage — bincode 3, rand 0.10, hkdf 0.13

**Date:** 2026-06-03
**Status:** landed
**PRs:** #559 (bincode), #558 (rand), #555 (hkdf), #404 (feedback feature — flagged, not a bump)

## Summary

Three open Dependabot major-version bumps were evaluated for merge against
current `main`. **All three are correctly held, not merged** — each is blocked
by a hard constraint, not a fixable lint:

| PR | Bump | Verdict | Blocker |
|----|------|---------|---------|
| #559 | bincode 1.3 → 3.0 | **Hold (pin 1.x)** | Canonical signed+hashed event encoding — format break invalidates the DAG |
| #555 | hkdf 0.12 → 0.13 | **Hold** | Needs `sha2` 0.11, which only exists as `0.11.0-rc.5` (pre-release) |
| #558 | rand 0.8 → 0.10 | **Hold** | `willow-crypto` is pinned to `rand_core` 0.6 by `x25519-dalek` 2 + `chacha20poly1305` 0.10 |

A fourth red PR, **#404**, is *not* a dependency bump — it is a full feature
crate (`crates/feedback/`, ~20 files). It is triaged separately at the bottom.

All three Dependabot branches were 200–250 commits behind `main`, so their CI
red was partly stale. Each bump was re-applied to current `main` and built
locally to get an accurate verdict; the blockers below reproduce on today's tree.

## #559 — bincode 1.3 → 3.0: format-stability boundary

bincode's byte encoding is **the canonical representation of a signed event**,
not an interchangeable serializer:

- `crates/state/src/event.rs:600-602` — `bincode::serialize(&signable)` produces
  the bytes that are SHA-256'd into the `EventHash` *and* Ed25519-signed. The
  `EventHash` **is** the event's identity (`event.rs:547`).
- `crates/state/src/event.rs:520-535` — `EventKind::discriminant()` depends on
  bincode 1.x encoding an enum variant index as a fixed-width little-endian
  `u32`. A sync test (`discriminant_matches_bincode_variant_index_low_byte`)
  pins this.
- `crates/transport/src/lib.rs:159,182` — bincode is the wire framing.
- bincode is also the on-disk encoding for persisted state (storage crate).

bincode 2/3 changes the default encoding (fixed-int → varint, endianness) **and**
removes the free `serialize`/`deserialize` functions (now `bincode::serde::*`
with an explicit config). The encoding change alone re-hashes every event:
existing `EventHash`es no longer match, old signatures no longer verify, the
per-author DAG chain (`prev`/`deps` by hash) breaks, persisted state is
unreadable, and peers on different bincode majors cannot interoperate.

Moving to bincode 2/3 is therefore a deliberate **protocol-version migration**
(canonical-encoding decision + format-version field + migration path), not a
Dependabot auto-bump. **Decision: pin bincode 1.x.** Revisit only behind a
spec'd protocol-version change.

## #555 — hkdf 0.12 → 0.13: blocked on pre-release RustCrypto

`willow-crypto` sits on one coherent RustCrypto generation: `sha2` 0.10 +
`hkdf` 0.12 + `hmac` 0.12 (all `digest` 0.10), alongside `chacha20poly1305` 0.10
and `x25519-dalek` 2.

hkdf 0.13 pulls `hmac` 0.13 / `digest` 0.11, so `Hkdf::<Sha256>` then needs a
`digest` 0.11 hash. The CI errors (`CoreProxy` / `HashMarker` / `FixedOutput`
not satisfied for `Sha256VarCore`) are exactly that mismatch. The fix would be
`sha2` 0.11 — but the registry only offers `0.11.0-rc.5`; stable is `0.10.9`.
Pulling a pre-release hash crate into a security-critical path is not acceptable.

**Decision: hold until the RustCrypto `digest` 0.11 generation ships stable**
(`sha2`/`hmac`/`hkdf` move together).

## #558 — rand 0.8 → 0.10: blocked on rand_core generation split

The tree already carries `rand` 0.8.5, 0.9.4, and 0.10.1 — `willow-identity`
and `willow-worker` are on 0.9 cleanly. The bump's problem is `willow-crypto`
(and the bundled `willow-agent`), which use `rand` 0.8.

`willow-crypto`'s `OsRng` must satisfy `rand_core` 0.6 to feed `x25519-dalek` 2
(`StaticSecret`/`EphemeralSecret`) and `chacha20poly1305` 0.10. Bumping crypto
to rand 0.10 moves it to `rand_core` 0.10; rustc then can't find a `RngCore`
impl those crates accept (the compiler even suggests
`chacha20poly1305::aead::rand_core::RngCore`). crypto cannot leave `rand_core`
0.6 until `x25519-dalek` and `chacha20poly1305` publish stable next-gen
releases on `rand_core` 0.9+.

**Decision: hold.** (A narrower bump of only `willow-agent`'s non-crypto RNG to
0.9 is possible but is a different change than this PR, which bundles crypto.)

## Tradeoffs / alternatives considered

- **Force the bumps with pinned pre-release crates** (`sha2 0.11.0-rc.5`,
  `rand_core` shims). Rejected: pre-release crypto in a P2P security app; and a
  rand_core shim cannot bridge `x25519-dalek` 2's trait bounds.
- **bincode 2/3 with `bincode::serde` + `legacy()` config** to preserve the 1.x
  encoding. Plausible *future* path, but still an API migration touching every
  call site and a risk to the canonical hash; out of scope for an auto-bump.
- **Leave the PRs open, red.** Rejected: they sit red indefinitely and re-churn
  on every Dependabot run. Closing with rationale is cleaner — Dependabot
  re-opens automatically when a compatible version appears.

## #404 — in-app feedback (NOT a dependency bump)

#404 adds a full `crates/feedback/` worker crate plus worker-actor wiring and a
spec/plan. Its red check is a `mio`-in-wasm build error (`crate::sys::*`
unresolved on `wasm32`), most likely stale-base noise — but it is a substantial
unrelated feature from a separate session, not a bump. It needs its own review
and a current-base CI run before any merge decision. Triaged out of this pass.

## Outcome

Merged this pass (green, low-risk): #664 (relay upgrade bundle), #557 (iroh
0.98.2), #556 (rmcp 1.6), #666 (taiki-e/install-action 2.80).

Held with rationale (this report + per-PR comment): #559, #558, #555.
Deferred for separate review: #404.
