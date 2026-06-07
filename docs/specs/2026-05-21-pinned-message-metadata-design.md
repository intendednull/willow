# Pinned-message metadata — `pinned by {name} · {when}` footer

**Date:** 2026-05-21
**Status:** landed (PR #644 — Channel::pinned_messages → BTreeMap<EventHash, PinMetadata>; client PinnedMetadata; pinned-by footer renders)
**Parent specs:**
[`reactions-pins.md`](2026-04-19-ui-design/reactions-pins.md),
[`2026-04-12-state-authority-and-mutations.md`](2026-04-12-state-authority-and-mutations.md)
**Implementation plan:** PR-C of phase-3c close-out
(`docs/plans/2026-05-08-ui-phase-3c-reactions-pins.md` AG-7 / T8)

## Problem

`docs/specs/2026-04-19-ui-design/reactions-pins.md` §Pinned panel contents
(line 123) specifies an optional `pinned by {name} · {when}` footer on each
pinned-panel entry — `--ink-3`, 10 px, mono `when`. The current
implementation does not render this footer, and the acceptance criterion at
line 230 has been ticked anyway. The root cause is upstream: the materialized
state stores `Channel::pinned_messages: BTreeSet<EventHash>` and so loses the
pinner identity + pin time from the `PinMessage` event. The renderer has
nothing to read.

## Decision

Replace the pinned-messages set with a map carrying capture metadata:

```rust
pub struct PinMetadata {
    /// Peer id of the user who emitted the PinMessage event.
    pub pinner: EndpointId,
    /// `event.timestamp_hint_ms` from the PinMessage event (ms wall clock).
    pub pinned_at_ms: u64,
}

// On Channel:
pub pinned_messages: BTreeMap<EventHash, PinMetadata>,
```

`willow-state::materialize` captures `event.author` and
`event.timestamp_hint_ms` when applying `EventKind::PinMessage`. The client
projection surfaces a derived `PinnedMetadata { pinner_display_name,
pinned_at_ms }` on `DisplayMessage` for the pinned-panel projection only
(message-list projections leave it `None`). The web renderer reads the
metadata to render the footer.

## Rejected alternatives

### Derive from the DAG on every render

Walk the DAG to locate the `PinMessage` event for each pinned hash and read
`author` + `timestamp_hint_ms` there. Rejected because:

- O(messages × pins) per pinned-panel render — the panel re-renders on every
  channel switch.
- Forces the client to retain old `PinMessage` events forever, even after
  `UnpinMessage` — those events have no other consumer once the materialized
  state has applied them.

The metadata captured eagerly at apply time is O(pins) extra memory per
channel and zero extra IO per render — strictly cheaper.

### Parallel `pin_metadata` index alongside the existing set

Keep `pinned_messages: BTreeSet<EventHash>` and add
`pin_metadata: BTreeMap<EventHash, PinMetadata>` as a sibling field on
`Channel`. Rejected because it doubles the invariants the apply branch must
maintain — every `PinMessage` would have to update both, and `UnpinMessage`
would have to remove from both. Replacing the set with a map keeps a single
source of truth: presence in the map *is* the "pinned" bit.

## Migration

None required. `ServerState` is materialized from the DAG by every consumer
(web/client, replay worker, storage worker, agent); there is no persisted
state-level struct. `Channel` derives `Serialize`/`Deserialize` but is not
written to disk or to the wire in any production path — the only call site
is a round-trip test in `crates/state/src/tests/materialize.rs`.

## Wire format

Unchanged. No new `EventKind`; the `PinMessage` payload remains
`{ channel_id, message_id }`. Only the materialized projection schema
changes, and the materialized schema is not part of the wire protocol.

## Footer omission contract

The footer renders whenever `DisplayMessage.pinned_metadata` is `Some`. The
projection populates `pinner_display_name` via `resolve_display_name`, which
already has a documented fallback ladder (`profile.display_name` →
`ProfileState.names` → `unknown peer`). The footer therefore renders even
when the pinner is a stranger or absent; the `pinned by unknown peer · …`
form is acceptable.

If the projection ever needs to *suppress* the footer (e.g. for very recent
pins where profile data has not yet synced), it can leave `pinned_metadata`
as `None`. v1 always emits it.

## Open question (resolved)

**Should the footer surface for the local user's own pins?** Yes. The spec
says "optional `pinned by {name} · {when}` footer" — interpret as "shown
when meaningful". Self-attribution is meaningful for audit trails ("yes, I
pinned this last spring") and keeps the rendering branch single-path. The
footer renders for all pinners including self.
