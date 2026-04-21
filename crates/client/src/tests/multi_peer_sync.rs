//! Multi-peer state synchronization tests.
//!
//! These tests cover the *sync semantics* that used to live in
//! `e2e/multi-peer-sync.spec.ts` — message / reaction / edit / delete
//! propagation, display-name metadata, typing-indicator state,
//! post-refresh replay, pre-existing-history replay, and missed-message
//! catch-up.
//!
//! DOM-reflection tests (sidebar rendering, channel switching, member
//! list surface) stay in Playwright. Anything that asserts *state*
//! converges across two peers belongs here — the test runs in <200 ms
//! instead of booting two browser contexts against a relay.
//!
//! ## Fixture pattern
//!
//! [`connected_pair`] mirrors the helper in `trust_flow.rs`:
//! * Alice owns the server (`test_client()` genesis author).
//! * Alice grants Bob `SendMessages` so his messages are accepted.
//! * Alice's DAG is replayed into Bob's state actors so the two peers
//!   start with identical state.
//! * Both clients connect to a shared [`MemHub`] for gossip.
//!
//! Tests that need a more specialised setup (pre-existing messages,
//! reconnect replay) build their own hub / replay sequence directly.

use std::sync::Arc;
use std::time::Duration;

use crate::{test_client, ClientEvent, ClientHandle};

use willow_actor::Addr;
use willow_actor::Broker;
use willow_network::mem::{MemHub, MemNetwork};
use willow_state::EventHash;

// ───── Helpers ──────────────────────────────────────────────────────────

