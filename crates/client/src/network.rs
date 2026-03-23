//! # Network Types
//!
//! Command and event enums for communicating between the client logic and the
//! network layer. These are UI-framework-agnostic counterparts to the Bevy
//! bridge types in `willow-app`.
//!
//! Also contains `spawn_network`, `build_network_config`, and the async
//! `run_network` functions that drive the libp2p swarm.

/// Global gossipsub topic for profile broadcasts.
pub const PROFILE_TOPIC: &str = "_willow_profiles";

/// Events flowing from the network into the client.
#[derive(Debug, Clone)]
pub enum NetworkEvent {
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
    /// An event was received from a peer (new wire format).
    EventReceived {
        event: willow_state::Event,
        from: String,
    },
    /// A sync request was received from a peer (new wire format).
    SyncRequested {
        state_hash: willow_state::StateHash,
        from: String,
        topic: Option<String>,
    },
    /// A batch of events was received as a sync response (new wire format).
    SyncBatchReceived {
        events: Vec<willow_state::Event>,
        from: String,
    },
    /// A server operation was received from a peer (legacy wire format).
    #[allow(deprecated)]
    OpReceived {
        stamped_op: crate::ops::StampedOp,
        from: String,
    },
    /// A sync request in legacy format.
    LegacySyncRequested {
        latest_hlc: willow_messaging::hlc::HlcTimestamp,
        from: String,
        topic: Option<String>,
    },
    /// A batch of ops in legacy format.
    #[allow(deprecated)]
    LegacySyncBatchReceived {
        ops: Vec<crate::ops::StampedOp>,
        from: String,
    },
}

/// Commands flowing from the client to the network.
#[derive(Debug, Clone)]
pub enum NetworkCommand {
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
    /// Broadcast an event (new wire format).
    BroadcastEvent {
        event: willow_state::Event,
        topic: Option<String>,
    },
    /// Request missing events from peers (new wire format).
    RequestSync {
        state_hash: willow_state::StateHash,
        topic: Option<String>,
    },
    /// Send a batch of events as a sync response (new wire format).
    SendSyncBatch {
        events: Vec<willow_state::Event>,
    },
    /// Broadcast a server state operation (legacy wire format).
    #[allow(deprecated)]
    BroadcastOp(crate::ops::StampedOp),
    /// Request missing ops from peers (legacy wire format).
    #[allow(deprecated)]
    LegacyRequestSync {
        latest_hlc: willow_messaging::hlc::HlcTimestamp,
        topic: Option<String>,
    },
    /// Send a batch of ops as a sync response (legacy wire format).
    #[allow(deprecated)]
    LegacySendSyncBatch {
        ops: Vec<crate::ops::StampedOp>,
    },
}

// ───── Network config ───────────────────────────────────────────────────────

/// Build a [`willow_network::NetworkConfig`] from an optional relay address.
pub fn build_network_config(relay_addr: Option<&str>) -> willow_network::NetworkConfig {
    let mut config = willow_network::NetworkConfig::default();

    if let Some(addr) = relay_addr {
        if let Ok(c) = config.clone().with_relay(addr) {
            config = c;
        }
    }

    config
}

// ───── Native ───────────────────────────────────────────────────────────────

