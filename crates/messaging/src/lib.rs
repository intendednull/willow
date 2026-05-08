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

// Re-export `DeliveryState` at crate root so `willow-client` can reach
// it without naming the `store` submodule — per Phase 2b Task 2.
pub use store::DeliveryState;

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

/// Maximum allowed length, in bytes, for a [`Content::File`] filename.
///
/// Matches POSIX `NAME_MAX` (255). Peer-supplied filenames longer than
/// this are rejected by [`Content::validate`].
pub const MAX_FILENAME_BYTES: usize = 255;

/// Maximum allowed length, in bytes, for a [`Content::File`] MIME type.
///
/// RFC 6838 §4.2 caps the registered `type/subtype` form at 127+127
/// characters; we use 255 as a conservative POSIX-aligned bound that
/// covers all realistic media types plus parameters. Peer-supplied
/// MIME types longer than this are rejected by [`Content::validate`].
pub const MAX_MIME_BYTES: usize = 255;

/// Maximum allowed length, in bytes, for [`SealedContent::ciphertext`].
///
/// `SealedContent::ciphertext` is opaque before AEAD verification, so a
/// peer can broadcast an arbitrarily large blob and force receivers to
/// allocate it during deserialise and during decryption before any
/// authentication failure surfaces. Capping the wire length closes that
/// pre-decrypt DoS surface.
///
/// 64 KiB comfortably exceeds any realistic chat payload (text bodies,
/// reactions, replies, edits, system messages) plus the 16-byte
/// ChaCha20-Poly1305 auth tag, while bounding worst-case allocations
/// from a single message at a level that's cheap to absorb. Peer-supplied
/// `Content::Encrypted` values whose `ciphertext` exceeds this bound are
/// rejected by [`Content::validate`].
pub const MAX_SEALED_CIPHERTEXT_BYTES: usize = 64 * 1024;

/// Errors returned by [`Content::validate`] / [`Message::validate`] when a
/// peer-supplied payload exceeds the structural bounds enforced by this
/// crate.
///
/// These checks guard against unbounded `String` fields in `Content::File`
/// being abused to inflate gossip payloads or memory footprint after a
/// message has already cleared signature verification. The `Content` enum
/// is decoded by `bincode` before any structural check runs, so callers
/// receiving peer-supplied `Content` MUST invoke `validate()` on the
/// decoded value before trusting any of its fields.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MessageValidationError {
    /// `Content::File::filename` exceeded [`MAX_FILENAME_BYTES`].
    #[error("filename too long: {actual} bytes (max {max})")]
    FilenameTooLong {
        /// Observed length in bytes.
        actual: usize,
        /// Configured maximum.
        max: usize,
    },

    /// `Content::File::mime_type` exceeded [`MAX_MIME_BYTES`].
    #[error("mime_type too long: {actual} bytes (max {max})")]
    MimeTypeTooLong {
        /// Observed length in bytes.
        actual: usize,
        /// Configured maximum.
        max: usize,
    },

    /// `Content::Encrypted` carried a `SealedContent::ciphertext` that
    /// exceeded [`MAX_SEALED_CIPHERTEXT_BYTES`].
    #[error("sealed ciphertext too long: {actual} bytes (max {max})")]
    SealedCiphertextTooLong {
        /// Observed length in bytes.
        actual: usize,
        /// Configured maximum.
        max: usize,
    },
}

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
        /// Original filename. Bounded to [`MAX_FILENAME_BYTES`] by
        /// [`Content::validate`]; values exceeding the cap are rejected
        /// at the inbound boundary.
        filename: String,
        /// MIME type (e.g. `image/png`). Bounded to [`MAX_MIME_BYTES`]
        /// by [`Content::validate`]; values exceeding the cap are
        /// rejected at the inbound boundary.
        mime_type: String,
        /// File size in bytes, as declared by the sender.
        ///
        /// **Attacker-declared.** UI MAY display this for the user but
        /// MUST NOT use it for preallocation (`Vec::with_capacity`,
        /// `reserve`, etc.), allocation hints, or trust decisions. The
        /// authoritative size is whatever the content-addressed `hash`
        /// resolves to once the file bytes are actually fetched.
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

impl Content {
    /// Validate structural bounds on peer-supplied content.
    ///
    /// `Content` is a wire enum decoded with `bincode` before any
    /// structural check runs, so callers handling peer-supplied values
    /// MUST invoke this method after decoding and before trusting the
    /// fields. Bounded fields:
    ///
    /// - [`Content::File::filename`] (≤ [`MAX_FILENAME_BYTES`])
    /// - [`Content::File::mime_type`] (≤ [`MAX_MIME_BYTES`])
    /// - [`SealedContent::ciphertext`] inside [`Content::Encrypted`]
    ///   (≤ [`MAX_SEALED_CIPHERTEXT_BYTES`])
    ///
    /// Other variants always validate successfully.
    ///
    /// The plaintext inside `Content::Encrypted` is opaque until the
    /// channel key is applied; callers should re-invoke `validate()`
    /// on the decrypted `Content` before trusting its inner fields.
    pub fn validate(&self) -> Result<(), MessageValidationError> {
        match self {
            Content::File {
                filename,
                mime_type,
                ..
            } => {
                if filename.len() > MAX_FILENAME_BYTES {
                    return Err(MessageValidationError::FilenameTooLong {
                        actual: filename.len(),
                        max: MAX_FILENAME_BYTES,
                    });
                }
                if mime_type.len() > MAX_MIME_BYTES {
                    return Err(MessageValidationError::MimeTypeTooLong {
                        actual: mime_type.len(),
                        max: MAX_MIME_BYTES,
                    });
                }
            }
            Content::Encrypted(sealed) => {
                if sealed.ciphertext.len() > MAX_SEALED_CIPHERTEXT_BYTES {
                    return Err(MessageValidationError::SealedCiphertextTooLong {
                        actual: sealed.ciphertext.len(),
                        max: MAX_SEALED_CIPHERTEXT_BYTES,
                    });
                }
            }
            Content::Text { .. }
            | Content::Reaction { .. }
            | Content::Reply { .. }
            | Content::Edit { .. }
            | Content::Delete { .. }
            | Content::System { .. } => {}
        }
        Ok(())
    }
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

