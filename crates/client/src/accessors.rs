use super::*;

impl<N: willow_network::Network> ClientHandle<N> {
    pub fn identity(&self) -> Identity {
        self.identity.clone()
    }

    pub fn state_addr(&self) -> &willow_actor::Addr<client_actor::ClientStateActor> {
        &self.state_addr
    }

    pub fn actor_system(&self) -> &willow_actor::SystemHandle {
        &self.system
    }

    pub fn notify_mutation(&self) {
        let _ = self.state_addr.do_send(client_actor::NotifyMutation);
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
        let chat = willow_actor::state::get(&self.chat_meta_addr).await;
        let profiles = willow_actor::state::get(&self.profile_state_addr).await;
        let local_peer_id = self.identity.endpoint_id();

        // If caller asks for a specific channel, compute for that channel.
        // Otherwise use current channel from ChatMeta.
        let view = views::compute_messages_view_for_channel(
            &es, &registry, &profiles, channel, local_peer_id,
        );
        view.messages
    }

    pub async fn channels(&self) -> Vec<String> {
        let es = willow_actor::state::get(&self.event_state_addr).await;
        let registry = willow_actor::state::get(&self.server_registry_addr).await;
        let view = views::compute_channels_view(&es, &registry);
        view.channels.into_iter().map(|c| c.name).collect()
    }

    pub async fn event_messages(&self, channel_id: &str) -> Vec<willow_state::ChatMessage> {
        let cid = channel_id.to_string();
        willow_actor::state::select(&self.event_state_addr, move |es| {
            es.messages
                .iter()
                .filter(|m| m.channel_id == cid && !m.deleted)
                .cloned()
                .collect()
        })
        .await
    }

    pub async fn peers(&self) -> Vec<willow_identity::EndpointId> {
        willow_actor::state::select(&self.chat_meta_addr, |c| c.peers.clone()).await
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
        willow_actor::state::select(&self.network_meta_addr, |n| n.connected).await
    }

    pub async fn has_permission(
        &self,
        peer_id: &willow_identity::EndpointId,
        perm: &willow_state::Permission,
    ) -> bool {
        let pid = *peer_id;
        let p = perm.clone();
        willow_actor::state::select(&self.event_state_addr, move |es| {
            es.has_permission(&pid, &p)
        })
        .await
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
        views::compute_unread_view(&registry).counts
    }

    pub async fn server_owner(&self) -> willow_identity::EndpointId {
        willow_actor::state::select(&self.event_state_addr, |es| es.owner).await
    }

    pub async fn channel_kinds(&self) -> Vec<(String, String)> {
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
}
