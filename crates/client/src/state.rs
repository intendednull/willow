//! # Client State
//!
//! Pure state types for the Willow client. These types hold the client's
//! runtime state without any UI framework dependency.

use std::collections::HashMap;

use willow_crypto::ChannelKey;
use willow_identity::EndpointId;
use willow_messaging::hlc::HLC;

/// The default channel name used when no channels exist.
pub const DEFAULT_CHANNEL: &str = "general";

/// All state for a single server.
pub struct ServerContext {
    /// Server ID (UUID string).
    pub server_id: String,
    /// Server display name.
    pub name: String,
    /// Per-channel encryption keys, keyed by topic.
    pub keys: HashMap<String, ChannelKey>,
    /// Unread message counts per channel topic.
    pub unread: HashMap<String, usize>,
}

impl ServerContext {
    /// Get the gossipsub topic for a channel by name.
    pub fn topic_for_name(&self, name: &str) -> Option<String> {
        Some(crate::util::make_topic(&self.server_id, name))
    }

    /// Get the channel name for a gossipsub topic.
    pub fn name_for_topic<'a>(&self, topic: &'a str) -> Option<&'a str> {
        let prefix = format!("{}/", self.server_id);
        topic.strip_prefix(&prefix)
    }
}

/// Chat state holding current channel, peers, and the HLC clock.
pub struct ChatState {
    /// The current channel *name* (human-readable, e.g. "general").
    pub current_channel: String,
    pub peers: Vec<EndpointId>,
    pub hlc: HLC,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            current_channel: DEFAULT_CHANNEL.to_string(),
            peers: Vec::new(),
            hlc: HLC::new(),
        }
    }
}

/// Queue-note classification for a `DisplayMessage`.
///
/// Driven by `docs/specs/2026-04-19-ui-design/message-row.md` §Queue
/// notes. The projection tags each row with one of three states so the
/// message-row renderer can apply the spec's inline hints, badges,
/// opacity treatment, and delivery-flash animation.
///
/// - [`Self::None`] — nothing to show. The row renders as a normal
///   delivered message.
/// - [`Self::LateArrival`] — a peer authored this while offline and it
///   only reached us now. The row shows `sent earlier · arrived now`
///   in italic amber below the body + a `queued` badge in the meta
///   row.
/// - [`Self::Pending`] — the local user authored this while offline
///   and no peer has acked it yet. The row renders at `opacity: 0.7`
///   with `queued · will send on reconnect` below the body + a
///   `queued` badge in the meta row. On transition to `None` the row
///   fades back to full opacity and the badge flashes to `check +
///   sent` for 900 ms (see `message.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueueNote {
    /// No queue note. The default for delivered / online-authored
    /// messages.
    #[default]
    None,
    /// Peer authored offline; arrived late to the local node.
    LateArrival,
    /// Local author sent while offline; not yet acked by any peer.
    Pending,
}

/// Pinned-message attribution + time, surfaced to the pinned-panel
/// renderer for the `pinned by {name} · {when}` footer
/// (`docs/specs/2026-04-19-ui-design/reactions-pins.md` §Pinned panel
/// contents). Populated by the view projection in `views.rs` from
/// `Channel::pinned_messages[hash]`; the display name is resolved at
/// projection time via the same `resolve_display_name` ladder used by
/// the row author name (profile → ProfileState → `unknown peer`).
///
/// `Some` when the carrying `DisplayMessage.pinned == true` and the
/// channel's `PinMetadata` is present; `None` otherwise. The renderer
/// gates the footer on `pinned_metadata.is_some()` so missing pinner
/// data never leaks an empty footer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedMetadata {
    /// Human display name of the peer who pinned the message.
    pub pinner_display_name: String,
    /// Wall-clock pin time in milliseconds (from the `PinMessage`
    /// event's `timestamp_hint_ms`). The renderer formats this via
    /// the same `format_relative_time` helper used for message rows.
    pub pinned_at_ms: u64,
}

