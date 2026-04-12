# Willow-Channel Removal — Implementation Plan

**Date**: 2026-04-12
**Spec**: `docs/specs/2026-04-12-willow-channel-removal.md`
**Depends on**: `docs/plans/2026-04-12-state-authority-and-mutations.md`
(permission pre-check must be in place before mutations are rewritten)

## Overview

Remove `willow-channel` and make `willow-state::ServerState` the
client's sole source of truth. Work is ordered to keep compilation
passing after each step.

## Step ordering

```
Step 1 (ChannelKind in state)
  ↓
Step 2 (Move invite types)    Step 3 (Rewrite mutations)
  ↓                              ↓
Step 4 (Remove Server from client)
  ↓
Step 5 (Rewrite join path)
  ↓
Step 6 (Rewrite tests)
  ↓
Step 7 (Delete crate)
  ↓
Step 8 (Update docs)
```

Steps 2 and 3 can run in parallel after Step 1. Steps 4-8 are
sequential. Step 3 depends on the authority plan's pre-check being
in place.

## Step 1: Add `ChannelKind` to willow-state

**File**: `crates/state/src/types.rs`

Add the enum with serde attributes that accept both the enum variant
names and the legacy `"text"` / `"voice"` strings:

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelKind {
    #[default]
    Text,
    Voice,
}
```

Serde's default derive on an enum serializes as the variant name
(`"Text"`, `"Voice"`). Existing DAG events were serialized with
lowercase strings (`"text"`, `"voice"`). To handle both, add a
custom deserializer or use `#[serde(alias = "text")]` /
`#[serde(alias = "voice")]` on each variant:

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelKind {
    #[default]
    #[serde(alias = "text")]
    Text,
    #[serde(alias = "voice")]
    Voice,
}
```

Update `Channel` to use it:

```rust
pub struct Channel {
    pub id: String,
    pub name: String,
    pub pinned_messages: BTreeSet<EventHash>,
    #[serde(default)]
    pub kind: ChannelKind,  // was String
}
```

**File**: `crates/state/src/event.rs`

Update `EventKind::CreateChannel`:

```rust
CreateChannel {
    name: String,
    channel_id: String,
    #[serde(default)]
    kind: ChannelKind,  // was String
},
```

Delete the `default_create_channel_kind()` function (replaced by
`ChannelKind`'s `Default` impl).

**File**: `crates/state/src/materialize.rs`

Update `apply_mutation` for `CreateChannel` — `kind` is now a
`ChannelKind`, use `kind.clone()` directly.

**File**: `crates/state/src/lib.rs`

Add `ChannelKind` to the re-exports (currently only exports
`Channel, ChatMessage, Member, Profile, Role`).

**Tests**: Update the ~12 tests in `crates/state/src/tests.rs` that
construct `EventKind::CreateChannel` with `kind: "text".into()` to
use `ChannelKind::Text`. Add a round-trip test verifying that
`"text"` (legacy string) deserializes to `ChannelKind::Text`.

Run `cargo test -p willow-state`.

## Step 2: Move `Invite` types to willow-client

**Files**:
- `crates/client/src/invite.rs` — add `InviteId(Uuid)` and `Invite`
  struct here (moved from `crates/channel/src/lib.rs`).
- Remove the `willow_channel::Invite` and `InviteId` imports.

Update `generate_invite()` signature — instead of taking
`&willow_channel::Server`, take the data it needs directly:

```rust
pub fn generate_invite(
    server_name: &str,
    server_id: &str,
    genesis_author: EndpointId,
    channels: &[(String, String)],  // (topic, channel_name)
    keys: &HashMap<String, ChannelKey>,
    recipient_ed25519_public: &[u8; 32],
) -> Option<String>
```

Note: `genesis_author` is needed for invite validation on the
receiving side.

Update the 4 invite tests to construct state via `ManagedDag` instead
of `willow_channel::Server::new()`:
- `secure_invite_round_trip`
- `wrong_recipient_cannot_decrypt`
- `multiple_channels_encrypted`
- `generate_invite_via_endpoint_id_produces_valid_invite`

Run `cargo test -p willow-client`.

## Step 3: Rewrite client mutations to use event pipeline

**Prerequisite**: The permission pre-check from the authority plan must
be in place, so `create_and_insert` rejects unauthorized events before
signing.

Each sub-step removes the `willow_channel::Server` mutation and keeps
only the event pipeline call. Currently the code does both (mutates
Server AND emits event) — after this step, only the event remains.

### 3a: `mutations.rs` — `create_channel` (line ~317)

Remove `entry.server.create_channel(name, ChannelKind::Text)`.
Keep `build_event(EventKind::CreateChannel { ... })`. The channel ID
is generated as `Uuid::new_v4().to_string()`.

Channel keys: currently read from `entry.server.channel_key()` after
`create_channel()`. After removal, key generation moves to the event
handler — when `apply_mutation` processes `CreateChannel`, the client
generates and stores the key in `ServerEntry.keys`.

### 3b: `actions.rs` — `create_voice_channel` (line ~127)

Same pattern: remove `entry.server.create_channel(name, Voice)`,
keep `build_event(EventKind::CreateChannel { kind: ChannelKind::Voice, ... })`.

### 3c: `actions.rs` — role operations (lines ~195-267)

Replace `willow_channel::RoleId(Uuid::parse_str(...))` with plain
`String` role IDs, since state uses strings throughout.

### 3d: `mutations.rs` — role creation (line ~507)

Replace `willow_channel::RoleId::new()` and
`willow_channel::Role::with_id()` with
`build_event(EventKind::CreateRole { role_id: Uuid::new_v4().to_string(), ... })`.

### 3e: `servers.rs` — server creation (line ~97)

Replace `willow_channel::Server::new(&name, peer_id)` and
`server.create_channel("general", ChannelKind::Text)` with event
pipeline calls through `ManagedDag`.

Run `cargo test -p willow-client`.

## Step 4: Remove `Server` from `ServerEntry`

**Files**: `crates/client/src/state_actors.rs`, `crates/client/src/state.rs`

Remove `server: willow_channel::Server` from `ServerEntry`.

Remove `name: String` (read from `ServerState.server_name` instead).

Remove `topic_map: HashMap<String, (String, willow_channel::ChannelId)>`.
Derive topic-to-channel mapping from `ServerState.channels`:

```rust
/// Derive topic string → channel name from ServerState.
pub fn topic_for_channel(server_id: &str, channels: &BTreeMap<String, Channel>) -> HashMap<String, String> {
    channels.iter().map(|(_, ch)| {
        (format!("{}/{}", server_id, ch.name), ch.name.clone())
    }).collect()
}
```

### 4a: `util.rs`

Change `make_topic()` to take `&str` server ID instead of
`&willow_channel::Server`. Update all ~9 call sites.

### 4b: `views.rs`

Replace `willow_channel::ChannelKind` pattern matches with
`willow_state::ChannelKind`. Read channel list from
`ServerState.channels` instead of `entry.server.channels()`.

(Depends on Step 1 — `ChannelKind` must be in `willow-state` first.)

### 4c: `persistence_actor.rs`

Change `PersistServerConfig` and `PersistServerById` to hold
server data from `ServerState` fields, not `willow_channel::Server`.

### 4d: `storage.rs`

Update `save_server()` / `load_server()` to serialize `ServerState`
instead of `willow_channel::Server`.

### 4e: `lib.rs`

Delete `reconcile_topic_map()` (no longer two representations to sync).

Delete `parse_permission()` — callers import `willow_state::Permission`
directly. Update the call site in `actions.rs` (line ~206).

Run `cargo test -p willow-client`.

## Step 5: Rewrite invite join path

**File**: `crates/client/src/joining.rs`

The join path currently constructs a `willow_channel::Server` from
invite data (line ~75):

```rust
let mut server = willow_channel::Server::with_id(
    willow_channel::ServerId(parsed_server_uuid),
    &accepted.server_name,
    accepted.genesis_author,
);
```

Replace with constructing a minimal `ServerEntry` (without a `Server`)
populated from the invite's channel list and keys. The joining peer
does not have the full event history yet — it will receive the genesis
event and full DAG from peers during sync. The invite provides enough
information (server name, channels, keys) to bootstrap the UI while
sync completes.

Run `cargo test -p willow-client`.

## Step 6: Rewrite tests

**File**: `crates/client/src/lib.rs`

The `test_client()` helper (line ~691) constructs a
`willow_channel::Server`. Note: lines ~727-759 already partially use
`ManagedDag` — complete the migration by removing the `Server::new()`
on line ~700 and using `ManagedDag` exclusively.

Tests that reference `willow_channel` types:
- `send_message_and_read_back` (line ~971)
- `create_channel_shows_in_list` (line ~982)
- Invite tamper test (line ~1094)

**File**: `crates/client/src/invite.rs`

Rewrite all 4 test functions to construct servers via `ManagedDag`
instead of `Server::new()`.

**File**: `crates/client/src/servers.rs`

Update any test code that creates `willow_channel::Server` directly.

Run `cargo test -p willow-client`.

## Step 7: Delete willow-channel

- `rm -rf crates/channel/` (root `Cargo.toml` uses
  `members = ["crates/*"]`, so deleting the directory is sufficient).
- Remove `willow-channel` from `crates/client/Cargo.toml`.
- `willow-state` is already in `crates/client/Cargo.toml`.
- Verify no remaining imports: `grep -r willow_channel crates/`

Run `just check` (fmt + clippy + test + WASM).

## Step 8: Update documentation

**File**: `CLAUDE.md`

- Remove `willow-channel` from the repository structure listing.
- Remove `willow-channel` from the dependency graph.
- Update "Adding a new permission" section (currently references
  `willow-channel` for display purposes — remove that line).
- Remove the deprecation note under "Authority Model" (crate is gone).

Run `just check` one final time.