/// Spawn the network task on a background thread (native) or via
/// `spawn_local` (WASM).
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_network(
    identity: willow_identity::Identity,
    event_tx: std::sync::mpsc::Sender<NetworkEvent>,
    cmd_rx: std::sync::mpsc::Receiver<NetworkCommand>,
    config: willow_network::NetworkConfig,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        rt.block_on(async move {
            let _ = run_network(identity, event_tx, cmd_rx, config).await;
        });
    });
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(deprecated)]
async fn run_network(
    identity: willow_identity::Identity,
    event_tx: std::sync::mpsc::Sender<NetworkEvent>,
    cmd_rx: std::sync::mpsc::Receiver<NetworkCommand>,
    config: willow_network::NetworkConfig,
) -> anyhow::Result<()> {
    use willow_network::{file_transfer::ChunkResponse, NetworkEvent as NetEvt, NetworkNode};

    let (node, mut events) = NetworkNode::start(identity, config).await?;
    let mut file_mgr = crate::files::FileManager::new();

    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                match event {
                    NetEvt::Message { topic, data, source } => {
                        if let Ok((manifest, willow_transport::MessageType::File)) =
                            willow_transport::unpack_envelope::<willow_files::FileManifest>(&data)
                        {
                            let from = source.map(|p| p.to_string()).unwrap_or_default();
                            file_mgr.register_manifest(manifest.clone());
                            let _ = event_tx.send(NetworkEvent::FileAnnounced {
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
                            let _ = event_tx.send(NetworkEvent::ProfileReceived {
                                peer_id: profile.peer_id.to_string(),
                                display_name: profile.display_name,
                            });
                        }
                        // Try new wire format first.
                        else if let Some((wire_msg, signer)) =
                            crate::ops::unpack_wire(&data)
                        {
                            let from = signer.to_string();
                            match wire_msg {
                                crate::ops::WireMessage::Event(event) => {
                                    let _ = event_tx.send(NetworkEvent::EventReceived {
                                        event,
                                        from,
                                    });
                                }
                                crate::ops::WireMessage::SyncRequest { state_hash, topic } => {
                                    let _ = event_tx.send(NetworkEvent::SyncRequested {
                                        state_hash,
                                        from,
                                        topic,
                                    });
                                }
                                crate::ops::WireMessage::SyncBatch { events } => {
                                    let _ = event_tx.send(NetworkEvent::SyncBatchReceived {
                                        events,
                                        from,
                                    });
                                }
                            }
                        }
                        // Fall back to legacy wire format.
                        else if let Some((sync_msg, signer)) =
                            crate::ops::unpack_sync(&data)
                        {
                            let from = signer.to_string();
                            match sync_msg {
                                crate::ops::SyncMessage::Op(stamped_op) => {
                                    if stamped_op.author == from {
                                        let _ = event_tx.send(NetworkEvent::OpReceived {
                                            stamped_op,
                                            from,
                                        });
                                    }
                                }
                                crate::ops::SyncMessage::SyncRequest { latest_hlc, topic } => {
                                    let _ = event_tx.send(NetworkEvent::LegacySyncRequested {
                                        latest_hlc,
                                        from,
                                        topic,
                                    });
                                }
                                crate::ops::SyncMessage::SyncBatch { ops } => {
                                    let _ = event_tx.send(NetworkEvent::LegacySyncBatchReceived {
                                        ops,
                                        from,
                                    });
                                }
                            }
                        } else {
                            let _ = event_tx.send(NetworkEvent::MessageReceived {
                                topic,
                                data,
                                source: source.map(|p| p.to_string()),
                            });
                        }
                    }
                    NetEvt::PeerConnected(peer) => {
                        let _ = event_tx.send(NetworkEvent::PeerConnected(peer.to_string()));
                    }
                    NetEvt::PeerDisconnected(peer) => {
                        let _ = event_tx.send(NetworkEvent::PeerDisconnected(peer.to_string()));
                    }
                    NetEvt::Listening(addr) => {
                        let _ = event_tx.send(NetworkEvent::Listening(addr.to_string()));
                    }
                    NetEvt::ChunkRequested { channel, hash, .. } => {
                        let response = if let Some(data) = file_mgr.get_chunk(&hash) {
                            ChunkResponse::Found { hash, data: data.to_vec() }
                        } else {
                            ChunkResponse::NotFound { hash }
                        };
                        let _ = node.respond_chunk(channel, response);
                    }
                    NetEvt::ChunkReceived {
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
                    handle_network_command(&cmd, &node, &mut file_mgr)?;
                }
            }
        }
    }

    Ok(())
}

// ───── WASM ─────────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
pub fn spawn_network(
    identity: willow_identity::Identity,
    event_tx: std::sync::mpsc::Sender<NetworkEvent>,
    cmd_rx: std::sync::mpsc::Receiver<NetworkCommand>,
    config: willow_network::NetworkConfig,
) {
    wasm_bindgen_futures::spawn_local(async move {
        let _ = run_network_wasm(identity, event_tx, cmd_rx, config).await;
    });
}

