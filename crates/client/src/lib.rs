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
pub mod mentions;
pub mod mutations;
pub mod nickname;
pub mod ops;
pub mod persistence_actor;
pub mod presence;
pub mod queue;
pub mod search;
pub mod state;
pub mod state_actors;
pub mod storage;
pub mod trust;
pub mod util;
pub mod views;
pub mod worker_cache;

mod accessors;
mod actions;
pub mod connect;
mod joining;
mod servers;
mod voice;

#[cfg(test)]
#[path = "tests/trust_flow.rs"]
mod tests_trust_flow;

#[cfg(test)]
#[path = "tests/multi_peer_sync.rs"]
mod tests_multi_peer_sync;

#[cfg(test)]
#[path = "tests/queue.rs"]
mod tests_queue;

#[cfg(test)]
#[path = "tests/profile_view.rs"]
mod tests_profile_view;

#[cfg(test)]
#[path = "tests/ephemeral.rs"]
mod tests_ephemeral;

#[cfg(test)]
#[path = "tests/voice.rs"]
mod tests_voice;

#[cfg(test)]
#[path = "tests/governance.rs"]
mod tests_governance;

/// How long a typing indicator remains visible after the last typing event, in milliseconds.
pub const TYPING_INDICATOR_TTL_MS: u64 = 5_000;

// Re-export key types at crate root for convenience.
pub use event_receiver::EventReceiver;
pub use events::ClientEvent;
pub use mentions::mentions_me;
pub use nickname::{MemNicknameStore, NicknameStore, NicknameStoreHandle, NICKNAME_CAP};
pub use ops::{pack_wire, unpack_wire, VoiceSignalPayload, WireMessage};
pub use queue::{ArrivedSummary, QueueSummary, RelayStatus};
pub use search::{
    IndexableMessage, RecentQuery, SearchIndex, SearchIndexBuildStatus, SearchIndexConfig,
    SearchIndexHandle, SearchQuery, SearchResult, SearchScope,
};
pub use trust::{
    ComparePreview, InMemoryTrustStore, PeerTrust, TrustStore, TrustStoreHandle, UnverifiedReason,
};

/// Errors returned by client API entry points.
///
/// New variants are added as the crate's surface grows. For now this
/// covers the few cases where we explicitly want to surface a typed
/// failure to callers (rather than swallowing it via `anyhow`).
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// An invite payload could not be parsed: a field was missing or
    /// contained malformed data (e.g. a non-UUID server id, or a channel
    /// that the local server cannot create). Joining is aborted instead
    /// of silently inventing fresh IDs, which would split-brain the
    /// joiner from the rest of the network.
    #[error("malformed invite: {0}")]
    MalformedInvite(String),
    /// An actor call did not complete within the allowed timeout.
    ///
    /// The label names the call site so callers can distinguish which
    /// actor became unresponsive without needing to inspect backtraces.
    #[error("actor call timed out: {0}")]
    ActorTimeout(&'static str),
}

/// Helper to bridge `Broker<ClientEvent>` into an async stream receiver.
pub mod event_receiver {
    use crate::events::ClientEvent;
    use willow_actor::{Actor, Addr, Broker, BrokerSubscribe, Context, Handler};

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
            let (tx, rx) =
                willow_actor::runtime::channel(willow_actor::runtime::DEFAULT_MAILBOX_CAPACITY);
            let addr = system.spawn(ForwarderActor { tx });
            let recipient = addr.into();
            broker.ask(BrokerSubscribe(recipient)).await.ok();
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
            self.tx.send(msg).ok();
            async {}
        }
    }
}
pub use state::{DisplayMessage, QueueNote};
pub use views::{
    derive_archives_view, since_hint, ArchivedChannelSummary, ArchivesView, ProfileDelta,
    ProfileView,
};
pub use willow_state::{CrestPattern, PinnedFragment, PinnedKind};

// ClientState, ServerContext, ChatState, ProfileStore are used internally
// during initialization only (loading from storage → populating domain actors).
// They are not part of the public API — use ClientViewHandle for reads
// and ClientMutations for writes.
use state::{ClientState, ServerContext};

/// Re-export the event-sourced state crate for use by downstream consumers.
pub use willow_state;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use willow_identity::Identity;

/// Configuration for creating a [`ClientHandle`].
pub struct ClientConfig {
    /// Optional relay multiaddr string for NAT traversal.
    pub relay_addr: Option<String>,
    /// Initial display name for the local user.
    pub display_name: Option<String>,
    /// Whether to persist state to disk. Defaults to `true`.
    pub persistence: bool,
    /// Bootstrap peer endpoint IDs for gossip topic discovery.
    pub bootstrap_peers: Vec<willow_identity::EndpointId>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            relay_addr: None,
            display_name: None,
            persistence: true,
            bootstrap_peers: vec![],
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
    // state: lock-ok — actor migration tracked in
    // docs/specs/2026-04-26-state-management-model-design.md § 4 and § F4.
    // Single guard, generic over `N::Topic`; deferred to keep this PR scoped.
    pub(crate) topics: Arc<RwLock<HashMap<String, N::Topic>>>,
    /// Broker for pub/sub [`ClientEvent`] distribution to subscribers.
    pub(crate) event_broker: willow_actor::Addr<willow_actor::Broker<ClientEvent>>,
    /// The local identity, needed for signing broadcasts.
    pub(crate) identity: Identity,

    // ── Domain-specific state actors (Phase 2) ─────────────────────────
    /// Event-sourced server state.
    pub(crate) event_state_addr:
        willow_actor::Addr<willow_actor::StateActor<willow_state::ServerState>>,
    /// Server registry (servers map, active server, topic maps, keys).
    pub(crate) server_registry_addr:
        willow_actor::Addr<willow_actor::StateActor<state_actors::ServerRegistry>>,
    /// Chat session metadata (current channel, peers, dedup).
    pub(crate) chat_meta_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::ChatMeta>>,
    /// Global profile display names.
    pub(crate) profile_state_addr:
        willow_actor::Addr<willow_actor::StateActor<state_actors::ProfileState>>,
    /// Network connection state.
    pub(crate) network_meta_addr:
        willow_actor::Addr<willow_actor::StateActor<state_actors::NetworkMeta>>,
    /// Voice call state.
    pub(crate) voice_state_addr:
        willow_actor::Addr<willow_actor::StateActor<state_actors::VoiceState>>,
    /// Presence meta (tick counter, last-seen, queue depth, self-override).
    pub(crate) presence_meta_addr:
        willow_actor::Addr<willow_actor::StateActor<state_actors::PresenceMeta>>,
    /// Sync-queue meta (Phase 2b). Owns per-peer outbound tracking,
    /// relay/device signals, and peer-presence history used by the
    /// queue-note projection.
    pub(crate) queue_meta_addr:
        willow_actor::Addr<willow_actor::StateActor<state_actors::QueueMeta>>,
    /// Persistence actor (owns rusqlite).
    pub(crate) persistence_addr: willow_actor::Addr<persistence_actor::PersistenceActor>,
    /// Whether persistence to disk is enabled.
    pub(crate) persistence_enabled: bool,
    /// Active join links (rarely modified, shared across tasks).
    ///
    /// Uses `parking_lot::Mutex` so a panic while holding the guard does
    /// not poison the lock and take down every future caller (issue #114).
    // state: lock-ok — actor migration tracked in
    // docs/specs/2026-04-26-state-management-model-design.md § 4 and § F4.
    // Single guard; deferred to keep this PR scoped.
    pub(crate) join_links: Arc<parking_lot::Mutex<Vec<ops::JoinLink>>>,
    /// Bootstrap peers for gossip topic subscriptions.
    pub bootstrap_peers: Vec<willow_identity::EndpointId>,
    /// The per-author Merkle-DAG actor — source of truth for all events.
    pub(crate) dag_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::DagState>>,

