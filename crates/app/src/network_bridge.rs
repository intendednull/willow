//! # Network Bridge
//!
//! Bridges the [`willow_network`] layer into Bevy's synchronous ECS world.
//!
//! Network startup is deferred — the swarm is not created until a
//! [`ConnectCommand`] message is written, allowing the UI to configure the
//! relay address first.

use bevy::prelude::*;
use std::sync::{mpsc as std_mpsc, Arc, Mutex};

use willow_identity::Identity;

/// Global gossipsub topic for profile broadcasts.
pub const PROFILE_TOPIC: &str = "_willow_profiles";

/// Bevy resource holding the local user's identity.
#[derive(Resource)]
pub struct LocalIdentity(pub Identity);

/// Bevy resource for receiving network events on the main thread.
#[derive(Resource, Clone)]
pub struct NetworkEventReceiver(pub Arc<Mutex<std_mpsc::Receiver<NetworkBridgeEvent>>>);

/// Bevy resource for sending commands to the network task.
#[derive(Resource, Clone)]
pub struct NetworkCommandSender(pub std_mpsc::Sender<NetworkBridgeCommand>);

/// Whether the network is connected.
#[derive(Resource, Default)]
pub struct NetworkConnected(pub bool);

/// Events flowing from the network into Bevy.
#[derive(Debug, Clone, Message)]
pub enum NetworkBridgeEvent {
    MessageReceived {
        topic: String,
        data: Vec<u8>,
        source: Option<String>,
    },
    PeerConnected(String),
    PeerDisconnected(String),
    Listening(String),
    /// A file was announced by a peer (manifest received via gossipsub).
    FileAnnounced {
        filename: String,
        mime_type: String,
        size: u64,
        file_hash: String,
        from: String,
        topic: String,
    },
    /// A file download completed.
    FileDownloaded {
        filename: String,
        file_hash: String,
    },
    /// A peer's profile was received.
    ProfileReceived {
        peer_id: String,
        display_name: String,
    },
}

/// Commands flowing from Bevy to the network.
#[derive(Debug, Clone)]
pub enum NetworkBridgeCommand {
    Subscribe(String),
    Publish {
        topic: String,
        data: Vec<u8>,
    },
    /// Share a file: split, store chunks, broadcast manifest on the given topic.
    ShareFile {
        topic: String,
        filename: String,
        mime_type: String,
        data: Vec<u8>,
    },
    /// Broadcast our profile to peers.
    BroadcastProfile {
        display_name: String,
    },
}

/// Message to trigger network connection with an optional relay address.
#[derive(Debug, Clone, Message)]
pub struct ConnectCommand {
    pub relay_addr: Option<String>,
}

/// Bevy plugin that sets up the network bridge.
pub struct NetworkPlugin;

impl Plugin for NetworkPlugin {
    fn build(&self, app: &mut App) {
        let identity = load_identity();
        info!(peer_id = %identity.peer_id(), "local identity ready");

        let (event_tx, event_rx) = std_mpsc::channel();
        let (cmd_tx, cmd_rx) = std_mpsc::channel();

        // Store channels for deferred connection.
        app.insert_resource(LocalIdentity(identity))
            .insert_resource(NetworkEventReceiver(Arc::new(Mutex::new(event_rx))))
            .insert_resource(NetworkCommandSender(cmd_tx))
            .insert_resource(NetworkConnected::default())
            .insert_resource(DeferredNetworkChannels(Arc::new(Mutex::new((
                Some(event_tx),
                Some(cmd_rx),
            )))))
            .add_message::<NetworkBridgeEvent>()
            .add_message::<ConnectCommand>()
            .add_systems(Update, (handle_connect_command, poll_network_events));
    }
}

type ChannelPair = (
    Option<std_mpsc::Sender<NetworkBridgeEvent>>,
    Option<std_mpsc::Receiver<NetworkBridgeCommand>>,
);

/// Holds the channels until the network is spawned.
#[derive(Resource)]
struct DeferredNetworkChannels(Arc<Mutex<ChannelPair>>);

/// System that handles ConnectCommand to start the network.
fn handle_connect_command(
    mut reader: MessageReader<ConnectCommand>,
    identity: Res<LocalIdentity>,
    deferred: Res<DeferredNetworkChannels>,
    mut connected: ResMut<NetworkConnected>,
) {
    for cmd in reader.read() {
        if connected.0 {
            warn!("network already connected, ignoring ConnectCommand");
            continue;
        }

        let Ok(mut channels) = deferred.0.lock() else {
            continue;
        };
        let Some(event_tx) = channels.0.take() else {
            continue;
        };
        let Some(cmd_rx) = channels.1.take() else {
            continue;
        };

        let settings = crate::storage::NetworkSettings {
            relay_addr: cmd.relay_addr.clone(),
        };
        crate::storage::save_settings(&settings);

        let config = build_network_config(cmd.relay_addr.as_deref());
        spawn_network(identity.0.clone(), event_tx, cmd_rx, config);
        connected.0 = true;
        info!("network started");
    }
}

