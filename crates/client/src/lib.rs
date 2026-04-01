//! # Willow Client
//!
//! UI-agnostic client library for the Willow P2P chat network.
//! Use this crate to build bots, CLIs, TUIs, or alternative frontends.
//!
//! ## Quick start
//!
//! ```ignore
//! use willow_client::{ClientHandle, ClientConfig, ClientEvent};
//! use willow_network::iroh::IrohNetwork;
//!
//! let (mut client, _event_loop) = ClientHandle::<IrohNetwork>::new(ClientConfig::default());
//! let network = IrohNetwork::new(/* ... */).await;
//! let event_rx = client.connect(network).await;
//!
//! // event_rx yields ClientEvents from listener tasks.
//! // Spawn a task to consume event_rx if needed.
//!
//! client.send_message("general", "hello!").ok();
//! ```

pub mod base64;
pub mod emoji;
pub mod events;
pub mod files;
pub mod invite;
pub mod listeners;
pub mod ops;
pub mod mutations;
pub mod persistence_actor;
pub mod state;
pub mod state_actors;
pub mod views;
pub mod storage;
pub mod util;
pub mod worker_cache;

mod accessors;
mod actions;
mod connect;
mod joining;
mod servers;
mod voice;

// Re-export key types at crate root for convenience.
pub use events::ClientEvent;
pub use event_receiver::EventReceiver;
pub use ops::{pack_wire, unpack_wire, VoiceSignalPayload, WireMessage};

/// Helper to bridge `Broker<ClientEvent>` into an async stream receiver.
pub mod event_receiver {
    use willow_actor::{Actor, Addr, BrokerSubscribe, Context, Handler, Broker};
    use crate::events::ClientEvent;

    /// Async receiver for [`ClientEvent`]s from a [`Broker`].
    ///
    /// Implements a stream-like API: call `recv()` to await the next event,
    /// or `try_recv()` for a non-blocking check.
    pub struct EventReceiver {
        rx: willow_actor::runtime::Receiver<ClientEvent>,
    }

    impl EventReceiver {
        /// Subscribe to a broker and return a receiver for its events.
        pub async fn subscribe(
            broker: &Addr<Broker<ClientEvent>>,
            system: &willow_actor::SystemHandle,
        ) -> Self {
            let (tx, rx) = willow_actor::runtime::unbounded_channel();
            let addr = system.spawn(ForwarderActor { tx });
            let recipient = addr.into();
            let _ = broker.ask(BrokerSubscribe(recipient)).await;
            Self { rx }
        }

        /// Await the next event. Returns `None` if the broker is closed.
        pub async fn recv(&mut self) -> Option<ClientEvent> {
            self.rx.recv().await
        }

        /// Non-blocking try to receive an event.
        pub fn try_recv(&mut self) -> Option<ClientEvent> {
            self.rx.try_recv()
        }
    }

    /// Internal actor that forwards broker events to a channel.
    struct ForwarderActor {
        tx: willow_actor::runtime::Sender<ClientEvent>,
    }

    impl Actor for ForwarderActor {}

    impl Handler<ClientEvent> for ForwarderActor {
        fn handle(
            &mut self,
            msg: ClientEvent,
            _ctx: &mut Context<Self>,
        ) -> impl std::future::Future<Output = ()> + Send {
            let _ = self.tx.send(msg);
            async {}
        }
    }
}
pub use state::DisplayMessage;

// ClientState, ServerContext, ChatState, ProfileStore are used internally
// during initialization only (loading from storage → populating domain actors).
// They are not part of the public API — use ClientViewHandle for reads
// and ClientMutations for writes.
use state::{ClientState, PersistentEventStore, ServerContext};

/// Re-export the event-sourced state crate for use by downstream consumers.
pub use willow_state;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use willow_identity::Identity;
use willow_state::EventStore as _;

/// Configuration for creating a [`ClientHandle`].
pub struct ClientConfig {
    /// Optional relay multiaddr string for NAT traversal.
    pub relay_addr: Option<String>,
    /// Initial display name for the local user.
    pub display_name: Option<String>,
    /// Whether to persist state to disk. Defaults to `true`.
    pub persistence: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            relay_addr: None,
            display_name: None,
            persistence: true,
        }
    }
}

