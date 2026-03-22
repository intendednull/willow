use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::ButtonState;
use bevy::prelude::*;
use std::sync::mpsc as std_mpsc;

use crate::network_bridge::{
    LocalIdentity, NetworkBridgeCommand, NetworkBridgeEvent, NetworkCommandSender,
};
use crate::ui::{ChannelKeyStore, ChatState, InputState, ServerState};
use willow_crypto::{generate_channel_key, seal_content};
use willow_identity::Identity;
use willow_messaging::hlc::HLC;
use willow_messaging::{ChannelId, Content, Message};
use willow_transport::{pack_envelope, unpack_envelope, MessageType};

/// Build a headless Bevy app with the UI systems but no window or GPU.
fn test_app() -> (App, std_mpsc::Receiver<NetworkBridgeCommand>) {
    let mut app = App::new();

    app.add_plugins(MinimalPlugins);

    let identity = Identity::generate();
    let (cmd_tx, cmd_rx) = std_mpsc::channel();

    app.insert_resource(LocalIdentity(identity));
    app.insert_resource(NetworkCommandSender(cmd_tx));

    app.insert_resource(ChatState::default());
    app.insert_resource(InputState::default());
    app.insert_resource(ChannelKeyStore::default());
    app.insert_resource(ServerState::default());
    app.add_message::<NetworkBridgeEvent>();
    app.add_message::<KeyboardInput>();
    app.add_systems(
        Update,
        (
            crate::ui::handle_keyboard_input,
            crate::ui::send_message,
            crate::ui::handle_network_events,
        ),
    );

    (app, cmd_rx)
}

/// Create a signed envelope from a message and identity.
fn sign_envelope(msg: &Message, identity: &Identity) -> Vec<u8> {
    let envelope_data = pack_envelope(MessageType::Chat, msg).unwrap();
    willow_identity::pack(&envelope_data, identity).unwrap()
}

