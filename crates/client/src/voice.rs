use super::*;

impl<N: willow_network::Network> ClientHandle<N> {
    pub async fn join_voice(&self, channel_id: &str) {
        self.mutation_handle.join_voice(channel_id).await;
    }

    pub async fn leave_voice(&self) {
        self.mutation_handle.leave_voice().await;
    }

    pub async fn toggle_mute(&self) -> bool {
        self.mutation_handle.toggle_mute().await
    }

    pub async fn toggle_deafen(&self) -> bool {
        self.mutation_handle.toggle_deafen().await
    }

    pub async fn voice_participants(&self, channel_id: &str) -> Vec<willow_identity::EndpointId> {
        let ch = channel_id.to_string();
        willow_actor::state::select(&self.voice_state_addr, move |v| {
            v.participants
                .get(&ch)
                .map(|p| p.iter().copied().collect())
                .unwrap_or_default()
        })
        .await
    }

    pub async fn active_voice_channel(&self) -> Option<String> {
        willow_actor::state::select(&self.voice_state_addr, |v| v.active_channel.clone()).await
    }

    pub async fn is_voice_muted(&self) -> bool {
        willow_actor::state::select(&self.voice_state_addr, |v| v.muted).await
    }

    pub async fn is_voice_deafened(&self) -> bool {
        willow_actor::state::select(&self.voice_state_addr, |v| v.deafened).await
    }

    pub fn send_voice_signal(
        &self,
        channel_id: &str,
        target: willow_identity::EndpointId,
        signal: ops::VoiceSignalPayload,
    ) {
        let msg = ops::WireMessage::VoiceSignal {
            channel_id: channel_id.to_string(),
            target_peer: target,
            signal,
        };
        if let Some(data) = ops::pack_wire(&msg, &self.identity) {
            self.mutation_handle.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }
    }
}