    /// Validate structural bounds on this message's payload.
    ///
    /// Convenience wrapper around [`Content::validate`]; see that
    /// method's docs for the contract. Callers receiving a peer-supplied
    /// [`Message`] over the wire MUST invoke this before trusting any
    /// fields of the inner [`Content`].
    pub fn validate(&self) -> Result<(), MessageValidationError> {
        self.content.validate()
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
        assert_eq!(decoded.hlc, msg.hlc);
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
    fn content_reaction_serde_round_trip() {
        let target = MessageId::new();
        let content = Content::Reaction {
            target: target.clone(),
            emoji: "🎉".into(),
        };
        let bytes = willow_transport::pack(&content).unwrap();
        let decoded: Content = willow_transport::unpack(&bytes).unwrap();
        assert_eq!(decoded, content);
    }

    #[test]
    fn content_reply_serde_round_trip() {
        let parent = MessageId::new();
        let content = Content::Reply {
            parent: parent.clone(),
            body: "great point!".into(),
        };
        let bytes = willow_transport::pack(&content).unwrap();
        let decoded: Content = willow_transport::unpack(&bytes).unwrap();
        assert_eq!(decoded, content);
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

    // ── validate() bounds ───────────────────────────────────────────────────

    fn file_content(filename: &str, mime_type: &str) -> Content {
        Content::File {
            hash: "deadbeef".into(),
            filename: filename.into(),
            mime_type: mime_type.into(),
            size_bytes: 0,
        }
    }

    #[test]
    fn validate_rejects_oversized_filename() {
        let too_long = "a".repeat(MAX_FILENAME_BYTES + 1);
        let content = file_content(&too_long, "image/png");
        assert_eq!(
            content.validate(),
            Err(MessageValidationError::FilenameTooLong {
                actual: MAX_FILENAME_BYTES + 1,
                max: MAX_FILENAME_BYTES,
            })
        );
    }

    #[test]
    fn validate_rejects_oversized_mime_type() {
        let too_long = "a".repeat(MAX_MIME_BYTES + 1);
        let content = file_content("ok.txt", &too_long);
        assert_eq!(
            content.validate(),
            Err(MessageValidationError::MimeTypeTooLong {
                actual: MAX_MIME_BYTES + 1,
                max: MAX_MIME_BYTES,
            })
        );
    }

    #[test]
    fn validate_accepts_filename_at_boundary() {
        let at_cap = "a".repeat(MAX_FILENAME_BYTES);
        let mime_at_cap = "a".repeat(MAX_MIME_BYTES);
        let content = file_content(&at_cap, &mime_at_cap);
        assert!(content.validate().is_ok());
    }

    #[test]
    fn validate_accepts_empty_filename_and_mime() {
        let content = file_content("", "");
        assert!(content.validate().is_ok());
    }

    #[test]
    fn validate_is_noop_for_non_file_variants() {
        let cases = [
            Content::Text {
                body: "x".repeat(1024),
            },
            Content::Reaction {
                target: MessageId::new(),
                emoji: "👍".into(),
            },
            Content::Reply {
                parent: MessageId::new(),
                body: "ok".into(),
            },
            Content::Edit {
                target: MessageId::new(),
                new_body: "edited".into(),
            },
            Content::Delete {
                target: MessageId::new(),
            },
            Content::System {
                description: "joined".into(),
            },
            Content::Encrypted(SealedContent {
                ciphertext: vec![1, 2, 3],
                nonce: [0u8; 12],
                key_epoch: 0,
                ratchet_counter: 0,
            }),
        ];
        for content in &cases {
            assert!(content.validate().is_ok(), "expected ok for {content:?}");
        }
    }

    #[test]
    fn validate_rejects_oversized_sealed_ciphertext() {
        let content = Content::Encrypted(SealedContent {
            ciphertext: vec![0u8; MAX_SEALED_CIPHERTEXT_BYTES + 1],
            nonce: [0u8; 12],
            key_epoch: 0,
            ratchet_counter: 0,
        });
        assert_eq!(
            content.validate(),
            Err(MessageValidationError::SealedCiphertextTooLong {
                actual: MAX_SEALED_CIPHERTEXT_BYTES + 1,
                max: MAX_SEALED_CIPHERTEXT_BYTES,
            })
        );
    }

    #[test]
    fn validate_accepts_sealed_ciphertext_at_boundary() {
        let content = Content::Encrypted(SealedContent {
            ciphertext: vec![0u8; MAX_SEALED_CIPHERTEXT_BYTES],
            nonce: [0u8; 12],
            key_epoch: 0,
            ratchet_counter: 0,
        });
        assert!(content.validate().is_ok());
    }

    #[test]
    fn message_validate_delegates_to_content() {
        let mut hlc = hlc::HLC::new();
        let peer = Identity::generate().endpoint_id();
        let channel = ChannelId::new();
        let too_long = "a".repeat(MAX_FILENAME_BYTES + 1);

        let msg = Message::file(channel, peer, "hash", too_long, "image/png", 42, &mut hlc);
        assert!(matches!(
            msg.validate(),
            Err(MessageValidationError::FilenameTooLong { .. })
        ));
    }
}
