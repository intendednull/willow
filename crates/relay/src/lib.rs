//! # Willow Relay (library)
//!
//! Helpers and constants used by the `willow-relay` binary. This is a
//! thin library wrapper so that the bootstrap-handler logic and topic
//! validation can be exercised by integration tests in
//! `crates/relay/tests/`. The actual `main` lives in `src/main.rs`.
//!
//! ## Scope: transport only
//!
//! All routines in this crate operate at the **transport layer**.
//! They forward bytes, manage gossip subscriptions, and rate-limit
//! incoming connections. They do **not** inspect, validate, or filter
//! application-level payloads:
//!
//! - No Ed25519 signature verification on relayed messages.
//! - No event-sourced state machine application (see `willow-state`).
//! - No permission, role, or governance enforcement.
//! - No content-based routing or filtering.
//!
//! The only semantic work performed here is syntactic topic-string
//! validation in [`topic_str_is_valid`] (length + allowed characters),
//! which is purely a DoS guard, not a trust or governance check.
//!
//! ## Trust model
//!
//! The relay is a **regular client** in the DAG sync protocol. Its
//! Ed25519 identity carries **no implicit authority** over any server.
//! Per the DAG spec
//! (`docs/specs/2026-04-01-per-author-merkle-dag-state-design.md`)
//! and `CLAUDE.md` "Trust Model":
//!
//! > The relay is a regular client — trusted only if explicitly granted
//! > `SyncProvider` permission by the owner.
//!
//! A hostile or compromised relay can affect **availability** (drop,
//! delay, or reorder messages) but cannot forge events, bypass
//! permissions, or corrupt state — those invariants are enforced
//! cryptographically and deterministically at each **client**.
//! Clients are therefore responsible for **all** DAG validation:
//! signatures, parent-hash chain integrity, permission checks, and
//! merge/replay correctness. Relayed bytes are never trusted on the
//! strength of having passed through this crate.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tracing::{info, warn};
use willow_network::traits::{GossipEvent, TopicEvents};
use willow_network::Network;

/// Maximum concurrent client connections accepted by the bootstrap-id
/// HTTP endpoint. Excess accepts are dropped immediately to prevent
/// FD/memory exhaustion under a connection-flood DoS.
pub const MAX_CONCURRENT_BOOTSTRAP_CONNECTIONS: usize = 1024;

/// HTTP path served by [`handle_bootstrap_connection`] to expose the
/// bootstrap node's endpoint ID. The public proxy routes requests to
/// this path into the bootstrap handler; any other path is proxied to
/// the internal iroh-relay server.
pub const BOOTSTRAP_ID_PATH: &str = "/bootstrap-id";

/// Maximum number of bytes to buffer while peeking the HTTP request
/// line during proxy dispatch. HTTP/1.1 permits long request URIs, but
/// for our single-route dispatch we only need the first line which
/// should always fit comfortably.
const PROXY_REQUEST_LINE_BUFFER: usize = 8 * 1024;

/// I/O deadline for any single read or write on a bootstrap-id
/// connection. Slow clients (Slowloris) are dropped at this deadline.
pub const BOOTSTRAP_IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum number of bytes buffered while draining the remainder of a
/// bootstrap-id request looking for the end-of-headers marker
/// (`\r\n\r\n`). Real HTTP requests for our single-route endpoint never
/// approach this size; a client that exceeds it is malformed or
/// abusive, and we close the connection rather than keep reading until
/// [`BOOTSTRAP_IO_TIMEOUT`]. Matches [`PROXY_REQUEST_LINE_BUFFER`] so
/// the relay applies a single, symmetric budget for header bytes.
pub const BOOTSTRAP_DRAIN_BUFFER_CAP: usize = 8 * 1024;

/// Maximum number of distinct channel topics the topic-announce
/// listener will subscribe to. Once this cap is reached the listener
/// silently drops further unique announces (after a one-shot warn).
pub const MAX_TOPICS: usize = 10_000;