/// System that drains the network event channel each frame.
fn poll_network_events(
    receiver: Res<NetworkEventReceiver>,
    mut messages: MessageWriter<NetworkBridgeEvent>,
) {
    let Ok(rx) = receiver.0.lock() else { return };
    while let Ok(event) = rx.try_recv() {
        messages.write(event);
    }
}

// ───── Network config ───────────────────────────────────────────────────────

fn build_network_config(relay_addr: Option<&str>) -> willow_network::NetworkConfig {
    let mut config = willow_network::NetworkConfig::default();

    if let Some(addr) = relay_addr {
        match config.clone().with_relay(addr) {
            Ok(c) => {
                info!(relay = %addr, "configured relay");
                config = c;
            }
            Err(e) => {
                warn!(relay = %addr, %e, "invalid relay address, ignoring");
            }
        }
    }

    config
}

// ───── Identity persistence ──────────────────────────────────────────────────

fn load_identity() -> Identity {
    if let Some(bytes) = crate::storage::load_identity_bytes() {
        if let Some(id) = Identity::from_ed25519_bytes(&bytes) {
            return id;
        }
    }

    let identity = Identity::generate();
    if let Some(bytes) = identity.to_ed25519_bytes() {
        crate::storage::save_identity_bytes(&bytes);
    }
    identity
}

// ───── Native ───────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn spawn_network(
    identity: Identity,
    event_tx: std_mpsc::Sender<NetworkBridgeEvent>,
    cmd_rx: std_mpsc::Receiver<NetworkBridgeCommand>,
    config: willow_network::NetworkConfig,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        rt.block_on(async move {
            match run_network(identity, event_tx, cmd_rx, config).await {
                Ok(()) => info!("network task exited cleanly"),
                Err(e) => error!("network task failed: {e}"),
            }
        });
    });
}

