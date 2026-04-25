# LLM Agent UX Spec

> **Status (round 2 audit):** This spec has been updated to reflect the
> post-state-machine refactor architecture. References to legacy
> `willow-channel`/`willow-messaging` paths have been replaced with the
> canonical `willow-state` event-sourced path where applicable. Items
> labeled **NEW** below are proposals that do not yet exist in the
> codebase; items labeled **EXISTING** are present today on `main`.
> Cross-cutting changes still required for this design (forward-compat
> for `EventKind`, additional `Permission` variants, atomic Kick+Rotate)
> are flagged inline.

Design specification for LLM-powered agents in Willow. Agents are first-class
participants in servers — they join channels, read messages, and reply like any
other member, but their identity, capabilities, and UI treatment are distinct.

---

## 1. Identity & Presence

### Agent Identity

An agent is a peer with a standard Ed25519 keypair. What distinguishes it from
a human member is a new `PeerKind` field. **NEW:** `PeerKind` does not currently
exist; the canonical `Profile` (`crates/state/src/types.rs`) only carries
`peer_id` and `display_name`, and `EventKind::SetProfile`
(`crates/state/src/lib.rs`) only carries `display_name`. This proposal adds the
field to both.

```rust
enum PeerKind {
    Human,
    Agent { provider: String },  // e.g. "claude-3", "local-llama"
}
```

To carry `PeerKind` (and the richer agent metadata in §8), this spec proposes
extending `EventKind::SetProfile` with optional fields:

```rust
EventKind::SetProfile {
    display_name: String,
    // NEW (additive, optional for backward compat — see §11):
    peer_kind: Option<PeerKind>,
    data_policy: Option<DataPolicy>,
}
```

- Agents MUST set a display name (e.g. "Claude", "Summary Bot").
- Agents MAY set an avatar and bio describing their capabilities (avatar/bio
  would also be additive optional fields on `SetProfile`).
- The `provider` string is informational — it tells other peers what model
  or service backs the agent, but has no protocol-level enforcement.

### Presence Indicators

| State | Meaning | Visual |
|-------|---------|--------|
| **Online** | Agent process is running and connected | Solid bot badge |
| **Thinking** | Agent is generating a response | Pulsing/animated badge |
| **Offline** | Agent process is not connected | Greyed-out badge |