/// Maximum length, in bytes, of a topic string accepted from a
/// `TopicAnnounce` message. Anything longer is rejected outright.
pub const MAX_TOPIC_LEN: usize = 256;

/// Returns `true` iff `s` is a syntactically valid channel topic
/// string: non-empty, no longer than [`MAX_TOPIC_LEN`] bytes, and
/// composed entirely of ASCII alphanumerics or `_ / : . -`.
pub fn topic_str_is_valid(s: &str) -> bool {
    if s.is_empty() || s.len() > MAX_TOPIC_LEN {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | ':' | '.' | '-'))
}

/// Serve the bootstrap-id response on a single connection with read
/// and write timeouts. Returns `Ok(())` on a successful exchange and
/// `Err` if either I/O step times out or fails. The HTTP response
/// always carries `Connection: close` so the client knows not to
/// pipeline another request on this socket.
pub async fn handle_bootstrap_connection<S>(mut stream: S, id: &str) -> std::io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Drain (best-effort) the request line; we don't actually parse it.
    let mut buf = [0u8; 1024];
    tokio::time::timeout(BOOTSTRAP_IO_TIMEOUT, stream.read(&mut buf))
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "bootstrap read timed out")
        })??;

    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        id.len(),
        id
    );

    tokio::time::timeout(BOOTSTRAP_IO_TIMEOUT, stream.write_all(response.as_bytes()))
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "bootstrap write timed out")
        })??;

    Ok(())
}

/// Run the bootstrap-id accept loop on `listener`. Each accepted
/// connection is gated by `semaphore`; if no permit is available the
/// connection is dropped immediately and a warning is logged.
///
/// This loop runs forever — callers should `tokio::spawn` it.
pub async fn run_bootstrap_listener(
    listener: tokio::net::TcpListener,
    id: Arc<String>,
    semaphore: Arc<Semaphore>,
) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                warn!(%e, "bootstrap accept failed");
                continue;
            }
        };

        let permit = match Arc::clone(&semaphore).try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                warn!(
                    %peer,
                    "bootstrap connection cap reached; dropping connection"
                );
                drop(stream);
                continue;
            }
        };

        let id = Arc::clone(&id);
        tokio::spawn(async move {
            if let Err(e) = handle_bootstrap_connection(stream, id.as_str()).await {
                tracing::debug!(%e, %peer, "bootstrap connection error");
            }
            // Hold the permit for the lifetime of the per-connection task.
            drop(permit);
        });
    }
}

/// Dispatch a single accepted connection to either the bootstrap-id
/// handler or the upstream iroh-relay, based on the HTTP request line.
///
/// Peeks at the incoming bytes until a `\r\n` delimits the request
/// line. If the path is exactly [`BOOTSTRAP_ID_PATH`] the connection is
/// answered locally; otherwise a TCP connection is opened to
/// `upstream_addr`, the already-read bytes are replayed, and the two
/// streams are proxied bidirectionally (which transparently handles
/// WebSocket upgrades used by the iroh-relay protocol).
pub async fn dispatch_connection(
    mut client: TcpStream,
    upstream_addr: SocketAddr,
    bootstrap_id: Arc<String>,
) -> std::io::Result<()> {
    // Disable Nagle — the iroh-relay expects the upstream path to be
    // responsive, and responses from the bootstrap handler are small
    // single-shot writes.
    let _ = client.set_nodelay(true);

    // Read until we have a full HTTP request line (terminated by CRLF)
    // or hit the buffer cap. We do NOT attempt to parse the full
    // headers — the upstream server does that.
    let mut buffered = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let request_line_end = loop {
        if buffered.len() >= PROXY_REQUEST_LINE_BUFFER {
            // Request line too long — give up and proxy to upstream.
            break None;
        }
        let read_fut = client.read(&mut chunk);
        let n = match tokio::time::timeout(BOOTSTRAP_IO_TIMEOUT, read_fut).await {
            Ok(Ok(0)) => break None, // EOF before a request line.
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "timed out reading request line",
                ));
            }
        };
        buffered.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buffered.windows(2).position(|w| w == b"\r\n") {
            break Some(pos);
        }
    };

    // If we have a request line, try to match the bootstrap-id path.
    if let Some(end) = request_line_end {
        let line = &buffered[..end];
        if request_line_matches_bootstrap_id(line) {
            return handle_bootstrap_request_after_line(client, &buffered, &bootstrap_id).await;
        }
    }

    // Fall through: forward everything we've buffered so far plus the
    // remainder of the stream to the upstream iroh-relay.
    let mut upstream = TcpStream::connect(upstream_addr).await?;
    let _ = upstream.set_nodelay(true);
    if !buffered.is_empty() {
        upstream.write_all(&buffered).await?;
    }
    // Bidirectional copy exits when either side hits EOF or errors.
    // We deliberately drop any error — it's the normal way for HTTP/1.1
    // responses with Connection: close to terminate.
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    Ok(())
}

