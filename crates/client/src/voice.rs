use super::*;
use crate::client_actor::{mutate_state, read_state};

impl<N: willow_network::Network> ClientHandle<N> {
    /// Join a voice channel. Leaves the current voice channel first if in one.
    pub async fn join_voice(&self, channel_id: &str) {
        let in_voice = read_state(&self.state_addr, |s| s.active_voice_channel.is_some()).await;
        if in_voice {
            self.leave_voice().await;
        }
        let ch = channel_id.to_string();
        let msg = mutate_state(&self.state_addr, move |s| {
            let my_peer_id = s.identity.endpoint_id();
            s.active_voice_channel = Some(ch.clone());
            s.voice_participants
                .entry(ch.clone())
                .or_default()
                .insert(my_peer_id);
            ops::WireMessage::VoiceJoin {
                channel_id: ch,
                peer_id: my_peer_id,
            }
        })
        .await;
        if let Some(data) = ops::pack_wire(&msg, &self.identity) {
            self.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }
    }

    /// Leave the current voice channel, if in one.
    pub async fn leave_voice(&self) {
        let maybe_msg = mutate_state(&self.state_addr, |s| {
            let my_peer_id = s.identity.endpoint_id();
            if let Some(ch) = s.active_voice_channel.take() {
                if let Some(p) = s.voice_participants.get_mut(&ch) {
                    p.remove(&my_peer_id);
                }
                Some(ops::WireMessage::VoiceLeave {
                    channel_id: ch,
                    peer_id: my_peer_id,
                })
            } else {
                None
            }
        })
        .await;
        if let Some(msg) = maybe_msg {
            if let Some(data) = ops::pack_wire(&msg, &self.identity) {
                self.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
            }
        }
    }

    /// Toggle mute state. Returns the new muted value.
    pub async fn toggle_mute(&self) -> bool {
        mutate_state(&self.state_addr, |s| {
            s.voice_muted = !s.voice_muted;
            s.voice_muted
        })
        .await
    }

    /// Toggle deafen state. Returns the new deafened value.
    pub async fn toggle_deafen(&self) -> bool {
        mutate_state(&self.state_addr, |s| {
            s.voice_deafened = !s.voice_deafened;
            s.voice_deafened
        })
        .await
    }

    /// Returns the list of peer IDs currently in the given voice channel.
    pub async fn voice_participants(&self, channel_id: &str) -> Vec<willow_identity::EndpointId> {
        let ch = channel_id.to_string();
        read_state(&self.state_addr, move |s| {
            s.voice_participants
                .get(&ch)
                .map(|p| p.iter().copied().collect())
                .unwrap_or_default()
        })
        .await
    }

    /// Returns the voice channel we are currently in, if any.
    pub async fn active_voice_channel(&self) -> Option<String> {
        read_state(&self.state_addr, |s| s.active_voice_channel.clone()).await
    }

    /// Returns whether we are currently muted.
    pub async fn is_voice_muted(&self) -> bool {
        read_state(&self.state_addr, |s| s.voice_muted).await
    }

    /// Returns whether we are currently deafened.
    pub async fn is_voice_deafened(&self) -> bool {
        read_state(&self.state_addr, |s| s.voice_deafened).await
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
