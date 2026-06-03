//! Integration test pinning the relay's **content-agnostic forwarding
//! contract** for the `HistorySyncComplete` end-of-stored-events marker
//! (spec `docs/specs/2026-04-24-history-sync-eose.md`, plan PR 5 Task 5.4).
//!
//! The relay is a regular gossip participant: it forwards gossip bytes through
//! the iroh-gossip mesh transparently. Its **only** wire-decoding code path is
//! [`willow_relay::topic_announce_listener`], which acts solely on
//! [`willow_common::WireMessage::TopicAnnounce`] and treats every other variant
//! as opaque pass-through. This test pins that a `HistorySyncComplete`-bearing
//! envelope survives forwarding **byte-for-byte unchanged** and is never
//! consumed, rewritten, or filtered by the relay — a regression guard against a
//! future size/type filter on the gossip path silently dropping the marker.

use std::time::Duration;

use willow_common::{pack_wire, unpack_wire, WireMessage};
use willow_network::mem::{MemHub, MemNetwork};
use willow_network::traits::{GossipEvent, Network, TopicEvents, TopicHandle};
use willow_state::EventHash;

/// A representative marker payload reused across assertions.
fn sample_marker() -> WireMessage {
    WireMessage::HistorySyncComplete {
        topic_id: [0x5A; 32],
        last_event_hash: Some(EventHash::from_bytes(b"last-streamed-event")),
        stream_generation: 0x0123_4567_89AB_CDEF,
    }
}

/// Yield repeatedly so the relay's listener task gets a chance to run.
async fn settle() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
}

/// The relay forwards a `HistorySyncComplete`-bearing envelope **unchanged**.
///
/// Topology: a provider, a consumer, and the relay all subscribe to one channel
/// topic. The relay runs its real wire-inspection listener
/// (`topic_announce_listener`). The provider broadcasts the signed marker
/// envelope; the consumer must receive the **byte-identical** envelope and
/// `unpack_wire` it back into the exact same marker, attributed to the original
/// provider as the verified signer. The relay must not have consumed, altered,
/// or filtered it.
#[tokio::test]
async fn relay_forwards_history_sync_complete_unchanged() {
    let hub = MemHub::new();
    let provider_net = MemNetwork::new(&hub);
    let consumer_net = MemNetwork::new(&hub);
    let relay_net = MemNetwork::new(&hub);

    let provider_id = provider_net.id();
    let provider_identity = provider_net.identity().clone();

    // The channel topic the marker rides on, plus the relay's ops topic.
    let channel_topic = willow_network::topic_id("server-xyz/general");
    let ops_topic = willow_network::topic_id("_willow_server_ops");

    // Provider + consumer subscribe to the channel topic.
    let (provider_handle, _provider_events) =
        provider_net.subscribe(channel_topic, vec![]).await.unwrap();
    let (_consumer_handle, mut consumer_events) =
        consumer_net.subscribe(channel_topic, vec![]).await.unwrap();

    // The relay subscribes to BOTH the ops topic (so the announce listener can
    // run) and the channel topic (so it participates in the marker's mesh),
    // exactly as it would after dynamically subscribing to an announced topic.
    let (_relay_channel_handle, _relay_channel_events) =
        relay_net.subscribe(channel_topic, vec![]).await.unwrap();
    let (_relay_ops_handle, relay_ops_events) =
        relay_net.subscribe(ops_topic, vec![]).await.unwrap();

    // Spawn the relay's real wire-inspection path on the ops topic.
    let listener = tokio::spawn(willow_relay::topic_announce_listener::<MemNetwork>(
        relay_ops_events,
        relay_net,
    ));

    // Give subscriptions time to establish.
    settle().await;

    // The provider signs and broadcasts the marker on the channel topic.
    let marker = sample_marker();
    let packed = pack_wire(&marker, &provider_identity).expect("pack marker");
    let sent_bytes = packed.clone();
    provider_handle
        .broadcast(bytes::Bytes::from(packed))
        .await
        .expect("broadcast marker");

    // The consumer must receive the marker, byte-for-byte unchanged.
    let received = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match consumer_events.next().await {
                Some(Ok(GossipEvent::Received(msg))) => return msg,
                Some(_) => continue,
                None => panic!("consumer stream closed before receiving marker"),
            }
        }
    })
    .await
    .expect("consumer did not receive forwarded marker within timeout");

    // Byte-for-byte identity: the relay must not rewrite the envelope.
    assert_eq!(
        received.content.as_ref(),
        sent_bytes.as_slice(),
        "relay forwarded marker bytes were altered in transit"
    );

    // And the forwarded bytes still decode to the exact same marker, attributed
    // to the original provider as the verified envelope signer (the provider
    // identity is NOT carried in the payload).
    let (decoded, signer) = unpack_wire(&received.content).expect("forwarded marker decodes");
    assert_eq!(
        signer, provider_id,
        "provider identity must survive as the verified signer"
    );
    match decoded {
        WireMessage::HistorySyncComplete {
            topic_id,
            last_event_hash,
            stream_generation,
        } => {
            assert_eq!(topic_id, [0x5A; 32]);
            assert_eq!(
                last_event_hash,
                Some(EventHash::from_bytes(b"last-streamed-event"))
            );
            assert_eq!(stream_generation, 0x0123_4567_89AB_CDEF);
        }
        other => panic!("expected HistorySyncComplete, got {other:?}"),
    }

    listener.abort();
}