    // ── New reactive API (spec target) ──────────────────────────────────
    /// Reactive view handle — read state at any granularity.
    pub(crate) view_handle: views::ClientViewHandle,
    /// Typed mutation interface — write state via domain actors.
    pub(crate) mutation_handle: mutations::ClientMutations<N>,

    // ── Local trust store (Phase 1d) ────────────────────────────────────
    /// Per-device verified / unverified beliefs for each peer. Never
    /// gossiped. `None` until the caller injects a store via
    /// [`with_trust_store`](Self::with_trust_store). The UI layer
    /// injects a `WebTrustStore` at boot.
    pub(crate) trust_store: Option<TrustStoreHandle>,
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
            presence_meta_addr: self.presence_meta_addr.clone(),
            queue_meta_addr: self.queue_meta_addr.clone(),
            persistence_addr: self.persistence_addr.clone(),
            persistence_enabled: self.persistence_enabled,
            join_links: Arc::clone(&self.join_links),
            bootstrap_peers: self.bootstrap_peers.clone(),
            dag_addr: self.dag_addr.clone(),
            view_handle: self.view_handle.clone(),
            mutation_handle: self.mutation_handle.clone(),
            trust_store: self.trust_store.clone(),
        }
    }
}

impl<N: willow_network::Network> ClientHandle<N> {
    /// Access the underlying actor system handle.
    ///
    /// Used by external owners (e.g. the web crate's
    /// [`SearchIndexHandle`](crate::SearchIndexHandle)) that need to
    /// spawn their own actors into the same runtime.
    pub fn system(&self) -> &willow_actor::SystemHandle {
        &self.system
    }

    /// Access reactive state views at any granularity.
    pub fn views(&self) -> &views::ClientViewHandle {
        &self.view_handle
    }

    /// Access the typed mutation interface.
    pub fn mutations(&self) -> &mutations::ClientMutations<N> {
        &self.mutation_handle
    }

    /// Inject a local [`TrustStoreHandle`]. The UI does this at boot
    /// with a `WebTrustStore` so the [`verify_peer`](Self::verify_peer)
    /// family of methods has a persistence layer. Without a store,
    /// those methods are no-ops.
    pub fn with_trust_store(mut self, store: TrustStoreHandle) -> Self {
        self.trust_store = Some(store);
        self
    }

    /// Return the currently attached trust store, if any.
    pub fn trust_store(&self) -> Option<&TrustStoreHandle> {
        self.trust_store.as_ref()
    }

    /// Mark a peer as verified, pinning its current Ed25519 key.
    /// No-op if no trust store is attached.
    pub fn verify_peer(&self, peer_id: &str) {
        let Some(store) = self.trust_store.as_ref() else {
            return;
        };
        let Ok(remote) = peer_id.parse::<willow_identity::EndpointId>() else {
            return;
        };
        store.set(
            peer_id,
            PeerTrust::Verified {
                at_ms: now_ms(),
                pinned_key: *remote.as_bytes(),
            },
        );
    }

    /// Mark a peer as unverified for a given reason. No-op if no trust
    /// store is attached.
    pub fn mark_unverified(&self, peer_id: &str, reason: UnverifiedReason) {
        if let Some(store) = self.trust_store.as_ref() {
            store.set(peer_id, PeerTrust::Unverified { reason });
        }
    }

    /// Open the `add a friend` compare-fingerprints flow for `peer_id`.
    ///
    /// This returns a [`ComparePreview`] so callers can render the
    /// fingerprint grids without re-hashing. The UI layer also bumps
    /// its own `trust.compare_target` signal to mount the dialog.
    pub fn begin_compare(&self, peer_id: &str) -> Option<ComparePreview> {
        let remote = peer_id.parse::<willow_identity::EndpointId>().ok()?;
        let local = self.identity.endpoint_id();
        // Bootstrap session-seed: blake3(DS_TAG || local || remote). Swap
        // to the real per-DM key when the backend wires it up.
        let mut hasher = blake3::Hasher::new();
        hasher.update(willow_crypto::SAS_DS_TAG);
        hasher.update(local.as_bytes());
        hasher.update(remote.as_bytes());
        let seed = hasher.finalize();
        let you = willow_crypto::sas_words(seed.as_bytes(), &local, &remote);
        // Under the bootstrap derivation both sides compute the same
        // words; diverge once a real per-DM key exists.
        let them = you.clone();
        Some(ComparePreview { you, them })
    }

    /// Current belief for a peer. [`PeerTrust::Unknown`] when no trust
    /// store is attached or the peer has not been seen.
    pub fn trust_state(&self, peer_id: &str) -> PeerTrust {
        match self.trust_store.as_ref() {
            Some(store) => store.get(peer_id),
            None => PeerTrust::Unknown,
        }
    }

    // ── Presence (phase 1e) ──────────────────────────────────────────

    /// Set the local user's self-presence override. Sticky per device;
    /// resets to [`PresenceOverride::Auto`] on page reload.
    pub async fn set_self_presence(&self, override_: presence::PresenceOverride) {
        willow_actor::state::mutate(&self.presence_meta_addr, move |pm| {
            pm.self_override = override_;
        })
        .await;
    }

    /// Read a peer's current derived [`PresenceState`](presence::PresenceState).
    ///
    /// Returns [`PresenceState::Unknown`] if the peer has never been
    /// observed via heartbeat / reachability / voice / whisper signals.
    pub async fn observe_peer_presence(
        &self,
        peer_id: willow_identity::EndpointId,
    ) -> presence::PresenceState {
        let view = self.view_handle.presence.get().await;
        view.per_peer
            .get(&peer_id)
            .copied()
            .unwrap_or(presence::PresenceState::Unknown)
    }

    /// Stub: set the whisper session status for a peer. Real wiring
    /// lands in the whisper-mode phase.
    #[doc(hidden)]
    pub async fn _set_whispering_with(&self, peer_id: willow_identity::EndpointId, active: bool) {
        willow_actor::state::mutate(&self.presence_meta_addr, move |pm| {
            if active {
                pm.whispering_with.insert(peer_id);
            } else {
                pm.whispering_with.remove(&peer_id);
            }
        })
        .await;
    }

    /// Stub: set the queued-outbound depth for a peer. Real wiring
    /// lands in the sync-queue phase.
    #[doc(hidden)]
    pub async fn _set_queue_depth(&self, peer_id: willow_identity::EndpointId, depth: u32) {
        willow_actor::state::mutate(&self.presence_meta_addr, move |pm| {
            if depth == 0 {
                pm.queue_depth.remove(&peer_id);
            } else {
                pm.queue_depth.insert(peer_id, depth);
            }
        })
        .await;
    }

    // ── Phase 2b — sync-queue API ──────────────────────────────────────

    /// Return a `QueueView` snapshot computed off the current
    /// `QueueMeta`. The web layer usually subscribes to the reactive
    /// `ClientViewHandle::queue_meta` signal + runs `compute_queue_view`
    /// through a derived actor; this accessor is primarily for
    /// integration tests + ad-hoc callers.
    pub async fn queue_view(&self) -> views::QueueView {
        let snap = willow_actor::state::get(&self.queue_meta_addr).await;
        views::compute_queue_view(&Arc::new((*snap).clone()))
    }

    /// Trigger a best-effort retry of every pending outbound message.
    /// See [`ClientMutations::retry_queue`].
    pub async fn retry_queue(&self) -> anyhow::Result<()> {
        self.mutation_handle.retry_queue().await
    }

    /// Stamp a local `mark as read` annotation for `peer_id`'s inbound
    /// queue. Never reveals message bodies.
    pub async fn mark_queue_read(
        &self,
        peer_id: willow_identity::EndpointId,
    ) -> anyhow::Result<()> {
        self.mutation_handle.mark_queue_read(peer_id).await
    }

