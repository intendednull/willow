//! Per-topic listener tasks that stream GossipEvents and mutate state via domain actors.

use std::sync::Arc;

use parking_lot::Mutex;

use crate::events::ClientEvent;
use crate::mutations;
use crate::persistence_actor;
use crate::state_actors;
use willow_actor::Addr;
use willow_common::SYNC_BATCH_LIMIT;
use willow_identity::EndpointId;
use willow_network::traits::TopicHandle;
use willow_network::traits::{GossipEvent, TopicEvents};

/// Maximum length (in Unicode scalar values / `char`s) for an attacker-supplied
/// display name accepted into [`state_actors::ProfileState::names`]. Inputs
/// longer than this are truncated on receipt and a `tracing::warn!` is logged.
///
/// We cap by `char` count rather than UTF-8 bytes to keep the limit friendly
/// for non-ASCII names. Worst-case memory per entry stays bounded
/// (`MAX_DISPLAY_NAME_LEN * 4` bytes).
///
/// See `[SEC-V-05]` (issue #234) — full mitigation also requires an LRU /
/// total-entry cap on the map (tracked separately).
pub(crate) const MAX_DISPLAY_NAME_LEN: usize = 128;

/// Maximum length (in `char`s) for an attacker-supplied typing-channel name
/// accepted into [`state_actors::NetworkMeta::typing_peers`]. Same rationale
/// as [`MAX_DISPLAY_NAME_LEN`].
pub(crate) const MAX_TYPING_CHANNEL_LEN: usize = 128;

/// Maximum length (in `char`s) for an attacker-supplied `reason` string on
/// [`crate::ops::WireMessage::JoinDenied`] before it is propagated to the
/// UI via [`crate::events::ClientEvent::JoinLinkDenied`]. The reason is
/// shown to the joining user verbatim, so it is both a phishing surface
/// and a memory-amplification vector — bound it tightly. See `[SEC-A-07]`
/// (issue #309).
pub(crate) const MAX_JOIN_DENIED_REASON_LEN: usize = 256;

/// Maximum number of distinct `channel_id`s tracked in
/// [`state_actors::VoiceState::participants`]. Caps the outer dimension of
/// the map so a malicious peer cannot flood `VoiceJoin` with fresh
/// `channel_id`s and exhaust memory on receiving clients. Real servers
/// have far fewer than this many voice channels in practice; the cap is
/// a defensive ceiling, not an expected operating point. See `[SEC-V-03]`
/// (issue #303).
pub(crate) const MAX_VOICE_CHANNELS: usize = 256;

/// Maximum number of `peer_id`s tracked per voice channel in
/// [`state_actors::VoiceState::participants`]. Caps the inner dimension of
/// the map. Same threat model as [`MAX_VOICE_CHANNELS`]. See `[SEC-V-03]`
/// (issue #303).
pub(crate) const MAX_PARTICIPANTS_PER_CHANNEL: usize = 256;

/// Truncate `s` to at most `max` Unicode scalar values, returning the original
/// string unchanged if it already fits. Splits at `char` boundaries so the
/// returned `String` is always valid UTF-8.
pub(crate) fn truncate_to_chars(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        s
    } else {
        s.chars().take(max).collect()
    }
}

/// Sanitize an attacker-supplied `JoinDenied.reason` string before it is
/// surfaced to the UI: strip control characters (anything matching
/// [`char::is_control`]) and cap the remaining length at
/// [`MAX_JOIN_DENIED_REASON_LEN`]. Stripping control characters first
/// means the cap counts only displayable code points, so the user sees
/// at most `MAX_JOIN_DENIED_REASON_LEN` real characters. See `[SEC-A-07]`
/// (issue #309).
pub(crate) fn sanitize_reason(s: String) -> String {
    let stripped: String = s.chars().filter(|c| !c.is_control()).collect();
    truncate_to_chars(stripped, MAX_JOIN_DENIED_REASON_LEN)
}

/// Log a `tracing::warn!` if `r` is `Err`, otherwise drop the success value.
///
/// Used in this listener hot loop to replace bare `.ok();` calls on
/// "fire-and-forget" sends — broker `do_send`, topic `broadcast`, persistence
/// `do_send`, etc. The previous pattern silently dropped these failures, so a
/// dead broker or broken topic would cause the listener to keep running
/// without any record. We still proceed (the listener should not abort on a
/// downstream failure), but field logs now surface the issue.
///
/// See issue #253.
fn warn_if_err<T, E: std::fmt::Debug>(r: Result<T, E>, context: &'static str) {
    if let Err(e) = r {
        tracing::warn!(?e, "{context}");
    }
}