#[cfg(target_arch = "wasm32")]
#[allow(deprecated)]
async fn run_network_wasm(
    identity: willow_identity::Identity,
    event_tx: std::sync::mpsc::Sender<NetworkEvent>,
    cmd_rx: std::sync::mpsc::Receiver<NetworkCommand>,
    config: willow_network::NetworkConfig,
) -> anyhow::Result<()> {
    use futures::StreamExt;
    use willow_network::{NetworkEvent as NetEvt, NetworkNode};

    let (node, mut events) = NetworkNode::start(identity, config).await?;
    let mut file_mgr = crate::files::FileManager::new();

    // Create a 16ms interval for polling commands (like native's tokio::time::sleep).
    let mut tick = Box::pin(futures::stream::unfold((), |_| async {
        gloo_timers::future::TimeoutFuture::new(16).await;
        Some(((), ()))
    }))
    .fuse();

    loop {
        futures::select! {
            event = events.next() => {
                let Some(event) = event else { break };
                match event {
                    NetEvt::Message { topic, data, source } => {
                        // Try parsing as a profile broadcast.
                        if let Ok((profile, willow_transport::MessageType::Identity)) =
                            willow_transport::unpack_envelope::<willow_identity::UserProfile>(&data)
                        {
                            let _ = event_tx.send(NetworkEvent::ProfileReceived {
                                peer_id: profile.peer_id.to_string(),
                                display_name: profile.display_name,
                            });
                        }
                        // Try new wire format first.
                        else if let Some((wire_msg, signer)) =
                            crate::ops::unpack_wire(&data)
                        {
                            let from = signer.to_string();
                            match wire_msg {
                                crate::ops::WireMessage::Event(event) => {
                                    let _ = event_tx.send(NetworkEvent::EventReceived {
                                        event,
                                        from,
                                    });
                                }
                                crate::ops::WireMessage::SyncRequest { state_hash, topic } => {
                                    let _ = event_tx.send(NetworkEvent::SyncRequested {
                                        state_hash,
                                        from,
                                        topic,
                                    });
                                }
                                crate::ops::WireMessage::SyncBatch { events } => {
                                    let _ = event_tx.send(NetworkEvent::SyncBatchReceived {
                                        events,
                                        from,
                                    });
                                }
                            }
                        }
                        // Fall back to legacy wire format.
                        else if let Some((sync_msg, signer)) =
                            crate::ops::unpack_sync(&data)
                        {
                            let from = signer.to_string();
                            match sync_msg {
                                crate::ops::SyncMessage::Op(stamped_op) => {
                                    if stamped_op.author == from {
                                        let _ = event_tx.send(NetworkEvent::OpReceived {
                                            stamped_op,
                                            from,
                                        });
                                    }
                                }
                                crate::ops::SyncMessage::SyncRequest { latest_hlc, topic } => {
                                    let _ = event_tx.send(NetworkEvent::LegacySyncRequested {
                                        latest_hlc,
                                        from,
                                        topic,
                                    });
                                }
                                crate::ops::SyncMessage::SyncBatch { ops } => {
                                    let _ = event_tx.send(NetworkEvent::LegacySyncBatchReceived {
                                        ops,
                                        from,
                                    });
                                }
                            }
                        } else {
                            let _ = event_tx.send(NetworkEvent::MessageReceived {
                                topic,
                                data,
                                source: source.map(|p| p.to_string()),
                            });
                        }
                    }
                    NetEvt::PeerConnected(peer) => {
                        let _ = event_tx.send(NetworkEvent::PeerConnected(peer.to_string()));
                    }
                    NetEvt::PeerDisconnected(peer) => {
                        let _ = event_tx.send(NetworkEvent::PeerDisconnected(peer.to_string()));
                    }
                    NetEvt::Listening(addr) => {
                        let _ = event_tx.send(NetworkEvent::Listening(addr.to_string()));
                    }
                    _ => {}
                }
            }

            // Poll commands every 16ms so messages get sent promptly.
            _ = tick.next() => {}

            complete => break,
        }

        // Process any queued commands after either arm fires.
        while let Ok(cmd) = cmd_rx.try_recv() {
            handle_network_command(&cmd, &node, &mut file_mgr)?;
        }
    }

    Ok(())
}

// ───── Shared command handler ───────────────────────────────────────────────

