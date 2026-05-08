# LLM Agent UX Spec

> **Status (round 3 audit):** This spec has been re-aligned against the
> post-state-machine refactor and the 2026-04-19 profile-card work. The
> canonical `Profile` and `ProfileDelta` schemas now carry rich identity
> fields, and admin/kick semantics live entirely in the governance / vote
> path (`ProposedAction::GrantAdmin` / `KickMember`). Items labeled
> **NEW** below are proposals that do not yet exist in the codebase;
> items labeled **EXISTING** are present today on `main`. Cross-cutting
> changes still required for this design (forward-compat envelope for
> `EventKind` + `WireMessage`, additional `Permission` variants, atomic
> Kick+Rotate, naming relationship to the existing `willow-agent` MCP
> crate) are flagged inline.

Design specification for LLM-powered agents in Willow. Agents are first-class
participants in servers — they join channels, read messages, and reply like any
other member, but their identity, capabilities, and UI treatment are distinct.

---

## 1. Identity & Presence

### Agent Identity

An agent is a peer with a standard Ed25519 keypair. What distinguishes it from
a human member is a new `PeerKind` field. **NEW:** `PeerKind` does not currently
exist. The canonical `Profile` (`crates/state/src/types.rs:94-128`) carries 10
fields today (`peer_id`, `display_name`, `pronouns`, `bio`, `tagline`,
`crest_pattern`, `crest_color`, `pinned`, `elsewhere`, `since`) and is mutated
via two events: a legacy `EventKind::SetProfile { display_name }` (at
`crates/state/src/event.rs:148`) and the canonical
`EventKind::UpdateProfile(Box<ProfileDelta>)`
(`crates/state/src/event.rs:150-168`) — a partial-field overlay with explicit
"unchanged / clear / set" semantics. This proposal extends both `Profile` and
`ProfileDelta` with `peer_kind` and `data_policy`, following the same
`#[serde(default)]` additive pattern already used for `pronouns`, `bio`,
`tagline`, `crest_*`, `pinned`, `elsewhere`, and `since`.

```rust
enum PeerKind {
    Human,
    Agent { provider: String },  // e.g. "claude-3", "local-llama"
}
```

The additive extensions are:

```rust
// In crates/state/src/types.rs:
pub struct Profile {
    // ... existing 10 fields ...
    #[serde(default)]
    pub peer_kind: Option<PeerKind>,
    #[serde(default)]
    pub data_policy: Option<DataPolicy>,
}

// In crates/state/src/types.rs (ProfileDelta — used by UpdateProfile):
pub struct ProfileDelta {
    // ... existing fields ...
    pub peer_kind: Option<Option<PeerKind>>,    // outer = "unchanged",
    pub data_policy: Option<Option<DataPolicy>>, // inner = "clear vs set"
}
```

New writes go through `EventKind::UpdateProfile(Box<ProfileDelta>)` —
that is the canonical extend-pattern. `EventKind::SetProfile { display_name }`
is retained for legacy display-name-only writes; this spec does NOT add
fields to it.

- Agents MUST set a display name (e.g. "Claude", "Summary Bot").
- `bio` and `tagline` already exist on `Profile` — agents can describe their
  capabilities there with no schema change.
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

- **"Regenerate"**: Available on the agent's most recent response in a
  thread. Sends a `RegenerateRequest` to the agent. The agent then emits:
  1. `EventKind::DeleteMessage { message_id }`, where `message_id` is the
     `EventHash` of the agent's previous response. This is a soft-delete
     tombstone, not a hard delete — replay still sees the original event.
     **Author-only is already enforced:** `apply_event(DeleteMessage)`
     in `crates/state/src/materialize.rs:467-477` only flips the
     `deleted` flag when `msg.author == event.author` (the same gate
     `EditMessage` applies at L453-465); a non-author DeleteMessage is
     silently ignored at the apply layer (it is not surfaced as
     `Rejected(_)`). Regenerate (delete-own + new-message) type-checks
     against this existing rule with no state-machine change required.
     A separate, looser proposal — letting moderators delete *others'*
     messages, gated by a NEW `ModerateMessages` permission — is
     tracked in §4 under "Delete others' messages" and is out of scope
     for shipping Regenerate.
  2. A new `EventKind::Message { channel_id, body, reply_to }` carrying
     the regenerated response. `reply_to` references the same parent
     event as the deleted response so threading is preserved.

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
`EventKind::Message` event (`crates/state/src/event.rs:128-132`):