Thinking state is signaled by a new ephemeral wire message (see
[Typing / Thinking Indicators](#6-typing--thinking-indicators)).

---

## 2. UI Treatment

### Member List

**EXISTING UI:** `crates/web/src/components/member_list.rs` currently renders
two sections in this order: Infrastructure (worker nodes) first, then Members
(regular peers).

**NEW:** Insert an **"Agents"** section between the two existing sections, so
the order becomes:

```
Infrastructure  (worker nodes — replay, storage)
Agents          (LLM agents)            ← new
Members         (human peers)
```

(Alternative considered: place Agents above Infrastructure to emphasize them
as participants rather than backend services. Rejected because Agents are
conceptually "automated participants" and group naturally near Infrastructure
while still being visually distinct from human peers.)

Each Agent entry shows:

- Bot badge icon (distinguishes from human members)
- Display name
- Provider tag (muted, e.g. "claude-3")
- Online/offline/thinking indicator

### Message Rendering

Agent messages render like human messages with these differences:

- **Bot badge:** Small icon next to the display name. **NEW:** No
  "verified badge" or "role badge" exists today in `crates/web/src/components/message.rs` —
  the bot badge is a wholly new UI element introduced by this spec.
- **"Agent" tag:** Subtle label after the name, similar to Discord's "BOT" tag.
- **No encryption indicator:** Agents that process cleartext for inference
  cannot claim E2E encryption. If the channel is encrypted, the agent's
  messages show a "processed in cleartext" notice (see [Privacy](#8-privacy)).
- **Streaming responses:** Long replies render incrementally as chunks arrive
  (see [Streaming](#5-streaming-responses)).
- **Collapsible:** Messages longer than 20 lines are auto-collapsed with a
  "Show more" toggle to keep chat scannable. This applies to all messages,
  not just agent responses.

### Message Actions

Agent messages support the same actions as human messages (reply, react,
pin, delete) with one addition:

- **"Regenerate"**: Available on the agent's most recent response in a thread.
  Sends a `RegenerateRequest` to the agent, which deletes the old response
  and produces a new one.

---

## 3. Interaction Model

### Invocation

Agents respond to messages in three ways:

1. **@mention**: `@Claude summarize the last hour` — explicit invocation.
   The agent receives all channel messages but only responds when mentioned.
2. **DM**: Direct messages to the agent peer. Always processed.
3. **Auto-respond channels**: Server owner can designate channels where an
   agent responds to every message (configurable via channel settings).

The default mode is **@mention only**. Auto-respond is opt-in per channel.

### Threading

When an agent responds to a message, it sets the `reply_to` field on its
`EventKind::Message` event (`crates/state/src/lib.rs`):

```rust
EventKind::Message {
    channel_id: String,
    body: String,
    reply_to: Option<String>,  // points at the invoking message's event ID
}
```

(There is also a legacy `Content::Reply { parent, body }` in
`crates/messaging/src/lib.rs`, but the canonical event-sourced path uses the
flat `reply_to` field on `EventKind::Message`. This spec uses the canonical
form.)

- The agent receives the full thread context when replying within a thread.
- Thread-scoped context avoids the agent responding to unrelated messages.
- Users can start a new thread with a fresh @mention to reset context.

### Slash Commands

Agents can register slash commands that appear in the command palette:

```
/ask <question>       — Ask the agent a question
/summarize [duration] — Summarize recent channel activity
/translate <lang>     — Translate the last message
```

Commands are announced via agent profile metadata and rendered in the UI's
command palette (`/` trigger). The command is sent as a regular message
prefixed with the command string — no new wire protocol needed.

---

## 4. Permissions & Trust

### Agent Permissions

Agents use the existing role-based permission system in
`willow_state::Permission` (`crates/state/src/types.rs`). The canonical enum
currently has these 7 variants:

`SyncProvider`, `ManageChannels`, `ManageRoles`, `KickMembers`,
`SendMessages`, `CreateInvite`, `Administrator`.

Note: there is no `ReadMessages` permission today. Read access is implicit
for any peer that has joined the server and decrypted the channel key —
event delivery is at the gossip layer, not gated by a permission. Likewise
there is no `ManageMessages`, `AttachFiles`, or `BanMembers` permission in
the canonical enum (those names exist only in the deprecated
`willow_channel::Permission`).

**Mapping agent capabilities to canonical permissions:**

| Agent capability | Canonical permission(s) |
|---|---|
| Read channel messages for context | (implicit — no permission needed) |
| Post responses | `SendMessages` |
| Pin / unpin messages | **NEW**: requires adding `ManagePins` permission, or reuse `ManageChannels` |
| Delete own messages | (implicit — author may always delete own) |
| Delete others' messages | **NEW**: requires adding `ModerateMessages` permission |
| Share generated files | **NEW**: file attachment is not yet a state event; would require an `EventKind::AttachFile` and an associated permission |

**Spec implication:** if agents need pin/moderate/attach capabilities, this
design depends on adding the corresponding `Permission` variants and the
matching event handlers in `apply_event` /
`required_permission()`. Those additions are out of scope for the agent
spec itself but must land before the corresponding agent capabilities ship.

Agents CAN be granted any permission, but the UI applies heavy friction
for dangerous permissions (`Administrator`, `ManageRoles`, `KickMembers`).
Attempting to grant these shows a confirmation dialog explaining the
risks, requiring the owner to explicitly acknowledge.

### Trust Model

- Agents are added to a server via the standard invite flow.
- The server owner (or admin) must explicitly trust the agent peer.
- Agent trust can be revoked at any time. Revocation is a two-event
  sequence emitted by the client: an `EventKind::KickMember` followed
  by an `EventKind::RotateChannelKey` for each affected channel.
  These are **not atomic at the state-machine layer** —
  `KickMember` only removes the member and their permissions
  (`crates/state/src/lib.rs`); key rotation is a separate event.
  The client is responsible for emitting both. (Future work: an
  atomic `EventKind::KickAndRotate` could enforce the linkage at the
  state machine layer; flagged but out of scope here.)
- Agents cannot trust other peers or grant permissions.

### Rate Limiting

To prevent runaway agents from flooding channels:

- **Server-level**: Owner configures max messages per minute per agent
  (default: 10/min). Excess messages are silently dropped.
- **Channel-level**: Optional per-channel override.
- **UI feedback**: If an agent hits the rate limit, a system message appears:
  "Agent rate limited — some responses may be delayed."

---

## 5. Streaming Responses

LLM responses are often long and generated token-by-token. Streaming provides
real-time feedback.

### Wire Format (ephemeral, not state events)

Token chunks are **ephemeral wire messages**, not state events. They flow
through `WireMessage` (`crates/common/src/wire.rs`) — the same envelope used
for `TypingIndicator` — and are not appended to the event store. Only the
final completed message is recorded as an `EventKind::Message`.

**NEW** variants on `WireMessage`:

```rust
enum WireMessage {
    // ... existing variants (Event, SyncRequest, SyncBatch, TypingIndicator,
    //     VoiceJoin, VoiceLeave, VoiceSignal, JoinRequest, JoinResponse,
    //     JoinDenied) ...
    StreamStart  { stream_id: Uuid, channel: String, reply_to: Option<String> },
    StreamChunk  { stream_id: Uuid, body: String },
    StreamEnd    { stream_id: Uuid, final_event_id: String },
    StreamCancel { stream_id: Uuid },
}
```

- `StreamStart` opens an ephemeral message bubble in the receiving UI.
- `StreamChunk` appends text to the in-progress bubble. Chunks are ordered
  by HLC. Clients buffer and display in order.
- `StreamEnd` closes the stream and references `final_event_id` — the ID of
  the `EventKind::Message` event that the agent has now broadcast carrying
  the complete body. Receivers replace the ephemeral bubble with the
  materialized event.
- `StreamCancel` is sent by a user to interrupt; the agent responds by
  emitting `StreamEnd` with whatever has been generated so far (along with
  the corresponding `EventKind::Message`).

### UI Behavior

- During streaming: blinking cursor at end of message, "Stop" button visible.
- **Stop**: User clicks "Stop" → sends `WireMessage::StreamCancel`. The
  agent emits `StreamEnd` with whatever was generated so far and a final
  `EventKind::Message`.
- After streaming: message renders as a normal message, sourced from the
  state event. No visual difference from a non-streamed message in history.
- If the user scrolls up during streaming, the scroll position stays locked.
  A "New messages" pill appears to jump back down.

### Replay Semantics

On event replay (new peer joining, reconnecting), only the final
`EventKind::Message` is materialized. Stream chunks are ephemeral — they
exist only in `WireMessage` traffic and are not persisted in the event
store. This keeps the event log compact.

---

## 6. Typing / Thinking Indicators

### Protocol

Indicators are **NEW** ephemeral variants on the existing `WireMessage`
enum (`crates/common/src/wire.rs`). They are not state events and are not
persisted. (There is no `EphemeralMessage` enum — typing already lives as
`WireMessage::TypingIndicator { channel: String }` today.)

```rust
enum WireMessage {
    // ... existing variants ...
    TypingIndicator { channel: String },                // EXISTING

    // NEW — additive, ephemeral:
    Thinking         { channel: String, agent_peer: String },
    StoppedThinking  { channel: String, agent_peer: String },
}
```

- Agents send `Thinking` when they begin inference.
- Agents send `StoppedThinking` when they begin streaming or abandon the
  request.
- Timeout: if no `StoppedThinking` arrives within 30 seconds, the UI
  auto-clears the indicator.

### UI

- Below the message input: "Claude is thinking..." with an animated
  ellipsis, same position as "Alice is typing..."
- In the member list: the agent's status shows a pulsing indicator.

---

## 7. Agent Configuration UI

### Channel Settings Panel

When an agent is a server member, channel settings gain an **"Agents"** tab:

```
[Agents]
  Claude (claude-3)           [Enabled] [Auto-respond: Off]
  Summary Bot (local-llama)   [Enabled] [Auto-respond: On ]

  Rate limit: [10] messages/min
```

- **Enabled**: Whether the agent can read/respond in this channel.
- **Auto-respond**: Whether the agent responds to every message (vs.
  @mention only).
- **Rate limit**: Per-channel override of the server-wide rate limit.

### Server Settings > Agents

A dedicated section in server settings for managing agents:

- List of all agent members with their provider, roles, and status.
- "Add Agent" button → generates an invite link.
- Per-agent configuration: display name override, default channels,
  system prompt (visible to the agent as context).
- "Remove Agent" → emits `EventKind::KickMember` followed by
  `EventKind::RotateChannelKey` for each affected channel (see §4
  Trust Model — the linkage lives in the client, not the state machine).

### System Prompt

Server owners can set a per-agent system prompt that is sent to the agent
as context. This allows customizing agent behavior per server:

```
You are a helpful assistant for the Willow dev team. Be concise.
Focus on Rust and P2P networking topics. Use code blocks for code.
```

The system prompt is stored as server metadata via a **NEW**
`EventKind::SetAgentConfig` event (see §10) and transmitted to the agent
on join/sync.

---

## 8. Privacy & Encryption

### Encryption

Agents participate in E2E encryption the same way human members do. They
receive channel keys via the invite flow, decrypt incoming messages locally,
and encrypt outgoing messages with the channel key. There is no special
"cleartext" mode — from the protocol's perspective, an agent is just another
member.

The only distinction is what happens after decryption: a `CloudProvider`
agent may send decrypted content to an external API for inference (see
[Data Handling Disclosure](#data-handling-disclosure) below). A `Local`
agent keeps everything on-device. This is a deployment concern, not a
protocol concern.

### Data Handling Disclosure

The agent's profile carries a `data_policy` field (added via the extended
`SetProfile` proposed in §1):

```rust
enum DataPolicy {
    Local,           // Inference runs locally, no data leaves the machine
    CloudProvider {   // Data sent to a cloud API
        provider: String,
        region: Option<String>,
    },
}
```

- The member list tooltip shows the data policy.
- When a user first @mentions an agent with `CloudProvider` policy, a
  one-time confirmation dialog appears: "This agent sends messages to
  [provider] for processing. Continue?"

---

## 9. Agent as Worker Node

### Implementation

`WorkerRole` (`crates/common/src/worker_types.rs`) is a **trait**, and
`WorkerRoleInfo` is a **separate enum** carrying capacity-only data. The
existing comment in that file flags `Bot` (not `Agent`) as a future variant —
this spec adopts the `Bot` naming for consistency.

This proposal adds:

1. **A new `WorkerRoleInfo::Bot` variant** for heartbeat capacity info:

   ```rust
   enum WorkerRoleInfo {
       Replay  { /* ... */ },                      // EXISTING
       Storage { /* ... */ },                      // EXISTING
       Bot {                                       // NEW
           inference_in_flight: u32,
           inference_capacity: u32,
       },
   }
   ```

   Following the existing pattern, `Bot` carries **capacity only** —
   identity (`display_name`, `provider`, `data_policy`, capabilities) lives
   on the agent's `Profile`/`SetProfile`, not on `WorkerRoleInfo`.

2. **A new `WorkerRole` trait implementer** — e.g. `BotWorker` — owned by
   the bot binary:

   ```rust
   struct BotWorker {
       config: AgentConfig,
       inference: Box<dyn InferenceBackend>,
   }

   impl WorkerRole for BotWorker {
       fn role_info(&self) -> WorkerRoleInfo { /* Bot { .. } */ }
       fn on_event(&mut self, event: &willow_state::Event) { /* see below */ }
       fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse { /* ... */ }
   }

   trait InferenceBackend: Send + 'static {
       fn generate(&mut self, prompt: Vec<ChatMessage>) -> Stream<String>;
       fn cancel(&mut self);
   }
   ```

This plugs into the existing worker node runtime. The data flow:

1. The worker subscribes to the well-known gossipsub topics
   `_willow_workers` and `_willow_server_ops` (`crates/common/src/worker_types.rs`).
   There is no per-channel topic; channel scoping is achieved by inspecting
   the `channel_id` field on `EventKind::Message` events.
2. Incoming `WireMessage::Event` envelopes are unpacked and the inner
   `willow_state::Event` is delivered to `WorkerRole::on_event(&Event)`.
   The bot filters `EventKind::Message { channel_id, body, reply_to }` events
   for @mentions and auto-respond channels.
3. The bot builds a prompt from thread context + system prompt.
4. Calls `inference.generate()` and emits chunks back via
   `WireMessage::StreamChunk` plus a final `EventKind::Message` event when
   complete (see §5).

### Worker Announcement

`WorkerAnnouncement` (`crates/common/src/worker_types.rs`) carries the
`WorkerRoleInfo` (capacity only) plus the `servers: Vec<String>` the worker
is participating in. Identity / display info lives on the agent's
`SetProfile` event, **not** on the announcement. This matches the existing
Replay/Storage pattern and avoids duplicating identity data.

### Configuration

Agent worker nodes are configured via a TOML file:

```toml
[agent]
display_name = "Claude"
provider = "claude-3"
data_policy = "cloud"
cloud_provider = "Anthropic"

[inference]
backend = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"
model = "claude-sonnet-4-20250514"
max_tokens = 4096

[behavior]
system_prompt = "You are a helpful assistant."
auto_respond_channels = []
rate_limit = 10  # messages per minute
max_context_messages = 50
```

Note: at the time this spec branch was authored, no agent crate or binary
exists in this PR's worktree (`crates/`) — this design predates any agent
implementation. (`main` may have introduced a `crates/agent/` crate for an
unrelated MCP server; that crate, if present, has a different scope and
should not be conflated with the LLM-agent worker described here.)

---

## 10. State Machine Changes

### New EventKind Variants

```rust
enum EventKind {
    // ... existing variants ...

    /// Configure an agent's behavior in this server.
    SetAgentConfig {
        peer_id: String,
        system_prompt: Option<String>,
        auto_respond_channels: Vec<String>,
        rate_limit: Option<u32>,
    },
}
```

### Streaming Is Not a State Event

Stream chunks are **wire-only** (`WireMessage::StreamStart` /
`StreamChunk` / `StreamEnd` / `StreamCancel`, see §5). They do not appear
on `EventKind`. Only the final `EventKind::Message` carrying the full body
is recorded in the event store. This avoids polluting the canonical event
log with thousands of token-sized events and keeps replay cheap.

### Permission Check

`SetAgentConfig` requires `Administrator` or server ownership. Agents
themselves cannot modify their own config. (Add this row to
`required_permission()` in `crates/state/src/materialize.rs` when
implementing.)

---

## 11. Migration & Compatibility

### State events

`EventKind` is bincode-serialized as a tagged enum
(`crates/transport/src/lib.rs`). Old peers therefore **fail at the
deserialization layer** when they encounter a new variant — they never
reach `apply_event`, and there is no `ApplyResult::Skipped` (the actual
variants are `Applied`, `AlreadySeen`, `ParentHashMismatch`, `Rejected`).

To allow new `EventKind` variants without breaking old peers, this spec
proposes adding a forward-compat catch-all:

```rust
enum EventKind {
    // ... existing variants ...

    /// Forward-compat: an unknown event variant. Old peers
    /// deserialize-then-skip; new peers may understand it.
    Unknown { kind: String, payload: Vec<u8> },
}
```

The transport-layer deserializer would fall back to `Unknown` when the
tag is not recognized, and `apply_event` would treat `Unknown` as a no-op.
This is a **prerequisite** for shipping `SetAgentConfig` (and any other new
variant) without forcing a flag-day upgrade. If we choose not to add this,
the spec must accept that `SetAgentConfig` is a breaking change that
requires all peers to upgrade simultaneously.

### Wire messages

`WireMessage` is also a bincode tagged enum. New ephemeral variants
(`StreamStart`/`Chunk`/`End`/`Cancel`, `Thinking`, `StoppedThinking`)
likewise need either:

1. A `WireMessage::Unknown { tag: u32, payload: Vec<u8> }` catch-all (older
   peers ignore unknown wire messages silently — acceptable since they're
   ephemeral); or
2. A flag-day upgrade.

Old clients silently drop unknown `WireMessage` envelopes today; there is
**no** "[unsupported message type]" UI fallback — that string does not
exist in `crates/web/src/components/message.rs` or anywhere else in the
codebase. This spec does not propose adding such a fallback (ephemeral
streaming chunks have no UI representation in old clients, which is fine —
they'll just see the final materialized message).

### No silent breakage

All additions in this spec are designed to be additive: new optional fields
on `SetProfile`, new tagged variants behind an `Unknown` catch-all, new
ephemeral wire messages. With the `Unknown` fallback in place, old clients
continue to function with reduced capability rather than panicking on
deserialization.

---

## 12. Implementation Phases

### Phase 0: Forward-compat foundation (Prerequisite)

- Add `EventKind::Unknown` and `WireMessage::Unknown` catch-all variants
  with custom deserializer fallback (see §11). Without this, every later
  phase is a breaking wire change.

### Phase 1: Identity & UI (Foundation)

- Extend `EventKind::SetProfile` with optional `peer_kind` and `data_policy`.
- Add `PeerKind` to profile broadcasts.
- Insert "Agents" section into the member list (between Infrastructure and
  Members).
- Add bot badge and "Agent" tag to message rendering.
- Add collapsible long messages: auto-collapse messages over 20 lines with
  a "Show more" toggle in `crates/web/src/components/message.rs` (no
  collapse logic exists today; this is net-new UI).

### Phase 2: Interaction (Core Loop)

- Implement @mention detection and routing.
- Add `BotWorker` implementing `WorkerRole`, with `InferenceBackend` trait
  and a new `WorkerRoleInfo::Bot` variant.
- Add agent configuration TOML loader and a new `willow-bot` binary
  (note: this PR worktree has no agent/bot crate yet — see §9).
- Thread-scoped context building (uses `reply_to` chain on
  `EventKind::Message`).

### Phase 3: Streaming (Polish)

- Add `WireMessage::StreamStart` / `StreamChunk` / `StreamEnd` /
  `StreamCancel` variants.
- Implement streaming message rendering in the web UI.
- Add stop button and cancel protocol.
- Add `WireMessage::Thinking` / `StoppedThinking` ephemeral indicators.

### Phase 4: Configuration & Privacy (Trust)

- `EventKind::SetAgentConfig` event and server settings UI.
- Per-channel agent enable/auto-respond toggles.
- Rate limiting enforcement.
- Data policy disclosure and confirmation dialog.
- Encryption transparency notices.

### Phase 5: Slash Commands & Discovery

- Agent-registered command metadata in profile.
- Command palette integration.
- Regenerate action on agent messages.

### Phase 6 (optional): Atomic Kick+Rotate

- If two-event Kick→Rotate proves error-prone in practice, add an atomic
  `EventKind::KickAndRotate { peer_id, encrypted_keys: Vec<(channel_id,
  Vec<(peer_id, Vec<u8>)>)> }` to enforce the linkage at the state-machine
  layer.
