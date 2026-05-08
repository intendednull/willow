//! Happy-path tests for the voice mutation API.
//!
//! Covers the six voice mutators in [`crate::mutations::ClientMutations`]:
//!
//! * `join_voice`         — local-state + outbound `WireMessage::VoiceJoin`
//! * `leave_voice`        — local-state + outbound `WireMessage::VoiceLeave`
//! * `toggle_mute`        — local-only flag toggle
//! * `toggle_deafen`      — local-only flag toggle
//! * `voice_peer_joined`  — listener-side mutation + `ClientEvent::VoiceJoined`
//! * `voice_peer_left`    — listener-side mutation + `ClientEvent::VoiceLeft`
//!
//! These run against the in-memory `test_client` harness — no real
//! network. `join_voice` / `leave_voice` therefore drop their wire-side
//! broadcast (no topic subscribed) but still mutate the voice
//! `StateActor`, which is exactly what the UI binds to. Wire-message
//! delivery between peers is exercised in the gossip listener tests.
//!
//! `voice_peer_joined` / `voice_peer_left` model what happens *after*
//! the listener has unpacked an inbound `WireMessage::VoiceJoin` /
//! `VoiceLeave` — building a fake wire message and routing it through
//! the listener would re-test serialisation rather than the mutator
//! itself, so we call them directly with a synthetic peer id.

use std::time::Duration;

use willow_identity::Identity;

use crate::event_receiver::EventReceiver;
use crate::events::ClientEvent;
use crate::test_client;
use crate::ClientHandle;

async fn subscribe_rx<N: willow_network::Network>(
    client: &ClientHandle<N>,
    broker: &willow_actor::Addr<willow_actor::Broker<ClientEvent>>,
) -> EventReceiver {
    EventReceiver::subscribe(broker, &client.system).await
}

// ───── join_voice ───────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn join_voice_sets_active_channel_and_inserts_self() {
    let (client, _broker) = test_client();
    let me = client.identity.endpoint_id();

    client.join_voice("voice-room").await;

    assert_eq!(
        client.active_voice_channel().await,
        Some("voice-room".to_string()),
        "join_voice must set the active channel"
    );
    let participants = client.voice_participants("voice-room").await;
    assert!(
        participants.contains(&me),
        "join_voice must insert the local peer into the channel's participant set, got {participants:?}"
    );
}

// ───── leave_voice ──────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leave_voice_clears_active_channel_and_removes_self() {
    let (client, _broker) = test_client();
    let me = client.identity.endpoint_id();

    client.join_voice("voice-room").await;
    assert_eq!(
        client.active_voice_channel().await,
        Some("voice-room".to_string()),
        "precondition: join_voice landed"
    );

    client.leave_voice().await;

    assert_eq!(
        client.active_voice_channel().await,
        None,
        "leave_voice must clear the active channel"
    );
    let participants = client.voice_participants("voice-room").await;
    assert!(
        !participants.contains(&me),
        "leave_voice must remove the local peer from the participant set, got {participants:?}"
    );
}

// ───── toggle_mute ──────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn toggle_mute_flips_state_and_returns_new_value() {
    let (client, _broker) = test_client();

    assert!(
        !client.is_voice_muted().await,
        "default voice state must be unmuted"
    );

    let after_first = client.toggle_mute().await;
    assert!(after_first, "first toggle_mute must return true");
    assert!(
        client.is_voice_muted().await,
        "is_voice_muted must reflect the toggled state"
    );

    let after_second = client.toggle_mute().await;
    assert!(!after_second, "second toggle_mute must return false");
    assert!(
        !client.is_voice_muted().await,
        "second toggle restores unmuted"
    );
}

// ───── toggle_deafen ────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn toggle_deafen_flips_state_and_returns_new_value() {
    let (client, _broker) = test_client();

    assert!(
        !client.is_voice_deafened().await,
        "default voice state must be undeafened"
    );

    let after_first = client.toggle_deafen().await;
    assert!(after_first, "first toggle_deafen must return true");
    assert!(
        client.is_voice_deafened().await,
        "is_voice_deafened must reflect the toggled state"
    );

    let after_second = client.toggle_deafen().await;
    assert!(!after_second, "second toggle_deafen must return false");
    assert!(
        !client.is_voice_deafened().await,
        "second toggle restores undeafened"
    );
}

// ───── voice_peer_joined ────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn voice_peer_joined_inserts_peer_and_emits_event() {
    let (client, broker) = test_client();
    let mut rx = subscribe_rx(&client, &broker).await;

    let other = Identity::generate().endpoint_id();
    client
        .mutations()
        .voice_peer_joined("voice-room".to_string(), other)
        .await;

    let participants = client.voice_participants("voice-room").await;
    assert!(
        participants.contains(&other),
        "voice_peer_joined must insert the remote peer into the participant set, got {participants:?}"
    );

    // Best-effort drain — surface the matching VoiceJoined event.
    let mut saw_event = false;
    for _ in 0..8 {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Some(ClientEvent::VoiceJoined {
                channel_id,
                peer_id,
            })) if channel_id == "voice-room" && peer_id == other => {
                saw_event = true;
                break;
            }
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => break,
        }
    }
    assert!(
        saw_event,
        "voice_peer_joined must publish a ClientEvent::VoiceJoined for the inserted peer"
    );
}

// ───── voice_peer_left ──────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn voice_peer_left_removes_peer_and_emits_event() {
    let (client, broker) = test_client();
    let mut rx = subscribe_rx(&client, &broker).await;

    let other = Identity::generate().endpoint_id();

    // Seed the participant set so there's something for `voice_peer_left`
    // to remove. Drain the receiver of the resulting `VoiceJoined` so it
    // doesn't shadow the event we actually care about asserting.
    client
        .mutations()
        .voice_peer_joined("voice-room".to_string(), other)
        .await;
    for _ in 0..8 {
        match tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
            Ok(Some(ClientEvent::VoiceJoined { .. })) => break,
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => break,
        }
    }

    client
        .mutations()
        .voice_peer_left("voice-room".to_string(), other)
        .await;

    let participants = client.voice_participants("voice-room").await;
    assert!(
        !participants.contains(&other),
        "voice_peer_left must remove the peer from the participant set, got {participants:?}"
    );

    let mut saw_event = false;
    for _ in 0..8 {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Some(ClientEvent::VoiceLeft {
                channel_id,
                peer_id,
            })) if channel_id == "voice-room" && peer_id == other => {
                saw_event = true;
                break;
            }
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => break,
        }
    }
    assert!(
        saw_event,
        "voice_peer_left must publish a ClientEvent::VoiceLeft for the removed peer"
    );
}
