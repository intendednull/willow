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
}

/// Commands flowing from Bevy to the network.
#[derive(Debug, Clone)]
pub enum NetworkBridgeCommand {
    Subscribe(String),
    Publish { topic: String, data: Vec<u8> },
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
    use willow_network::{NetworkEvent, NetworkNode};

    let (node, mut events) = NetworkNode::start(identity, config).await?;

    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                let bridge_event = match event {
                    NetworkEvent::Message { topic, data, source } => {
                        NetworkBridgeEvent::MessageReceived {
                            topic,
                            data,
                            source: source.map(|p| p.to_string()),
                        }
                    }
                    NetworkEvent::PeerConnected(peer) => {
                        NetworkBridgeEvent::PeerConnected(peer.to_string())
                    }
                    NetworkEvent::PeerDisconnected(peer) => {
                        NetworkBridgeEvent::PeerDisconnected(peer.to_string())
                    }
                    NetworkEvent::Listening(addr) => {
                        NetworkBridgeEvent::Listening(addr.to_string())
                    }
                    _ => continue,
                };
                let _ = event_tx.send(bridge_event);
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

    loop {
        futures::select! {
            event = events.next() => {
                let Some(event) = event else { break };
                let bridge_event = match event {
                    NetworkEvent::Message { topic, data, source } => {
                        NetworkBridgeEvent::MessageReceived {
                            topic,
                            data,
                            source: source.map(|p| p.to_string()),
                        }
                    }
                    NetworkEvent::PeerConnected(peer) => {
                        NetworkBridgeEvent::PeerConnected(peer.to_string())
                    }
                    NetworkEvent::PeerDisconnected(peer) => {
                        NetworkBridgeEvent::PeerDisconnected(peer.to_string())
                    }
                    NetworkEvent::Listening(addr) => {
                        NetworkBridgeEvent::Listening(addr.to_string())
                    }
                    _ => continue,
                };
                let _ = event_tx.send(bridge_event);

                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        NetworkBridgeCommand::Subscribe(topic) => {
                            node.subscribe(&topic)?;
                        }
                        NetworkBridgeCommand::Publish { topic, data } => {
                            node.publish(&topic, data)?;
                        }
                    }
                }
            }

            complete => break,
        }
    }

    Ok(())
}
