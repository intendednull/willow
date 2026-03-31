use super::*;

impl<N: willow_network::Network> ClientHandle<N> {
    /// Join a voice channel. Leaves the current voice channel first if in one.
    pub async fn join_voice(&self, channel_id: &str) {
        let in_voice = willow_actor::state::select(&self.voice_state_addr, |v| {
            v.active_channel.is_some()
        })
        .await;
        if in_voice {
            self.leave_voice().await;
        }
        let ch = channel_id.to_string();
        let my_peer_id = self.identity.endpoint_id();
        willow_actor::state::mutate(&self.voice_state_addr, move |v| {
            v.active_channel = Some(ch.clone());
            v.participants.entry(ch).or_default().insert(my_peer_id);
        })
        .await;
        let msg = ops::WireMessage::VoiceJoin {
            channel_id: channel_id.to_string(),
            peer_id: my_peer_id,
        };
        if let Some(data) = ops::pack_wire(&msg, &self.identity) {
            self.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }
    }

    /// Leave the current voice channel, if in one.
    pub async fn leave_voice(&self) {
        let my_peer_id = self.identity.endpoint_id();
        let maybe_ch = willow_actor::state::mutate(&self.voice_state_addr, move |v| {
            if let Some(ch) = v.active_channel.take() {
                if let Some(p) = v.participants.get_mut(&ch) {
                    p.remove(&my_peer_id);
                }
                Some(ch)
            } else {
                None
            }
        })
        .await;
        if let Some(ch) = maybe_ch {
            let msg = ops::WireMessage::VoiceLeave {
                channel_id: ch,
                peer_id: my_peer_id,
            };
            if let Some(data) = ops::pack_wire(&msg, &self.identity) {
                self.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
            }
        }
    }

    /// Toggle mute state. Returns the new muted value.
    pub async fn toggle_mute(&self) -> bool {
        willow_actor::state::mutate(&self.voice_state_addr, |v| {
            v.muted = !v.muted;
            v.muted
        })
        .await
    }

    /// Toggle deafen state. Returns the new deafened value.
    pub async fn toggle_deafen(&self) -> bool {
        willow_actor::state::mutate(&self.voice_state_addr, |v| {
            v.deafened = !v.deafened;
            v.deafened
        })
        .await
    }

    /// Returns the list of peer IDs currently in the given voice channel.
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

    /// Returns the voice channel we are currently in, if any.
    pub async fn active_voice_channel(&self) -> Option<String> {
        willow_actor::state::select(&self.voice_state_addr, |v| v.active_channel.clone()).await
    }

    /// Returns whether we are currently muted.
    pub async fn is_voice_muted(&self) -> bool {
        willow_actor::state::select(&self.voice_state_addr, |v| v.muted).await
    }

    /// Returns whether we are currently deafened.
    pub async fn is_voice_deafened(&self) -> bool {
        willow_actor::state::select(&self.voice_state_addr, |v| v.deafened).await
    }

    /// Send a voice signaling message to a specific peer.
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
            self.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }
    }
}
