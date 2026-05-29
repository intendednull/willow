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

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tracing::{info, warn};
use willow_identity::EndpointId;
use willow_network::traits::{GossipEvent, TopicEvents};
use willow_network::Network;

/// Maximum concurrent client connections accepted by the public relay
/// proxy listener. Every connection accepted by [`run_proxy_listener`]
/// — `/bootstrap-id`, the `/.well-known/willow` capability endpoint, and
/// the iroh-relay proxy fallthrough alike — is gated by this cap. Excess
/// accepts are dropped immediately to prevent FD/memory exhaustion under
/// a connection-flood DoS.
///
/// (Formerly `MAX_CONCURRENT_BOOTSTRAP_CONNECTIONS`; renamed because it
/// gates the whole proxy listener, not just bootstrap-id traffic.)
pub const MAX_CONCURRENT_PROXY_CONNECTIONS: usize = 1024;

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

/// HTTP path of the signed relay capability document
/// (`GET /.well-known/willow`), an RFC 8615 well-known URI. Clients fetch
/// it before connecting to discover protocol versions, limits, and
/// operator metadata. See the
/// [capability-doc spec](../../../docs/specs/2026-04-24-relay-capability-doc.md).
pub const CAPABILITY_PATH: &str = "/.well-known/willow";

/// `Content-Type` for the capability document. The `+json` structured
/// suffix opts into generic JSON tooling while the distinct media type
/// disambiguates it from a plain `application/json` body.
pub const CAPABILITY_CONTENT_TYPE: &str = "application/willow+json; charset=utf-8";

/// `Cache-Control` for a steady-state (`status: "ok"`) capability
/// document. Dynamic `degraded`/`read_only` status (which would warrant a
/// shorter, must-revalidate TTL) is deferred, so this is effectively the
/// only tier until dynamic status lands.
const CAPABILITY_CACHE_CONTROL: &str = "public, max-age=300";

/// HTTP methods accepted on [`CAPABILITY_PATH`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityMethod {
    /// `GET` — return the signed document (or `304` on a matching ETag).
    Get,
    /// `OPTIONS` — CORS preflight, answered with `204` + CORS headers.
    Options,
}

/// Maximum number of distinct channel topics the topic-announce
/// listener will subscribe to. Once this cap is reached the listener
/// drops further unique announces (with rate-limited warns; see
/// [`WARN_RATE_LIMIT`]).
pub const MAX_TOPICS: usize = 10_000;

/// Maximum length, in bytes, of a topic string accepted from a
/// `TopicAnnounce` message. Anything longer is rejected outright.
pub const MAX_TOPIC_LEN: usize = 256;

/// Maximum number of topic entries accepted in a single `TopicAnnounce`
/// wire message. Larger announces are rejected outright before any
/// per-topic work runs (validation + blake3 hash). Without this cap a
/// peer can ship a 256 KB envelope containing ~128 000 two-byte topics
/// and force the relay to do O(n) work per message — a CPU-amplification
/// vector. 64 distinct channels per announce is comfortably more than
/// any legitimate client subscribes to in a single batch.
pub const MAX_TOPICS_PER_ANNOUNCE: usize = 64;

/// Maximum number of distinct topic subscriptions the relay will hold
/// on behalf of a single signer (`EndpointId`). Once a signer reaches
/// this cap, announcing a new topic evicts that signer's least-recently
/// used topic (LRU). This prevents one peer from monopolising the
/// global slot table ([`MAX_TOPICS`]) and starving other clients.
pub const MAX_TOPICS_PER_SIGNER: usize = 100;

/// Minimum gap between repeated cap-hit warns. Replaces the previous
/// once-per-session flag so operators see ongoing pressure without
/// having the log spammed by every announce that overflows the cap.
pub const WARN_RATE_LIMIT: Duration = Duration::from_secs(60);