/// Snapshot Alice's DAG (genesis + any owner-side events so far).
async fn snapshot_dag<N: willow_network::Network>(
    client: &ClientHandle<N>,
) -> Vec<willow_state::Event> {
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

/// Overwrite `target`'s managed DAG + materialized state with `events`.
/// Used to fast-forward a fresh client to Alice's current state.
async fn replay_dag_into<N: willow_network::Network>(
    target: &ClientHandle<N>,
    events: Vec<willow_state::Event>,
) {
    let events_for_dag = events.clone();
    willow_actor::state::mutate(&target.dag_addr, move |ds| {
        ds.managed = willow_state::ManagedDag::empty(5000);
        for event in events_for_dag {
            ds.managed.insert_and_apply(event).ok();
        }
    })
    .await;
    let state =
        willow_actor::state::select(&target.dag_addr, |ds| ds.managed.state().clone()).await;
    willow_actor::state::mutate(&target.event_state_addr, move |es| {
        *es = state;
    })
    .await;
}

/// Build two connected clients on a shared [`MemHub`]. Bob receives
/// `SendMessages` so his messages are accepted when Alice applies them.
async fn connected_pair() -> (
    ClientHandle<MemNetwork>,
    Addr<Broker<ClientEvent>>,
    ClientHandle<MemNetwork>,
    Addr<Broker<ClientEvent>>,
) {
    let hub = Arc::new(MemHub::new());

    let (mut alice, alice_broker) = test_client();
    let alice_net = MemNetwork::new(&hub);
    alice.connect(alice_net).await;

    let (mut bob, bob_broker) = test_client();
    let bob_peer = bob.identity.endpoint_id();

    alice
        .mutations()
        .grant_permission(bob_peer, willow_state::Permission::SendMessages)
        .await
        .expect("grant SendMessages");

    let events = snapshot_dag(&alice).await;
    replay_dag_into(&bob, events).await;

    let bob_net = MemNetwork::new(&hub);
    bob.connect(bob_net).await;

    (alice, alice_broker, bob, bob_broker)
}

/// Poll `client`'s event store until `general` contains a message with
/// body `body`. Returns `true` on arrival, `false` on deadline.
async fn wait_for_message<N: willow_network::Network>(
    client: &ClientHandle<N>,
    body: &str,
    deadline: Duration,
) -> bool {
    tokio::time::timeout(deadline, async {
        loop {
            let general_id = general_channel_id(client).await;
            if !general_id.is_empty()
                && client
                    .event_messages(&general_id)
                    .await
                    .iter()
                    .any(|m| m.body == body)
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok()
}

/// Look up the `general` channel id from a client's materialized state.
async fn general_channel_id<N: willow_network::Network>(client: &ClientHandle<N>) -> String {
    willow_actor::state::select(&client.event_state_addr, |es| {
        es.channels
            .iter()
            .find(|(_, ch)| ch.name == "general")
            .map(|(id, _)| id.clone())
            .unwrap_or_default()
    })
    .await
}

/// Find the [`EventHash`] of the message with `body` in the `general`
/// channel, regardless of `deleted` flag.
async fn find_message_hash<N: willow_network::Network>(
    client: &ClientHandle<N>,
    body: &str,
) -> Option<EventHash> {
    let body = body.to_string();
    willow_actor::state::select(&client.event_state_addr, move |es| {
        es.messages.iter().find(|m| m.body == body).map(|m| m.id)
    })
    .await
}

/// Poll until `predicate(msg)` holds for *some* message matching `body`.
async fn wait_until<N, F>(
    client: &ClientHandle<N>,
    body: &str,
    deadline: Duration,
    mut predicate: F,
) -> bool
where
    N: willow_network::Network,
    F: FnMut(&willow_state::ChatMessage) -> bool,
{
    tokio::time::timeout(deadline, async {
        loop {
            let body = body.to_string();
            let hit = willow_actor::state::select(&client.event_state_addr, move |es| {
                es.messages.iter().find(|m| m.body == body).cloned()
            })
            .await;
            if let Some(m) = hit {
                if predicate(&m) {
                    return;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok()
}

/// Poll until the `ChatMessage` with `hash` satisfies `predicate`. Used
/// for post-delete / post-edit assertions where the body itself has
/// mutated and can no longer serve as the lookup key.
async fn wait_until_by_hash<N, F>(
    client: &ClientHandle<N>,
    hash: EventHash,
    deadline: Duration,
    mut predicate: F,
) -> bool
where
    N: willow_network::Network,
    F: FnMut(&willow_state::ChatMessage) -> bool,
{
    tokio::time::timeout(deadline, async {
        loop {
            let hit = willow_actor::state::select(&client.event_state_addr, move |es| {
                es.messages.iter().find(|m| m.id == hash).cloned()
            })
            .await;
            if let Some(m) = hit {
                if predicate(&m) {
                    return;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok()
}

// ───── 1. messages sync both directions ─────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messages_sync_both_directions() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (alice, _alice_broker, bob, _bob_broker) = connected_pair().await;

            // Alice -> Bob.
            alice
                .send_message("general", "Hello from Alice")
                .await
                .expect("Alice can send");
            assert!(
                wait_for_message(&bob, "Hello from Alice", Duration::from_secs(5)).await,
                "Bob must receive Alice's message"
            );

            // Bob -> Alice.
            bob.send_message("general", "Hello from Bob")
                .await
                .expect("Bob can send after GrantPermission");
            assert!(
                wait_for_message(&alice, "Hello from Bob", Duration::from_secs(5)).await,
                "Alice must receive Bob's message"
            );
        })
        .await;
}

// ───── 2. reactions sync between peers ──────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reactions_sync_between_peers() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (alice, _alice_broker, bob, _bob_broker) = connected_pair().await;

            alice
                .send_message("general", "react to this")
                .await
                .expect("Alice can send");
            assert!(
                wait_for_message(&bob, "react to this", Duration::from_secs(5)).await,
                "Bob sees the original message"
            );

            let msg_hash = find_message_hash(&alice, "react to this")
                .await
                .expect("message must exist in Alice's state");

            alice
                .mutations()
                .react(&msg_hash, "thumbsup")
                .await
                .expect("Alice reacts");

            // Alice sees her own reaction.
            assert!(
                wait_until(&alice, "react to this", Duration::from_secs(5), |m| {
                    m.reactions
                        .get("thumbsup")
                        .map(|set| !set.is_empty())
                        .unwrap_or(false)
                })
                .await,
                "Alice must see her reaction locally"
            );

            // Bob sees the reaction via gossip.
            assert!(
                wait_until(&bob, "react to this", Duration::from_secs(5), |m| {
                    m.reactions
                        .get("thumbsup")
                        .map(|set| !set.is_empty())
                        .unwrap_or(false)
                })
                .await,
                "Bob must see the reaction synced"
            );
        })
        .await;
}

// ───── 3. edits sync between peers ──────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edits_sync_between_peers() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (alice, _alice_broker, bob, _bob_broker) = connected_pair().await;

            alice
                .send_message("general", "original text")
                .await
                .expect("Alice can send");
            assert!(
                wait_for_message(&bob, "original text", Duration::from_secs(5)).await,
                "Bob sees the original message"
            );

            let msg_hash = find_message_hash(&alice, "original text")
                .await
                .expect("message in Alice's state");

            alice
                .mutations()
                .edit_message(&msg_hash, "edited text")
                .await
                .expect("Alice edits");

            // Edit replaces the body; assert the updated body landed on both sides.
            assert!(
                wait_for_message(&alice, "edited text", Duration::from_secs(5)).await,
                "Alice's own store must reflect the edit"
            );
            assert!(
                wait_for_message(&bob, "edited text", Duration::from_secs(5)).await,
                "Bob must receive the edited body"
            );
        })
        .await;
}

// ───── 4. deletes sync between peers ────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deletes_sync_between_peers() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (alice, _alice_broker, bob, _bob_broker) = connected_pair().await;

            alice
                .send_message("general", "delete me soon")
                .await
                .expect("Alice can send");
            assert!(
                wait_for_message(&bob, "delete me soon", Duration::from_secs(5)).await,
                "Bob sees the original message before delete"
            );

            let msg_hash = find_message_hash(&alice, "delete me soon")
                .await
                .expect("message in Alice's state");

            alice
                .mutations()
                .delete_message(&msg_hash)
                .await
                .expect("Alice deletes");

            // Deleted flag flips on both sides. `event_messages` filters
            // soft-deleted rows out and the body is rewritten to
            // `[message deleted]`, so we look up by hash instead.
            assert!(
                wait_until_by_hash(&alice, msg_hash, Duration::from_secs(5), |m| m.deleted).await,
                "Alice must see her own delete applied"
            );
            assert!(
                wait_until_by_hash(&bob, msg_hash, Duration::from_secs(5), |m| m.deleted).await,
                "Bob must see the delete synced"
            );
        })
        .await;
}

// ───── 5. messages persist across a fresh client (refresh) ──────────────

/// Simulates a browser refresh: we snapshot Alice's DAG, spawn a fresh
/// `test_client` for each peer, replay the DAG in, and assert the
/// messages are still present. The real app re-hydrates from the event
/// store on page load — this exercises the same "replay DAG → state"
/// path without the browser in the loop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messages_persist_after_refresh_for_both_peers() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (alice, _alice_broker, bob, _bob_broker) = connected_pair().await;

            alice
                .send_message("general", "persistent msg")
                .await
                .expect("Alice can send");
            assert!(
                wait_for_message(&bob, "persistent msg", Duration::from_secs(5)).await,
                "Bob receives persistent msg"
            );

            // Snapshot each peer's DAG and "refresh" by building a new
            // client + replaying the captured events.
            let alice_events = snapshot_dag(&alice).await;
            let bob_events = snapshot_dag(&bob).await;

            let (alice2, _a2) = test_client();
            replay_dag_into(&alice2, alice_events).await;
            let (bob2, _b2) = test_client();
            replay_dag_into(&bob2, bob_events).await;

            assert!(
                wait_for_message(&alice2, "persistent msg", Duration::from_secs(2)).await,
                "Alice's refreshed store must still contain the message"
            );
            assert!(
                wait_for_message(&bob2, "persistent msg", Duration::from_secs(2)).await,
                "Bob's refreshed store must still contain the message"
            );
        })
        .await;
}

