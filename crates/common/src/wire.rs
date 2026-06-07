//! Gossipsub wire message format shared between clients and workers.
//!
//! All gossipsub messages are signed envelopes wrapping a [`WireMessage`].
//! Use [`pack_wire`] to serialize and sign, [`unpack_wire`] to verify
//! and deserialize.

use serde::{Deserialize, Serialize};
use willow_identity::EndpointId;

/// All network communication uses `WireMessage` wrappers around
/// [`willow_state::Event`]s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireMessage {
    /// A single event.
    ///
    /// `Box`ed so the enum stays under clippy's `large_enum_variant`
    /// 200-byte threshold. Phase 3b's `EventKind::FileMessage` pushed
    /// the inline `Event` variant over the cap (8 fields, ~280 bytes
    /// inline), and most `WireMessage` instances on the wire are NOT
    /// `Event` — they are `SyncRequest` / `TypingIndicator` / voice
    /// signalling — so the heap indirection only costs the message-
    /// sending path and pays back on every other variant by keeping
    /// the enum compact.
    Event(Box<willow_state::Event>),
    /// Request events since a given state.
    SyncRequest {
        /// The event hash the sender's state is at — the responder
        /// returns events that the sender is missing.
        state_hash: willow_state::EventHash,
        /// If set, request events for a specific topic (channel).
        topic: Option<String>,
    },
    /// Batch of events in response to a sync request.
    SyncBatch {
        /// The events the responder is sending.
        events: Vec<willow_state::Event>,
    },
    /// Heads-based delta-sync request (additive successor to
    /// [`WireMessage::SyncRequest`]).
    ///
    /// The requester sends its current [`willow_state::HeadsSummary`] plus a
    /// [`SyncFilter`]; the responder streams the per-author tail the requester
    /// is missing as one or more [`WireMessage::SyncBatchV2`] envelopes. The
    /// legacy `SyncRequest`/`SyncBatch` variants stay defined and decodable for
    /// one release cycle so old and new peers interoperate (no
    /// `PROTOCOL_VERSION` bump). `request_id` is a `String` to match the worker
    /// path's correlation type (`WorkerWireMessage::Request { request_id:
    /// String, .. }`), so a single demux table can cover both paths.
    ///
    /// See `docs/specs/2026-04-24-negentropy-sync.md`.
    SyncRequestV2 {
        /// Correlates this request with its [`WireMessage::SyncBatchV2`]
        /// replies. UUID v4 by convention.
        request_id: String,
        /// The requester's current per-author DAG heads — the responder serves
        /// only events past these.
        heads: willow_state::HeadsSummary,
        /// Scopes the delta by server / channels / authors / kinds / time.
        filter: SyncFilter,
    },
    /// One envelope of a streamed heads-based delta response (additive
    /// successor to [`WireMessage::SyncBatch`]).
    ///
    /// `more = true` on every batch but the last; the final batch carries
    /// `more = false` and is the canonical end-of-stream marker. Each envelope
    /// is sized to stay within the gossip layer's 64 KiB `max_message_size`
    /// (see [`SYNC_ENVELOPE_BUDGET`]). `request_id` echoes the originating
    /// [`WireMessage::SyncRequestV2`].
    SyncBatchV2 {
        /// Echoes the originating [`WireMessage::SyncRequestV2::request_id`].
        request_id: String,
        /// The events in this batch, in `(author, seq)` ascending order.
        events: Vec<willow_state::Event>,
        /// `true` if more batches follow for this `request_id`; `false` on the
        /// final batch (end-of-stream).
        more: bool,
    },
    /// End-of-stored-events marker: a sync provider has finished streaming the
    /// historical portion of its store for a topic (additive — rides the same
    /// signed `MessageType::Channel` envelope as every other variant; no
    /// `PROTOCOL_VERSION` bump, no new `MessageType` slot). Subsequent events on
    /// the same topic from this provider are live, not backfill.
    ///
    /// The provider's identity is **not** in the payload: like every other
    /// `WireMessage`, the marker is signed by [`pack_wire`] and the receiver
    /// recovers the provider's [`EndpointId`] from the verified envelope signer
    /// at [`unpack_wire`] time. Carrying it explicitly would invite a relay-
    /// rewrite / MITM attack re-attributing the marker to a different trusted
    /// provider, and wastes the bytes besides.
    ///
    /// See `docs/specs/2026-04-24-history-sync-eose.md`.
    HistorySyncComplete {
        /// The `TopicId` (blake3 of the canonical topic string) this marker
        /// applies to. Carried explicitly (rather than left implicit in the
        /// gossip topic) so the marker survives relay-bridge forwarding and a
        /// single audit log can correlate markers across topics.
        topic_id: [u8; 32],
        /// Hash of the last event the provider streamed before emitting the
        /// marker, or `None` if the provider had zero stored events for this
        /// topic. The receiver uses it to detect truncation; `None` cleanly
        /// distinguishes "empty store" from "done streaming N events".
        last_event_hash: Option<willow_state::EventHash>,
        /// Per-`(topic_id, provider)` stream cursor. A random `u64` per stream
        /// (equality-based dedup needs no ordering); receivers ignore a marker
        /// whose `(provider, stream_generation)` they have already observed, so
        /// a provider that restarts and re-streams cannot confuse a client
        /// holding a stale marker.
        stream_generation: u64,
    },
    /// Ephemeral typing indicator — not stored or persisted.
    TypingIndicator {
        /// The channel name the peer is typing in.
        channel: String,
    },
    /// A peer joined a voice channel.
    VoiceJoin {
        /// The voice channel being joined.
        channel_id: String,
        /// The peer who joined.
        peer_id: EndpointId,
    },
    /// A peer left a voice channel.
    VoiceLeave {
        /// The voice channel being left.
        channel_id: String,
        /// The peer who left.
        peer_id: EndpointId,
    },
    /// A WebRTC signaling message for voice chat.
    VoiceSignal {
        /// The voice channel this signal relates to.
        channel_id: String,
        /// The intended recipient peer.
        target_peer: EndpointId,
        /// The signaling payload.
        signal: VoiceSignalPayload,
    },
    /// A peer is requesting to join via a shareable link.
    JoinRequest {
        link_id: String,
        peer_id: EndpointId,
    },
    /// The inviter's response with an encrypted invite for the requester.
    ///
    /// `link_id` echoes the originating `JoinRequest.link_id` so the requester
    /// can scope the reply to the specific outstanding join attempt and verify
    /// that the message `signer` is the inviter it sent the request to. Without
    /// this binding any peer with `target_peer` could spoof a response (see
    /// issue #309 / SEC-A-07).
    JoinResponse {
        link_id: String,
        target_peer: EndpointId,
        invite_data: String,
    },
    /// The inviter denied the join request.
    ///
    /// `link_id` echoes the originating `JoinRequest.link_id`; see
    /// [`WireMessage::JoinResponse`] for the rationale.
    JoinDenied {
        link_id: String,
        target_peer: EndpointId,
        reason: String,
    },
    /// Announce channel topics this peer is subscribed to, so the relay
    /// can dynamically subscribe and serve as bootstrap for those topics.
    TopicAnnounce {
        /// Topic name strings (e.g. "{server_id}/{channel_name}").
        topics: Vec<String>,
    },
    /// Announce the sender's display name so peers can resolve the profile
    /// without requiring a separate PROFILE_TOPIC subscription. Sent on
    /// SERVER_OPS_TOPIC so delivery is guaranteed whenever the sync path works.
    ProfileAnnounce {
        /// Human-readable display name of the sender.
        display_name: String,
    },
    /// A signed worker node message (announcement, departure, request, or response).
    ///
    /// Worker gossip messages travel on the `_willow_workers` topic.
    /// They are wrapped in this variant so they share the same Ed25519-signed
    /// envelope as all other gossipsub messages.
    Worker(crate::WorkerWireMessage),
}

