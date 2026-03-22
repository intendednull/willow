//! # Network Bridge
//!
//! Bridges the [`willow_network`] layer into Bevy's synchronous ECS world.
//!
//! - **Native**: spawns a tokio runtime on a background thread.
//! - **WASM**: uses `wasm_bindgen_futures::spawn_local` (single-threaded).

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

/// Bevy plugin that sets up the network bridge.
pub struct NetworkPlugin;

impl Plugin for NetworkPlugin {
    fn build(&self, app: &mut App) {
        let identity = load_identity();
        info!(peer_id = %identity.peer_id(), "local identity ready");

        let (event_tx, event_rx) = std_mpsc::channel();
        let (cmd_tx, cmd_rx) = std_mpsc::channel();

        spawn_network(identity.clone(), event_tx, cmd_rx);

        app.insert_resource(LocalIdentity(identity))
            .insert_resource(NetworkEventReceiver(Arc::new(Mutex::new(event_rx))))
            .insert_resource(NetworkCommandSender(cmd_tx))
            .add_message::<NetworkBridgeEvent>()
            .add_systems(Update, poll_network_events);
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

// ───── Identity persistence ──────────────────────────────────────────────────

fn load_identity() -> Identity {
    // Try to load persisted identity bytes.
    if let Some(bytes) = crate::storage::load_identity_bytes() {
        if let Some(id) = Identity::from_ed25519_bytes(&bytes) {
            return id;
        }
    }

    // Generate fresh and persist.
    let identity = Identity::generate();
    if let Some(bytes) = identity.to_ed25519_bytes() {
        crate::storage::save_identity_bytes(&bytes);
    }
    identity
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_network(
    identity: Identity,
    event_tx: std_mpsc::Sender<NetworkBridgeEvent>,
    cmd_rx: std_mpsc::Receiver<NetworkBridgeCommand>,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        rt.block_on(async move {
            match run_network(identity, event_tx, cmd_rx).await {
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
) -> anyhow::Result<()> {
    use willow_network::{NetworkConfig, NetworkEvent, NetworkNode};

    let config = NetworkConfig::default();
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
) {
    wasm_bindgen_futures::spawn_local(async move {
        match run_network_wasm(identity, event_tx, cmd_rx).await {
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
) -> anyhow::Result<()> {
    use futures::StreamExt;
    use willow_network::{NetworkConfig, NetworkEvent, NetworkNode};

    let config = NetworkConfig::default();
    let (node, mut events) = NetworkNode::start(identity, config).await?;

    // In WASM we use a simple poll loop since there's no tokio::select!.
    loop {
        // Check for network events.
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

                // Process pending commands.
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
