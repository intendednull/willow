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
    /// An event was received from a peer.
    EventReceived {
        event: willow_state::Event,
        from: String,
    },
    /// A sync request was received from a peer.
    SyncRequested {
        state_hash: willow_state::StateHash,
        from: String,
        topic: Option<String>,
    },
    /// A batch of events was received as a sync response.
    SyncBatchReceived {
        events: Vec<willow_state::Event>,
        from: String,
    },
    /// A typing indicator was received from a peer.
    TypingReceived {
        peer_id: String,
        channel: String,
    },
    /// A peer joined a voice channel.
    VoiceJoinReceived {
        /// The voice channel that was joined.
        channel_id: String,
        /// The peer who joined.
        peer_id: String,
    },
    /// A peer left a voice channel.
    VoiceLeaveReceived {
        /// The voice channel that was left.
        channel_id: String,
        /// The peer who left.
        peer_id: String,
    },
    /// A voice signaling message was received (targeted at us).
    VoiceSignalReceived {
        /// The voice channel this signal relates to.
        channel_id: String,
        /// The peer who sent the signal.
        from_peer: String,
        /// The signaling payload.
        signal: crate::ops::VoiceSignalPayload,
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
    /// Broadcast a typing indicator on the server ops topic.
    SendTyping {
        channel: String,
    },
    /// Broadcast an event.
    BroadcastEvent {
        event: willow_state::Event,
        topic: Option<String>,
    },
    /// Request missing events from peers.
    RequestSync {
        state_hash: willow_state::StateHash,
        topic: Option<String>,
    },
    /// Send a batch of events as a sync response.
    SendSyncBatch {
        events: Vec<willow_state::Event>,
    },
    /// Broadcast a voice join to all peers.
    SendVoiceJoin {
        /// The voice channel being joined.
        channel_id: String,
    },
    /// Broadcast a voice leave to all peers.
    SendVoiceLeave {
        /// The voice channel being left.
        channel_id: String,
    },
    /// Send a voice signaling message to a specific peer.
    SendVoiceSignal {
        /// The voice channel this signal relates to.
        channel_id: String,
        /// The intended recipient peer.
        target_peer: String,
        /// The signaling payload.
        signal: crate::ops::VoiceSignalPayload,
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
    event_tx: futures::channel::mpsc::UnboundedSender<NetworkEvent>,
    cmd_rx: futures::channel::mpsc::UnboundedReceiver<NetworkCommand>,
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
async fn run_network(
    identity: willow_identity::Identity,
    event_tx: futures::channel::mpsc::UnboundedSender<NetworkEvent>,
    mut cmd_rx: futures::channel::mpsc::UnboundedReceiver<NetworkCommand>,
    config: willow_network::NetworkConfig,
) -> anyhow::Result<()> {
    use futures::StreamExt;
    use willow_network::{file_transfer::ChunkResponse, NetworkEvent as NetEvt, NetworkNode};

    let (node, mut events) = NetworkNode::start(identity, config).await?;
    let mut file_mgr = crate::files::FileManager::new();
    let local_peer_id = node.peer_id().to_string();

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
                            let _ = event_tx.unbounded_send(NetworkEvent::FileAnnounced {
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
                            let _ = event_tx.unbounded_send(NetworkEvent::ProfileReceived {
                                peer_id: profile.peer_id.to_string(),
                                display_name: profile.display_name,
                            });
                        }
                        // Try wire format.
                        else if let Some((wire_msg, signer)) =
                            crate::ops::unpack_wire(&data)
                        {
                            let from = signer.to_string();
                            match wire_msg {
                                crate::ops::WireMessage::Event(event) => {
                                    let _ = event_tx.unbounded_send(NetworkEvent::EventReceived {
                                        event,
                                        from,
                                    });
                                }
                                crate::ops::WireMessage::SyncRequest { state_hash, topic } => {
                                    let _ = event_tx.unbounded_send(NetworkEvent::SyncRequested {
                                        state_hash,
                                        from,
                                        topic,
                                    });
                                }
                                crate::ops::WireMessage::SyncBatch { events } => {
                                    let _ = event_tx.unbounded_send(NetworkEvent::SyncBatchReceived {
                                        events,
                                        from,
                                    });
                                }
                                crate::ops::WireMessage::TypingIndicator { channel } => {
                                    let _ = event_tx.unbounded_send(NetworkEvent::TypingReceived {
                                        peer_id: from,
                                        channel,
                                    });
                                }
                                crate::ops::WireMessage::VoiceJoin { channel_id, peer_id } => {
                                    let _ = event_tx.unbounded_send(NetworkEvent::VoiceJoinReceived {
                                        channel_id,
                                        peer_id,
                                    });
                                }
                                crate::ops::WireMessage::VoiceLeave { channel_id, peer_id } => {
                                    let _ = event_tx.unbounded_send(NetworkEvent::VoiceLeaveReceived {
                                        channel_id,
                                        peer_id,
                                    });
                                }
                                crate::ops::WireMessage::VoiceSignal { channel_id, target_peer, signal } => {
                                    if target_peer == local_peer_id {
                                        let _ = event_tx.unbounded_send(NetworkEvent::VoiceSignalReceived {
                                            channel_id,
                                            from_peer: from,
                                            signal,
                                        });
                                    }
                                }
                            }
                        } else {
                            let _ = event_tx.unbounded_send(NetworkEvent::MessageReceived {
                                topic,
                                data,
                                source: source.map(|p| p.to_string()),
                            });
                        }
                    }
                    NetEvt::PeerConnected(peer) => {
                        let _ = event_tx.unbounded_send(NetworkEvent::PeerConnected(peer.to_string()));
                    }
                    NetEvt::PeerDisconnected(peer) => {
                        let _ = event_tx.unbounded_send(NetworkEvent::PeerDisconnected(peer.to_string()));
                    }
                    NetEvt::Listening(addr) => {
                        let _ = event_tx.unbounded_send(NetworkEvent::Listening(addr.to_string()));
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

            cmd = cmd_rx.next() => {
                if let Some(cmd) = cmd {
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
    event_tx: futures::channel::mpsc::UnboundedSender<NetworkEvent>,
    cmd_rx: futures::channel::mpsc::UnboundedReceiver<NetworkCommand>,
    config: willow_network::NetworkConfig,
) {
    wasm_bindgen_futures::spawn_local(async move {
        let _ = run_network_wasm(identity, event_tx, cmd_rx, config).await;
    });
}

#[cfg(target_arch = "wasm32")]
async fn run_network_wasm(
    identity: willow_identity::Identity,
    event_tx: futures::channel::mpsc::UnboundedSender<NetworkEvent>,
    mut cmd_rx: futures::channel::mpsc::UnboundedReceiver<NetworkCommand>,
    config: willow_network::NetworkConfig,
) -> anyhow::Result<()> {
    use futures::StreamExt;
    use willow_network::{NetworkEvent as NetEvt, NetworkNode};

    let (node, mut events) = NetworkNode::start(identity, config).await?;
    let mut file_mgr = crate::files::FileManager::new();
    let local_peer_id = node.peer_id().to_string();

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
                            let _ = event_tx.unbounded_send(NetworkEvent::ProfileReceived {
                                peer_id: profile.peer_id.to_string(),
                                display_name: profile.display_name,
                            });
                        }
                        // Try wire format.
                        else if let Some((wire_msg, signer)) =
                            crate::ops::unpack_wire(&data)
                        {
                            let from = signer.to_string();
                            match wire_msg {
                                crate::ops::WireMessage::Event(event) => {
                                    let _ = event_tx.unbounded_send(NetworkEvent::EventReceived {
                                        event,
                                        from,
                                    });
                                }
                                crate::ops::WireMessage::SyncRequest { state_hash, topic } => {
                                    let _ = event_tx.unbounded_send(NetworkEvent::SyncRequested {
                                        state_hash,
                                        from,
                                        topic,
                                    });
                                }
                                crate::ops::WireMessage::SyncBatch { events } => {
                                    let _ = event_tx.unbounded_send(NetworkEvent::SyncBatchReceived {
                                        events,
                                        from,
                                    });
                                }
                                crate::ops::WireMessage::TypingIndicator { channel } => {
                                    let _ = event_tx.unbounded_send(NetworkEvent::TypingReceived {
                                        peer_id: from,
                                        channel,
                                    });
                                }
                                crate::ops::WireMessage::VoiceJoin { channel_id, peer_id } => {
                                    let _ = event_tx.unbounded_send(NetworkEvent::VoiceJoinReceived {
                                        channel_id,
                                        peer_id,
                                    });
                                }
                                crate::ops::WireMessage::VoiceLeave { channel_id, peer_id } => {
                                    let _ = event_tx.unbounded_send(NetworkEvent::VoiceLeaveReceived {
                                        channel_id,
                                        peer_id,
                                    });
                                }
                                crate::ops::WireMessage::VoiceSignal { channel_id, target_peer, signal } => {
                                    if target_peer == local_peer_id {
                                        let _ = event_tx.unbounded_send(NetworkEvent::VoiceSignalReceived {
                                            channel_id,
                                            from_peer: from,
                                            signal,
                                        });
                                    }
                                }
                            }
                        } else {
                            let _ = event_tx.unbounded_send(NetworkEvent::MessageReceived {
                                topic,
                                data,
                                source: source.map(|p| p.to_string()),
                            });
                        }
                    }
                    NetEvt::PeerConnected(peer) => {
                        let _ = event_tx.unbounded_send(NetworkEvent::PeerConnected(peer.to_string()));
                    }
                    NetEvt::PeerDisconnected(peer) => {
                        let _ = event_tx.unbounded_send(NetworkEvent::PeerDisconnected(peer.to_string()));
                    }
                    NetEvt::Listening(addr) => {
                        let _ = event_tx.unbounded_send(NetworkEvent::Listening(addr.to_string()));
                    }
                    _ => {}
                }
            }

            cmd = cmd_rx.next() => {
                if let Some(cmd) = cmd {
                    handle_network_command(&cmd, &node, &mut file_mgr)?;
                }
            }

            complete => break,
        }
    }

    Ok(())
}

