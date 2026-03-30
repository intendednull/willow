//! Per-topic listener tasks that stream GossipEvents and emit ClientEvents.
//!
//! Each server subscription spawns one listener via [`spawn_topic_listener`].
//! The listener calls [`TopicEvents::next()`] in a loop and routes incoming
//! wire messages to the shared state machine, emitting [`ClientEvent`]s for
//! the UI layer.

use std::sync::{Arc, RwLock};

use futures::channel::mpsc as futures_mpsc;
use willow_identity::EndpointId;
use willow_network::traits::{GossipEvent, TopicEvents};
use willow_state::EventStore as _;

use crate::events::ClientEvent;
use crate::SharedState;

/// Spawn an async task that listens for gossip events on a topic,
/// processes incoming wire messages, and emits [`ClientEvent`]s.
///
/// The task runs until the [`TopicEvents`] stream ends.
pub fn spawn_topic_listener<E: TopicEvents + 'static>(
    events: E,
    shared: Arc<RwLock<SharedState>>,
    event_tx: futures_mpsc::UnboundedSender<ClientEvent>,
) {
    #[cfg(not(target_arch = "wasm32"))]
    tokio::task::spawn_local(topic_listener_loop(events, shared, event_tx));

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(topic_listener_loop(events, shared, event_tx));
}

/// The core async loop that drains a [`TopicEvents`] stream.
async fn topic_listener_loop<E: TopicEvents>(
    mut events: E,
    shared: Arc<RwLock<SharedState>>,
    event_tx: futures_mpsc::UnboundedSender<ClientEvent>,
) {
    while let Some(Ok(gossip_event)) = events.next().await {
        match gossip_event {
            GossipEvent::Received(msg) => {
                process_received_message(&msg.content, msg.sender, &shared, &event_tx);
            }
            GossipEvent::NeighborUp(id) => {
                let mut s = shared.write().unwrap();
                if !s.state.chat.peers.contains(&id) {
                    s.state.chat.peers.push(id);
                }
                let _ = event_tx.unbounded_send(ClientEvent::PeerConnected(id));
            }
            GossipEvent::NeighborDown(id) => {
                let mut s = shared.write().unwrap();
                s.state.chat.peers.retain(|p| p != &id);
                let _ = event_tx.unbounded_send(ClientEvent::PeerDisconnected(id));
            }
        }
    }
}

