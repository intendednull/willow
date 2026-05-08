//! UI-facing action methods on [`ClientHandle`].
//!
//! Most entry points in this module are thin pass-throughs that forward
//! their arguments to the corresponding method on
//! [`crate::mutations::ClientMutations`]. Their behaviour is exercised
//! through the mutation handle directly in `tests/multi_peer_sync.rs`,
//! `tests/trust_flow.rs`, `tests/ephemeral.rs`, and the inline `tests`
//! module at the bottom of `lib.rs`. State-machine-level invariants are
//! covered by `crates/state/src/tests.rs`.
//!
//! Methods that do non-trivial translation work *before* delegating —
//! validation (`share_file_inline`), ID minting (`create_voice_channel`),
//! direct event assembly with no mutation-handle helper
//! (`set_permission`, `assign_role`), or derived-view composition
//! (`pinned_message_ids`, `pinned_messages`, `is_pinned`) — are covered
//! at the client tier in `tests/actions.rs`.

use super::*;

impl<N: willow_network::Network> ClientHandle<N> {
    pub async fn send_message(&self, channel: &str, body: &str) -> anyhow::Result<()> {
        self.mutation_handle.send_message(channel, body).await
    }

    pub async fn send_reply(
        &self,
        channel: &str,
        parent_hash: &willow_state::EventHash,
        body: &str,
    ) -> anyhow::Result<()> {
        self.mutation_handle
            .send_reply(channel, parent_hash, body)
            .await
    }

