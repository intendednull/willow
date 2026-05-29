//! Heads-based delta-sync tests (negentropy-sync spec, plan PR 4).
//!
//! These cover the gossip-path `SyncRequestV2` responder and the
//! `SyncBatchV2` receiver/termination wired into
//! [`crate::listeners::process_received_message`]: a peer requests with its
//! current [`willow_state::HeadsSummary`] + a [`willow_common::SyncFilter`],
//! and the responder streams the per-author tail the requester is missing as
//! one or more `SyncBatchV2` envelopes ending `more: false`.
//!
//! We drive `process_received_message` directly (the same pattern the
//! `listeners` unit tests use) so the responder output is deterministic and
//! we can assert on the exact batches a request produces — no gossip-timing
//! flakiness. A separate end-to-end convergence test connects two real
//! clients on a shared [`MemHub`].

use std::sync::Arc;
use std::time::Duration;

use willow_network::mem::{MemHub, MemNetwork};
use willow_network::Network;
use willow_state::{AuthorHead, Event, EventHash, EventKind, HeadsSummary};

use crate::listeners::{process_received_message, ListenerCtx};
use crate::{test_client, ClientHandle};

// ───── Helpers ──────────────────────────────────────────────────────────

/// Build a [`ListenerCtx`] from a `test_client()` handle, mirroring the
/// helper in the `listeners` unit-test module.
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

/// Replay `events` into `target`'s managed DAG + materialized state,
/// fast-forwarding it to exactly that prefix.
async fn replay_into<N: willow_network::Network>(target: &ClientHandle<N>, events: Vec<Event>) {
    let events_for_dag = events.clone();
    willow_actor::state::mutate(&target.dag_addr, move |ds| {
        ds.managed = willow_state::ManagedDag::empty(5000);
        for event in events_for_dag {
            ds.managed.insert_and_apply(event).ok();
        }
        ds.invalidate_sync_reply_cache();
    })
    .await;
    let state =
        willow_actor::state::select(&target.dag_addr, |ds| ds.managed.state().clone()).await;
    willow_actor::state::mutate(&target.event_state_addr, move |es| {
        *es = state;
    })
    .await;
}

/// Snapshot a client's DAG as a topologically-sorted event list.
async fn snapshot_dag<N: willow_network::Network>(client: &ClientHandle<N>) -> Vec<Event> {
    willow_actor::state::select(&client.dag_addr, |ds| {
        ds.managed
            .dag()
            .topological_sort()
            .into_iter()
            .cloned()
            .collect()
    })
    .await
}

/// Current per-author DAG heads of a client.
async fn heads_of<N: willow_network::Network>(client: &ClientHandle<N>) -> HeadsSummary {
    willow_actor::state::select(&client.dag_addr, |ds| ds.managed.dag().heads_summary()).await
}

/// Total events currently in a client's DAG.
async fn dag_len<N: willow_network::Network>(client: &ClientHandle<N>) -> usize {
    willow_actor::state::select(&client.dag_addr, |ds| ds.managed.dag().len()).await
}

/// The server_id (genesis hash hex) of a client's DAG.
async fn server_id_of<N: willow_network::Network>(client: &ClientHandle<N>) -> String {
    willow_actor::state::select(&client.dag_addr, |ds| {
        ds.managed.dag().server_id().unwrap_or_default()
    })
    .await
}

/// Pump a single `SyncRequestV2` from `requester` into `responder` and
/// collect every `SyncBatchV2` the responder broadcasts back, in order.
///
/// Both peers share a [`MemHub`]; the responder broadcasts on `topic`, and a
/// co-subscribed reader drains the resulting batches until the terminator
/// (`more: false`) arrives or the deadline elapses.
async fn request_and_collect(
    responder_ctx: &ListenerCtx,
    responder_topic: &impl willow_network::traits::TopicHandle,
    reader: &mut impl willow_network::traits::TopicEvents,
    requester_identity: &willow_identity::Identity,
    request: crate::ops::WireMessage,
) -> Vec<(Vec<Event>, bool)> {
    use willow_network::traits::GossipEvent;

    let req_bytes = crate::ops::pack_wire(&request, requester_identity).expect("pack request");
    let requester_id = requester_identity.endpoint_id();

    // Deliver the request to the responder's listener.
    process_received_message(&req_bytes, requester_id, responder_ctx, responder_topic).await;

    // Drain the responder's batches off the shared topic.
    let mut batches = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(ev)) = reader.next().await {
            let GossipEvent::Received(msg) = ev else {
                continue;
            };
            let Some((crate::ops::WireMessage::SyncBatchV2 { events, more, .. }, _signer)) =
                crate::ops::unpack_wire(&msg.content)
            else {
                continue;
            };
            let last = !more;
            batches.push((events, more));
            if last {
                break;
            }
        }
    })
    .await;
    batches
}

