//! # Willow Messaging
//!
//! Chat messages, reactions, threads, and distributed ordering for the Willow
//! P2P network.
//!
//! ## Hybrid Logical Clocks
//!
//! Because Willow has no central server, messages from different peers can
//! arrive out of order. We use a **Hybrid Logical Clock** ([`HLC`]) that
//! combines wall-clock time with a logical counter to establish a consistent
//! ordering across all peers — even when their system clocks disagree slightly.
//!
//! ## Message types
//!
//! A [`Message`] can carry different kinds of [`Content`]:
//!
//! - **Text** — plain chat messages with optional formatting
//! - **File** — a reference to a shared file (hash + metadata)
//! - **Reaction** — an emoji reaction to another message
//! - **Reply** — a threaded reply to another message
//! - **Edit** — a replacement for a previously sent message
//! - **Delete** — a tombstone marking a message as removed
//! - **System** — join / leave / channel events
//!
//! ## Message store
//!
//! The [`MessageStore`] trait abstracts over storage backends.
//! [`InMemoryStore`] provides a simple in-process implementation suitable for
//! testing and lightweight nodes.

pub mod hlc;
pub mod store;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use willow_identity::EndpointId;

use crate::hlc::HlcTimestamp;

// ───── IDs ───────────────────────────────────────────────────────────────────

/// Unique identifier for a message.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub Uuid);

impl MessageId {
    /// Generate a new random message ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a channel that messages belong to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub Uuid);

impl ChannelId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ChannelId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ChannelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ───── Content types ─────────────────────────────────────────────────────────

/// The payload inside a [`Message`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Content {
    /// A plain text chat message.
    Text {
        /// The message body (UTF-8, may contain markdown).
        body: String,
    },

    /// A reference to a shared file.
    File {
        /// Content-addressed hash of the file data.
        hash: String,
        /// Original filename.
        filename: String,
        /// MIME type (e.g. `image/png`).
        mime_type: String,
        /// File size in bytes.
        size_bytes: u64,
    },

    /// An emoji reaction to another message.
    Reaction {
        /// The message being reacted to.
        target: MessageId,
        /// The emoji (Unicode or custom shortcode).
        emoji: String,
    },

    /// A threaded reply to another message.
    Reply {
        /// The message being replied to.
        parent: MessageId,
        /// The reply body.
        body: String,
    },

    /// An edit replacing a previously sent message.
    Edit {
        /// The message being edited.
        target: MessageId,
        /// The new body text.
        new_body: String,
    },

    /// A tombstone indicating a message was deleted.
    Delete {
        /// The message being deleted.
        target: MessageId,
    },

    /// A system event (join, leave, channel rename, etc.).
    System {
        /// Human-readable description of the event.
        description: String,
    },

    /// Encrypted content. The plaintext [`Content`] was serialized and
    /// encrypted with the channel's symmetric key. Only channel members
    /// with the key can decrypt this.
    Encrypted(SealedContent),
}

/// The encrypted form of a [`Content`] value.
///
/// Produced by encrypting a serialized `Content` with ChaCha20-Poly1305.
/// The `key_epoch` field enables key rotation without breaking in-flight
/// messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedContent {
    /// ChaCha20-Poly1305 ciphertext (content bytes + 16-byte auth tag).
    pub ciphertext: Vec<u8>,
    /// 96-bit random nonce.
    pub nonce: [u8; 12],
    /// Which generation of the channel key was used.
    pub key_epoch: u32,
    /// Ratchet counter — which per-message key was used within the epoch.
    /// The receiver uses this + the epoch's channel key to derive the same
    /// message key. Defaults to 0 for backwards compat with pre-ratchet messages.
    #[serde(default)]
    pub ratchet_counter: u64,
}

// ───── Message ───────────────────────────────────────────────────────────────

/// A single message in a Willow channel.
///
/// Messages are immutable once created — edits and deletes are represented as
/// new messages that reference the original via [`Content::Edit`] and
/// [`Content::Delete`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Unique identifier for this message.
    pub id: MessageId,
    /// The channel this message belongs to.
    pub channel_id: ChannelId,
    /// Who sent this message.
    pub author: EndpointId,
    /// What the message contains.
    pub content: Content,
    /// Wall-clock time when the message was created.
    pub created_at: DateTime<Utc>,
    /// Hybrid Logical Clock timestamp for consistent distributed ordering.
    pub hlc: HlcTimestamp,
}