    /// Legacy inline base64 file-share path (256 KB cap).
    ///
    /// **Deprecated.** Use [`Self::upload_attachment`] +
    /// [`Self::send_attachment_message`] instead — those route through
    /// the iroh blob store and the typed
    /// [`willow_state::EventKind::FileMessage`] variant rather than
    /// shoving base64 into a text-message body. The legacy method
    /// stays so historical messages still render and pre-3b client
    /// code keeps compiling.
    #[deprecated(
        since = "0.1.1",
        note = "use upload_attachment + send_attachment_message"
    )]
    pub async fn share_file_inline(
        &self,
        channel: &str,
        filename: &str,
        data: &[u8],
    ) -> anyhow::Result<()> {
        const MAX_INLINE_SIZE: usize = 256 * 1024;
        if data.len() > MAX_INLINE_SIZE {
            anyhow::bail!("file too large for inline sharing (max 256 KB)");
        }
        let encoded = base64::encode(data);
        let body = format!("[file:{}:{}]", filename, encoded);
        self.send_message(channel, &body).await
    }

    /// Upload bytes to the blob store and return the content hash + size.
    ///
    /// The blob store dedupes by hash, so re-uploading the same bytes
    /// returns the same `BlobHash` without re-storing. The hash is
    /// content-addressed; the returned size is the byte length of the
    /// supplied buffer (authoritative, not sender-asserted).
    ///
    /// Errors if the network has not been started — call
    /// [`Self::connect`] first.
    ///
    /// Spec: `docs/specs/2026-04-19-ui-design/files-inline.md`.
    pub async fn upload_attachment(
        &self,
        data: Vec<u8>,
    ) -> anyhow::Result<(willow_network::BlobHash, u64)> {
        let network = self
            .network
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("network not connected"))?;
        let size = data.len() as u64;
        let hash = crate::files::share_file(network.blobs(), data).await?;
        Ok((hash, size))
    }

    /// Publish an [`EventKind::FileMessage`] referencing an
    /// already-uploaded blob hash.
    ///
    /// Pair with [`Self::upload_attachment`]: upload the bytes, then
    /// pass the returned hash here along with the user-facing
    /// metadata (filename, mime, optional dimensions, optional
    /// caption). Receivers subscribe to the channel topic, see the
    /// event, and fetch the bytes via the blob store.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_attachment_message(
        &self,
        channel: &str,
        hash: &willow_network::BlobHash,
        filename: &str,
        mime_type: &str,
        size_bytes: u64,
        width: Option<u32>,
        height: Option<u32>,
        caption: &str,
        reply_to: Option<willow_state::EventHash>,
    ) -> anyhow::Result<()> {
        let hash_hex = blob_hash_to_hex(hash);
        self.mutation_handle
            .send_file_message(
                channel,
                &hash_hex,
                filename,
                mime_type,
                size_bytes,
                width,
                height,
                caption,
                reply_to,
            )
            .await
    }

    pub async fn edit_message(
        &self,
        _channel: &str,
        message_id: &willow_state::EventHash,
        new_body: &str,
    ) -> anyhow::Result<()> {
        self.mutation_handle
            .edit_message(message_id, new_body)
            .await
    }

    pub async fn delete_message(
        &self,
        _channel: &str,
        message_id: &willow_state::EventHash,
    ) -> anyhow::Result<()> {
        self.mutation_handle.delete_message(message_id).await
    }

    pub async fn react(
        &self,
        _channel: &str,
        message_id: &willow_state::EventHash,
        emoji: &str,
    ) -> anyhow::Result<()> {
        self.mutation_handle.react(message_id, emoji).await
    }

    pub async fn pin_message(
        &self,
        channel: &str,
        message_id: &willow_state::EventHash,
    ) -> anyhow::Result<()> {
        self.mutation_handle.pin_message(channel, message_id).await
    }

    pub async fn unpin_message(
        &self,
        channel: &str,
        message_id: &willow_state::EventHash,
    ) -> anyhow::Result<()> {
        self.mutation_handle
            .unpin_message(channel, message_id)
            .await
    }

    pub async fn pinned_message_ids(&self, channel: &str) -> Vec<willow_state::EventHash> {
        let channel = channel.to_string();
        willow_actor::state::select(&self.event_state_addr, move |es| {
            let channel_id = es
                .channels
                .iter()
                .find(|(_, ch)| ch.name == channel)
                .map(|(id, _)| id.clone())
                .unwrap_or_default();
            es.channels
                .get(&channel_id)
                .map(|ch| {
                    let mut ids: Vec<willow_state::EventHash> =
                        ch.pinned_messages.iter().cloned().collect();
                    ids.sort();
                    ids
                })
                .unwrap_or_default()
        })
        .await
    }

    pub async fn pinned_messages(&self, channel: &str) -> Vec<state::DisplayMessage> {
        let pinned_ids = self.pinned_message_ids(channel).await;
        if pinned_ids.is_empty() {
            return vec![];
        }
        let pinned_set: std::collections::HashSet<String> =
            pinned_ids.iter().map(|h| h.to_string()).collect();
        self.messages(channel)
            .await
            .into_iter()
            .filter(|m| pinned_set.contains(&m.id))
            .collect()
    }

    pub async fn is_pinned(&self, channel: &str, message_id: &willow_state::EventHash) -> bool {
        self.pinned_message_ids(channel)
            .await
            .iter()
            .any(|id| id == message_id)
    }

    pub async fn create_channel(&self, name: &str) -> anyhow::Result<()> {
        self.mutation_handle.create_channel(name).await
    }

    /// Create a non-permanent ("ephemeral") channel that
    /// auto-archives after `idle_threshold_ms` of inactivity.
    pub async fn create_ephemeral_channel(
        &self,
        name: &str,
        kind: willow_state::EphemeralKind,
        idle_threshold_ms: u64,
    ) -> anyhow::Result<()> {
        self.mutation_handle
            .create_ephemeral_channel(name, kind, idle_threshold_ms)
            .await
    }

    /// Revive an auto-archived ephemeral channel by name without
    /// posting a message.
    pub async fn revive_channel(&self, name: &str) -> anyhow::Result<()> {
        self.mutation_handle.revive_channel(name).await
    }

    pub async fn create_voice_channel(&self, name: &str) -> anyhow::Result<()> {
        let name = name.to_string();
        let ch_id_str = uuid::Uuid::new_v4().to_string();
        let event = self
            .mutation_handle
            .build_event(willow_state::EventKind::CreateChannel {
                name,
                channel_id: ch_id_str,
                kind: willow_state::ChannelKind::Voice,
                ephemeral: None,
            })
            .await?;
        self.mutation_handle.apply_event(&event).await;
        self.mutation_handle.broadcast_event(&event);
        Ok(())
    }

    pub async fn delete_channel(&self, name: &str) -> anyhow::Result<()> {
        self.mutation_handle.delete_channel(name).await
    }

    pub async fn propose_grant_admin(
        &self,
        peer_id: willow_identity::EndpointId,
    ) -> anyhow::Result<()> {
        self.mutation_handle.propose_grant_admin(peer_id).await
    }

    pub async fn propose_revoke_admin(
        &self,
        peer_id: willow_identity::EndpointId,
    ) -> anyhow::Result<()> {
        self.mutation_handle.propose_revoke_admin(peer_id).await
    }

    pub async fn propose_kick_member(
        &self,
        peer_id: willow_identity::EndpointId,
    ) -> anyhow::Result<()> {
        self.mutation_handle.propose_kick_member(peer_id).await
    }

    pub async fn create_role(&self, name: &str) -> anyhow::Result<()> {
        self.mutation_handle.create_role(name).await
    }

    pub async fn delete_role(&self, role_id: &str) -> anyhow::Result<()> {
        self.mutation_handle.delete_role(role_id).await
    }

    pub async fn set_permission(
        &self,
        role_id: &str,
        permission: willow_state::Permission,
        granted: bool,
    ) -> anyhow::Result<()> {
        let role_id = role_id.to_string();
        let event = self
            .mutation_handle
            .build_event(willow_state::EventKind::SetPermission {
                role_id,
                permission,
                granted,
            })
            .await?;
        self.mutation_handle.apply_event(&event).await;
        self.mutation_handle.broadcast_event(&event);
        Ok(())
    }

    pub async fn assign_role(
        &self,
        peer_id: willow_identity::EndpointId,
        role_id: &str,
    ) -> anyhow::Result<()> {
        let role_id = role_id.to_string();
        let event = self
            .mutation_handle
            .build_event(willow_state::EventKind::AssignRole { peer_id, role_id })
            .await?;
        self.mutation_handle.apply_event(&event).await;
        self.mutation_handle.broadcast_event(&event);
        Ok(())
    }

    /// Switch the current channel.
    pub async fn switch_channel(&self, channel: &str) {
        self.mutation_handle.switch_channel(channel).await;
    }

    /// Toggle per-identity mute on a channel (phase 1f).
    pub async fn mutate_channel_mute(&self, channel: &str, muted: bool) -> anyhow::Result<()> {
        self.mutation_handle
            .mutate_channel_mute(channel, muted)
            .await
    }

    /// Toggle per-identity mute for the active grove (phase 1f).
    pub async fn mutate_grove_mute(&self, muted: bool) -> anyhow::Result<()> {
        self.mutation_handle.mutate_grove_mute(muted).await
    }
}