#[allow(deprecated)]
fn handle_network_command(
    cmd: &NetworkCommand,
    node: &willow_network::NetworkNode,
    file_mgr: &mut crate::files::FileManager,
) -> anyhow::Result<()> {
    match cmd {
        NetworkCommand::Subscribe(topic) => {
            node.subscribe(topic)?;
        }
        NetworkCommand::Publish { topic, data } => {
            node.publish(topic, data.clone())?;
        }
        NetworkCommand::ShareFile {
            topic,
            filename,
            mime_type,
            data,
        } => {
            if let Some((_manifest, envelope)) =
                file_mgr.share_file(data, filename.clone(), mime_type.clone())
            {
                node.publish(topic, envelope)?;
            }
        }
        NetworkCommand::BroadcastProfile { display_name } => {
            let profile = willow_identity::UserProfile::new(
                willow_identity::PeerId::from(node.peer_id()),
                display_name,
            );
            if let Ok(data) =
                willow_transport::pack_envelope(willow_transport::MessageType::Identity, &profile)
            {
                let _ = node.publish(PROFILE_TOPIC, data);
            }
        }
        NetworkCommand::BroadcastEvent { event, topic } => {
            let identity = willow_identity::Identity::from_ed25519_bytes(
                &crate::storage::load_identity_bytes().unwrap_or_default(),
            )
            .unwrap_or_else(willow_identity::Identity::generate);
            let publish_topic = topic.as_deref().unwrap_or(crate::ops::SERVER_OPS_TOPIC);
            if let Some(data) =
                crate::ops::pack_wire(&crate::ops::WireMessage::Event(event.clone()), &identity)
            {
                let _ = node.publish(publish_topic, data);
            }
        }
        NetworkCommand::RequestSync { state_hash, topic } => {
            let identity = willow_identity::Identity::from_ed25519_bytes(
                &crate::storage::load_identity_bytes().unwrap_or_default(),
            )
            .unwrap_or_else(willow_identity::Identity::generate);
            let msg = crate::ops::WireMessage::SyncRequest {
                state_hash: state_hash.clone(),
                topic: topic.clone(),
            };
            if let Some(data) = crate::ops::pack_wire(&msg, &identity) {
                let publish_topic = topic.as_deref().unwrap_or(crate::ops::SERVER_OPS_TOPIC);
                let _ = node.publish(publish_topic, data);
            }
        }
        NetworkCommand::SendSyncBatch { events } => {
            let identity = willow_identity::Identity::from_ed25519_bytes(
                &crate::storage::load_identity_bytes().unwrap_or_default(),
            )
            .unwrap_or_else(willow_identity::Identity::generate);
            let msg = crate::ops::WireMessage::SyncBatch {
                events: events.clone(),
            };
            if let Some(data) = crate::ops::pack_wire(&msg, &identity) {
                let _ = node.publish(crate::ops::SERVER_OPS_TOPIC, data);
            }
        }
        // Legacy command handlers (kept for willow-app backward compat).
        NetworkCommand::BroadcastOp(stamped_op) => {
            let identity = willow_identity::Identity::from_ed25519_bytes(
                &crate::storage::load_identity_bytes().unwrap_or_default(),
            )
            .unwrap_or_else(willow_identity::Identity::generate);
            let publish_topic = match &stamped_op.op {
                crate::ops::Op::ChatMessage { topic, .. } => topic.clone(),
                _ => crate::ops::SERVER_OPS_TOPIC.to_string(),
            };
            if let Some(data) =
                crate::ops::pack_sync(&crate::ops::SyncMessage::Op(stamped_op.clone()), &identity)
            {
                let _ = node.publish(&publish_topic, data);
            }
        }
        NetworkCommand::LegacyRequestSync { latest_hlc, topic } => {
            let identity = willow_identity::Identity::from_ed25519_bytes(
                &crate::storage::load_identity_bytes().unwrap_or_default(),
            )
            .unwrap_or_else(willow_identity::Identity::generate);
            let msg = crate::ops::SyncMessage::SyncRequest {
                latest_hlc: *latest_hlc,
                topic: topic.clone(),
            };
            if let Some(data) = crate::ops::pack_sync(&msg, &identity) {
                let publish_topic = topic.as_deref().unwrap_or(crate::ops::SERVER_OPS_TOPIC);
                let _ = node.publish(publish_topic, data);
            }
        }
        NetworkCommand::LegacySendSyncBatch { ops } => {
            let identity = willow_identity::Identity::from_ed25519_bytes(
                &crate::storage::load_identity_bytes().unwrap_or_default(),
            )
            .unwrap_or_else(willow_identity::Identity::generate);
            let msg = crate::ops::SyncMessage::SyncBatch { ops: ops.clone() };
            if let Some(data) = crate::ops::pack_sync(&msg, &identity) {
                let _ = node.publish(crate::ops::SERVER_OPS_TOPIC, data);
            }
        }
    }
    Ok(())
}