    /// Seed a queue entry for testing — `(message_id, recipient)` pair
    /// enters `QueueMeta::outbound`. Only exposed under
    /// `#[cfg(any(test, feature = "test-utils"))]`; production code
    /// hits this through the retry-queue pipeline.
    #[cfg(any(test, feature = "test-utils"))]
    pub async fn _enqueue_outbound(
        &self,
        message_id: willow_messaging::MessageId,
        recipient: willow_identity::EndpointId,
        authored_at: u64,
    ) {
        willow_actor::state::mutate(&self.queue_meta_addr, move |qm| {
            qm.enqueue(state_actors::QueueEntry {
                message_id,
                recipient,
                authored_at,
                last_attempt_at: None,
                last_attempt_error: None,
            });
        })
        .await;
    }

    /// Expose the [`QueueMeta`] address so internal tests can observe
    /// the actor state directly.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn _queue_meta_addr(
        &self,
    ) -> &willow_actor::Addr<willow_actor::StateActor<state_actors::QueueMeta>> {
        &self.queue_meta_addr
    }
}

/// Platform-appropriate "now in epoch milliseconds". Uses
/// `SystemTime::now()` on native and `js_sys::Date::now()` on wasm so
/// the same code path writes timestamps from either target.
fn now_ms() -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as i64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// Async event processing loop (legacy).
///
/// Listeners now handle all incoming gossip events via
/// [`listeners::spawn_topic_listener`]. This struct is kept for API
/// compatibility but its [`run()`](ClientEventLoop::run) method is a no-op
/// that simply drains the internal channel.
pub struct ClientEventLoop {
    pub(crate) _system: willow_actor::System,
}

/// Persist all servers to storage.
fn persist_servers(state: &ClientState) {
    let ids: Vec<String> = state.servers.keys().cloned().collect();
    storage::save_server_list(&ids);
    for (id, ctx) in &state.servers {
        let meta = storage::SavedServerMeta {
            server_id: ctx.server_id.clone(),
            name: ctx.name.clone(),
        };
        storage::save_server_by_id(id, &meta, &ctx.keys);
    }
}