/// Returns `true` and updates `last_warn` to `now` iff a warn should be
/// emitted now: either no warn has been emitted yet, or the previous
/// warn is older than `interval`. Pulled out so tests can exercise the
/// rate-limiter directly without poking at the tracing subscriber.
pub fn should_emit_warn(last_warn: &mut Option<Instant>, now: Instant, interval: Duration) -> bool {
    match *last_warn {
        Some(prev) if now.duration_since(prev) < interval => false,
        _ => {
            *last_warn = Some(now);
            true
        }
    }
}

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
    capability_json: Arc<str>,
    capability_etag: Arc<str>,
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

    // If we have a request line, try to match a locally-served path.
    // Capability-doc routes are checked before bootstrap-id; everything
    // unmatched falls through to the upstream iroh-relay.
    if let Some(end) = request_line_end {
        let line = &buffered[..end];
        if let Some(method) = request_line_matches_capability_doc(line) {
            return match method {
                CapabilityMethod::Get => {
                    handle_capability_request_after_line(
                        client,
                        &buffered,
                        &capability_json,
                        &capability_etag,
                    )
                    .await
                }
                CapabilityMethod::Options => {
                    handle_capability_options(client, &buffered).await
                }
            };
        }
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

/// Returns the [`CapabilityMethod`] iff the HTTP request line (without
/// trailing CRLF) targets [`CAPABILITY_PATH`] with `GET` or `OPTIONS`.
/// Any other method (`POST`, `PUT`, …) on the path returns `None` so the
/// request falls through to the upstream proxy, matching the
/// bootstrap-id endpoint's GET-only behaviour. The path is matched
/// verbatim (no query string) because the endpoint takes no parameters.
fn request_line_matches_capability_doc(line: &[u8]) -> Option<CapabilityMethod> {
    // Expected shape: "GET /.well-known/willow HTTP/1.x" (or OPTIONS).
    let mut parts = line.splitn(3, |b| *b == b' ');
    let method = parts.next()?;
    let path = parts.next()?;
    if path != CAPABILITY_PATH.as_bytes() {
        return None;
    }
    match method {
        b"GET" => Some(CapabilityMethod::Get),
        b"OPTIONS" => Some(CapabilityMethod::Options),
        _ => None,
    }
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
    drain_request_headers(&mut client, already_read).await?;

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

/// Drain the remainder of an HTTP request's headers up to the
/// end-of-headers marker (`\r\n\r\n`), bounded by [`BOOTSTRAP_IO_TIMEOUT`]
/// and [`BOOTSTRAP_DRAIN_BUFFER_CAP`]. Shared by every locally-served
/// endpoint (bootstrap-id and the capability doc) so a well-formed
/// response is written only after the client's request is fully read.
///
/// Returns `Err(InvalidData)` if the headers exceed the drain cap before
/// the marker is seen, and `Err(_)` on an underlying read error. A
/// timeout is treated as "client sent everything it is going to" and
/// returns `Ok(())` so the handler still responds — matching the prior
/// bootstrap-id behaviour.
async fn drain_request_headers<S>(client: &mut S, already_read: &[u8]) -> std::io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // If the end-of-headers marker "\r\n\r\n" is already in what we read,
    // we don't need to read more.
    if already_read.windows(4).any(|w| w == b"\r\n\r\n") {
        return Ok(());
    }

    // Accumulate every chunk we read into a single buffer and search the
    // *whole* buffer for "\r\n\r\n" each iteration. Searching only the
    // latest chunk would miss a marker that straddles a chunk boundary,
    // leaving the loop spinning until BOOTSTRAP_IO_TIMEOUT (issue #238).
    //
    // Seed the accumulator with the trailing 3 bytes of `already_read` so
    // a marker straddling the boundary between the request line and the
    // first newly-read chunk is also caught. (3 bytes is the most that
    // can contribute to a 4-byte marker without itself containing the
    // full marker — if `already_read` already held it, we wouldn't be
    // here.)
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
                            "request headers exceeded drain cap; closing"
                        );
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "request headers exceeded drain cap",
                        ));
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

/// Parse the (optional) `If-None-Match` header value from the bytes of an
/// HTTP request, returning the entity-tag with surrounding double quotes
/// stripped. Case-insensitive on the header name per RFC 9110; only the
/// first match is honoured. Returns `None` if the header is absent.
fn parse_if_none_match(request: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(request).ok()?;
    for line in text.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("If-None-Match") {
            let v = value.trim();
            // Strip optional weak-validator prefix and surrounding quotes.
            let v = v.strip_prefix("W/").unwrap_or(v).trim();
            let v = v.trim_matches('"');
            return Some(v.to_string());
        }
    }
    None
}

