# Web join flow: pkarr bootstrap resolution + progress surface

**Date:** 2026-05-29
**Status:** active
**Spec:** `specs/2026-04-24-outbox-relay-discovery.md` (Layer 1 / "Bootstrap flow for share links")
**Plan:** `plans/2026-05-28-relay-upgrade-bundle.md` — PR 6 Task 6.4
**Scope:** `crates/web/src/app.rs`, `crates/web/src/state.rs`, `crates/web/src/components/join_page.rs`, `e2e/multi-peer-sync.spec.ts`

## Problem

The web join flow (`crates/web/src/app.rs`) resolves bootstrap addressing
exactly one way: `fetch_bootstrap_id(relay_url)` against the compiled-in
`DEFAULT_RELAY_URL`. PR 6 Task 6.2 added
`JoinToken.bootstrap_endpoint_ids: Vec<EndpointId>` — 2-3 self-certifying
endpoint keys the inviter ships in the share link — but nothing consumes them.

Per the outbox-relay-discovery spec's "Bootstrap flow for share links":

1. decode the token → obtain `bootstrap_endpoint_ids`,
2. resolve each via iroh pkarr **first**,
3. fall back to the configured relay (`DEFAULT_RELAY_URL`) when pkarr yields
   nothing,
4. surface progress, because DHT resolution latency is seconds.

## Key constraint: pkarr is native-only

The wasm32 client cannot itself query the BitTorrent mainline DHT — the
`mainline` UDP crate does not build for wasm, and
`crates/network/src/iroh.rs` gates `DhtAddressLookup` behind
`#[cfg(not(target_arch = "wasm32"))]`. So on the web, "resolve via pkarr"
means: **seed the bootstrap `EndpointId`s into `Config::bootstrap_peers`**.
The wasm client reaches them through the relay's `MemoryLookup` (relay-mediated
QUIC), while the *native* peers in the mesh (relay + workers) publish/resolve
those same `EndpointId`s over the DHT. The composition still matches the spec:
the joiner addresses peers by `EndpointId`, never by a durable URL.

This is not a wasm-only hack to dodge the spec — it is the spec's Layer-1
fallback policy (pinned decision Q2, plan `2026-05-28`): "fall back to
`Config::relay_url` for relay-mediated dialing; surface an error only if both
pkarr and the configured relay fail." The relay URL stays the fallback either
way; the join token's bootstrap IDs are the *preferred* set when present.

## Decision: preference order + a pure resolver

Extract a pure-ish async helper `resolve_bootstrap_peers(token_boot_ids,
relay_fetch) -> Vec<EndpointId>`:

- the token's `bootstrap_endpoint_ids` **lead** (preferred; iroh resolves each
  via pkarr on the native side, relay-mediated on wasm),
- the configured relay's `/bootstrap-id` node is **always appended** as a
  fallback candidate (de-duplicated), so the client connects to the first
  responsive endpoint and never ends up seeded with *only* an undiallable peer.

This matches the spec's joining flow ("connect to the first responsive
endpoint", steps 2-3) and pinned decision Q2 (the relay stays the fallback;
surface an error only if both pkarr and the relay fail). Crucially, an
unreachable *first* bootstrap (a stale or tampered `bootstrap_endpoint_id`) is
simply skipped by iroh's dialing while the appended relay node still carries the
mesh — that is exactly the "first bootstrap unreachable → fall through" e2e
path. Returning token IDs *exclusively* when present would have broken that
case: a single bad bootstrap would orphan the session.

The ordering logic (prefer token IDs, always append relay, de-dup) is the
load-bearing part and is unit-tested directly (`bootstrap_*`) without a browser:
the relay fetch is injected, mirroring how `fetch_relay_info` injects its
transport in `willow-client`.

### Progress surface: reuse `join_status`, add one `"resolving"` value

`UiState::join_status` is already a string state machine
(`"" | "connecting" | "denied:<reason>"`) that `JoinPage` renders. Adding a
fourth value `"resolving"` (shown as "Finding the server…") is strictly
additive: existing readers fall through their `else` to the idle Join button,
and the JoinPage gains one branch.

**Runner-up rejected:** a dedicated `NetworkState::resolving: ReadSignal<bool>`
signal. It would duplicate the join state machine across two signals that must
stay consistent (resolving implies not-yet-connecting), inviting the classic
two-flags-out-of-sync bug. One string with one more value keeps the join
lifecycle in a single place. The cost — readers must know `"resolving"` is
non-terminal — is already true of `"connecting"`.

The connect task sets `join_status = "resolving"` *before* network construction
**iff** a join token with non-empty `bootstrap_endpoint_ids` is present, then
lets the existing post-`PeerConnected` path advance to `"connecting"` when the
user actually joins. For the no-token boot path nothing changes.

## Tests

- **Unit (wasm + native compile, native run):** `bootstrap_*` in `app.rs` —
  token IDs lead and the relay node is appended; an unreachable first bootstrap
  keeps the relay fallback; the relay node is de-duplicated; empty token + failed
  fetch yields empty.
- **e2e (write-only, CI):** `e2e/multi-peer-sync.spec.ts`
  - share-link join carrying `bootstrap_endpoint_ids`, with the first bootstrap
    unreachable → join still completes via the surviving bootstrap / relay
    fallback (heads converge),
  - owner migrates the relay/sync provider mid-session; an already-connected
    client re-resolves the new provider via pkarr and keeps syncing **without a
    page reload**.

State-machine tier is empty by design: no new `EventKind` (spec §Tests).
