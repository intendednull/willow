use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::ButtonState;
use bevy::prelude::*;
use std::sync::mpsc as std_mpsc;

use crate::network_bridge::{
    LocalIdentity, NetworkBridgeCommand, NetworkBridgeEvent, NetworkCommandSender,
};
use crate::ui::{
    AppView, ChannelKeyStore, ChannelManagement, ChatState, InputState, OpLog, ProfileStore,
    SearchFilter, ServerState, SettingsInput, UnreadCounts,
};
use willow_crypto::{generate_channel_key, seal_content};
use willow_identity::Identity;
use willow_messaging::hlc::HLC;
use willow_messaging::{ChannelId, Content, Message};
use willow_transport::{pack_envelope, MessageType};

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
    app.insert_resource(ProfileStore::default());
    app.insert_resource(UnreadCounts::default());
    app.insert_resource(OpLog::default());
    app.insert_resource(SearchFilter::default());
    app.insert_resource(ChannelManagement::default());
    app.insert_resource(ButtonInput::<KeyCode>::default());
    app.insert_resource(crate::ui::MessageDbRes(None));
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

/// Create a `StampedOp::ChatMessage` from a `Content` value.
///
/// Returns the stamped op and the op_id (which doubles as message ID).
fn make_chat_op(
    topic: &str,
    content: Content,
    author: &str,
) -> (crate::server_sync::StampedOp, String) {
    let mut hlc = HLC::new();
    let content_data = willow_transport::pack(&content).unwrap();
    let stamped = crate::server_sync::StampedOp::new(
        crate::server_sync::Op::ChatMessage {
            topic: topic.into(),
            content_data,
        },
        &mut hlc,
        author,
    );
    let op_id = stamped.op_id.clone();
    (stamped, op_id)
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
        NetworkBridgeCommand::BroadcastOp(stamped) => {
            assert!(matches!(
                stamped.op,
                crate::server_sync::Op::ChatMessage { .. }
            ));
        }
        other => panic!("expected BroadcastOp, got {other:?}"),
    }
}

#[test]
fn sent_message_is_valid_stamped_op() {
    let (mut app, cmd_rx) = test_app();

    send_key(&mut app, KeyCode::KeyX, Some("x"));
    app.update();
    send_key(&mut app, KeyCode::Enter, None);
    app.update();
    app.update();

    let cmd = cmd_rx.try_recv().unwrap();
    if let NetworkBridgeCommand::BroadcastOp(stamped) = cmd {
        assert!(!stamped.op_id.is_empty());
        assert!(!stamped.author.is_empty());
        if let crate::server_sync::Op::ChatMessage { content_data, .. } = &stamped.op {
            let content: Content = willow_transport::unpack(content_data).expect("valid content");
            assert!(matches!(content, Content::Text { ref body } if body == "x"));
        } else {
            panic!("expected ChatMessage op");
        }
    } else {
        panic!("expected BroadcastOp");
    }
}

// ───── Network Event Tests ──────────────────────────────────────────────────