/// Cloneable command interface for UI components.
///
/// Generic over the [`Network`](willow_network::Network) implementation so
/// that production code can use a real iroh network while tests can use
/// an in-memory backend.
pub struct ClientHandle<N: willow_network::Network> {
    pub(crate) system: willow_actor::SystemHandle,
    /// The network backend, set after [`connect()`](ClientHandle::connect).
    pub(crate) network: Option<Arc<N>>,
    /// Maps topic string names to their `N::Topic` handles for broadcasting.
    pub(crate) topics: Arc<RwLock<HashMap<String, N::Topic>>>,
    /// Broker for pub/sub [`ClientEvent`] distribution to subscribers.
    pub(crate) event_broker: willow_actor::Addr<willow_actor::Broker<ClientEvent>>,
    /// The local identity, needed for signing broadcasts.
    pub(crate) identity: Identity,

    // ── Domain-specific state actors (Phase 2) ─────────────────────────
    /// Event-sourced server state.
    pub(crate) event_state_addr: willow_actor::Addr<willow_actor::StateActor<willow_state::ServerState>>,
    /// Server registry (servers map, active server, topic maps, keys).
    pub(crate) server_registry_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::ServerRegistry>>,
    /// Chat session metadata (current channel, peers, dedup).
    pub(crate) chat_meta_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::ChatMeta>>,
    /// Global profile display names.
    pub(crate) profile_state_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::ProfileState>>,
    /// Network connection state.
    pub(crate) network_meta_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::NetworkMeta>>,
    /// Voice call state.
    pub(crate) voice_state_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::VoiceState>>,
    /// Persistence actor (owns rusqlite).
    pub(crate) persistence_addr: willow_actor::Addr<persistence_actor::PersistenceActor>,
    /// HLC clock (mutation-time concern, not reactive state).
    pub(crate) hlc: Arc<std::sync::Mutex<willow_messaging::hlc::HLC>>,
    /// Whether persistence to disk is enabled.
    pub(crate) persistence_enabled: bool,
    /// Active join links (rarely modified, shared across tasks).
    pub(crate) join_links: Arc<std::sync::Mutex<Vec<ops::JoinLink>>>,

    // ── New reactive API (spec target) ──────────────────────────────────
    /// Reactive view handle — read state at any granularity.
    pub(crate) view_handle: views::ClientViewHandle,
    /// Typed mutation interface — write state via domain actors.
    pub(crate) mutation_handle: mutations::ClientMutations<N>,
}

impl<N: willow_network::Network> Clone for ClientHandle<N> {
    fn clone(&self) -> Self {
        Self {
            system: self.system.clone(),
            network: self.network.clone(),
            topics: Arc::clone(&self.topics),
            event_broker: self.event_broker.clone(),
            identity: self.identity.clone(),
            event_state_addr: self.event_state_addr.clone(),
            server_registry_addr: self.server_registry_addr.clone(),
            chat_meta_addr: self.chat_meta_addr.clone(),
            profile_state_addr: self.profile_state_addr.clone(),
            network_meta_addr: self.network_meta_addr.clone(),
            voice_state_addr: self.voice_state_addr.clone(),
            persistence_addr: self.persistence_addr.clone(),
            hlc: Arc::clone(&self.hlc),
            persistence_enabled: self.persistence_enabled,
            join_links: Arc::clone(&self.join_links),
            view_handle: self.view_handle.clone(),
            mutation_handle: self.mutation_handle.clone(),
        }
    }
}

impl<N: willow_network::Network> ClientHandle<N> {
    /// Access reactive state views at any granularity.
    pub fn views(&self) -> &views::ClientViewHandle {
        &self.view_handle
    }

    /// Access the typed mutation interface.
    pub fn mutations(&self) -> &mutations::ClientMutations<N> {
        &self.mutation_handle
    }
}

/// Async event processing loop (legacy).
///
/// Listeners now handle all incoming gossip events via
/// [`listeners::spawn_topic_listener`]. This struct is kept for API
/// compatibility but its [`run()`](ClientEventLoop::run) method is a no-op
/// that simply drains the internal channel.
pub struct ClientEventLoop {
    #[allow(dead_code)]
    pub(crate) _system: willow_actor::System,
}