/// Process a single received gossip message.
///
/// Tries profile broadcast first, then falls back to the signed
/// [`WireMessage`](crate::ops::WireMessage) envelope format.
fn process_received_message(
    data: &[u8],
    _sender: EndpointId,
    shared: &Arc<RwLock<SharedState>>,
    event_tx: &futures_mpsc::UnboundedSender<ClientEvent>,
) {
    // Try profile broadcast first (unsigned envelope, Identity message type).
    if let Ok((profile, willow_transport::MessageType::Identity)) =
        willow_transport::unpack_envelope::<willow_identity::UserProfile>(data)
    {
        let mut s = shared.write().unwrap();
        s.state
            .profiles
            .names
            .insert(profile.peer_id, profile.display_name.clone());
        let _ = event_tx.unbounded_send(ClientEvent::ProfileUpdated {
            peer_id: profile.peer_id,
            display_name: profile.display_name,
        });
        return;
    }

    // Try signed wire message format.
    let Some((wire_msg, signer)) = crate::ops::unpack_wire(data) else {
        return;
    };

    match wire_msg {
        crate::ops::WireMessage::Event(event) => {
            // Verify the event author matches the envelope signer.
            if event.author != signer {
                return;
            }

            let mut s = shared.write().unwrap();
            // Track sender as an online peer.
            if !s.state.chat.peers.contains(&signer) {
                s.state.chat.peers.push(signer);
                let _ = event_tx.unbounded_send(ClientEvent::PeerConnected(signer));
            }
            // Apply to event-sourced state.
            let result = willow_state::apply_lenient(&mut s.state.event_state, &event);
            if matches!(result, willow_state::ApplyResult::Applied) {
                s.state.event_store.append(event.clone());
                let hash = s.state.event_state.hash();
                s.state.event_store.set_latest_hash(hash);
                // Persist if configured.
                if s.config.persistence {
                    if let Some(sid) = &s.state.active_server {
                        crate::storage::save_server_state(sid, &s.state.event_state);
                    }
                }
                // Emit client events derived from the applied event.
                let mut events = Vec::new();
                crate::emit_client_events_for(&mut s, &event, &mut events);
                drop(s);
                for e in events {
                    let _ = event_tx.unbounded_send(e);
                }
            }
        }
        crate::ops::WireMessage::SyncBatch { events: batch } => {
            let mut s = shared.write().unwrap();
            if !s.state.chat.peers.contains(&signer) {
                s.state.chat.peers.push(signer);
                let _ = event_tx.unbounded_send(ClientEvent::PeerConnected(signer));
            }
            let mut sorted = batch;
            sorted.sort_by_key(|e| e.timestamp_ms);
            let count = sorted.len();
            let mut client_events = Vec::new();
            for event in &sorted {
                let result = willow_state::apply_lenient(&mut s.state.event_state, event);
                if matches!(result, willow_state::ApplyResult::Applied) {
                    s.state.event_store.append(event.clone());
                    let hash = s.state.event_state.hash();
                    s.state.event_store.set_latest_hash(hash);
                    crate::emit_client_events_for(&mut s, event, &mut client_events);
                }
            }
            if count > 0 {
                if s.config.persistence {
                    if let Some(sid) = &s.state.active_server {
                        crate::storage::save_server_state(sid, &s.state.event_state);
                    }
                }
                crate::reconcile_topic_map(&mut s.state);
                client_events.push(ClientEvent::SyncCompleted { ops_applied: count });
            }
            drop(s);
            for e in client_events {
                let _ = event_tx.unbounded_send(e);
            }
        }
        crate::ops::WireMessage::SyncRequest { state_hash, .. } => {
            // Respond with missing events.
            // NOTE: We need a TopicHandle to broadcast the response.
            // For now, just log — the response mechanism will be wired
            // when connect() is fully implemented.
            let s = shared.read().unwrap();
            let missing = s.state.event_store.events_since(&state_hash);
            if !missing.is_empty() {
                tracing::info!(
                    count = missing.len(),
                    "sync request: have events to send (not yet wired)"
                );
            }
        }
        crate::ops::WireMessage::TypingIndicator { channel } => {
            let mut s = shared.write().unwrap();
            let now = crate::util::current_time_ms();
            s.typing_peers.insert(signer, (channel, now));
            if !s.state.chat.peers.contains(&signer) {
                s.state.chat.peers.push(signer);
                let _ = event_tx.unbounded_send(ClientEvent::PeerConnected(signer));
            }
        }
        crate::ops::WireMessage::VoiceJoin {
            channel_id,
            peer_id,
        } => {
            let mut s = shared.write().unwrap();
            s.voice_participants
                .entry(channel_id.clone())
                .or_default()
                .insert(peer_id);
            let _ = event_tx.unbounded_send(ClientEvent::VoiceJoined {
                channel_id,
                peer_id,
            });
        }
        crate::ops::WireMessage::VoiceLeave {
            channel_id,
            peer_id,
        } => {
            let mut s = shared.write().unwrap();
            if let Some(p) = s.voice_participants.get_mut(&channel_id) {
                p.remove(&peer_id);
            }
            let _ = event_tx.unbounded_send(ClientEvent::VoiceLeft {
                channel_id,
                peer_id,
            });
        }
        crate::ops::WireMessage::VoiceSignal {
            channel_id,
            target_peer,
            signal,
        } => {
            let our_id = shared.read().unwrap().identity.endpoint_id();
            if target_peer == our_id {
                let _ = event_tx.unbounded_send(ClientEvent::VoiceSignal {
                    channel_id,
                    from_peer: signer,
                    signal,
                });
            }
        }
        crate::ops::WireMessage::JoinRequest { link_id, peer_id } => {
            let mut s = shared.write().unwrap();
            let valid = s
                .join_links
                .iter_mut()
                .find(|l| l.link_id == link_id && l.is_valid());
            if let Some(link) = valid {
                link.used += 1;
                // TODO: generate invite and respond via topic broadcast.
                tracing::info!(
                    %link_id,
                    %peer_id,
                    "join link request (invite generation not yet wired)"
                );
            }
        }
        crate::ops::WireMessage::JoinResponse {
            target_peer,
            invite_data,
        } => {
            let our_id = shared.read().unwrap().identity.endpoint_id();
            if target_peer == our_id {
                let _ = event_tx.unbounded_send(ClientEvent::JoinLinkResponse { invite_data });
            }
        }
        crate::ops::WireMessage::JoinDenied {
            target_peer,
            reason,
        } => {
            let our_id = shared.read().unwrap().identity.endpoint_id();
            if target_peer == our_id {
                let _ = event_tx.unbounded_send(ClientEvent::JoinLinkDenied { reason });
            }
        }
    }
}