#[test]
fn incoming_chat_message_added_to_state() {
    let (mut app, _rx) = test_app();

    let remote = Identity::generate();
    let author = remote.peer_id().to_string();
    let (stamped, _op_id) = make_chat_op(
        "general",
        Content::Text {
            body: "hello from remote".into(),
        },
        &author,
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped,
            from: author,
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
    let author = remote.peer_id().to_string();
    let (stamped, _) = make_chat_op(
        "general",
        Content::Text {
            body: "sync clock".into(),
        },
        &author,
    );
    let remote_hlc = stamped.hlc;

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped,
            from: author,
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

    let cmd = cmd_rx.try_recv().expect("expected BroadcastOp");
    if let NetworkBridgeCommand::BroadcastOp(stamped) = cmd {
        if let crate::server_sync::Op::ChatMessage { content_data, .. } = &stamped.op {
            let content: Content = willow_transport::unpack(content_data).unwrap();
            assert!(
                matches!(content, Content::Encrypted(_)),
                "content should be encrypted when key is present"
            );
        } else {
            panic!("expected ChatMessage op");
        }
    } else {
        panic!("expected BroadcastOp");
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

    let cmd = cmd_rx.try_recv().expect("expected BroadcastOp");
    if let NetworkBridgeCommand::BroadcastOp(stamped) = cmd {
        if let crate::server_sync::Op::ChatMessage { content_data, .. } = &stamped.op {
            let content: Content = willow_transport::unpack(content_data).unwrap();
            assert!(
                matches!(content, Content::Text { .. }),
                "content should be plaintext when no key"
            );
        } else {
            panic!("expected ChatMessage op");
        }
    } else {
        panic!("expected BroadcastOp");
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
    let author = remote.peer_id().to_string();
    let plaintext_content = Content::Text {
        body: "encrypted hello".into(),
    };
    let sealed = seal_content(&plaintext_content, &key, 0).unwrap();
    let encrypted_content = Content::Encrypted(sealed);
    let (stamped, _) = make_chat_op("general", encrypted_content, &author);

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped,
            from: author,
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
    let author = remote.peer_id().to_string();
    let sealed = seal_content(
        &Content::Text {
            body: "secret".into(),
        },
        &sender_key,
        0,
    )
    .unwrap();
    let encrypted_content = Content::Encrypted(sealed);
    let (stamped, _) = make_chat_op("general", encrypted_content, &author);

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped,
            from: author,
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
    let author = remote.peer_id().to_string();
    let (stamped, _) = make_chat_op(
        "general",
        Content::Text {
            body: "plaintext msg".into(),
        },
        &author,
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped,
            from: author,
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
fn typing_in_settings_view_updates_name_field() {
    let (mut app, _rx) = test_app();

    // Switch to settings view and clear any persisted values.
    *app.world_mut().resource_mut::<AppView>() = AppView::Settings;
    app.world_mut().resource_mut::<SettingsInput>().display_name = String::new();
    app.world_mut().resource_mut::<SettingsInput>().relay_addr = String::new();

    send_key(&mut app, KeyCode::KeyA, Some("A"));
    send_key(&mut app, KeyCode::KeyL, Some("l"));
    send_key(&mut app, KeyCode::KeyI, Some("i"));
    send_key(&mut app, KeyCode::KeyC, Some("c"));
    send_key(&mut app, KeyCode::KeyE, Some("e"));
    app.update();

    let settings = app.world().resource::<SettingsInput>();
    assert_eq!(settings.display_name, "Alice");
    assert!(settings.relay_addr.is_empty());

    // Chat input should be untouched.
    let input = app.world().resource::<InputState>();
    assert_eq!(input.text, "");
}

#[test]
fn tab_switches_settings_field() {
    use crate::ui::SettingsField;
    let (mut app, _rx) = test_app();

    *app.world_mut().resource_mut::<AppView>() = AppView::Settings;

    // Default is DisplayName.
    assert_eq!(
        app.world().resource::<SettingsInput>().focused_field,
        SettingsField::DisplayName
    );

    // Tab switches to RelayAddr.
    send_key(&mut app, KeyCode::Tab, None);
    app.update();
    assert_eq!(
        app.world().resource::<SettingsInput>().focused_field,
        SettingsField::RelayAddr
    );

    // Tab cycles through all fields.
    send_key(&mut app, KeyCode::Tab, None);
    app.update();
    assert_eq!(
        app.world().resource::<SettingsInput>().focused_field,
        SettingsField::InviteRecipient
    );

    send_key(&mut app, KeyCode::Tab, None);
    app.update();
    assert_eq!(
        app.world().resource::<SettingsInput>().focused_field,
        SettingsField::JoinCode
    );

    // Tab wraps back to DisplayName.
    send_key(&mut app, KeyCode::Tab, None);
    app.update();
    assert_eq!(
        app.world().resource::<SettingsInput>().focused_field,
        SettingsField::DisplayName
    );
}

#[test]
fn typing_in_relay_field_after_tab() {
    let (mut app, _rx) = test_app();

    *app.world_mut().resource_mut::<AppView>() = AppView::Settings;
    app.world_mut().resource_mut::<SettingsInput>().display_name = String::new();
    app.world_mut().resource_mut::<SettingsInput>().relay_addr = String::new();

    // Tab to relay field.
    send_key(&mut app, KeyCode::Tab, None);
    app.update();

    send_key(&mut app, KeyCode::Slash, Some("/"));
    send_key(&mut app, KeyCode::KeyI, Some("i"));
    send_key(&mut app, KeyCode::KeyP, Some("p"));
    send_key(&mut app, KeyCode::Digit4, Some("4"));
    app.update();

    let settings = app.world().resource::<SettingsInput>();
    assert_eq!(settings.relay_addr, "/ip4");
    assert!(settings.display_name.is_empty());
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
fn backspace_in_settings_removes_from_focused_field() {
    let (mut app, _rx) = test_app();

    *app.world_mut().resource_mut::<AppView>() = AppView::Settings;
    {
        let mut si = app.world_mut().resource_mut::<SettingsInput>();
        si.display_name = "abc".to_string();
        si.cursor = 3; // cursor at end
    }

    send_key(&mut app, KeyCode::Backspace, None);
    app.update();

    let settings = app.world().resource::<SettingsInput>();
    assert_eq!(settings.display_name, "ab");
}

// ───── Profile Tests ────────────────────────────────────────────────────────

#[test]
fn profile_store_returns_name_when_set() {
    let mut store = ProfileStore::default();
    store.names.insert("12D3KooWTest".into(), "Alice".into());

    assert_eq!(store.display_name("12D3KooWTest"), "Alice");
}

#[test]
fn profile_store_falls_back_to_truncated_id() {
    let store = ProfileStore::default();
    let name = store.display_name("12D3KooWAbCdEfGhIjKlMnOp");
    assert_eq!(name, "12D3KooWAbCd...");
}

#[test]
fn send_message_uses_profile_display_name() {
    let (mut app, _rx) = test_app();

    // Set a display name in the profile store.
    let peer_id = app
        .world()
        .resource::<LocalIdentity>()
        .0
        .peer_id()
        .to_string();
    app.world_mut()
        .resource_mut::<ProfileStore>()
        .names
        .insert(peer_id, "Bob".into());

    send_key(&mut app, KeyCode::KeyX, Some("x"));
    app.update();
    send_key(&mut app, KeyCode::Enter, None);
    app.update();
    app.update();

    let state = app.world().resource::<ChatState>();
    assert_eq!(state.messages[0].author, "Bob");
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

#[cfg(not(target_arch = "wasm32"))]
fn test_message_db() -> (crate::storage::MessageDb, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::storage::MessageDb::open_path(dir.path().join("test.db")).unwrap();
    (db, dir)
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn message_db_insert_and_load() {
    let (db, _dir) = test_message_db();

    db.insert(&crate::storage::StoredMessage {
        topic: "test-topic".into(),
        author: "alice".into(),
        body: "hello".into(),
        is_local: false,
        timestamp_ms: 1000,
    });

    db.insert(&crate::storage::StoredMessage {
        topic: "test-topic".into(),
        author: "bob".into(),
        body: "world".into(),
        is_local: true,
        timestamp_ms: 2000,
    });

    let loaded = db.load_topic("test-topic", 100);
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].author, "alice");
    assert_eq!(loaded[0].body, "hello");
    assert!(!loaded[0].is_local);
    assert_eq!(loaded[1].author, "bob");
    assert_eq!(loaded[1].body, "world");
    assert!(loaded[1].is_local);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn message_db_topics_isolated() {
    let (db, _dir) = test_message_db();

    db.insert(&crate::storage::StoredMessage {
        topic: "alpha".into(),
        author: "a".into(),
        body: "1".into(),
        is_local: false,
        timestamp_ms: 100,
    });
    db.insert(&crate::storage::StoredMessage {
        topic: "beta".into(),
        author: "b".into(),
        body: "2".into(),
        is_local: false,
        timestamp_ms: 200,
    });

    assert_eq!(db.load_topic("alpha", 100).len(), 1);
    assert_eq!(db.load_topic("beta", 100).len(), 1);
    assert_eq!(db.load_topic("gamma", 100).len(), 0);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn message_db_ordered_by_timestamp() {
    let (db, _dir) = test_message_db();

    // Insert out of order.
    db.insert(&crate::storage::StoredMessage {
        topic: "t".into(),
        author: "a".into(),
        body: "third".into(),
        is_local: false,
        timestamp_ms: 3000,
    });
    db.insert(&crate::storage::StoredMessage {
        topic: "t".into(),
        author: "a".into(),
        body: "first".into(),
        is_local: false,
        timestamp_ms: 1000,
    });
    db.insert(&crate::storage::StoredMessage {
        topic: "t".into(),
        author: "a".into(),
        body: "second".into(),
        is_local: false,
        timestamp_ms: 2000,
    });

    let loaded = db.load_topic("t", 100);
    assert_eq!(loaded[0].body, "first");
    assert_eq!(loaded[1].body, "second");
    assert_eq!(loaded[2].body, "third");
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn message_db_limit() {
    let (db, _dir) = test_message_db();

    for i in 0..10 {
        db.insert(&crate::storage::StoredMessage {
            topic: "t".into(),
            author: "a".into(),
            body: format!("msg {i}"),
            is_local: false,
            timestamp_ms: i as u64,
        });
    }

    assert_eq!(db.load_topic("t", 3).len(), 3);
    assert_eq!(db.count_topic("t"), 10);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn message_db_count_and_topics() {
    let (db, _dir) = test_message_db();

    db.insert(&crate::storage::StoredMessage {
        topic: "a".into(),
        author: "x".into(),
        body: "y".into(),
        is_local: false,
        timestamp_ms: 0,
    });
    db.insert(&crate::storage::StoredMessage {
        topic: "b".into(),
        author: "x".into(),
        body: "y".into(),
        is_local: false,
        timestamp_ms: 0,
    });

    assert_eq!(db.count_topic("a"), 1);
    assert_eq!(db.count_topic("b"), 1);
    let mut topics = db.topics();
    topics.sort();
    assert_eq!(topics, vec!["a", "b"]);
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

// ───── File Sharing Tests ───────────────────────────────────────────────────

#[test]
fn file_announcement_appears_in_chat() {
    let (mut app, _rx) = test_app();

    app.world_mut()
        .write_message(NetworkBridgeEvent::FileAnnounced {
            filename: "photo.jpg".into(),
            mime_type: "image/jpeg".into(),
            size: 51200,
            file_hash: "abc123".into(),
            from: "peer-xyz".into(),
            topic: "general".into(),
        });
    app.update();

    let state = app.world().resource::<ChatState>();
    assert_eq!(state.messages.len(), 1);
    assert!(state.messages[0].body.contains("photo.jpg"));
    assert!(state.messages[0].body.contains("50 KB"));
}

#[test]
fn file_download_event_does_not_crash() {
    let (mut app, _rx) = test_app();

    app.world_mut()
        .write_message(NetworkBridgeEvent::FileDownloaded {
            filename: "doc.pdf".into(),
            file_hash: "def456".into(),
        });
    app.update();

    // Should not crash, no message added for download complete.
    let state = app.world().resource::<ChatState>();
    assert!(state.messages.is_empty());
}

// ───── Storage Round-Trip Tests ─────────────────────────────────────────────

#[test]
fn profile_persistence_round_trip() {
    use crate::storage::LocalProfile;

    let profile = LocalProfile {
        display_name: "Alice".into(),
    };
    let bytes = willow_transport::pack(&profile).unwrap();
    let decoded: LocalProfile = willow_transport::unpack(&bytes).unwrap();
    assert_eq!(decoded.display_name, "Alice");
}

#[test]
fn stored_message_serde_round_trip() {
    use crate::storage::StoredMessage;

    let msg = StoredMessage {
        topic: "test/general".into(),
        author: "Alice".into(),
        body: "hello world".into(),
        is_local: true,
        timestamp_ms: 1234567890,
    };
    let bytes = willow_transport::pack(&msg).unwrap();
    let decoded: StoredMessage = willow_transport::unpack(&bytes).unwrap();
    assert_eq!(decoded.topic, msg.topic);
    assert_eq!(decoded.author, msg.author);
    assert_eq!(decoded.body, msg.body);
    assert_eq!(decoded.is_local, msg.is_local);
    assert_eq!(decoded.timestamp_ms, msg.timestamp_ms);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn message_db_empty_topic_returns_empty() {
    let (db, _dir) = test_message_db();
    assert!(db.load_topic("nonexistent", 100).is_empty());
    assert_eq!(db.count_topic("nonexistent"), 0);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn message_db_large_message_body() {
    let (db, _dir) = test_message_db();
    let big_body = "x".repeat(100_000);

    db.insert(&crate::storage::StoredMessage {
        topic: "t".into(),
        author: "a".into(),
        body: big_body.clone(),
        is_local: false,
        timestamp_ms: 0,
    });

    let loaded = db.load_topic("t", 100);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].body, big_body);
}

// ───── Unread Count Tests ───────────────────────────────────────────────────

#[test]
fn incoming_message_on_other_channel_increments_unread() {
    let (mut app, _rx) = test_app();

    // Current channel is "general", message arrives on "other-topic".
    let remote = Identity::generate();
    let author = remote.peer_id().to_string();
    let (stamped, _) = make_chat_op(
        "other-topic",
        Content::Text {
            body: "hello".into(),
        },
        &author,
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped,
            from: author,
        });
    app.update();

    let unread = app.world().resource::<UnreadCounts>();
    assert_eq!(unread.counts.get("other-topic").copied().unwrap_or(0), 1);
}

#[test]
fn incoming_message_on_current_channel_no_unread() {
    let (mut app, _rx) = test_app();

    // Current channel is "general" — set up topic mapping.
    app.world_mut()
        .resource_mut::<ServerState>()
        .topic_map
        .insert(
            "my-topic".into(),
            ("general".into(), willow_channel::ChannelId::new()),
        );
    app.world_mut().resource_mut::<ChatState>().current_channel = "general".into();

    let remote = Identity::generate();
    let author = remote.peer_id().to_string();
    let (stamped, _) = make_chat_op(
        "my-topic",
        Content::Text {
            body: "hello".into(),
        },
        &author,
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped,
            from: author,
        });
    app.update();

    let unread = app.world().resource::<UnreadCounts>();
    assert_eq!(unread.counts.get("my-topic").copied().unwrap_or(0), 0);
}

// ───── Timestamp Format Tests ───────────────────────────────────────────────

#[test]
fn format_timestamp_basic() {
    use crate::ui::format_timestamp;
    // 13:45 = 13*3600 + 45*60 = 49500 seconds = 49500000 ms
    assert_eq!(format_timestamp(49_500_000), "13:45");
}

#[test]
fn format_timestamp_midnight() {
    use crate::ui::format_timestamp;
    assert_eq!(format_timestamp(0), "");
    assert_eq!(format_timestamp(1000), "00:00"); // 1 second
}

#[test]
fn format_timestamp_wraps_24h() {
    use crate::ui::format_timestamp;
    // 25 hours = 25*3600*1000 = 90_000_000 ms → should show 01:00
    assert_eq!(format_timestamp(90_000_000), "01:00");
}

// ───── Theme Tests ──────────────────────────────────────────────────────────

#[test]
fn theme_colors_are_distinct() {
    // Ensure key theme colors are different from each other.
    assert_ne!(crate::theme::SIDEBAR_BG, crate::theme::MAIN_BG);
    assert_ne!(crate::theme::TEXT_PRIMARY, crate::theme::TEXT_MUTED);
    assert_ne!(crate::theme::AUTHOR_LOCAL, crate::theme::AUTHOR_REMOTE);
}

// ───── Search Tests ─────────────────────────────────────────────────────────

#[test]
fn search_filters_messages_by_body() {
    let (mut app, _rx) = test_app();

    // Add messages on "general" topic via the op pipeline.
    let remote = Identity::generate();
    let author = remote.peer_id().to_string();

    for body in ["hello world", "goodbye world", "hello again"] {
        let (stamped, _) = make_chat_op("general", Content::Text { body: body.into() }, &author);
        app.world_mut()
            .write_message(NetworkBridgeEvent::OpReceived {
                stamped_op: stamped,
                from: author.clone(),
            });
    }
    app.update();

    let state = app.world().resource::<ChatState>();
    assert_eq!(state.messages.len(), 3);
}

// ───── Escape Key Tests ─────────────────────────────────────────────────────

#[test]
fn escape_exits_settings_view() {
    let (mut app, _rx) = test_app();

    *app.world_mut().resource_mut::<AppView>() = AppView::Settings;

    send_key(&mut app, KeyCode::Escape, None);
    app.update();

    assert_eq!(*app.world().resource::<AppView>(), AppView::Chat);
}

// ───── Message Pruning Tests ────────────────────────────────────────────────

#[test]
fn prune_if_needed_caps_messages() {
    use crate::ui::ChatMessage;

    let mut state = ChatState::default();
    for i in 0..1100 {
        state.messages.push(ChatMessage::new(
            "t".into(),
            "a".into(),
            format!("msg {i}"),
            false,
            i as u64,
        ));
    }
    assert_eq!(state.messages.len(), 1100);
    state.prune_if_needed();
    assert_eq!(state.messages.len(), 1000);
    // Oldest messages were removed.
    assert_eq!(state.messages[0].body, "msg 100");
}

// ───── Profile Received Tests ───────────────────────────────────────────────

#[test]
fn profile_received_updates_store() {
    let (mut app, _rx) = test_app();

    app.world_mut()
        .write_message(NetworkBridgeEvent::ProfileReceived {
            peer_id: "12D3KooWTestPeer".into(),
            display_name: "Alice".into(),
        });
    app.update();

    let profiles = app.world().resource::<ProfileStore>();
    assert_eq!(profiles.display_name("12D3KooWTestPeer"), "Alice");
}

// ───── Identity PeerId Extraction Tests ─────────────────────────────────────

#[test]
fn ed25519_public_from_peer_id_valid() {
    let id = Identity::generate();
    let peer_str = id.peer_id().to_string();
    let pub_bytes = willow_identity::ed25519_public_from_peer_id(&peer_str);
    assert!(pub_bytes.is_some());
    assert_eq!(pub_bytes.unwrap().len(), 32);
}

#[test]
fn ed25519_public_from_peer_id_invalid() {
    assert!(willow_identity::ed25519_public_from_peer_id("not-a-peer-id").is_none());
    assert!(willow_identity::ed25519_public_from_peer_id("").is_none());
}

// ───── Invite Tamper Tests ──────────────────────────────────────────────────

#[test]
fn tampered_invite_code_fails() {
    use willow_channel::ChannelKind;

    let owner = Identity::generate();
    let recipient = Identity::generate();

    let mut server = willow_channel::Server::new("Test", owner.peer_id());
    let ch_id = server.create_channel("general", ChannelKind::Text).unwrap();

    let mut keys = std::collections::HashMap::new();
    let mut topic_map = std::collections::HashMap::new();
    let topic = format!("{}/general", server.id);
    if let Some(key) = server.channel_key(&ch_id) {
        keys.insert(topic.clone(), key.clone());
    }
    topic_map.insert(topic, ("general".into(), ch_id));

    let ed_kp = recipient.keypair().clone().try_into_ed25519().unwrap();
    let full = ed_kp.to_bytes();
    let mut pub_bytes = [0u8; 32];
    pub_bytes.copy_from_slice(&full[32..]);

    let mut code = crate::invite::generate_invite(&server, &keys, &topic_map, &pub_bytes).unwrap();

    // Tamper with the code.
    if let Some(byte) = unsafe { code.as_bytes_mut().last_mut() } {
        *byte = if *byte == b'A' { b'B' } else { b'A' };
    }

    // Should fail to decrypt.
    assert!(crate::invite::accept_invite(&code, &recipient).is_none());
}

// ───── Full Invite Flow Tests ───────────────────────────────────────────────

/// End-to-end test: owner creates server → generates invite → recipient
/// accepts → recipient can decrypt messages sent with the channel key.
#[test]
fn full_invite_flow_owner_to_recipient() {
    use willow_channel::ChannelKind;

    let owner = Identity::generate();
    let recipient = Identity::generate();

    // Owner creates a server with channels.
    let mut server = willow_channel::Server::new("E2E Server", owner.peer_id());
    let ch1 = server.create_channel("general", ChannelKind::Text).unwrap();
    let ch2 = server.create_channel("random", ChannelKind::Text).unwrap();

    let mut keys = std::collections::HashMap::new();
    let mut topic_map = std::collections::HashMap::new();
    for (ch_id, name) in [(ch1, "general"), (ch2, "random")] {
        let topic = format!("{}/{name}", server.id);
        if let Some(key) = server.channel_key(&ch_id) {
            keys.insert(topic.clone(), key.clone());
        }
        topic_map.insert(topic, (name.into(), ch_id));
    }

    // Owner gets recipient's public key from their PeerId.
    let recipient_peer_str = recipient.peer_id().to_string();
    let recipient_pub = crate::invite::peer_id_to_ed25519_public(&recipient_peer_str).unwrap();

    // Owner generates an encrypted invite.
    let code = crate::invite::generate_invite(&server, &keys, &topic_map, &recipient_pub).unwrap();

    // Recipient accepts the invite.
    let accepted = crate::invite::accept_invite(&code, &recipient).unwrap();
    assert_eq!(accepted.server_name, "E2E Server");
    assert_eq!(accepted.channel_keys.len(), 2);

    // Verify recipient's decrypted keys match the originals.
    for (topic, (_, decrypted_key)) in &accepted.channel_keys {
        let original_key = &keys[topic];
        assert_eq!(decrypted_key.as_bytes(), original_key.as_bytes());
    }

    // Owner sends an encrypted message using the channel key.
    let general_topic = format!("{}/general", server.id);
    let owner_key = &keys[&general_topic];
    let content = Content::Text {
        body: "hello from owner".into(),
    };
    let sealed = willow_crypto::seal_content(&content, owner_key, 0).unwrap();

    // Recipient decrypts with their copy of the key.
    let (_, recipient_key) = &accepted.channel_keys[&general_topic];
    let decrypted = willow_crypto::open_content(&sealed, recipient_key).unwrap();
    assert_eq!(
        decrypted,
        Content::Text {
            body: "hello from owner".into()
        }
    );
}

/// Verify that PeerId extraction works for the invite flow.
#[test]
fn invite_flow_peer_id_extraction() {
    let id = Identity::generate();
    let peer_str = id.peer_id().to_string();

    // Extract public key from PeerId string.
    let pub_bytes = crate::invite::peer_id_to_ed25519_public(&peer_str).unwrap();

    // Encrypt something for this public key.
    let key = generate_channel_key();
    let encrypted = willow_crypto::encrypt_channel_key_for(&key, &pub_bytes).unwrap();

    // Decrypt with the original identity.
    let decrypted = willow_crypto::decrypt_channel_key(&encrypted, &id).unwrap();
    assert_eq!(decrypted.as_bytes(), key.as_bytes());
}

/// Test that accepting an invite sets up the correct topic → key mapping.
#[test]
fn accepted_invite_provides_correct_topic_mapping() {
    use willow_channel::ChannelKind;

    let owner = Identity::generate();
    let recipient = Identity::generate();

    let mut server = willow_channel::Server::new("Mapped", owner.peer_id());
    server.create_channel("alpha", ChannelKind::Text).unwrap();
    server.create_channel("beta", ChannelKind::Text).unwrap();

    let mut keys = std::collections::HashMap::new();
    let mut topic_map = std::collections::HashMap::new();
    for ch in server.channels() {
        let topic = format!("{}/{}", server.id, ch.name);
        if let Some(key) = server.channel_key(&ch.id) {
            keys.insert(topic.clone(), key.clone());
        }
        topic_map.insert(topic, (ch.name.clone(), ch.id.clone()));
    }

    let recipient_pub =
        crate::invite::peer_id_to_ed25519_public(&recipient.peer_id().to_string()).unwrap();

    let code = crate::invite::generate_invite(&server, &keys, &topic_map, &recipient_pub).unwrap();
    let accepted = crate::invite::accept_invite(&code, &recipient).unwrap();

    // Each topic in the accepted invite should map to the right channel name.
    for (topic, (name, _)) in &accepted.channel_keys {
        assert!(topic.ends_with(name));
    }
}

/// Test the headless app can process a joined invite's keys.
#[test]
fn app_processes_accepted_invite_keys() {
    use willow_channel::ChannelKind;

    let (mut app, cmd_rx) = test_app();

    let owner = Identity::generate();
    let recipient_identity = app.world().resource::<LocalIdentity>().0.clone();

    // Owner creates a server.
    let mut server = willow_channel::Server::new("AppTest", owner.peer_id());
    let ch_id = server.create_channel("general", ChannelKind::Text).unwrap();

    let mut keys = std::collections::HashMap::new();
    let mut topic_map = std::collections::HashMap::new();
    let topic = format!("{}/general", server.id);
    if let Some(key) = server.channel_key(&ch_id) {
        keys.insert(topic.clone(), key.clone());
    }
    topic_map.insert(topic.clone(), ("general".into(), ch_id));

    // Generate invite for the app's identity.
    let recipient_pub =
        crate::invite::peer_id_to_ed25519_public(&recipient_identity.peer_id().to_string())
            .unwrap();
    let code = crate::invite::generate_invite(&server, &keys, &topic_map, &recipient_pub).unwrap();

    // Accept the invite.
    let accepted = crate::invite::accept_invite(&code, &recipient_identity).unwrap();

    // Install the decrypted keys into the app's ChannelKeyStore.
    for (topic, (_, key)) in &accepted.channel_keys {
        app.world_mut()
            .resource_mut::<ChannelKeyStore>()
            .keys
            .insert(topic.clone(), key.clone());
    }

    // Now the app should be able to decrypt a message encrypted with the owner's key.
    let owner_key = &keys[&topic];
    let content = Content::Text {
        body: "encrypted for app".into(),
    };
    let sealed = willow_crypto::seal_content(&content, owner_key, 0).unwrap();
    let encrypted_content = Content::Encrypted(sealed);
    let owner_str = owner.peer_id().to_string();
    let (stamped, _) = make_chat_op(&topic, encrypted_content, &owner_str);

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped,
            from: owner_str,
        });
    app.update();

    let state = app.world().resource::<ChatState>();
    assert_eq!(state.messages.len(), 1);
    assert_eq!(state.messages[0].body, "encrypted for app");

    // Drain any subscribe commands.
    while cmd_rx.try_recv().is_ok() {}
}

// ───── Emoji Reaction Tests ─────────────────────────────────────────────────

#[test]
fn reaction_updates_target_message() {
    let (mut app, _rx) = test_app();

    // Send a message first so we have a target.
    let sender = Identity::generate();
    let sender_str = sender.peer_id().to_string();
    let (stamped_msg, msg_op_id) = make_chat_op(
        "general",
        Content::Text {
            body: "react to me".into(),
        },
        &sender_str,
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped_msg,
            from: sender_str,
        });
    app.update();

    // Now send a reaction targeting that message (using op_id as target).
    let reactor = Identity::generate();
    let reactor_str = reactor.peer_id().to_string();
    let target_id =
        willow_messaging::MessageId(uuid::Uuid::parse_str(&msg_op_id).unwrap_or_default());
    let (stamped_reaction, _) = make_chat_op(
        "general",
        Content::Reaction {
            target: target_id,
            emoji: "👍".into(),
        },
        &reactor_str,
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped_reaction,
            from: reactor_str,
        });
    app.update();

    // The original message should now have the reaction.
    let state = app.world().resource::<ChatState>();
    assert_eq!(state.messages.len(), 1); // Reaction doesn't add a new message.
    assert!(state.messages[0].reactions.contains_key("👍"));
    assert_eq!(state.messages[0].reactions["👍"].len(), 1);
}

#[test]
fn multiple_reactions_on_same_message() {
    let (mut app, _rx) = test_app();

    let sender = Identity::generate();
    let sender_str = sender.peer_id().to_string();
    let (stamped_msg, msg_op_id) = make_chat_op(
        "general",
        Content::Text {
            body: "popular msg".into(),
        },
        &sender_str,
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped_msg,
            from: sender_str,
        });
    app.update();

    // Two different people react with the same emoji.
    let target_id =
        willow_messaging::MessageId(uuid::Uuid::parse_str(&msg_op_id).unwrap_or_default());
    for _ in 0..2 {
        let reactor = Identity::generate();
        let reactor_str = reactor.peer_id().to_string();
        let (stamped_reaction, _) = make_chat_op(
            "general",
            Content::Reaction {
                target: target_id.clone(),
                emoji: "🎉".into(),
            },
            &reactor_str,
        );
        app.world_mut()
            .write_message(NetworkBridgeEvent::OpReceived {
                stamped_op: stamped_reaction,
                from: reactor_str,
            });
    }
    app.update();

    let state = app.world().resource::<ChatState>();
    assert_eq!(state.messages[0].reactions["🎉"].len(), 2);
}

#[test]
fn reaction_to_nonexistent_message_ignored() {
    let (mut app, _rx) = test_app();

    let reactor = Identity::generate();
    let reactor_str = reactor.peer_id().to_string();
    let (stamped, _) = make_chat_op(
        "general",
        Content::Reaction {
            target: willow_messaging::MessageId::new(),
            emoji: "👎".into(),
        },
        &reactor_str,
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped,
            from: reactor_str,
        });
    app.update();

    let state = app.world().resource::<ChatState>();
    assert!(state.messages.is_empty());
}

// ───── Edit & Delete Tests ──────────────────────────────────────────────────

#[test]
fn edit_updates_message_body() {
    let (mut app, _rx) = test_app();

    let sender = Identity::generate();
    let sender_str = sender.peer_id().to_string();
    let (stamped_msg, msg_op_id) = make_chat_op(
        "general",
        Content::Text {
            body: "original".into(),
        },
        &sender_str,
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped_msg,
            from: sender_str.clone(),
        });
    app.update();

    let target_id =
        willow_messaging::MessageId(uuid::Uuid::parse_str(&msg_op_id).unwrap_or_default());
    let (stamped_edit, _) = make_chat_op(
        "general",
        Content::Edit {
            target: target_id,
            new_body: "edited body".into(),
        },
        &sender_str,
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped_edit,
            from: sender_str,
        });
    app.update();

    let state = app.world().resource::<ChatState>();
    assert_eq!(state.messages.len(), 1);
    assert_eq!(state.messages[0].body, "edited body");
    assert!(state.messages[0].edited);
}

#[test]
fn delete_marks_message() {
    let (mut app, _rx) = test_app();

    let sender = Identity::generate();
    let sender_str = sender.peer_id().to_string();
    let (stamped_msg, msg_op_id) = make_chat_op(
        "general",
        Content::Text {
            body: "delete me".into(),
        },
        &sender_str,
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped_msg,
            from: sender_str.clone(),
        });
    app.update();

    let target_id =
        willow_messaging::MessageId(uuid::Uuid::parse_str(&msg_op_id).unwrap_or_default());
    let (stamped_del, _) = make_chat_op(
        "general",
        Content::Delete { target: target_id },
        &sender_str,
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped_del,
            from: sender_str,
        });
    app.update();

    let state = app.world().resource::<ChatState>();
    assert_eq!(state.messages.len(), 1);
    assert!(state.messages[0].deleted);
    assert_eq!(state.messages[0].body, "[message deleted]");
}

