use super::*;
use crate::util;

impl<N: willow_network::Network> ClientHandle<N> {
    pub fn identity(&self) -> Identity {
        self.identity.clone()
    }

    /// Subscribe to client events.
    pub async fn subscribe_events(&self) -> crate::EventReceiver {
        crate::EventReceiver::subscribe(&self.event_broker, &self.system).await
    }

    /// Get the event broker address.
    pub fn event_broker(&self) -> &willow_actor::Addr<willow_actor::Broker<ClientEvent>> {
        &self.event_broker
    }

    pub fn peer_id(&self) -> String {
        self.identity.endpoint_id().to_string()
    }

    pub async fn display_name(&self) -> String {
        let pid = self.identity.endpoint_id();
        let es = willow_actor::state::get(&self.event_state_addr).await;
        if let Some(profile) = es.profiles.get(&pid) {
            return profile.display_name.clone();
        }
        let profiles = willow_actor::state::get(&self.profile_state_addr).await;
        profiles.display_name(&pid)
    }

    pub async fn peer_display_name(&self, peer_id: &willow_identity::EndpointId) -> String {
        let pid = *peer_id;
        let es = willow_actor::state::get(&self.event_state_addr).await;
        let profiles = willow_actor::state::get(&self.profile_state_addr).await;
        views::resolve_display_name(&es, &profiles, &pid)
    }

    pub async fn messages(&self, channel: &str) -> Vec<state::DisplayMessage> {
        let es = willow_actor::state::get(&self.event_state_addr).await;
        let registry = willow_actor::state::get(&self.server_registry_addr).await;
        let profiles = willow_actor::state::get(&self.profile_state_addr).await;
        let local_peer_id = self.identity.endpoint_id();

        // If caller asks for a specific channel, compute for that channel.
        // Otherwise use current channel from ChatMeta.
        let view = views::compute_messages_view_for_channel(
            &es,
            &registry,
            &profiles,
            channel,
            local_peer_id,
        );
        view.messages
    }

    pub async fn channels(&self) -> Vec<String> {
        let es = willow_actor::state::get(&self.event_state_addr).await;
        let registry = willow_actor::state::get(&self.server_registry_addr).await;
        let view = views::compute_channels_view(&es, &registry);
        view.channels.into_iter().map(|c| c.name).collect()
    }

    /// Snapshot of the current materialized server state.
    ///
    /// Useful for assertion-style tests that need to inspect channel
    /// metadata (kinds, ephemeral config, last-activity HLC).
    pub async fn state_snapshot(&self) -> willow_state::ServerState {
        let arc = willow_actor::state::get(&self.event_state_addr).await;
        (*arc).clone()
    }

    /// Derive the archives view at the given frontier HLC (physical
    /// milliseconds). Lists every ephemeral channel whose
    /// `last_activity_hlc + idle_threshold_ms` is below the frontier.
    ///
    /// Spec: `docs/specs/2026-04-19-ui-design/ephemeral-channels.md`
    /// §Archive surface.
    pub async fn archives_view_at(&self, frontier_hlc_ms: u64) -> views::ArchivesView {
        let arc = willow_actor::state::get(&self.event_state_addr).await;
        views::derive_archives_view(&arc, frontier_hlc_ms)
    }

    pub async fn event_messages(&self, channel_id: &str) -> Vec<willow_state::ChatMessage> {
        let cid = channel_id.to_string();
        let addr = self.event_state_addr.clone();
        util::with_timeout("event_messages", async move {
            willow_actor::state::select(&addr, move |es| {
                es.messages
                    .iter()
                    .filter(|m| m.channel_id == cid && !m.deleted)
                    .cloned()
                    .collect()
            })
            .await
        })
        .await
        .unwrap_or_default()
    }

    pub async fn peers(&self) -> Vec<willow_identity::EndpointId> {
        let addr = self.chat_meta_addr.clone();
        util::with_timeout("peers", async move {
            willow_actor::state::select(&addr, |c| c.peers.clone()).await
        })
        .await
        .unwrap_or_default()
    }

    pub async fn server_members(&self) -> Vec<(willow_identity::EndpointId, String, bool)> {
        let es = willow_actor::state::get(&self.event_state_addr).await;
        let chat = willow_actor::state::get(&self.chat_meta_addr).await;
        let profiles = willow_actor::state::get(&self.profile_state_addr).await;
        let local_pid = self.identity.endpoint_id();
        let view = views::compute_members_view(&es, &chat, &profiles, local_pid);
        view.members
            .into_iter()
            .map(|m| (m.peer_id, m.display_name, m.is_online))
            .collect()
    }

    pub async fn is_connected(&self) -> bool {
        let addr = self.network_meta_addr.clone();
        util::with_timeout("is_connected", async move {
            willow_actor::state::select(&addr, |n| n.connected).await
        })
        .await
        .unwrap_or(false)
    }

    pub async fn has_permission(
        &self,
        peer_id: &willow_identity::EndpointId,
        perm: &willow_state::Permission,
    ) -> bool {
        let pid = *peer_id;
        let p = *perm;
        let addr = self.event_state_addr.clone();
        util::with_timeout("has_permission", async move {
            willow_actor::state::select(&addr, move |es| es.has_permission(&pid, &p)).await
        })
        .await
        .unwrap_or(false)
    }