/// The relay's wire-inspection listener treats `HistorySyncComplete` as opaque:
/// feeding the marker directly onto the relay's ops topic must NOT cause the
/// relay to subscribe to anything (the marker is not a `TopicAnnounce`), and the
/// listener must keep running and still process a later genuine announce.
///
/// This is the sharpest pin of the content-agnostic contract: even on the one
/// topic where the relay *does* decode wire messages, the marker is a no-op, so
/// it can never be consumed or rewritten on any topic.
#[tokio::test]
async fn relay_announce_listener_ignores_history_sync_complete() {
    let hub = MemHub::new();
    let provider_net = MemNetwork::new(&hub);
    let relay_net = MemNetwork::new(&hub);
    let observer_net = MemNetwork::new(&hub);
    let provider_identity = provider_net.identity().clone();

    let ops_topic = willow_network::topic_id("_willow_server_ops");

    // A sentinel topic: if (and only if) the relay processes a genuine announce
    // do we expect it to subscribe and the observer to see NeighborUp.
    let sentinel = "history-eose-sentinel".to_string();
    let sentinel_topic = willow_network::topic_id(&sentinel);
    let (_, mut observer_events) = observer_net
        .subscribe(sentinel_topic, vec![])
        .await
        .unwrap();

    let (provider_handle, _) = provider_net.subscribe(ops_topic, vec![]).await.unwrap();
    let (_, relay_ops_events) = relay_net.subscribe(ops_topic, vec![]).await.unwrap();
    let relay_id = relay_net.id();

    let listener = tokio::spawn(willow_relay::topic_announce_listener::<MemNetwork>(
        relay_ops_events,
        relay_net,
    ));

    settle().await;

    // Send the marker ON THE OPS TOPIC — exactly where the relay decodes wire
    // messages. The relay must treat it as a no-op (not an announce).
    let marker = pack_wire(&sample_marker(), &provider_identity).expect("pack marker");
    provider_handle
        .broadcast(bytes::Bytes::from(marker))
        .await
        .expect("broadcast marker on ops topic");
    settle().await;

    // Now send a GENUINE announce for the sentinel topic. The listener must
    // still be running (the marker did not crash/consume it) and subscribe.
    let announce = pack_wire(
        &WireMessage::TopicAnnounce {
            topics: vec![sentinel.clone()],
        },
        &provider_identity,
    )
    .expect("pack announce");
    provider_handle
        .broadcast(bytes::Bytes::from(announce))
        .await
        .expect("broadcast announce");

    // The observer sees NeighborUp from the relay ⇒ the listener processed the
    // announce after the marker, proving the marker was a harmless no-op.
    let neighbor = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match observer_events.next().await {
                Some(Ok(GossipEvent::NeighborUp(id))) => return id,
                Some(_) => continue,
                None => panic!("observer stream closed unexpectedly"),
            }
        }
    })
    .await
    .expect("relay did not subscribe to sentinel after marker — listener may have consumed marker");

    assert_eq!(
        neighbor, relay_id,
        "expected the relay to subscribe to the sentinel announce after the marker"
    );

    listener.abort();
}