/// Scopes a heads-based delta sync to a subset of a server's events.
///
/// Empty `Option`s mean "no restriction on that axis". `server_id` is always
/// required and identifies the DAG (its genesis hash, hex). See
/// `docs/specs/2026-04-24-negentropy-sync.md` § Filter semantics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncFilter {
    /// Required. Event-DAG genesis hash hex — matches the `String` server-id
    /// convention used elsewhere (e.g. `EventDag::server_id()`). Not a newtype.
    pub server_id: String,
    /// If set, narrows **chat-shaped** kinds (e.g. `Message`, `PinMessage`,
    /// `RotateChannelKey`) to these channel ids. Structural events
    /// (`GrantPermission`, `CreateChannel`, …) ignore this axis so server
    /// structure always reconciles fully.
    pub channels: Option<Vec<String>>,
    /// If set, restrict the delta to events authored by these endpoints.
    pub authors: Option<Vec<willow_identity::EndpointId>>,
    /// If set, restrict to events whose stable `EventKind` discriminant byte
    /// is in this whitelist.
    pub event_kinds: Option<Vec<u8>>,
    /// Advisory pre-filter only: a soft floor on the display-only
    /// `timestamp_hint_ms`. The authoritative cursor is the per-author `seq`
    /// in `heads`; `since_ms` only narrows the responder's scan width.
    pub since_ms: Option<u64>,
}

/// Per-envelope byte budget for streamed heads-based sync batches.
///
/// The binding cap on a `SyncBatchV2` envelope is the **gossip layer's** 64 KiB
/// `max_message_size` (`crates/network/src/iroh.rs`), not transport's 256 KB
/// [`willow_transport::MAX_DESER_SIZE`] anti-DoS cap — frames over 64 KiB are
/// dropped at the gossip layer before transport ever sees them. A responder
/// must therefore pack each batch so the *fully framed* envelope
/// (`SignedMessage` + `Envelope` + `WireMessage` tag + payload) stays ≤ 64 KiB.
///
/// The reserved slack (`64 KiB − SYNC_ENVELOPE_BUDGET`) covers that framing
/// overhead. Its size is measured empirically — not estimated — by
/// `sync_batch_v2_framing_overhead_is_small` in this module's tests, which packs
/// a real single-event `SyncBatchV2` through [`pack_wire`] and asserts the
/// overhead is comfortably under the reserved 256 bytes. Greedy packers add
/// each event's own serialized size to a running total and start a new batch
/// before the total would exceed this budget (an O(n) accumulator, never an
/// O(n²) whole-batch re-serialization per candidate).
pub const SYNC_ENVELOPE_BUDGET: usize = 64 * 1024 - 256;

