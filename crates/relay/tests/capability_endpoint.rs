//! Integration tests for the signed `/.well-known/willow` capability
//! document endpoint (spec `docs/specs/2026-04-24-relay-capability-doc.md`).
//!
//! These exercise the public proxy dispatch path directly via
//! [`willow_relay::run_proxy_listener`] with a dummy upstream, mirroring the
//! harness in `bootstrap_endpoint.rs`. The capability JSON + ETag are built
//! once here (as the binary builds them once at startup) and threaded into the
//! listener.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use willow_common::relay_info::{
    canonical_json, capability_etag, features, sign_capability_doc, verify_capability_doc,
    Limitation, Retention, WillowRelayInfo,
};
use willow_identity::Identity;
use willow_relay::run_proxy_listener;

const CAPABILITY_PATH: &str = "/.well-known/willow";

/// Build a representative, signed capability document plus its rendered JSON
/// and strong ETag — the exact tuple the binary pre-renders at startup.
fn build_capability() -> (Identity, String, String) {
    let identity = Identity::generate();
    let mut info = WillowRelayInfo {
        name: Some("Test Relay".into()),
        description: Some("integration-test relay".into()),
        contact: None,
        admin_pubkey: None,
        pubkey: hex::encode(identity.public_key().as_bytes()),
        software: Some("willow-relay".into()),
        version: Some("0.1.x".into()),
        terms_of_service: None,
        privacy_policy: None,
        icon: None,
        protocol_versions: vec![willow_transport::PROTOCOL_VERSION],
        supported_features: vec![
            features::GOSSIP.into(),
            features::HISTORY.into(),
            features::BLOBS.into(),
        ],
        signature: String::new(),
        limitation: Some(Limitation {
            max_message_bytes: Some(256 * 1024),
            max_topic_len: Some(256),
            max_topics: Some(10_000),
            max_connections: Some(1024),
            max_blob_bytes: Some(0),
            invite_required: false,
            payment_required: false,
            hlc_lower_limit: None,
            min_client_version: None,
        }),
        retention: Some(Retention {
            mode: "storage".into(),
            max_events_per_author: Some(1_000),
            max_age_seconds: None,
            channel_key_escrow: false,
        }),
        payments_url: None,
        invites_url: None,
        status: Some("ok".into()),
        status_detail: None,
    };
    sign_capability_doc(&mut info, &identity).expect("sign capability doc");
    let json = serde_json::to_string(&info).expect("serialize capability doc");
    let etag = capability_etag(&canonical_json(&info, true).expect("canonicalize"));
    (identity, json, etag)
}

/// Spawn a dummy upstream that accepts any connection, drains briefly, then
/// closes. Returns its address and a receiver that fires per accepted conn.
async fn spawn_dummy_upstream() -> (std::net::SocketAddr, tokio::sync::mpsc::Receiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let tx = tx.clone();
            tokio::spawn(async move {
                let _ = tx.send(()).await;
                let mut buf = [0u8; 1024];
                let _ =
                    tokio::time::timeout(Duration::from_millis(100), stream.read(&mut buf)).await;
                drop(stream);
            });
        }
    });
    (addr, rx)
}

/// Spawn the public proxy listener with a pre-rendered capability doc + ETag
/// and return its bound address.
async fn spawn_proxy(
    upstream_addr: std::net::SocketAddr,
    info_json: &str,
    etag: &str,
) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let semaphore = Arc::new(Semaphore::new(16));
    let id = Arc::new("0123456789abcdef0123456789abcdef".to_string());
    let info_json: Arc<str> = Arc::from(info_json);
    let etag: Arc<str> = Arc::from(etag);
    tokio::spawn(run_proxy_listener(
        listener,
        upstream_addr,
        id,
        semaphore,
        info_json,
        etag,
    ));
    addr
}

/// Read the full HTTP response from `stream`.
async fn read_full_response(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        let mut chunk = [0u8; 1024];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
            if buf.len() >= 64 * 1024 {
                break;
            }
        }
    })
    .await;
    String::from_utf8_lossy(&buf).into_owned()
}

/// Split an HTTP response into (status+headers, body).
fn split_headers_body(response: &str) -> (&str, &str) {
    match response.find("\r\n\r\n") {
        Some(pos) => (&response[..pos], &response[pos + 4..]),
        None => (response, ""),
    }
}