/// Context passed to listener tasks with all the actor addresses they need.
pub struct ListenerCtx {
    pub event_state: Addr<willow_actor::StateActor<willow_state::ServerState>>,
    pub chat_meta: Addr<willow_actor::StateActor<state_actors::ChatMeta>>,
    pub profiles: Addr<willow_actor::StateActor<state_actors::ProfileState>>,
    pub network: Addr<willow_actor::StateActor<state_actors::NetworkMeta>>,
    pub voice: Addr<willow_actor::StateActor<state_actors::VoiceState>>,
    pub persistence: Addr<persistence_actor::PersistenceActor>,
    /// Whether persistence to disk is enabled.
    pub persistence_enabled: bool,
    pub event_broker: Addr<willow_actor::Broker<ClientEvent>>,
    pub identity: willow_identity::Identity,
    // state: lock-ok — mirrors `ClientHandle.join_links`; actor migration
    // tracked in docs/specs/2026-04-26-state-management-model-design.md § F4.
    pub join_links: Arc<Mutex<Vec<crate::ops::JoinLink>>>,
    /// Mirrors `ClientHandle.pending_joins`; required for the SEC-A-07
    /// signer-binding check on `JoinResponse` / `JoinDenied` (issue
    /// #309).
    // state: lock-ok — same rationale as `join_links`.
    pub pending_joins: Arc<Mutex<std::collections::HashMap<String, EndpointId>>>,
    pub dag: Addr<willow_actor::StateActor<state_actors::DagState>>,
    pub server_registry: Addr<willow_actor::StateActor<state_actors::ServerRegistry>>,
    /// Optional callback invoked on `NeighborUp`. Used by the profile topic
    /// listener to re-broadcast the local profile when a new peer joins the
    /// gossip mesh — ensuring late-arriving peers see the display name even
    /// when the initial broadcast happened before the link was established.
    pub on_neighbor_up: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Clone for ListenerCtx {
    fn clone(&self) -> Self {
        Self {
            event_state: self.event_state.clone(),
            chat_meta: self.chat_meta.clone(),
            profiles: self.profiles.clone(),
            network: self.network.clone(),
            voice: self.voice.clone(),
            persistence: self.persistence.clone(),
            persistence_enabled: self.persistence_enabled,
            event_broker: self.event_broker.clone(),
            identity: self.identity.clone(),
            join_links: Arc::clone(&self.join_links),
            pending_joins: Arc::clone(&self.pending_joins),
            dag: self.dag.clone(),
            server_registry: self.server_registry.clone(),
            on_neighbor_up: self.on_neighbor_up.clone(),
        }
    }
}

/// Spawn an async task that listens for gossip events on a topic.
pub fn spawn_topic_listener<T: TopicHandle + 'static, E: TopicEvents + 'static>(
    events: E,
    topic: T,
    ctx: ListenerCtx,
) {
    #[cfg(not(target_arch = "wasm32"))]
    tokio::task::spawn_local(topic_listener_loop(events, topic, ctx));

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(topic_listener_loop(events, topic, ctx));
}

async fn topic_listener_loop<T: TopicHandle, E: TopicEvents>(
    mut events: E,
    topic: T,
    ctx: ListenerCtx,
) {
    while let Some(Ok(gossip_event)) = events.next().await {
        match gossip_event {
            GossipEvent::Received(msg) => {
                process_received_message(&msg.content, msg.sender, &ctx, &topic).await;
            }
            GossipEvent::NeighborUp(id) => {
                let id2 = id;
                willow_actor::state::mutate(&ctx.chat_meta, move |c| {
                    if !c.peers.contains(&id2) {
                        c.peers.push(id2);
                    }
                })
                .await;
                warn_if_err(
                    ctx.event_broker
                        .do_send(willow_actor::Publish(ClientEvent::PeerConnected(id))),
                    "event_broker.do_send Publish(PeerConnected)",
                );
                // Re-broadcast local profile so the newly joined peer gets
                // our display name even if they missed the initial broadcast.
                if let Some(cb) = &ctx.on_neighbor_up {
                    cb();
                }
            }
            GossipEvent::NeighborDown(id) => {
                let id2 = id;
                willow_actor::state::mutate(&ctx.chat_meta, move |c| {
                    c.peers.retain(|p| p != &id2);
                })
                .await;
                warn_if_err(
                    ctx.event_broker
                        .do_send(willow_actor::Publish(ClientEvent::PeerDisconnected(id))),
                    "event_broker.do_send Publish(PeerDisconnected)",
                );
            }
        }
    }
}

// ───── DAG helpers ──────────────────────────────────────────────────────────

/// Compute the cached `WireMessage::SyncRequest` reply payload — the
/// first [`SYNC_REPLY_LIMIT`](state_actors::SYNC_REPLY_LIMIT) events of
/// the DAG's deterministic topological sort. Cache populated lazily on
/// first read after invalidation; cleared by every successful insertion
/// path on `DagState`. See GEN-08 / issue #268.
pub(crate) async fn compute_sync_reply(
    dag: &Addr<willow_actor::StateActor<state_actors::DagState>>,
) -> Vec<willow_state::Event> {
    willow_actor::state::mutate(dag, |ds| ds.sync_reply_events()).await
}

/// Try to insert an event into the DAG. On success, ManagedDag atomically
/// applies it to state and resolves pending events. On chain gap, the
/// event is buffered. Duplicates are silently ignored.
///
/// Atomicity is guaranteed by ManagedDag::insert_and_apply — DAG insert
/// and state materialization happen in the same call.
async fn try_insert_event(ctx: &ListenerCtx, event: willow_state::Event) {
    // ManagedDag handles insert + apply + pending resolution atomically.
    let (applied, all_applied) = willow_actor::state::mutate(&ctx.dag, move |ds| {
        match ds.managed.insert_and_apply(event) {
            Ok(outcome) => {
                let mut all = Vec::new();
                if let Some(ref ev) = outcome.applied {
                    all.push(ev.clone());
                }
                for r in &outcome.resolved {
                    all.push(r.clone());
                }
                // Any event entering the DAG (including chains drained
                // from the pending buffer) changes the topological-sort
                // prefix, so invalidate the SyncRequest-reply cache.
                // See GEN-08 / issue #268.
                if outcome.applied.is_some() || !outcome.resolved.is_empty() {
                    ds.invalidate_sync_reply_cache();
                }
                (outcome.applied, all)
            }
            Err(willow_state::InsertError::PrevMismatch {
                author,
                expected,
                got,
            }) => {
                tracing::warn!(
                    %author, %expected, %got,
                    "PrevMismatch: equivocation or conflicting chain — dropping event"
                );
                (None, vec![])
            }
            Err(err) => {
                tracing::warn!("DAG insert error: {err}");
                (None, vec![])
            }
        }
    })
    .await;

    // Sync state and emit side-effects for all applied events.
    if applied.is_some() {
        // Sync state once (ManagedDag already applied all resolved events).
        let state = willow_actor::state::select(&ctx.dag, |ds| ds.managed.state().clone()).await;
        willow_actor::state::mutate(&ctx.event_state, move |es| {
            *es = state;
        })
        .await;

        // Persist and emit for each applied event.
        for ev in &all_applied {
            warn_if_err(
                ctx.persistence
                    .do_send(persistence_actor::PersistEvent { event: ev.clone() }),
                "persistence.do_send PersistEvent",
            );
            let client_events = mutations::derive_client_events(ev);
            for e in client_events {
                warn_if_err(
                    ctx.event_broker.do_send(willow_actor::Publish(e)),
                    "event_broker.do_send Publish(derived ClientEvent)",
                );
            }
        }
        // Persist the state snapshot so messages survive a page reload
        // without requiring a network sync round-trip. Use a direct synchronous
        // storage write so the snapshot is on disk before this task yields —
        // fire-and-forget actor messages may be delayed past a page reload.
        // Key by registry UUID (not ServerState.server_id which is the genesis
        // hash) so that load_server_state() can find the saved snapshot.
        if ctx.persistence_enabled {
            let snapshot_state =
                willow_actor::state::select(&ctx.dag, |ds| ds.managed.state().clone()).await;
            let registry_id =
                willow_actor::state::select(&ctx.server_registry, |reg| reg.active_server.clone())
                    .await;
            if let Some(id) = registry_id {
                crate::storage::save_server_state(&id, &snapshot_state);
            }
        }
    }
}

// ───── Message processing ───────────────────────────────────────────────────

async fn process_received_message<T: TopicHandle>(
    data: &[u8],
    _sender: EndpointId,
    ctx: &ListenerCtx,
    topic: &T,
) {
    // Try profile broadcast first. Profile envelopes are Ed25519-signed
    // and we reject anything where the signer doesn't match the claimed
    // `peer_id` — otherwise any peer could spoof another peer's display
    // name. See issue #145.
    match willow_identity::unpack_profile(data) {
        Ok(profile) => {
            let peer_id = profile.peer_id;
            let display_name = profile.display_name.clone();
            willow_actor::state::mutate(&ctx.profiles, move |p| {
                p.insert_name(peer_id, display_name);
            })
            .await;
            warn_if_err(
                ctx.event_broker
                    .do_send(willow_actor::Publish(ClientEvent::ProfileUpdated {
                        peer_id: profile.peer_id,
                        display_name: profile.display_name,
                    })),
                "event_broker.do_send Publish(ProfileUpdated from profile broadcast)",
            );
            return;
        }
        Err(willow_identity::IdentityError::PeerMismatch { claimed, signer }) => {
            tracing::warn!(
                %claimed,
                %signer,
                "rejecting spoofed profile broadcast: signer does not match claimed peer_id"
            );
            return;
        }
        Err(_) => {
            // Not a profile envelope — fall through to the WireMessage
            // path below.
        }
    }

    let Some((wire_msg, signer)) = crate::ops::unpack_wire(data) else {
        return;
    };

    match wire_msg {
        crate::ops::WireMessage::Event(event) => {
            let event = *event;
            // Verify the event author matches the signer.
            if event.author != signer {
                return;
            }
            // Track peer.
            let signer2 = signer;
            willow_actor::state::mutate(&ctx.chat_meta, move |c| {
                if !c.peers.contains(&signer2) {
                    c.peers.push(signer2);
                }
            })
            .await;
            // Insert into DAG (handles dedup, ordering, buffering).
            try_insert_event(ctx, event).await;
        }
        crate::ops::WireMessage::SyncBatch { events: batch } => {
            // Reject oversized batches to prevent memory exhaustion.
            // Bound matches the producer side in `willow-storage`; the
            // shared constant lives in `willow_common::SYNC_BATCH_LIMIT`.
            if batch.len() > SYNC_BATCH_LIMIT {
                tracing::warn!(
                    size = batch.len(),
                    "rejecting oversized sync batch (max {})",
                    SYNC_BATCH_LIMIT
                );
                return;
            }
            // Track peer.
            let signer2 = signer;
            willow_actor::state::mutate(&ctx.chat_meta, move |c| {
                if !c.peers.contains(&signer2) {
                    c.peers.push(signer2);
                }
            })
            .await;
            let count = batch.len();
            // Insert each event into the DAG. The DAG enforces per-author
            // chain ordering; out-of-order events are buffered automatically.
            for event in batch {
                try_insert_event(ctx, event).await;
            }
            if count > 0 {
                // ManagedDag automatically marks synced when genesis arrives.
                // Just check if we need to signal completion.
                let _is_synced =
                    willow_actor::state::select(&ctx.dag, |ds| ds.managed.is_synced()).await;

                warn_if_err(
                    ctx.event_broker
                        .do_send(willow_actor::Publish(ClientEvent::SyncCompleted {
                            ops_applied: count,
                        })),
                    "event_broker.do_send Publish(SyncCompleted)",
                );
            }
        }
        crate::ops::WireMessage::SyncRequest { state_hash, .. } => {
            let _ = state_hash; // Legacy field — can't filter by state hash in DAG model.
                                // TODO(#382): Migrate clients to worker's heads-based sync
                                // protocol (WorkerRequest::Sync { heads }) for efficient delta
                                // sync. For now, send the first SYNC_REPLY_LIMIT events from
                                // topological sort. Receiver will dedup via InsertError::Duplicate.
                                // The reply Vec is cached on `DagState` and invalidated on
                                // every successful DAG insert; see GEN-08 / issue #268.
            let events = compute_sync_reply(&ctx.dag).await;
            if !events.is_empty() {
                let msg = crate::ops::WireMessage::SyncBatch { events };
                if let Some(data) = crate::ops::pack_wire(&msg, &ctx.identity) {
                    warn_if_err(
                        topic.broadcast(bytes::Bytes::from(data)).await,
                        "topic.broadcast SyncBatch (SyncRequest reply)",
                    );
                }
            }
        }
        // Heads-based delta-sync variants (negentropy-sync spec). The wire
        // types + state/storage queries land in PR 3; the gossip
        // responder/receiver cutover and the SyncProvider serving gate are
        // PR 4. Until then a peer that already speaks V2 is logged and ignored
        // here — the additive variants must not break the existing gossip path,
        // and the legacy SyncRequest/SyncBatch arms above keep serving sync
        // through the migration window.
        crate::ops::WireMessage::SyncRequestV2 { request_id, .. } => {
            tracing::debug!(
                %request_id,
                %signer,
                "received SyncRequestV2 (heads-based sync); responder lands in PR 4 — ignoring"
            );
        }
        crate::ops::WireMessage::SyncBatchV2 {
            request_id,
            events,
            more,
        } => {
            tracing::debug!(
                %request_id,
                more,
                count = events.len(),
                %signer,
                "received SyncBatchV2 (heads-based sync); receiver lands in PR 4 — ignoring"
            );
        }
        crate::ops::WireMessage::TypingIndicator { channel } => {
            // Bound attacker-supplied input. See `MAX_TYPING_CHANNEL_LEN`
            // and issue #234 ([SEC-V-05]).
            let original_len = channel.chars().count();
            let channel = truncate_to_chars(channel, MAX_TYPING_CHANNEL_LEN);
            if original_len > MAX_TYPING_CHANNEL_LEN {
                tracing::warn!(
                    %signer, original_len, capped_len = MAX_TYPING_CHANNEL_LEN,
                    "TypingIndicator channel exceeded cap — truncating"
                );
            }
            let now = crate::util::current_time_ms();
            willow_actor::state::mutate(&ctx.network, move |n| {
                n.insert_typing(signer, channel, now);
            })
            .await;
            let signer2 = signer;
            willow_actor::state::mutate(&ctx.chat_meta, move |c| {
                if !c.peers.contains(&signer2) {
                    c.peers.push(signer2);
                }
            })
            .await;
        }
        crate::ops::WireMessage::VoiceJoin {
            channel_id,
            peer_id,
        } => {
            // [SEC-V-03] / #303: drop voice messages whose `channel_id`
            // does not exist in the materialized server state, and bound
            // both the outer (distinct channels) and inner (participants
            // per channel) dimensions of `VoiceState.participants`.
            // Without these gates, any signed peer can flood arbitrary
            // `channel_id`s and exhaust receiving clients' memory.
            let known = willow_actor::state::select(&ctx.event_state, {
                let ch = channel_id.clone();
                move |es| es.channels.contains_key(&ch)
            })
            .await;
            if !known {
                tracing::debug!(
                    %signer, channel_id = %channel_id,
                    "dropping VoiceJoin: channel_id not in ServerState.channels"
                );
                return;
            }
            let ch = channel_id.clone();
            let inserted = willow_actor::state::mutate(&ctx.voice, move |v| {
                let new_channel = !v.participants.contains_key(&ch);
                if new_channel && v.participants.len() >= MAX_VOICE_CHANNELS {
                    return false;
                }
                let set = v.participants.entry(ch).or_default();
                if !set.contains(&peer_id) && set.len() >= MAX_PARTICIPANTS_PER_CHANNEL {
                    return false;
                }
                set.insert(peer_id);
                true
            })
            .await;
            if !inserted {
                tracing::debug!(
                    %signer, channel_id = %channel_id, %peer_id,
                    "dropping VoiceJoin: VoiceState cap reached"
                );
                return;
            }
            warn_if_err(
                ctx.event_broker
                    .do_send(willow_actor::Publish(ClientEvent::VoiceJoined {
                        channel_id,
                        peer_id,
                    })),
                "event_broker.do_send Publish(VoiceJoined)",
            );
        }
        crate::ops::WireMessage::VoiceLeave {
            channel_id,
            peer_id,
        } => {
            // [SEC-V-03] / #303: drop Leaves for unknown channels so the
            // event broker never publishes phantom-channel events to the
            // UI. The `participants` map is unaffected even without the
            // gate (`get_mut` returns None), but the event-broker fanout
            // would still leak attacker-controlled `channel_id`s.
            let known = willow_actor::state::select(&ctx.event_state, {
                let ch = channel_id.clone();
                move |es| es.channels.contains_key(&ch)
            })
            .await;
            if !known {
                tracing::debug!(
                    %signer, channel_id = %channel_id,
                    "dropping VoiceLeave: channel_id not in ServerState.channels"
                );
                return;
            }
            let ch = channel_id.clone();
            willow_actor::state::mutate(&ctx.voice, move |v| {
                if let Some(p) = v.participants.get_mut(&ch) {
                    p.remove(&peer_id);
                }
            })
            .await;
            warn_if_err(
                ctx.event_broker
                    .do_send(willow_actor::Publish(ClientEvent::VoiceLeft {
                        channel_id,
                        peer_id,
                    })),
                "event_broker.do_send Publish(VoiceLeft)",
            );
        }
        crate::ops::WireMessage::VoiceSignal {
            channel_id,
            target_peer,
            signal,
        } => {
            if target_peer == ctx.identity.endpoint_id() {
                // [SEC-V-03] / #303: drop signals for unknown channels so
                // attacker-supplied `channel_id`s never reach the UI's
                // event handlers. Signals do not mutate `participants`
                // directly, but the existence check shuts the same fanout
                // gap as Join/Leave.
                let known = willow_actor::state::select(&ctx.event_state, {
                    let ch = channel_id.clone();
                    move |es| es.channels.contains_key(&ch)
                })
                .await;
                if !known {
                    tracing::debug!(
                        %signer, channel_id = %channel_id,
                        "dropping VoiceSignal: channel_id not in ServerState.channels"
                    );
                    return;
                }
                warn_if_err(
                    ctx.event_broker
                        .do_send(willow_actor::Publish(ClientEvent::VoiceSignal {
                            channel_id,
                            from_peer: signer,
                            signal,
                        })),
                    "event_broker.do_send Publish(VoiceSignal)",
                );
            }
        }
        crate::ops::WireMessage::JoinRequest { link_id, peer_id } => {
            // Security: the wire-supplied `peer_id` is attacker-controlled
            // and must never be trusted as a grant target. The only
            // authenticated peer identity we have is `signer`, which was
            // verified against the envelope's Ed25519 signature by
            // `unpack_wire`. Reject any request where the two disagree,
            // matching the ProfileAnnounce spoof guard above. See issue
            // #239 / SEC-A-03.
            if peer_id != signer {
                tracing::warn!(
                    %peer_id,
                    %signer,
                    "rejecting JoinRequest: wire-supplied peer_id does not match verified signer"
                );
                return;
            }
            let _ = peer_id; // the verified `signer` is the grant target below
                             // Three possible outcomes:
                             //   - Link valid → bump `used`, send `JoinResponse`.
                             //   - Link found but not currently usable (disabled / expired /
                             //     exhausted) → reply with `JoinDenied { reason }` so the
                             //     joiner sees a concrete error instead of an opaque timeout.
                             //   - Link not found → drop silently. This is the
                             //     anti-enumeration property: if the inviter never created
                             //     this `link_id`, attackers cannot probe by trial. Only
                             //     `link_id`s that *exist* (and that this peer therefore
                             //     plausibly created) receive a denial; everything else is
                             //     indistinguishable from "wrong inviter."
            #[derive(Clone)]
            enum LinkOutcome {
                Respond,
                Deny(&'static str),
                Silent,
            }
            // Track which server we charged the use against so we can
            // persist the bumped `used` count once we drop the lock —
            // otherwise the counter resets on restart and an exhausted
            // link looks fresh again.
            let mut bumped_server: Option<String> = None;
            let outcome = {
                let mut links = ctx.join_links.lock();
                if let Some(link) = links.iter_mut().find(|l| l.link_id == link_id) {
                    if link.is_valid() {
                        link.used += 1;
                        bumped_server = Some(link.server_id.clone());
                        LinkOutcome::Respond
                    } else if !link.active {
                        LinkOutcome::Deny("link_disabled")
                    } else {
                        // Either `used >= max_uses` or `expires_at` is in
                        // the past — spec collapses both under `link_expired`.
                        LinkOutcome::Deny("link_expired")
                    }
                } else {
                    LinkOutcome::Silent
                }
            };
            if let Some(sid) = bumped_server {
                let snapshot: Vec<crate::ops::JoinLink> = ctx
                    .join_links
                    .lock()
                    .iter()
                    .filter(|l| l.server_id == sid)
                    .cloned()
                    .collect();
                warn_if_err(
                    ctx.persistence
                        .do_send(crate::persistence_actor::PersistJoinLinks {
                            server_id: sid,
                            links: snapshot,
                        }),
                    "persistence.do_send PersistJoinLinks (used++)",
                );
            }
            let should_respond = matches!(outcome, LinkOutcome::Respond);
            if let LinkOutcome::Deny(reason) = &outcome {
                let msg = crate::ops::WireMessage::JoinDenied {
                    link_id: link_id.clone(),
                    target_peer: signer,
                    reason: (*reason).to_string(),
                };
                if let Some(data) = crate::ops::pack_wire(&msg, &ctx.identity) {
                    warn_if_err(
                        topic.broadcast(bytes::Bytes::from(data)).await,
                        "topic.broadcast JoinDenied",
                    );
                }
            }
            if should_respond {
                // Generate invite for the requesting peer using event_state + registry.
                let server_registry = ctx.server_registry.clone();
                let event_state = ctx.event_state.clone();
                // Use the verified signer as the grant target — never the
                // attacker-supplied `peer_id` from the wire message.
                let peer_endpoint = signer;

                // Get server info from registry.
                let reg_info = willow_actor::state::select(&server_registry, move |reg| {
                    reg.active().map(|entry| {
                        (
                            entry.name.clone(),
                            entry.server_id.clone(),
                            entry.keys.clone(),
                        )
                    })
                })
                .await;

                let invite_result = if let Some((server_name, server_id, keys)) = reg_info {
                    // Build topic_names and get owner from event_state.
                    let sid = server_id.clone();
                    let fallback_id = ctx.identity.endpoint_id();
                    let (topic_names, genesis_author) =
                        willow_actor::state::select(&event_state, move |es| {
                            let names: std::collections::HashMap<String, String> = es
                                .channels
                                .values()
                                .map(|ch| {
                                    let topic = crate::util::make_topic(&sid, &ch.name);
                                    (topic, ch.name.clone())
                                })
                                .collect();
                            // Same correctness fix as in `generate_invite`
                            // (issue #241): pick the authoritative genesis
                            // author rather than an arbitrary admin.
                            let author = es.genesis_author.unwrap_or(fallback_id);
                            (names, author)
                        })
                        .await;

                    let pub_key = crate::invite::endpoint_id_to_ed25519_public(&peer_endpoint);
                    crate::invite::generate_invite(
                        &server_name,
                        &server_id,
                        genesis_author,
                        &keys,
                        &topic_names,
                        &pub_key,
                    )
                } else {
                    None
                };
                if let Some(invite_data) = invite_result {
                    let msg = crate::ops::WireMessage::JoinResponse {
                        link_id: link_id.clone(),
                        target_peer: peer_endpoint,
                        invite_data,
                    };
                    if let Some(data) = crate::ops::pack_wire(&msg, &ctx.identity) {
                        warn_if_err(
                            topic.broadcast(bytes::Bytes::from(data)).await,
                            "topic.broadcast JoinResponse",
                        );
                    }

                    // Grant SendMessages permission to the joining peer so
                    // they can send messages, reactions, etc. This creates
                    // a signed event in the inviter's DAG and broadcasts it
                    // so all peers (including the new joiner) learn that
                    // the new member has send permission.
                    let identity = ctx.identity.clone();
                    // Grant permission to the authenticated signer — never
                    // the wire-supplied peer_id (issue #239 / SEC-A-03).
                    let granted_peer = peer_endpoint;
                    let ts = crate::util::current_time_ms();
                    let grant_event = willow_actor::state::mutate(&ctx.dag, move |ds| {
                        let ev = ds
                            .managed
                            .create_and_insert(
                                &identity,
                                willow_state::EventKind::GrantPermission {
                                    peer_id: granted_peer,
                                    permission: willow_state::Permission::SendMessages,
                                },
                                ts,
                            )
                            .ok();
                        if ev.is_some() {
                            // SyncRequest-reply cache must be invalidated on every
                            // successful insertion path; see GEN-08 / issue #268.
                            ds.invalidate_sync_reply_cache();
                        }
                        ev
                    })
                    .await;
                    if let Some(event) = grant_event {
                        // Sync event_state mirror from the DAG.
                        let new_state =
                            willow_actor::state::select(&ctx.dag, |ds| ds.managed.state().clone())
                                .await;
                        willow_actor::state::mutate(&ctx.event_state, move |es| {
                            *es = new_state;
                        })
                        .await;
                        // Persist.
                        warn_if_err(
                            ctx.persistence
                                .do_send(crate::persistence_actor::PersistEvent {
                                    event: event.clone(),
                                }),
                            "persistence.do_send PersistEvent (GrantPermission)",
                        );
                        // Broadcast to other peers.
                        if let Some(data) = crate::ops::pack_wire(
                            &crate::ops::WireMessage::Event(Box::new(event)),
                            &ctx.identity,
                        ) {
                            warn_if_err(
                                topic.broadcast(bytes::Bytes::from(data)).await,
                                "topic.broadcast Event(GrantPermission)",
                            );
                        }
                    }
                }
            }
        }
        crate::ops::WireMessage::JoinResponse {
            link_id,
            target_peer,
            invite_data,
        } => {
            // Three independent gates — all must pass before we surface
            // the response to the UI:
            //   1. `target_peer` is us (cheap rejection of unrelated traffic).
            //   2. We have an outstanding join attempt for this `link_id`.
            //   3. The packet was signed by the inviter we sent the
            //      original `JoinRequest` to.
            // Without (3), any peer that observed the `JoinRequest` (or
            // simply guessed our `target_peer`) could spoof a response
            // and force us to attempt invite decryption — a DoS surface
            // even though `invite_data` is encrypted to our pubkey. See
            // issue #309 / SEC-A-07.
            if target_peer != ctx.identity.endpoint_id() {
                return;
            }
            let expected_inviter = {
                let mut pending = ctx.pending_joins.lock();
                pending.remove(&link_id)
            };
            let Some(expected_inviter) = expected_inviter else {
                tracing::debug!(
                    %signer, %link_id,
                    "dropping JoinResponse: no outstanding join request for this link_id"
                );
                return;
            };
            if signer != expected_inviter {
                tracing::debug!(
                    %signer, %expected_inviter, %link_id,
                    "dropping JoinResponse: signer is not the expected inviter"
                );
                // Reinsert so a later legitimate response from the real
                // inviter still resolves the join attempt.
                ctx.pending_joins.lock().insert(link_id, expected_inviter);
                return;
            }
            warn_if_err(
                ctx.event_broker
                    .do_send(willow_actor::Publish(ClientEvent::JoinLinkResponse {
                        invite_data,
                    })),
                "event_broker.do_send Publish(JoinLinkResponse)",
            );
        }
        crate::ops::WireMessage::JoinDenied {
            link_id,
            target_peer,
            reason,
        } => {
            // Same three-gate check as `JoinResponse`. The unencrypted
            // `reason` makes spoofing especially dangerous here: an
            // attacker could inject a phishing string ("invite link
            // expired — please re-enter your seed phrase at evil.example")
            // that the UI shows verbatim. We additionally sanitize the
            // reason (control chars stripped, length capped) before
            // surfacing it, even when the signer check passes — defense
            // in depth against a misbehaving but otherwise legitimate
            // inviter. See issue #309 / SEC-A-07.
            if target_peer != ctx.identity.endpoint_id() {
                return;
            }
            let expected_inviter = {
                let mut pending = ctx.pending_joins.lock();
                pending.remove(&link_id)
            };
            let Some(expected_inviter) = expected_inviter else {
                tracing::debug!(
                    %signer, %link_id,
                    "dropping JoinDenied: no outstanding join request for this link_id"
                );
                return;
            };
            if signer != expected_inviter {
                tracing::debug!(
                    %signer, %expected_inviter, %link_id,
                    "dropping JoinDenied: signer is not the expected inviter"
                );
                ctx.pending_joins.lock().insert(link_id, expected_inviter);
                return;
            }
            let reason = sanitize_reason(reason);
            warn_if_err(
                ctx.event_broker
                    .do_send(willow_actor::Publish(ClientEvent::JoinLinkDenied {
                        reason,
                    })),
                "event_broker.do_send Publish(JoinLinkDenied)",
            );
        }
        // TopicAnnounce is consumed by the relay; clients ignore it.
        crate::ops::WireMessage::TopicAnnounce { .. } => {}
        // Update the local ProfileState with the sender's display name.
        // Sent on SERVER_OPS_TOPIC so delivery is guaranteed via the sync path.
        crate::ops::WireMessage::ProfileAnnounce { display_name } => {
            let peer_id = signer;
            // Bound attacker-supplied input. See `MAX_DISPLAY_NAME_LEN`
            // and issue #234 ([SEC-V-05]).
            let original_len = display_name.chars().count();
            let display_name = truncate_to_chars(display_name, MAX_DISPLAY_NAME_LEN);
            if original_len > MAX_DISPLAY_NAME_LEN {
                tracing::warn!(
                    %peer_id, original_len, capped_len = MAX_DISPLAY_NAME_LEN,
                    "ProfileAnnounce display_name exceeded cap — truncating"
                );
            }
            let name = display_name.clone();
            willow_actor::state::mutate(&ctx.profiles, move |p| {
                p.insert_name(peer_id, name);
            })
            .await;
            warn_if_err(
                ctx.event_broker
                    .do_send(willow_actor::Publish(ClientEvent::ProfileUpdated {
                        peer_id,
                        display_name,
                    })),
                "event_broker.do_send Publish(ProfileUpdated from ProfileAnnounce)",
            );
        }
        // Worker messages travel on the _willow_workers topic and are never
        // delivered to the client's server-ops listener.
        crate::ops::WireMessage::Worker(_) => {}
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    //! Listener tests for the JoinRequest signer guard (SEC-A-03 / #239).
    use super::*;
    use crate::{test_client, ClientHandle};
    use std::sync::Arc;
    use willow_network::Network;

    /// A `JoinRequest` whose wire-supplied `peer_id` disagrees with the
    /// Ed25519 `signer` must be rejected. Concretely: no `GrantPermission`
    /// event should be emitted for either identity, and no `JoinResponse`
    /// should be broadcast.
    ///
    /// Without the fix, the listener trusts `peer_id` from the wire and
    /// hands the authoritative invite + `SendMessages` grant to whichever
    /// identity the attacker names — even though the packet was signed by
    /// someone else entirely.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_request_with_mismatched_peer_id_is_rejected() {
        let (client, _rx) = test_client();

        // Inject a join link so the listener would otherwise be willing
        // to respond. Expires far in the future.
        let link_id = "test-link-id".to_string();
        {
            let mut links = client.join_links.lock();
            links.push(crate::ops::JoinLink {
                link_id: link_id.clone(),
                server_id: "unused".into(),
                max_uses: 10,
                used: 0,
                active: true,
                expires_at: None,
                created_at: 0,
            });
        }

        // Construct a MemNetwork + topic handle to satisfy the
        // TopicHandle generic expected by process_received_message.
        let hub = willow_network::mem::MemHub::new();
        let net = willow_network::mem::MemNetwork::new(&hub);
        let (topic_handle, _events) = net
            .subscribe(willow_network::topic_id("unit-test-topic"), vec![])
            .await
            .expect("subscribe must succeed in test");

        // Build a ListenerCtx from the ClientHandle.
        let ctx = ListenerCtx {
            event_state: client.event_state_addr.clone(),
            chat_meta: client.chat_meta_addr.clone(),
            profiles: client.profile_state_addr.clone(),
            network: client.network_meta_addr.clone(),
            voice: client.voice_state_addr.clone(),
            persistence: client.persistence_addr.clone(),
            persistence_enabled: false,
            event_broker: client.event_broker.clone(),
            identity: client.identity.clone(),
            join_links: Arc::clone(&client.join_links),
            pending_joins: Arc::clone(&client.pending_joins),
            dag: client.dag_addr.clone(),
            server_registry: client.server_registry_addr.clone(),
            on_neighbor_up: None,
        };

        // Mallory signs the packet, but claims it's from Bob. Neither
        // identity is the inviter/client.
        let mallory = willow_identity::Identity::generate();
        let bob_id = willow_identity::Identity::generate().endpoint_id();
        let msg = crate::ops::WireMessage::JoinRequest {
            link_id: link_id.clone(),
            peer_id: bob_id,
        };
        let bytes = crate::ops::pack_wire(&msg, &mallory).expect("pack_wire must succeed");

        let sender = mallory.endpoint_id();
        process_received_message(&bytes, sender, &ctx, &topic_handle).await;

        // Give any actor tasks a moment to settle.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Neither Bob (the attacker-named target) nor Mallory (the real
        // signer) should have been granted SendMessages — the listener
        // must drop the packet entirely on signer/peer_id mismatch.
        let mallory_id = mallory.endpoint_id();
        let (bob_has, mallory_has) =
            willow_actor::state::select(&client.event_state_addr, move |es| {
                (
                    es.has_permission(&bob_id, &willow_state::Permission::SendMessages),
                    es.has_permission(&mallory_id, &willow_state::Permission::SendMessages),
                )
            })
            .await;
        assert!(
            !bob_has,
            "spoofed peer_id must not receive SendMessages grant"
        );
        assert!(
            !mallory_has,
            "signer must not receive SendMessages grant when peer_id mismatches"
        );

        // The join link's `used` counter must not have been incremented
        // since the request was rejected before the link was consumed.
        let used = {
            let links = client.join_links.lock();
            links
                .iter()
                .find(|l| l.link_id == link_id)
                .map(|l| l.used)
                .expect("link must still be present")
        };
        assert_eq!(used, 0, "rejected requests must not consume the join link");
    }

    /// When signer and wire peer_id agree, the listener grants
    /// SendMessages to that verified peer. This is the positive
    /// counterpart of the mismatch test above.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_request_with_matching_signer_grants_send_messages() {
        let (client, _rx) = test_client();

        let link_id = "match-link".to_string();
        {
            let mut links = client.join_links.lock();
            links.push(crate::ops::JoinLink {
                link_id: link_id.clone(),
                server_id: "unused".into(),
                max_uses: 10,
                used: 0,
                active: true,
                expires_at: None,
                created_at: 0,
            });
        }

        let hub = willow_network::mem::MemHub::new();
        let net = willow_network::mem::MemNetwork::new(&hub);
        let (topic_handle, _events) = net
            .subscribe(willow_network::topic_id("unit-test-topic-2"), vec![])
            .await
            .expect("subscribe must succeed");

        let ctx = ListenerCtx {
            event_state: client.event_state_addr.clone(),
            chat_meta: client.chat_meta_addr.clone(),
            profiles: client.profile_state_addr.clone(),
            network: client.network_meta_addr.clone(),
            voice: client.voice_state_addr.clone(),
            persistence: client.persistence_addr.clone(),
            persistence_enabled: false,
            event_broker: client.event_broker.clone(),
            identity: client.identity.clone(),
            join_links: Arc::clone(&client.join_links),
            pending_joins: Arc::clone(&client.pending_joins),
            dag: client.dag_addr.clone(),
            server_registry: client.server_registry_addr.clone(),
            on_neighbor_up: None,
        };

        let bob = willow_identity::Identity::generate();
        let bob_id = bob.endpoint_id();
        let msg = crate::ops::WireMessage::JoinRequest {
            link_id: link_id.clone(),
            peer_id: bob_id,
        };
        let bytes = crate::ops::pack_wire(&msg, &bob).expect("pack_wire must succeed");

        process_received_message(&bytes, bob_id, &ctx, &topic_handle).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let bob_has = willow_actor::state::select(&client.event_state_addr, move |es| {
            es.has_permission(&bob_id, &willow_state::Permission::SendMessages)
        })
        .await;
        assert!(
            bob_has,
            "verified signer must be granted SendMessages via join link"
        );
    }

    // ─── [SEC-V-05] / #234 — bound attacker-supplied strings ───────────────

    #[test]
    fn truncate_to_chars_passes_short_input_unchanged() {
        let s = "hello".to_string();
        assert_eq!(truncate_to_chars(s.clone(), 128), s);
    }

    #[test]
    fn truncate_to_chars_caps_long_input_at_char_boundary() {
        // 200 ASCII chars → cap at 128.
        let s: String = "x".repeat(200);
        let out = truncate_to_chars(s, 128);
        assert_eq!(out.chars().count(), 128);
        assert_eq!(out.len(), 128); // ASCII so bytes == chars
    }

    #[test]
    fn truncate_to_chars_handles_multibyte_at_boundary() {
        // 200 four-byte chars (each `🌲` is U+1F332, 4 bytes UTF-8). Capping
        // at 128 must split on a `char` boundary, never mid-codepoint.
        let s: String = "🌲".repeat(200);
        let out = truncate_to_chars(s, 128);
        assert_eq!(out.chars().count(), 128);
        // Output must still be valid UTF-8 (it is by construction since
        // `String` enforces it; but assert byte-len matches expectation).
        assert_eq!(out.len(), 128 * 4);
    }

    /// A `ProfileAnnounce` with an oversized `display_name` must end up in
    /// `ProfileState.names` with at most `MAX_DISPLAY_NAME_LEN` chars.
    /// Without the cap, an attacker could grow the map's per-entry footprint
    /// without bound (#234, [SEC-V-05]).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn profile_announce_display_name_is_truncated_on_receipt() {
        let (client, _rx) = test_client();

        let hub = willow_network::mem::MemHub::new();
        let net = willow_network::mem::MemNetwork::new(&hub);
        let (topic_handle, _events) = net
            .subscribe(willow_network::topic_id("unit-test-profile-cap"), vec![])
            .await
            .expect("subscribe must succeed");

        let ctx = ListenerCtx {
            event_state: client.event_state_addr.clone(),
            chat_meta: client.chat_meta_addr.clone(),
            profiles: client.profile_state_addr.clone(),
            network: client.network_meta_addr.clone(),
            voice: client.voice_state_addr.clone(),
            persistence: client.persistence_addr.clone(),
            persistence_enabled: false,
            event_broker: client.event_broker.clone(),
            identity: client.identity.clone(),
            join_links: Arc::clone(&client.join_links),
            pending_joins: Arc::clone(&client.pending_joins),
            dag: client.dag_addr.clone(),
            server_registry: client.server_registry_addr.clone(),
            on_neighbor_up: None,
        };

        let attacker = willow_identity::Identity::generate();
        let attacker_id = attacker.endpoint_id();

        // 4096 chars — well past the 128-char cap.
        let oversized: String = "A".repeat(4096);
        let msg = crate::ops::WireMessage::ProfileAnnounce {
            display_name: oversized,
        };
        let bytes = crate::ops::pack_wire(&msg, &attacker).expect("pack_wire must succeed");

        process_received_message(&bytes, attacker_id, &ctx, &topic_handle).await;

        // Poll for the entry to land. `mutate` awaits the actor reply, so
        // the state should already be visible — but poll briefly to absorb
        // any scheduling jitter on slow CI hosts.
        let stored_len = poll_until_some(
            || {
                willow_actor::state::select(&client.profile_state_addr, move |p| {
                    p.names.get(&attacker_id).map(|n| n.chars().count())
                })
            },
            std::time::Duration::from_secs(2),
        )
        .await;

        assert_eq!(
            stored_len,
            Some(MAX_DISPLAY_NAME_LEN),
            "oversized display_name must be capped at MAX_DISPLAY_NAME_LEN on receipt"
        );
    }