    pub async fn roles_data(&self) -> Vec<(String, String, Vec<String>)> {
        let es = willow_actor::state::get(&self.event_state_addr).await;
        let view = views::compute_roles_view(&es);
        view.roles
            .into_iter()
            .map(|r| (r.id, r.name, r.permissions))
            .collect()
    }

    pub async fn unread_counts(&self) -> HashMap<String, usize> {
        let registry = willow_actor::state::get(&self.server_registry_addr).await;
        let events = willow_actor::state::get(&self.event_state_addr).await;
        views::compute_unread_view(&registry, &events, self.identity.endpoint_id()).counts()
    }

    /// Check if a peer is an admin.
    pub async fn is_admin(&self, peer_id: &willow_identity::EndpointId) -> bool {
        let pid = *peer_id;
        let addr = self.event_state_addr.clone();
        util::with_timeout("is_admin", async move {
            willow_actor::state::select(&addr, move |es| es.is_admin(&pid)).await
        })
        .await
        .unwrap_or(false)
    }

    /// Get the set of admin EndpointIds.
    pub async fn admins(&self) -> std::collections::BTreeSet<willow_identity::EndpointId> {
        willow_actor::state::select(&self.event_state_addr, |es| es.admins.clone()).await
    }

    pub async fn channel_kinds(&self) -> Vec<(String, willow_state::ChannelKind)> {
        willow_actor::state::select(&self.event_state_addr, |es| {
            es.channels
                .values()
                .map(|ch| (ch.name.clone(), ch.kind.clone()))
                .collect()
        })
        .await
    }

    pub async fn current_channel(&self) -> String {
        willow_actor::state::select(&self.chat_meta_addr, |c| c.current_channel.clone()).await
    }

    pub async fn server_description(&self) -> String {
        willow_actor::state::select(&self.event_state_addr, |es| es.description.clone()).await
    }

    /// Most recent message authored by the local peer in `channel_id`.
    ///
    /// Spec: `docs/specs/2026-04-19-ui-design/composer.md` §Keyboard —
    /// "ArrowUp when textarea empty: enters edit mode on most recent
    /// own message". Plan: `docs/plans/2026-04-26-ui-phase-3a-composer.md`
    /// Task T2.
    ///
    /// Returns [`None`] when the channel doesn't exist or the local
    /// peer hasn't authored any non-deleted message in it. The lookup
    /// goes through [`views::compute_messages_view_for_channel`] so the
    /// returned [`state::DisplayMessage`] carries the same projections
    /// (mentions, pinned, queue note, …) as the rendered row — the
    /// composer can pre-fill from `body` without re-deriving anything.
    pub async fn last_own_message(&self, channel_id: &str) -> Option<state::DisplayMessage> {
        let es = willow_actor::state::get(&self.event_state_addr).await;
        let registry = willow_actor::state::get(&self.server_registry_addr).await;
        let profiles = willow_actor::state::get(&self.profile_state_addr).await;
        let local = self.identity.endpoint_id();
        let view =
            views::compute_messages_view_for_channel(&es, &registry, &profiles, channel_id, local);
        // `compute_messages_view` sorts ascending by timestamp_ms; the
        // most recent local message is the last `is_local && !deleted`
        // entry. Iterate in reverse so we return on the first match.
        view.messages
            .into_iter()
            .rev()
            .find(|m| m.author_peer_id == local && !m.deleted)
    }

    pub async fn typing_peers(&self) -> Vec<(String, String)> {
        let my_id = self.identity.endpoint_id();
        willow_actor::state::mutate(&self.network_meta_addr, move |n| {
            let now = crate::util::current_time_ms();
            // Keep map + recency in lockstep: helper drops both.
            n.sweep_typing(now, crate::TYPING_INDICATOR_TTL_MS);
            n.typing_peers
                .iter()
                .filter(|(pid, _)| *pid != &my_id)
                .map(|(pid, (channel, _))| (pid.to_string(), channel.clone()))
                .collect()
        })
        .await
    }
}

// ── Test-only address getters (test-hooks feature) ────────────────────────
//
// Gated behind `test-hooks` so non-test consumers (`willow-agent`,
// `willow-replay`, etc.) never see them. The address itself doesn't grant
// write access without an active mutator — these are a read-only handle
// for `WillowTestHooks` in the web crate, which cannot hold a generic
// `ClientHandle<N>` across the wasm_bindgen boundary.

#[cfg(feature = "test-hooks")]
impl<N: willow_network::Network> ClientHandle<N> {
    /// Clone the per-author Merkle-DAG actor address. Test-only.
    pub fn dag_addr_clone(
        &self,
    ) -> willow_actor::Addr<willow_actor::StateActor<crate::state_actors::DagState>> {
        self.dag_addr.clone()
    }

    /// Clone the materialised `ServerState` actor address. Test-only.
    ///
    /// Used by the snapshot builder in `WillowTestHooks` to read the
    /// channels view for assertion-style polling.
    pub fn event_state_addr_clone(
        &self,
    ) -> willow_actor::Addr<willow_actor::StateActor<willow_state::ServerState>> {
        self.event_state_addr.clone()
    }
}
