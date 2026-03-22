//! # Willow Transport
//!
//! Binary serialization and protocol framing for Willow P2P messages.
//!
//! This crate provides the lowest-level building block for Willow's networking
//! stack: converting Rust types to bytes and back. Every message that crosses
//! the network goes through this layer.
//!
//! ## Protocol Envelope
//!
//! All messages are wrapped in an [`Envelope`] that carries a protocol version
//! and a type tag so that peers can negotiate compatibility and dispatch
//! messages to the right handler.
//!
//! ## Examples
//!
//! ```
//! use willow_transport::{pack, unpack};
//!
//! let greeting = String::from("hello, willow");
//! let bytes = pack(&greeting).unwrap();
//! let decoded: String = unpack(&bytes).unwrap();
//! assert_eq!(decoded, "hello, willow");
//! ```

use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Current protocol version. Bumped whenever the wire format changes in an
/// incompatible way.
pub const PROTOCOL_VERSION: u16 = 1;

// ───── Errors ────────────────────────────────────────────────────────────────

/// Errors that can occur during serialization or deserialization.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// Failed to serialize a value to bytes.
    #[error("serialization failed: {0}")]
    Serialize(String),

    /// Failed to deserialize bytes back into a value.
    #[error("deserialization failed: {0}")]
    Deserialize(String),

    /// The remote peer is speaking a protocol version we don't understand.
    #[error("unsupported protocol version {got} (expected {expected})")]
    UnsupportedVersion { expected: u16, got: u16 },
}

// ───── Message Types ─────────────────────────────────────────────────────────

/// Identifies the kind of payload inside an [`Envelope`].
///
/// This lets the receiving peer dispatch the raw bytes to the correct
/// deserializer without having to guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum MessageType {
    /// A chat message (text, reactions, edits, etc.).
    Chat = 0,
    /// A channel or server management operation.
    Channel = 1,
    /// Peer identity or profile information.
    Identity = 2,
    /// File transfer metadata or chunk.
    File = 3,
    /// WebRTC signaling payload (offer, answer, ICE candidates).
    Signal = 4,
    /// Presence or status update.
    Presence = 5,
    /// Application-level ping / keep-alive.
    Ping = 6,
}

// ───── Envelope ──────────────────────────────────────────────────────────────

/// A versioned wrapper around an arbitrary payload.
///
/// Every byte sequence that enters or leaves the network is framed inside an
/// `Envelope` so that peers can:
///
/// 1. Reject messages from incompatible protocol versions early.
/// 2. Route the inner payload to the correct handler based on [`MessageType`].
///
/// ```
/// use willow_transport::{Envelope, MessageType, pack, unpack};
///
/// let envelope = Envelope::new(MessageType::Chat, b"hello".to_vec());
/// let bytes = pack(&envelope).unwrap();
/// let decoded: Envelope = unpack(&bytes).unwrap();
/// assert_eq!(decoded.message_type, MessageType::Chat);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    /// Protocol version that produced this envelope.
    pub version: u16,
    /// What kind of payload is inside.
    pub message_type: MessageType,
    /// The serialized inner payload.
    pub payload: Vec<u8>,
}

impl Envelope {
    /// Create a new envelope stamped with the current [`PROTOCOL_VERSION`].
    pub fn new(message_type: MessageType, payload: Vec<u8>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            message_type,
            payload,
        }
    }

    /// Validate that this envelope's version is compatible with ours.
    pub fn validate_version(&self) -> Result<(), TransportError> {
        if self.version != PROTOCOL_VERSION {
            return Err(TransportError::UnsupportedVersion {
                expected: PROTOCOL_VERSION,
                got: self.version,
            });
        }
        Ok(())
    }
}

// ───── Core serialization ────────────────────────────────────────────────────

/// Serialize any [`Serialize`]-able value to a byte vector using bincode.
///
/// # Errors
///
/// Returns [`TransportError::Serialize`] if bincode encoding fails.
pub fn pack<T: Serialize>(data: &T) -> Result<Vec<u8>, TransportError> {
    bincode::serialize(data).map_err(|e| TransportError::Serialize(e.to_string()))
}