```rust
EventKind::Message {
    channel_id: String,
    body: String,
    reply_to: Option<EventHash>,  // 32-byte SHA-256 of the invoking event
}
```

`reply_to` is `Option<EventHash>` — not `Option<String>`. `EventHash` is the
content-addressed hash of the parent `Event` (see
`crates/state/src/hash.rs`). Threading is structural: the agent
references the invoking message's event hash directly, and clients walk
the `reply_to` chain to reconstruct thread context.

Note on the parallel `willow-messaging` API: `Content::Reply { parent, body }`
in `crates/messaging/src/lib.rs:124-130` is **not legacy** — it is the live
encrypted-content path that drives `Content::Encrypted(SealedContent)` and
the E2E pipeline (`crates/crypto`). The two paths coexist:

- `EventKind::Message.body: String` is plaintext today and travels in the
  state DAG.
- `Content::*` (including `Reply`) travels through `Message` → `SealedContent`
  for E2E channels.

This spec assumes agent messages flow on the `EventKind::Message` path.
**Open design question (Phase 4 / §8):** if `EventKind::Message.body` must
be encrypted on E2E channels, either (a) the body must itself be a
`SealedContent`-style ciphertext blob (a breaking schema change) or
(b) agents must publish via the `Content`-based message path and the
event-sourced `body` must be reserved for an opaque envelope. Pick one
before shipping encryption-aware agents — see §8.

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
`willow_state::Permission` (`crates/state/src/event.rs:21-33`). The
canonical enum has exactly **5 variants** today:

`SyncProvider`, `ManageChannels`, `ManageRoles`, `SendMessages`, `CreateInvite`.

**There is no `Administrator` permission and no `KickMembers` permission.**
Admin status and member removal both flow through the governance / vote path:

- **Admin status** is conferred via
  `EventKind::Propose { action: ProposedAction::GrantAdmin { peer_id } }`
  followed by `EventKind::Vote` events that meet the configured
  `VoteThreshold` (Majority / Unanimous / Count(n))
  (`crates/state/src/event.rs:43-64`). It is checked at runtime via
  `state.is_admin(author)`, NOT via a `Permission` enum variant. The
  server owner (genesis author) has implicit unilateral override.
- **Member removal** is
  `EventKind::Propose { action: ProposedAction::KickMember { peer_id } }`
  + votes (same threshold rules). There is no direct `EventKind::KickMember`.

Note: there is no `ReadMessages` permission today. Read access is implicit
for any peer that has joined the server and decrypted the channel key —
event delivery is at the gossip layer, not gated by a permission. Likewise
there is no `ManageMessages`, `AttachFiles`, or `BanMembers` permission.
The `willow_channel` crate referenced in earlier drafts no longer exists.

**Mapping agent capabilities to canonical permissions:**

| Agent capability | Canonical mechanism |
|---|---|
| Read channel messages for context | (implicit — no permission needed) |
| Post responses | `Permission::SendMessages` |
| Pin / unpin messages | (implicit today — `PinMessage`/`UnpinMessage` are unrestricted in `required_permission()`; tighten by adding a `ManagePins` permission if needed) |
| Delete own messages | (implicit — `DeleteMessage` requires `SendMessages`; only the author should be able to delete their own message — see §5/§9) |
| Delete others' messages | **NEW**: requires adding a `ModerateMessages` permission and gating `DeleteMessage` by author-or-moderator |
| Share generated files | **NEW**: file attachment is not yet a state event; would require an `EventKind::AttachFile` and an associated permission |

**Spec implication:** if agents need moderate/attach capabilities, this
design depends on adding the corresponding `Permission` variants and the
matching event handlers in `apply_event` / `required_permission()`
(`crates/state/src/materialize.rs:280-318`). Those additions are out of
scope for the agent spec itself but must land before the corresponding
agent capabilities ship.

Agents CAN be granted any of the 5 permissions, but the UI applies heavy
friction for dangerous capabilities. Specifically:

- Granting `ManageRoles` shows a confirmation dialog (the agent could
  re-grant other permissions to itself or third parties).
- Proposing `ProposedAction::GrantAdmin { peer_id: <agent> }` shows an
  even stronger confirmation — admin status enables proposing kicks and
  changing the vote threshold.
- The UI also surfaces the multi-vote nature of these proposals so the
  owner understands a single click is not enough on multi-admin servers.

### Trust Model

- Agents are added to a server via the standard invite flow.
- The server owner (or admin) must explicitly trust the agent peer.
- Agent trust can be revoked. Revocation is a multi-event sequence:
  1. An admin emits
     `EventKind::Propose { action: ProposedAction::KickMember { peer_id } }`
     (`crates/state/src/event.rs:43-49`).
  2. Other admins emit `EventKind::Vote { proposal, accept: true }` until
     the configured `VoteThreshold` is met (the server owner can push
     unilaterally per the owner-override path in
     `check_and_apply_proposal` —
     `crates/state/src/materialize.rs:188-209`, owner-override check
     at L197-201; `apply_proposed_action` at L212-245 contains no
     owner-override).
  3. Once the proposal applies, the proposing admin (or any
     `ManageChannels` holder) emits
     `EventKind::RotateChannelKey { channel_id, encrypted_keys }` for
     each channel the kicked agent had access to.

  These steps are **not atomic at the state-machine layer**: the kick
  removes membership/permissions; key rotation is a separate event the
  client must emit. Future work: a new `ProposedAction::KickAndRotate`
  variant (or a top-level `EventKind::KickAndRotate`) could enforce the
  linkage in the state machine — see Phase 6. Pick exactly one shape
  before implementing; mixing both adds two ways to express the same
  thing.
- Agents cannot trust other peers or grant permissions, and cannot
  themselves be admins unless explicitly elected via
  `ProposedAction::GrantAdmin`.

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

**NEW** variants on `WireMessage` (`crates/common/src/wire.rs:13`).
Existing variants today: `Event`, `SyncRequest`, `SyncBatch`,
`TypingIndicator`, `VoiceJoin`, `VoiceLeave`, `VoiceSignal`, `JoinRequest`,
`JoinResponse`, `JoinDenied`, `TopicAnnounce`, `ProfileAnnounce`, `Worker`.

```rust
enum WireMessage {
    // ... existing variants ...
    StreamStart  { stream_id: Uuid, channel: String, reply_to: Option<EventHash> },
    StreamChunk  { stream_id: Uuid, body: String },
    StreamEnd    { stream_id: Uuid, final_event_id: EventHash },
    StreamCancel { stream_id: Uuid },
}
```

`reply_to` and `final_event_id` are `EventHash` — matching
`EventKind::Message.reply_to` (`crates/state/src/event.rs:131`) and event
identity throughout the state machine.

- `StreamStart` opens an ephemeral message bubble in the receiving UI.
- `StreamChunk` appends text to the in-progress bubble. Chunks are ordered
  by HLC. Clients buffer and display in order.
- `StreamEnd` closes the stream and references `final_event_id` — the
  `EventHash` of the `EventKind::Message` event the agent has now
  broadcast carrying the complete body. Receivers replace the
  ephemeral bubble with the materialized event.
- `StreamCancel` is sent by a user to interrupt. On cancel, the agent
  emits `StreamEnd` along with an `EventKind::Message` carrying
  **whatever was generated so far** — i.e. the partial body becomes a
  permanent state event. (Alternative considered: drop the partial
  output entirely so cancel produces no state event. Rejected because
  the user has already seen the partial text on screen and a silent
  history erase is more confusing than a truncated message.)

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
- "Remove Agent" → emits
  `EventKind::Propose { action: ProposedAction::KickMember { peer_id } }`,
  collects vote events to threshold, then emits
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

> **Open design question (cross-references §3).** Today
> `EventKind::Message.body` is a plain `String` and travels in the state
> DAG. Encrypted human messages travel through the parallel
> `Content`/`SealedContent` API in `crates/messaging` + `crates/crypto`.
> For agent messages on E2E channels, the spec must decide between:
> (a) treating `EventKind::Message.body` as opaque ciphertext (e.g. base64
> SealedContent) and decrypting in the rendering layer, or (b) routing
> agent output through the `Content`-based path and reserving
> `EventKind::Message` for plaintext-only channels. (a) keeps the
> event-sourced path uniform but requires schema changes; (b) preserves
> the existing encryption pipeline but bifurcates how agents and humans
> publish. Pick before shipping Phase 4.