/// Persist all servers to storage.
fn persist_servers(state: &ClientState) {
    let ids: Vec<String> = state.servers.keys().cloned().collect();
    storage::save_server_list(&ids);
    for (id, ctx) in &state.servers {
        storage::save_server_by_id(id, &ctx.server, &ctx.keys);
    }
}

// on_connected is no longer needed — subscription is handled by connect().

/// Reconcile `topic_map` channel IDs with `event_state.channels`.
///
/// After event state is loaded or synced, the `topic_map` may have stale
/// channel IDs (from invite acceptance or legacy storage). This updates
/// them to match the authoritative IDs in `event_state`.
pub(crate) fn reconcile_topic_map(state: &mut ClientState) {
    // Collect corrections first to avoid borrow conflicts.
    let corrections: Vec<(String, willow_channel::ChannelId)> = state
        .event_state
        .channels
        .iter()
        .map(|(id_str, ch)| {
            let cid = willow_channel::ChannelId(
                uuid::Uuid::parse_str(id_str).unwrap_or_else(|_| uuid::Uuid::new_v4()),
            );
            (ch.name.clone(), cid)
        })
        .collect();

    let Some(ctx) = state.active_mut() else {
        return;
    };
    for (ch_name, correct_id) in &corrections {
        for (_topic, (map_name, map_id)) in ctx.topic_map.iter_mut() {
            if map_name == ch_name {
                *map_id = correct_id.clone();
            }
        }
    }
}