/// Complete a `GET /.well-known/willow` request once the proxy has
/// identified the path. Drains the rest of the request, then writes the
/// pre-rendered (built + signed once at startup) capability document with
/// CORS headers, a strong `ETag`, and a steady-state `Cache-Control`. If
/// the client's `If-None-Match` matches the current ETag, a bodyless
/// `304 Not Modified` is returned instead.
async fn handle_capability_request_after_line(
    mut client: TcpStream,
    already_read: &[u8],
    info_json: &str,
    etag: &str,
) -> std::io::Result<()> {
    let if_none_match = parse_if_none_match(already_read);
    drain_request_headers(&mut client, already_read).await?;

    let response = if if_none_match.as_deref() == Some(etag) {
        format!(
            "HTTP/1.1 304 Not Modified\r\n\
             Access-Control-Allow-Origin: *\r\n\
             Access-Control-Allow-Methods: GET, OPTIONS\r\n\
             Access-Control-Allow-Headers: Accept, Content-Type, If-None-Match\r\n\
             ETag: \"{etag}\"\r\n\
             Cache-Control: {CAPABILITY_CACHE_CONTROL}\r\n\
             Connection: close\r\n\
             \r\n"
        )
    } else {
        format!(
            "HTTP/1.1 200 OK\r\n\
             Access-Control-Allow-Origin: *\r\n\
             Access-Control-Allow-Methods: GET, OPTIONS\r\n\
             Access-Control-Allow-Headers: Accept, Content-Type, If-None-Match\r\n\
             Content-Type: {CAPABILITY_CONTENT_TYPE}\r\n\
             ETag: \"{etag}\"\r\n\
             Cache-Control: {CAPABILITY_CACHE_CONTROL}\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {info_json}",
            info_json.len(),
        )
    };

    tokio::time::timeout(BOOTSTRAP_IO_TIMEOUT, client.write_all(response.as_bytes()))
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "capability write timed out")
        })??;
    Ok(())
}

/// Answer an `OPTIONS /.well-known/willow` CORS preflight with `204 No
/// Content` and the full ACAO/ACAM/ACAH header set. Carries no body.
async fn handle_capability_options(
    mut client: TcpStream,
    already_read: &[u8],
) -> std::io::Result<()> {
    drain_request_headers(&mut client, already_read).await?;

    let response = "HTTP/1.1 204 No Content\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: GET, OPTIONS\r\n\
         Access-Control-Allow-Headers: Accept, Content-Type, If-None-Match\r\n\
         Connection: close\r\n\
         \r\n";

    tokio::time::timeout(BOOTSTRAP_IO_TIMEOUT, client.write_all(response.as_bytes()))
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "capability write timed out")
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
    capability_json: Arc<str>,
    capability_etag: Arc<str>,
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
        let capability_json = Arc::clone(&capability_json);
        let capability_etag = Arc::clone(&capability_etag);
        tokio::spawn(async move {
            if let Err(e) =
                dispatch_connection(stream, upstream_addr, id, capability_json, capability_etag)
                    .await
            {
                tracing::debug!(%e, %peer, "relay proxy connection error");
            }
            drop(permit);
        });
    }
}

/// In-memory state for [`topic_announce_listener`].
///
/// Pulled out of the listener's stack so the per-message processing
/// step can be exercised by unit tests against a `MemNetwork` without
/// having to drive the full `events` stream. Each field's invariants
/// are documented inline.
#[derive(Default)]
struct AnnounceState {
    /// Reference count of how many distinct signers currently hold a
    /// subscription on each topic. The relay holds exactly one gossip
    /// subscription per topic with a non-zero refcount; the entry is
    /// removed (and the subscription torn down) when the count hits 0.
    /// Size is bounded by [`MAX_TOPICS`].
    topic_refcount: HashMap<String, usize>,

    /// Per-signer LRU of currently-held topics, oldest at the front.
    /// Bounded by [`MAX_TOPICS_PER_SIGNER`] entries per signer; on
    /// overflow the front entry is evicted (and its global refcount
    /// decremented) before the new topic is inserted at the back.
    /// Re-announcing an existing topic promotes it to the back.
    signer_topics: HashMap<EndpointId, VecDeque<String>>,