// ───── Reply Tests ──────────────────────────────────────────────────────────

#[test]
fn reply_shows_parent_preview() {
    let (mut app, _rx) = test_app();

    // Send a parent message.
    let sender = Identity::generate();
    let sender_str = sender.peer_id().to_string();
    let (stamped_parent, parent_op_id) = make_chat_op(
        "general",
        Content::Text {
            body: "parent message".into(),
        },
        &sender_str,
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped_parent,
            from: sender_str,
        });
    app.update();

    // Send a reply.
    let replier = Identity::generate();
    let replier_str = replier.peer_id().to_string();
    let parent_msg_id =
        willow_messaging::MessageId(uuid::Uuid::parse_str(&parent_op_id).unwrap_or_default());
    let (stamped_reply, _) = make_chat_op(
        "general",
        Content::Reply {
            parent: parent_msg_id,
            body: "this is my reply".into(),
        },
        &replier_str,
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped_reply,
            from: replier_str,
        });
    app.update();

    let state = app.world().resource::<ChatState>();
    assert_eq!(state.messages.len(), 2);
    assert_eq!(state.messages[1].body, "this is my reply");
    assert!(state.messages[1].reply_preview.is_some());
    assert!(state.messages[1]
        .reply_preview
        .as_ref()
        .unwrap()
        .contains("parent message"));
}