/// Per-variant size cap for small signaling messages: 4 KB.
///
/// Used by tiny control-plane messages whose payload is just an EndpointId,
/// a short channel id, and maybe a short reason string. A few hundred bytes
/// is typical; 4 KB leaves headroom for future fields without inviting abuse.
const SIGNALING_CAP: usize = 4 * 1024;

/// Default per-variant size cap: 64 KB.
///
/// Lines up with the gossip layer's `max_message_size`. Used as the
/// fall-through for variants that don't have a dedicated cap.
const DEFAULT_CAP: usize = 64 * 1024;

impl WireMessage {
    /// Returns the maximum permitted serialized size, in bytes, for this
    /// variant when it appears on the wire.
    ///
    /// Per-variant caps are layered *on top of* the transport-level
    /// [`willow_transport::MAX_DESER_SIZE`] (256 KB) cap which gates every
    /// envelope before deserialization. The per-variant cap is enforced
    /// post-decode by [`unpack_wire`] as defense-in-depth: a peer who tries
    /// to ship a 200 KB `TypingIndicator` is misbehaving even though the
    /// payload fits inside the transport envelope.
    ///
    /// Caps are sized to the variant's actual payload shape:
    ///
    /// - **Body-carrying variants** (`Event`, `SyncBatch`, `Worker`,
    ///   `TopicAnnounce`): `MAX_DESER_SIZE` (256 KB). `Event`, `SyncBatch`,
    ///   and `Worker` carry user-generated message bodies, batched event
    ///   payloads, or worker sync responses, so they need the full envelope
    ///   budget. `TopicAnnounce` is also sized at the envelope budget
    ///   because the relay's per-topic limits (`MAX_TOPICS = 10_000`,
    ///   `MAX_TOPIC_LEN = 256`) already permit announces well beyond any
    ///   tighter cap, and the relay's loop does the real per-topic
    ///   validation; the per-variant cap would only fight legitimate
    ///   traffic.
    /// - **`ProfileAnnounce`**: `DEFAULT_CAP` (64 KB). Display name has no
    ///   formal length limit yet, but 64 KB is wildly more than any
    ///   reasonable display name.
    /// - **Signaling variants** (`TypingIndicator`, `VoiceJoin`,
    ///   `VoiceLeave`, `VoiceSignal`, `JoinRequest`, `JoinResponse`,
    ///   `JoinDenied`, `SyncRequest`): `SIGNALING_CAP` (4 KB). These
    ///   carry only ids, short strings, and SDP/ICE blobs — all small.
    pub fn max_size(&self) -> usize {
        match self {
            // User-generated bodies, batched payloads, and topic announces:
            // full envelope budget. (TopicAnnounce's own per-topic limits live
            // in the relay's `topic_announce_listener`, not here.)
            //
            // The heads-based sync variants sit here too: `SyncBatchV2` carries
            // a batched event payload (the responder self-limits each envelope
            // to `SYNC_ENVELOPE_BUDGET` ≈ 64 KiB), and `SyncRequestV2` carries a
            // `HeadsSummary` bounded by `MAX_AUTHORS_PER_SYNC` (~256 authors ×
            // ~72 B ≈ 18 KiB) plus a small filter — both can legitimately exceed
            // the 4 KiB signaling cap, so the per-variant cap stays at the full
            // envelope budget as defense-in-depth and the real bound is the
            // gossip layer's 64 KiB ceiling.
            WireMessage::Event(_)
            | WireMessage::SyncBatch { .. }
            | WireMessage::SyncRequestV2 { .. }
            | WireMessage::SyncBatchV2 { .. }
            | WireMessage::Worker(_)
            | WireMessage::TopicAnnounce { .. } => willow_transport::MAX_DESER_SIZE as usize,
            // Profile announce: display_name is unbounded today; allow 64 KB.
            WireMessage::ProfileAnnounce { .. } => DEFAULT_CAP,
            // Signaling / control plane: tiny payloads only.
            // `HistorySyncComplete` carries only a 32-byte topic id, an optional
            // 32-byte event hash, and a u64 generation (~80 bytes on the wire) —
            // it belongs with the small control-plane variants.
            WireMessage::SyncRequest { .. }
            | WireMessage::HistorySyncComplete { .. }
            | WireMessage::TypingIndicator { .. }
            | WireMessage::VoiceJoin { .. }
            | WireMessage::VoiceLeave { .. }
            | WireMessage::VoiceSignal { .. }
            | WireMessage::JoinRequest { .. }
            | WireMessage::JoinResponse { .. }
            | WireMessage::JoinDenied { .. } => SIGNALING_CAP,
        }
    }

