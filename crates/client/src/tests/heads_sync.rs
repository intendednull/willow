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
use crate::{test_client, ClientEvent, ClientHandle, EventReceiver};

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
        history_stream_generation: 0,
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

// ───── 3. a regular member (not owner/admin, no grant) refuses to serve ────
//
// Reconciled 2026-05-30: the responder gate is `ServerState::is_sync_provider`,
// which honors the authority model — the owner/admins hold `SyncProvider`
// *implicitly* (owner = root of all permissions; admins inherit every
// permission), and explicit grant-holders serve too. A *regular* member who is
// neither owner/admin nor explicitly granted `SyncProvider` still refuses, so
// the gate remains meaningful. This test pins that residual refusal.
//
// Earlier this test asserted the *owner herself* served nothing without an
// explicit self-grant; that encoded the now-corrected strict-explicit deviation
// from pinned decision 4 (which the owner satisfies implicitly). The owner-serve
// path is now pinned by `owner_without_explicit_grant_serves` below.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn regular_member_without_sync_provider_serves_nothing() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let hub = Arc::new(MemHub::new());

            // Alice owns the server and authors a chain.
            let (alice, _ab) = test_client();
            for i in 0..5 {
                alice
                    .send_message("general", &format!("m{i}"))
                    .await
                    .expect("send");
            }
            let server_id = server_id_of(&alice).await;

            // Carol is a *regular member*: she holds Alice's full DAG (replayed
            // in) but her own identity is NOT the genesis author, so she is
            // neither owner nor admin, and was never granted SyncProvider.
            let (carol, _cb) = test_client();
            let alice_full = snapshot_dag(&alice).await;
            replay_into(&carol, alice_full).await;
            let carol_local = carol.identity.endpoint_id();
            let (carol_is_admin, carol_explicit, carol_provider) =
                willow_actor::state::select(&carol.event_state_addr, move |es| {
                    (
                        es.is_admin(&carol_local),
                        es.has_explicit_permission(
                            &carol_local,
                            &willow_state::Permission::SyncProvider,
                        ),
                        es.is_sync_provider(&carol_local),
                    )
                })
                .await;
            assert!(!carol_is_admin, "test precondition: Carol is not an admin");
            assert!(
                !carol_explicit,
                "test precondition: Carol has no explicit SyncProvider grant"
            );
            assert!(
                !carol_provider,
                "test precondition: Carol is not a SyncProvider by any path"
            );

            let carol_ctx = make_ctx(&carol);
            let topic_id = willow_network::topic_id("heads-sync-test-3");
            let carol_net = MemNetwork::new(&hub);
            let (carol_topic, _e) = carol_net.subscribe(topic_id, vec![]).await.expect("sub");
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
                request_and_collect(&carol_ctx, &carol_topic, &mut reader_events, &bob, request)
                    .await;
            assert!(
                batches.is_empty(),
                "a regular member without SyncProvider must not serve a SyncRequestV2 (got {} batches)",
                batches.len()
            );
        })
        .await;
}

// ───── 3a. the server OWNER serves without an explicit self-grant ─────────
//
// Bug fix 2026-05-30: the SyncRequestV2 gate previously used
// `has_explicit_permission`, which ignores the owner/admin implicit-permission
// rule, so the server owner — the canonical source of her own server's state —
// refused to serve a joining peer. In the web E2E mesh nobody holds an explicit
// grant, so joiners could never backfill. Pinned decision 4 says "a peer serves
// a delta only if it holds SyncProvider", and the owner DOES hold it implicitly
// (authority model: owner = root of all permissions). The gate now honors that.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_without_explicit_grant_serves() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let hub = Arc::new(MemHub::new());

            // Alice owns the server (genesis author = auto-admin) and authors a
            // chain. She does NOT grant herself an explicit SyncProvider.
            let (alice, _ab) = test_client();
            let alice_peer = alice.identity.endpoint_id();
            let (is_admin, has_explicit) =
                willow_actor::state::select(&alice.event_state_addr, move |es| {
                    (
                        es.is_admin(&alice_peer),
                        es.has_explicit_permission(
                            &alice_peer,
                            &willow_state::Permission::SyncProvider,
                        ),
                    )
                })
                .await;
            assert!(is_admin, "test precondition: owner is auto-admin");
            assert!(
                !has_explicit,
                "test precondition: owner holds NO explicit SyncProvider grant"
            );

            for i in 0..8 {
                alice
                    .send_message("general", &format!("owner {i}"))
                    .await
                    .expect("send");
            }
            let server_id = server_id_of(&alice).await;
            let alice_total = dag_len(&alice).await;

            let alice_ctx = make_ctx(&alice);
            let topic_id = willow_network::topic_id("heads-sync-test-3a");
            let alice_net = MemNetwork::new(&hub);
            let (alice_topic, _e) = alice_net.subscribe(topic_id, vec![]).await.expect("sub");
            let reader_net = MemNetwork::new(&hub);
            let (_rt, mut reader_events) = reader_net
                .subscribe(topic_id, vec![])
                .await
                .expect("sub reader");

            // Bob requests everything (empty heads).
            let bob = willow_identity::Identity::generate();
            let request = crate::ops::WireMessage::SyncRequestV2 {
                request_id: "req-3a".to_string(),
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
                !batches.is_empty(),
                "the owner must serve a SyncRequestV2 even without an explicit grant"
            );
            assert!(!batches.last().unwrap().1, "final batch clears `more`");
            let total: usize = batches.iter().map(|(e, _)| e.len()).sum();
            assert_eq!(
                total, alice_total,
                "owner streams the entire chain to a joining peer"
            );
        })
        .await;
}