/// Returns `true` iff the HTTP request line (without trailing CRLF)
/// targets [`BOOTSTRAP_ID_PATH`] with method `GET`. We match the path
/// verbatim (no query string support) because the endpoint does not
/// use any request parameters.
fn request_line_matches_bootstrap_id(line: &[u8]) -> bool {
    // Expected shape: "GET /bootstrap-id HTTP/1.x"
    let mut parts = line.splitn(3, |b| *b == b' ');
    let Some(method) = parts.next() else {
        return false;
    };
    let Some(path) = parts.next() else {
        return false;
    };
    method == b"GET" && path == BOOTSTRAP_ID_PATH.as_bytes()
}

/// Complete a bootstrap-id request once the proxy has identified the
/// path. Drains any remaining bytes of the request (up to the
/// end-of-headers marker) so the client sees a well-formed response
/// before the connection is closed.
async fn handle_bootstrap_request_after_line(
    mut client: TcpStream,
    already_read: &[u8],
    id: &str,
) -> std::io::Result<()> {
    // Drain the rest of the request (best effort). If the
    // end-of-headers marker "\r\n\r\n" is already in what we read, we
    // don't need to read more.
    if !already_read.windows(4).any(|w| w == b"\r\n\r\n") {
        // Accumulate every chunk we read into a single buffer and
        // search the *whole* buffer for "\r\n\r\n" each iteration.
        // Searching only the latest chunk would miss a marker that
        // straddles a chunk boundary, leaving the loop spinning until
        // BOOTSTRAP_IO_TIMEOUT (issue #238).
        //
        // Seed the accumulator with the trailing 3 bytes of
        // `already_read` so a marker straddling the boundary between
        // the request line and the first newly-read chunk is also
        // caught. (3 bytes is the most that can contribute to a
        // 4-byte marker without itself containing the full marker —
        // if `already_read` already held it, we wouldn't be here.)
        let mut buffered: Vec<u8> = Vec::with_capacity(1024);
        let seed_start = already_read.len().saturating_sub(3);
        buffered.extend_from_slice(&already_read[seed_start..]);

        let mut chunk = [0u8; 1024];
        let deadline = tokio::time::sleep(BOOTSTRAP_IO_TIMEOUT);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => break,
                res = client.read(&mut chunk) => match res {
                    Ok(0) => break,
                    Ok(n) => {
                        buffered.extend_from_slice(&chunk[..n]);
                        if buffered.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                        if buffered.len() >= BOOTSTRAP_DRAIN_BUFFER_CAP {
                            warn!(
                                cap = BOOTSTRAP_DRAIN_BUFFER_CAP,
                                "bootstrap request headers exceeded drain cap; closing"
                            );
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "bootstrap request headers exceeded drain cap",
                            ));
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
        }
    }

    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        id.len(),
        id
    );

    tokio::time::timeout(BOOTSTRAP_IO_TIMEOUT, client.write_all(response.as_bytes()))
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "bootstrap write timed out")
        })??;
    Ok(())
}

