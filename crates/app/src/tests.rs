use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::ButtonState;
use bevy::prelude::*;
use std::sync::mpsc as std_mpsc;

use crate::network_bridge::{
    LocalIdentity, NetworkBridgeCommand, NetworkBridgeEvent, NetworkCommandSender,
};
use crate::ui::{AppView, ChannelKeyStore, ChatState, InputState, ServerState, SettingsInput};
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
    app.insert_resource(AppView::default());
    app.insert_resource(SettingsInput::default());
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

// ───── Settings / View Toggle Tests ─────────────────────────────────────────

#[test]
fn default_view_is_chat() {
    let (app, _rx) = test_app();
    let view = app.world().resource::<AppView>();
    assert_eq!(*view, AppView::Chat);
}

#[test]
fn typing_in_settings_view_updates_relay_field() {
    let (mut app, _rx) = test_app();

    // Switch to settings view.
    *app.world_mut().resource_mut::<AppView>() = AppView::Settings;

    send_key(&mut app, KeyCode::Slash, Some("/"));
    send_key(&mut app, KeyCode::KeyI, Some("i"));
    send_key(&mut app, KeyCode::KeyP, Some("p"));
    send_key(&mut app, KeyCode::Digit4, Some("4"));
    app.update();

    let settings = app.world().resource::<SettingsInput>();
    assert_eq!(settings.relay_addr, "/ip4");

    // Chat input should be untouched.
    let input = app.world().resource::<InputState>();
    assert_eq!(input.text, "");
}

#[test]
fn typing_in_chat_view_does_not_update_relay_field() {
    let (mut app, _rx) = test_app();

    // Stay in chat view (default).
    send_key(&mut app, KeyCode::KeyA, Some("a"));
    send_key(&mut app, KeyCode::KeyB, Some("b"));
    app.update();

    let settings = app.world().resource::<SettingsInput>();
    assert!(settings.relay_addr.is_empty());

    let input = app.world().resource::<InputState>();
    assert_eq!(input.text, "ab");
}

#[test]
fn enter_in_settings_does_not_send_message() {
    let (mut app, cmd_rx) = test_app();

    *app.world_mut().resource_mut::<AppView>() = AppView::Settings;

    send_key(&mut app, KeyCode::KeyX, Some("x"));
    app.update();
    send_key(&mut app, KeyCode::Enter, None);
    app.update();
    app.update();

    // No message should be sent.
    let state = app.world().resource::<ChatState>();
    assert!(state.messages.is_empty());
    assert!(cmd_rx.try_recv().is_err());
}

#[test]
fn backspace_in_settings_removes_from_relay() {
    let (mut app, _rx) = test_app();

    *app.world_mut().resource_mut::<AppView>() = AppView::Settings;
    app.world_mut().resource_mut::<SettingsInput>().relay_addr = "abc".to_string();

    send_key(&mut app, KeyCode::Backspace, None);
    app.update();

    let settings = app.world().resource::<SettingsInput>();
    assert_eq!(settings.relay_addr, "ab");
}

// ───── Storage Settings Tests ───────────────────────────────────────────────

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn network_settings_round_trip() {
    use crate::storage::{self, NetworkSettings};

    let settings = NetworkSettings {
        relay_addr: Some("/ip4/1.2.3.4/tcp/9091/ws/p2p/12D3KooWTest".to_string()),
    };

    // Serialize and deserialize.
    let bytes = willow_transport::pack(&settings).unwrap();
    let decoded: NetworkSettings = willow_transport::unpack(&bytes).unwrap();

    assert_eq!(
        decoded.relay_addr.as_deref(),
        Some("/ip4/1.2.3.4/tcp/9091/ws/p2p/12D3KooWTest")
    );
}

#[test]
fn settings_input_defaults_to_empty_relay() {
    let settings = SettingsInput::default();
    assert!(settings.relay_addr.is_empty());
}

// ───── Network Config Tests ─────────────────────────────────────────────────

#[test]
fn network_config_multiple_relays() {
    use willow_network::NetworkConfig;

    let config = NetworkConfig::default()
        .with_relay(
            "/ip4/1.1.1.1/tcp/9090/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN",
        )
        .unwrap()
        .with_relay(
            "/ip4/2.2.2.2/tcp/9091/ws/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN",
        )
        .unwrap();

    assert_eq!(config.bootstrap_peers.len(), 2);
    assert_eq!(
        config.bootstrap_peers[0].1.to_string(),
        "/ip4/1.1.1.1/tcp/9090"
    );
    assert_eq!(
        config.bootstrap_peers[1].1.to_string(),
        "/ip4/2.2.2.2/tcp/9091/ws"
    );
}

// ───── Server State Tests ───────────────────────────────────────────────────

#[test]
fn server_state_topic_mapping() {
    use crate::ui::ServerState;
    use willow_channel::{ChannelKind, Server};

    let owner = Identity::generate();
    let mut server = Server::new("Test", owner.peer_id());
    let ch_id = server.create_channel("general", ChannelKind::Text).unwrap();

    let mut state = ServerState::default();
    let topic = format!("{}/general", server.id);
    state
        .topic_map
        .insert(topic.clone(), ("general".to_string(), ch_id));
    state.server = Some(server);

    assert_eq!(state.topic_for_name("general"), Some(topic.clone()));
    assert_eq!(state.name_for_topic(&topic), Some("general"));
    assert_eq!(state.topic_for_name("nonexistent"), None);
    assert_eq!(state.name_for_topic("bogus"), None);
}

#[test]
fn server_state_channel_names_sorted() {
    use crate::ui::ServerState;
    use willow_channel::{ChannelKind, Server};

    let owner = Identity::generate();
    let mut server = Server::new("Test", owner.peer_id());
    server.create_channel("voice", ChannelKind::Voice).unwrap();
    server.create_channel("general", ChannelKind::Text).unwrap();
    server.create_channel("random", ChannelKind::Text).unwrap();

    let mut state = ServerState::default();
    state.server = Some(server);

    let names = state.channel_names();
    assert_eq!(names, vec!["general", "random", "voice"]);
}
