//! # Network Bridge
//!
//! Bridges the async tokio-based [`willow_network`] layer into Bevy's
//! synchronous ECS world.
//!
//! ## How it works
//!
//! 1. On startup, we spawn a tokio runtime on a background thread and start a
//!    [`NetworkNode`](willow_network::NetworkNode).
//! 2. Network events flow from the node into a crossbeam-style receiver that a
//!    Bevy system polls each frame.
//! 3. Outbound commands (publish, subscribe) go from Bevy systems to the node
//!    handle, which is stored as a Bevy resource.

use bevy::prelude::*;
use std::sync::{mpsc as std_mpsc, Arc, Mutex};

use willow_identity::Identity;

/// Bevy resource holding the local user's identity.
#[derive(Resource)]
pub struct LocalIdentity(pub Identity);

/// Bevy resource for receiving network events on the main thread.
///
/// Wrapped in `Arc<Mutex<>>` to satisfy Bevy's `Send + Sync` requirement on
/// resources. The mutex is only locked briefly each frame to drain events.
#[derive(Resource, Clone)]
pub struct NetworkEventReceiver(pub Arc<Mutex<std_mpsc::Receiver<NetworkBridgeEvent>>>);

/// Bevy resource for sending commands to the network task.
#[derive(Resource, Clone)]
pub struct NetworkCommandSender(pub std_mpsc::Sender<NetworkBridgeCommand>);

/// Events flowing from the network into Bevy.
#[derive(Debug, Clone, Event)]
pub enum NetworkBridgeEvent {
    /// A chat message arrived on a topic.
    MessageReceived {
        topic: String,
        data: Vec<u8>,
        source: Option<String>,
    },
    /// A peer connected.
    PeerConnected(String),
    /// A peer disconnected.
    PeerDisconnected(String),
    /// We are now listening on an address.
    Listening(String),
}

/// Commands flowing from Bevy to the network.
#[derive(Debug, Clone)]
pub enum NetworkBridgeCommand {
    /// Subscribe to a gossipsub topic.
    Subscribe(String),
    /// Publish data to a gossipsub topic.
    Publish { topic: String, data: Vec<u8> },
}

/// Bevy plugin that sets up the network bridge.
pub struct NetworkPlugin;

impl Plugin for NetworkPlugin {
    fn build(&self, app: &mut App) {
        let identity = Identity::generate();
        info!(peer_id = %identity.peer_id(), "generated local identity");

        let (event_tx, event_rx) = std_mpsc::channel();
        let (cmd_tx, cmd_rx) = std_mpsc::channel();

        // Spawn the tokio runtime + network node on a background thread.
        let net_identity = identity.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
            rt.block_on(async move {
                match run_network(net_identity, event_tx, cmd_rx).await {
                    Ok(()) => info!("network task exited cleanly"),
                    Err(e) => error!("network task failed: {e}"),
                }
            });
        });

        app.insert_resource(LocalIdentity(identity))
            .insert_resource(NetworkEventReceiver(Arc::new(Mutex::new(event_rx))))
            .insert_resource(NetworkCommandSender(cmd_tx))
            .add_event::<NetworkBridgeEvent>()
            .add_systems(Update, poll_network_events);
    }
}

/// System that drains the network event channel each frame.
fn poll_network_events(
    receiver: Res<NetworkEventReceiver>,
    mut events: EventWriter<NetworkBridgeEvent>,
) {
    let Ok(rx) = receiver.0.lock() else { return };
    while let Ok(event) = rx.try_recv() {
        events.send(event);
    }
}

/// Run the network node on the tokio runtime (called from background thread).
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
