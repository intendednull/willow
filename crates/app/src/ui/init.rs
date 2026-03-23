//! Startup systems — server initialization, channel subscription.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::network_bridge::{ConnectCommand, LocalIdentity, NetworkCommandSender};
use willow_channel::ChannelKind;

use super::layout::make_topic;
use super::resources::*;

/// Load or create server, fire ConnectCommand, and load persisted messages.
pub fn init_server(
    identity: Res<LocalIdentity>,
    mut server_state: ResMut<ServerState>,
    mut key_store: ResMut<ChannelKeyStore>,
    mut connect_writer: MessageWriter<ConnectCommand>,
    mut state: ResMut<ChatState>,
    db: Res<MessageDbRes>,
    mut op_log: ResMut<OpLog>,
) {
    let (server, keys) = if let Some((server, keys)) = crate::storage::load_server() {
        info!("loaded server '{}' from disk", server.name);
        (server, keys)
    } else {
        info!("creating new server");
        let mut server = willow_channel::Server::new("My Server", identity.0.peer_id());
        let mut keys = HashMap::new();

        let default_channels = ["general", "random", "voice"];
        for name in default_channels {
            let ch_id = server
                .create_channel(name, ChannelKind::Text)
                .expect("default channel creation should not fail");

            let topic = make_topic(&server, name);
            if let Some(key) = server.channel_key(&ch_id) {
                keys.insert(topic, key.clone());
            }
        }

        (server, keys)
    };

    for ch in server.channels() {
        let topic = make_topic(&server, &ch.name);
        server_state
            .topic_map
            .insert(topic.clone(), (ch.name.clone(), ch.id.clone()));
    }

    key_store.keys = keys;
    crate::storage::save_server(&server, &key_store.keys);

    if let Some(ref db_arc) = db.0 {
        if let Ok(db_lock) = db_arc.lock() {
            for topic in server_state.topic_map.keys() {
                let stored = db_lock.load_topic(topic, 500);
                for sm in stored {
                    state.messages.push(ChatMessage::new(
                        sm.topic,
                        sm.author,
                        sm.body,
                        sm.is_local,
                        sm.timestamp_ms,
                    ));
                }
            }
            if !state.messages.is_empty() {
                state.messages_dirty = true;
                info!("loaded {} messages from database", state.messages.len());
            }
        }
    }

    // Load op log.
    if let Some(ops) = crate::storage::load_op_log() {
        for op in ops {
            op_log.record(op);
        }
        info!("loaded {} ops from op log", op_log.ops.len());
    }

    server_state.server = Some(server);

    let settings = crate::storage::load_settings().unwrap_or_default();
    connect_writer.write(ConnectCommand {
        relay_addr: settings.relay_addr,
    });
}

/// Subscribe to all channels once the network is connected.
pub fn subscribe_channels(
    connected: Res<crate::network_bridge::NetworkConnected>,
    server_state: Res<ServerState>,
    net_cmd: Res<NetworkCommandSender>,
    op_log: Res<OpLog>,
    mut subscribed: Local<bool>,
) {
    if *subscribed || !connected.0 {
        return;
    }
    *subscribed = true;

    for topic in server_state.topic_map.keys() {
        let _ = net_cmd
            .0
            .send(crate::network_bridge::NetworkBridgeCommand::Subscribe(
                topic.clone(),
            ));
    }

    // Subscribe to the global profile broadcast topic.
    let _ = net_cmd
        .0
        .send(crate::network_bridge::NetworkBridgeCommand::Subscribe(
            crate::network_bridge::PROFILE_TOPIC.to_string(),
        ));

    // Subscribe to server state operations topic.
    let _ = net_cmd
        .0
        .send(crate::network_bridge::NetworkBridgeCommand::Subscribe(
            crate::server_sync::SERVER_OPS_TOPIC.to_string(),
        ));

    // Broadcast our profile so peers learn our display name.
    let saved_profile = crate::storage::load_profile().unwrap_or_default();
    if !saved_profile.display_name.is_empty() {
        let _ = net_cmd.0.send(
            crate::network_bridge::NetworkBridgeCommand::BroadcastProfile {
                display_name: saved_profile.display_name,
            },
        );
    }

    // Request missing server ops from peers.
    let _ = net_cmd
        .0
        .send(crate::network_bridge::NetworkBridgeCommand::RequestSync {
            latest_hlc: op_log.latest_hlc(),
            topic: None,
        });
    info!("requested server state sync");

    // Request chat history for each channel.
    for topic in server_state.topic_map.keys() {
        let _ = net_cmd
            .0
            .send(crate::network_bridge::NetworkBridgeCommand::RequestSync {
                latest_hlc: willow_messaging::hlc::HlcTimestamp::ZERO,
                topic: Some(topic.clone()),
            });
    }
    info!(
        "requested chat history for {} channels",
        server_state.topic_map.len()
    );

    info!("subscribed to {} channels", server_state.topic_map.len());
}