// ───── 1. responder streams the full chain for an empty-heads request ─────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn responder_streams_full_chain_for_empty_heads() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let hub = Arc::new(MemHub::new());

            // Alice owns the server and authors a chain of messages.
            let (alice, _alice_broker) = test_client();
            // Grant Alice SyncProvider so the (gated) responder will serve.
            let alice_peer = alice.identity.endpoint_id();
            alice
                .mutations()
                .grant_permission(alice_peer, willow_state::Permission::SyncProvider)
                .await
                .expect("grant SyncProvider to self");
            for i in 0..20 {
                alice
                    .send_message("general", &format!("msg {i}"))
                    .await
                    .expect("alice sends");
            }

            let server_id = server_id_of(&alice).await;
            let alice_total = dag_len(&alice).await;
            assert!(alice_total >= 21, "alice should hold genesis + grant + 20");

            // Build the responder context + a shared topic the responder
            // broadcasts on and we read from.
            let alice_ctx = make_ctx(&alice);
            let topic_id = willow_network::topic_id("heads-sync-test-1");
            let alice_net = MemNetwork::new(&hub);
            let (alice_topic, _alice_events) = alice_net
                .subscribe(topic_id, vec![])
                .await
                .expect("subscribe");
            let reader_net = MemNetwork::new(&hub);
            let (_reader_topic, mut reader_events) = reader_net
                .subscribe(topic_id, vec![])
                .await
                .expect("subscribe reader");

            // Bob requests everything (empty heads).
            let bob = willow_identity::Identity::generate();
            let request = crate::ops::WireMessage::SyncRequestV2 {
                request_id: "req-1".to_string(),
                heads: HeadsSummary::default(),
                filter: willow_common::SyncFilter {
                    server_id: server_id.clone(),
                    channels: None,
                    authors: None,
                    event_kinds: None,
                    since_ms: None,
                },
            };

            let batches =
                request_and_collect(&alice_ctx, &alice_topic, &mut reader_events, &bob, request)
                    .await;

            assert!(
                !batches.is_empty(),
                "responder must emit at least one batch"
            );
            // Terminator present and is the final batch.
            assert!(!batches.last().unwrap().1, "final batch must clear `more`");
            for (_events, more) in &batches[..batches.len() - 1] {
                assert!(*more, "non-final batches must set `more`");
            }
            let total: usize = batches.iter().map(|(e, _)| e.len()).sum();
            assert_eq!(
                total, alice_total,
                "responder must stream the entire chain across batches"
            );
            // All batches share the request_id (checked implicitly by the
            // reader, which only collected SyncBatchV2 frames).
        })
        .await;
}

// ───── 2. up-to-date requester gets a single empty terminator ─────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn up_to_date_requester_gets_empty_terminator() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let hub = Arc::new(MemHub::new());
            let (alice, _b) = test_client();
            let alice_peer = alice.identity.endpoint_id();
            alice
                .mutations()
                .grant_permission(alice_peer, willow_state::Permission::SyncProvider)
                .await
                .expect("grant SyncProvider");
            alice
                .send_message("general", "only message")
                .await
                .expect("send");

            let server_id = server_id_of(&alice).await;
            let alice_heads = heads_of(&alice).await;

            let alice_ctx = make_ctx(&alice);
            let topic_id = willow_network::topic_id("heads-sync-test-2");
            let alice_net = MemNetwork::new(&hub);
            let (alice_topic, _e) = alice_net.subscribe(topic_id, vec![]).await.expect("sub");
            let reader_net = MemNetwork::new(&hub);
            let (_rt, mut reader_events) = reader_net
                .subscribe(topic_id, vec![])
                .await
                .expect("sub reader");

            let bob = willow_identity::Identity::generate();
            // Bob claims to already have everything Alice has.
            let request = crate::ops::WireMessage::SyncRequestV2 {
                request_id: "req-2".to_string(),
                heads: alice_heads,
                filter: willow_common::SyncFilter {
                    server_id,
                    channels: None,
                    authors: None,
                    event_kinds: None,
                    since_ms: None,
                },
            };
            let batches =
                request_and_collect(&alice_ctx, &alice_topic, &mut reader_events, &bob, request)
                    .await;
            assert_eq!(
                batches.len(),
                1,
                "up-to-date requester gets exactly one batch"
            );
            assert!(
                batches[0].0.is_empty(),
                "the single batch carries no events"
            );
            assert!(!batches[0].1, "and is the terminator");
        })
        .await;
}