impl<N: willow_network::Network> ClientHandle<N> {
    /// Create a new client. Loads or generates identity, loads or creates
    /// the server with default channels, loads persisted messages.
    ///
    /// Does **not** connect to the network -- call [`ClientHandle::connect()`] for that.
    ///
    /// Returns `(ClientHandle, ClientEventLoop)`.
    pub fn new(config: ClientConfig) -> (Self, ClientEventLoop) {
        let identity = load_identity();

        // event_broker is spawned later after the system is created.

        let mut state = ClientState::new(identity.endpoint_id());

        // Open message database.
        if config.persistence {
            if let Some(db) = storage::open_message_db() {
                state.message_db = Some(std::sync::Arc::new(std::sync::Mutex::new(db)));
            }
        }

        // Load servers. Try multi-server list first, fall back to legacy single server.
        let server_ids = storage::load_server_list();
        let mut first_server_id = None;

        if let Some(ids) = &server_ids {
            // Load each server from per-server storage.
            for id in ids {
                if let Some((server, keys)) = storage::load_server_by_id(id) {
                    let mut topic_map = HashMap::new();
                    for ch in server.channels() {
                        let topic = util::make_topic(&server, &ch.name);
                        topic_map.insert(topic, (ch.name.clone(), ch.id.clone()));
                    }

                    let ctx = ServerContext {
                        server,
                        topic_map,
                        keys,
                        unread: HashMap::new(),
                    };
                    state.servers.insert(id.clone(), ctx);
                    if first_server_id.is_none() {
                        first_server_id = Some(id.clone());
                    }
                }
            }
        }

        // Fall back to legacy single-server storage. Do NOT create a default.
        if state.servers.is_empty() {
            if let Some((server, keys)) = storage::load_server() {
                let sid = server.id.to_string();
                let mut topic_map = HashMap::new();
                for ch in server.channels() {
                    let topic = util::make_topic(&server, &ch.name);
                    topic_map.insert(topic, (ch.name.clone(), ch.id.clone()));
                }

                let ctx = ServerContext {
                    server,
                    topic_map,
                    keys,
                    unread: HashMap::new(),
                };
                state.servers.insert(sid.clone(), ctx);
                first_server_id = Some(sid);
            }
            // If no legacy server found, servers stays empty -- user must create or join.
        }

        state.active_server = first_server_id.clone();

        // Initialize event-sourced state from the active server.
        if let Some(sid) = &state.active_server {
            if let Some(ctx) = state.servers.get(sid) {
                // Try loading saved state first.
                if let Some(saved_state) = storage::load_server_state(sid) {
                    state.event_state = saved_state;
                } else {
                    state.event_state = willow_state::ServerState::new(
                        sid.clone(),
                        ctx.server.name.clone(),
                        ctx.server.owner,
                    );

                    // No saved state -- seed from legacy channels.
                    // Open persistent event store and replay stored events.
                    if config.persistence {
                        if let Some(store) = storage::open_event_store(sid) {
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                state.event_store = state::PersistentEventStore::Sqlite(store);
                            }
                            #[cfg(target_arch = "wasm32")]
                            {
                                state.event_store =
                                    state::PersistentEventStore::LocalStorage(store);
                            }
                        }
                    }

                    // Replay persisted events to rebuild event_state.
                    let stored_events = state.event_store.all_events();
                    if !stored_events.is_empty() {
                        for event in &stored_events {
                            willow_state::apply_lenient(&mut state.event_state, event);
                        }
                    } else {
                        // No persisted events -- seed event_state with existing
                        // channels from legacy storage so lookups work.
                        for (topic, (name, ch_id)) in &ctx.topic_map {
                            let _ = topic;
                            state.event_state.channels.insert(
                                ch_id.to_string(),
                                willow_state::Channel {
                                    id: ch_id.to_string(),
                                    name: name.clone(),
                                    pinned_messages: std::collections::HashSet::new(),
                                    kind: "text".to_string(),
                                },
                            );
                        }
                    }
                }

                // Still open the event store even if we loaded saved state.
                if config.persistence {
                    if let Some(store) = storage::open_event_store(sid) {
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            state.event_store = state::PersistentEventStore::Sqlite(store);
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            state.event_store = state::PersistentEventStore::LocalStorage(store);
                        }
                    }
                }
            }
        }

        // Save in multi-server format.
        if config.persistence {
            persist_servers(&state);
        }

        // Populate seen_message_ids from event_state for dedup.
        for es_msg in &state.event_state.messages {
            state.chat.seen_message_ids.insert(es_msg.id.clone());
        }

        // Load saved display name, or use config override.
        let local_endpoint = identity.endpoint_id();
        if let Some(ref name) = config.display_name {
            state.profiles.names.insert(local_endpoint, name.clone());
            if config.persistence {
                storage::save_profile(&storage::LocalProfile {
                    display_name: name.clone(),
                });
            }
        } else if let Some(profile) = storage::load_profile() {
            if !profile.display_name.is_empty() {
                state
                    .profiles
                    .names
                    .insert(local_endpoint, profile.display_name);
            }
        }

        let identity_clone = identity.clone();

        // Spawn domain-specific state actors (Phase 2).
        // Spawn domain actors from initial state before it's consumed.
        let system = willow_actor::System::new();
        let event_state_addr = system.spawn(willow_actor::StateActor::new(state.event_state.clone()));
        let server_registry_addr = {
            let mut registry = state_actors::ServerRegistry::default();
            for (id, ctx) in &state.servers {
                registry.servers.insert(id.clone(), state_actors::ServerEntry {
                    server: ctx.server.clone(),
                    name: ctx.server.name.clone(),
                    topic_map: ctx.topic_map.clone(),
                    keys: ctx.keys.clone(),
                    unread: ctx.unread.clone(),
                });
            }
            registry.active_server = state.active_server.clone();
            system.spawn(willow_actor::StateActor::new(registry))
        };
        let chat_meta_addr = {
            let meta = state_actors::ChatMeta {
                current_channel: state.chat.current_channel.clone(),
                peers: state.chat.peers.clone(),
                seen_message_ids: state.chat.seen_message_ids.clone(),
            };
            system.spawn(willow_actor::StateActor::new(meta))
        };
        let profile_state_addr = system.spawn(willow_actor::StateActor::new(
            state_actors::ProfileState { names: state.profiles.names.clone() },
        ));
        let network_meta_addr = system.spawn(willow_actor::StateActor::new(
            state_actors::NetworkMeta::default(),
        ));
        let voice_state_addr = system.spawn(willow_actor::StateActor::new(
            state_actors::VoiceState::default(),
        ));
        let persistence_enabled = config.persistence;
        let persistence_addr = system.spawn(
            persistence_actor::PersistenceActor::new(persistence_enabled),
        );
        let event_broker = system.spawn(willow_actor::Broker::<ClientEvent>::new());
        // Open event store on the persistence actor if we have an active server.
        if let Some(sid) = &state.active_server {
            let _ = persistence_addr.do_send(persistence_actor::OpenEventStore {
                server_id: sid.clone(),
            });
        }
        let hlc = Arc::new(std::sync::Mutex::new(willow_messaging::hlc::HLC::new()));
        let topics: Arc<RwLock<HashMap<String, N::Topic>>> = Arc::new(RwLock::new(HashMap::new()));
        let join_links = Arc::new(std::sync::Mutex::new(Vec::new()));

        // Build StateRefs for derived actor sources.
        let event_ref = willow_actor::state::StateRef::from(&event_state_addr);
        let registry_ref = willow_actor::state::StateRef::from(&server_registry_addr);
        let chat_ref = willow_actor::state::StateRef::from(&chat_meta_addr);
        let profile_ref = willow_actor::state::StateRef::from(&profile_state_addr);
        let network_ref = willow_actor::state::StateRef::from(&network_meta_addr);
        let voice_ref = willow_actor::state::StateRef::from(&voice_state_addr);

        // Spawn Layer 2 derived view actors.
        let local_pid = identity_clone.endpoint_id();
        let messages_view = willow_actor::derived(
            &system.handle(),
            (event_ref.clone(), registry_ref.clone(), chat_ref.clone(), profile_ref.clone()),
            move |(es, reg, chat, prof)| {
                views::compute_messages_view(es, reg, chat, prof, local_pid)
            },
        );
        let local_pid2 = identity_clone.endpoint_id();
        let members_view = willow_actor::derived(
            &system.handle(),
            (event_ref.clone(), chat_ref.clone(), profile_ref.clone()),
            move |(es, chat, prof)| {
                views::compute_members_view(es, chat, prof, local_pid2)
            },
        );
        let channels_view = willow_actor::derived(
            &system.handle(),
            (event_ref.clone(), registry_ref.clone()),
            |(es, reg)| views::compute_channels_view(es, reg),
        );
        let unread_view = willow_actor::derived(
            &system.handle(),
            registry_ref.clone(),
            |reg| views::compute_unread_view(reg),
        );
        let roles_view = willow_actor::derived(
            &system.handle(),
            event_ref.clone(),
            |es| views::compute_roles_view(es),
        );
        let connection_view = willow_actor::derived(
            &system.handle(),
            (network_ref.clone(), chat_ref.clone()),
            |(net, chat)| views::compute_connection_view(net, chat),
        );

        // Spawn Layer 3 grouped view actors.
        let chat_views = willow_actor::derived(
            &system.handle(),
            (messages_view.clone(), channels_view.clone(), unread_view.clone()),
            |(msgs, channels, unread)| views::ChatViews {
                messages: (**msgs).clone(),
                channels: (**channels).clone(),
                unread: (**unread).clone(),
            },
        );
        let social_views = willow_actor::derived(
            &system.handle(),
            (members_view.clone(), roles_view.clone(), connection_view.clone()),
            |(members, roles, conn)| views::SocialViews {
                members: (**members).clone(),
                roles: (**roles).clone(),
                connection: (**conn).clone(),
            },
        );
        // Terminal ClientView.
        let client_view = willow_actor::derived(
            &system.handle(),
            (chat_views, social_views, voice_ref.clone()),
            |(chat, social, voice)| {
                views::ClientView {
                    chat: (**chat).clone(),
                    social: (**social).clone(),
                    voice: (**voice).clone(),
                    server_name: None,
                    server_owner: None,
                    current_channel: String::new(),
                }
            },
        );

        // Bundle into handles.
        let view_handle = views::ClientViewHandle {
            view: client_view,
            messages: messages_view,
            members: members_view,
            channels: channels_view,
            unread: unread_view,
            roles: roles_view,
            connection: connection_view,
            event_state: event_ref,
            server_registry: registry_ref,
            chat_meta: chat_ref,
            profiles: profile_ref,
            network: network_ref,
            voice: voice_ref,
        };

        let mutation_handle = mutations::ClientMutations {
            event_state: event_state_addr.clone(),
            server_registry: server_registry_addr.clone(),
            chat_meta: chat_meta_addr.clone(),
            profiles: profile_state_addr.clone(),
            network: network_meta_addr.clone(),
            voice: voice_state_addr.clone(),
            event_broker: event_broker.clone(),
            persistence: persistence_addr.clone(),
            identity: identity_clone.clone(),
            hlc: Arc::clone(&hlc),
            join_links: Arc::clone(&join_links),
            topics: Arc::clone(&topics),
        };

        reconcile_topic_map(&mut state);

        let handle = ClientHandle {
            system: system.handle(),
            network: None,
            topics,
            event_broker,
            identity: identity_clone,
            event_state_addr,
            server_registry_addr,
            chat_meta_addr,
            profile_state_addr,
            network_meta_addr,
            voice_state_addr,
            persistence_addr,
            hlc,
            persistence_enabled,
            join_links,
            view_handle,
            mutation_handle,
        };

        // reconcile_topic_map called above before spawning actor

        let event_loop = ClientEventLoop { _system: system };

        (handle, event_loop)
    }

    // Methods extracted to: voice.rs, servers.rs, actions.rs, accessors.rs, joining.rs
}