    /// A `TypingIndicator` with an oversized `channel` must end up in
    /// `NetworkMeta.typing_peers` with at most `MAX_TYPING_CHANNEL_LEN` chars.
    /// Same threat model as the profile case (#234, [SEC-V-05]).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn typing_indicator_channel_is_truncated_on_receipt() {
        let (client, _rx) = test_client();

        let hub = willow_network::mem::MemHub::new();
        let net = willow_network::mem::MemNetwork::new(&hub);
        let (topic_handle, _events) = net
            .subscribe(willow_network::topic_id("unit-test-typing-cap"), vec![])
            .await
            .expect("subscribe must succeed");

        let ctx = ListenerCtx {
            event_state: client.event_state_addr.clone(),
            chat_meta: client.chat_meta_addr.clone(),
            profiles: client.profile_state_addr.clone(),
            network: client.network_meta_addr.clone(),
            voice: client.voice_state_addr.clone(),
            persistence: client.persistence_addr.clone(),
            persistence_enabled: false,
            event_broker: client.event_broker.clone(),
            identity: client.identity.clone(),
            join_links: Arc::clone(&client.join_links),
            pending_joins: Arc::clone(&client.pending_joins),
            dag: client.dag_addr.clone(),
            server_registry: client.server_registry_addr.clone(),
            on_neighbor_up: None,
        };

