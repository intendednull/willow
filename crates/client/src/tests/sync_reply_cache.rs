//! Tests for the cached `WireMessage::SyncRequest` reply payload.
//!
//! Per [GEN-08] (issue #268) the listener used to recompute
//! `topological_sort()` on every `SyncRequest` and then truncate to 500.
//! For a 50k-event server every sync request paid the full O(N) sort
//! and allocation. The fix caches the materialized first-N events on
//! `DagState` and invalidates on every successful insertion.
//!
//! These tests pin three behaviours:
//!
//! 1. **Semantic preservation.** The cached reply matches what
//!    `topological_sort().take(SYNC_REPLY_LIMIT)` would produce.
//! 2. **Invalidation correctness.** An insert between two reads
//!    surfaces in the second reply.
//! 3. **Cache-hit smoke test.** Two consecutive reads with no
//!    insertion in between return byte-identical Vecs (a regression
//!    canary in case the cache layer drops back to recomputing).
//!
//! [GEN-08]: https://github.com/willow-org/willow/issues/268

use crate::listeners::compute_sync_reply;
use crate::state_actors::SYNC_REPLY_LIMIT;
use crate::test_client;

/// Build a deterministic baseline by inserting `n` `SetProfile` events
/// (cheap, no permission checks beyond `SendMessages` which the genesis
/// author has implicitly via owner status) and return the sorted-prefix
/// the listener would ship.
async fn topological_prefix(
    client: &crate::ClientHandle<willow_network::mem::MemNetwork>,
) -> Vec<willow_state::Event> {
    willow_actor::state::select(&client.dag_addr, |ds| {
        ds.managed
            .dag()
            .topological_sort()
            .into_iter()
            .take(SYNC_REPLY_LIMIT)
            .cloned()
            .collect()
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cached_reply_matches_topological_sort_prefix() {
    // Semantic preservation. After enough inserts to cross the truncation
    // boundary, the cached reply must equal what the old un-cached path
    // produced.
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (client, _broker) = test_client();

            // test_client() seeds genesis (CreateServer) + one
            // CreateChannel, so we already have 2 events. Push enough
            // SetProfile events to comfortably exceed SYNC_REPLY_LIMIT.
            for i in 0..(SYNC_REPLY_LIMIT + 50) {
                client
                    .mutations()
                    .build_event(willow_state::EventKind::SetProfile {
                        display_name: format!("name-{i}"),
                    })
                    .await
                    .expect("local SetProfile must insert");
            }

            let expected = topological_prefix(&client).await;
            let cached = compute_sync_reply(&client.dag_addr).await;

            assert_eq!(
                cached.len(),
                SYNC_REPLY_LIMIT,
                "reply must be truncated to SYNC_REPLY_LIMIT"
            );
            // Event lacks PartialEq; compare hash sequences. Hashes
            // bind every signed field of an event, so identical hash
            // sequences imply identical event sequences.
            let cached_hashes: Vec<_> = cached.iter().map(|e| e.hash).collect();
            let expected_hashes: Vec<_> = expected.iter().map(|e| e.hash).collect();
            assert_eq!(
                cached_hashes, expected_hashes,
                "cached reply must match topological_sort().take(SYNC_REPLY_LIMIT)"
            );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_invalidates_on_insert() {
    // Invalidation correctness. A successful insertion between two
    // reads must surface in the second reply.
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (client, _broker) = test_client();

            // Genesis + one CreateChannel = 2 events, well under the cap.
            let first = compute_sync_reply(&client.dag_addr).await;
            let baseline_len = first.len();

            // Insert one more event via build_event (the path that
            // mutations like create_channel / send_message ultimately
            // funnel through).
            client
                .mutations()
                .build_event(willow_state::EventKind::SetProfile {
                    display_name: "after-cache-fill".into(),
                })
                .await
                .expect("local SetProfile must insert");

            let second = compute_sync_reply(&client.dag_addr).await;
            assert_eq!(
                second.len(),
                baseline_len + 1,
                "post-insert reply must include the new event"
            );
            assert!(
                second.iter().any(|e| matches!(
                    &e.kind,
                    willow_state::EventKind::SetProfile { display_name }
                        if display_name == "after-cache-fill"
                )),
                "post-insert reply must contain the SetProfile event"
            );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_hit_returns_identical_vec() {
    // Cache-hit smoke test. Two consecutive reads with no insertion in
    // between must produce byte-identical Vecs. Guards against a future
    // refactor that drops back to per-call recomputation.
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (client, _broker) = test_client();

            for i in 0..10 {
                client
                    .mutations()
                    .build_event(willow_state::EventKind::SetProfile {
                        display_name: format!("name-{i}"),
                    })
                    .await
                    .expect("local SetProfile must insert");
            }

            let first = compute_sync_reply(&client.dag_addr).await;
            let second = compute_sync_reply(&client.dag_addr).await;

            let first_hashes: Vec<_> = first.iter().map(|e| e.hash).collect();
            let second_hashes: Vec<_> = second.iter().map(|e| e.hash).collect();
            assert_eq!(
                first_hashes, second_hashes,
                "two consecutive reads with no insertion must yield identical replies"
            );
        })
        .await;
}