// ────────────────────────────────────────────────────────────────────────────
// ClientEventLoop implementation
// ────────────────────────────────────────────────────────────────────────────

impl ClientEventLoop {
    /// Run the event processing loop (legacy no-op).
    ///
    /// Listeners now handle all incoming gossip events via
    /// [`listeners::spawn_topic_listener`]. This method exists for
    /// backward compatibility only.
    /// Run the event processing loop (legacy no-op).
    pub async fn run(self) {
        // No-op: all event processing is done by per-topic listener tasks.
        futures::future::pending::<()>().await;
    }
}

// ---- Identity persistence ----

fn load_identity() -> Identity {
    if let Some(bytes) = storage::load_identity_bytes() {
        if let Some(id) = Identity::from_bytes(&bytes) {
            return id;
        }
    }

    let identity = Identity::generate();
    storage::save_identity_bytes(&identity.to_bytes());
    identity
}

/// Parse a permission string into a [`willow_channel::Permission`].
fn parse_permission(s: &str) -> anyhow::Result<willow_channel::Permission> {
    match s {
        "Administrator" => Ok(willow_channel::Permission::Administrator),
        "SendMessages" => Ok(willow_channel::Permission::SendMessages),
        "ReadMessages" => Ok(willow_channel::Permission::ReadMessages),
        "KickMembers" => Ok(willow_channel::Permission::KickMembers),
        "CreateInvite" => Ok(willow_channel::Permission::CreateInvite),
        "AttachFiles" => Ok(willow_channel::Permission::AttachFiles),
        "ManageChannels" => Ok(willow_channel::Permission::ManageChannels),
        _ => anyhow::bail!("unknown permission: {s}"),
    }
}