### Data Handling Disclosure

The agent's profile carries a `data_policy` field (added via
`Profile`/`ProfileDelta` in §1):

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

`WorkerRole` (`crates/common/src/worker_types.rs:131`) is a **trait**, and
`WorkerRoleInfo` (`crates/common/src/worker_types.rs:20`) is a **separate
enum** carrying capacity-only data. The comment at line 35 of that file
already flags `Bot` (not `Agent`) as a future variant — this spec adopts
the `Bot` naming for the in-protocol LLM worker.

> **Naming note (resolves round 3 collision).** The existing crate
> `crates/agent/` ships the `willow-agent` binary, an MCP server that
> exposes a Willow `ClientHandle` to AI agents over JSON-RPC
> (`crates/agent/Cargo.toml`). It is **not** an in-protocol LLM bot — it
> is a host-side bridge for external LLMs. To avoid collision, this spec
> introduces a **new** crate `crates/bot/` shipping a `willow-bot`
> binary that implements `WorkerRole`. The runner-up was reusing
> `crates/agent/` and adding a second binary inside it, but that
> conflates two very different scopes (MCP transport vs in-protocol
> participation) and forces shared dependencies that the MCP server
> doesn't need (e.g. inference SDKs).

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
   on the agent's `Profile`/`UpdateProfile`, not on `WorkerRoleInfo`.

   Adding the variant requires:

   - Updating `WorkerRoleInfo::role_name()`
     (`crates/common/src/worker_types.rs:40-46`) — the match is
     exhaustive today (no `_` catch-all), so adding the `Bot` variant
     requires a matching arm `WorkerRoleInfo::Bot { .. } => "bot"` or
     the build breaks.
   - Considering `PartialEq` / serde round-trips: `WorkerRoleInfo`
     derives both `PartialEq` and `Serialize`/`Deserialize`, so existing
     round-trip tests in `crates/common/src/worker_types.rs` should
     gain a `Bot` round-trip case.

2. **A new `WorkerRole` trait implementer** — `BotWorker` — owned by
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

1. The worker subscribes to the well-known topics
   `network::topics::SERVER_OPS_TOPIC` and `network::topics::WORKERS_TOPIC`
   for membership / governance / sync coordination, **and** to
   `network::topics::channel_topic(server_id, channel_id)` for each
   channel it participates in
   (`crates/network/src/topics.rs:17-49`). Per-channel topics DO exist
   today: `channel_topic(server_id, channel_id)` and
   `voice_topic(server_id, channel_id)` are how channel-scoped gossip
   already flows. The earlier draft of this spec wrongly claimed
   per-channel topics did not exist.

   The spec prefers the typed `network::topics::*` re-exports (or the
   `*_TOPIC` `LazyLock<TopicId>` statics) over the string constants
   `WORKERS_TOPIC` / `SERVER_OPS_TOPIC` in
   `common::worker_types`.
2. Incoming `WireMessage::Event` envelopes are unpacked and the inner
   `willow_state::Event` is delivered to `WorkerRole::on_event(&Event)`.
   The bot filters `EventKind::Message { channel_id, body, reply_to }`
   events for @mentions and auto-respond channels.
3. The bot builds a prompt from thread context (walking the `reply_to`
   chain) plus the per-server system prompt (see §10).
4. Calls `inference.generate()` and emits chunks back via
   `WireMessage::StreamChunk` plus a final `EventKind::Message` event
   when complete (see §5). The final event flows on the appropriate
   `channel_topic`, not on `_willow_server_ops`.

### Worker Announcement

`WorkerAnnouncement` (`crates/common/src/worker_types.rs:50-55`) carries
the `WorkerRoleInfo` (capacity only) plus the `servers: Vec<String>` the
worker is participating in. Identity / display info lives on the agent's
`Profile` (mutated via `EventKind::UpdateProfile`), **not** on the
announcement. This matches the existing Replay/Storage pattern and avoids
duplicating identity data.

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

