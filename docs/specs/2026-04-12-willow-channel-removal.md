# Willow-Channel Removal

**Date:** 2026-04-12
**Status:** landed — `willow-channel` crate deleted; `ServerState` is the sole shared-type owner. One named follow-up remains in `crates/client/src/lib.rs::reconcile_topic_map` (see *Deferred follow-ups* at the bottom).
**Implementation plan:** [`docs/plans/2026-04-12-willow-channel-removal.md`](../plans/2026-04-12-willow-channel-removal.md)

> Remove the `willow-channel` crate entirely. Consolidate all shared
> types into `willow-state` and eliminate the dual-state representation
> in the client.

## Problem

The client currently holds two parallel representations of server state:

- `willow_state::ServerState` — the authoritative event-sourced state.
- `willow_channel::Server` — a mutable in-memory copy with its own
  `Channel`, `Role`, `Member` types and UUID-based IDs.

These are kept in sync via `reconcile_topic_map()`, which is fragile
and a source of bugs. The `willow-channel` types duplicate what
`willow-state` already provides, with different ID schemes (`Uuid`
wrappers vs plain `String`), different collection types (`HashMap` vs
`BTreeMap`), and methods (`create_channel()`, `add_member()`) that
bypass the event pipeline.

`willow-channel` is only used by `willow-client`. No other crate
depends on it.

## Target architecture

After removal:

- `willow-state` is the sole owner of shared types (`Channel`, `Role`,
  `Member`, `Permission`, `ServerState`).
- The client holds `ServerState` directly — no parallel `Server` copy.
- `ChannelKind` moves from `willow-channel` to `willow-state::types`.
- Invite helpers move to `willow-client::invite` (invites are
  client-local, not event-sourced).
- All mutations go through the event pipeline; there are no mutable
  `Server` methods like `create_channel()` or `add_member()`.
- UI derives reactive views from `ServerState` fields.

## What willow-channel provides today

| Type / Feature | Used for | Disposition |
|---|---|---|
| `ServerId(Uuid)`, `ChannelId(Uuid)`, `RoleId(Uuid)` | UUID-wrapped ID newtypes | **Delete.** State uses `String` IDs throughout; the event-sourced DAG derives server ID from the genesis hash. |
| `InviteId(Uuid)` | Invite identifiers | **Move** to `willow-client::invite`. |
| `ChannelKind` (Text/Voice) | Channel type discrimination | **Move** to `willow-state::types`. Replace `kind: String` in `EventKind::CreateChannel` and `types::Channel` with the enum. |
| `ChannelError` | Error type for channel ops | **Delete.** Errors come from `ApplyResult::Rejected` or `InsertError`. |
| `Permission` | Re-export of `willow_state::Permission` | **Delete re-export.** Consumers import from `willow-state` directly. |
| `Channel`, `Role`, `Member` structs | Data model | **Delete.** `willow-state::types` already defines these. `willow-channel::Channel` has extra fields (`topic: Option<String>`, `created_at: DateTime<Utc>`) not present in `willow-state::types::Channel` — drop them (not used in the event-sourced model). |
| `Invite` struct | Invite data | **Move** to `willow-client::invite`. |
| `Server` struct + methods | Mutable in-memory state | **Delete.** All mutation goes through the event pipeline. |
| `Server::channel_key()`, `set_channel_key()` | Channel key storage | **Move** key storage to client-local state (not part of `ServerState`). |
| Role `color` field | UI metadata | **Add** to `willow-state::types::Role` if needed. |

## Client changes

### `ServerEntry` (state_actors.rs)

Before:
```rust
pub struct ServerEntry {
    pub server: willow_channel::Server,
    pub topic_map: HashMap<String, (String, willow_channel::ChannelId)>,
    pub keys: HashMap<String, ChannelKey>,
    // ...
}
```

After:
```rust
pub struct ServerEntry {
    pub keys: HashMap<String, ChannelKey>,
    // ...
}
```