// ───── 3b. legacy SyncRequest responder stays UNGATED ─────────────────────
//
// Pinned decision 4 gates *only* the new `SyncRequestV2` responder. The
// legacy 500-event `SyncRequest`/`SyncBatch` path is retained ungated for the
// migration window (it is removed later with the legacy variants). This pins
// that asymmetry: the *same* peer that refuses a `SyncRequestV2` without
// `SyncProvider` (test 3) still answers an inbound legacy `SyncRequest`.

/// Pump a single legacy `SyncRequest` into `responder` and collect every
/// legacy `SyncBatch` it broadcasts back, in order. Mirrors
/// [`request_and_collect`] but for the un-versioned wire variants.
async fn request_legacy_and_collect(
    responder_ctx: &ListenerCtx,
    responder_topic: &impl willow_network::traits::TopicHandle,
    reader: &mut impl willow_network::traits::TopicEvents,
    requester_identity: &willow_identity::Identity,
    request: crate::ops::WireMessage,
) -> Vec<Vec<Event>> {
    use willow_network::traits::GossipEvent;

    let req_bytes = crate::ops::pack_wire(&request, requester_identity).expect("pack request");
    let requester_id = requester_identity.endpoint_id();

    process_received_message(&req_bytes, requester_id, responder_ctx, responder_topic).await;

    // The legacy responder emits at most one `SyncBatch` (no terminator), so
    // drain for a short bounded window and collect whatever arrives.
    let mut batches = Vec::new();
    let _ = tokio::time::timeout(Duration::from_millis(500), async {
        while let Some(Ok(ev)) = reader.next().await {
            let GossipEvent::Received(msg) = ev else {
                continue;
            };
            if let Some((crate::ops::WireMessage::SyncBatch { events }, _signer)) =
                crate::ops::unpack_wire(&msg.content)
            {
                batches.push(events);
            }
        }
    })
    .await;
    batches
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_sync_request_responder_stays_ungated() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let hub = Arc::new(MemHub::new());
            // Alice does NOT grant herself SyncProvider — exactly the setup
            // under which she refuses a SyncRequestV2 (test 3).
            let (alice, _b) = test_client();
            for i in 0..5 {
                alice
                    .send_message("general", &format!("legacy {i}"))
                    .await
                    .expect("send");
            }
            let alice_total = dag_len(&alice).await;

            let alice_ctx = make_ctx(&alice);
            let topic_id = willow_network::topic_id("heads-sync-test-3b");
            let alice_net = MemNetwork::new(&hub);
            let (alice_topic, _e) = alice_net.subscribe(topic_id, vec![]).await.expect("sub");
            let reader_net = MemNetwork::new(&hub);
            let (_rt, mut reader_events) = reader_net
                .subscribe(topic_id, vec![])
                .await
                .expect("sub reader");

            let bob = willow_identity::Identity::generate();
            // Legacy request: `state_hash` is ignored by the responder.
            let request = crate::ops::WireMessage::SyncRequest {
                state_hash: EventHash::ZERO,
                topic: None,
            };
            let batches = request_legacy_and_collect(
                &alice_ctx,
                &alice_topic,
                &mut reader_events,
                &bob,
                request,
            )
            .await;

            assert_eq!(
                batches.len(),
                1,
                "legacy SyncRequest responder must still answer without SyncProvider"
            );
            let served: usize = batches.iter().map(|e| e.len()).sum();
            assert_eq!(
                served, alice_total,
                "legacy responder ships the full (≤500) topological prefix ungated"
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

// ───── 5. owner-served gossip backfill emits a HistorySyncComplete the ─────
//        joiner surfaces as ClientEvent::HistorySynced (provider = owner)
//
// Bug fix 2026-05-30: the HistorySyncComplete EOSE marker was previously emitted
// ONLY by workers in response to a WorkerRequest::Sync. Web clients backfill
// over gossip (SyncRequestV2) and never touch the worker ALPN path, so no marker
// ever reached them — the EOSE feature was dead end-to-end for clients. The
// gossip SyncRequestV2 responder now also emits the marker after a successful
// serve, and the receiver's trust gate honors the owner/admins (Change B), so a
// peer/owner-served backfill produces an observable HistorySynced.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_served_backfill_emits_history_synced() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let hub = Arc::new(MemHub::new());

            // Alice owns the server (genesis author) and authors a chain. No
            // explicit SyncProvider self-grant — she serves via the owner rule.
            let (alice, _ab) = test_client();
            for i in 0..6 {
                alice
                    .send_message("general", &format!("hist {i}"))
                    .await
                    .expect("send");
            }
            let server_id = server_id_of(&alice).await;
            let alice_total = dag_len(&alice).await;
            let alice_peer = alice.identity.endpoint_id();

            // Bob is a joiner that holds only Alice's genesis prefix, so Alice's
            // marker is trusted (Alice is the owner in Bob's replayed state) and
            // the backfill is non-empty.
            let (bob, bob_broker) = test_client();
            let full = snapshot_dag(&alice).await;
            let genesis_only: Vec<Event> = full.iter().take(1).cloned().collect();
            replay_into(&bob, genesis_only).await;
            let mut bob_rx = EventReceiver::subscribe(&bob_broker, &bob.system).await;

            // Shared topic: Alice broadcasts batches + marker; we drain them and
            // replay each frame into Bob's listener (the gossip delivery the
            // MemHub would otherwise perform), so Bob both converges AND sees the
            // EOSE marker.
            let topic_id = willow_network::topic_id("heads-sync-test-5");
            let alice_ctx = make_ctx(&alice);
            let alice_net = MemNetwork::new(&hub);
            let (alice_topic, _ae) = alice_net.subscribe(topic_id, vec![]).await.expect("sub");
            let reader_net = MemNetwork::new(&hub);
            let (_rt, mut reader_events) = reader_net
                .subscribe(topic_id, vec![])
                .await
                .expect("sub reader");

            // Bob requests everything (empty heads) — pump it into Alice.
            let bob_identity = bob.identity.clone();
            let request = crate::ops::WireMessage::SyncRequestV2 {
                request_id: "req-5".to_string(),
                heads: HeadsSummary::default(),
                filter: willow_common::SyncFilter {
                    server_id,
                    channels: None,
                    authors: None,
                    event_kinds: None,
                    since_ms: None,
                },
            };
            let req_bytes = crate::ops::pack_wire(&request, &bob_identity).expect("pack request");
            process_received_message(
                &req_bytes,
                bob_identity.endpoint_id(),
                &alice_ctx,
                &alice_topic,
            )
            .await;

            // Drain every frame Alice broadcast and replay it into Bob's
            // listener, stopping once we have seen the HistorySyncComplete marker.
            let bob_ctx = make_ctx(&bob);
            let bob_topic = {
                let bob_net = MemNetwork::new(&hub);
                let (h, _e) = bob_net
                    .subscribe(willow_network::topic_id("heads-sync-test-5-bob"), vec![])
                    .await
                    .expect("sub bob");
                h
            };
            use willow_network::traits::{GossipEvent, TopicEvents as _};
            let drained = tokio::time::timeout(Duration::from_secs(5), async {
                while let Some(Ok(ev)) = reader_events.next().await {
                    let GossipEvent::Received(msg) = ev else {
                        continue;
                    };
                    let sender = msg.sender;
                    // Replay verbatim into Bob's listener.
                    process_received_message(&msg.content, sender, &bob_ctx, &bob_topic).await;
                    if let Some((crate::ops::WireMessage::HistorySyncComplete { .. }, _signer)) =
                        crate::ops::unpack_wire(&msg.content)
                    {
                        // Marker is the last frame Alice emits after a serve.
                        break;
                    }
                }
            })
            .await;
            assert!(drained.is_ok(), "responder must emit a terminal marker");

            // Bob converged to Alice's full DAG.
            assert_eq!(
                dag_len(&bob).await,
                alice_total,
                "Bob converges to the owner's full DAG via the gossip backfill"
            );

            // Bob surfaced exactly one HistorySynced for the owner-served topic.
            let mut synced = Vec::new();
            let _ = tokio::time::timeout(Duration::from_millis(300), async {
                loop {
                    match bob_rx.try_recv() {
                        Some(ClientEvent::HistorySynced {
                            topic,
                            provider,
                            still_pending,
                        }) => synced.push((topic, provider, still_pending)),
                        Some(_) => {}
                        None => tokio::task::yield_now().await,
                    }
                }
            })
            .await;
            assert_eq!(
                synced.len(),
                1,
                "owner-served backfill emits exactly one HistorySynced (got {synced:?})"
            );
            assert_eq!(
                synced[0].1, alice_peer,
                "the HistorySynced provider is the owner (recovered from the marker signer)"
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