    /// Short, stable name for the variant — used in tracing logs so we can
    /// see which variant tripped a per-variant cap without dumping the
    /// whole payload.
    fn variant_name(&self) -> &'static str {
        match self {
            WireMessage::Event(_) => "Event",
            WireMessage::SyncRequest { .. } => "SyncRequest",
            WireMessage::SyncBatch { .. } => "SyncBatch",
            WireMessage::SyncRequestV2 { .. } => "SyncRequestV2",
            WireMessage::SyncBatchV2 { .. } => "SyncBatchV2",
            WireMessage::HistorySyncComplete { .. } => "HistorySyncComplete",
            WireMessage::TypingIndicator { .. } => "TypingIndicator",
            WireMessage::VoiceJoin { .. } => "VoiceJoin",
            WireMessage::VoiceLeave { .. } => "VoiceLeave",
            WireMessage::VoiceSignal { .. } => "VoiceSignal",
            WireMessage::JoinRequest { .. } => "JoinRequest",
            WireMessage::JoinResponse { .. } => "JoinResponse",
            WireMessage::JoinDenied { .. } => "JoinDenied",
            WireMessage::TopicAnnounce { .. } => "TopicAnnounce",
            WireMessage::ProfileAnnounce { .. } => "ProfileAnnounce",
            WireMessage::Worker(_) => "Worker",
        }
    }
}

/// WebRTC signaling payload for voice chat negotiation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VoiceSignalPayload {
    /// SDP offer for initiating a connection.
    Offer(String),
    /// SDP answer in response to an offer.
    Answer(String),
    /// ICE candidate for connection establishment.
    IceCandidate(String),
}

/// Serialize a [`WireMessage`] into a signed envelope ready for gossipsub.
pub fn pack_wire(msg: &WireMessage, identity: &willow_identity::Identity) -> Option<Vec<u8>> {
    let envelope =
        willow_transport::pack_envelope(willow_transport::MessageType::Channel, msg).ok()?;
    willow_identity::pack(&envelope, identity).ok()
}

/// Verify and deserialize a [`WireMessage`] from a signed envelope.
///
/// Beyond signature verification and the transport-level
/// [`willow_transport::MAX_DESER_SIZE`] cap, this enforces per-variant size
/// caps via [`WireMessage::max_size`] as defense-in-depth: messages whose
/// re-serialized size exceeds the variant's cap are dropped with a
/// `tracing::warn!` and the function returns `None`.
pub fn unpack_wire(data: &[u8]) -> Option<(WireMessage, willow_identity::EndpointId)> {
    let (envelope_bytes, signer) = willow_identity::unpack::<Vec<u8>>(data).ok()?;
    let (msg, willow_transport::MessageType::Channel) =
        willow_transport::unpack_envelope::<WireMessage>(&envelope_bytes).ok()?
    else {
        return None;
    };

    // Per-variant size cap: defense-in-depth on top of the transport-level
    // MAX_DESER_SIZE cap. Computing serialized_size is O(payload) but cheap
    // — bincode just walks the structure without allocating. If the size
    // can't be computed (shouldn't happen for already-decoded messages),
    // err on the side of letting it through; the transport cap already
    // bounds the worst case.
    let cap = msg.max_size() as u64;
    if let Ok(size) = bincode::serialized_size(&msg) {
        if size > cap {
            tracing::warn!(
                variant = msg.variant_name(),
                size,
                cap,
                signer = %signer,
                "dropping wire message exceeding per-variant size cap"
            );
            return None;
        }
    }

    Some((msg, signer))
}