        let attacker = willow_identity::Identity::generate();
        let attacker_id = attacker.endpoint_id();

        // 1024 ASCII chars — well past the 128-char cap, well under the
        // 4 KB `SIGNALING_CAP` enforced by `unpack_wire`.
        let oversized: String = "C".repeat(1024);
        let msg = crate::ops::WireMessage::TypingIndicator { channel: oversized };
        let bytes = crate::ops::pack_wire(&msg, &attacker).expect("pack_wire must succeed");

        process_received_message(&bytes, attacker_id, &ctx, &topic_handle).await;

        let stored_len = poll_until_some(
            || {
                willow_actor::state::select(&client.network_meta_addr, move |n| {
                    n.typing_peers
                        .get(&attacker_id)
                        .map(|(ch, _ts)| ch.chars().count())
                })
            },
            std::time::Duration::from_secs(2),
        )
        .await;

        assert_eq!(
            stored_len,
            Some(MAX_TYPING_CHANNEL_LEN),
            "oversized typing channel must be capped at MAX_TYPING_CHANNEL_LEN on receipt"
        );
    }

    // ─── [SEC-V-03] / #303 — voice.participants growth caps ────────────────

    /// Build a `ListenerCtx` from a `test_client()` ClientHandle.
    fn make_ctx<N: willow_network::Network>(client: &ClientHandle<N>) -> ListenerCtx {
        ListenerCtx {
            event_state: client.event_state_addr.clone(),
            chat_meta: client.chat_meta_addr.clone(),
            profiles: client.profile_state_addr.clone(),
            network: client.network_meta_addr.clone(),
            voice: client.voice_state_addr.clone(),
            persistence: client.persistence_addr.clone(),
            persistence_enabled: false,
            event_broker: client.event_broker.clone(),
            identity: client.identity.clone(),
            join_links: Arc::clone(&client.join_links),
            pending_joins: Arc::clone(&client.pending_joins),
            dag: client.dag_addr.clone(),
            server_registry: client.server_registry_addr.clone(),
            on_neighbor_up: None,
        }
    }

    /// Helper to read the current `VoiceState`.
    async fn voice_snapshot<N: willow_network::Network>(
        client: &ClientHandle<N>,
    ) -> state_actors::VoiceState {
        willow_actor::state::select(&client.voice_state_addr, |v| v.clone()).await
    }

    /// Resolve the test client's known channel id (the "general" channel
    /// seeded by `test_client()`).
    async fn known_channel_id<N: willow_network::Network>(client: &ClientHandle<N>) -> String {
        let snap = client.state_snapshot().await;
        snap.channels
            .keys()
            .next()
            .cloned()
            .expect("test_client must seed at least one channel")
    }

    /// VoiceJoin against an unknown `channel_id` must not mutate
    /// `VoiceState.participants` (defends against attackers flooding
    /// random channel ids until memory is exhausted).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn voice_join_with_unknown_channel_is_dropped() {
        let (client, _rx) = test_client();
        let hub = willow_network::mem::MemHub::new();
        let net = willow_network::mem::MemNetwork::new(&hub);
        let (topic_handle, _events) = net
            .subscribe(willow_network::topic_id("unit-test-voice-unknown"), vec![])
            .await
            .expect("subscribe must succeed");
        let ctx = make_ctx(&client);

        let attacker = willow_identity::Identity::generate();
        let attacker_id = attacker.endpoint_id();

        let msg = crate::ops::WireMessage::VoiceJoin {
            channel_id: "no-such-channel".to_string(),
            peer_id: attacker_id,
        };
        let bytes = crate::ops::pack_wire(&msg, &attacker).expect("pack_wire must succeed");
        process_received_message(&bytes, attacker_id, &ctx, &topic_handle).await;

        // Brief settle window for actor mailboxes.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let snap = voice_snapshot(&client).await;
        assert!(
            snap.participants.is_empty(),
            "VoiceJoin against an unknown channel_id must not create an entry; got {:?}",
            snap.participants
        );
    }

    /// Sanity check: VoiceJoin against a real channel records the
    /// participant. Without this we wouldn't know the cap test below
    /// is exercising the real path.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn voice_join_with_known_channel_records_participant() {
        let (client, _rx) = test_client();
        let hub = willow_network::mem::MemHub::new();
        let net = willow_network::mem::MemNetwork::new(&hub);
        let (topic_handle, _events) = net
            .subscribe(willow_network::topic_id("unit-test-voice-known"), vec![])
            .await
            .expect("subscribe must succeed");
        let ctx = make_ctx(&client);
        let channel_id = known_channel_id(&client).await;

        let attacker = willow_identity::Identity::generate();
        let attacker_id = attacker.endpoint_id();
        let msg = crate::ops::WireMessage::VoiceJoin {
            channel_id: channel_id.clone(),
            peer_id: attacker_id,
        };
        let bytes = crate::ops::pack_wire(&msg, &attacker).expect("pack_wire must succeed");
        process_received_message(&bytes, attacker_id, &ctx, &topic_handle).await;

        let stored = poll_until_some(
            || {
                let ch = channel_id.clone();
                willow_actor::state::select(&client.voice_state_addr, move |v| {
                    v.participants
                        .get(&ch)
                        .filter(|set| set.contains(&attacker_id))
                        .map(|set| set.len())
                })
            },
            std::time::Duration::from_secs(2),
        )
        .await;
        assert_eq!(
            stored,
            Some(1),
            "VoiceJoin on a known channel must record the participant"
        );
    }

    /// Once `MAX_VOICE_CHANNELS` distinct channel_ids have been recorded,
    /// further VoiceJoin packets that reference *new* channel_ids must be
    /// dropped — even if the channel exists in `ServerState`. This caps the
    /// outer dimension of `participants`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn voice_join_rejects_when_distinct_channels_at_cap() {
        let (client, _rx) = test_client();
        let hub = willow_network::mem::MemHub::new();
        let net = willow_network::mem::MemNetwork::new(&hub);
        let (topic_handle, _events) = net
            .subscribe(willow_network::topic_id("unit-test-voice-chcap"), vec![])
            .await
            .expect("subscribe must succeed");
        let ctx = make_ctx(&client);
        let channel_id = known_channel_id(&client).await;

        // Pre-fill `participants` with `MAX_VOICE_CHANNELS` distinct
        // synthetic channel_ids. We seed directly via the actor — the
        // listener would normally reject any of these as unknown, but
        // we want to test the *cap* in isolation. `state_snapshot()`'s
        // single channel is added below as the (cap+1)-th attempt.
        willow_actor::state::mutate(&client.voice_state_addr, move |v| {
            for i in 0..MAX_VOICE_CHANNELS {
                v.participants.entry(format!("filler-{i}")).or_default();
            }
        })
        .await;

        let attacker = willow_identity::Identity::generate();
        let attacker_id = attacker.endpoint_id();
        // Use the *real* channel_id so the existence check passes; the
        // cap is what must reject this message.
        let msg = crate::ops::WireMessage::VoiceJoin {
            channel_id: channel_id.clone(),
            peer_id: attacker_id,
        };
        let bytes = crate::ops::pack_wire(&msg, &attacker).expect("pack_wire must succeed");
        process_received_message(&bytes, attacker_id, &ctx, &topic_handle).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let snap = voice_snapshot(&client).await;
        assert_eq!(
            snap.participants.len(),
            MAX_VOICE_CHANNELS,
            "distinct-channels cap must hold against new VoiceJoin entries"
        );
        assert!(
            !snap.participants.contains_key(&channel_id),
            "VoiceJoin for a new channel_id must be dropped once the cap is reached"
        );
    }

    /// Once a single channel has `MAX_PARTICIPANTS_PER_CHANNEL` participants,
    /// further VoiceJoin packets for that channel must be dropped. This caps
    /// the inner dimension of `participants`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn voice_join_rejects_when_participants_at_cap() {
        let (client, _rx) = test_client();
        let hub = willow_network::mem::MemHub::new();
        let net = willow_network::mem::MemNetwork::new(&hub);
        let (topic_handle, _events) = net
            .subscribe(willow_network::topic_id("unit-test-voice-pcap"), vec![])
            .await
            .expect("subscribe must succeed");
        let ctx = make_ctx(&client);
        let channel_id = known_channel_id(&client).await;

        // Fill the cap with synthetic participants on the real channel.
        let cap_channel = channel_id.clone();
        willow_actor::state::mutate(&client.voice_state_addr, move |v| {
            let set = v.participants.entry(cap_channel).or_default();
            for _ in 0..MAX_PARTICIPANTS_PER_CHANNEL {
                set.insert(willow_identity::Identity::generate().endpoint_id());
            }
        })
        .await;

        // Confirm preconditions.
        let pre = voice_snapshot(&client).await;
        assert_eq!(
            pre.participants.get(&channel_id).map(|s| s.len()),
            Some(MAX_PARTICIPANTS_PER_CHANNEL),
            "test setup must fill participants to cap"
        );

        let attacker = willow_identity::Identity::generate();
        let attacker_id = attacker.endpoint_id();
        let msg = crate::ops::WireMessage::VoiceJoin {
            channel_id: channel_id.clone(),
            peer_id: attacker_id,
        };
        let bytes = crate::ops::pack_wire(&msg, &attacker).expect("pack_wire must succeed");
        process_received_message(&bytes, attacker_id, &ctx, &topic_handle).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let snap = voice_snapshot(&client).await;
        let set = snap
            .participants
            .get(&channel_id)
            .expect("channel set must still exist");
        assert_eq!(
            set.len(),
            MAX_PARTICIPANTS_PER_CHANNEL,
            "participant cap must hold; got {}",
            set.len()
        );
        assert!(
            !set.contains(&attacker_id),
            "new participant must be dropped once cap is reached"
        );
    }

    /// VoiceLeave against an unknown channel_id must not create an entry
    /// in `participants`. Without the gate, an attacker can still grow
    /// the map by sending Leaves first (`get_mut` returns None today, so
    /// this is effectively a no-op — but we still want to assert the gate
    /// so future refactors don't reintroduce the hole, and we add a
    /// `tracing::debug!` for observability symmetry with Join).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn voice_leave_with_unknown_channel_is_dropped() {
        let (client, _rx) = test_client();
        let hub = willow_network::mem::MemHub::new();
        let net = willow_network::mem::MemNetwork::new(&hub);
        let (topic_handle, _events) = net
            .subscribe(willow_network::topic_id("unit-test-voice-leave"), vec![])
            .await
            .expect("subscribe must succeed");
        let ctx = make_ctx(&client);

        let attacker = willow_identity::Identity::generate();
        let attacker_id = attacker.endpoint_id();
        let msg = crate::ops::WireMessage::VoiceLeave {
            channel_id: "no-such-channel".to_string(),
            peer_id: attacker_id,
        };
        let bytes = crate::ops::pack_wire(&msg, &attacker).expect("pack_wire must succeed");
        process_received_message(&bytes, attacker_id, &ctx, &topic_handle).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let snap = voice_snapshot(&client).await;
        assert!(
            snap.participants.is_empty(),
            "VoiceLeave against an unknown channel_id must not produce a state mutation"
        );
    }

    /// Poll a closure that returns `impl Future<Output = Option<T>>` until
    /// it yields `Some` or the deadline expires. Returns the last observed
    /// value (whether `Some` or `None`).
    async fn poll_until_some<T, Fut, F>(mut f: F, deadline: std::time::Duration) -> Option<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Option<T>>,
    {
        let start = std::time::Instant::now();
        loop {
            let v = f().await;
            if v.is_some() {
                return v;
            }
            if start.elapsed() >= deadline {
                return v;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    // ─── [SEC-A-07] / #309 — JoinResponse / JoinDenied signer guard ──
    //
    // Without the fix, the requester accepted any signed packet whose
    // `target_peer == self.endpoint_id()` — meaning any peer that could
    // observe the topic (or simply guess the requester's `EndpointId`)
    // could:
    //
    //   * Spoof a `JoinDenied` carrying an attacker-chosen `reason`,
    //     surfaced verbatim to the user via `ClientEvent::JoinLinkDenied`
    //     — a phishing surface.
    //   * Spoof a `JoinResponse` with a bogus `invite_data`, forcing the
    //     requester to attempt decryption / UI churn — a DoS surface
    //     even though the payload is encrypted to the requester's pubkey.
    //
    // The tests below drive `process_received_message` with synthetic
    // packets and assert that responses signed by anyone other than the
    // inviter the requester sent its `JoinRequest` to are dropped. They
    // also cover the `reason`-string sanitization that runs after the
    // signer check passes.

    /// Build a `ListenerCtx` from a `test_client()` plus a `MemNetwork`
    /// topic handle, mirroring the pattern used by the JoinRequest guard
    /// tests above.
    async fn make_join_guard_ctx() -> (
        ClientHandle<willow_network::mem::MemNetwork>,
        ListenerCtx,
        <willow_network::mem::MemNetwork as Network>::Topic,
    ) {
        let (client, _rx) = test_client();
        let hub = willow_network::mem::MemHub::new();
        let net = willow_network::mem::MemNetwork::new(&hub);
        let (topic, _events) = net
            .subscribe(willow_network::topic_id("sec-a-07-test"), vec![])
            .await
            .expect("subscribe must succeed");
        let ctx = ListenerCtx {
            event_state: client.event_state_addr.clone(),
            chat_meta: client.chat_meta_addr.clone(),
            profiles: client.profile_state_addr.clone(),
            network: client.network_meta_addr.clone(),
            voice: client.voice_state_addr.clone(),
            persistence: client.persistence_addr.clone(),
            persistence_enabled: false,
            event_broker: client.event_broker.clone(),
            identity: client.identity.clone(),
            join_links: Arc::clone(&client.join_links),
            pending_joins: Arc::clone(&client.pending_joins),
            dag: client.dag_addr.clone(),
            server_registry: client.server_registry_addr.clone(),
            on_neighbor_up: None,
        };
        (client, ctx, topic)
    }

    /// Drain pending `ClientEvent`s from `rx` after a short settle window.
    async fn drain_events(
        rx: &mut crate::EventReceiver,
        settle: std::time::Duration,
    ) -> Vec<ClientEvent> {
        tokio::time::sleep(settle).await;
        let mut out = Vec::new();
        while let Ok(ev) =
            tokio::time::timeout(std::time::Duration::from_millis(5), rx.recv()).await
        {
            match ev {
                Some(e) => out.push(e),
                None => break,
            }
        }
        out
    }

    // ─── reason-string sanitization (pure-function tests) ────────────

    #[test]
    fn sanitize_reason_strips_control_chars() {
        let dirty = "hello\nworld\t\u{7}!\u{1b}[31mred".to_string();
        let clean = sanitize_reason(dirty);
        // Control chars stripped; printable text preserved verbatim.
        assert_eq!(clean, "helloworld![31mred");
        assert!(
            !clean.chars().any(|c| c.is_control()),
            "no control chars must survive"
        );
    }

    #[test]
    fn sanitize_reason_caps_length() {
        let huge: String = "a".repeat(MAX_JOIN_DENIED_REASON_LEN * 4);
        let clean = sanitize_reason(huge);
        assert_eq!(clean.chars().count(), MAX_JOIN_DENIED_REASON_LEN);
    }

    #[test]
    fn sanitize_reason_caps_after_stripping_controls() {
        // Control chars should be removed *first*, so the cap counts only
        // the displayable code points the user would actually see.
        let mixed: String = "\n".repeat(1000) + &"x".repeat(MAX_JOIN_DENIED_REASON_LEN);
        let clean = sanitize_reason(mixed);
        assert_eq!(clean.chars().count(), MAX_JOIN_DENIED_REASON_LEN);
        assert!(clean.chars().all(|c| c == 'x'));
    }

    #[test]
    fn sanitize_reason_passes_short_clean_input_unchanged() {
        let s = "link expired".to_string();
        assert_eq!(sanitize_reason(s.clone()), s);
    }

    // ─── signer-binding guard — the SEC-A-07 fix ─────────────────────

    /// A `JoinDenied` whose `signer` is *not* the inviter the requester
    /// sent its `JoinRequest` to must be dropped silently — no
    /// `JoinLinkDenied` event surfaces the attacker-controlled `reason`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_denied_from_non_inviter_is_dropped() {
        let (client, ctx, topic) = make_join_guard_ctx().await;
        let mut rx = client.subscribe_events().await;

        // Set up: requester is mid-join, expecting a reply from `alice`.
        let alice = willow_identity::Identity::generate();
        let alice_id = alice.endpoint_id();
        let link_id = "test-link-spoof-denied".to_string();
        client
            .pending_joins
            .lock()
            .insert(link_id.clone(), alice_id);

        // Mallory (not Alice) signs a JoinDenied with a phishing reason.
        let mallory = willow_identity::Identity::generate();
        let phishing = "Your invite expired. Re-enter your seed phrase             at evil.example to recover access.";
        let msg = crate::ops::WireMessage::JoinDenied {
            link_id: link_id.clone(),
            target_peer: client.identity.endpoint_id(),
            reason: phishing.to_string(),
        };
        let bytes = crate::ops::pack_wire(&msg, &mallory).expect("pack must succeed");

        process_received_message(&bytes, mallory.endpoint_id(), &ctx, &topic).await;

        let events = drain_events(&mut rx, std::time::Duration::from_millis(50)).await;
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ClientEvent::JoinLinkDenied { .. })),
            "spoofed JoinDenied must not surface a JoinLinkDenied event, got: {events:?}"
        );

        // The pending entry must still be there — a later legitimate
        // denial from Alice should still resolve the join attempt.
        assert_eq!(
            client.pending_joins.lock().get(&link_id).copied(),
            Some(alice_id),
            "rejected JoinDenied must not consume the pending-join entry"
        );
    }

    /// A `JoinResponse` whose `signer` is *not* the expected inviter must
    /// be dropped silently — no `JoinLinkResponse` event surfaces, so the
    /// requester never wastes work attempting to decrypt the bogus
    /// payload.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_response_from_non_inviter_is_dropped() {
        let (client, ctx, topic) = make_join_guard_ctx().await;
        let mut rx = client.subscribe_events().await;

        let alice = willow_identity::Identity::generate();
        let alice_id = alice.endpoint_id();
        let link_id = "test-link-spoof-resp".to_string();
        client
            .pending_joins
            .lock()
            .insert(link_id.clone(), alice_id);

        let mallory = willow_identity::Identity::generate();
        let msg = crate::ops::WireMessage::JoinResponse {
            link_id: link_id.clone(),
            target_peer: client.identity.endpoint_id(),
            invite_data: "garbage that would fail to decrypt anyway".to_string(),
        };
        let bytes = crate::ops::pack_wire(&msg, &mallory).expect("pack must succeed");

        process_received_message(&bytes, mallory.endpoint_id(), &ctx, &topic).await;

        let events = drain_events(&mut rx, std::time::Duration::from_millis(50)).await;
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ClientEvent::JoinLinkResponse { .. })),
            "spoofed JoinResponse must not surface a JoinLinkResponse event, got: {events:?}"
        );
        assert_eq!(
            client.pending_joins.lock().get(&link_id).copied(),
            Some(alice_id),
        );
    }

    /// Regression: a `JoinDenied` signed by the actual inviter must
    /// surface a `JoinLinkDenied` event with a sanitized `reason`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_denied_from_real_inviter_surfaces_sanitized_event() {
        let (client, ctx, topic) = make_join_guard_ctx().await;
        let mut rx = client.subscribe_events().await;

        let alice = willow_identity::Identity::generate();
        let alice_id = alice.endpoint_id();
        let link_id = "test-link-real-denied".to_string();
        client
            .pending_joins
            .lock()
            .insert(link_id.clone(), alice_id);

        // Alice's reason carries control chars + ANSI noise we want
        // stripped before display.
        let raw_reason = "link\u{7} expired\n";
        let msg = crate::ops::WireMessage::JoinDenied {
            link_id: link_id.clone(),
            target_peer: client.identity.endpoint_id(),
            reason: raw_reason.to_string(),
        };
        let bytes = crate::ops::pack_wire(&msg, &alice).expect("pack must succeed");

        process_received_message(&bytes, alice_id, &ctx, &topic).await;

        let events = drain_events(&mut rx, std::time::Duration::from_millis(50)).await;
        let denied: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ClientEvent::JoinLinkDenied { reason } => Some(reason.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(denied.len(), 1, "expected exactly one JoinLinkDenied event");
        assert_eq!(denied[0], "link expired");
        assert!(
            client.pending_joins.lock().get(&link_id).is_none(),
            "legitimate JoinDenied must consume the pending-join entry"
        );
    }

    /// Regression: a `JoinResponse` signed by the actual inviter must
    /// surface a `JoinLinkResponse` event with the original
    /// `invite_data`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_response_from_real_inviter_surfaces_event() {
        let (client, ctx, topic) = make_join_guard_ctx().await;
        let mut rx = client.subscribe_events().await;

        let alice = willow_identity::Identity::generate();
        let alice_id = alice.endpoint_id();
        let link_id = "test-link-real-resp".to_string();
        client
            .pending_joins
            .lock()
            .insert(link_id.clone(), alice_id);

        let invite = "real-invite-payload-base64";
        let msg = crate::ops::WireMessage::JoinResponse {
            link_id: link_id.clone(),
            target_peer: client.identity.endpoint_id(),
            invite_data: invite.to_string(),
        };
        let bytes = crate::ops::pack_wire(&msg, &alice).expect("pack must succeed");

        process_received_message(&bytes, alice_id, &ctx, &topic).await;

        let events = drain_events(&mut rx, std::time::Duration::from_millis(50)).await;
        let responses: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ClientEvent::JoinLinkResponse { invite_data } => Some(invite_data.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0], invite);
        assert!(
            client.pending_joins.lock().get(&link_id).is_none(),
            "legitimate JoinResponse must consume the pending-join entry"
        );
    }

    /// A `JoinResponse` with no matching `pending_joins` entry must be
    /// dropped — for example, if it arrives long after the requester
    /// abandoned the join attempt and cleared local state.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_response_without_pending_entry_is_dropped() {
        let (client, ctx, topic) = make_join_guard_ctx().await;
        let mut rx = client.subscribe_events().await;
        // Note: no `pending_joins` insert.

        let alice = willow_identity::Identity::generate();
        let msg = crate::ops::WireMessage::JoinResponse {
            link_id: "unknown-link".to_string(),
            target_peer: client.identity.endpoint_id(),
            invite_data: "anything".to_string(),
        };
        let bytes = crate::ops::pack_wire(&msg, &alice).expect("pack must succeed");

        process_received_message(&bytes, alice.endpoint_id(), &ctx, &topic).await;

        let events = drain_events(&mut rx, std::time::Duration::from_millis(50)).await;
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ClientEvent::JoinLinkResponse { .. })),
            "JoinResponse without matching pending entry must be ignored"
        );
    }

    // ─── JoinDenied emission on the inviter side ───────────────────────
    //
    // The spec promises that when a joiner sends a `JoinRequest` for a
    // link that exists but is disabled/expired/exhausted, the inviter
    // replies with `JoinDenied { reason }` so the joiner sees a concrete
    // error instead of a 30-second timeout. Anti-enumeration: unknown
    // `link_id`s receive no reply (silent drop).
    //
    // The tests below subscribe a second peer (`observer`) to the same
    // hub topic as the inviter, drive the inviter's listener with a
    // signed `JoinRequest`, then assert on what the observer received
    // from the inviter's outbound broadcast. `MemTopicEvents::next`
    // skips self-messages, so the inviter cannot observe its own
    // broadcasts; that's why we need the second peer.

    /// Build an inviter `ListenerCtx` driven by a shared hub topic, plus
    /// an `observer` topic on the same hub used to read the inviter's
    /// outbound broadcasts.
    async fn make_inviter_and_observer() -> (
        ClientHandle<willow_network::mem::MemNetwork>,
        ListenerCtx,
        <willow_network::mem::MemNetwork as Network>::Topic,
        <willow_network::mem::MemNetwork as Network>::Events,
    ) {
        let (client, _rx) = test_client();
        let hub = willow_network::mem::MemHub::new();
        let inviter_net = willow_network::mem::MemNetwork::new(&hub);
        let (topic, _inviter_events) = inviter_net
            .subscribe(willow_network::topic_id("join-denied-emit-test"), vec![])
            .await
            .expect("inviter subscribe must succeed");
        let observer_net = willow_network::mem::MemNetwork::new(&hub);
        let (_observer_topic, observer_events) = observer_net
            .subscribe(willow_network::topic_id("join-denied-emit-test"), vec![])
            .await
            .expect("observer subscribe must succeed");
        let ctx = ListenerCtx {
            event_state: client.event_state_addr.clone(),
            chat_meta: client.chat_meta_addr.clone(),
            profiles: client.profile_state_addr.clone(),
            network: client.network_meta_addr.clone(),
            voice: client.voice_state_addr.clone(),
            persistence: client.persistence_addr.clone(),
            persistence_enabled: false,
            event_broker: client.event_broker.clone(),
            identity: client.identity.clone(),
            join_links: Arc::clone(&client.join_links),
            pending_joins: Arc::clone(&client.pending_joins),
            dag: client.dag_addr.clone(),
            server_registry: client.server_registry_addr.clone(),
            on_neighbor_up: None,
        };
        (client, ctx, topic, observer_events)
    }

    /// Drain the next `JoinDenied` reason from the observer's topic
    /// event stream, with a short settle window. Returns `None` if
    /// nothing arrives in the deadline.
    async fn next_join_denied_reason(
        events: &mut <willow_network::mem::MemNetwork as Network>::Events,
        deadline: std::time::Duration,
    ) -> Option<String> {
        use willow_network::{GossipEvent, TopicEvents};
        let task = async {
            loop {
                let Some(Ok(evt)) = events.next().await else {
                    return None;
                };
                let GossipEvent::Received(msg) = evt else {
                    continue;
                };
                let Some((wire, _signer)) = crate::ops::unpack_wire(&msg.content) else {
                    continue;
                };
                if let crate::ops::WireMessage::JoinDenied { reason, .. } = wire {
                    return Some(reason);
                }
            }
        };
        tokio::time::timeout(deadline, task).await.ok().flatten()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_request_for_disabled_link_replies_with_link_disabled() {
        let (client, ctx, topic, mut observer) = make_inviter_and_observer().await;
        let link_id = "disabled-link".to_string();
        client.join_links.lock().push(crate::ops::JoinLink {
            link_id: link_id.clone(),
            server_id: "unused".into(),
            max_uses: 10,
            used: 0,
            active: false, // disabled by inviter
            expires_at: None,
            created_at: 0,
        });

        let bob = willow_identity::Identity::generate();
        let bob_id = bob.endpoint_id();
        let bytes = crate::ops::pack_wire(
            &crate::ops::WireMessage::JoinRequest {
                link_id: link_id.clone(),
                peer_id: bob_id,
            },
            &bob,
        )
        .expect("pack must succeed");
        process_received_message(&bytes, bob_id, &ctx, &topic).await;

        let reason =
            next_join_denied_reason(&mut observer, std::time::Duration::from_millis(500)).await;
        assert_eq!(reason, Some("link_disabled".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_request_for_expired_link_replies_with_link_expired() {
        let (client, ctx, topic, mut observer) = make_inviter_and_observer().await;
        let link_id = "expired-link".to_string();
        client.join_links.lock().push(crate::ops::JoinLink {
            link_id: link_id.clone(),
            server_id: "unused".into(),
            max_uses: 10,
            used: 0,
            active: true,
            // Expired one second before "now" — `current_time_ms()` is
            // monotonic-ish; this guarantees `is_valid()` returns false.
            expires_at: Some(crate::util::current_time_ms().saturating_sub(1_000)),
            created_at: 0,
        });

        let bob = willow_identity::Identity::generate();
        let bob_id = bob.endpoint_id();
        let bytes = crate::ops::pack_wire(
            &crate::ops::WireMessage::JoinRequest {
                link_id: link_id.clone(),
                peer_id: bob_id,
            },
            &bob,
        )
        .expect("pack must succeed");
        process_received_message(&bytes, bob_id, &ctx, &topic).await;

        let reason =
            next_join_denied_reason(&mut observer, std::time::Duration::from_millis(500)).await;
        assert_eq!(reason, Some("link_expired".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_request_for_exhausted_link_replies_with_link_expired() {
        let (client, ctx, topic, mut observer) = make_inviter_and_observer().await;
        let link_id = "exhausted-link".to_string();
        client.join_links.lock().push(crate::ops::JoinLink {
            link_id: link_id.clone(),
            server_id: "unused".into(),
            max_uses: 1,
            used: 1, // already used up
            active: true,
            expires_at: None,
            created_at: 0,
        });

        let bob = willow_identity::Identity::generate();
        let bob_id = bob.endpoint_id();
        let bytes = crate::ops::pack_wire(
            &crate::ops::WireMessage::JoinRequest {
                link_id: link_id.clone(),
                peer_id: bob_id,
            },
            &bob,
        )
        .expect("pack must succeed");
        process_received_message(&bytes, bob_id, &ctx, &topic).await;

        // Spec collapses "max_uses reached" and "expires_at in the past"
        // under the same `link_expired` reason — both mean "this link
        // can no longer be used."
        let reason =
            next_join_denied_reason(&mut observer, std::time::Duration::from_millis(500)).await;
        assert_eq!(reason, Some("link_expired".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_request_for_unknown_link_id_is_silent() {
        let (_client, ctx, topic, mut observer) = make_inviter_and_observer().await;

        let bob = willow_identity::Identity::generate();
        let bob_id = bob.endpoint_id();
        let bytes = crate::ops::pack_wire(
            &crate::ops::WireMessage::JoinRequest {
                link_id: "no-such-link".to_string(),
                peer_id: bob_id,
            },
            &bob,
        )
        .expect("pack must succeed");
        process_received_message(&bytes, bob_id, &ctx, &topic).await;

        // Anti-enumeration: unknown `link_id` MUST NOT produce a reply.
        // The inviter must look indistinguishable from "wrong inviter."
        let reason =
            next_join_denied_reason(&mut observer, std::time::Duration::from_millis(250)).await;
        assert_eq!(
            reason, None,
            "unknown link_id must not produce a JoinDenied (anti-enumeration)"
        );
    }
}