#[tokio::test]
async fn capability_get_returns_signed_doc_with_cors_and_etag() {
    let (_id, info_json, etag) = build_capability();
    let (upstream, _rx) = spawn_dummy_upstream().await;
    let addr = spawn_proxy(upstream, &info_json, &etag).await;

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(
            format!("GET {CAPABILITY_PATH} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes(),
        )
        .await
        .expect("write request");

    let response = read_full_response(&mut stream).await;
    let (headers, body) = split_headers_body(&response);

    assert!(
        headers.starts_with("HTTP/1.1 200 OK\r\n"),
        "expected 200, got: {headers}"
    );
    assert!(
        headers.contains("Content-Type: application/willow+json; charset=utf-8\r\n"),
        "missing/incorrect content-type: {headers}"
    );
    assert!(
        headers.contains("Access-Control-Allow-Origin: *\r\n"),
        "missing ACAO: {headers}"
    );
    assert!(
        headers.contains("Access-Control-Allow-Methods: GET, OPTIONS\r\n"),
        "missing ACAM: {headers}"
    );
    assert!(
        headers.contains("Access-Control-Allow-Headers: Accept, Content-Type, If-None-Match\r\n"),
        "missing ACAH: {headers}"
    );
    assert!(
        headers.contains(&format!("ETag: \"{etag}\"\r\n")),
        "missing/incorrect ETag: {headers}"
    );
    assert!(
        headers.contains("Cache-Control: public, max-age=300\r\n"),
        "missing Cache-Control: {headers}"
    );
    assert!(
        headers.contains("Connection: close\r\n"),
        "missing Connection: close: {headers}"
    );

    // Body parses as a WillowRelayInfo and its signature verifies.
    let info: WillowRelayInfo = serde_json::from_str(body).expect("body parses as WillowRelayInfo");
    assert_eq!(info.protocol_versions, vec![willow_transport::PROTOCOL_VERSION]);
    assert!(
        verify_capability_doc(&info).expect("verify"),
        "served document signature must verify"
    );
}

#[tokio::test]
async fn capability_options_preflight_returns_204_with_cors() {
    let (_id, info_json, etag) = build_capability();
    let (upstream, _rx) = spawn_dummy_upstream().await;
    let addr = spawn_proxy(upstream, &info_json, &etag).await;

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(
            format!("OPTIONS {CAPABILITY_PATH} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes(),
        )
        .await
        .expect("write request");

    let response = read_full_response(&mut stream).await;
    let (headers, body) = split_headers_body(&response);

    assert!(
        headers.starts_with("HTTP/1.1 204 No Content\r\n"),
        "expected 204, got: {headers}"
    );
    assert!(
        headers.contains("Access-Control-Allow-Origin: *\r\n"),
        "missing ACAO: {headers}"
    );
    assert!(
        headers.contains("Access-Control-Allow-Methods: GET, OPTIONS\r\n"),
        "missing ACAM: {headers}"
    );
    assert!(
        headers.contains("Access-Control-Allow-Headers: Accept, Content-Type, If-None-Match\r\n"),
        "missing ACAH: {headers}"
    );
    assert!(body.is_empty(), "204 body must be empty, got: {body:?}");
}

#[tokio::test]
async fn capability_if_none_match_returns_304() {
    let (_id, info_json, etag) = build_capability();
    let (upstream, _rx) = spawn_dummy_upstream().await;
    let addr = spawn_proxy(upstream, &info_json, &etag).await;

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(
            format!(
                "GET {CAPABILITY_PATH} HTTP/1.1\r\nHost: localhost\r\nIf-None-Match: \"{etag}\"\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .expect("write request");

    let response = read_full_response(&mut stream).await;
    let (headers, body) = split_headers_body(&response);

    assert!(
        headers.starts_with("HTTP/1.1 304 Not Modified\r\n"),
        "expected 304, got: {headers}"
    );
    assert!(
        headers.contains(&format!("ETag: \"{etag}\"\r\n")),
        "304 must echo the ETag: {headers}"
    );
    assert!(body.is_empty(), "304 body must be empty, got: {body:?}");
}

#[tokio::test]
async fn capability_stale_if_none_match_returns_full_doc() {
    let (_id, info_json, etag) = build_capability();
    let (upstream, _rx) = spawn_dummy_upstream().await;
    let addr = spawn_proxy(upstream, &info_json, &etag).await;

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    // A mismatched If-None-Match must NOT short-circuit to 304.
    stream
        .write_all(
            format!(
                "GET {CAPABILITY_PATH} HTTP/1.1\r\nHost: localhost\r\nIf-None-Match: \"stale-etag\"\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .expect("write request");

    let response = read_full_response(&mut stream).await;
    let (headers, _body) = split_headers_body(&response);
    assert!(
        headers.starts_with("HTTP/1.1 200 OK\r\n"),
        "stale ETag must serve the full doc (200), got: {headers}"
    );
}

#[tokio::test]
async fn unrelated_path_still_proxies_upstream() {
    // The iroh-relay uses paths like /relay and /ping. They must keep
    // proxying to upstream unchanged after the capability branch lands.
    let (_id, info_json, etag) = build_capability();
    let (upstream, mut rx) = spawn_dummy_upstream().await;
    let addr = spawn_proxy(upstream, &info_json, &etag).await;

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"GET /relay HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\r\n")
        .await
        .expect("write request");

    let got = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("unrelated path must proxy to upstream");
    assert!(got.is_some(), "upstream channel closed unexpectedly");
}

#[tokio::test]
async fn capability_post_falls_through_to_upstream() {
    // Only GET/OPTIONS on the capability path are handled locally; a POST
    // must proxy upstream like any other request.
    let (_id, info_json, etag) = build_capability();
    let (upstream, mut rx) = spawn_dummy_upstream().await;
    let addr = spawn_proxy(upstream, &info_json, &etag).await;

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(
            format!("POST {CAPABILITY_PATH} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes(),
        )
        .await
        .expect("write request");

    let got = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("POST on capability path must proxy to upstream");
    assert!(got.is_some(), "upstream channel closed unexpectedly");
}
