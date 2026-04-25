//! # Phase 2b client-level queue tests
//!
//! Exercise [`ClientHandle::queue_view`], [`ClientHandle::retry_queue`],
//! and [`ClientHandle::mark_queue_read`] against the in-memory
//! `test_client` harness. No networking — the tests poke
//! [`QueueMeta`](crate::state_actors::QueueMeta) directly via the
//! test-only `_enqueue_outbound` helper and observe the derived
//! `QueueView`.

use std::sync::Arc;

use willow_identity::Identity;
use willow_messaging::MessageId;

use crate::event_receiver::EventReceiver;
use crate::events::ClientEvent;
use crate::queue::ArrivedSummary;
use crate::state_actors::QUEUE_ARRIVALS_CAP;
use crate::test_client;
use crate::ClientHandle;

async fn subscribe_rx<N: willow_network::Network>(
    client: &ClientHandle<N>,
    broker: &willow_actor::Addr<willow_actor::Broker<ClientEvent>>,
) -> EventReceiver {
    EventReceiver::subscribe(broker, &client.system).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_view_depth_aggregates_across_peers() {
    let (client, _broker) = test_client();

    let alice = Identity::generate().endpoint_id();
    let bob = Identity::generate().endpoint_id();
    let m1 = MessageId::new();
    let m2 = MessageId::new();
    let m3 = MessageId::new();

    client._enqueue_outbound(m1, alice, 1_000).await;
    client._enqueue_outbound(m2, alice, 2_000).await;
    client._enqueue_outbound(m3, bob, 3_000).await;

    let view = client.queue_view().await;
    assert_eq!(view.depth, 3);
    assert_eq!(view.peer_count, 2);
    assert_eq!(view.per_peer.get(&alice).unwrap().outbound, 2);
    assert_eq!(view.per_peer.get(&bob).unwrap().outbound, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_queue_is_noop_when_empty() {
    let (client, _broker) = test_client();
    // No entries — still succeeds, still emits a QueueChanged event.
    client.retry_queue().await.unwrap();
    let view = client.queue_view().await;
    assert_eq!(view.depth, 0);
    assert_eq!(view.peer_count, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_queue_stamps_last_attempt_on_every_entry() {
    let (client, _broker) = test_client();
    let alice = Identity::generate().endpoint_id();
    let bob = Identity::generate().endpoint_id();
    client
        ._enqueue_outbound(MessageId::new(), alice, 1_000)
        .await;
    client._enqueue_outbound(MessageId::new(), bob, 2_000).await;

    client.retry_queue().await.unwrap();

    // Inspect the actor directly.
    let qm = willow_actor::state::get(client._queue_meta_addr()).await;
    for entry in qm.outbound.values() {
        assert!(
            entry.last_attempt_at.is_some(),
            "retry_queue must stamp last_attempt_at on every outbound entry"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mark_queue_read_writes_last_seen_marker() {
    let (client, _broker) = test_client();
    let alice = Identity::generate().endpoint_id();
    client.mark_queue_read(alice).await.unwrap();
    let qm = willow_actor::state::get(client._queue_meta_addr()).await;
    assert!(
        qm.marks.contains_key(&alice),
        "mark_queue_read must write a tick marker for the peer"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recent_arrivals_decay_after_24h() {
    let (client, _broker) = test_client();
    let alice = Identity::generate().endpoint_id();

    // Seed one arrival with a stale tick, bump `now` past 24 h
    // (86_400 s), then decay.
    willow_actor::state::mutate(client._queue_meta_addr(), move |qm| {
        qm.record_arrival(ArrivedSummary {
            peer_id: alice,
            at_tick: 0,
            count: 3,
            preview: None,
        });
        qm.now = 90_000; // past 24 h = 86_400 s
        qm.decay_arrivals(86_400);
    })
    .await;

    let view = client.queue_view().await;
    assert!(
        view.recent_arrivals.is_empty(),
        "arrivals older than 24 h must be pruned by decay_arrivals"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recent_arrivals_retained_within_24h() {
    let (client, _broker) = test_client();
    let alice = Identity::generate().endpoint_id();

    willow_actor::state::mutate(client._queue_meta_addr(), move |qm| {
        qm.record_arrival(ArrivedSummary {
            peer_id: alice,
            at_tick: 10_000,
            count: 2,
            preview: None,
        });
        qm.now = 10_100; // well under 24 h
        qm.decay_arrivals(86_400);
    })
    .await;

    let view = client.queue_view().await;
    assert_eq!(view.recent_arrivals.len(), 1);
    assert_eq!(view.recent_arrivals[0].count, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_online_mutation_emits_event_and_updates_offline_stamp() {
    let (client, broker) = test_client();

    // Subscribe to broker so we can drain events.
    let mut rx = subscribe_rx(&client, &broker).await;

    client.mutations().set_device_online(false).await;
    client.mutations().set_device_online(true).await;

    // Poll a few events off the receiver. The broker fans out via a
    // bounded channel so in the worst case we might not see the events
    // before the receiver is dropped; check both the actor state (the
    // offline_since_tick flips back to None) AND best-effort drain the
    // receiver for two `DeviceOnlineChanged` events.
    let qm = willow_actor::state::get(client._queue_meta_addr()).await;
    assert_eq!(qm.offline_since_tick, None);
    assert!(qm.device_online);

    // Drain up to 8 events from the broker — best-effort check.
    let mut seen_false = false;
    let mut seen_true = false;
    for _ in 0..8 {
        match tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await {
            Ok(Some(ClientEvent::DeviceOnlineChanged(false))) => seen_false = true,
            Ok(Some(ClientEvent::DeviceOnlineChanged(true))) => seen_true = true,
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    assert!(
        seen_false && seen_true,
        "device_online transitions must surface as two ClientEvent::DeviceOnlineChanged events"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_status_mutation_emits_event() {
    use crate::queue::RelayStatus;
    let (client, broker) = test_client();
    let mut rx = subscribe_rx(&client, &broker).await;

    client
        .mutations()
        .set_relay_status(RelayStatus::Unreachable)
        .await;

    let qm = willow_actor::state::get(client._queue_meta_addr()).await;
    assert_eq!(qm.relay_status, RelayStatus::Unreachable);

    // Best-effort drain.
    for _ in 0..8 {
        match tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await {
            Ok(Some(ClientEvent::RelayStatusChanged(RelayStatus::Unreachable))) => return,
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    panic!("expected ClientEvent::RelayStatusChanged(Unreachable)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn arrivals_capacity_bounded() {
    let (client, _broker) = test_client();
    willow_actor::state::mutate(client._queue_meta_addr(), |qm| {
        for _ in 0..QUEUE_ARRIVALS_CAP + 10 {
            qm.record_arrival(ArrivedSummary {
                peer_id: Identity::generate().endpoint_id(),
                at_tick: 0,
                count: 1,
                preview: None,
            });
        }
    })
    .await;
    let qm = willow_actor::state::get(client._queue_meta_addr()).await;
    assert_eq!(qm.recent_arrivals.len(), QUEUE_ARRIVALS_CAP);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_view_inbound_hint_propagates() {
    let (client, _broker) = test_client();
    let alice = Identity::generate().endpoint_id();
    willow_actor::state::mutate(client._queue_meta_addr(), move |qm| {
        qm.inbound_hint_per_peer.insert(alice, 7);
    })
    .await;
    let view = client.queue_view().await;
    assert_eq!(view.inbound_per_peer.get(&alice), Some(&7));
}

// ───── 60 s reconnection gate (spec §Reconnection toast) ──────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_transition_captures_last_offline_ticks_for_gate() {
    // Offline-then-online transition must record the duration of the
    // offline window in `last_offline_ticks` so the reconnection toast
    // + welcome-back banner can gate on "≥ 60 s offline" without the
    // UI having to observe the pre-clear `offline_since_tick`.
    let (client, _broker) = test_client();

    // Prime: start online at `now = 10`. Go offline, stay 65 s, back online.
    willow_actor::state::mutate(client._queue_meta_addr(), |qm| {
        qm.device_online = true;
        qm.now = 10;
        qm.set_device_online(false);
        qm.now = 75; // 65 s later
        qm.set_device_online(true);
    })
    .await;

    let view = client.queue_view().await;
    assert_eq!(view.last_offline_ticks, Some(65));
    assert!(view.device_online);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn short_offline_transition_still_records_last_offline_ticks() {
    // Short offline (< 60 s) also populates the field — the gate lives
    // in the UI components, not in the capture. The client records the
    // duration faithfully; the reconnection toast + welcome-back
    // banner decide whether to fire based on the value.
    let (client, _broker) = test_client();

    willow_actor::state::mutate(client._queue_meta_addr(), |qm| {
        qm.device_online = true;
        qm.now = 100;
        qm.set_device_online(false);
        qm.now = 130; // 30 s later — under the 60 s gate
        qm.set_device_online(true);
    })
    .await;

    let view = client.queue_view().await;
    assert_eq!(view.last_offline_ticks, Some(30));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn last_offline_ticks_is_none_on_first_connect() {
    // Fresh client that has never been offline — `last_offline_ticks`
    // stays `None` so the welcome-back banner does not fire on first
    // connect.
    let (client, _broker) = test_client();
    let view = client.queue_view().await;
    assert_eq!(view.last_offline_ticks, None);
}

// Sanity-check the `Arc::new((*snap).clone())` path used by the
// `retry_queue` + `mark_queue_read` methods — guards against future
// changes accidentally making the snapshot lose data.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_view_snapshot_survives_retry_queue_call() {
    let (client, _broker) = test_client();
    let alice = Identity::generate().endpoint_id();
    client
        ._enqueue_outbound(MessageId::new(), alice, 1_000)
        .await;
    client.retry_queue().await.unwrap();
    let view = client.queue_view().await;
    assert_eq!(view.depth, 1, "retry_queue must not drain entries");
    assert_eq!(view.peer_count, 1);
    assert!(
        view.per_peer.get(&alice).unwrap().last_attempt_at.is_some(),
        "retry_queue must stamp last_attempt_at on every entry"
    );
    let _ = Arc::new(view); // compile-time prove it's Arc-cloneable
}