// ───── OpLog & Server Sync Tests ──────────────────────────────────────────

#[test]
fn oplog_dedup_rejects_duplicate() {
    use crate::server_sync::{Op, StampedOp};
    use willow_messaging::hlc::HLC;

    let mut log = OpLog::default();
    let mut hlc = HLC::new();

    let op = StampedOp::new(
        Op::CreateChannel {
            name: "test".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        "peer-a",
    );

    assert!(log.record(op.clone()));
    assert!(!log.record(op)); // duplicate
    assert_eq!(log.ops.len(), 1);
}

#[test]
fn oplog_trust_peer_updates_set() {
    use crate::server_sync::{Op, StampedOp};
    use willow_messaging::hlc::HLC;

    let mut log = OpLog::default();
    let mut hlc = HLC::new();

    assert!(!log.is_trusted("alice", "owner"));

    log.record(StampedOp::new(
        Op::TrustPeer {
            peer_id: "alice".into(),
        },
        &mut hlc,
        "owner",
    ));
    assert!(log.is_trusted("alice", "owner"));

    log.record(StampedOp::new(
        Op::UntrustPeer {
            peer_id: "alice".into(),
        },
        &mut hlc,
        "owner",
    ));
    assert!(!log.is_trusted("alice", "owner"));
}

#[test]
fn oplog_owner_always_trusted() {
    let log = OpLog::default();
    assert!(log.is_trusted("owner-peer", "owner-peer"));
    assert!(!log.is_trusted("stranger", "owner-peer"));
}

#[test]
fn oplog_rebuild_restores_state() {
    use crate::server_sync::{Op, StampedOp};
    use willow_messaging::hlc::HLC;

    let mut log = OpLog::default();
    let mut hlc = HLC::new();

    log.record(StampedOp::new(
        Op::TrustPeer {
            peer_id: "alice".into(),
        },
        &mut hlc,
        "owner",
    ));
    log.record(StampedOp::new(
        Op::CreateChannel {
            name: "general".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        "owner",
    ));

    // Simulate reload: clear runtime state, rebuild
    log.seen_ids.clear();
    log.trusted_peers.clear();
    log.rebuild();

    assert_eq!(log.ops.len(), 2);
    assert!(log.is_trusted("alice", "owner"));
    assert!(log.seen_ids.len() == 2);
}

#[test]
fn oplog_latest_hlc_tracks_most_recent() {
    use crate::server_sync::{Op, StampedOp};
    use willow_messaging::hlc::{HlcTimestamp, HLC};

    let mut log = OpLog::default();
    let mut hlc = HLC::new();

    assert_eq!(log.latest_hlc(), HlcTimestamp::ZERO);

    let op1 = StampedOp::new(
        Op::CreateChannel {
            name: "a".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        "peer",
    );
    let t1 = op1.hlc;
    log.record(op1);
    assert_eq!(log.latest_hlc(), t1);

    let op2 = StampedOp::new(
        Op::CreateChannel {
            name: "b".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        "peer",
    );
    let t2 = op2.hlc;
    log.record(op2);
    assert_eq!(log.latest_hlc(), t2);
    assert!(t2 > t1);
}

#[test]
fn untrusted_op_rejected_by_handler() {
    let (mut app, _cmd_rx) = test_app();

    // Set up a server with known owner
    let owner = app.world().resource::<LocalIdentity>().0.peer_id().clone();
    let server = willow_channel::Server::new("Test", owner.clone());
    app.world_mut().resource_mut::<ServerState>().server = Some(server);

    // Create a signed op from a different (untrusted) identity
    let untrusted = Identity::generate();
    let mut hlc = HLC::new();
    let stamped = crate::server_sync::StampedOp::new(
        crate::server_sync::Op::CreateChannel {
            name: "hacked".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        &untrusted.peer_id().to_string(),
    );
    let data = crate::server_sync::pack_op(&stamped, &untrusted).unwrap();
    let (sync_msg, _) = crate::server_sync::unpack_sync(&data).unwrap();

    // Inject the op event
    match sync_msg {
        crate::server_sync::SyncMessage::Op(stamped_op) => {
            app.world_mut()
                .write_message(NetworkBridgeEvent::OpReceived {
                    stamped_op: stamped_op.clone(),
                    from: stamped_op.author.clone(),
                });
        }
        _ => panic!("expected Op"),
    }
    app.update();

    // Channel should NOT have been created
    let server_state = app.world().resource::<ServerState>();
    let server = server_state.server.as_ref().unwrap();
    assert!(
        !server.channels().iter().any(|ch| ch.name == "hacked"),
        "untrusted op should be rejected"
    );

    // But the op_id should be recorded for dedup
    let op_log = app.world().resource::<OpLog>();
    assert!(op_log.seen_ids.contains(&stamped.op_id));
}

#[test]
fn trusted_op_applied_by_handler() {
    let (mut app, _cmd_rx) = test_app();

    let owner = app.world().resource::<LocalIdentity>().0.peer_id().clone();
    let server = willow_channel::Server::new("Test", owner.clone());
    app.world_mut().resource_mut::<ServerState>().server = Some(server);

    // Create a signed op from the owner (always trusted)
    let owner_identity = app.world().resource::<LocalIdentity>().0.clone();
    let mut hlc = HLC::new();
    let stamped = crate::server_sync::StampedOp::new(
        crate::server_sync::Op::CreateChannel {
            name: "legit".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        &owner_identity.peer_id().to_string(),
    );

    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped.clone(),
            from: owner_identity.peer_id().to_string(),
        });
    app.update();

    // Channel SHOULD have been created
    let server_state = app.world().resource::<ServerState>();
    let server = server_state.server.as_ref().unwrap();
    assert!(
        server.channels().iter().any(|ch| ch.name == "legit"),
        "trusted op should be applied"
    );
}

#[test]
fn sync_message_round_trip() {
    use crate::server_sync::{Op, StampedOp, SyncMessage};
    use willow_messaging::hlc::HLC;

    let id = Identity::generate();
    let mut hlc = HLC::new();

    // Test Op variant
    let stamped = StampedOp::new(
        Op::CreateChannel {
            name: "ch".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        "peer",
    );
    let msg = SyncMessage::Op(stamped.clone());
    let data = crate::server_sync::pack_sync(&msg, &id).unwrap();
    let (decoded, signer) = crate::server_sync::unpack_sync(&data).unwrap();
    assert_eq!(signer, id.peer_id());
    assert!(matches!(decoded, SyncMessage::Op(ref s) if s.op_id == stamped.op_id));

    // Test SyncRequest variant
    let req = SyncMessage::SyncRequest {
        latest_hlc: hlc.now(),
        topic: None,
    };
    let data = crate::server_sync::pack_sync(&req, &id).unwrap();
    let (decoded, _) = crate::server_sync::unpack_sync(&data).unwrap();
    assert!(matches!(decoded, SyncMessage::SyncRequest { .. }));

    // Test SyncBatch variant
    let batch = SyncMessage::SyncBatch { ops: vec![stamped] };
    let data = crate::server_sync::pack_sync(&batch, &id).unwrap();
    let (decoded, _) = crate::server_sync::unpack_sync(&data).unwrap();
    match decoded {
        SyncMessage::SyncBatch { ops } => assert_eq!(ops.len(), 1),
        _ => panic!("expected SyncBatch"),
    }
}

#[test]
fn set_permission_is_idempotent() {
    let (mut app, _cmd_rx) = test_app();

    let owner = app.world().resource::<LocalIdentity>().0.peer_id().clone();
    let mut server = willow_channel::Server::new("Test", owner.clone());
    let role = willow_channel::Role::new("mod");
    let role_id = server.create_role(role);
    app.world_mut().resource_mut::<ServerState>().server = Some(server);

    let owner_id_str = owner.to_string();
    let mut hlc = HLC::new();

    // Apply SetPermission { granted: true } twice — should be idempotent
    for _ in 0..2 {
        let stamped = crate::server_sync::StampedOp::new(
            crate::server_sync::Op::SetPermission {
                role_id: role_id.to_string(),
                permission: "SendMessages".into(),
                granted: true,
            },
            &mut hlc,
            &owner_id_str,
        );
        app.world_mut()
            .write_message(NetworkBridgeEvent::OpReceived {
                stamped_op: stamped.clone(),
                from: owner_id_str.clone(),
            });
        app.update();
    }

    let server = app
        .world()
        .resource::<ServerState>()
        .server
        .as_ref()
        .unwrap();
    let role = server.role(&role_id).unwrap();
    assert!(role
        .permissions
        .contains(&willow_channel::Permission::SendMessages));
}

#[test]
fn catchup_sync_applies_missing_ops() {
    // Simulate the full catch-up flow:
    // 1. Peer A (owner) creates a channel — recorded in A's OpLog
    // 2. Peer B connects and sends SyncRequest
    // 3. A's handler responds with ops newer than B's HLC
    // 4. B receives the SyncBatch and applies the ops
    // 5. B's server state now has the channel

    // --- Set up Peer A with a server and a channel ---
    let (mut app_a, cmd_rx_a) = test_app();
    let owner = app_a
        .world()
        .resource::<LocalIdentity>()
        .0
        .peer_id()
        .clone();
    let owner_str = owner.to_string();
    let mut server_a = willow_channel::Server::new("Sync Test", owner.clone());
    let ch_id = server_a
        .create_channel("synced-channel", willow_channel::ChannelKind::Text)
        .unwrap();
    app_a.world_mut().resource_mut::<ServerState>().server = Some(server_a);

    // Record the CreateChannel op in A's OpLog
    let mut hlc_a = HLC::new();
    let create_op = crate::server_sync::StampedOp::new(
        crate::server_sync::Op::CreateChannel {
            name: "synced-channel".into(),
            channel_id: ch_id.to_string(),
        },
        &mut hlc_a,
        &owner_str,
    );
    app_a
        .world_mut()
        .resource_mut::<OpLog>()
        .record(create_op.clone());

    // --- Peer B sends SyncRequest (latest_hlc = ZERO, i.e. "I have nothing") ---
    // A receives the request. Inject it as a SyncRequested event.
    app_a
        .world_mut()
        .write_message(NetworkBridgeEvent::SyncRequested {
            latest_hlc: willow_messaging::hlc::HlcTimestamp::ZERO,
            from: owner_str.clone(), // from a trusted peer (owner trusts self)
            topic: None,
        });
    app_a.update();

    // A should have sent a SendSyncBatch command with the op.
    let mut found_batch = false;
    while let Ok(cmd) = cmd_rx_a.try_recv() {
        if let NetworkBridgeCommand::SendSyncBatch { ops } = cmd {
            assert!(!ops.is_empty(), "batch should contain ops");
            assert!(
                ops.iter()
                    .any(|op| matches!(&op.op, crate::server_sync::Op::CreateChannel { name, .. } if name == "synced-channel")),
                "batch should contain the CreateChannel op"
            );
            found_batch = true;

            // --- Now simulate B receiving this batch ---
            let (mut app_b, _cmd_rx_b) = test_app();
            // B has a fresh server (same owner for simplicity — in reality different)
            let b_owner = app_b
                .world()
                .resource::<LocalIdentity>()
                .0
                .peer_id()
                .clone();
            let server_b = willow_channel::Server::new("B Server", b_owner);
            app_b.world_mut().resource_mut::<ServerState>().server = Some(server_b);

            // Trust A's owner so B accepts the ops.
            app_b
                .world_mut()
                .resource_mut::<OpLog>()
                .trusted_peers
                .insert(owner_str.clone());

            // Inject the SyncBatch event into B
            app_b
                .world_mut()
                .write_message(NetworkBridgeEvent::SyncBatchReceived {
                    ops: ops.clone(),
                    from: owner_str.clone(),
                });
            app_b.update();

            // B should now have the "synced-channel"
            let b_server = app_b
                .world()
                .resource::<ServerState>()
                .server
                .as_ref()
                .unwrap();
            assert!(
                b_server
                    .channels()
                    .iter()
                    .any(|ch| ch.name == "synced-channel"),
                "B should have the synced channel after catch-up"
            );

            // B's OpLog should contain the op
            let b_log = app_b.world().resource::<OpLog>();
            assert!(b_log.seen_ids.contains(&create_op.op_id));
        }
    }
    assert!(found_batch, "A should have sent a SyncBatch command");
}

#[test]
fn catchup_filters_by_hlc() {
    // Verify that SyncRequest only returns ops NEWER than the requested HLC.
    let (mut app, cmd_rx) = test_app();
    let owner = app.world().resource::<LocalIdentity>().0.peer_id().clone();
    let owner_str = owner.to_string();
    let server = willow_channel::Server::new("Filter Test", owner.clone());
    app.world_mut().resource_mut::<ServerState>().server = Some(server);

    // Record two ops with advancing HLC
    let mut hlc = HLC::new();
    let old_op = crate::server_sync::StampedOp::new(
        crate::server_sync::Op::CreateChannel {
            name: "old-channel".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        &owner_str,
    );
    let old_hlc = old_op.hlc;
    app.world_mut()
        .resource_mut::<OpLog>()
        .record(old_op.clone());

    let new_op = crate::server_sync::StampedOp::new(
        crate::server_sync::Op::CreateChannel {
            name: "new-channel".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        &owner_str,
    );
    app.world_mut()
        .resource_mut::<OpLog>()
        .record(new_op.clone());

    // SyncRequest with old_hlc — should only return new_op
    app.world_mut()
        .write_message(NetworkBridgeEvent::SyncRequested {
            latest_hlc: old_hlc,
            from: owner_str.clone(),
            topic: None,
        });
    app.update();

    let mut batch_ops = Vec::new();
    while let Ok(cmd) = cmd_rx.try_recv() {
        if let NetworkBridgeCommand::SendSyncBatch { ops } = cmd {
            batch_ops = ops;
        }
    }
    assert_eq!(
        batch_ops.len(),
        1,
        "should only return ops newer than old_hlc"
    );
    assert_eq!(batch_ops[0].op_id, new_op.op_id);
}

#[test]
fn catchup_rejects_untrusted_sync_request() {
    let (mut app, cmd_rx) = test_app();
    let owner = app.world().resource::<LocalIdentity>().0.peer_id().clone();
    let server = willow_channel::Server::new("Trust Test", owner.clone());
    app.world_mut().resource_mut::<ServerState>().server = Some(server);

    // Record an op
    let mut hlc = HLC::new();
    let op = crate::server_sync::StampedOp::new(
        crate::server_sync::Op::CreateChannel {
            name: "secret".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        &owner.to_string(),
    );
    app.world_mut().resource_mut::<OpLog>().record(op);

    // SyncRequest from an UNTRUSTED peer
    let stranger = Identity::generate();
    app.world_mut()
        .write_message(NetworkBridgeEvent::SyncRequested {
            latest_hlc: willow_messaging::hlc::HlcTimestamp::ZERO,
            from: stranger.peer_id().to_string(),
            topic: None,
        });
    app.update();

    // Should NOT have sent any SyncBatch
    let mut found_batch = false;
    while let Ok(cmd) = cmd_rx.try_recv() {
        if matches!(cmd, NetworkBridgeCommand::SendSyncBatch { .. }) {
            found_batch = true;
        }
    }
    assert!(!found_batch, "should not respond to untrusted sync request");
}

// ───── Chat History Catch-Up Tests ──────────────────────────────────────────

/// Build a headless app with a real (temporary) MessageDb for testing chat ops.
fn test_app_with_db() -> (
    App,
    std_mpsc::Receiver<NetworkBridgeCommand>,
    std::path::PathBuf,
) {
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
    app.insert_resource(ProfileStore::default());
    app.insert_resource(UnreadCounts::default());
    app.insert_resource(OpLog::default());
    app.insert_resource(SearchFilter::default());
    app.insert_resource(ChannelManagement::default());
    app.insert_resource(ButtonInput::<KeyCode>::default());

    // Create a temporary SQLite database.
    let tmp_dir = std::env::temp_dir().join(format!("willow_test_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&tmp_dir);
    let db_path = tmp_dir.join("messages.db");
    let db = crate::storage::MessageDb::open_path(&db_path).expect("open test db");
    let db_res = crate::ui::MessageDbRes(Some(std::sync::Arc::new(std::sync::Mutex::new(db))));
    app.insert_resource(db_res);

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

    (app, cmd_rx, tmp_dir)
}

#[test]
fn chat_ops_stored_on_send() {
    let (mut app, _cmd_rx, tmp_dir) = test_app_with_db();

    // Set up a topic in ServerState.
    let topic = "test-topic-send";
    app.world_mut()
        .resource_mut::<ServerState>()
        .topic_map
        .insert(
            topic.to_string(),
            (
                "general".to_string(),
                willow_channel::ChannelId(uuid::Uuid::new_v4()),
            ),
        );
    app.world_mut().resource_mut::<ChatState>().current_channel = "general".to_string();

    // Type and send a message.
    send_key(&mut app, KeyCode::KeyH, Some("h"));
    send_key(&mut app, KeyCode::KeyI, Some("i"));
    app.update();
    send_key(&mut app, KeyCode::Enter, None);
    app.update();
    app.update();

    // Verify chat op was stored in the DB.
    let db_res = app.world().resource::<crate::ui::MessageDbRes>();
    let db_arc = db_res.0.as_ref().expect("db should exist");
    let db_lock = db_arc.lock().unwrap();
    let ops = db_lock.load_chat_ops_since(topic, 0, 0, 100);
    assert_eq!(ops.len(), 1, "should have stored one chat op");
    assert!(
        matches!(&ops[0].op, crate::server_sync::Op::ChatMessage { topic: t, .. } if t == topic)
    );

    // Clean up.
    drop(db_lock);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn chat_ops_stored_on_receive() {
    let (mut app, _cmd_rx, tmp_dir) = test_app_with_db();

    let topic = "test-topic-recv";

    // Set up a server with an owner so trust checks pass.
    let owner_id = app.world().resource::<LocalIdentity>().0.peer_id();
    let owner_str = owner_id.to_string();
    let server = willow_channel::Server::new("Test", owner_id);
    app.world_mut().resource_mut::<ServerState>().server = Some(server);
    app.world_mut()
        .resource_mut::<ServerState>()
        .topic_map
        .insert(
            topic.to_string(),
            (
                "general".to_string(),
                willow_channel::ChannelId(uuid::Uuid::new_v4()),
            ),
        );

    // Create a chat message op.
    let content = Content::Text {
        body: "hello from remote".into(),
    };
    let (stamped, _op_id) = make_chat_op(topic, content, &owner_str);

    // Inject as OpReceived.
    app.world_mut()
        .write_message(NetworkBridgeEvent::OpReceived {
            stamped_op: stamped,
            from: owner_str,
        });
    app.update();

    // Verify chat op was stored in the DB.
    let db_res = app.world().resource::<crate::ui::MessageDbRes>();
    let db_arc = db_res.0.as_ref().expect("db should exist");
    let db_lock = db_arc.lock().unwrap();
    let ops = db_lock.load_chat_ops_since(topic, 0, 0, 100);
    assert_eq!(ops.len(), 1, "should have stored one chat op on receive");

    drop(db_lock);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn sync_request_with_topic_returns_chat_ops() {
    let (mut app, cmd_rx, tmp_dir) = test_app_with_db();

    let topic = "test-topic-sync";

    // Set up a server with an owner.
    let owner_id = app.world().resource::<LocalIdentity>().0.peer_id();
    let owner_str = owner_id.to_string();
    let server = willow_channel::Server::new("Test", owner_id);
    app.world_mut().resource_mut::<ServerState>().server = Some(server);
    app.world_mut()
        .resource_mut::<ServerState>()
        .topic_map
        .insert(
            topic.to_string(),
            (
                "general".to_string(),
                willow_channel::ChannelId(uuid::Uuid::new_v4()),
            ),
        );

    // Store a chat op in the DB.
    let content = Content::Text {
        body: "catchup msg".into(),
    };
    let (stamped, _op_id) = make_chat_op(topic, content, &owner_str);
    {
        let db_res = app.world().resource::<crate::ui::MessageDbRes>();
        let db_arc = db_res.0.as_ref().unwrap();
        let db_lock = db_arc.lock().unwrap();
        db_lock.insert_chat_op(&stamped, topic);
    }

    // Inject a SyncRequested event WITH a topic.
    app.world_mut()
        .write_message(NetworkBridgeEvent::SyncRequested {
            latest_hlc: willow_messaging::hlc::HlcTimestamp::ZERO,
            from: owner_str,
            topic: Some(topic.to_string()),
        });
    app.update();

    // Should respond with a SendSyncBatch containing the chat op.
    let mut found_batch = false;
    while let Ok(cmd) = cmd_rx.try_recv() {
        if let NetworkBridgeCommand::SendSyncBatch { ops } = cmd {
            assert_eq!(ops.len(), 1, "batch should contain one chat op");
            assert!(
                matches!(&ops[0].op, crate::server_sync::Op::ChatMessage { topic: t, .. } if t == topic)
            );
            found_batch = true;
        }
    }
    assert!(found_batch, "should have sent a SyncBatch with chat ops");

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn sync_request_without_topic_returns_server_ops() {
    let (mut app, cmd_rx, tmp_dir) = test_app_with_db();

    // Set up a server with an owner.
    let owner_id = app.world().resource::<LocalIdentity>().0.peer_id();
    let owner_str = owner_id.to_string();
    let server = willow_channel::Server::new("Test", owner_id);
    app.world_mut().resource_mut::<ServerState>().server = Some(server);

    // Add a server op to the OpLog.
    let mut hlc = HLC::new();
    let server_op = crate::server_sync::StampedOp::new(
        crate::server_sync::Op::CreateChannel {
            name: "catchup-ch".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        &owner_str,
    );
    app.world_mut()
        .resource_mut::<OpLog>()
        .record(server_op.clone());

    // SyncRequest without topic (None) should return server ops from OpLog.
    app.world_mut()
        .write_message(NetworkBridgeEvent::SyncRequested {
            latest_hlc: willow_messaging::hlc::HlcTimestamp::ZERO,
            from: owner_str,
            topic: None,
        });
    app.update();

    let mut found_batch = false;
    while let Ok(cmd) = cmd_rx.try_recv() {
        if let NetworkBridgeCommand::SendSyncBatch { ops } = cmd {
            assert!(
                ops.iter().any(|op| {
                    matches!(&op.op, crate::server_sync::Op::CreateChannel { name, .. } if name == "catchup-ch")
                }),
                "batch should contain the server op"
            );
            found_batch = true;
        }
    }
    assert!(found_batch, "should have sent a SyncBatch with server ops");

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn chat_ops_hlc_filtering() {
    // Test that load_chat_ops_since correctly filters by HLC.
    let tmp_dir = std::env::temp_dir().join(format!("willow_test_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&tmp_dir);
    let db_path = tmp_dir.join("messages.db");
    let db = crate::storage::MessageDb::open_path(&db_path).expect("open test db");

    let topic = "filter-test";

    // Create ops with known HLC timestamps.
    let mut hlc = HLC::new();
    let op1 = crate::server_sync::StampedOp::new(
        crate::server_sync::Op::ChatMessage {
            topic: topic.into(),
            content_data: willow_transport::pack(&Content::Text {
                body: "msg1".into(),
            })
            .unwrap(),
        },
        &mut hlc,
        "peer-a",
    );
    let hlc1 = op1.hlc;

    let op2 = crate::server_sync::StampedOp::new(
        crate::server_sync::Op::ChatMessage {
            topic: topic.into(),
            content_data: willow_transport::pack(&Content::Text {
                body: "msg2".into(),
            })
            .unwrap(),
        },
        &mut hlc,
        "peer-a",
    );

    let op3 = crate::server_sync::StampedOp::new(
        crate::server_sync::Op::ChatMessage {
            topic: topic.into(),
            content_data: willow_transport::pack(&Content::Text {
                body: "msg3".into(),
            })
            .unwrap(),
        },
        &mut hlc,
        "peer-a",
    );

    db.insert_chat_op(&op1, topic);
    db.insert_chat_op(&op2, topic);
    db.insert_chat_op(&op3, topic);

    // Request ops since ZERO — should return all 3.
    let all = db.load_chat_ops_since(topic, 0, 0, 100);
    assert_eq!(all.len(), 3, "should return all 3 ops");

    // Request ops since hlc1 — should skip op1, return 2.
    let after_first = db.load_chat_ops_since(topic, hlc1.millis, hlc1.counter, 100);
    assert_eq!(after_first.len(), 2, "should skip first op");

    // Request with limit 1 — should return only 1.
    let limited = db.load_chat_ops_since(topic, 0, 0, 1);
    assert_eq!(limited.len(), 1, "limit should cap results");

    // Request for a different topic — should return 0.
    let other = db.load_chat_ops_since("other-topic", 0, 0, 100);
    assert_eq!(other.len(), 0, "different topic should return nothing");

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn chat_op_dedup_on_insert() {
    // Inserting the same op_id twice should not duplicate.
    let tmp_dir = std::env::temp_dir().join(format!("willow_test_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&tmp_dir);
    let db_path = tmp_dir.join("messages.db");
    let db = crate::storage::MessageDb::open_path(&db_path).expect("open test db");

    let topic = "dedup-test";
    let content = Content::Text { body: "dup".into() };
    let (stamped, _) = make_chat_op(topic, content, "peer-a");

    db.insert_chat_op(&stamped, topic);
    db.insert_chat_op(&stamped, topic); // duplicate

    let ops = db.load_chat_ops_since(topic, 0, 0, 100);
    assert_eq!(ops.len(), 1, "duplicate insert should be ignored");

    let _ = std::fs::remove_dir_all(&tmp_dir);
}
