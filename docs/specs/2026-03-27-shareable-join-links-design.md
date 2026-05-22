# Shareable Join Links

**Date:** 2026-03-27
**Status:** landed — wire protocol, key-exchange handshake, link generation/deletion, JoinPage, routing, persistence, denial emission for invalid links, and max-uses/expiration UI controls all shipped. See [`docs/plans/2026-03-27-shareable-join-links.md`](../plans/2026-03-27-shareable-join-links.md) for the original migration. Persistence + JoinDenied gaps closed in PR #660 (2026-05-22); max-uses/expiration UI shipped in PR #661 (2026-05-22). One spec divergence preserved intentionally: silent-drop for *unknown* link_ids is kept as an anti-enumeration property — see *Realised state* below.
**Implementation plan:** [`docs/plans/2026-03-27-shareable-join-links.md`](../plans/2026-03-27-shareable-join-links.md)

> **Realised state (post-2026-05 audit + follow-up PRs).** The original
> spec landed via the linked plan in late March 2026. A 2026-05 audit
> surfaced nine misalignments, broken into three categories:
>
> **Substantive gaps — fixed in follow-up PRs:**
>
> - **JoinLinks now persist** across page reloads (PR #660). Previously
>   `create_join_link` / `delete_join_link` only mutated the in-memory
>   `Arc<Mutex<Vec<JoinLink>>>` and the listener's `used += 1` bump
>   never reached disk, so creating a link → refreshing wiped it and
>   exhausted links looked fresh after restart. `ClientHandle::new` now
>   hydrates `join_links` from `storage::load_join_links(server_id)`
>   for every loaded server, and mutation paths send `PersistJoinLinks`
>   to the persistence actor.
> - **Invalid-link denial now emits `JoinDenied { reason }`** (PR #660).
>   The listener previously dropped requests silently for disabled /
>   expired / exhausted links — joiner saw a 30-second timeout instead
>   of the spec-promised explicit denial. Now distinguishes
>   `link_disabled` (active = false) and `link_expired` (used >=
>   max_uses OR expires_at < now) reasons.
> - **Max-uses + expiration UI** shipped via a `<details>` disclosure
>   in `settings.rs` (PR #661). Defaults to `max_uses=5` /
>   `expires=Never`; selector offers Never / 1 hour / 24 hours / 7
>   days. The `create_join_link(max_uses, expires_at)` signature was
>   already correct; only the call site was hardcoded.
>
> **Intentional deviation from spec (anti-enumeration):**
>
> - **Unknown `link_id`s drop silently.** The spec's §Error Handling
>   table lists `link_not_found` as a valid `JoinDenied` reason. The
>   realised listener emits a denial for known-but-invalid links
>   (`link_disabled` / `link_expired`) but drops requests for `link_id`s
>   that aren't in the inviter's table at all. Rationale: replying for
>   unknown link_ids would let attackers enumerate which link_ids exist
>   ("I just need to find one that gets a Denied instead of a
>   timeout"). Silent drop preserves the property that the inviter is
>   indistinguishable from "wrong inviter" for unknown link_ids — only
>   link_ids you *plausibly created* receive a denial.
>
> **Doc drift — spec text below is stale on these points:**
>
> - **Wire field types.** `JoinRequest.peer_id`, `JoinResponse.target_peer`,
>   `JoinDenied.target_peer` are `EndpointId` in code, not `String`
>   (the spec sketches use `String`). `JoinToken.inviter_peer_id` is
>   likewise `EndpointId`. The wire protocol carries strongly-typed
>   identities; the spec's String sketches are pseudocode.
> - **`link_id` binding on responses.** `JoinResponse` and `JoinDenied`
>   each carry an additional `link_id: String` field added for issue
>   #309 / SEC-A-07 (signer-binding gating). The joiner records the
>   expected inviter in `pending_joins` *before* broadcasting the
>   `JoinRequest`, then drops responses whose `link_id` doesn't match
>   the outstanding attempt or whose signer doesn't match the recorded
>   inviter. Meaningful security mechanism not described in the spec's
>   wire protocol section.
> - **`JoinLink` extra fields.** The struct also carries `server_id:
>   String` (required for per-server persistence) and `created_at: u64`
>   (drives the UI's relative-age display).
> - **Test coverage.** Spec §Testing calls for browser tests of the
>   join-flow UI and link-management surfaces; realised coverage is
>   at the client tier (state-actor + listener tests) plus a
>   markup-contract browser test for the new options disclosure (PR
>   #661). The end-to-end join flow itself is covered by
>   `e2e/join-links.spec.ts`.
>
> The body below is preserved as the original target. The *Realised
> state* list above is authoritative for current implementation shape;
> do not edit the body in place to match it.

## Goal

Replace the multi-step invite flow (share PeerId, generate per-recipient invite, paste blob) with a single shareable URL that triggers automatic P2P key exchange when clicked.

## Constraints

- Inviter must be online when the joiner clicks the link.
- Existing per-recipient crypto (X25519 DH + ChaCha20-Poly1305) is preserved — the link itself contains no secrets.
- Works in a browser without app installation.
- Bevy desktop app is out of scope — web-only for this iteration.

## Security Model

The link is a **pointer, not a credential**. It contains no secrets — only enough information to find the inviter on the P2P network. All sensitive data (channel keys) flows over the existing signed gossipsub channel during a live exchange.

**Threat model for gossipsub broadcast:**

All `JoinRequest`, `JoinResponse`, and `JoinDenied` messages are broadcast on `_willow_server_ops` via gossipsub. This means all subscribed peers see the metadata (link_id, joiner PeerId, encrypted invite blob). This is an accepted tradeoff — the same pattern is used for `VoiceSignal` messages. The encrypted invite blob is useless to non-target peers (per-recipient X25519 encryption).

**Link exhaustion:** Anyone with the link URL can send `JoinRequest` messages to burn uses. This is the same tradeoff as Discord invite links — shareable links are inherently open to whoever has them. The owner mitigates by setting conservative `max_uses`, deleting compromised links, or disabling the feature entirely. Rate-limiting is not needed for a friend-group deployment.

## Architecture

Three phases:

### 1. Link Generation (inviter)

Any peer with `CreateInvite` permission (owner has this implicitly) can generate join links. They set a max join count and optional expiration. The app generates a token containing:

```
JoinToken {
    inviter_peer_id: String,
    server_id: String,
    link_id: String,       // UUID unique to this link
    server_name: String,   // displayed on the join page
    inviter_name: String,  // "Invited by [name]"
}
```

Serialized via `willow_transport::pack()`, base64-encoded, and placed in a URL fragment:

```
https://willow.intendednull.com/#join=<base64-token>
```

The inviter's client stores link metadata locally:

```
JoinLink {
    link_id: String,
    max_uses: u32,
    used: u32,
    active: bool,
    expires_at: Option<u64>,  // timestamp in ms, None = never expires
}
```

Persisted to storage (survives page reload). Multiple links can be generated with different limits.

### 2. P2P Key Exchange (automatic)

When the joiner clicks the link:

1. Web app loads, reads `#join=<token>` from the URL fragment.
2. App connects to the relay and subscribes to `_willow_server_ops` (happens automatically via `on_connected()`).
3. App sends a `JoinRequest` via gossipsub on `_willow_server_ops`.
4. Inviter's app receives the request, validates the `link_id` (active, under limit, not expired), and auto-generates a per-recipient encrypted invite using the joiner's PeerId.
5. Inviter increments `used` counter, persists the updated link state.
6. Inviter sends a `JoinResponse` back via gossipsub, targeted at the joiner.
7. Joiner's app receives the response, calls `accept_invite()` programmatically with the `invite_data` field, and transitions into the server.
8. Joiner clears the URL fragment via `history.replaceState` to prevent re-triggering on refresh.

If the joiner refreshes the page mid-handshake, the `#join=` fragment is still in the URL and the flow restarts cleanly (re-sends `JoinRequest`).

### 3. Offline Handling

If the inviter is not reachable, the joiner sees "Connecting to server owner..." with auto-retry (exponential backoff, 2s to 30s cap). A cancel button returns to the welcome screen. After 60s of no response, the message updates to "Server owner appears to be offline. You can keep waiting or try again later."

## Wire Protocol

Three new `WireMessage` variants:

```rust
JoinRequest {
    link_id: String,
    peer_id: String,       // joiner's PeerId for encryption
}

JoinResponse {
    target_peer: String,   // only this peer processes it
    invite_data: String,   // base64 invite (same format as current invite codes)
}

JoinDenied {
    target_peer: String,
    reason: String,        // "link_disabled", "link_expired", "link_not_found"
}
```

All sent on the `_willow_server_ops` gossipsub topic, wrapped in signed envelopes (existing `pack_wire` / `unpack_wire`).

**Relay history exclusion:** The relay only stores `WireMessage::Event` variants. `JoinRequest`, `JoinResponse`, and `JoinDenied` are ephemeral handshake messages and are not persisted by the relay (same as `TypingIndicator`, `VoiceJoin`, etc.).

## UX Flow

### Inviter (server settings)

- New "Invite Links" section on the Server tab, below existing "Invite a Peer" section. Supplements (does not replace) the existing per-recipient invite flow.
- "Max joins" number input (default 5).
- Optional "Expires after" duration selector (1 hour, 24 hours, 7 days, never).
- "Generate Link" button.
- Generated links shown in a list: truncated URL, uses remaining (e.g. "3/5 uses"), copy button, delete button.
- Multiple links can be active simultaneously.

### Joiner

- Clicks link. App loads a **dedicated JoinPage** (not the welcome screen, not a modal).
- JoinPage shows: server name (from token), "Invited by [name]", a name input (pre-filled from saved profile), and a "Join [Server]" button.
- Clicking Join: button morphs to "Connecting..." with a pulse animation. Input disabled. No spinner overlay — the button IS the loading state.
- After 15s with no response: hint text appears below: "[Inviter] isn't online right now. We'll keep trying." Retries silently with exponential backoff (2s → 30s).
- On success: JoinPage crossfades out (opacity + scale), chat view fades in. URL fragment cleared. ~300ms transition.
- On denied/expired: error text replaces button. "This invite link is no longer valid." Back button returns to welcome.
- Works identically for new users and existing-server users — the JoinPage handles both.

## Error Handling

| Scenario | Behavior |
|----------|----------|
| Inviter disabled the setting | `JoinDenied { reason: "link_disabled" }` — "This invite link is no longer active." |
| Max uses reached | `JoinDenied { reason: "link_expired" }` — "This invite link has been fully used." |
| Link time-expired | Inviter checks `expires_at` before responding. `JoinDenied { reason: "link_expired" }`. |
| Inviter offline mid-exchange | Auto-retry with backoff. After 60s: "Server owner appears to be offline." |
| Simultaneous joiners | Processed sequentially. Counter decremented per join. Overflow gets `link_expired`. |
| Joiner already in server | Inviter sends invite anyway (re-sends keys). `accept_invite` merges harmlessly. |
| Corrupted/invalid token | Detected immediately client-side. "Invalid invite link." No P2P attempt. |
| Page refresh mid-handshake | Fragment persists, flow restarts cleanly. |

## Testing

**Client unit tests** (`crates/client/src/lib.rs` test module):
- `JoinToken` serialization round-trip via `willow_transport::pack` / `unpack`.
- `JoinLink` counter logic: increment, max reached, deactivation, time expiration.
- New `WireMessage` variants (`JoinRequest`, `JoinResponse`, `JoinDenied`) round-trip via `pack_wire` / `unpack_wire`.
- Permission check: only peers with `CreateInvite` can generate links.

**Browser tests** (`crates/web/tests/browser.rs`):
- Join flow UI: spinner state, error messages, fragment detection and cleanup.
- Link management UI in settings: generate, copy, delete.

**E2E Playwright** (`e2e/join-links.spec.ts`):
- Desktop creates link, second peer navigates to link URL, verifies auto-join completes and messages sync.
- Link with max_uses=1: second joiner gets "link expired" after first use.
- Inviter offline: joiner sees waiting state with retry.