    /// Last time a per-message-cap warn was emitted; throttles the log
    /// at [`WARN_RATE_LIMIT`] so an attacker cannot spam the log.
    warn_per_msg_last: Option<Instant>,
    /// Last time a global-cap warn was emitted (rate-limited).
    warn_global_full_last: Option<Instant>,
    /// Last time a per-signer-eviction warn was emitted (rate-limited).
    warn_signer_evict_last: Option<Instant>,
}

/// Outcome of processing a single topic for a given signer. The
/// network-level effects (subscribe / unsubscribe) are returned so the
/// caller drives them outside `&mut self`. A single announce can yield
/// up to two actions: an LRU eviction may free a global slot
/// (Unsubscribe) and the new topic may need joining (Subscribe).
#[derive(Debug, Default, PartialEq, Eq)]
struct TopicActions {
    /// Topic to unsubscribe from (LRU eviction freed the last reference).
    pub unsubscribe: Option<String>,
    /// Topic to subscribe to (first reference globally).
    pub subscribe: Option<String>,
    /// True iff this topic was rejected by the global cap.
    pub rejected_global: bool,
    /// True iff this topic triggered a per-signer LRU eviction.
    pub evicted_for_signer: bool,
}

impl AnnounceState {
    /// Apply one announced topic from `signer`. Returns network-level
    /// effects the caller must drive. Pure with respect to the network.
    fn process_topic(&mut self, signer: EndpointId, topic_str: &str) -> TopicActions {
        let mut actions = TopicActions::default();

        let entry = self.signer_topics.entry(signer).or_default();

        // If signer already holds this topic, promote it (LRU touch).
        if let Some(pos) = entry.iter().position(|t| t == topic_str) {
            if let Some(t) = entry.remove(pos) {
                entry.push_back(t);
            }
            return actions;
        }

        // Per-signer cap: evict signer's LRU topic to make room.
        if entry.len() >= MAX_TOPICS_PER_SIGNER {
            actions.evicted_for_signer = true;
            if let Some(oldest) = entry.pop_front() {
                if let Some(count) = self.topic_refcount.get_mut(&oldest) {
                    *count -= 1;
                    if *count == 0 {
                        self.topic_refcount.remove(&oldest);
                        actions.unsubscribe = Some(oldest);
                    }
                }
            }
        }

        // Global cap: reject if topic is new globally and table is full.
        let already_global = self.topic_refcount.contains_key(topic_str);
        if !already_global && self.topic_refcount.len() >= MAX_TOPICS {
            actions.rejected_global = true;
            return actions;
        }

        // Accept: append to signer queue and bump global refcount.
        let entry = self.signer_topics.entry(signer).or_default();
        entry.push_back(topic_str.to_string());
        let count = self
            .topic_refcount
            .entry(topic_str.to_string())
            .or_insert(0);
        *count += 1;
        if *count == 1 {
            actions.subscribe = Some(topic_str.to_string());
        }
        actions
    }
}