Server name, channels, roles, and members come from `ServerState`
directly. `topic_map` is derived from `ServerState.channels`.

### Mutations

Before: `entry.server.create_channel(name, kind)` (direct struct mutation).

After: `build_event(EventKind::CreateChannel { ... })` (event pipeline).

This is already how most mutations work. The exceptions are:
- `create_voice_channel` in `actions.rs` — mutates `Server` directly
- `create_channel` in `mutations.rs` — mutates `Server` directly
- Invite join path in `joining.rs` — constructs a `Server` from invite data

These need to be rewritten to emit events instead.

### Tests

Several test functions create `willow_channel::Server` directly and
will break:

- `crates/client/src/lib.rs` — `test_client()` helper, plus
  `send_message_and_read_back`, `create_channel_shows_in_list`, and
  invite-related tests all construct `Server::new()`.
- `crates/client/src/invite.rs` — four test functions
  (`secure_invite_round_trip`, `wrong_recipient_cannot_decrypt`,
  `multiple_channels_encrypted`,
  `generate_invite_via_endpoint_id_produces_valid_invite`) create
  servers with `Server::new()` and `create_channel()`.

These must be rewritten to use event-sourced `ServerState` via
`ManagedDag` or replaced with state-machine-level tests.

### Invite system

`generate_invite()` currently takes a `&willow_channel::Server`. After
removal it takes the data it actually needs: server name, channel
list, and channel keys. The `Invite` and `InviteId` types move to
`willow-client::invite`.

### Files to modify

All changes are in `crates/client/src/`:

| File | Changes |
|------|---------|
| `state_actors.rs` | Remove `server` field from `ServerEntry`, derive from `ServerState` |
| `state.rs` | Remove `server: Server` field, remove `topic_map` |
| `storage.rs` | Serialize/deserialize `ServerState` instead of `Server` |
| `mutations.rs` | Remove `Server::create_channel()` calls, use event pipeline |
| `actions.rs` | Same — replace direct mutation with events |
| `joining.rs` | Construct `ServerState` from invite data, not `Server` |
| `invite.rs` | Take channel list + keys directly, not `&Server` |
| `views.rs` | Read from `ServerState` instead of `Server` |
| `persistence_actor.rs` | Persist `ServerState`, not `Server` |
| `servers.rs` | Use event pipeline for server creation |
| `util.rs` | `make_topic()` takes `&str` server ID, not `&Server` |
| `lib.rs` | Remove `parse_permission()`. `reconcile_topic_map()` is retained as a documented helper for invite-validation follow-up — see *Deferred follow-ups* below. |

### Workspace changes

- Delete `crates/channel/` entirely. The root `Cargo.toml` uses
  `members = ["crates/*"]`, so deleting the directory is sufficient.
- Remove `willow-channel` from `crates/client/Cargo.toml`.
- Add `willow-state` to `crates/client/Cargo.toml` (if not present).
- Update `CLAUDE.md` dependency graph to remove `willow-channel`.

## Deferred follow-ups

### `reconcile_topic_map()` cleanup

`crates/client/src/lib.rs::reconcile_topic_map<V>()` was *not* deleted as
originally specified. It now converts `HashMap<String, V>` →
`HashMap<willow_messaging::ChannelId, V>` and gracefully skips entries with
unparseable UUIDs (cf. issue #141 — previously, unparseable IDs were
silently replaced by random UUIDs, causing the client to diverge from the
network).

The function is currently dead in production (only its own regression test
exercises it). It is retained as a pre-wired utility for callers that
deserialize topic maps from wire data (`accept_invite`, the join path).
Integration into the full invite/join flow is tracked as follow-up work —
once those sites consume the helper, this section can be removed and the
table row above can drop to "Removed."

If the follow-up never lands, `reconcile_topic_map` and its test should be
deleted along with the vestigial `willow_messaging::ChannelId` import — it
adds nothing to the current code path.
