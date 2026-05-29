//! History-sync EOSE marker tests (history-sync-eose spec, plan PR 5 Task 5.2).
//!
//! These cover the client-side handling of [`WireMessage::HistorySyncComplete`]
//! wired into [`crate::listeners::process_received_message`]: a trusted
//! `SyncProvider` peer signals end-of-stored-events for a topic, and the client
//! emits exactly one [`ClientEvent::HistorySynced`] per
//! `(topic, provider, stream_generation)` — gated on the signer holding an
//! explicit `SyncProvider` grant, deduped on `(provider, stream_generation)`,
//! and suppressed entirely when the marker's `last_event_hash` is not present
//! locally (silent-truncation guard).
//!
//! We drive `process_received_message` directly (the same deterministic pattern
//! the `heads_sync` tests use) and collect emitted events off an
//! [`EventReceiver`] subscribed to the client's broker — no gossip-timing
//! flakiness.

use std::sync::Arc;
use std::time::Duration;

use willow_network::mem::{MemHub, MemNetwork};
use willow_network::Network;
use willow_state::{Event, EventHash, Permission};

use crate::listeners::{process_received_message, ListenerCtx};
use crate::{test_client, ClientEvent, ClientHandle, EventReceiver};

// ───── Helpers ──────────────────────────────────────────────────────────

/// Build a [`ListenerCtx`] from a `test_client()` handle, mirroring the
/// helper in the `heads_sync` test module.
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

/// A throwaway [`TopicHandle`] on a fresh [`MemHub`] so the handler's `topic`
/// parameter is satisfiable. The `HistorySyncComplete` arm never broadcasts, so
/// nothing is ever read from it.
async fn dummy_topic(label: &str) -> impl willow_network::traits::TopicHandle {
    let hub = MemHub::new();
    let net = MemNetwork::new(&hub);
    let (topic, _events) = net
        .subscribe(willow_network::topic_id(label), vec![])
        .await
        .expect("subscribe must succeed in test");
    topic
}

/// Collect every [`ClientEvent::HistorySynced`] that lands on `rx` within
/// `deadline`, draining the broker until it goes quiet.
async fn collect_history_synced(
    rx: &mut EventReceiver,
    deadline: Duration,
) -> Vec<(String, willow_identity::EndpointId, usize)> {
    let mut out = Vec::new();
    let _ = tokio::time::timeout(deadline, async {
        loop {
            match rx.try_recv() {
                Some(ClientEvent::HistorySynced {
                    topic,
                    provider,
                    still_pending,
                }) => out.push((topic, provider, still_pending)),
                Some(_) => {}
                None => tokio::task::yield_now().await,
            }
        }
    })
    .await;
    out
}

