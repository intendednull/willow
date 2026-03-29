# LLM Agent UX Spec

Design specification for LLM-powered agents in Willow. Agents are first-class
participants in servers — they join channels, read messages, and reply like any
other member, but their identity, capabilities, and UI treatment are distinct.

---

## 1. Identity & Presence

### Agent Identity

An agent is a peer with a standard Ed25519 keypair. What distinguishes it from
a human member is a new `PeerKind` field carried in the profile:

```rust
enum PeerKind {
    Human,
    Agent { provider: String },  // e.g. "claude-3", "local-llama"
}
```

- `PeerKind` is broadcast via `SetProfile` events alongside `display_name`.
- Agents MUST set a display name (e.g. "Claude", "Summary Bot").
- Agents MAY set an avatar and bio describing their capabilities.
- The `provider` string is informational — it tells other peers what model
  or service backs the agent, but has no protocol-level enforcement.

### Presence Indicators

| State | Meaning | Visual |
|-------|---------|--------|
| **Online** | Agent process is running and connected | Solid bot badge |
| **Thinking** | Agent is generating a response | Pulsing/animated badge |
| **Offline** | Agent process is not connected | Greyed-out badge |

Thinking state is signaled by a new ephemeral presence message (see
[Typing / Thinking Indicators](#6-typing--thinking-indicators)).

---

## 2. UI Treatment

### Member List

Agents appear in the member list under a separate **"Agents"** section,
below human members and above infrastructure workers. Each entry shows:

- Bot badge icon (distinguishes from human members)
- Display name
- Provider tag (muted, e.g. "claude-3")
- Online/offline/thinking indicator

### Message Rendering

Agent messages render like human messages with these differences:

- **Bot badge**: Small icon next to the display name, same position as the
  verified/role badge.
- **"Agent" tag**: Subtle label after the name, similar to Discord's "BOT" tag.
- **No encryption indicator**: Agents that process cleartext for inference
  cannot claim E2E encryption. If the channel is encrypted, the agent's
  messages show a "processed in cleartext" notice (see [Privacy](#8-privacy)).
- **Streaming responses**: Long replies render incrementally as chunks arrive
  (see [Streaming](#5-streaming-responses)).
- **Collapsible**: Messages longer than 20 lines are auto-collapsed with a
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

When an agent responds to a message, it uses a `Reply` content type linking
to the invoking message. Subsequent exchanges in that thread (user replies to
the agent's response, agent replies back) form a conversation thread.

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

Agents use the existing role-based permission system. An agent's capabilities
are bounded by the permissions granted to its roles:

| Permission | Agent Use |
|------------|-----------|
| `ReadMessages` | Read channel messages for context |
| `SendMessages` | Post responses |
| `ManageMessages` | Pin messages, delete own responses |
| `AttachFiles` | Share generated files (images, docs) |

Agents MUST NOT be granted `Administrator`, `ManageRoles`, `KickMembers`,
or `BanMembers` — the UI warns if an owner attempts this.

### Trust Model

- Agents are added to a server via the standard invite flow.
- The server owner (or admin) must explicitly trust the agent peer.
- Agent trust can be revoked at any time; revocation kicks the agent and
  rotates channel keys (existing key-rotation-on-kick mechanism).
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

### Wire Format

Streaming uses a sequence of `Message` events with a shared `stream_id`:

```rust
enum Content {
    // ... existing variants ...
    StreamStart { stream_id: Uuid, reply_to: Option<MessageId> },
    StreamChunk { stream_id: Uuid, body: String },
    StreamEnd { stream_id: Uuid },
}
```

- `StreamStart` creates the message bubble in the UI (empty or with an
  initial chunk).
- `StreamChunk` appends text to the existing bubble. Chunks are ordered
  by HLC. Clients buffer and display in order.
- `StreamEnd` finalizes the message. The complete body is stored as a
  single `Text` message in the event log for replay.

### UI Behavior

- During streaming: blinking cursor at end of message, "Stop" button visible.
- **Stop**: User clicks "Stop" → sends a `StreamCancel { stream_id }` to the
  agent. The agent sends `StreamEnd` with whatever was generated so far.
- After streaming: message renders as a normal message. No visual difference
  from a non-streamed message in history.
- If the user scrolls up during streaming, the scroll position stays locked.
  A "New messages" pill appears to jump back down.

### Replay Semantics

On event replay (new peer joining, reconnecting), only the final `StreamEnd`
event is materialized. Intermediate chunks are ephemeral — they are not
persisted in the event store. This keeps the event log compact.

---

## 6. Typing / Thinking Indicators

### Protocol

A new ephemeral gossipsub message (not an event — not persisted):

```rust
enum EphemeralMessage {
    Typing { channel_id: ChannelId },            // existing
    Thinking { channel_id: ChannelId, agent: bool }, // new
    StoppedThinking { channel_id: ChannelId },       // new
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
- "Remove Agent" → kicks the agent and rotates keys.

### System Prompt

Server owners can set a per-agent system prompt that is sent to the agent
as context. This allows customizing agent behavior per server:

```
You are a helpful assistant for the Willow dev team. Be concise.
Focus on Rust and P2P networking topics. Use code blocks for code.
```

The system prompt is stored as server metadata (a new `SetAgentConfig`
event) and transmitted to the agent on join/sync.

---

## 8. Privacy & Encryption

### Cleartext Processing

LLM agents must read message content to generate responses. This conflicts
with E2E encryption.

**Approach**: Transparent disclosure.

- When an agent joins an encrypted channel, the channel header shows:
  "Messages in this channel are readable by agent: Claude"
- Agent messages show a small "processed in cleartext" indicator.
- The agent receives the channel key like any other member. Encryption
  still protects messages in transit and from non-members.

### Data Handling Disclosure

Agent profiles include a `data_policy` field:

```rust
struct AgentProfile {
    kind: PeerKind,
    data_policy: DataPolicy,
}

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

Agents are implemented as a new `WorkerRole` variant:

```rust
struct AgentRole {
    config: AgentConfig,
    inference: Box<dyn InferenceBackend>,
}

trait InferenceBackend: Send + 'static {
    fn generate(&mut self, prompt: Vec<ChatMessage>) -> Stream<String>;
    fn cancel(&mut self);
}
```

This plugs into the existing worker node runtime. The agent:

1. Subscribes to channel gossipsub topics.
2. Receives messages via the state actor's `on_event()`.
3. Filters for @mentions or auto-respond channels.
4. Builds a prompt from thread context + system prompt.
5. Calls `inference.generate()` and streams chunks back via gossipsub.

### Worker Announcement

```rust
WorkerRoleInfo::Agent {
    display_name: String,
    provider: String,
    capabilities: Vec<String>,  // ["chat", "summarize", "translate"]
    data_policy: DataPolicy,
    channels: Vec<ChannelId>,   // Channels the agent is active in
}
```

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

---

## 10. State Machine Changes

### New EventKind Variants

```rust
enum EventKind {
    // ... existing variants ...

    /// Configure an agent's behavior in this server.
    SetAgentConfig {
        peer_id: PeerId,
        system_prompt: Option<String>,
        auto_respond_channels: Vec<ChannelId>,
        rate_limit: Option<u32>,
    },
}
```

### New Content Variants

```rust
enum Content {
    // ... existing variants ...

    StreamStart { stream_id: Uuid, reply_to: Option<MessageId> },
    StreamChunk { stream_id: Uuid, body: String },
    StreamEnd   { stream_id: Uuid },
}
```

### Permission Check

`SetAgentConfig` requires `Administrator` or server ownership. Agents
themselves cannot modify their own config.

---

## 11. Migration & Compatibility

- **Wire compatibility**: New `Content` variants are unknown to old clients.
  Old clients render them as "[unsupported message type]" — the existing
  fallback behavior.
- **State compatibility**: New `EventKind` variants are skipped by old
  `apply()` implementations (returns `ApplyResult::Skipped`).
- **No breaking changes**: All additions are additive. Existing messages,
  events, and protocols are unchanged.

---

## 12. Implementation Phases

### Phase 1: Identity & UI (Foundation)

- Add `PeerKind` to profile broadcasts.
- Add "Agents" section to member list.
- Add bot badge and "Agent" tag to message rendering.
- Add collapsible long messages.

### Phase 2: Interaction (Core Loop)

- Implement @mention detection and routing.
- Add `AgentRole` worker implementation with `InferenceBackend` trait.
- Add agent configuration TOML and startup.
- Thread-scoped context building.

### Phase 3: Streaming (Polish)

- Add `StreamStart`/`StreamChunk`/`StreamEnd` content variants.
- Implement streaming message rendering in web and desktop UIs.
- Add stop button and cancel protocol.
- Thinking indicator ephemeral messages.

### Phase 4: Configuration & Privacy (Trust)

- `SetAgentConfig` event and server settings UI.
- Per-channel agent enable/auto-respond toggles.
- Rate limiting enforcement.
- Data policy disclosure and confirmation dialog.
- Encryption transparency notices.

### Phase 5: Slash Commands & Discovery

- Agent-registered command metadata in profile.
- Command palette integration.
- Regenerate action on agent messages.