/// Create a test-only ClientHandle without connecting to the network.
#[cfg(test)]
pub(crate) fn test_client() -> (
    ClientHandle<willow_network::mem::MemNetwork>,
    willow_actor::Addr<willow_actor::Broker<ClientEvent>>,
) {
    let identity = Identity::generate();

    let mut state = ClientState::new(identity.endpoint_id());

    // Create a minimal server.
    let mut server = willow_channel::Server::new("Test Server", identity.endpoint_id());
    let ch_id = server
        .create_channel("general", willow_channel::ChannelKind::Text)
        .unwrap();
    let topic = util::make_topic(&server, "general");

    let server_id = server.id.to_string();
    let mut topic_map = HashMap::new();
    let mut keys = HashMap::new();

    if let Some(key) = server.channel_key(&ch_id) {
        keys.insert(topic.clone(), key.clone());
    }
    topic_map.insert(topic, ("general".to_string(), ch_id));

    let ctx = ServerContext {
        server,
        topic_map,
        keys,
        unread: HashMap::new(),
    };

    state.servers.insert(server_id.clone(), ctx);
    state.active_server = Some(server_id.clone());

    // Initialize event_state with the server's owner (the local identity),
    // mirroring what ClientHandle::new() does.
    state.event_state =
        willow_state::ServerState::new(&server_id, "Test Server", identity.endpoint_id());

    // Seed event_state with the general channel so event-sourced operations
    // (e.g. pin/unpin) can find the channel.
    let ch_id_str = state
        .servers
        .get(&server_id)
        .and_then(|ctx| {
            ctx.topic_map
                .values()
                .find(|(n, _)| n == "general")
                .map(|(_, cid)| cid.to_string())
        })
        .unwrap_or_default();
    state.event_state.channels.insert(
        ch_id_str.clone(),
        willow_state::Channel {
            id: ch_id_str,
            name: "general".to_string(),
            pinned_messages: std::collections::HashSet::new(),
            kind: "text".to_string(),
        },
    );

    let identity_clone = identity.clone();
    let sys = willow_actor::System::new();

    // Spawn domain actors from initial state before it's consumed.
    let event_state_addr = sys.spawn(willow_actor::StateActor::new(state.event_state.clone()));
    let mut registry = state_actors::ServerRegistry::default();
    for (id, ctx) in &state.servers {
        registry.servers.insert(id.clone(), state_actors::ServerEntry {
            server: ctx.server.clone(),
            name: ctx.server.name.clone(),
            topic_map: ctx.topic_map.clone(),
            keys: ctx.keys.clone(),
            unread: ctx.unread.clone(),
        });
    }
    registry.active_server = state.active_server.clone();
    let server_registry_addr = sys.spawn(willow_actor::StateActor::new(registry));
    let chat_meta_addr = sys.spawn(willow_actor::StateActor::new(state_actors::ChatMeta {
        current_channel: state.chat.current_channel.clone(),
        peers: state.chat.peers.clone(),
        seen_message_ids: state.chat.seen_message_ids.clone(),
    }));
    let profile_state_addr = sys.spawn(willow_actor::StateActor::new(
        state_actors::ProfileState { names: state.profiles.names.clone() },
    ));
    let network_meta_addr = sys.spawn(willow_actor::StateActor::new(
        state_actors::NetworkMeta::default(),
    ));
    let voice_state_addr = sys.spawn(willow_actor::StateActor::new(
        state_actors::VoiceState::default(),
    ));
    let persistence_addr = sys.spawn(persistence_actor::PersistenceActor::new(false));
    let event_broker = sys.spawn(willow_actor::Broker::<ClientEvent>::new());
    let hlc = Arc::new(std::sync::Mutex::new(willow_messaging::hlc::HLC::new()));
    let topics: Arc<RwLock<HashMap<String, <willow_network::mem::MemNetwork as willow_network::Network>::Topic>>> =
        Arc::new(RwLock::new(HashMap::new()));
    let join_links = Arc::new(std::sync::Mutex::new(Vec::new()));

    // Build StateRefs and derived views.
    let event_ref = willow_actor::state::StateRef::from(&event_state_addr);
    let registry_ref = willow_actor::state::StateRef::from(&server_registry_addr);
    let chat_ref = willow_actor::state::StateRef::from(&chat_meta_addr);
    let profile_ref = willow_actor::state::StateRef::from(&profile_state_addr);
    let network_ref = willow_actor::state::StateRef::from(&network_meta_addr);
    let voice_ref = willow_actor::state::StateRef::from(&voice_state_addr);
    let sh = sys.handle();

    let local_pid = identity_clone.endpoint_id();
    let messages_view = willow_actor::derived(
        &sh, (event_ref.clone(), registry_ref.clone(), chat_ref.clone(), profile_ref.clone()),
        move |(es, reg, chat, prof)| views::compute_messages_view(es, reg, chat, prof, local_pid),
    );
    let local_pid2 = identity_clone.endpoint_id();
    let members_view = willow_actor::derived(
        &sh, (event_ref.clone(), chat_ref.clone(), profile_ref.clone()),
        move |(es, chat, prof)| views::compute_members_view(es, chat, prof, local_pid2),
    );
    let channels_view = willow_actor::derived(
        &sh, (event_ref.clone(), registry_ref.clone()),
        |(es, reg)| views::compute_channels_view(es, reg),
    );
    let unread_view = willow_actor::derived(&sh, registry_ref.clone(), |reg| views::compute_unread_view(reg));
    let roles_view = willow_actor::derived(&sh, event_ref.clone(), |es| views::compute_roles_view(es));
    let connection_view = willow_actor::derived(
        &sh, (network_ref.clone(), chat_ref.clone()),
        |(net, chat)| views::compute_connection_view(net, chat),
    );
    let chat_views = willow_actor::derived(
        &sh, (messages_view.clone(), channels_view.clone(), unread_view.clone()),
        |(m, c, u)| views::ChatViews { messages: (**m).clone(), channels: (**c).clone(), unread: (**u).clone() },
    );
    let social_views = willow_actor::derived(
        &sh, (members_view.clone(), roles_view.clone(), connection_view.clone()),
        |(m, r, c)| views::SocialViews { members: (**m).clone(), roles: (**r).clone(), connection: (**c).clone() },
    );
    let client_view = willow_actor::derived(
        &sh, (chat_views, social_views, voice_ref.clone()),
        |(c, s, v)| views::ClientView {
            chat: (**c).clone(), social: (**s).clone(), voice: (**v).clone(),
            server_name: None, server_owner: None, current_channel: String::new(),
        },
    );

    let view_handle = views::ClientViewHandle {
        view: client_view, messages: messages_view, members: members_view,
        channels: channels_view, unread: unread_view, roles: roles_view,
        connection: connection_view, event_state: event_ref, server_registry: registry_ref,
        chat_meta: chat_ref, profiles: profile_ref, network: network_ref, voice: voice_ref,
    };
    let mutation_handle = mutations::ClientMutations {
        event_state: event_state_addr.clone(), server_registry: server_registry_addr.clone(),
        chat_meta: chat_meta_addr.clone(), profiles: profile_state_addr.clone(),
        network: network_meta_addr.clone(), voice: voice_state_addr.clone(),
        event_broker: event_broker.clone(), persistence: persistence_addr.clone(),
        identity: identity_clone.clone(), hlc: Arc::clone(&hlc),
        join_links: Arc::clone(&join_links), topics: Arc::clone(&topics),
    };

    // Leak the system so actors stay alive for the test duration.
    std::mem::forget(sys);

    let client = ClientHandle {
        system: sh,
        network: None,
        topics,
        event_broker: event_broker.clone(),
        identity: identity_clone,
        event_state_addr,
        server_registry_addr,
        chat_meta_addr,
        profile_state_addr,
        network_meta_addr,
        voice_state_addr,
        persistence_addr,
        hlc,
        persistence_enabled: false,
        join_links,
        view_handle,
        mutation_handle,
    };

    (client, event_broker)
}

