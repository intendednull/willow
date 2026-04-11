//! Integration tests for the bootstrap-id HTTP endpoint hardening
//! (issue #112) and the topic-announce listener bounds (issue #113).
//!
//! These tests exercise the helpers exposed by `willow-relay` directly,
//! without standing up a full relay/iroh stack.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use willow_relay::{
    handle_bootstrap_connection, run_bootstrap_listener, topic_str_is_valid, BOOTSTRAP_IO_TIMEOUT,
    MAX_TOPIC_LEN,
};

const TEST_ID: &str = "0123456789abcdef0123456789abcdef";

/// Spawn the bootstrap listener bound to an ephemeral loopback port
/// and return its address.
async fn spawn_listener_with_capacity(capacity: usize) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let semaphore = Arc::new(Semaphore::new(capacity));
    let id = Arc::new(TEST_ID.to_string());
    tokio::spawn(run_bootstrap_listener(listener, id, semaphore));
    addr
}

/// Read the full HTTP response from `stream` and return it as a string.
async fn read_full_response(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    // Read until EOF or 8 KiB, whichever comes first.
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        let mut chunk = [0u8; 1024];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
            if buf.len() >= 8192 {
                break;
            }
        }
    })
    .await;
    String::from_utf8_lossy(&buf).into_owned()
}

#[tokio::test]
async fn bootstrap_endpoint_serves_normal_request_quickly() {
    let addr = spawn_listener_with_capacity(8).await;

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .expect("write request");

    let response = read_full_response(&mut stream).await;

    assert!(
        response.starts_with("HTTP/1.1 200 OK\r\n"),
        "response: {response}"
    );
    assert!(response.contains(TEST_ID), "body missing id: {response}");
}

#[tokio::test]
async fn bootstrap_response_contains_connection_close_header() {
    let addr = spawn_listener_with_capacity(8).await;

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .expect("write request");

    let response = read_full_response(&mut stream).await;
    assert!(
        response.contains("Connection: close\r\n"),
        "response missing Connection: close header: {response}"
    );
}

#[tokio::test(start_paused = true)]
async fn handle_bootstrap_connection_times_out_slow_reader() {
    // Use an in-memory duplex pipe so we can drive the test with paused
    // tokio time. The "client" side never writes, so the handler's read
    // call should block until BOOTSTRAP_IO_TIMEOUT elapses in virtual
    // time, then return TimedOut.
    let (server, _client) = tokio::io::duplex(64);

    let handler = tokio::spawn(async move { handle_bootstrap_connection(server, TEST_ID).await });

    // Advance virtual time past the read deadline.
    tokio::time::advance(BOOTSTRAP_IO_TIMEOUT + Duration::from_millis(1)).await;

    let result = handler.await.expect("handler join");
    let err = result.expect_err("expected timeout error");
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
}

#[tokio::test]
async fn bootstrap_listener_drops_connections_when_capacity_saturated() {
    // Capacity of 1 — first connection holds the only permit, second
    // connection should be accepted, immediately denied a permit, and
    // closed by the server before any response is written.
    let addr = spawn_listener_with_capacity(1).await;

    // First client: open a connection but do NOT send a request. The
    // handler's read will block waiting for request data, holding the
    // sole permit for up to BOOTSTRAP_IO_TIMEOUT (5s). The test does
    // not wait that long — we drop `hold` at the end.
    let hold = TcpStream::connect(addr).await.expect("first connect");

    // Give the listener a moment to accept the first connection and
    // spawn the handler that grabs the permit. 50ms on local loopback
    // is generous; this is the only timing-sensitive step.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Second client: should be accepted, denied a permit, and dropped.
    // Do NOT write a request — the server-side stream is dropped
    // immediately on permit denial, so reading from the client side
    // should yield EOF (clean FIN) rather than RST.
    let mut denied = TcpStream::connect(addr).await.expect("second connect");

    // The server should close the socket without responding. Reading
    // should yield EOF (0 bytes), or in some kernels a ConnectionReset
    // — either outcome proves the server did not write a response.
    let mut buf = [0u8; 1024];
    let read = tokio::time::timeout(Duration::from_secs(1), denied.read(&mut buf))
        .await
        .expect("denied read should not block forever");
    match read {
        Ok(0) => {} // EOF — expected
        Ok(n) => panic!(
            "expected EOF on saturated connection, got {n} bytes: {:?}",
            &buf[..n]
        ),
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {} // also acceptable
        Err(e) => panic!("unexpected read error on saturated connection: {e}"),
    }

    // Keep `hold` alive until the assertion is done so the permit stays
    // taken. Drop it explicitly here for clarity.
    drop(hold);
}

#[tokio::test]
async fn bootstrap_listener_recovers_after_permit_released() {
    // After a connection completes and releases its permit, new
    // connections should succeed again.
    let addr = spawn_listener_with_capacity(1).await;

    // First request: serve and complete.
    {
        let mut stream = TcpStream::connect(addr).await.expect("connect 1");
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .expect("write 1");
        let response = read_full_response(&mut stream).await;
        assert!(response.contains(TEST_ID));
    }

    // The server-side task drops its permit when its scope ends. Give
    // the runtime a tick to run the drop, then try again.
    tokio::task::yield_now().await;
    tokio::time::sleep(Duration::from_millis(10)).await;

    let mut stream = TcpStream::connect(addr).await.expect("connect 2");
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .expect("write 2");
    let response = read_full_response(&mut stream).await;
    assert!(
        response.contains(TEST_ID),
        "second request failed: {response}"
    );
}

// ── Topic validation (#113) ─────────────────────────────────────────────
//
// Most of the topic_str_is_valid coverage lives in the `tests` module
// inside `crates/relay/src/lib.rs` (those are unit tests). Re-assert
// the headline cases here so the integration suite documents the API
// surface advertised by the public helper.

#[test]
fn topic_str_is_valid_public_api() {
    assert!(topic_str_is_valid("general"));
    assert!(!topic_str_is_valid(""));
    assert!(!topic_str_is_valid(&"x".repeat(MAX_TOPIC_LEN + 1)));
    assert!(!topic_str_is_valid("bad char!"));
}