/// Lowercase-hex of a 32-byte topic id — mirrors the encoding the handler uses
/// for the `HistorySynced.topic` field so tests can assert the exact value.
fn topic_hex(topic_id: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in topic_id {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Deliver a `HistorySyncComplete` marker signed by `provider` into `ctx`.
async fn deliver_marker(
    ctx: &ListenerCtx,
    topic: &impl willow_network::traits::TopicHandle,
    provider: &willow_identity::Identity,
    topic_id: [u8; 32],
    last_event_hash: Option<EventHash>,
    stream_generation: u64,
) {
    let msg = crate::ops::WireMessage::HistorySyncComplete {
        topic_id,
        last_event_hash,
        stream_generation,
    };
    let bytes = crate::ops::pack_wire(&msg, provider).expect("pack marker");
    process_received_message(&bytes, provider.endpoint_id(), ctx, topic).await;
}

/// Grant `peer` an explicit `SyncProvider` permission in `client`'s DAG, so its
/// markers are trusted by the EOSE handler.
async fn grant_sync_provider<N: willow_network::Network>(
    client: &ClientHandle<N>,
    peer: willow_identity::EndpointId,
) {
    client
        .mutations()
        .grant_permission(peer, Permission::SyncProvider)
        .await
        .expect("grant SyncProvider");
}

const TOPIC_A: [u8; 32] = [0xAA; 32];

// ───── 1. one HistorySynced per (topic, provider) from a trusted marker ────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trusted_marker_emits_exactly_one_history_synced() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (client, broker) = test_client();
            let mut rx = EventReceiver::subscribe(&broker, &client.system).await;

            // A separate provider identity, granted SyncProvider by the owner.
            let provider = willow_identity::Identity::generate();
            grant_sync_provider(&client, provider.endpoint_id()).await;
            // Drain the ProfileUpdated / grant-derived events so the next
            // collection sees only HistorySynced.
            let _ = collect_history_synced(&mut rx, Duration::from_millis(50)).await;

            let ctx = make_ctx(&client);
            let topic = dummy_topic("eose-1").await;

            // Empty-store marker (last_event_hash = None never truncates).
            deliver_marker(&ctx, &topic, &provider, TOPIC_A, None, 7).await;

            let events = collect_history_synced(&mut rx, Duration::from_millis(300)).await;
            assert_eq!(
                events.len(),
                1,
                "exactly one HistorySynced for a trusted marker"
            );
            assert_eq!(events[0].0, topic_hex(&TOPIC_A), "topic is hex of topic_id");
            assert_eq!(
                events[0].1,
                provider.endpoint_id(),
                "provider is the signer"
            );
            assert_eq!(
                events[0].2, 0,
                "no other connected trusted providers → still_pending 0"
            );
        })
        .await;
}

// ───── 2. untrusted marker emits nothing ──────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn untrusted_marker_emits_no_event() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (client, broker) = test_client();
            let mut rx = EventReceiver::subscribe(&broker, &client.system).await;

            // A provider that was NEVER granted SyncProvider.
            let imposter = willow_identity::Identity::generate();

            let ctx = make_ctx(&client);
            let topic = dummy_topic("eose-2").await;

            deliver_marker(&ctx, &topic, &imposter, TOPIC_A, None, 1).await;

            let events = collect_history_synced(&mut rx, Duration::from_millis(300)).await;
            assert!(
                events.is_empty(),
                "an untrusted peer's marker must not flip the loading flag (got {events:?})"
            );
        })
        .await;
}

// ───── 3. stream_generation dedup: repeat ignored, new generation re-emits ─

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_generation_dedups_new_generation_reemits() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (client, broker) = test_client();
            let mut rx = EventReceiver::subscribe(&broker, &client.system).await;

            let provider = willow_identity::Identity::generate();
            grant_sync_provider(&client, provider.endpoint_id()).await;
            let _ = collect_history_synced(&mut rx, Duration::from_millis(50)).await;

            let ctx = make_ctx(&client);
            let topic = dummy_topic("eose-3").await;

            // First marker, generation 42 → emit.
            deliver_marker(&ctx, &topic, &provider, TOPIC_A, None, 42).await;
            // Repeat of the SAME generation → no emit.
            deliver_marker(&ctx, &topic, &provider, TOPIC_A, None, 42).await;

            let first = collect_history_synced(&mut rx, Duration::from_millis(300)).await;
            assert_eq!(
                first.len(),
                1,
                "same (provider, stream_generation) emits exactly once"
            );

            // Provider restarts, re-streams under a fresh random generation.
            deliver_marker(&ctx, &topic, &provider, TOPIC_A, None, 99).await;
            let second = collect_history_synced(&mut rx, Duration::from_millis(300)).await;
            assert_eq!(
                second.len(),
                1,
                "a new stream_generation re-emits HistorySynced"
            );
        })
        .await;
}