fn send_key(app: &mut App, key_code: KeyCode, text: Option<&str>) {
    let text = text.map(|s| s.into());
    app.world_mut().write_message(KeyboardInput {
        key_code,
        logical_key: Key::Unidentified(NativeKey::Unidentified),
        state: ButtonState::Pressed,
        text,
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
}

// ───── Keyboard Input Tests ─────────────────────────────────────────────────

#[test]
fn typing_updates_input_buffer() {
    let (mut app, _rx) = test_app();

    send_key(&mut app, KeyCode::KeyH, Some("h"));
    send_key(&mut app, KeyCode::KeyI, Some("i"));
    app.update();

    let input = app.world().resource::<InputState>();
    assert_eq!(input.text, "hi");
}

#[test]
fn backspace_removes_last_character() {
    let (mut app, _rx) = test_app();

    send_key(&mut app, KeyCode::KeyA, Some("a"));
    send_key(&mut app, KeyCode::KeyB, Some("b"));
    app.update();

    send_key(&mut app, KeyCode::Backspace, None);
    app.update();

    let input = app.world().resource::<InputState>();
    assert_eq!(input.text, "a");
}

#[test]
fn backspace_on_empty_does_nothing() {
    let (mut app, _rx) = test_app();

    send_key(&mut app, KeyCode::Backspace, None);
    app.update();

    let input = app.world().resource::<InputState>();
    assert_eq!(input.text, "");
}

#[test]
fn enter_on_empty_does_not_send() {
    let (mut app, _rx) = test_app();

    send_key(&mut app, KeyCode::Enter, None);
    app.update();

    let state = app.world().resource::<ChatState>();
    assert!(state.messages.is_empty());
}

// ───── Message Sending Tests ────────────────────────────────────────────────

#[test]
fn enter_sends_message_and_clears_input() {
    let (mut app, cmd_rx) = test_app();

    send_key(&mut app, KeyCode::KeyH, Some("h"));
    send_key(&mut app, KeyCode::KeyI, Some("i"));
    app.update();

    send_key(&mut app, KeyCode::Enter, None);
    app.update();
    app.update();

    let input = app.world().resource::<InputState>();
    assert_eq!(input.text, "");

    let state = app.world().resource::<ChatState>();
    assert_eq!(state.messages.len(), 1);
    assert_eq!(state.messages[0].body, "hi");
    assert!(state.messages[0].is_local);

    let cmd = cmd_rx.try_recv().expect("expected a network command");
    match cmd {
        NetworkBridgeCommand::Publish { data, .. } => {
            assert!(!data.is_empty());
        }
        other => panic!("expected Publish, got {other:?}"),
    }
}

#[test]
fn sent_message_is_valid_signed_envelope() {
    let (mut app, cmd_rx) = test_app();

    send_key(&mut app, KeyCode::KeyX, Some("x"));
    app.update();
    send_key(&mut app, KeyCode::Enter, None);
    app.update();
    app.update();

    let cmd = cmd_rx.try_recv().unwrap();
    if let NetworkBridgeCommand::Publish { data, .. } = cmd {
        let (envelope_data, _signer) =
            willow_identity::unpack::<Vec<u8>>(&data).expect("valid signature");
        let (msg, msg_type) = unpack_envelope::<Message>(&envelope_data).expect("valid envelope");
        assert_eq!(msg_type, MessageType::Chat);
        assert!(matches!(msg.content, Content::Text { ref body } if body == "x"));
    } else {
        panic!("expected Publish");
    }
}

// ───── Network Event Tests ──────────────────────────────────────────────────

#[test]
fn incoming_chat_message_added_to_state() {
    let (mut app, _rx) = test_app();

    let remote = Identity::generate();
    let mut hlc = HLC::new();
    let msg = Message::text(
        ChannelId::new(),
        remote.peer_id(),
        "hello from remote",
        &mut hlc,
    );
    let data = sign_envelope(&msg, &remote);

    app.world_mut()
        .write_message(NetworkBridgeEvent::MessageReceived {
            topic: "general".into(),
            data,
            source: Some(remote.peer_id().to_string()),
        });

    app.update();

    let state = app.world().resource::<ChatState>();
    assert_eq!(state.messages.len(), 1);
    assert_eq!(state.messages[0].body, "hello from remote");
    assert!(!state.messages[0].is_local);
    assert_eq!(state.messages[0].topic, "general");
}

#[test]
fn incoming_message_updates_hlc() {
    let (mut app, _rx) = test_app();

    let remote = Identity::generate();
    let mut hlc = HLC::new();
    let msg = Message::text(ChannelId::new(), remote.peer_id(), "sync clock", &mut hlc);
    let remote_hlc = msg.hlc;
    let data = sign_envelope(&msg, &remote);

    app.world_mut()
        .write_message(NetworkBridgeEvent::MessageReceived {
            topic: "general".into(),
            data,
            source: Some(remote.peer_id().to_string()),
        });

    app.update();

    let state = app.world().resource::<ChatState>();
    assert!(state.hlc.latest() > remote_hlc);
}

#[test]
fn peer_connected_event_adds_peer() {
    let (mut app, _rx) = test_app();

    app.world_mut()
        .write_message(NetworkBridgeEvent::PeerConnected("peer-abc".into()));

    app.update();

    let state = app.world().resource::<ChatState>();
    assert_eq!(state.peers, vec!["peer-abc"]);
}

#[test]
fn peer_connected_deduplicates() {
    let (mut app, _rx) = test_app();

    app.world_mut()
        .write_message(NetworkBridgeEvent::PeerConnected("peer-abc".into()));
    app.update();

    app.world_mut()
        .write_message(NetworkBridgeEvent::PeerConnected("peer-abc".into()));
    app.update();

    let state = app.world().resource::<ChatState>();
    assert_eq!(state.peers.len(), 1);
}

#[test]
fn peer_disconnected_removes_peer() {
    let (mut app, _rx) = test_app();

    app.world_mut()
        .write_message(NetworkBridgeEvent::PeerConnected("peer-abc".into()));
    app.update();

    app.world_mut()
        .write_message(NetworkBridgeEvent::PeerDisconnected("peer-abc".into()));
    app.update();

    let state = app.world().resource::<ChatState>();
    assert!(state.peers.is_empty());
}

#[test]
fn malformed_data_is_ignored() {
    let (mut app, _rx) = test_app();

    app.world_mut()
        .write_message(NetworkBridgeEvent::MessageReceived {
            topic: "general".into(),
            data: vec![0xFF, 0xFE, 0xFD],
            source: Some("peer-xyz".into()),
        });

    app.update();

    let state = app.world().resource::<ChatState>();
    assert!(state.messages.is_empty());
}

// ───── Serialization Round-Trip ─────────────────────────────────────────────

#[test]
fn message_survives_full_round_trip() {
    let identity = Identity::generate();
    let mut hlc = HLC::new();
    let msg = Message::text(ChannelId::new(), identity.peer_id(), "round trip", &mut hlc);
    let data = pack_envelope(MessageType::Chat, &msg).unwrap();
    let (decoded, msg_type) =
        willow_transport::unpack_envelope::<Message>(&data).expect("round trip");
    assert_eq!(msg_type, MessageType::Chat);
    assert_eq!(decoded.id, msg.id);
    assert!(matches!(decoded.content, Content::Text { ref body } if body == "round trip"));
}

// ───── Encryption Tests ─────────────────────────────────────────────────────

#[test]
fn send_message_encrypts_content_when_key_present() {
    let (mut app, cmd_rx) = test_app();

    // Install a channel key for the "general" topic (the fallback topic used
    // when no ServerState is configured).
    let key = generate_channel_key();
    app.world_mut()
        .resource_mut::<ChannelKeyStore>()
        .keys
        .insert("general".into(), key);

    send_key(&mut app, KeyCode::KeyX, Some("x"));
    app.update();
    send_key(&mut app, KeyCode::Enter, None);
    app.update();
    app.update();

    let cmd = cmd_rx.try_recv().expect("expected publish");
    if let NetworkBridgeCommand::Publish { data, .. } = cmd {
        let (envelope_data, _) =
            willow_identity::unpack::<Vec<u8>>(&data).expect("valid signature");
        let (msg, _) = unpack_envelope::<Message>(&envelope_data).expect("valid envelope");
        assert!(
            matches!(msg.content, Content::Encrypted(_)),
            "content should be encrypted when key is present"
        );
    } else {
        panic!("expected Publish");
    }
}

#[test]
fn send_message_plaintext_when_no_key() {
    let (mut app, cmd_rx) = test_app();

    send_key(&mut app, KeyCode::KeyX, Some("x"));
    app.update();
    send_key(&mut app, KeyCode::Enter, None);
    app.update();
    app.update();

    let cmd = cmd_rx.try_recv().expect("expected publish");
    if let NetworkBridgeCommand::Publish { data, .. } = cmd {
        let (envelope_data, _) =
            willow_identity::unpack::<Vec<u8>>(&data).expect("valid signature");
        let (msg, _) = unpack_envelope::<Message>(&envelope_data).expect("valid envelope");
        assert!(
            matches!(msg.content, Content::Text { .. }),
            "content should be plaintext when no key"
        );
    } else {
        panic!("expected Publish");
    }
}

#[test]
fn receive_encrypted_message_decrypts() {
    let (mut app, _rx) = test_app();

    let key = generate_channel_key();
    app.world_mut()
        .resource_mut::<ChannelKeyStore>()
        .keys
        .insert("general".into(), key.clone());

    let remote = Identity::generate();
    let mut hlc = HLC::new();
    let plaintext_content = Content::Text {
        body: "encrypted hello".into(),
    };
    let sealed = seal_content(&plaintext_content, &key, 0).unwrap();
    let mut msg = Message::text(ChannelId::new(), remote.peer_id(), "placeholder", &mut hlc);
    msg.content = Content::Encrypted(sealed);
    let data = sign_envelope(&msg, &remote);

    app.world_mut()
        .write_message(NetworkBridgeEvent::MessageReceived {
            topic: "general".into(),
            data,
            source: Some(remote.peer_id().to_string()),
        });
    app.update();

    let state = app.world().resource::<ChatState>();
    assert_eq!(state.messages.len(), 1);
    assert_eq!(state.messages[0].body, "encrypted hello");
}

#[test]
fn receive_encrypted_message_wrong_key_ignored() {
    let (mut app, _rx) = test_app();

    let sender_key = generate_channel_key();
    let wrong_key = generate_channel_key();
    app.world_mut()
        .resource_mut::<ChannelKeyStore>()
        .keys
        .insert("general".into(), wrong_key);

    let remote = Identity::generate();
    let mut hlc = HLC::new();
    let sealed = seal_content(
        &Content::Text {
            body: "secret".into(),
        },
        &sender_key,
        0,
    )
    .unwrap();
    let mut msg = Message::text(ChannelId::new(), remote.peer_id(), "x", &mut hlc);
    msg.content = Content::Encrypted(sealed);
    let data = sign_envelope(&msg, &remote);

    app.world_mut()
        .write_message(NetworkBridgeEvent::MessageReceived {
            topic: "general".into(),
            data,
            source: Some(remote.peer_id().to_string()),
        });
    app.update();

    let state = app.world().resource::<ChatState>();
    assert!(
        state.messages.is_empty(),
        "wrong key should be silently ignored"
    );
}

#[test]
fn receive_unencrypted_message_still_works() {
    let (mut app, _rx) = test_app();

    let key = generate_channel_key();
    app.world_mut()
        .resource_mut::<ChannelKeyStore>()
        .keys
        .insert("general".into(), key);

    let remote = Identity::generate();
    let mut hlc = HLC::new();
    let msg = Message::text(
        ChannelId::new(),
        remote.peer_id(),
        "plaintext msg",
        &mut hlc,
    );
    let data = sign_envelope(&msg, &remote);

    app.world_mut()
        .write_message(NetworkBridgeEvent::MessageReceived {
            topic: "general".into(),
            data,
            source: Some(remote.peer_id().to_string()),
        });
    app.update();

    let state = app.world().resource::<ChatState>();
    assert_eq!(state.messages.len(), 1);
    assert_eq!(state.messages[0].body, "plaintext msg");
}

#[test]
fn unsigned_message_is_rejected() {
    let (mut app, _rx) = test_app();

    let remote = Identity::generate();
    let mut hlc = HLC::new();
    let msg = Message::text(ChannelId::new(), remote.peer_id(), "no sig", &mut hlc);
    let data = pack_envelope(MessageType::Chat, &msg).unwrap();

    app.world_mut()
        .write_message(NetworkBridgeEvent::MessageReceived {
            topic: "general".into(),
            data,
            source: Some(remote.peer_id().to_string()),
        });
    app.update();

    let state = app.world().resource::<ChatState>();
    assert!(
        state.messages.is_empty(),
        "unsigned messages should be rejected"
    );
}