/// Render a [`willow_network::BlobHash`] as a 64-char lowercase hex
/// string for stamping onto a wire event.
///
/// `BlobHash` is a transparent `[u8; 32]` (BLAKE3 digest). The wire
/// event ([`willow_state::EventKind::FileMessage::hash`]) carries it
/// as a `String` because `willow-state` does not depend on
/// `willow-network`. Hex encoding keeps the value safely round-trippable
/// across that crate boundary; the receiver decodes back via
/// [`hex_to_blob_hash`] before calling `BlobStore::get`.
pub fn blob_hash_to_hex(hash: &willow_network::BlobHash) -> String {
    let mut s = String::with_capacity(64);
    for byte in hash.as_bytes() {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

/// Inverse of [`blob_hash_to_hex`]. Returns `None` on malformed input
/// (non-hex characters or wrong length).
///
/// Re-exported from `crate::lib` so downstream crates (e.g. the web
/// renderer) can decode the wire-event hash before calling
/// [`willow_network::BlobStore::get`].
pub fn hex_to_blob_hash(hex: &str) -> Option<willow_network::BlobHash> {
    if hex.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        let byte_str = hex.get(i * 2..i * 2 + 2)?;
        *byte = u8::from_str_radix(byte_str, 16).ok()?;
    }
    Some(willow_network::BlobHash::from_bytes(bytes))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod blob_hash_hex_tests {
    use super::*;

    #[test]
    fn round_trip_via_hex() {
        let hash = willow_network::BlobHash::new(b"hello world");
        let hex = blob_hash_to_hex(&hash);
        assert_eq!(hex.len(), 64, "BLAKE3 → 64 hex chars");
        let back = hex_to_blob_hash(&hex).expect("round-trip must decode");
        assert_eq!(back, hash);
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(hex_to_blob_hash("abc").is_none());
        assert!(hex_to_blob_hash(&"a".repeat(63)).is_none());
        assert!(hex_to_blob_hash(&"a".repeat(65)).is_none());
    }

    #[test]
    fn rejects_non_hex_chars() {
        assert!(hex_to_blob_hash(&"z".repeat(64)).is_none());
        let mut bad = "0".repeat(63);
        bad.push('Z');
        assert!(hex_to_blob_hash(&bad).is_none());
    }
}