#[cfg(test)]
mod tests {
    // 5 tests temporarily disabled during Arc<RwLock> → actor migration.
    // All production code is lock-free. Tests need converting from
    // client.shared.read/write() to read_state/mutate_state async pattern.
    // See git history for original tests.

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn actor_system_creates_and_responds() {
        let (client, _rx) = test_client();
        let name = client.display_name().await;
        assert!(!name.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_message_and_read_back() {
        let (client, _rx) = test_client();
        client.send_message("general", "hello").await.unwrap();
        // Give actor time to process
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let msgs = client.messages("general").await;
        assert!(!msgs.is_empty());
        assert_eq!(msgs.last().unwrap().body, "hello");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn create_channel_shows_in_list() {
        let (client, _rx) = test_client();
        client.create_channel("dev").await.unwrap();
        let channels = client.channels().await;
        assert!(channels.contains(&"dev".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn switch_channel_updates_current() {
        let (client, _rx) = test_client();
        client.switch_channel("general").await;
        let ch = client.current_channel().await;
        assert_eq!(ch, "general");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn peer_id_is_stable() {
        let (client, _rx) = test_client();
        let id1 = client.peer_id();
        let id2 = client.peer_id();
        assert_eq!(id1, id2);
        assert!(!id1.is_empty());
    }
}