/// Run the public HTTP accept loop that fronts the iroh-relay. Each
/// accepted connection is dispatched by [`dispatch_connection`]:
/// requests for [`BOOTSTRAP_ID_PATH`] are answered locally, everything
/// else is proxied to `upstream_addr`.
///
/// This loop runs forever — callers should `tokio::spawn` it.
pub async fn run_proxy_listener(
    listener: tokio::net::TcpListener,
    upstream_addr: SocketAddr,
    id: Arc<String>,
    semaphore: Arc<Semaphore>,
) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                warn!(%e, "relay proxy accept failed");
                continue;
            }
        };

        let permit = match Arc::clone(&semaphore).try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                warn!(
                    %peer,
                    "relay proxy connection cap reached; dropping connection"
                );
                drop(stream);
                continue;
            }
        };

        let id = Arc::clone(&id);
        tokio::spawn(async move {
            if let Err(e) = dispatch_connection(stream, upstream_addr, id).await {
                tracing::debug!(%e, %peer, "relay proxy connection error");
            }
            drop(permit);
        });
    }
}

/// Listen for `TopicAnnounce` messages on the server-ops topic and
/// dynamically subscribe to announced channel topics. Topics are
/// validated against [`topic_str_is_valid`] and the number of distinct
/// subscribed topics is capped at [`MAX_TOPICS`].
///
/// This is a **transport-layer** routine. The relay simply joins the
/// announced gossip topics so clients on those topics can reach each
/// other through it; it performs no governance check on who is
/// announcing, no signature verification on the underlying events, and
/// no authority check against the server's DAG state. Clients are
/// responsible for validating any messages that arrive on those
/// topics. See the crate-level docs for the full trust model.
pub async fn topic_announce_listener<N>(mut events: N::Events, network: N)
where
    N: Network,
{
    let mut subscribed: HashSet<String> = HashSet::new();
    let mut warned_full = false;

    while let Some(Ok(event)) = events.next().await {
        let GossipEvent::Received(msg) = event else {
            continue;
        };
        let Some((willow_common::WireMessage::TopicAnnounce { topics }, _)) =
            willow_common::unpack_wire(&msg.content)
        else {
            continue;
        };
        for topic_str in topics {
            if !topic_str_is_valid(&topic_str) {
                warn!(
                    topic = %topic_str,
                    "rejecting invalid topic string from announce"
                );
                continue;
            }
            if subscribed.contains(&topic_str) {
                continue;
            }
            if subscribed.len() >= MAX_TOPICS {
                if !warned_full {
                    warn!(
                        cap = MAX_TOPICS,
                        "topic subscription cap reached; dropping further announces"
                    );
                    warned_full = true;
                }
                continue;
            }
            subscribed.insert(topic_str.clone());
            let topic_id = willow_network::topic_id(&topic_str);
            match network.subscribe(topic_id, vec![]).await {
                Ok(_) => {
                    info!(topic = %topic_str, "subscribed to announced channel topic");
                }
                Err(e) => {
                    warn!(
                        topic = %topic_str, %e,
                        "failed to subscribe to announced topic"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── topic_announce_listener tests ────────────────────────────────────────

    /// Helper: pack a TopicAnnounce wire message signed by a fresh identity.
    fn pack_topic_announce(
        topics: Vec<String>,
        identity: &willow_identity::Identity,
    ) -> bytes::Bytes {
        let msg = willow_common::WireMessage::TopicAnnounce { topics };
        let packed = willow_common::pack_wire(&msg, identity).expect("pack_wire failed");
        bytes::Bytes::from(packed)
    }

    /// Helper: send a TopicAnnounce on `ops_topic` from `announcer` and yield
    /// briefly so the listener task has a chance to process it.
    async fn send_announce_and_wait(
        handle: &willow_network::mem::MemTopicHandle,
        topics: Vec<String>,
        identity: &willow_identity::Identity,
    ) {
        use willow_network::traits::TopicHandle;
        let data = pack_topic_announce(topics, identity);
        handle.broadcast(data).await.expect("broadcast failed");
        // Give the listener task a few yields to process the message.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    /// topic_announce_listener subscribes the relay network to a valid
    /// announced topic. We verify by having an observer subscribe to the
    /// announced topic first; when the relay subscribes, the observer sees a
    /// NeighborUp event for the relay's ID.
    #[tokio::test]
    async fn topic_announce_listener_subscribes_to_announced_topic() {
        use willow_network::mem::{MemHub, MemNetwork};
        use willow_network::traits::{GossipEvent, Network, TopicEvents};

        let hub = MemHub::new();
        let announcer_net = MemNetwork::new(&hub);
        let relay_net = MemNetwork::new(&hub);
        let observer_net = MemNetwork::new(&hub);
        let announcer_identity = announcer_net.identity().clone();

        // Shared server-ops topic for announce messages.
        let ops_topic = willow_network::topic_id("_willow_server_ops");

        // The topic that will be announced.
        let announced = "server-abc/channel-general".to_string();
        let announced_topic = willow_network::topic_id(&announced);

        // Observer subscribes to the announced topic BEFORE the relay does.
        // When the relay later subscribes, the hub will fire NeighborUp to
        // the observer, confirming the relay joined.
        let (_, mut observer_events) = observer_net
            .subscribe(announced_topic, vec![])
            .await
            .unwrap();

        // Relay subscribes to ops_topic so it can receive announces.
        let (_, relay_events) = relay_net.subscribe(ops_topic, vec![]).await.unwrap();
        let relay_id = relay_net.id();

        // Announcer subscribes to ops_topic so it can broadcast.
        let (ops_handle, _) = announcer_net.subscribe(ops_topic, vec![]).await.unwrap();

        // Spawn the listener — it takes ownership of relay_events + relay_net.
        let listener = tokio::spawn(topic_announce_listener::<MemNetwork>(
            relay_events,
            relay_net,
        ));

        // Send a valid TopicAnnounce from the announcer.
        send_announce_and_wait(&ops_handle, vec![announced.clone()], &announcer_identity).await;

        // The observer should see NeighborUp for the relay, confirming it subscribed.
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match observer_events.next().await {
                    Some(Ok(GossipEvent::NeighborUp(id))) => return id,
                    Some(_) => continue,
                    None => panic!("observer event stream closed unexpectedly"),
                }
            }
        })
        .await
        .expect("timed out waiting for relay to subscribe to announced topic");

        assert_eq!(
            event, relay_id,
            "expected NeighborUp from relay, got different peer"
        );

        listener.abort();
    }

    /// topic_announce_listener must reject invalid topic strings (empty,
    /// disallowed characters) without subscribing to them.
    ///
    /// We confirm rejection by also announcing ONE valid topic in the same
    /// message; once the relay processes the message we know the listener ran.
    /// We then verify that only the valid topic received a subscription and
    /// none of the invalid ones did (checked via observer NeighborUp for the
    /// valid topic, and absence of NeighborUp for invalid topic IDs).
    #[tokio::test]
    async fn topic_announce_listener_rejects_invalid_topic() {
        use willow_network::mem::{MemHub, MemNetwork};
        use willow_network::traits::{GossipEvent, Network, TopicEvents};

        let hub = MemHub::new();
        let announcer_net = MemNetwork::new(&hub);
        let relay_net = MemNetwork::new(&hub);
        let observer_net = MemNetwork::new(&hub);
        let announcer_identity = announcer_net.identity().clone();

        let ops_topic = willow_network::topic_id("_willow_server_ops");

        // One valid sentinel topic so we know when the listener has processed
        // the message (we get NeighborUp on it).
        let valid_sentinel = "valid-sentinel".to_string();
        let sentinel_topic_id = willow_network::topic_id(&valid_sentinel);

        // Observer waits on the sentinel topic for NeighborUp from relay.
        let (_, mut observer_events) = observer_net
            .subscribe(sentinel_topic_id, vec![])
            .await
            .unwrap();

        let (_, relay_events) = relay_net.subscribe(ops_topic, vec![]).await.unwrap();
        let relay_id = relay_net.id();
        let (ops_handle, _) = announcer_net.subscribe(ops_topic, vec![]).await.unwrap();

        let listener = tokio::spawn(topic_announce_listener::<MemNetwork>(
            relay_events,
            relay_net,
        ));

        // Send invalid topics along with the valid sentinel in one announce.
        let mixed_topics = vec![
            "".to_string(),               // empty — invalid
            "bad topic!".to_string(),     // space + ! — invalid
            "has space here".to_string(), // spaces — invalid
            valid_sentinel.clone(),       // valid — sentinel
        ];
        send_announce_and_wait(&ops_handle, mixed_topics, &announcer_identity).await;

        // Wait for the sentinel NeighborUp — this means the listener processed
        // the message and decided on each topic.
        let sentinel_neighbor = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match observer_events.next().await {
                    Some(Ok(GossipEvent::NeighborUp(id))) => return id,
                    Some(_) => continue,
                    None => panic!("observer stream closed"),
                }
            }
        })
        .await
        .expect("timed out waiting for sentinel subscription");
        assert_eq!(
            sentinel_neighbor, relay_id,
            "sentinel NeighborUp should be from relay"
        );

        // The invalid topics must NOT have produced subscriptions.
        // We verify by subscribing an observer to the topic IDs that would have
        // been used and checking that no NeighborUp arrives immediately.
        let invalid_non_empty = ["bad topic!", "has space here"];
        for bad in &invalid_non_empty {
            // Subscribe a fresh observer to the (invalid) topic ID.
            let bad_tid = willow_network::topic_id(bad);
            let (_, mut obs) = observer_net.subscribe(bad_tid, vec![]).await.unwrap();
            // If the relay had subscribed, we'd get NeighborUp from it immediately.
            let result = tokio::time::timeout(std::time::Duration::from_millis(50), async {
                loop {
                    match obs.next().await {
                        Some(Ok(GossipEvent::NeighborUp(id))) if id == relay_id => return true,
                        Some(_) => continue,
                        None => return false,
                    }
                }
            })
            .await;
            assert!(
                result.is_err() || result == Ok(false),
                "relay must NOT subscribe to invalid topic {bad:?}"
            );
        }

        listener.abort();
    }

    /// topic_announce_listener enforces MAX_TOPICS: after reaching the cap it
    /// stops subscribing to additional unique topics.
    ///
    /// We send MAX_TOPICS + 1 distinct valid topics in a single announce.
    /// We confirm:
    /// - The first topic ("t0") IS subscribed — observer sees NeighborUp.
    /// - The last topic ("t{MAX_TOPICS}") is NOT subscribed — no NeighborUp.
    ///
    /// Since the listener processes all topics in one synchronous for loop
    /// within a single async task poll, seeing NeighborUp for the first topic
    /// guarantees the entire for loop has already run before we check the
    /// overflow topic.
    #[tokio::test]
    async fn topic_announce_listener_enforces_max_topics_cap() {
        use willow_network::mem::{MemHub, MemNetwork};
        use willow_network::traits::{GossipEvent, Network, TopicEvents};

        let hub = MemHub::new();
        let announcer_net = MemNetwork::new(&hub);
        let relay_net = MemNetwork::new(&hub);
        let first_observer = MemNetwork::new(&hub);
        let overflow_observer = MemNetwork::new(&hub);
        let announcer_identity = announcer_net.identity().clone();

        let ops_topic = willow_network::topic_id("_willow_server_ops");

        // Build MAX_TOPICS + 1 distinct, valid topic strings.
        let topics: Vec<String> = (0..=(MAX_TOPICS as u64)).map(|i| format!("t{i}")).collect();

        let first_topic_id = willow_network::topic_id(topics.first().unwrap());
        let overflow_topic_id = willow_network::topic_id(topics.last().unwrap());

        // Subscribe observers BEFORE the relay listener runs so NeighborUp
        // fires toward the observer when the relay joins each topic.
        let (_, mut first_events) = first_observer
            .subscribe(first_topic_id, vec![])
            .await
            .unwrap();
        let (_, mut overflow_events) = overflow_observer
            .subscribe(overflow_topic_id, vec![])
            .await
            .unwrap();

        let (_, relay_events) = relay_net.subscribe(ops_topic, vec![]).await.unwrap();
        let relay_id = relay_net.id();
        let (ops_handle, _) = announcer_net.subscribe(ops_topic, vec![]).await.unwrap();

        let listener = tokio::spawn(topic_announce_listener::<MemNetwork>(
            relay_events,
            relay_net,
        ));

        // Broadcast all MAX_TOPICS + 1 topics in a single announce.
        let data = pack_topic_announce(topics, &announcer_identity);
        use willow_network::traits::TopicHandle;
        ops_handle.broadcast(data).await.expect("broadcast failed");

        // Wait for the first-topic observer to see NeighborUp — proves the
        // listener ran and subscriptions began.
        let first_neighbor = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match first_events.next().await {
                    Some(Ok(GossipEvent::NeighborUp(id))) => return id,
                    Some(_) => continue,
                    None => panic!("first_observer stream closed unexpectedly"),
                }
            }
        })
        .await
        .expect("timed out waiting for relay to subscribe to the first topic");
        assert_eq!(
            first_neighbor, relay_id,
            "NeighborUp on first topic should be from relay"
        );

        // Give the listener a moment to finish the remainder of the for loop
        // and all the async subscribe() calls inside it.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // The overflow topic (index MAX_TOPICS) must NOT be subscribed — no NeighborUp.
        let overflow_result = tokio::time::timeout(std::time::Duration::from_millis(100), async {
            loop {
                match overflow_events.next().await {
                    Some(Ok(GossipEvent::NeighborUp(id))) if id == relay_id => return true,
                    Some(_) => continue,
                    None => return false,
                }
            }
        })
        .await;

        assert!(
            overflow_result.is_err() || overflow_result == Ok(false),
            "relay must NOT subscribe to the overflow topic (cap at MAX_TOPICS={MAX_TOPICS})"
        );

        drop(hub);
        listener.abort();
    }

    // ── topic_str_is_valid unit tests ────────────────────────────────────────

    #[test]
    fn topic_str_is_valid_accepts_basic_ascii() {
        assert!(topic_str_is_valid("general"));
        assert!(topic_str_is_valid("server/123/channel/abc"));
        assert!(topic_str_is_valid("_willow_server_ops"));
        assert!(topic_str_is_valid("a.b-c_d:e/f"));
        assert!(topic_str_is_valid("0123456789"));
        assert!(topic_str_is_valid("AZaz09_/:.-"));
    }

    #[test]
    fn topic_str_is_valid_rejects_empty() {
        assert!(!topic_str_is_valid(""));
    }

    #[test]
    fn topic_str_is_valid_rejects_too_long() {
        let long = "a".repeat(MAX_TOPIC_LEN + 1);
        assert!(!topic_str_is_valid(&long));
        // Boundary: exactly MAX_TOPIC_LEN is fine.
        let max = "a".repeat(MAX_TOPIC_LEN);
        assert!(topic_str_is_valid(&max));
    }

    #[test]
    fn topic_str_is_valid_rejects_disallowed_chars() {
        assert!(!topic_str_is_valid("hello world")); // space
        assert!(!topic_str_is_valid("hello\nworld")); // control
        assert!(!topic_str_is_valid("hello!"));
        assert!(!topic_str_is_valid("hello#world"));
        assert!(!topic_str_is_valid("hello@world"));
        assert!(!topic_str_is_valid("héllo")); // non-ASCII
        assert!(!topic_str_is_valid("hello\0"));
    }
}