// ───── 6. typing indicator shows on other peer ──────────────────────────

/// Typing is not broadcast through the event DAG — the web client uses
/// an ephemeral `TypingIndicator` wire message via
/// [`ClientHandle::send_typing`], and the receiver calls
/// [`ClientMutations::record_typing`]. This test drives `record_typing`
/// directly on Bob to assert the state surface (`typing_peers`) the UI
/// binds to.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typing_indicator_updates_peer_state() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (alice, _alice_broker, bob, _bob_broker) = connected_pair().await;

            // Simulate Bob receiving a typing signal from Alice.
            let alice_peer = alice.identity.endpoint_id();
            bob.mutations()
                .record_typing(alice_peer, "general".to_string())
                .await;

            let typing = bob.typing_peers().await;
            assert!(
                typing
                    .iter()
                    .any(|(pid, ch)| pid == &alice_peer.to_string() && ch == "general"),
                "Bob must see Alice in typing_peers, got {typing:?}"
            );
        })
        .await;
}

// ───── 7. display names shown in messages ───────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn display_names_attached_to_messages() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (alice, _alice_broker, bob, _bob_broker) = connected_pair().await;

            // Alice advertises her display name via the DAG-backed
            // `SetProfile` event — this is what the UI's name-entry
            // path does and is what propagates to Bob over gossip.
            alice
                .set_server_display_name("Alice")
                .await
                .expect("Alice sets display name");

            alice
                .send_message("general", "check my name")
                .await
                .expect("Alice can send");
            assert!(
                wait_for_message(&bob, "check my name", Duration::from_secs(5)).await,
                "Bob receives the message"
            );

            // The SetProfile event and the message event race through
            // gossip independently; poll until Bob's state reflects
            // Alice's new display name.
            let resolved = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let msgs = bob.messages("general").await;
                    if let Some(m) = msgs.iter().find(|m| m.body == "check my name") {
                        if m.author_display_name == "Alice" {
                            return m.author_display_name.clone();
                        }
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await;
            assert!(
                resolved.is_ok(),
                "display name must resolve to Alice on Bob's side"
            );
        })
        .await;
}