Note: `crates/agent/` (the `willow-agent` MCP server binary) exists today
on the rebased branch. It is **not** the in-protocol LLM bot described
here — see the naming-note callout above. This spec proposes a separate
`crates/bot/` crate shipping `willow-bot`, leaving `willow-agent` for the
MCP/JSON-RPC bridge to external agents.

---

## 10. State Machine Changes

### New EventKind Variants

```rust
enum EventKind {
    // ... existing variants ...

    /// Configure an agent's behavior in this server.
    SetAgentConfig {
        peer_id: EndpointId,                  // 32-byte Ed25519 pubkey
        system_prompt: Option<String>,
        auto_respond_channels: Vec<String>,
        rate_limit: Option<u32>,
    },
}
```

`peer_id` is `EndpointId` (re-exported from iroh-base via
`willow-identity`), matching the rest of the state machine — not `String`.

### Streaming Is Not a State Event

Stream chunks are **wire-only** (`WireMessage::StreamStart` /
`StreamChunk` / `StreamEnd` / `StreamCancel`, see §5). They do not appear
on `EventKind`. Only the final `EventKind::Message` carrying the body
the agent committed to (full or, on cancel, partial) is recorded in the
event store. This keeps the canonical event log free of thousands of
token-sized events and keeps replay cheap.

### Permission Check

`SetAgentConfig` is an **admin-only event**. There is no `Administrator`
permission in `Permission`; the gate is `state.is_admin(author)`, which
lives in the dedicated admin-only block of `check_permission`
(`crates/state/src/materialize.rs:117-126`) — NOT in
`required_permission()` (which is for fine-grained `Permission`-gated
events only, lines 280-318). Implementation:

- Add `EventKind::SetAgentConfig { .. }` to the `matches!` arm at
  `crates/state/src/materialize.rs:117-123`, alongside `GrantPermission`,
  `RevokePermission`, `RenameServer`, `SetServerDescription`.
- Do **not** add it to `required_permission()`; that table only maps
  variants to `Permission` enum values.
- Agents themselves cannot modify their own config — they are not
  admins by default, and even if elected admin via
  `ProposedAction::GrantAdmin` the UX surfaces this clearly.

---

## 11. Migration & Compatibility

### State events

`EventKind` is bincode-serialized as a tagged enum
(`crates/transport/src/lib.rs`). Old peers **fail at the
deserialization layer** when they encounter a new variant — they never
reach `apply_event`. `ApplyResult`
(`crates/state/src/materialize.rs:33-41`) has exactly **3 variants**:

```rust
pub enum ApplyResult {
    Applied,
    Rejected(String),
    AlreadyApplied,
}
```

There is no `AlreadySeen`, no `ParentHashMismatch`, no `Skipped`.
Per-author chain mismatches (the `prev` field on `Event`) are caught in
the DAG layer before `apply_event` runs, not represented in
`ApplyResult`.

To allow new `EventKind` variants without forcing a flag-day upgrade,
this spec needs a forward-compat catch-all. **Important:** bincode's
default tagged-enum format does **not** support a "catch-all unknown
variant" out of the box — encountering an unrecognized discriminant
returns `Err`, and `unpack_wire` returns `None` with no fallback
opportunity. Two viable approaches:

**Option A: Custom `Deserialize` impl with explicit fallback.**
Hand-write `Deserialize` for `EventKind` that reads the tag manually,
attempts the per-variant payload, and falls through to a synthetic
`EventKind::Unknown { tag: u32, payload: Vec<u8> }` when the tag is not
recognized. Brittle — any future re-ordering of variants silently
shifts tag numbers, so this requires explicit `#[serde(rename = "...")]`
or a hand-managed tag table.

**Option B: Versioned envelope (preferred).** Wrap every event payload
in a stable outer envelope so the transport layer knows the variant tag
without deserializing the payload:

```rust
struct VersionedEvent {
    kind_tag: u32,           // stable, hand-assigned per EventKind variant
    payload: Vec<u8>,        // bincode-serialized variant body
}
```

Old peers deserialize the envelope, recognize unknown `kind_tag`, and
keep the raw `payload` in an `EventKind::Unknown { kind_tag, payload }`
they treat as a no-op. New peers route on `kind_tag` to the correct
variant. This trades one extra round-trip of bincode framing per event
for clean forward-compat — worth it. (The same shape extends to
`WireMessage` for ephemeral types.)