// ───── 4. last_event_hash mismatch suppresses a false completion ───────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn truncation_mismatch_suppresses_completion() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (client, broker) = test_client();
            let mut rx = EventReceiver::subscribe(&broker, &client.system).await;

            let provider = willow_identity::Identity::generate();
            grant_sync_provider(&client, provider.endpoint_id()).await;
            let _ = collect_history_synced(&mut rx, Duration::from_millis(50)).await;

            let ctx = make_ctx(&client);
            let topic = dummy_topic("eose-4").await;

            // The provider claims a last event the client never received: a hash
            // that is not in the local DAG. Streaming did not reach us intact.
            // Construct a raw (non-content-derived) hash so it can never collide
            // with a real event.
            let phantom = EventHash([0x11; 32]);
            assert!(
                !dag_contains(&client, phantom).await,
                "test precondition: phantom hash must NOT be in the DAG"
            );

            deliver_marker(&ctx, &topic, &provider, TOPIC_A, Some(phantom), 5).await;

            let events = collect_history_synced(&mut rx, Duration::from_millis(300)).await;
            assert!(
                events.is_empty(),
                "a last_event_hash we do not hold must NOT produce a completion (got {events:?})"
            );

            // Positive control: a marker whose last_event_hash IS in the DAG
            // (the local genesis) completes normally on a fresh generation.
            let genesis = dag_first_hash(&client).await;
            deliver_marker(&ctx, &topic, &provider, TOPIC_A, Some(genesis), 6).await;
            let ok = collect_history_synced(&mut rx, Duration::from_millis(300)).await;
            assert_eq!(
                ok.len(),
                1,
                "a last_event_hash present locally completes normally"
            );
        })
        .await;
}

// ───── 5. still_pending counts other connected trusted providers ──────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn still_pending_counts_other_connected_trusted_providers() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (client, broker) = test_client();
            let mut rx = EventReceiver::subscribe(&broker, &client.system).await;

            // Two providers, both granted SyncProvider and both "connected"
            // (present in ChatMeta.peers).
            let p1 = willow_identity::Identity::generate();
            let p2 = willow_identity::Identity::generate();
            grant_sync_provider(&client, p1.endpoint_id()).await;
            grant_sync_provider(&client, p2.endpoint_id()).await;
            let p1_id = p1.endpoint_id();
            let p2_id = p2.endpoint_id();
            willow_actor::state::mutate(&client.chat_meta_addr, move |c| {
                c.peers.push(p1_id);
                c.peers.push(p2_id);
            })
            .await;
            let _ = collect_history_synced(&mut rx, Duration::from_millis(50)).await;

            let ctx = make_ctx(&client);
            let topic = dummy_topic("eose-5").await;

            // p1 completes first: p2 is still pending → still_pending == 1.
            deliver_marker(&ctx, &topic, &p1, TOPIC_A, None, 1).await;
            let first = collect_history_synced(&mut rx, Duration::from_millis(300)).await;
            assert_eq!(first.len(), 1, "p1's marker emits once");
            assert_eq!(
                first[0].2, 1,
                "one other connected trusted provider (p2) still streaming"
            );

            // p2 completes: no providers left pending → still_pending == 0.
            deliver_marker(&ctx, &topic, &p2, TOPIC_A, None, 2).await;
            let second = collect_history_synced(&mut rx, Duration::from_millis(300)).await;
            assert_eq!(second.len(), 1, "p2's marker emits once");
            assert_eq!(second[0].2, 0, "no trusted providers still streaming");
        })
        .await;
}

// ───── DAG inspection helpers ─────────────────────────────────────────────

async fn dag_contains<N: willow_network::Network>(
    client: &ClientHandle<N>,
    hash: EventHash,
) -> bool {
    willow_actor::state::select(&client.dag_addr, move |ds| {
        ds.managed.dag().get(&hash).is_some()
    })
    .await
}

/// The hash of the first event in the client's DAG topological order (genesis).
async fn dag_first_hash<N: willow_network::Network>(client: &ClientHandle<N>) -> EventHash {
    willow_actor::state::select(&client.dag_addr, |ds| {
        let sorted = ds.managed.dag().topological_sort();
        sorted.first().map(|e| e.hash).expect("DAG has a genesis")
    })
    .await
}

// keep otherwise-unused imports honest
const _: fn() = || {
    let _: Option<Event> = None;
};