/// Deserialize a byte slice back into a concrete type.
///
/// # Errors
///
/// Returns [`TransportError::Deserialize`] if bincode decoding fails or the
/// bytes don't match the expected type.
pub fn unpack<T: DeserializeOwned>(data: &[u8]) -> Result<T, TransportError> {
    bincode::deserialize(data).map_err(|e| TransportError::Deserialize(e.to_string()))
}

/// Wrap an inner payload inside a versioned [`Envelope`], then serialize the
/// whole thing.
///
/// This is the primary function for outbound messages — it handles both the
/// inner serialization and the framing in a single call.
///
/// # Errors
///
/// Returns [`TransportError::Serialize`] if either the payload or the envelope
/// fails to serialize.
pub fn pack_envelope<T: Serialize>(
    message_type: MessageType,
    data: &T,
) -> Result<Vec<u8>, TransportError> {
    let payload = pack(data)?;
    let envelope = Envelope::new(message_type, payload);
    pack(&envelope)
}

/// Deserialize an [`Envelope`] from raw bytes, validate its version, and
/// deserialize the inner payload.
///
/// This is the primary function for inbound messages.
///
/// # Errors
///
/// Returns an error if the bytes are malformed, the protocol version is
/// unsupported, or the inner payload can't be deserialized into `T`.
pub fn unpack_envelope<T: DeserializeOwned>(
    data: &[u8],
) -> Result<(T, MessageType), TransportError> {
    let envelope: Envelope = unpack(data)?;
    envelope.validate_version()?;
    let payload: T = unpack(&envelope.payload)?;
    Ok((payload, envelope.message_type))
}

// ───── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_and_unpack_primitive() {
        let value = 42u64;
        let bytes = pack(&value).unwrap();
        let decoded: u64 = unpack(&bytes).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn pack_and_unpack_string() {
        let value = String::from("hello, willow");
        let bytes = pack(&value).unwrap();
        let decoded: String = unpack(&bytes).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn pack_and_unpack_struct() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct TestMsg {
            id: u32,
            body: String,
        }

        let msg = TestMsg {
            id: 1,
            body: "test".into(),
        };
        let bytes = pack(&msg).unwrap();
        let decoded: TestMsg = unpack(&bytes).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn envelope_round_trip() {
        let payload = vec![1u8, 2, 3, 4];
        let env = Envelope::new(MessageType::Chat, payload.clone());

        let bytes = pack(&env).unwrap();
        let decoded: Envelope = unpack(&bytes).unwrap();

        assert_eq!(decoded.version, PROTOCOL_VERSION);
        assert_eq!(decoded.message_type, MessageType::Chat);
        assert_eq!(decoded.payload, payload);
    }

    #[test]
    fn envelope_version_validation_passes() {
        let env = Envelope::new(MessageType::Ping, vec![]);
        assert!(env.validate_version().is_ok());
    }

    #[test]
    fn envelope_version_validation_fails() {
        let env = Envelope {
            version: 999,
            message_type: MessageType::Ping,
            payload: vec![],
        };
        let err = env.validate_version().unwrap_err();
        assert!(matches!(
            err,
            TransportError::UnsupportedVersion {
                expected: PROTOCOL_VERSION,
                got: 999
            }
        ));
    }

    #[test]
    fn pack_envelope_round_trip() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Inner {
            x: i32,
        }

        let inner = Inner { x: -7 };
        let bytes = pack_envelope(MessageType::Channel, &inner).unwrap();
        let (decoded, msg_type) = unpack_envelope::<Inner>(&bytes).unwrap();

        assert_eq!(decoded, inner);
        assert_eq!(msg_type, MessageType::Channel);
    }

    #[test]
    fn deserialize_garbage_fails() {
        let garbage = vec![0xFF, 0xFE, 0xFD];
        let result = unpack::<String>(&garbage);
        assert!(result.is_err());
    }

    #[test]
    fn message_type_values_are_stable() {
        // These values are part of the wire protocol — changing them would
        // break compatibility with older peers.
        assert_eq!(MessageType::Chat as u8, 0);
        assert_eq!(MessageType::Channel as u8, 1);
        assert_eq!(MessageType::Identity as u8, 2);
        assert_eq!(MessageType::File as u8, 3);
        assert_eq!(MessageType::Signal as u8, 4);
        assert_eq!(MessageType::Presence as u8, 5);
        assert_eq!(MessageType::Ping as u8, 6);
    }
}