/// Listen for `TopicAnnounce` messages on the server-ops topic and
/// dynamically subscribe to announced channel topics. Topics are
/// validated against [`topic_str_is_valid`] and the number of distinct
/// subscribed topics is capped at [`MAX_TOPICS`].
///
/// Three layered caps protect the relay from CPU amplification and
/// slot-table exhaustion:
///
/// 1. **[`MAX_TOPICS_PER_ANNOUNCE`]** — per-message ceiling. Announces
///    carrying more entries than this are rejected outright before any
///    per-topic validation or hashing runs. Without this cap an
///    attacker can pack ~128 000 two-byte topics into a 256 KB
///    envelope and force the relay to do O(n) work per message.
/// 2. **[`MAX_TOPICS_PER_SIGNER`]** — per-signer slot cap with LRU
///    eviction. Stops one peer from monopolising the global slot
///    table by pumping ~10 000 unique topics into it.
/// 3. **[`MAX_TOPICS`]** — global slot cap. Once reached, additional
///    new topics are dropped and the warn is rate-limited at
///    [`WARN_RATE_LIMIT`].
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
    let mut state = AnnounceState::default();

    while let Some(Ok(event)) = events.next().await {
        let GossipEvent::Received(msg) = event else {
            continue;
        };
        let Some((willow_common::WireMessage::TopicAnnounce { topics }, signer)) =
            willow_common::unpack_wire(&msg.content)
        else {
            continue;
        };

        // Per-message cap: drop the whole announce if it exceeds the
        // per-message limit. Done BEFORE any per-topic loop so an
        // oversized announce costs O(1) work, not O(n).
        if topics.len() > MAX_TOPICS_PER_ANNOUNCE {
            if should_emit_warn(
                &mut state.warn_per_msg_last,
                Instant::now(),
                WARN_RATE_LIMIT,
            ) {
                warn!(
                    cap = MAX_TOPICS_PER_ANNOUNCE,
                    count = topics.len(),
                    %signer,
                    "rejecting TopicAnnounce exceeding per-message cap"
                );
            }
            continue;
        }

        for topic_str in topics {
            if !topic_str_is_valid(&topic_str) {
                warn!(
                    topic = %topic_str,
                    "rejecting invalid topic string from announce"
                );
                continue;
            }

            let actions = state.process_topic(signer, &topic_str);

            if actions.evicted_for_signer
                && should_emit_warn(
                    &mut state.warn_signer_evict_last,
                    Instant::now(),
                    WARN_RATE_LIMIT,
                )
            {
                warn!(
                    cap = MAX_TOPICS_PER_SIGNER,
                    %signer,
                    "per-signer topic cap reached; evicting LRU topic"
                );
            }
            if actions.rejected_global
                && should_emit_warn(
                    &mut state.warn_global_full_last,
                    Instant::now(),
                    WARN_RATE_LIMIT,
                )
            {
                warn!(
                    cap = MAX_TOPICS,
                    "topic subscription cap reached; dropping further announces"
                );
            }

            // Drive the eviction unsubscribe FIRST so the global slot
            // is freed before any subsequent subscribe attempt.
            if let Some(topic) = actions.unsubscribe {
                let topic_id = willow_network::topic_id(&topic);
                match network.unsubscribe(topic_id).await {
                    Ok(_) => {
                        info!(topic = %topic, "unsubscribed evicted topic");
                    }
                    Err(e) => {
                        warn!(
                            topic = %topic, %e,
                            "failed to unsubscribe evicted topic"
                        );
                    }
                }
            }
            if let Some(topic) = actions.subscribe {
                let topic_id = willow_network::topic_id(&topic);
                match network.subscribe(topic_id, vec![]).await {
                    Ok(_) => {
                        info!(topic = %topic, "subscribed to announced channel topic");
                    }
                    Err(e) => {
                        warn!(
                            topic = %topic, %e,
                            "failed to subscribe to announced topic"
                        );
                    }
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

    /// topic_announce_listener enforces MAX_TOPICS_PER_ANNOUNCE: an announce
    /// carrying more entries than the per-message cap is rejected outright.
    ///
    /// We send MAX_TOPICS_PER_ANNOUNCE + 1 valid topics in one announce, then
    /// — to confirm the listener is still alive and processing — send a
    /// follow-up announce with a smaller, valid sentinel batch from the same
    /// signer. The sentinel must succeed (NeighborUp on its topic) while the
    /// oversized topics must NOT (no NeighborUp).
    #[tokio::test]
    async fn topic_announce_listener_rejects_oversized_announce() {
        use willow_network::mem::{MemHub, MemNetwork};
        use willow_network::traits::{GossipEvent, Network, TopicEvents};

        let hub = MemHub::new();
        let announcer_net = MemNetwork::new(&hub);
        let relay_net = MemNetwork::new(&hub);
        let oversized_observer = MemNetwork::new(&hub);
        let sentinel_observer = MemNetwork::new(&hub);
        let announcer_identity = announcer_net.identity().clone();

        let ops_topic = willow_network::topic_id("_willow_server_ops");

        // Oversized batch: MAX_TOPICS_PER_ANNOUNCE + 1 valid topics.
        let oversized: Vec<String> = (0..=(MAX_TOPICS_PER_ANNOUNCE as u64))
            .map(|i| format!("over-{i}"))
            .collect();
        let oversized_first_id = willow_network::topic_id(&oversized[0]);

        // Sentinel: a small valid announce that must still be processed
        // (proves the listener didn't crash on the oversized announce).
        let sentinel = "sentinel-after-overflow".to_string();
        let sentinel_id = willow_network::topic_id(&sentinel);

        let (_, mut oversized_events) = oversized_observer
            .subscribe(oversized_first_id, vec![])
            .await
            .unwrap();
        let (_, mut sentinel_events) = sentinel_observer
            .subscribe(sentinel_id, vec![])
            .await
            .unwrap();

        let (_, relay_events) = relay_net.subscribe(ops_topic, vec![]).await.unwrap();
        let relay_id = relay_net.id();
        let (ops_handle, _) = announcer_net.subscribe(ops_topic, vec![]).await.unwrap();

        let listener = tokio::spawn(topic_announce_listener::<MemNetwork>(
            relay_events,
            relay_net,
        ));

        use willow_network::traits::TopicHandle;
        // Send the oversized announce — must be rejected.
        let data = pack_topic_announce(oversized, &announcer_identity);
        ops_handle.broadcast(data).await.expect("broadcast failed");
        // Then send the small sentinel announce.
        send_announce_and_wait(&ops_handle, vec![sentinel.clone()], &announcer_identity).await;

        // Sentinel must arrive — proves listener is still alive and
        // processing announces past the rejected one.
        let sentinel_neighbor = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match sentinel_events.next().await {
                    Some(Ok(GossipEvent::NeighborUp(id))) if id == relay_id => return id,
                    Some(_) => continue,
                    None => panic!("sentinel observer stream closed"),
                }
            }
        })
        .await
        .expect("timed out waiting for sentinel subscription");
        assert_eq!(sentinel_neighbor, relay_id);

        // The oversized announce's first topic must NOT have produced a
        // subscription — checking after the sentinel arrives gives the
        // listener ample time to have processed both announces.
        let oversized_result = tokio::time::timeout(std::time::Duration::from_millis(100), async {
            loop {
                match oversized_events.next().await {
                    Some(Ok(GossipEvent::NeighborUp(id))) if id == relay_id => return true,
                    Some(_) => continue,
                    None => return false,
                }
            }
        })
        .await;
        assert!(
            oversized_result.is_err() || oversized_result == Ok(false),
            "oversized announce must be rejected; first topic must NOT be subscribed"
        );

        drop(hub);
        listener.abort();
    }

    // ── AnnounceState unit tests ────────────────────────────────────────────
    //
    // These exercise the per-signer LRU and global slot accounting directly,
    // without driving the full listener task. The state machine is the load-
    // bearing piece — running it through MemNetwork at MAX_TOPICS scale
    // (10 000 entries) is wasteful when the same invariants hold per call.

    fn fresh_signer() -> EndpointId {
        willow_identity::Identity::generate().endpoint_id()
    }

    #[test]
    fn announce_state_per_signer_lru_evicts_oldest() {
        // Add MAX_TOPICS_PER_SIGNER topics for one signer, then add one more.
        // The oldest must be evicted; the newest must be present.
        let mut state = AnnounceState::default();
        let signer = fresh_signer();
        for i in 0..MAX_TOPICS_PER_SIGNER as u64 {
            let actions = state.process_topic(signer, &format!("t{i}"));
            assert_eq!(
                actions.subscribe.as_deref(),
                Some(format!("t{i}").as_str()),
                "first reference should produce Subscribe"
            );
            assert!(actions.unsubscribe.is_none());
            assert!(!actions.evicted_for_signer);
            assert!(!actions.rejected_global);
        }
        assert_eq!(
            state.signer_topics.get(&signer).map(|q| q.len()),
            Some(MAX_TOPICS_PER_SIGNER)
        );

        // The 101st: must evict t0 (oldest) and subscribe to the new one.
        let actions = state.process_topic(signer, "t-new");
        assert!(actions.evicted_for_signer);
        assert_eq!(actions.unsubscribe.as_deref(), Some("t0"));
        assert_eq!(actions.subscribe.as_deref(), Some("t-new"));
        assert!(!actions.rejected_global);

        // Signer still holds exactly MAX_TOPICS_PER_SIGNER entries.
        let queue = state.signer_topics.get(&signer).unwrap();
        assert_eq!(queue.len(), MAX_TOPICS_PER_SIGNER);
        // t0 gone; t-new present.
        assert!(!queue.contains(&"t0".to_string()));
        assert!(queue.contains(&"t-new".to_string()));
        // t0 fully removed from the global table.
        assert!(!state.topic_refcount.contains_key("t0"));
    }

    #[test]
    fn announce_state_per_signer_lru_does_not_starve_other_signers() {
        // Signer A pumps MAX_TOPICS_PER_SIGNER topics; Signer B can still
        // announce successfully — its quota is independent.
        let mut state = AnnounceState::default();
        let signer_a = fresh_signer();
        let signer_b = fresh_signer();
        for i in 0..MAX_TOPICS_PER_SIGNER as u64 {
            state.process_topic(signer_a, &format!("a-{i}"));
        }
        // B's first announce must subscribe (no eviction, no rejection).
        let actions = state.process_topic(signer_b, "b-first");
        assert_eq!(actions.subscribe.as_deref(), Some("b-first"));
        assert!(!actions.evicted_for_signer);
        assert!(!actions.rejected_global);
        assert!(actions.unsubscribe.is_none());
        // B's slot is recorded under B (not under A).
        assert_eq!(state.signer_topics.get(&signer_b).map(|q| q.len()), Some(1));
    }

    #[test]
    fn announce_state_repeat_announce_promotes_lru_no_resubscribe() {
        // Re-announcing a topic the signer already holds is a no-op on the
        // network and promotes the topic to the back of the LRU queue.
        let mut state = AnnounceState::default();
        let signer = fresh_signer();
        state.process_topic(signer, "first");
        state.process_topic(signer, "second");
        let actions = state.process_topic(signer, "first");
        assert!(actions.subscribe.is_none());
        assert!(actions.unsubscribe.is_none());
        assert!(!actions.evicted_for_signer);
        // After re-announce, "first" is the most recent entry.
        let queue = state.signer_topics.get(&signer).unwrap();
        assert_eq!(queue.back().map(|s| s.as_str()), Some("first"));
        assert_eq!(queue.front().map(|s| s.as_str()), Some("second"));
    }

    #[test]
    fn announce_state_shared_topic_refcount_keeps_subscription() {
        // Two signers announce the same topic. Only the first results in a
        // Subscribe; both signers contribute to the refcount. Evicting the
        // first signer's topic must NOT unsubscribe (the second still holds it).
        let mut state = AnnounceState::default();
        let signer_a = fresh_signer();
        let signer_b = fresh_signer();
        let actions = state.process_topic(signer_a, "shared");
        assert_eq!(actions.subscribe.as_deref(), Some("shared"));
        let actions = state.process_topic(signer_b, "shared");
        assert!(actions.subscribe.is_none());

        // Force eviction on signer A by filling its quota.
        for i in 0..MAX_TOPICS_PER_SIGNER as u64 {
            state.process_topic(signer_a, &format!("filler-{i}"));
        }
        // "shared" should have been evicted from A but the global subscription
        // is still held by B — no Unsubscribe action.
        assert!(state.topic_refcount.contains_key("shared"));
        assert_eq!(state.topic_refcount.get("shared"), Some(&1));
    }

    #[test]
    fn should_emit_warn_rate_limits_to_one_per_window() {
        // Two warn attempts inside the window: first true, second false.
        // After the window: true again.
        let mut last = None;
        let t0 = Instant::now();
        let interval = Duration::from_millis(100);
        assert!(should_emit_warn(&mut last, t0, interval));
        assert!(!should_emit_warn(
            &mut last,
            t0 + Duration::from_millis(50),
            interval
        ));
        assert!(should_emit_warn(
            &mut last,
            t0 + Duration::from_millis(150),
            interval
        ));
    }

    #[test]
    fn announce_state_rejects_at_global_cap() {
        // Fill the global table by spreading entries across many signers
        // (because per-signer cap is lower than global cap). Then a fresh
        // signer's new topic must be rejected globally.
        let mut state = AnnounceState::default();
        let mut signers = Vec::new();
        let mut idx: u64 = 0;
        while state.topic_refcount.len() < MAX_TOPICS {
            let s = fresh_signer();
            for _ in 0..MAX_TOPICS_PER_SIGNER {
                if state.topic_refcount.len() >= MAX_TOPICS {
                    break;
                }
                state.process_topic(s, &format!("g-{idx}"));
                idx += 1;
            }
            signers.push(s);
        }
        assert_eq!(state.topic_refcount.len(), MAX_TOPICS);

        // Fresh signer's new topic — must be rejected (global table full,
        // topic not already present).
        let outsider = fresh_signer();
        let actions = state.process_topic(outsider, "new-after-full");
        assert!(actions.rejected_global);
        assert!(actions.subscribe.is_none());
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