impl Message {
    /// Create a new text message.
    pub fn text(
        channel_id: ChannelId,
        author: EndpointId,
        body: impl Into<String>,
        hlc: &mut hlc::HLC,
    ) -> Self {
        Self {
            id: MessageId::new(),
            channel_id,
            author,
            content: Content::Text { body: body.into() },
            created_at: Utc::now(),
            hlc: hlc.now(),
        }
    }

    /// Create a reaction to another message.
    pub fn reaction(
        channel_id: ChannelId,
        author: EndpointId,
        target: MessageId,
        emoji: impl Into<String>,
        hlc: &mut hlc::HLC,
    ) -> Self {
        Self {
            id: MessageId::new(),
            channel_id,
            author,
            content: Content::Reaction {
                target,
                emoji: emoji.into(),
            },
            created_at: Utc::now(),
            hlc: hlc.now(),
        }
    }

    /// Create a reply to another message.
    pub fn reply(
        channel_id: ChannelId,
        author: EndpointId,
        parent: MessageId,
        body: impl Into<String>,
        hlc: &mut hlc::HLC,
    ) -> Self {
        Self {
            id: MessageId::new(),
            channel_id,
            author,
            content: Content::Reply {
                parent,
                body: body.into(),
            },
            created_at: Utc::now(),
            hlc: hlc.now(),
        }
    }

    /// Create a file-sharing message.
    pub fn file(
        channel_id: ChannelId,
        author: EndpointId,
        hash: impl Into<String>,
        filename: impl Into<String>,
        mime_type: impl Into<String>,
        size_bytes: u64,
        hlc: &mut hlc::HLC,
    ) -> Self {
        Self {
            id: MessageId::new(),
            channel_id,
            author,
            content: Content::File {
                hash: hash.into(),
                filename: filename.into(),
                mime_type: mime_type.into(),
                size_bytes,
            },
            created_at: Utc::now(),
            hlc: hlc.now(),
        }
    }
}