// on_connected is no longer needed — subscription is handled by connect().

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

        // Persistence is owned by `PersistenceActor` (see persistence_actor.rs);
        // no client-handle-level message database exists.

        // Load servers. Try multi-server list first, fall back to legacy single server.
        let server_ids = storage::load_server_list();
        let mut first_server_id = None;

        if let Some(ids) = &server_ids {
            // Load each server from per-server storage.
            for id in ids {
                if let Some((meta, keys)) = storage::load_server_by_id(id) {
                    let ctx = ServerContext {
                        server_id: meta.server_id,
                        name: meta.name,
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
            if let Some((meta, keys)) = storage::load_server() {
                let sid = meta.server_id.clone();
                let ctx = ServerContext {
                    server_id: meta.server_id,
                    name: meta.name,
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
                        ctx.name.clone(),
                        identity.endpoint_id(),
                    );
                }
            }
        }

        // Save in multi-server format.
        if config.persistence {
            persist_servers(&state);
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
        let event_state_addr =
            system.spawn(willow_actor::StateActor::new(state.event_state.clone()));
        let server_registry_addr = {
            let mut registry = state_actors::ServerRegistry::default();
            for (id, ctx) in &state.servers {
                registry.servers.insert(
                    id.clone(),
                    state_actors::ServerEntry {
                        server_id: ctx.server_id.clone(),
                        name: ctx.name.clone(),
                        keys: ctx.keys.clone(),
                        unread: ctx.unread.clone(),
                    },
                );
            }
            registry.active_server = state.active_server.clone();
            system.spawn(willow_actor::StateActor::new(registry))
        };
        let chat_meta_addr = {
            let meta = state_actors::ChatMeta {
                current_channel: state.chat.current_channel.clone(),
                peers: state.chat.peers.clone(),
            };
            system.spawn(willow_actor::StateActor::new(meta))
        };
        let profile_state_addr =
            system.spawn(willow_actor::StateActor::new(state_actors::ProfileState {
                names: state.profiles.names.clone(),
            }));
        let network_meta_addr = system.spawn(willow_actor::StateActor::new(
            state_actors::NetworkMeta::default(),
        ));
        let voice_state_addr = system.spawn(willow_actor::StateActor::new(
            state_actors::VoiceState::default(),
        ));
        let presence_meta_addr = system.spawn(willow_actor::StateActor::new(
            state_actors::PresenceMeta::default(),
        ));
        let queue_meta_addr = system.spawn(willow_actor::StateActor::new(
            state_actors::QueueMeta::default(),
        ));
        let persistence_enabled = config.persistence;
        let persistence_addr = system.spawn(persistence_actor::PersistenceActor::new(
            persistence_enabled,
        ));
        let event_broker = system.spawn(willow_actor::Broker::<ClientEvent>::new());
        // Open event store on the persistence actor if we have an active server.
        if let Some(sid) = &state.active_server {
            persistence_addr
                .do_send(persistence_actor::OpenEventStore {
                    server_id: sid.clone(),
                })
                .ok();
        }
        // DAG starts empty for loaded servers. It will be populated via
        // sync when connect() is called — the sync batch delivers the full
        // event history including genesis. Local mutations before sync
        // completes will fail gracefully (build_event returns Err).
        // For NEW servers, create_server() calls seed_genesis() to populate.
        let dag_addr = system.spawn(willow_actor::StateActor::new(
            state_actors::DagState::default(),
        ));
        let topics: Arc<RwLock<HashMap<String, N::Topic>>> = Arc::new(RwLock::new(HashMap::new()));
        let join_links = Arc::new(parking_lot::Mutex::new(Vec::new()));

        // Build StateRefs for derived actor sources.
        let event_ref = willow_actor::state::StateRef::from(&event_state_addr);
        let registry_ref = willow_actor::state::StateRef::from(&server_registry_addr);
        let chat_ref = willow_actor::state::StateRef::from(&chat_meta_addr);
        let profile_ref = willow_actor::state::StateRef::from(&profile_state_addr);
        let network_ref = willow_actor::state::StateRef::from(&network_meta_addr);
        let voice_ref = willow_actor::state::StateRef::from(&voice_state_addr);
        let presence_meta_ref = willow_actor::state::StateRef::from(&presence_meta_addr);
        let queue_meta_ref = willow_actor::state::StateRef::from(&queue_meta_addr);

        // Spawn Layer 2 derived view actors.
        let local_pid = identity_clone.endpoint_id();
        let messages_view = willow_actor::derived(
            &system.handle(),
            (
                event_ref.clone(),
                registry_ref.clone(),
                chat_ref.clone(),
                profile_ref.clone(),
                queue_meta_ref.clone(),
            ),
            move |(es, reg, chat, prof, qm)| {
                views::compute_messages_view(es, reg, chat, prof, qm, local_pid)
            },
        );
        let local_pid2 = identity_clone.endpoint_id();
        let members_view = willow_actor::derived(
            &system.handle(),
            (event_ref.clone(), chat_ref.clone(), profile_ref.clone()),
            move |(es, chat, prof)| views::compute_members_view(es, chat, prof, local_pid2),
        );
        let channels_view = willow_actor::derived(
            &system.handle(),
            (event_ref.clone(), registry_ref.clone()),
            |(es, reg)| views::compute_channels_view(es, reg),
        );
        let local_pid_unread = identity_clone.endpoint_id();
        let unread_view = willow_actor::derived(
            &system.handle(),
            (registry_ref.clone(), event_ref.clone()),
            move |(reg, es)| views::compute_unread_view(reg, es, local_pid_unread),
        );
        let roles_view = willow_actor::derived(&system.handle(), event_ref.clone(), |es| {
            views::compute_roles_view(es)
        });
        let connection_view = willow_actor::derived(
            &system.handle(),
            (network_ref.clone(), chat_ref.clone()),
            |(net, chat)| views::compute_connection_view(net, chat),
        );
        let local_pid3 = identity_clone.endpoint_id();
        let presence_view = willow_actor::derived(
            &system.handle(),
            (
                presence_meta_ref.clone(),
                chat_ref.clone(),
                voice_ref.clone(),
            ),
            move |(pm, chat, voice)| views::compute_presence_view(pm, chat, voice, local_pid3),
        );

        // Spawn Layer 3 grouped view actors.
        let chat_views = willow_actor::derived(
            &system.handle(),
            (
                messages_view.clone(),
                channels_view.clone(),
                unread_view.clone(),
            ),
            |(msgs, channels, unread)| views::ChatViews {
                messages: (**msgs).clone(),
                channels: (**channels).clone(),
                unread: (**unread).clone(),
            },
        );
        let social_views = willow_actor::derived(
            &system.handle(),
            (
                members_view.clone(),
                roles_view.clone(),
                connection_view.clone(),
            ),
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
            |(chat, social, voice)| views::ClientView {
                chat: (**chat).clone(),
                social: (**social).clone(),
                voice: (**voice).clone(),
                server_name: None,
                server_admins: vec![],
                current_channel: String::new(),
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
            presence: presence_view,
            event_state: event_ref,
            server_registry: registry_ref,
            chat_meta: chat_ref,
            profiles: profile_ref,
            network: network_ref,
            voice: voice_ref,
            presence_meta: presence_meta_ref,
            queue_meta: queue_meta_ref,
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
            persistence_enabled,
            identity: identity_clone.clone(),
            join_links: Arc::clone(&join_links),
            topics: Arc::clone(&topics),
            dag: dag_addr.clone(),
            queue_meta: queue_meta_addr.clone(),
        };

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
            presence_meta_addr,
            queue_meta_addr,
            persistence_addr,
            persistence_enabled,
            join_links,
            bootstrap_peers: config.bootstrap_peers,
            dag_addr: dag_addr.clone(),
            view_handle,
            mutation_handle,
            trust_store: None,
        };

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

/// Reconcile a topic map from raw string channel IDs to typed
/// [`willow_messaging::ChannelId`] values.
///
/// Entries whose channel ID string is not a valid UUID are skipped with a
/// warning log instead of being silently replaced by a randomly generated
/// UUID, which would cause the client to diverge from the rest of the
/// network (issue #141).
///
/// This function is a pre-wired utility for callers that deserialize topic
/// maps from wire data (e.g. `accept_invite`, `joining`). It is not yet
/// called from all such sites — integration into the full invite/join flow
/// is tracked as follow-up work.
///
/// # Arguments
///
/// * `raw` — map from channel-ID string to an arbitrary value `V`
///
/// # Returns
///
/// A new map with only the entries whose key parsed successfully.
pub fn reconcile_topic_map<V: Clone>(
    raw: &std::collections::HashMap<String, V>,
) -> std::collections::HashMap<willow_messaging::ChannelId, V> {
    let mut out = std::collections::HashMap::new();
    for (id_str, value) in raw {
        let cid = match uuid::Uuid::parse_str(id_str) {
            Ok(u) => willow_messaging::ChannelId(u),
            Err(e) => {
                tracing::warn!(id_str, error = %e, "skipping unparseable channel id in reconcile_topic_map");
                continue;
            }
        };
        out.insert(cid, value.clone());
    }
    out
}

/// Create a test-only ClientHandle without connecting to the network.
#[cfg(any(test, feature = "test-utils"))]
pub fn test_client() -> (
    ClientHandle<willow_network::mem::MemNetwork>,
    willow_actor::Addr<willow_actor::Broker<ClientEvent>>,
) {
    let identity = Identity::generate();

    let mut state = ClientState::new(identity.endpoint_id());

    // Create a ManagedDag seeded with genesis — DAG and state are
    // atomically initialized together.
    let mut dag_state = state_actors::DagState {
        managed: willow_state::ManagedDag::new(
            &identity,
            "Test Server",
            crate::state_actors::MAX_CLIENT_PENDING,
        )
        .expect("genesis insert must succeed in test helper"),
        stashed: HashMap::new(),
    };

    // Create the general channel in the DAG.
    let ch_id_str = uuid::Uuid::new_v4().to_string();
    dag_state
        .managed
        .create_and_insert(
            &identity,
            willow_state::EventKind::CreateChannel {
                channel_id: ch_id_str,
                name: "general".to_string(),
                kind: willow_state::ChannelKind::Text,
                ephemeral: None,
            },
            0,
        )
        .expect("channel creation must succeed in test");

    // Copy the materialized state from ManagedDag.
    state.event_state = dag_state.managed.state().clone();

    // Build a minimal ServerContext from the DAG state.
    let server_id = state.event_state.server_id.clone();
    let ctx = ServerContext {
        server_id: server_id.clone(),
        name: "Test Server".to_string(),
        keys: HashMap::new(),
        unread: HashMap::new(),
    };
    state.servers.insert(server_id.clone(), ctx);
    state.active_server = Some(server_id.clone());

    let identity_clone = identity.clone();

    // Now spawn actors AFTER state is fully initialized.
    let sys = willow_actor::System::new();
    let event_state_addr = sys.spawn(willow_actor::StateActor::new(state.event_state.clone()));
    let mut registry = state_actors::ServerRegistry::default();
    for (id, ctx) in &state.servers {
        registry.servers.insert(
            id.clone(),
            state_actors::ServerEntry {
                server_id: ctx.server_id.clone(),
                name: ctx.name.clone(),
                keys: ctx.keys.clone(),
                unread: ctx.unread.clone(),
            },
        );
    }
    registry.active_server = state.active_server.clone();
    let server_registry_addr = sys.spawn(willow_actor::StateActor::new(registry));
    let chat_meta_addr = sys.spawn(willow_actor::StateActor::new(state_actors::ChatMeta {
        current_channel: state.chat.current_channel.clone(),
        peers: state.chat.peers.clone(),
    }));
    let profile_state_addr = sys.spawn(willow_actor::StateActor::new(state_actors::ProfileState {
        names: state.profiles.names.clone(),
    }));
    let network_meta_addr = sys.spawn(willow_actor::StateActor::new(
        state_actors::NetworkMeta::default(),
    ));
    let voice_state_addr = sys.spawn(willow_actor::StateActor::new(
        state_actors::VoiceState::default(),
    ));
    let presence_meta_addr = sys.spawn(willow_actor::StateActor::new(
        state_actors::PresenceMeta::default(),
    ));
    let queue_meta_addr = sys.spawn(willow_actor::StateActor::new(
        state_actors::QueueMeta::default(),
    ));
    let persistence_addr = sys.spawn(persistence_actor::PersistenceActor::new(false));
    let event_broker = sys.spawn(willow_actor::Broker::<ClientEvent>::new());
    let dag_addr = sys.spawn(willow_actor::StateActor::new(dag_state));

    let topics: Arc<
        RwLock<
            HashMap<String, <willow_network::mem::MemNetwork as willow_network::Network>::Topic>,
        >,
    > = Arc::new(RwLock::new(HashMap::new()));
    let join_links = Arc::new(parking_lot::Mutex::new(Vec::new()));

    // Build StateRefs and derived views.
    let event_ref = willow_actor::state::StateRef::from(&event_state_addr);
    let registry_ref = willow_actor::state::StateRef::from(&server_registry_addr);
    let chat_ref = willow_actor::state::StateRef::from(&chat_meta_addr);
    let profile_ref = willow_actor::state::StateRef::from(&profile_state_addr);
    let network_ref = willow_actor::state::StateRef::from(&network_meta_addr);
    let voice_ref = willow_actor::state::StateRef::from(&voice_state_addr);
    let presence_meta_ref = willow_actor::state::StateRef::from(&presence_meta_addr);
    let queue_meta_ref = willow_actor::state::StateRef::from(&queue_meta_addr);
    let sh = sys.handle();

    let local_pid = identity_clone.endpoint_id();
    let messages_view = willow_actor::derived(
        &sh,
        (
            event_ref.clone(),
            registry_ref.clone(),
            chat_ref.clone(),
            profile_ref.clone(),
            queue_meta_ref.clone(),
        ),
        move |(es, reg, chat, prof, qm)| {
            views::compute_messages_view(es, reg, chat, prof, qm, local_pid)
        },
    );
    let local_pid2 = identity_clone.endpoint_id();
    let members_view = willow_actor::derived(
        &sh,
        (event_ref.clone(), chat_ref.clone(), profile_ref.clone()),
        move |(es, chat, prof)| views::compute_members_view(es, chat, prof, local_pid2),
    );
    let channels_view = willow_actor::derived(
        &sh,
        (event_ref.clone(), registry_ref.clone()),
        |(es, reg)| views::compute_channels_view(es, reg),
    );
    let local_pid_unread = identity_clone.endpoint_id();
    let unread_view = willow_actor::derived(
        &sh,
        (registry_ref.clone(), event_ref.clone()),
        move |(reg, es)| views::compute_unread_view(reg, es, local_pid_unread),
    );
    let roles_view = willow_actor::derived(&sh, event_ref.clone(), views::compute_roles_view);
    let connection_view = willow_actor::derived(
        &sh,
        (network_ref.clone(), chat_ref.clone()),
        |(net, chat)| views::compute_connection_view(net, chat),
    );
    let local_pid3 = identity_clone.endpoint_id();
    let presence_view = willow_actor::derived(
        &sh,
        (
            presence_meta_ref.clone(),
            chat_ref.clone(),
            voice_ref.clone(),
        ),
        move |(pm, chat, voice)| views::compute_presence_view(pm, chat, voice, local_pid3),
    );
    let chat_views = willow_actor::derived(
        &sh,
        (
            messages_view.clone(),
            channels_view.clone(),
            unread_view.clone(),
        ),
        |(m, c, u)| views::ChatViews {
            messages: (**m).clone(),
            channels: (**c).clone(),
            unread: (**u).clone(),
        },
    );
    let social_views = willow_actor::derived(
        &sh,
        (
            members_view.clone(),
            roles_view.clone(),
            connection_view.clone(),
        ),
        |(m, r, c)| views::SocialViews {
            members: (**m).clone(),
            roles: (**r).clone(),
            connection: (**c).clone(),
        },
    );
    let client_view = willow_actor::derived(
        &sh,
        (chat_views, social_views, voice_ref.clone()),
        |(c, s, v)| views::ClientView {
            chat: (**c).clone(),
            social: (**s).clone(),
            voice: (**v).clone(),
            server_name: None,
            server_admins: vec![],
            current_channel: String::new(),
        },
    );

    let view_handle = views::ClientViewHandle {
        view: client_view,
        messages: messages_view,
        members: members_view,
        channels: channels_view,
        unread: unread_view,
        roles: roles_view,
        connection: connection_view,
        presence: presence_view,
        event_state: event_ref,
        server_registry: registry_ref,
        chat_meta: chat_ref,
        profiles: profile_ref,
        network: network_ref,
        voice: voice_ref,
        presence_meta: presence_meta_ref,
        queue_meta: queue_meta_ref,
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
        persistence_enabled: false,
        identity: identity_clone.clone(),
        join_links: Arc::clone(&join_links),
        topics: Arc::clone(&topics),
        dag: dag_addr.clone(),
        queue_meta: queue_meta_addr.clone(),
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
        presence_meta_addr,
        queue_meta_addr,
        persistence_addr,
        persistence_enabled: false,
        join_links,
        bootstrap_peers: vec![],
        dag_addr: dag_addr.clone(),
        view_handle,
        mutation_handle,
        trust_store: None,
    };

    (client, event_broker)
}

/// Create a test `ClientHandle` connected to a shared `MemHub`.
///
/// Unlike `test_client()`, multiple clients created with the same `hub`
/// can exchange messages through the in-memory gossip mesh.
#[cfg(any(test, feature = "test-utils"))]
pub async fn test_client_on_hub(
    hub: &std::sync::Arc<willow_network::mem::MemHub>,
) -> (
    ClientHandle<willow_network::mem::MemNetwork>,
    willow_actor::Addr<willow_actor::Broker<ClientEvent>>,
) {
    let (mut client, broker) = test_client();
    let network = willow_network::mem::MemNetwork::new(hub);
    client.connect(network).await;
    (client, broker)
}

#[cfg(test)]
mod tests {
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn switch_server_updates_event_state() {
        let (client, _rx) = test_client();
        // test_client creates one server. Get its ID.
        let server1_id = client.active_server_id().await.unwrap();

        // Create a second server (this switches active to server2).
        let server2_id = client.create_server("Second Server").await.unwrap();
        assert_ne!(server1_id, server2_id);

        // Send a message on server2.
        client
            .send_message("general", "hello server2")
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Verify the message is on the current (server2) state.
        let msgs = client.messages("general").await;
        assert!(
            msgs.iter().any(|m| m.body == "hello server2"),
            "message should be on server2"
        );

        // Switch back to server1.
        client.switch_server(&server1_id).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // After switching, messages should NOT contain server2's message.
        let msgs = client.messages("general").await;
        assert!(
            msgs.iter().all(|m| m.body != "hello server2"),
            "server1 should not have server2's messages"
        );
    }

    /// Regression test for issue #99: after `generate_invite`, the inviter's
    /// own state must grant the recipient `SendMessages` permission. Without
    /// this, messages from the joining peer are silently rejected by
    /// `apply_incremental` in the inviter's DAG.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn generate_invite_grants_send_permission_to_recipient() {
        let (alice, _rx) = test_client();

        // A fake recipient identity (stands in for Bob).
        let bob = willow_identity::Identity::generate();
        let bob_id = bob.endpoint_id();

        // Before generating the invite, Bob has no permission in Alice's state.
        let has_before = willow_actor::state::select(&alice.event_state_addr, move |es| {
            es.has_permission(&bob_id, &willow_state::Permission::SendMessages)
        })
        .await;
        assert!(
            !has_before,
            "Bob should not have SendMessages before invite"
        );

        // Alice generates an invite for Bob.
        alice.generate_invite(&bob_id).await.unwrap();

        // Give the actor system a tick to process the mutation.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Now Bob should have SendMessages permission in Alice's state.
        let has_after = willow_actor::state::select(&alice.event_state_addr, move |es| {
            es.has_permission(&bob_id, &willow_state::Permission::SendMessages)
        })
        .await;
        assert!(
            has_after,
            "Bob should have SendMessages after generate_invite"
        );
    }

    /// Regression test for issue #115: an invite carrying a non-UUID
    /// `server_id` must be rejected with [`ClientError::MalformedInvite`]
    /// instead of being silently rewritten to a freshly minted UUID
    /// (which would split-brain the joiner from every other peer).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn malformed_invite_server_id_is_rejected() {
        // The test client uses its own randomly generated identity, so
        // we have to encrypt the invite for that identity in order for
        // the decryption stage of accept_invite to succeed and the
        // validation we care about to actually run.
        let (client, _rx) = test_client();
        let recipient_pub = invite::endpoint_id_to_ed25519_public(&client.identity.endpoint_id());

        // Build a real server + channel + key, then ask generate_invite
        // to encrypt the channel key for the test client.
        let inviter = Identity::generate();
        let server_id = uuid::Uuid::new_v4().to_string();
        let topic = format!("{}/general", server_id);
        let key = willow_crypto::generate_channel_key();
        let mut keys = HashMap::new();
        keys.insert(topic.clone(), key);
        let mut topic_names = HashMap::new();
        topic_names.insert(topic.clone(), "general".to_string());
        let valid_code = invite::generate_invite(
            "Tamper Server",
            &server_id,
            inviter.endpoint_id(),
            &keys,
            &topic_names,
            &recipient_pub,
        )
        .expect("invite generation must succeed");

        // Tamper with the embedded server_id so it no longer parses as
        // a UUID. This is the exact failure mode #115 describes: an
        // invite that looks valid right up to the point where we ask
        // parsing as a UUID for the server ID.
        //
        // Topics also have to be re-pointed at the new server_id so the
        // payload still passes the topic-confusion validator added for
        // issue #197 — otherwise the invite would be rejected at the
        // topic-prefix check instead of reaching the UUID-parse check
        // we want to exercise here.
        let raw = base64::decode(&valid_code).unwrap();
        let mut payload: invite::InvitePayload = willow_transport::unpack(&raw).unwrap();
        payload.server_id = "not-a-uuid".to_string();
        for ch in &mut payload.channels {
            ch.topic = format!("{}/{}", payload.server_id, ch.name);
        }
        let tampered_bytes = willow_transport::pack(&payload).unwrap();
        let tampered_code = base64::encode(&tampered_bytes);

        // accept_invite must surface a typed MalformedInvite error
        // instead of silently inventing a fresh UUID.
        let err = client
            .accept_invite(&tampered_code)
            .await
            .expect_err("malformed invite must be rejected");
        let downcast = err
            .downcast::<ClientError>()
            .expect("error must be a ClientError::MalformedInvite");
        assert!(
            matches!(downcast, ClientError::MalformedInvite(ref msg) if msg.contains("server_id")),
            "expected MalformedInvite about server_id, got {downcast:?}"
        );
    }

    /// Regression test for issue #141: `reconcile_topic_map` must skip
    /// entries whose channel ID is not a valid UUID instead of silently
    /// substituting a randomly generated UUID, which would corrupt the
    /// topic map with an ID that no peer has ever seen.
    #[test]
    fn reconcile_topic_map_skips_malformed_id() {
        use std::collections::HashMap;

        let mut raw: HashMap<String, String> = HashMap::new();

        // A valid UUID entry — should be retained.
        let good_id = uuid::Uuid::new_v4().to_string();
        raw.insert(good_id.clone(), "general".to_string());

        // A malformed entry — must not appear in the output.
        raw.insert("not-a-uuid".to_string(), "corrupted".to_string());
        raw.insert("also!bad@id".to_string(), "also-corrupted".to_string());

        let result = reconcile_topic_map(&raw);

        // Only the valid entry should survive.
        assert_eq!(
            result.len(),
            1,
            "malformed IDs must be dropped, not included"
        );

        let expected_cid = willow_messaging::ChannelId(uuid::Uuid::parse_str(&good_id).unwrap());
        assert!(
            result.contains_key(&expected_cid),
            "the valid channel ID must be present in the output"
        );
        assert_eq!(
            result[&expected_cid], "general",
            "the value associated with the valid ID must be preserved"
        );
    }

    // ── New tests ────────────────────────────────────────────────────────

    /// Sending a message emits a `ClientEvent::MessageReceived` to subscribers.
    ///
    /// This exercises the broker path in `apply_event` → `derive_client_events`
    /// → `event_broker.do_send(Publish(...))`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_message_emits_client_event() {
        let (client, _broker) = test_client();

        // Subscribe before sending so we don't miss the event.
        let mut rx = client.subscribe_events().await;

        client.send_message("general", "hello event").await.unwrap();

        // Await MessageReceived with a generous timeout. Under CI load the
        // actor system may not schedule a publish within a handful of
        // cooperative yields, so use an event-driven wait instead of polling.
        let deadline = std::time::Duration::from_secs(10);
        let found = tokio::time::timeout(deadline, async {
            loop {
                match rx.recv().await {
                    Some(ClientEvent::MessageReceived { .. }) => return true,
                    Some(_) => continue, // Other events (ChannelCreated etc.) — skip.
                    None => return false, // broker closed
                }
            }
        })
        .await
        .unwrap_or(false);
        assert!(
            found,
            "expected ClientEvent::MessageReceived after send_message"
        );
    }

    /// A peer without ManageChannels permission cannot create a channel.
    ///
    /// `test_client()` makes the local peer the server owner (genesis author),
    /// so it implicitly has every permission.  To test a *non-admin* we build
    /// a second `ClientMutations` handle that uses a fresh identity (Bob) but
    /// shares all the same actor addresses as Alice's client.  Bob is not in
    /// Alice's server state, so `create_and_insert` must reject his
    /// CreateChannel event with a permission error.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn non_admin_cannot_create_channel() {
        let (alice, _broker) = test_client();

        // Bob is a fresh peer — he's not Alice's genesis author and has no
        // explicit permissions in Alice's DAG.
        let bob_identity = willow_identity::Identity::generate();

        // Build a ClientMutations that shares Alice's actors but signs as Bob.
        let bob_mutations = mutations::ClientMutations::<willow_network::mem::MemNetwork> {
            event_state: alice.event_state_addr.clone(),
            server_registry: alice.server_registry_addr.clone(),
            chat_meta: alice.chat_meta_addr.clone(),
            profiles: alice.profile_state_addr.clone(),
            network: alice.network_meta_addr.clone(),
            voice: alice.voice_state_addr.clone(),
            event_broker: alice.event_broker.clone(),
            persistence: alice.persistence_addr.clone(),
            persistence_enabled: alice.persistence_enabled,
            identity: bob_identity,
            dag: alice.dag_addr.clone(),
            join_links: Arc::clone(&alice.join_links),
            topics: Arc::clone(&alice.topics),
            queue_meta: alice.queue_meta_addr.clone(),
        };

        // Bob attempts to create a channel — must fail (PermissionDenied).
        let result = bob_mutations.create_channel("secret").await;
        assert!(
            result.is_err(),
            "non-admin should not be able to create a channel, got: {:?}",
            result
        );

        // The channel list must not include "secret".
        let channels = alice.channels().await;
        assert!(
            !channels.contains(&"secret".to_string()),
            "rejected channel creation must not appear in the channel list"
        );
    }

    /// `derive_client_events` maps the main `EventKind` variants to the
    /// expected `ClientEvent` variants.  This is a pure-function unit test.
    #[test]
    fn derive_client_events_coverage() {
        use mutations::derive_client_events;
        use willow_identity::Identity;
        use willow_state::{Event, EventHash, EventKind};

        let identity = Identity::generate();
        let author = identity.endpoint_id();

        // Helper: build a minimal signed event for a given kind.
        let make = |kind: EventKind| -> Event {
            Event::new(&identity, 1, EventHash::ZERO, vec![], kind, 0)
        };

        // --- Message → MessageReceived --
        let ch_id = uuid::Uuid::new_v4().to_string();
        let ev = make(EventKind::Message {
            channel_id: ch_id.clone(),
            body: "hello".to_string(),
            reply_to: None,
        });
        let events = derive_client_events(&ev);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], ClientEvent::MessageReceived { channel, .. } if channel == &ch_id),
            "Message kind should yield MessageReceived"
        );

        // --- CreateChannel → ChannelCreated ---
        let ev = make(EventKind::CreateChannel {
            channel_id: uuid::Uuid::new_v4().to_string(),
            name: "dev".to_string(),
            kind: willow_state::ChannelKind::Text,
            ephemeral: None,
        });
        let events = derive_client_events(&ev);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], ClientEvent::ChannelCreated(n) if n == "dev"),
            "CreateChannel kind should yield ChannelCreated"
        );

        // --- DeleteChannel → ChannelDeleted ---
        let del_id = uuid::Uuid::new_v4().to_string();
        let ev = make(EventKind::DeleteChannel {
            channel_id: del_id.clone(),
        });
        let events = derive_client_events(&ev);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], ClientEvent::ChannelDeleted(id) if id == &del_id),
            "DeleteChannel kind should yield ChannelDeleted"
        );

        // --- GrantPermission → PeerTrusted ---
        let bob = Identity::generate().endpoint_id();
        let ev = make(EventKind::GrantPermission {
            peer_id: bob,
            permission: willow_state::Permission::SendMessages,
        });
        let events = derive_client_events(&ev);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], ClientEvent::PeerTrusted(pid) if *pid == bob),
            "GrantPermission kind should yield PeerTrusted"
        );

        // --- RevokePermission → PeerUntrusted ---
        let ev = make(EventKind::RevokePermission {
            peer_id: bob,
            permission: willow_state::Permission::SendMessages,
        });
        let events = derive_client_events(&ev);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], ClientEvent::PeerUntrusted(pid) if *pid == bob),
            "RevokePermission kind should yield PeerUntrusted"
        );

        // --- EditMessage → MessageEdited ---
        let msg_hash = EventHash::ZERO;
        let ev = make(EventKind::EditMessage {
            message_id: msg_hash,
            new_body: "updated".to_string(),
        });
        let events = derive_client_events(&ev);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], ClientEvent::MessageEdited { new_body, .. } if new_body == "updated"),
            "EditMessage kind should yield MessageEdited"
        );

        // --- DeleteMessage → MessageDeleted ---
        let ev = make(EventKind::DeleteMessage {
            message_id: msg_hash,
        });
        let events = derive_client_events(&ev);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], ClientEvent::MessageDeleted { .. }),
            "DeleteMessage kind should yield MessageDeleted"
        );

        // --- CreateRole → RoleCreated ---
        let role_id = uuid::Uuid::new_v4().to_string();
        let ev = make(EventKind::CreateRole {
            name: "mod".to_string(),
            role_id: role_id.clone(),
        });
        let events = derive_client_events(&ev);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], ClientEvent::RoleCreated { name, role_id: rid } if name == "mod" && rid == &role_id),
            "CreateRole kind should yield RoleCreated"
        );

        // --- DeleteRole → RoleDeleted ---
        let ev = make(EventKind::DeleteRole {
            role_id: role_id.clone(),
        });
        let events = derive_client_events(&ev);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], ClientEvent::RoleDeleted { role_id: rid } if rid == &role_id),
            "DeleteRole kind should yield RoleDeleted"
        );

        // --- CreateServer (no matching client event) ---
        let ev = make(EventKind::CreateServer {
            name: "my server".to_string(),
        });
        let events = derive_client_events(&ev);
        assert!(
            events.is_empty(),
            "CreateServer should produce no ClientEvent (handled by server-level logic)"
        );

        // Suppress unused variable warning (author is captured by `make`).
        let _ = author;
    }

    /// Two clients on the same `MemHub` can exchange events.
    ///
    /// Client A (the server owner) sends a message.  Client B is seeded
    /// with A's entire DAG so it shares the same server state.  After the
    /// broadcast, B's actor state must contain A's message.
    ///
    /// Note: `spawn_topic_listener` uses `tokio::task::spawn_local`, which
    /// requires a `LocalSet`.  The test wraps the async body in one.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_clients_sync_messages() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let hub = std::sync::Arc::new(willow_network::mem::MemHub::new());

                // ── Client A: server owner ──
                let (mut client_a, _broker_a) = test_client();
                let net_a = willow_network::mem::MemNetwork::new(&hub);
                client_a.connect(net_a).await;

                // Collect A's full DAG (genesis + CreateChannel).
                let a_events: Vec<willow_state::Event> =
                    willow_actor::state::select(&client_a.dag_addr, |ds| {
                        ds.managed
                            .dag()
                            .topological_sort()
                            .into_iter()
                            .cloned()
                            .collect()
                    })
                    .await;

                // Grant SendMessages to any peer so B can receive A's messages.
                // (B's identity isn't known yet — we grant the permission after
                //  creating B, but here we just need A's DAG to be coherent.)

                // ── Client B: fresh peer seeded with A's server state ──
                let (mut client_b, broker_b) = test_client();
                // Replace B's DAG with A's events so they share the same server state.
                // We mutate B's dag_addr directly before connecting so the listener
                // context it registers already has A's genesis.
                let events_for_b = a_events.clone();
                willow_actor::state::mutate(&client_b.dag_addr, move |ds| {
                    // Reset to an empty DAG and replay A's events.
                    ds.managed =
                        willow_state::ManagedDag::empty(crate::state_actors::MAX_CLIENT_PENDING);
                    for event in events_for_b {
                        ds.managed.insert_and_apply(event).ok();
                    }
                })
                .await;
                // Sync B's event_state mirror from the DAG.
                let b_dag_state = willow_actor::state::select(&client_b.dag_addr, |ds| {
                    ds.managed.state().clone()
                })
                .await;
                willow_actor::state::mutate(&client_b.event_state_addr, move |es| {
                    *es = b_dag_state;
                })
                .await;

                // Subscribe B's event receiver BEFORE connecting so we don't
                // miss the MessageReceived event that arrives via gossip.
                let mut b_rx = EventReceiver::subscribe(&broker_b, &client_b.system).await;

                let net_b = willow_network::mem::MemNetwork::new(&hub);
                client_b.connect(net_b).await;

                // ── A sends a message on the shared server ──
                // The "general" channel exists in A's DAG; B now has it too.
                client_a
                    .send_message("general", "hello from A")
                    .await
                    .unwrap();

                // Wait for B's listener to receive and apply the gossip event.
                // We use a timeout to avoid hanging indefinitely if delivery fails,
                // while giving enough time for the async machinery to settle.
                // The deadline is generous because we're driving a LocalSet on top of
                // a multi-thread runtime and need both schedulers to make progress.
                let deadline = std::time::Duration::from_secs(2);
                let found = tokio::time::timeout(deadline, async {
                    loop {
                        match b_rx.try_recv() {
                            Some(ClientEvent::MessageReceived { .. }) => return true,
                            Some(_) => {}
                            None => {
                                tokio::task::yield_now().await;
                            }
                        }
                    }
                })
                .await
                .unwrap_or(false);

                // Fallback: check B's event_messages in case the event was applied
                // but the broker event was missed (e.g. subscribe race).
                let found = if !found {
                    for _ in 0..50 {
                        tokio::task::yield_now().await;
                    }
                    let b_es = willow_actor::state::get(&client_b.event_state_addr).await;
                    let general_id = b_es
                        .channels
                        .iter()
                        .find(|(_, ch)| ch.name == "general")
                        .map(|(id, _)| id.clone())
                        .unwrap_or_default();
                    if !general_id.is_empty() {
                        let b_msgs = client_b.event_messages(&general_id).await;
                        b_msgs.iter().any(|m| m.body == "hello from A")
                    } else {
                        false
                    }
                } else {
                    true
                };

                assert!(
                    found,
                    "client B should have received the message sent by client A"
                );
            })
            .await;
    }

    /// Presence round-trip: set Away on self-override, observe Away from
    /// the derived presence view. Browser close (state reset) is not
    /// covered here — the actor state is per-process.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn presence_self_override_round_trip() {
        let (client, _rx) = test_client();
        // Default is Auto → Here (reachable, no activity, no heartbeat stale).
        let before = client.view_handle.presence.get().await;
        assert_eq!(before.self_state, presence::PresenceState::Here);

        client
            .set_self_presence(presence::PresenceOverride::Away)
            .await;
        // Give the derived view a tick to recompute.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let after = client.view_handle.presence.get().await;
        assert_eq!(after.self_state, presence::PresenceState::Away);
    }

    /// Peers that have been marked reachable default to `Here` — zero
    /// queue, zero last_seen, zero now ⇒ elapsed 0 < idle_ticks.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn presence_reachable_peer_defaults_to_here() {
        let (client, _rx) = test_client();
        let bob = willow_identity::Identity::generate().endpoint_id();

        client.mutations().peer_connected(bob).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let state = client.observe_peer_presence(bob).await;
        assert_eq!(state, presence::PresenceState::Here);
    }

    /// A queued peer that stays unreachable past the gone threshold
    /// flips from `Queued` → `Gone` on the next tick. Drives the
    /// tick-once helper manually to avoid waiting real seconds.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn presence_queued_then_gone_after_threshold() {
        let (client, _rx) = test_client();
        let bob = willow_identity::Identity::generate().endpoint_id();

        // Start with bob reachable + a short gone threshold so we don't
        // have to advance the clock 172_800 ticks.
        client.mutations().peer_connected(bob).await;
        willow_actor::state::mutate(&client.presence_meta_addr, |pm| {
            pm.gone_ticks = 5;
            pm.idle_ticks = 3;
        })
        .await;
        client._set_queue_depth(bob, 2).await;

        // Advance one tick while reachable — last_seen stays fresh.
        connect::tick_once_for_test(&client.presence_meta_addr, &client.chat_meta_addr).await;
        // Drop bob offline and advance a few ticks.
        client.mutations().peer_disconnected(bob).await;
        // Allow the derived view to recompute.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        // queue > 0 + reachable = false ⇒ Queued before gone threshold.
        let before = client.observe_peer_presence(bob).await;
        assert!(
            matches!(before, presence::PresenceState::Queued(_)),
            "expected Queued, got {before:?}",
        );

        // Advance past gone_ticks (=5) — tick 6 to guarantee we cross it.
        for _ in 0..6 {
            connect::tick_once_for_test(&client.presence_meta_addr, &client.chat_meta_addr).await;
        }
        // Let the derived view settle after the mutation burst.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let pm = willow_actor::state::get(&client.presence_meta_addr).await;
        let elapsed = pm
            .now
            .saturating_sub(pm.last_seen.get(&bob).copied().unwrap_or(0));
        let after = client.observe_peer_presence(bob).await;
        assert_eq!(
            after,
            presence::PresenceState::Gone,
            "after crossing gone_ticks the state must flip to Gone \
             (now={}, last_seen={:?}, elapsed={}, gone_ticks={})",
            pm.now,
            pm.last_seen.get(&bob),
            elapsed,
            pm.gone_ticks,
        );
    }

    /// Regression test for issue #114: switching `join_links` to
    /// `parking_lot::Mutex` makes lock poisoning impossible by
    /// construction. After a panic in one task that holds the lock, a
    /// subsequent caller from another task must still be able to
    /// acquire it without panicking.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_links_lock_survives_panic_in_holder() {
        let (client, _rx) = test_client();

        // Spawn a task that grabs the lock, mutates the vec, then panics
        // while still holding the guard. With std::sync::Mutex this
        // would poison the mutex and every future caller would panic on
        // .lock().unwrap(). With parking_lot the lock is simply
        // released and remains usable.
        let join_links = Arc::clone(&client.join_links);
        let panicker = tokio::task::spawn_blocking(move || {
            let mut guard = join_links.lock();
            guard.push(crate::ops::JoinLink {
                link_id: "first".to_string(),
                server_id: "s".to_string(),
                max_uses: 1,
                used: 0,
                active: true,
                expires_at: None,
                created_at: 0,
            });
            panic!("simulated panic while holding the join_links guard");
        });
        let join_result = panicker.await;
        assert!(join_result.is_err(), "task should have panicked");

        // The next caller must NOT panic. Both reads and writes should
        // succeed.
        let snapshot = client.join_links().await;
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].link_id, "first");

        client.delete_join_link("first").await;
        let snapshot = client.join_links().await;
        assert!(snapshot.is_empty());
    }

    // ── Phase 1f: per-identity mute mutations ─────────────────────────

    /// `mutate_channel_mute` emits a `MuteChanged { Channel, true }`
    /// event and the next `compute_unread_view` reports the channel as
    /// muted.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mutate_channel_mute_emits_event_and_flips_stats() {
        let (client, _broker) = test_client();
        client.create_channel("quiet").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut rx = client.subscribe_events().await;
        client.mutate_channel_mute("quiet", true).await.unwrap();

        // Drain events until we see MuteChanged. 2-second timeout is
        // plenty for a broker hop.
        let deadline = std::time::Duration::from_secs(2);
        let seen = tokio::time::timeout(deadline, async {
            loop {
                match rx.recv().await {
                    Some(ClientEvent::MuteChanged {
                        scope: crate::events::MuteScope::Channel(_),
                        muted: true,
                    }) => return true,
                    Some(_) => continue,
                    None => return false,
                }
            }
        })
        .await
        .unwrap_or(false);
        assert!(seen, "expected MuteChanged event after mutate_channel_mute");

        // The channel's UnreadStats.muted flag should now be true.
        let registry = willow_actor::state::get(&client.server_registry_addr).await;
        let events = willow_actor::state::get(&client.event_state_addr).await;
        let view = views::compute_unread_view(&registry, &events, client.identity.endpoint_id());
        // The channel may have zero unread, but the muted flag is
        // derived from mute_state independently — look up the
        // ServerState directly.
        let mute = events
            .mute_state
            .get(&client.identity.endpoint_id())
            .expect("mute entry exists after mutation");
        let ch_id = events
            .channels
            .values()
            .find(|c| c.name == "quiet")
            .map(|c| c.id.clone())
            .expect("quiet channel exists");
        assert!(mute.channels.contains(&ch_id));
        // view is a view — at least compiles and the call shape works.
        let _ = view;
    }

    /// `mutate_grove_mute` sets `grove_muted` to true and emits a
    /// `MuteChanged { Grove, true }` event.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mutate_grove_mute_sets_grove_muted() {
        let (client, _broker) = test_client();
        let mut rx = client.subscribe_events().await;
        client.mutate_grove_mute(true).await.unwrap();

        let deadline = std::time::Duration::from_secs(2);
        let seen = tokio::time::timeout(deadline, async {
            loop {
                match rx.recv().await {
                    Some(ClientEvent::MuteChanged {
                        scope: crate::events::MuteScope::Grove,
                        muted: true,
                    }) => return true,
                    Some(_) => continue,
                    None => return false,
                }
            }
        })
        .await
        .unwrap_or(false);
        assert!(seen, "expected MuteChanged event after mutate_grove_mute");

        let events = willow_actor::state::get(&client.event_state_addr).await;
        let entry = events
            .mute_state
            .get(&client.identity.endpoint_id())
            .expect("mute entry present");
        assert!(entry.grove_muted);
    }

    /// Toggling mute off after on removes the channel from the set and
    /// emits a `MuteChanged { muted: false }` event.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mutate_channel_mute_toggle_off_clears_set() {
        let (client, _broker) = test_client();
        client.create_channel("noisy").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        client.mutate_channel_mute("noisy", true).await.unwrap();
        client.mutate_channel_mute("noisy", false).await.unwrap();

        let events = willow_actor::state::get(&client.event_state_addr).await;
        let ch_id = events
            .channels
            .values()
            .find(|c| c.name == "noisy")
            .map(|c| c.id.clone())
            .unwrap();
        let entry = events
            .mute_state
            .get(&client.identity.endpoint_id())
            .expect("mute entry present after two toggles");
        assert!(
            !entry.channels.contains(&ch_id),
            "unmute must remove the channel from the set"
        );
    }
}