// ───── Shared command handler ───────────────────────────────────────────────

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
        NetworkCommand::SendTyping { channel } => {
            let identity = willow_identity::Identity::from_ed25519_bytes(
                &crate::storage::load_identity_bytes().unwrap_or_default(),
            )
            .unwrap_or_else(willow_identity::Identity::generate);
            let msg = crate::ops::WireMessage::TypingIndicator {
                channel: channel.clone(),
            };
            if let Some(data) = crate::ops::pack_wire(&msg, &identity) {
                let _ = node.publish(crate::ops::SERVER_OPS_TOPIC, data);
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
        NetworkCommand::SendVoiceJoin { channel_id } => {
            let identity = willow_identity::Identity::from_ed25519_bytes(
                &crate::storage::load_identity_bytes().unwrap_or_default(),
            )
            .unwrap_or_else(willow_identity::Identity::generate);
            let msg = crate::ops::WireMessage::VoiceJoin {
                channel_id: channel_id.clone(),
                peer_id: willow_identity::PeerId::from(node.peer_id()).to_string(),
            };
            if let Some(data) = crate::ops::pack_wire(&msg, &identity) {
                let _ = node.publish(crate::ops::SERVER_OPS_TOPIC, data);
            }
        }
        NetworkCommand::SendVoiceLeave { channel_id } => {
            let identity = willow_identity::Identity::from_ed25519_bytes(
                &crate::storage::load_identity_bytes().unwrap_or_default(),
            )
            .unwrap_or_else(willow_identity::Identity::generate);
            let msg = crate::ops::WireMessage::VoiceLeave {
                channel_id: channel_id.clone(),
                peer_id: willow_identity::PeerId::from(node.peer_id()).to_string(),
            };
            if let Some(data) = crate::ops::pack_wire(&msg, &identity) {
                let _ = node.publish(crate::ops::SERVER_OPS_TOPIC, data);
            }
        }
        NetworkCommand::SendVoiceSignal {
            channel_id,
            target_peer,
            signal,
        } => {
            let identity = willow_identity::Identity::from_ed25519_bytes(
                &crate::storage::load_identity_bytes().unwrap_or_default(),
            )
            .unwrap_or_else(willow_identity::Identity::generate);
            let msg = crate::ops::WireMessage::VoiceSignal {
                channel_id: channel_id.clone(),
                target_peer: target_peer.clone(),
                signal: signal.clone(),
            };
            if let Some(data) = crate::ops::pack_wire(&msg, &identity) {
                let _ = node.publish(crate::ops::SERVER_OPS_TOPIC, data);
            }
        }
    }
    Ok(())
}
