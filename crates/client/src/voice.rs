use super::*;

impl<N: willow_network::Network> ClientHandle<N> {
    /// Join a voice channel. Leaves the current voice channel first if in one.
    pub fn join_voice(&self, channel_id: &str) {
        // Leave current voice channel if in one.
        if self.shared.read().unwrap().active_voice_channel.is_some() {
            self.leave_voice();
        }
        let mut shared = self.shared.write().unwrap();
        let my_peer_id = shared.identity.endpoint_id();
        shared.active_voice_channel = Some(channel_id.to_string());
        // Add ourselves to participants.
        shared
            .voice_participants
            .entry(channel_id.to_string())
            .or_default()
            .insert(my_peer_id);
        // Broadcast join.
        let msg = ops::WireMessage::VoiceJoin {
            channel_id: channel_id.to_string(),
            peer_id: my_peer_id,
        };
        if let Some(data) = ops::pack_wire(&msg, &self.identity) {
            self.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }
    }

    /// Leave the current voice channel, if in one.
    pub fn leave_voice(&self) {
        let mut shared = self.shared.write().unwrap();
        let my_peer_id = shared.identity.endpoint_id();
        if let Some(ch) = shared.active_voice_channel.take() {
            // Remove ourselves from participants.
            if let Some(participants) = shared.voice_participants.get_mut(&ch) {
                participants.remove(&my_peer_id);
            }
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
    pub fn toggle_mute(&self) -> bool {
        let mut shared = self.shared.write().unwrap();
        shared.voice_muted = !shared.voice_muted;
        shared.voice_muted
    }

    /// Toggle deafen state. Returns the new deafened value.
    pub fn toggle_deafen(&self) -> bool {
        let mut shared = self.shared.write().unwrap();
        shared.voice_deafened = !shared.voice_deafened;
        shared.voice_deafened
    }

    /// Returns the list of peer IDs currently in the given voice channel.
    pub fn voice_participants(&self, channel_id: &str) -> Vec<willow_identity::EndpointId> {
        let shared = self.shared.read().unwrap();
        shared
            .voice_participants
            .get(channel_id)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Returns the voice channel we are currently in, if any.
    pub fn active_voice_channel(&self) -> Option<String> {
        self.shared.read().unwrap().active_voice_channel.clone()
    }

    /// Returns whether we are currently muted.
    pub fn is_voice_muted(&self) -> bool {
        self.shared.read().unwrap().voice_muted
    }

    /// Returns whether we are currently deafened.
    pub fn is_voice_deafened(&self) -> bool {
        self.shared.read().unwrap().voice_deafened
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