/// Greedily pack `events` into a sequence of `(batch, more)` pairs, each batch
/// sized so its framed [`WireMessage::SyncBatchV2`] envelope stays within
/// `budget` bytes (use [`SYNC_ENVELOPE_BUDGET`]).
///
/// The packer uses an **incremental** size accumulator — it adds each event's
/// own serialized length to a running total and starts a new batch before the
/// total would exceed `budget`. This is O(n) over the events, never the
/// O(n²) of re-serializing the whole candidate batch per event.
///
/// `more` is `true` on every batch but the last and `false` on the final
/// batch, so the last pair is the canonical end-of-stream marker. An **empty**
/// `events` input yields exactly one `(vec![], false)` pair: a fully
/// caught-up requester still receives a single zero-event terminator (the
/// responder always confirms completion).
///
/// An individual event larger than `budget` is placed alone in its own batch
/// rather than dropped — correctness (the requester needs every event) beats
/// the soft byte budget, and the gossip layer, not this helper, is the hard
/// gate. Such an event is pathological in practice given the per-`EventKind`
/// body limits.
pub fn pack_sync_batches(
    events: Vec<willow_state::Event>,
    budget: usize,
) -> Vec<(Vec<willow_state::Event>, bool)> {
    let mut batches: Vec<Vec<willow_state::Event>> = Vec::new();
    let mut current: Vec<willow_state::Event> = Vec::new();
    let mut acc: usize = 0;

    for event in events {
        let size = bincode::serialized_size(&event).unwrap_or(0) as usize;
        // Start a new batch when adding this event would overflow the budget,
        // unless the current batch is empty (a single oversized event still
        // ships, alone, rather than being dropped).
        if !current.is_empty() && acc + size > budget {
            batches.push(std::mem::take(&mut current));
            acc = 0;
        }
        acc += size;
        current.push(event);
    }
    // The final (possibly empty) batch is always pushed so a caught-up
    // requester receives a single zero-event terminator.
    batches.push(current);

    let last = batches.len() - 1;
    batches
        .into_iter()
        .enumerate()
        .map(|(i, b)| (b, i != last))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;
    use willow_state::{EventHash, EventKind};

    fn make_event(id: &Identity, kind: EventKind) -> willow_state::Event {
        willow_state::Event::new(id, 1, EventHash::ZERO, vec![], kind, 1000)
    }

    #[test]
    fn pack_unpack_event_round_trip() {
        let id = Identity::generate();
        let event = make_event(
            &id,
            EventKind::Message {
                channel_id: "general".to_string(),
                body: "hello from common".to_string(),
                reply_to: None,
            },
        );
        let event_hash = event.hash;

        let msg = WireMessage::Event(Box::new(event));
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();

        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::Event(e) => assert_eq!(e.hash, event_hash),
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn pack_unpack_sync_batch_round_trip() {
        let id = Identity::generate();
        let events = vec![make_event(
            &id,
            EventKind::CreateChannel {
                name: "ch".to_string(),
                channel_id: "cid".to_string(),
                kind: willow_state::ChannelKind::Text,
                ephemeral: None,
            },
        )];

        let msg = WireMessage::SyncBatch { events };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();

        match decoded {
            WireMessage::SyncBatch { events } => assert_eq!(events.len(), 1),
            _ => panic!("expected SyncBatch"),
        }
    }

    #[test]
    fn pack_unpack_sync_request_round_trip() {
        let id = Identity::generate();
        let msg = WireMessage::SyncRequest {
            state_hash: EventHash::from_bytes(b"test"),
            topic: Some("_willow_server_ops".to_string()),
        };

        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();

        match decoded {
            WireMessage::SyncRequest { state_hash, topic } => {
                assert_eq!(state_hash, EventHash::from_bytes(b"test"));
                assert_eq!(topic, Some("_willow_server_ops".to_string()));
            }
            _ => panic!("expected SyncRequest"),
        }
    }

    // ── Additive heads-based sync variants (negentropy-sync spec) ────────────

    fn sample_heads(id: &Identity) -> willow_state::HeadsSummary {
        use std::collections::BTreeMap;
        use willow_state::AuthorHead;
        let mut heads = BTreeMap::new();
        heads.insert(
            id.endpoint_id(),
            AuthorHead {
                seq: 7,
                hash: EventHash::from_bytes(b"head"),
            },
        );
        willow_state::HeadsSummary { heads }
    }

    fn sample_filter(id: &Identity) -> SyncFilter {
        SyncFilter {
            server_id: "srv-genesis-hash".to_string(),
            channels: Some(vec!["general".to_string(), "random".to_string()]),
            authors: Some(vec![id.endpoint_id()]),
            event_kinds: Some(vec![0u8, 3u8]),
            since_ms: Some(1_700_000_000_000),
        }
    }

    #[test]
    fn pack_unpack_sync_request_v2_round_trip() {
        let id = Identity::generate();
        let heads = sample_heads(&id);
        let filter = sample_filter(&id);
        let msg = WireMessage::SyncRequestV2 {
            request_id: "req-uuid-1".to_string(),
            heads: heads.clone(),
            filter: filter.clone(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();
        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::SyncRequestV2 {
                request_id,
                heads: h,
                filter: f,
            } => {
                assert_eq!(request_id, "req-uuid-1");
                assert_eq!(h, heads);
                assert_eq!(f, filter);
            }
            _ => panic!("expected SyncRequestV2"),
        }
    }

    #[test]
    fn pack_unpack_sync_batch_v2_round_trip() {
        let id = Identity::generate();
        let events = vec![make_event(
            &id,
            EventKind::Message {
                channel_id: "general".to_string(),
                body: "hello v2".to_string(),
                reply_to: None,
            },
        )];
        let msg = WireMessage::SyncBatchV2 {
            request_id: "req-uuid-1".to_string(),
            events,
            more: true,
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::SyncBatchV2 {
                request_id,
                events,
                more,
            } => {
                assert_eq!(request_id, "req-uuid-1");
                assert_eq!(events.len(), 1);
                assert!(more);
            }
            _ => panic!("expected SyncBatchV2"),
        }
    }

    #[test]
    fn sync_batch_v2_final_batch_signals_end_of_stream() {
        let id = Identity::generate();
        let msg = WireMessage::SyncBatchV2 {
            request_id: "done".to_string(),
            events: vec![],
            more: false,
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::SyncBatchV2 { events, more, .. } => {
                assert!(events.is_empty(), "zero-event terminator allowed");
                assert!(!more, "final batch must clear `more`");
            }
            _ => panic!("expected SyncBatchV2"),
        }
    }

    /// Pin the empirical per-envelope framing overhead so `SYNC_ENVELOPE_BUDGET`
    /// is measured, not estimated. We pack a real single-event `SyncBatchV2`
    /// through the production `pack_wire` path (SignedMessage + Envelope +
    /// WireMessage tag + payload), subtract the inner serialized event size, and
    /// assert the overhead is comfortably inside the 256-byte slack we reserve.
    #[test]
    fn sync_batch_v2_framing_overhead_is_small() {
        let id = Identity::generate();
        let event = make_event(
            &id,
            EventKind::Message {
                channel_id: "c".to_string(),
                body: "x".to_string(),
                reply_to: None,
            },
        );
        let event_size = bincode::serialized_size(&event).unwrap() as usize;

        let msg = WireMessage::SyncBatchV2 {
            request_id: "r".to_string(),
            events: vec![event],
            more: false,
        };
        let framed = pack_wire(&msg, &id).unwrap().len();

        // Everything in the framed envelope that is NOT the inner event:
        // signature (64) + pubkey (32) + their length prefixes, the Envelope
        // (version u16 + MessageType u8 + payload length prefix), the
        // WireMessage enum tag, and the SyncBatchV2 header (request_id + events
        // Vec length prefix + `more`).
        let overhead = framed - event_size;
        assert!(
            overhead < 256,
            "SyncBatchV2 framing overhead {overhead} B must stay within the \
             reserved 256-byte slack (64 KiB - SYNC_ENVELOPE_BUDGET); measured \
             event_size={event_size}, framed={framed}"
        );
        // The reserved slack must leave the budget strictly under the gossip
        // cap. These are compile-time constants, so assert them in a const
        // block (statically checked, and clippy-clean re: assertions_on_constants).
        const { assert!(SYNC_ENVELOPE_BUDGET < 64 * 1024) };
        const { assert!(SYNC_ENVELOPE_BUDGET == 64 * 1024 - 256) };
    }

    /// A greedily-packed `SyncBatchV2` whose accumulated event sizes stay under
    /// `SYNC_ENVELOPE_BUDGET` must produce a framed envelope under the 64 KiB
    /// gossip ceiling once the measured framing overhead is added.
    #[test]
    fn greedy_batch_within_budget_fits_under_gossip_cap() {
        let id = Identity::generate();
        let one = make_event(
            &id,
            EventKind::Message {
                channel_id: "general".to_string(),
                body: "a".repeat(200),
                reply_to: None,
            },
        );
        let per = bincode::serialized_size(&one).unwrap() as usize;

        // Greedily accumulate by each event's own serialized size (the O(n)
        // accumulator the responder uses), stopping before the budget.
        let mut events = Vec::new();
        let mut acc = 0usize;
        while acc + per <= SYNC_ENVELOPE_BUDGET {
            events.push(one.clone());
            acc += per;
        }
        assert!(!events.is_empty(), "at least one event should fit");

        let msg = WireMessage::SyncBatchV2 {
            request_id: "batch".to_string(),
            events,
            more: true,
        };
        let framed = pack_wire(&msg, &id).unwrap().len();
        assert!(
            framed <= 64 * 1024,
            "framed batch {framed} B must fit the 64 KiB gossip cap"
        );
    }

    // ── pack_sync_batches greedy byte-budgeted packer ───────────────────────

    #[test]
    fn pack_sync_batches_empty_yields_single_terminator() {
        let batches = pack_sync_batches(vec![], SYNC_ENVELOPE_BUDGET);
        assert_eq!(batches.len(), 1, "empty input still yields one terminator");
        assert!(batches[0].0.is_empty(), "the terminator carries no events");
        assert!(!batches[0].1, "the terminator clears `more`");
    }

    #[test]
    fn pack_sync_batches_small_input_is_one_batch() {
        let id = Identity::generate();
        let events = vec![make_event(
            &id,
            EventKind::Message {
                channel_id: "c".into(),
                body: "hi".into(),
                reply_to: None,
            },
        )];
        let batches = pack_sync_batches(events, SYNC_ENVELOPE_BUDGET);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].0.len(), 1);
        assert!(!batches[0].1, "single batch is the terminator");
    }

    #[test]
    fn pack_sync_batches_splits_on_budget_and_marks_more() {
        let id = Identity::generate();
        let one = make_event(
            &id,
            EventKind::Message {
                channel_id: "general".into(),
                body: "a".repeat(500),
                reply_to: None,
            },
        );
        let per = bincode::serialized_size(&one).unwrap() as usize;
        // A tiny budget that fits ~3 events per batch, forcing a split.
        let budget = per * 3 + per / 2;
        let total = 10;
        let events: Vec<_> = (0..total).map(|_| one.clone()).collect();

        let batches = pack_sync_batches(events, budget);
        assert!(batches.len() > 1, "must split into multiple batches");

        // Every batch but the last sets `more`; the last clears it.
        let last = batches.len() - 1;
        for (i, (_b, more)) in batches.iter().enumerate() {
            assert_eq!(*more, i != last, "more flag set on all but the last batch");
        }
        // No events lost or duplicated.
        let count: usize = batches.iter().map(|(b, _)| b.len()).sum();
        assert_eq!(count, total, "all events accounted for across batches");
        // Each non-final batch respects the budget (incremental accumulator).
        for (b, _) in &batches {
            let acc: usize = b
                .iter()
                .map(|e| bincode::serialized_size(e).unwrap() as usize)
                .sum();
            // A batch may exceed budget only when it holds a single event.
            assert!(acc <= budget || b.len() == 1, "batch within budget");
        }
    }

    #[test]
    fn pack_sync_batches_oversized_single_event_ships_alone() {
        let id = Identity::generate();
        let big = make_event(
            &id,
            EventKind::Message {
                channel_id: "general".into(),
                body: "z".repeat(2000),
                reply_to: None,
            },
        );
        let per = bincode::serialized_size(&big).unwrap() as usize;
        // Budget smaller than a single event — it must still ship, alone.
        let batches = pack_sync_batches(vec![big], per / 2);
        // One data batch + (it is the terminator since there is nothing after).
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].0.len(), 1);
        assert!(!batches[0].1);
    }

    // ── History-sync EOSE marker (history-sync-eose spec) ────────────────────

    #[test]
    fn pack_unpack_history_sync_complete_round_trip() {
        let id = Identity::generate();
        let topic_id = [7u8; 32];
        let last = EventHash::from_bytes(b"last-streamed-event");
        let msg = WireMessage::HistorySyncComplete {
            topic_id,
            last_event_hash: Some(last),
            stream_generation: 0xDEAD_BEEF_CAFE_F00D,
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();

        // The provider identity is the verified envelope signer, not a payload
        // field — derive it here exactly as a real receiver would.
        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::HistorySyncComplete {
                topic_id: t,
                last_event_hash,
                stream_generation,
            } => {
                assert_eq!(t, topic_id);
                assert_eq!(last_event_hash, Some(last));
                assert_eq!(stream_generation, 0xDEAD_BEEF_CAFE_F00D);
            }
            _ => panic!("expected HistorySyncComplete"),
        }
    }

    #[test]
    fn history_sync_complete_empty_store_has_no_last_hash() {
        // `last_event_hash: None` encodes "the provider had zero stored events
        // for this topic" — distinct from "done streaming N events" (decision 7).
        let id = Identity::generate();
        let msg = WireMessage::HistorySyncComplete {
            topic_id: [0u8; 32],
            last_event_hash: None,
            stream_generation: 1,
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::HistorySyncComplete {
                last_event_hash, ..
            } => assert!(last_event_hash.is_none(), "empty store => None"),
            _ => panic!("expected HistorySyncComplete"),
        }
    }

    #[test]
    fn history_sync_complete_serialized_size_is_tiny() {
        // The marker is ~80 bytes on the wire and imposes no new size class —
        // it serialises well under the 256 KB MAX_DESER_SIZE ceiling and even
        // under the smallest (4 KB) per-variant signaling cap.
        let id = Identity::generate();
        let msg = WireMessage::HistorySyncComplete {
            topic_id: [0xAB; 32],
            last_event_hash: Some(EventHash::from_bytes(b"h")),
            stream_generation: u64::MAX,
        };
        let framed = pack_wire(&msg, &id).unwrap().len();
        assert!(
            framed < 256 * 1024,
            "framed marker {framed} B must stay well under the 256 KB MAX_DESER_SIZE cap"
        );
        // Sanity floor on how small it actually is: signature (64) + pubkey (32)
        // + envelope/tag overhead + the fixed-size payload is comfortably under
        // 1 KB — pin a loose ceiling so a future field bloat is caught.
        assert!(
            framed < 1024,
            "framed marker {framed} B is expected to be ~a couple hundred bytes"
        );
    }

    #[test]
    fn tampered_data_fails_unpack() {
        let id = Identity::generate();
        let event = make_event(
            &id,
            EventKind::DeleteChannel {
                channel_id: "c".to_string(),
            },
        );
        let msg = WireMessage::Event(Box::new(event));

        let mut data = pack_wire(&msg, &id).unwrap();
        if let Some(b) = data.last_mut() {
            *b ^= 0xFF;
        }
        assert!(unpack_wire(&data).is_none());
    }

    #[test]
    fn pack_unpack_worker_message_round_trip() {
        use crate::{WorkerAnnouncement, WorkerRoleInfo, WorkerWireMessage};
        let id = Identity::generate();
        let announcement = WorkerAnnouncement {
            peer_id: id.endpoint_id(),
            role: WorkerRoleInfo::Replay {
                servers_loaded: 2,
                events_buffered: 100,
                max_events: 1000,
                pending_count: 0,
            },
            servers: vec!["srv-abc".to_string()],
            timestamp: 12345,
        };
        let msg = WireMessage::Worker(WorkerWireMessage::Announcement(announcement.clone()));
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();
        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::Worker(WorkerWireMessage::Announcement(a)) => {
                assert_eq!(a.peer_id, announcement.peer_id);
                assert_eq!(a.servers, announcement.servers);
                assert_eq!(a.timestamp, announcement.timestamp);
            }
            _ => panic!("expected Worker(Announcement)"),
        }
    }

    #[test]
    fn pack_unpack_typing_indicator_round_trip() {
        let id = Identity::generate();
        let msg = WireMessage::TypingIndicator {
            channel: "general".to_string(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();
        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::TypingIndicator { channel } => {
                assert_eq!(channel, "general");
            }
            _ => panic!("expected TypingIndicator"),
        }
    }

    #[test]
    fn pack_unpack_topic_announce_round_trip() {
        let id = Identity::generate();
        let topics = vec![
            "srv-abc/general".to_string(),
            "srv-abc/announcements".to_string(),
        ];
        let msg = WireMessage::TopicAnnounce {
            topics: topics.clone(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();
        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::TopicAnnounce {
                topics: decoded_topics,
            } => {
                assert_eq!(decoded_topics, topics);
            }
            _ => panic!("expected TopicAnnounce"),
        }
    }

    #[test]
    fn pack_unpack_join_request_round_trip() {
        use willow_identity::Identity;
        let id = Identity::generate();
        let peer = Identity::generate().endpoint_id();
        let msg = WireMessage::JoinRequest {
            link_id: "link-xyz".to_string(),
            peer_id: peer,
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();
        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::JoinRequest { link_id, peer_id } => {
                assert_eq!(link_id, "link-xyz");
                assert_eq!(peer_id, peer);
            }
            _ => panic!("expected JoinRequest"),
        }
    }

    #[test]
    fn pack_unpack_join_response_round_trip() {
        let id = Identity::generate();
        let target = Identity::generate().endpoint_id();
        let msg = WireMessage::JoinResponse {
            link_id: "link-xyz".to_string(),
            target_peer: target,
            invite_data: "encrypted-invite-payload".to_string(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::JoinResponse {
                link_id,
                target_peer,
                invite_data,
            } => {
                assert_eq!(link_id, "link-xyz");
                assert_eq!(target_peer, target);
                assert_eq!(invite_data, "encrypted-invite-payload");
            }
            _ => panic!("expected JoinResponse"),
        }
    }

    #[test]
    fn pack_unpack_join_denied_round_trip() {
        let id = Identity::generate();
        let target = Identity::generate().endpoint_id();
        let msg = WireMessage::JoinDenied {
            link_id: "link-xyz".to_string(),
            target_peer: target,
            reason: "invite expired".to_string(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::JoinDenied {
                link_id,
                target_peer,
                reason,
            } => {
                assert_eq!(link_id, "link-xyz");
                assert_eq!(target_peer, target);
                assert_eq!(reason, "invite expired");
            }
            _ => panic!("expected JoinDenied"),
        }
    }

    #[test]
    fn pack_unpack_voice_join_round_trip() {
        let id = Identity::generate();
        let peer = Identity::generate().endpoint_id();
        let msg = WireMessage::VoiceJoin {
            channel_id: "voice-1".to_string(),
            peer_id: peer,
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::VoiceJoin {
                channel_id,
                peer_id,
            } => {
                assert_eq!(channel_id, "voice-1");
                assert_eq!(peer_id, peer);
            }
            _ => panic!("expected VoiceJoin"),
        }
    }

    #[test]
    fn pack_unpack_voice_leave_round_trip() {
        let id = Identity::generate();
        let peer = Identity::generate().endpoint_id();
        let msg = WireMessage::VoiceLeave {
            channel_id: "voice-1".to_string(),
            peer_id: peer,
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::VoiceLeave {
                channel_id,
                peer_id,
            } => {
                assert_eq!(channel_id, "voice-1");
                assert_eq!(peer_id, peer);
            }
            _ => panic!("expected VoiceLeave"),
        }
    }

    #[test]
    fn pack_unpack_voice_signal_offer_round_trip() {
        let id = Identity::generate();
        let target = Identity::generate().endpoint_id();
        let msg = WireMessage::VoiceSignal {
            channel_id: "voice-2".to_string(),
            target_peer: target,
            signal: VoiceSignalPayload::Offer("sdp-offer-data".to_string()),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::VoiceSignal {
                channel_id,
                target_peer,
                signal,
            } => {
                assert_eq!(channel_id, "voice-2");
                assert_eq!(target_peer, target);
                match signal {
                    VoiceSignalPayload::Offer(sdp) => assert_eq!(sdp, "sdp-offer-data"),
                    _ => panic!("expected Offer signal"),
                }
            }
            _ => panic!("expected VoiceSignal"),
        }
    }

    #[test]
    fn per_variant_caps_are_sized_appropriately() {
        // Sanity: caps should be ordered signaling < profile <= body, and
        // body-carrying variants (Event, SyncBatch, Worker, TopicAnnounce)
        // all sit at the full envelope budget (MAX_DESER_SIZE).
        let signaling = WireMessage::TypingIndicator {
            channel: String::new(),
        }
        .max_size();
        let topic_announce = WireMessage::TopicAnnounce { topics: vec![] }.max_size();
        let profile = WireMessage::ProfileAnnounce {
            display_name: String::new(),
        }
        .max_size();
        let id = Identity::generate();
        let event = WireMessage::Event(Box::new(make_event(
            &id,
            EventKind::Message {
                channel_id: "c".into(),
                body: "b".into(),
                reply_to: None,
            },
        )));
        let event_cap = event.max_size();

        assert!(signaling < profile, "signaling cap < profile cap");
        assert!(profile < event_cap, "profile cap < event cap");
        assert_eq!(event_cap, willow_transport::MAX_DESER_SIZE as usize);
        assert_eq!(
            topic_announce, event_cap,
            "TopicAnnounce sits at the full envelope budget alongside other body-carrying variants"
        );
    }

    #[test]
    fn oversize_signaling_message_is_rejected() {
        // Build a TypingIndicator whose channel string blows past the
        // 4 KB signaling cap. The transport-level MAX_DESER_SIZE cap
        // (256 KB) still accepts it, so this exercises the per-variant
        // post-decode rejection path.
        let id = Identity::generate();
        let big_channel = "x".repeat(8 * 1024); // 8 KB > 4 KB signaling cap
        let msg = WireMessage::TypingIndicator {
            channel: big_channel,
        };
        let data = pack_wire(&msg, &id).unwrap();
        assert!(
            unpack_wire(&data).is_none(),
            "oversize signaling variant should be rejected by per-variant cap"
        );
    }

    #[test]
    fn empty_data_fails_unpack() {
        assert!(unpack_wire(&[]).is_none());
    }

    #[test]
    fn garbage_data_fails_unpack() {
        assert!(unpack_wire(b"not a valid message at all").is_none());
    }
}