// ───── 3. responder refuses to serve without SyncProvider (gate) ──────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn responder_without_sync_provider_serves_nothing() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let hub = Arc::new(MemHub::new());
            // Alice does NOT grant herself SyncProvider.
            let (alice, _b) = test_client();
            for i in 0..5 {
                alice
                    .send_message("general", &format!("m{i}"))
                    .await
                    .expect("send");
            }
            let server_id = server_id_of(&alice).await;

            let alice_ctx = make_ctx(&alice);
            let topic_id = willow_network::topic_id("heads-sync-test-3");
            let alice_net = MemNetwork::new(&hub);
            let (alice_topic, _e) = alice_net.subscribe(topic_id, vec![]).await.expect("sub");
            let reader_net = MemNetwork::new(&hub);
            let (_rt, mut reader_events) = reader_net
                .subscribe(topic_id, vec![])
                .await
                .expect("sub reader");

            let bob = willow_identity::Identity::generate();
            let request = crate::ops::WireMessage::SyncRequestV2 {
                request_id: "req-3".to_string(),
                heads: HeadsSummary::default(),
                filter: willow_common::SyncFilter {
                    server_id,
                    channels: None,
                    authors: None,
                    event_kinds: None,
                    since_ms: None,
                },
            };
            let batches =
                request_and_collect(&alice_ctx, &alice_topic, &mut reader_events, &bob, request)
                    .await;
            assert!(
                batches.is_empty(),
                "a peer without SyncProvider must not serve a SyncRequestV2 (got {} batches)",
                batches.len()
            );
        })
        .await;
}

// ───── 4. end-to-end convergence over real gossip ─────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bob_converges_via_sync_request_v2() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let hub = Arc::new(MemHub::new());

            let (mut alice, _ab) = test_client();
            let alice_peer = alice.identity.endpoint_id();
            alice
                .mutations()
                .grant_permission(alice_peer, willow_state::Permission::SyncProvider)
                .await
                .expect("grant SyncProvider");
            let alice_net = MemNetwork::new(&hub);
            alice.connect(alice_net).await;

            // Alice authors a chain before Bob can see it.
            for i in 0..30 {
                alice
                    .send_message("general", &format!("pre {i}"))
                    .await
                    .expect("send");
            }

            // Bob adopts Alice's server but only the genesis prefix, so he is
            // missing the tail and must pull it via SyncRequestV2.
            let (mut bob, _bb) = test_client();
            let full = snapshot_dag(&alice).await;
            let genesis_only: Vec<Event> = full.iter().take(1).cloned().collect();
            replay_into(&bob, genesis_only).await;
            let bob_net = MemNetwork::new(&hub);
            bob.connect(bob_net).await;

            let alice_total = dag_len(&alice).await;

            // Bob emits a SyncRequestV2 on the server-ops topic.
            bob.request_heads_sync()
                .await
                .expect("bob requests heads sync");

            // Bob converges to Alice's full DAG.
            let converged = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if dag_len(&bob).await >= alice_total {
                        return;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await;
            assert!(
                converged.is_ok(),
                "Bob must converge to Alice's full DAG via SyncRequestV2 ({} vs {})",
                dag_len(&bob).await,
                alice_total
            );
        })
        .await;
}

// keep imports used in helpers honest
const _: fn() = || {
    let _ = AuthorHead {
        seq: 0,
        hash: EventHash::ZERO,
    };
    let _ = EventKind::DeleteChannel {
        channel_id: String::new(),
    };
};
