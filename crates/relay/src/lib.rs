//! # Willow Relay (library)
//!
//! Helpers and constants used by the `willow-relay` binary. This is a
//! thin library wrapper so that the bootstrap-handler logic and topic
//! validation can be exercised by integration tests in
//! `crates/relay/tests/`. The actual `main` lives in `src/main.rs`.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;
use tracing::{info, warn};
use willow_network::traits::{GossipEvent, TopicEvents};
use willow_network::Network;

/// Maximum concurrent client connections accepted by the bootstrap-id
/// HTTP endpoint. Excess accepts are dropped immediately to prevent
/// FD/memory exhaustion under a connection-flood DoS.
pub const MAX_CONCURRENT_BOOTSTRAP_CONNECTIONS: usize = 1024;

/// I/O deadline for any single read or write on a bootstrap-id
/// connection. Slow clients (Slowloris) are dropped at this deadline.
pub const BOOTSTRAP_IO_TIMEOUT: Duration = Duration::from_secs(5);

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

/// Listen for `TopicAnnounce` messages on the server-ops topic and
/// dynamically subscribe to announced channel topics. Topics are
/// validated against [`topic_str_is_valid`] and the number of distinct
/// subscribed topics is capped at [`MAX_TOPICS`].
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