`apply_event` would treat `EventKind::Unknown` as a no-op
(`ApplyResult::Applied`) so the DAG link is preserved but no state
mutates.

This is a **prerequisite** for shipping `SetAgentConfig` (and any other
new variant) without a coordinated flag-day upgrade. Without it,
`SetAgentConfig` is a breaking schema change.

### Wire messages

`WireMessage` is also a bincode tagged enum
(`crates/common/src/wire.rs`). New ephemeral variants
(`StreamStart`/`Chunk`/`End`/`Cancel`, `Thinking`, `StoppedThinking`)
hit the same bincode limitation as above. Two options:

1. Apply Option B (versioned envelope) to `WireMessage` as well:
   `{ wire_tag: u32, payload: Vec<u8> }`. Old peers ignore unknown
   `wire_tag` values silently, which is acceptable for ephemeral
   traffic.
2. Flag-day upgrade.

Old clients silently drop unknown `WireMessage` envelopes today — there
is **no** "[unsupported message type]" UI fallback. This spec does not
propose adding one; ephemeral streaming chunks have no UI representation
in old clients, which is fine — they'll just see the final materialized
message rendered from the `EventKind::Message` event.

### No silent breakage

All additions in this spec are designed to be additive: new optional
fields on `Profile` and `ProfileDelta` (already `#[serde(default)]`),
new tagged variants behind a versioned-envelope `Unknown` fallback, new
ephemeral wire messages. With the envelope in place, old clients
continue to function with reduced capability rather than panicking on
deserialization.

---

## 12. Implementation Phases

### Phase 0: Forward-compat foundation (Prerequisite)

- Adopt the versioned-envelope approach (Option B in §11) for both
  `EventKind` and `WireMessage`, with `Unknown` no-op fallbacks. Without
  this, every later phase is a breaking wire change.

### Phase 1: Identity & UI (Foundation)

- Extend `Profile` and `ProfileDelta` with `peer_kind: Option<PeerKind>`
  and `data_policy: Option<DataPolicy>` (both `#[serde(default)]` on
  `Profile`; nested `Option<Option<_>>` on `ProfileDelta` per the
  established pattern). Update `apply_event(UpdateProfile)` to overlay
  the new fields.
- Wire `PeerKind` and `DataPolicy` through the profile UI (member list,
  profile card).
- Insert "Agents" section into the member list (between Infrastructure
  and Members).
- Add bot badge and "Agent" tag to message rendering.
- Add collapsible long messages: auto-collapse messages over 20 lines
  with a "Show more" toggle in `crates/web/src/components/message.rs`
  (no collapse logic exists today; this is net-new UI). Applies to all
  long messages, not only agent ones.

### Phase 2: Interaction (Core Loop)

- Create new `crates/bot/` crate shipping `willow-bot` binary (separate
  from the existing `crates/agent/` MCP crate — see §9 naming-note).
- Add `BotWorker` implementing `WorkerRole`, with `InferenceBackend` trait
  and a new `WorkerRoleInfo::Bot` variant. Update
  `WorkerRoleInfo::role_name()` for the new arm.
- Implement @mention detection and routing.
- Add agent configuration TOML loader.
- Thread-scoped context building (walks the `reply_to: Option<EventHash>`
  chain on `EventKind::Message`).

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

If the multi-event sequence (`Propose KickMember` → votes →
`RotateChannelKey` per channel) proves error-prone in practice, add an
atomic kick-with-rotation. Pick exactly one shape:

1. New `ProposedAction::KickAndRotate { peer_id, rotations: Vec<(channel_id,
   Vec<(EndpointId, Vec<u8>)>)> }` that, on threshold, removes the member
   AND rotates each listed channel key in a single state transition. This
   keeps everything inside the existing governance / vote machinery.
2. New top-level `EventKind::KickAndRotate { peer_id, rotations: ... }`
   that bypasses governance and only requires `state.is_admin(author)`.
   Shorter latency but loses the multi-admin checkpoint that the vote
   path provides.

Option 1 is preferred because it preserves the structural invariant that
member removal is governed; Option 2 is documented for completeness.