// ───── 8. pre-existing messages visible to peer who joins later ─────────

/// Mirrors the `SyncBatch` history-replay path: Alice sends messages
/// *before* Bob joins, then Bob replays Alice's DAG on join. The iroh
/// gossip path delivers these as a `SyncBatch`; the MemNetwork hub does
/// not, so we stand in for history replay by snapshotting Alice's DAG
/// and replaying into Bob before he connects. The state-level assertion
/// — Bob's store contains the pre-existing messages after join — is
/// identical to what the e2e test checked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pre_existing_messages_visible_to_joiner() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let hub = Arc::new(MemHub::new());

            let (mut alice, _alice_broker) = test_client();
            let alice_net = MemNetwork::new(&hub);
            alice.connect(alice_net).await;

            // Fresh identity for Bob; grant him SendMessages so his DAG
            // replay includes permission state identical to Alice's.
            let (bob, _bob_broker) = test_client();
            let bob_peer = bob.identity.endpoint_id();
            alice
                .mutations()
                .grant_permission(bob_peer, willow_state::Permission::SendMessages)
                .await
                .expect("grant SendMessages");

            // Alice sends three messages *before* Bob joins.
            alice
                .send_message("general", "msg before join 1")
                .await
                .expect("send 1");
            alice
                .send_message("general", "msg before join 2")
                .await
                .expect("send 2");
            alice
                .send_message("general", "msg before join 3")
                .await
                .expect("send 3");

            // Bob joins: history-replay via the captured DAG.
            let events = snapshot_dag(&alice).await;
            replay_dag_into(&bob, events).await;

            // All three pre-existing messages are visible.
            for body in [
                "msg before join 1",
                "msg before join 2",
                "msg before join 3",
            ] {
                assert!(
                    wait_for_message(&bob, body, Duration::from_secs(2)).await,
                    "Bob must see pre-existing message {body:?} after join"
                );
            }
        })
        .await;
}

// ───── 9. missed messages received after peer reconnects ────────────────

/// `MemHub` has no retained-message semantics, so we simulate the
/// offline / reconnect path by snapshotting Alice's DAG (which grows
/// while Bob is detached) and replaying it into Bob on reconnect.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missed_messages_replayed_on_reconnect() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (alice, _alice_broker, bob, _bob_broker) = connected_pair().await;

            // Baseline: both online, message lands.
            alice
                .send_message("general", "before disconnect")
                .await
                .expect("send baseline");
            assert!(
                wait_for_message(&bob, "before disconnect", Duration::from_secs(5)).await,
                "baseline must land before disconnect"
            );

            // Alice sends a message while Bob is offline. We don't
            // actually tear Bob's MemNetwork down — we just assert the
            // end-state after replay, since that's what the UI sees
            // post-reconnect.
            alice
                .send_message("general", "sent while offline")
                .await
                .expect("send while offline");

            // Reconnect replay: capture Alice's DAG and replay into Bob.
            // On a real reconnect this is the SyncRequest/SyncBatch path.
            let events = snapshot_dag(&alice).await;
            replay_dag_into(&bob, events).await;

            assert!(
                wait_for_message(&bob, "sent while offline", Duration::from_secs(2)).await,
                "Bob must receive the missed message after reconnect replay"
            );
        })
        .await;
}