// ───── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;

    #[test]
    fn message_id_is_unique() {
        let a = MessageId::new();
        let b = MessageId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn create_text_message() {
        let mut hlc = hlc::HLC::new();
        let peer = Identity::generate().endpoint_id();
        let channel = ChannelId::new();

        let msg = Message::text(channel.clone(), peer, "hello!", &mut hlc);

        assert_eq!(msg.channel_id, channel);
        assert_eq!(msg.author, peer);
        assert!(matches!(msg.content, Content::Text { ref body } if body == "hello!"));
    }

    #[test]
    fn create_reaction() {
        let mut hlc = hlc::HLC::new();
        let peer = Identity::generate().endpoint_id();
        let channel = ChannelId::new();
        let target = MessageId::new();

        let msg = Message::reaction(channel, peer, target.clone(), "👍", &mut hlc);

        assert!(matches!(
            msg.content,
            Content::Reaction { target: ref t, ref emoji } if *t == target && emoji == "👍"
        ));
    }

    #[test]
    fn create_reply() {
        let mut hlc = hlc::HLC::new();
        let peer = Identity::generate().endpoint_id();
        let channel = ChannelId::new();
        let parent = MessageId::new();

        let msg = Message::reply(channel, peer, parent.clone(), "I agree", &mut hlc);

        assert!(matches!(
            msg.content,
            Content::Reply { parent: ref p, ref body } if *p == parent && body == "I agree"
        ));
    }

    #[test]
    fn message_serde_round_trip() {
        let mut hlc = hlc::HLC::new();
        let peer = Identity::generate().endpoint_id();
        let channel = ChannelId::new();

        let msg = Message::text(channel, peer, "serialize me", &mut hlc);
        let bytes = willow_transport::pack(&msg).unwrap();
        let decoded: Message = willow_transport::unpack(&bytes).unwrap();

        assert_eq!(decoded.id, msg.id);
        assert_eq!(decoded.content, msg.content);
        assert_eq!(decoded.author, msg.author);
    }

    #[test]
    fn create_file_message() {
        let mut hlc = hlc::HLC::new();
        let peer = Identity::generate().endpoint_id();
        let channel = ChannelId::new();

        let msg = Message::file(
            channel,
            peer,
            "abc123hash",
            "photo.jpg",
            "image/jpeg",
            1024,
            &mut hlc,
        );

        assert!(matches!(
            msg.content,
            Content::File {
                ref hash,
                ref filename,
                ref mime_type,
                size_bytes,
            } if hash == "abc123hash"
                && filename == "photo.jpg"
                && mime_type == "image/jpeg"
                && size_bytes == 1024
        ));
    }

    #[test]
    fn content_edit_serde_round_trip() {
        let target = MessageId::new();
        let content = Content::Edit {
            target: target.clone(),
            new_body: "edited text".into(),
        };
        let bytes = willow_transport::pack(&content).unwrap();
        let decoded: Content = willow_transport::unpack(&bytes).unwrap();
        assert_eq!(decoded, content);
    }

    #[test]
    fn content_delete_serde_round_trip() {
        let target = MessageId::new();
        let content = Content::Delete {
            target: target.clone(),
        };
        let bytes = willow_transport::pack(&content).unwrap();
        let decoded: Content = willow_transport::unpack(&bytes).unwrap();
        assert_eq!(decoded, content);
    }

    #[test]
    fn content_system_serde_round_trip() {
        let content = Content::System {
            description: "user joined the channel".into(),
        };
        let bytes = willow_transport::pack(&content).unwrap();
        let decoded: Content = willow_transport::unpack(&bytes).unwrap();
        assert_eq!(decoded, content);
    }

    #[test]
    fn content_encrypted_serde_round_trip() {
        let content = Content::Encrypted(SealedContent {
            ciphertext: vec![1, 2, 3, 4],
            nonce: [0u8; 12],
            key_epoch: 5,
            ratchet_counter: 42,
        });
        let bytes = willow_transport::pack(&content).unwrap();
        let decoded: Content = willow_transport::unpack(&bytes).unwrap();
        assert_eq!(decoded, content);
    }

    #[test]
    fn file_message_serde_round_trip() {
        let mut hlc = hlc::HLC::new();
        let peer = Identity::generate().endpoint_id();
        let channel = ChannelId::new();

        let msg = Message::file(
            channel,
            peer,
            "hash",
            "doc.pdf",
            "application/pdf",
            999,
            &mut hlc,
        );
        let bytes = willow_transport::pack(&msg).unwrap();
        let decoded: Message = willow_transport::unpack(&bytes).unwrap();

        assert_eq!(decoded.id, msg.id);
        assert_eq!(decoded.content, msg.content);
    }

    #[test]
    fn unicode_message_body() {
        let mut hlc = hlc::HLC::new();
        let peer = Identity::generate().endpoint_id();
        let channel = ChannelId::new();

        let body = "Hello 🌍 こんにちは 안녕하세요 مرحبا";
        let msg = Message::text(channel, peer, body, &mut hlc);

        let bytes = willow_transport::pack(&msg).unwrap();
        let decoded: Message = willow_transport::unpack(&bytes).unwrap();

        assert!(
            matches!(decoded.content, Content::Text { ref body } if body == "Hello 🌍 こんにちは 안녕하세요 مرحبا")
        );
    }

    #[test]
    fn empty_message_body() {
        let mut hlc = hlc::HLC::new();
        let peer = Identity::generate().endpoint_id();
        let channel = ChannelId::new();

        let msg = Message::text(channel, peer, "", &mut hlc);
        let bytes = willow_transport::pack(&msg).unwrap();
        let decoded: Message = willow_transport::unpack(&bytes).unwrap();

        assert!(matches!(decoded.content, Content::Text { ref body } if body.is_empty()));
    }

    #[test]
    fn message_id_display() {
        let id = MessageId::new();
        let display = format!("{id}");
        assert!(!display.is_empty());
        // UUID format: 8-4-4-4-12 hex chars
        assert_eq!(display.len(), 36);
    }

    #[test]
    fn channel_id_display() {
        let id = ChannelId::new();
        let display = format!("{id}");
        assert_eq!(display.len(), 36);
    }

    #[test]
    fn sealed_content_default_ratchet_counter() {
        let sealed = SealedContent {
            ciphertext: vec![],
            nonce: [0u8; 12],
            key_epoch: 0,
            ratchet_counter: 0,
        };
        assert_eq!(sealed.ratchet_counter, 0);
    }
}