/// A message prepared for display. Computed on-the-fly from
/// event_state, never stored. Display names are resolved at
/// construction time so they're never stale.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayMessage {
    pub id: String,
    pub channel_id: String,
    pub author_peer_id: EndpointId,
    pub author_display_name: String,
    pub body: String,
    pub is_local: bool,
    pub timestamp_ms: u64,
    pub reactions: HashMap<String, Vec<String>>,
    pub edited: bool,
    pub deleted: bool,
    pub reply_to: Option<String>,
    pub reply_preview: Option<String>,
    /// Peer IDs explicitly mentioned by this message, as resolved by
    /// `willow_client::mentions::parse_mentions` at projection time.
    ///
    /// Populated once by the view projection rather than re-parsed at
    /// render time, so `mentions_me(msg, &local_peer)` is an O(1) read
    /// per frame. Never empty for a resolved `@you` or `@handle` whose
    /// resolver path produced a peer id; unresolved tokens stay in the
    /// body as plain text and are not reflected here.
    pub mentions: Vec<EndpointId>,
    /// Whether this message is currently pinned in its channel.
    ///
    /// Derived at projection time from
    /// `ServerState::channels[cid].pinned_messages`. The pin event
    /// stream is owned by `reactions-pins.md`; Phase 2a Task 6 consumes
    /// the projection to drive the row marker + badge + run-break rule
    /// per `docs/specs/2026-04-19-ui-design/message-row.md` §Pins.
    pub pinned: bool,
    /// `Some(_)` when [`Self::pinned`] is `true` and the channel's
    /// `PinMetadata` is present in the materialized state. Surfaces
    /// the pinner display name + pin time so the pinned-panel
    /// renderer can show the `pinned by {name} · {when}` footer
    /// (`docs/specs/2026-04-19-ui-design/reactions-pins.md` §Pinned
    /// panel contents, line 123). `None` for non-pinned rows.
    pub pinned_metadata: Option<PinnedMetadata>,
    /// Whether this message is part of a whisper (violet-rule placeholder).
    ///
    /// Phase 2a Task 8 reserves the layout + styling surface behind an
    /// always-false gate: the projection in
    /// `views::compute_messages_view` hard-codes `false` and the
    /// whisper-specific `EventKind` (`WhisperStart`) has not shipped
    /// yet. Once `whisper-mode.md` lands the projection will flip this
    /// via event lookup and the row renderer in `message.rs` + the
    /// run-break predicate in `chat.rs` will light up without further
    /// plumbing. Per `docs/specs/2026-04-19-ui-design/message-row.md`
    /// §Whisper hand-off, a `true` value must render the violet left
    /// rule, tinted bg, italic body, and `whisper` badge in the meta
    /// row.
    pub whisper: bool,
    /// Queue-note state for this row (see [`QueueNote`]).
    ///
    /// Populated by the view projection in
    /// `views::compute_messages_view`. Phase 2b (see
    /// `docs/plans/2026-04-21-ui-phase-2b-sync-queue.md`) closed the
    /// original `TODO(sync-queue.md)` gate: the projection now derives
    /// real `Pending` / `LateArrival` values from `QueueMeta`. The
    /// renderer is wired for the full tri-state. The grouping
    /// predicate in `chat.rs` treats any non-`None` variant as a
    /// run-break per
    /// `docs/specs/2026-04-19-ui-design/message-row.md` §Queue notes.
    pub queue_note: QueueNote,
    /// `Some(_)` when this message carries a file attachment (any
    /// `EventKind::FileMessage`). `None` for plain text. Populated by
    /// the view projection in `views::compute_messages_view` from the
    /// underlying `ChatMessage::attachment` field.
    ///
    /// The renderer in `message.rs` uses this as the discriminator
    /// between the text-body branch and the
    /// `crates/web/src/components/attachment/` rendering branch
    /// (`pick(mime, size)` → `<AttachmentImage>` /
    /// `<AttachmentFileCard>` / `<AttachmentVoiceNote>`).
    ///
    /// Spec: `docs/specs/2026-04-19-ui-design/files-inline.md`.
    pub attachment: Option<willow_state::FileAttachment>,
}

/// Maps EndpointId -> display names. Updated from profile broadcasts.
#[derive(Default, Clone)]
pub struct ProfileStore {
    pub names: HashMap<EndpointId, String>,
}

impl ProfileStore {
    /// Look up a display name for a peer, falling back to truncated ID.
    pub fn display_name(&self, peer_id: &EndpointId) -> String {
        self.names
            .get(peer_id)
            .cloned()
            .unwrap_or_else(|| crate::util::truncate_peer_id(&peer_id.to_string()))
    }
}

/// Aggregate client state bundle. Holds all runtime state for the client
/// without any UI framework dependency.
pub struct ClientState {
    /// Current channel, peers, and HLC clock.
    pub chat: ChatState,
    /// All servers, keyed by ServerId string.
    pub servers: HashMap<String, ServerContext>,
    /// Currently active server ID.
    pub active_server: Option<String>,
    /// Peer display names (global across all servers).
    pub profiles: ProfileStore,
    /// Emoji shortcode expansion registry.
    pub emoji: crate::emoji::EmojiRegistry,

    // --- Event-sourced state (willow-state) ---
    /// Event-sourced server state — the single source of truth.
    pub event_state: willow_state::ServerState,
}

impl ClientState {
    /// Create with a placeholder event state. The real owner will be set
    /// when a server is loaded or created.
    pub fn new(owner: EndpointId) -> Self {
        Self {
            chat: ChatState::default(),
            servers: HashMap::new(),
            active_server: None,
            profiles: ProfileStore::default(),
            emoji: crate::emoji::EmojiRegistry::new(),
            event_state: willow_state::ServerState::new("", "", owner),
        }
    }
}

impl ClientState {
    /// Get the active server context (if any).
    pub fn active(&self) -> Option<&ServerContext> {
        self.active_server
            .as_ref()
            .and_then(|id| self.servers.get(id))
    }

    /// Get the active server context mutably.
    pub fn active_mut(&mut self) -> Option<&mut ServerContext> {
        self.active_server
            .as_ref()
            .and_then(|id| self.servers.get_mut(id))
    }

    /// List all server IDs and names.
    pub fn server_list(&self) -> Vec<(String, String)> {
        self.servers
            .iter()
            .map(|(id, ctx)| (id.clone(), ctx.name.clone()))
            .collect()
    }

    /// Find which server owns a given topic.
    pub fn find_server_for_topic(&self, topic: &str) -> Option<&str> {
        for (id, ctx) in &self.servers {
            let prefix = format!("{}/", ctx.server_id);
            if topic.starts_with(&prefix) {
                return Some(id);
            }
        }
        None
    }
}