#[cfg(not(target_arch = "wasm32"))]
async fn run_network(
    identity: Identity,
    event_tx: std_mpsc::Sender<NetworkBridgeEvent>,
    cmd_rx: std_mpsc::Receiver<NetworkBridgeCommand>,
    config: willow_network::NetworkConfig,
) -> anyhow::Result<()> {
    use willow_network::{file_transfer::ChunkResponse, NetworkEvent, NetworkNode};

    let (node, mut events) = NetworkNode::start(identity, config).await?;
    let mut file_mgr = crate::file_manager::FileManager::new();

    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                match event {
                    NetworkEvent::Message { topic, data, source } => {
                        // Try to parse as a known envelope type.
                        if let Ok((manifest, willow_transport::MessageType::File)) =
                            willow_transport::unpack_envelope::<willow_files::FileManifest>(&data)
                        {
                            let from = source.map(|p| p.to_string()).unwrap_or_default();
                            file_mgr.register_manifest(manifest.clone());
                            let _ = event_tx.send(NetworkBridgeEvent::FileAnnounced {
                                filename: manifest.filename.clone(),
                                mime_type: manifest.mime_type.clone(),
                                size: manifest.total_size,
                                file_hash: manifest.file_hash.to_hex(),
                                from,
                                topic: topic.clone(),
                            });
                        } else if let Ok((profile, willow_transport::MessageType::Identity)) =
                            willow_transport::unpack_envelope::<willow_identity::UserProfile>(&data)
                        {
                            let _ = event_tx.send(NetworkBridgeEvent::ProfileReceived {
                                peer_id: profile.peer_id.to_string(),
                                display_name: profile.display_name,
                            });
                        } else {
                            let _ = event_tx.send(NetworkBridgeEvent::MessageReceived {
                                topic,
                                data,
                                source: source.map(|p| p.to_string()),
                            });
                        }
                    }
                    NetworkEvent::PeerConnected(peer) => {
                        let _ = event_tx.send(NetworkBridgeEvent::PeerConnected(peer.to_string()));
                    }
                    NetworkEvent::PeerDisconnected(peer) => {
                        let _ = event_tx.send(NetworkBridgeEvent::PeerDisconnected(peer.to_string()));
                    }
                    NetworkEvent::Listening(addr) => {
                        let _ = event_tx.send(NetworkBridgeEvent::Listening(addr.to_string()));
                    }
                    NetworkEvent::ChunkRequested { channel, hash, .. } => {
                        // Auto-respond with chunk data if we have it.
                        let response = if let Some(data) = file_mgr.get_chunk(&hash) {
                            ChunkResponse::Found { hash, data: data.to_vec() }
                        } else {
                            ChunkResponse::NotFound { hash }
                        };
                        let _ = node.respond_chunk(channel, response);
                    }
                    NetworkEvent::ChunkReceived {
                        response: ChunkResponse::Found { hash, data },
                        ..
                    } => {
                        file_mgr.add_chunk(hash, data);
                    }
                    _ => {}
                }
            }

            _ = tokio::time::sleep(std::time::Duration::from_millis(16)) => {
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        NetworkBridgeCommand::Subscribe(topic) => {
                            node.subscribe(&topic)?;
                        }
                        NetworkBridgeCommand::Publish { topic, data } => {
                            node.publish(&topic, data)?;
                        }
                        NetworkBridgeCommand::ShareFile { topic, filename, mime_type, data } => {
                            if let Some((manifest, envelope)) = file_mgr.share_file(&data, filename.clone(), mime_type) {
                                node.publish(&topic, envelope)?;
                                info!(file = %filename, hash = %manifest.file_hash, "shared file");
                            }
                        }
                        NetworkBridgeCommand::BroadcastProfile { display_name } => {
                            let profile = willow_identity::UserProfile::new(
                                willow_identity::PeerId::from(node.peer_id()),
                                &display_name,
                            );
                            if let Ok(data) = willow_transport::pack_envelope(
                                willow_transport::MessageType::Identity, &profile,
                            ) {
                                let _ = node.publish(PROFILE_TOPIC, data);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

// ───── WASM ─────────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn spawn_network(
    identity: Identity,
    event_tx: std_mpsc::Sender<NetworkBridgeEvent>,
    cmd_rx: std_mpsc::Receiver<NetworkBridgeCommand>,
    config: willow_network::NetworkConfig,
) {
    wasm_bindgen_futures::spawn_local(async move {
        match run_network_wasm(identity, event_tx, cmd_rx, config).await {
            Ok(()) => info!("network task exited cleanly"),
            Err(e) => error!("network task failed: {e}"),
        }
    });
}

#[cfg(target_arch = "wasm32")]
async fn run_network_wasm(
    identity: Identity,
    event_tx: std_mpsc::Sender<NetworkBridgeEvent>,
    cmd_rx: std_mpsc::Receiver<NetworkBridgeCommand>,
    config: willow_network::NetworkConfig,
) -> anyhow::Result<()> {
    use futures::StreamExt;
    use willow_network::{NetworkEvent, NetworkNode};

    let (node, mut events) = NetworkNode::start(identity, config).await?;
    let mut file_mgr = crate::file_manager::FileManager::new();

    loop {
        futures::select! {
            event = events.next() => {
                let Some(event) = event else { break };
                match event {
                    NetworkEvent::Message { topic, data, source } => {
                        let _ = event_tx.send(NetworkBridgeEvent::MessageReceived {
                            topic,
                            data,
                            source: source.map(|p| p.to_string()),
                        });
                    }
                    NetworkEvent::PeerConnected(peer) => {
                        let _ = event_tx.send(NetworkBridgeEvent::PeerConnected(peer.to_string()));
                    }
                    NetworkEvent::PeerDisconnected(peer) => {
                        let _ = event_tx.send(NetworkBridgeEvent::PeerDisconnected(peer.to_string()));
                    }
                    NetworkEvent::Listening(addr) => {
                        let _ = event_tx.send(NetworkBridgeEvent::Listening(addr.to_string()));
                    }
                    _ => {}
                }

                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        NetworkBridgeCommand::Subscribe(topic) => {
                            node.subscribe(&topic)?;
                        }
                        NetworkBridgeCommand::Publish { topic, data } => {
                            node.publish(&topic, data)?;
                        }
                        NetworkBridgeCommand::ShareFile { topic, filename, mime_type, data } => {
                            if let Some((_manifest, envelope)) = file_mgr.share_file(&data, filename, mime_type) {
                                node.publish(&topic, envelope)?;
                            }
                        }
                        NetworkBridgeCommand::BroadcastProfile { display_name } => {
                            let profile = willow_identity::UserProfile::new(
                                willow_identity::PeerId::from(node.peer_id()),
                                &display_name,
                            );
                            if let Ok(data) = willow_transport::pack_envelope(
                                willow_transport::MessageType::Identity, &profile,
                            ) {
                                let _ = node.publish(PROFILE_TOPIC, data);
                            }
                        }
                    }
                }
            }

            complete => break,
        }
    }

    Ok(())
}
