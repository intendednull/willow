//! # Willow Relay Server
//!
//! Wraps an iroh-relay server for NAT traversal and runs a bootstrap
//! gossip node that subscribes to system topics so new peers can join
//! the gossip mesh. Dynamically subscribes to channel topics announced
//! by peers via [`TopicAnnounce`](willow_common::WireMessage::TopicAnnounce).
//!
//! ## Role: transport-layer only
//!
//! The relay is a **transparent transport layer**. It forwards gossip
//! traffic and helps NATed peers discover each other, but it performs
//! **no application-level validation**:
//!
//! - It does **not** verify Ed25519 signatures on relayed messages.
//! - It does **not** apply or check the event-sourced state machine
//!   (see `willow-state`). Chain integrity, parent-hash verification,
//!   permission checks, dedup, and governance rules (roles, bans,
//!   channel ACLs) are *not* enforced here.
//! - It does **not** inspect, filter, or re-route messages based on
//!   their semantic content — all payloads are opaque bytes to the relay.
//!
//! The only semantic work this binary does is:
//!
//! 1. Validating that `TopicAnnounce` strings are syntactically
//!    well-formed (length + allowed character set) before subscribing
//!    to them. This is a DoS guard, not a governance check.
//! 2. Rate/cap limiting (max concurrent bootstrap connections, max
//!    distinct topics) to protect the relay itself.
//!
//! All DAG-level validation — signature checking, parent-hash
//! verification, permission enforcement, merge/replay correctness — is
//! the responsibility of **each client**. Clients must never trust
//! relayed bytes just because they arrived through the relay.
//!
//! ## Trust model
//!
//! From the DAG spec (see
//! [`docs/specs/2026-04-01-per-author-merkle-dag-state-design.md`](../../../docs/specs/2026-04-01-per-author-merkle-dag-state-design.md))
//! and `CLAUDE.md` "Trust Model":
//!
//! > The relay is a regular client — trusted only if explicitly granted
//! > `SyncProvider` permission by the owner.
//!
//! Concretely:
//!
//! - The relay operator's identity (the bootstrap node's Ed25519 key)
//!   has **no** implicit authority over any server. The owner of a
//!   server must explicitly grant `SyncProvider` via a `GrantPermission`
//!   event for the relay's peer ID to be considered an authoritative
//!   sync source for that server.
//! - Without such a grant, clients treat the relay like any other
//!   untrusted peer: they accept forwarded bytes for DAG sync but
//!   validate everything locally before applying it.
//! - A hostile or compromised relay can drop, delay, or reorder
//!   messages (availability impact) but cannot forge events, bypass
//!   permissions, or corrupt state — those are prevented by
//!   cryptographic signatures and deterministic state machine
//!   validation at the client.
//!
//! ## Cross-references
//!
//! - Trust model summary: `CLAUDE.md` → "Trust Model".
//! - DAG design + `SyncProvider` permission:
//!   `docs/specs/2026-04-01-per-author-merkle-dag-state-design.md`.
//! - State-authority checks enforced at clients:
//!   `docs/specs/2026-04-12-state-authority-and-mutations.md`.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iroh_base::RelayUrl;
use iroh_relay::server::{AccessConfig, RelayConfig, Server, ServerConfig};
use tokio::sync::Semaphore;
use tracing::info;
use willow_common::relay_info::{
    canonical_json, capability_etag, features, sign_capability_doc, Limitation, WillowRelayInfo,
};
use willow_identity::Identity;
use willow_network::Network;
use willow_relay::{
    run_proxy_listener, topic_announce_listener, CAPABILITY_PATH, MAX_CONCURRENT_PROXY_CONNECTIONS,
    MAX_TOPICS, MAX_TOPIC_LEN,
};

#[derive(Parser)]
#[command(name = "willow-relay", about = "Willow iroh relay + bootstrap node")]
struct Args {
    /// Port for the iroh relay HTTP server.
    #[arg(long, default_value = "3340")]
    relay_port: u16,

    /// Path to persist the bootstrap node's identity key.
    #[arg(long)]
    identity: Option<std::path::PathBuf>,

    /// Print the bootstrap node's EndpointId and exit.
    #[arg(long)]
    print_id: bool,

    // ── Capability-document operator metadata (all optional) ─────────
    // Advertised at GET /.well-known/willow. Each falls back to its
    // env var, then to None (omitted from the document). See the spec:
    // docs/specs/2026-04-24-relay-capability-doc.md.
    /// Operator-chosen display name (≤ 60 UTF-8 bytes by convention).
    #[arg(long, env = "WILLOW_RELAY_NAME")]
    relay_name: Option<String>,

    /// Plain-text relay description (no markup).
    #[arg(long, env = "WILLOW_RELAY_DESCRIPTION")]
    relay_description: Option<String>,

    /// Operator contact (e.g. mailto: / https: / matrix:).
    #[arg(long, env = "WILLOW_RELAY_CONTACT")]
    relay_contact: Option<String>,

    /// Terms-of-service URL.
    #[arg(long, env = "WILLOW_RELAY_TOS")]
    relay_tos: Option<String>,

    /// Privacy-policy URL.
    #[arg(long, env = "WILLOW_RELAY_PRIVACY")]
    relay_privacy: Option<String>,

    /// Square icon URL (≥ 64×64).
    #[arg(long, env = "WILLOW_RELAY_ICON")]
    relay_icon: Option<String>,
}

/// Construct, sign, and pre-render the relay's capability document.
///
/// Returns `(info_json, etag)`: the serialized JSON body served at
/// `GET /.well-known/willow` and the strong ETag (lowercase-hex SHA-256
/// over the canonical bytes *with* the signature). Built once at startup
/// so the request path performs no crypto.
///
/// `protocol_versions` is sourced from the live
/// [`willow_transport::PROTOCOL_VERSION`] (highest-first, no duplicates),
/// not a literal. Advertised `supported_features` cover the capabilities that
/// exist today (gossip, history, blobs) plus `history-eose` now that PR 5's
/// `HistorySyncComplete` marker has landed and the relay forwards it unchanged;
/// the remaining sibling-spec tag (`seq-vector-sync`) is added only once its
/// implementing PR lands. Operator metadata comes from optional CLI args / env
/// vars, defaulting to `None` (the field is then omitted).
fn build_capability_doc(args: &Args, identity: &Identity) -> Result<(String, String)> {
    let mut info = WillowRelayInfo {
        name: args.relay_name.clone(),
        description: args.relay_description.clone(),
        contact: args.relay_contact.clone(),
        admin_pubkey: None,
        pubkey: hex::encode(identity.public_key().as_bytes()),
        software: Some("willow-relay".into()),
        version: None,
        terms_of_service: args.relay_tos.clone(),
        privacy_policy: args.relay_privacy.clone(),
        icon: args.relay_icon.clone(),
        protocol_versions: vec![willow_transport::PROTOCOL_VERSION],
        supported_features: vec![
            features::GOSSIP.into(),
            features::HISTORY.into(),
            features::BLOBS.into(),
            // EOSE: the relay forwards the `HistorySyncComplete` end-of-stored-
            // events marker unchanged (content-agnostic gossip passthrough,
            // pinned by `tests/history_sync_passthrough.rs`). Advertised here
            // now that PR 5's marker has landed.
            features::HISTORY_EOSE.into(),
        ],
        signature: String::new(),
        limitation: Some(Limitation {
            max_message_bytes: Some(willow_transport::MAX_DESER_SIZE as u32),
            max_topic_len: Some(MAX_TOPIC_LEN as u16),
            max_topics: Some(MAX_TOPICS as u32),
            max_connections: Some(MAX_CONCURRENT_PROXY_CONNECTIONS as u32),
            max_blob_bytes: Some(0),
            invite_required: false,
            payment_required: false,
            hlc_lower_limit: None,
            min_client_version: None,
        }),
        retention: None,
        payments_url: None,
        invites_url: None,
        // Static "ok" in v1; dynamic degraded/read_only health is deferred
        // (so the two-tier Cache-Control is effectively single-tier).
        status: Some("ok".into()),
        status_detail: None,
    };

    sign_capability_doc(&mut info, identity).context("failed to sign capability document")?;
    let json = serde_json::to_string(&info).context("failed to serialize capability document")?;
    let etag = capability_etag(
        &canonical_json(&info, true).context("failed to canonicalize capability document")?,
    );
    Ok((json, etag))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Load or generate bootstrap identity.
    let identity = if let Some(ref path) = args.identity {
        Identity::load_or_generate(path)?
    } else {
        Identity::generate()
    };

    if args.print_id {
        println!("{}", identity.endpoint_id());
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    info!(id = %identity.endpoint_id().fmt_short(), "bootstrap node identity");

    // ── Build + sign the capability document once, at startup ────────────
    // Served at GET /.well-known/willow. Pre-render JSON + ETag so the
    // hot path does zero crypto per request. `protocol_versions` is
    // sourced from the live transport constant (never a literal), and
    // limits mirror the live relay constants.
    let (capability_json, capability_etag) = build_capability_doc(&args, &identity)?;
    info!(
        path = CAPABILITY_PATH,
        etag = %capability_etag,
        "capability document built + signed"
    );

    // ── Public listener (HTTP dispatch: /bootstrap-id + proxy) ───────────
    // Bind the public-facing TCP port FIRST so `relay_port = 0` can pick
    // an ephemeral port that we can then advertise. The iroh-relay is
    // bound to a loopback ephemeral port below, and the proxy below
    // forwards non-bootstrap traffic to it.
    let public_bind: SocketAddr = (Ipv4Addr::UNSPECIFIED, args.relay_port).into();
    let public_listener = tokio::net::TcpListener::bind(public_bind)
        .await
        .context("failed to bind public relay HTTP port")?;
    let public_port = public_listener.local_addr()?.port();
    info!(port = public_port, "public relay HTTP listener ready");

    // ── iroh-relay server (loopback only, plain HTTP, no TLS) ────────────
    // The iroh-relay is only reachable through the proxy above; binding
    // to 127.0.0.1 prevents direct access from the network.
    let relay_bind: SocketAddr = (Ipv4Addr::LOCALHOST, 0).into();

    let mut relay_server = Server::spawn(ServerConfig::<(), ()> {
        relay: Some(RelayConfig::<(), ()> {
            http_bind_addr: relay_bind,
            tls: None,
            limits: Default::default(),
            key_cache_capacity: Some(1024),
            access: AccessConfig::Everyone,
        }),
        quic: None,
        metrics_addr: None,
    })
    .await
    .context("failed to start iroh-relay server")?;

    let upstream_relay_addr = relay_server
        .http_addr()
        .context("relay server has no HTTP address")?;
    // Advertise the PUBLIC port to bootstrap node and clients, not the
    // loopback upstream port.
    let advertised_addr: SocketAddr = (Ipv4Addr::LOCALHOST, public_port).into();
    let relay_url: RelayUrl = format!("http://{advertised_addr}")
        .parse()
        .context("failed to parse relay URL")?;
    info!(
        %relay_url,
        upstream = %upstream_relay_addr,
        "iroh-relay server running"
    );

    // ── Bootstrap gossip node ────────────────────────────────────────────
    let config = willow_network::iroh::Config {
        secret_key: identity.secret_key().clone(),
        relay_url: Some(relay_url),
        bootstrap_peers: vec![],
        mdns: true,
    };
    let network = willow_network::iroh::IrohNetwork::new(config)
        .await
        .context("failed to start bootstrap network")?;

    // Subscribe to system topics so new peers can bootstrap into the mesh.
    let server_ops_topic = willow_network::topic_id("_willow_server_ops");
    let workers_topic = willow_network::topic_id("_willow_workers");
    let profiles_topic = willow_network::topic_id("_willow_profiles");

    // Keep the server_ops events stream to listen for TopicAnnounce messages.
    let (_ops_sender, ops_events) = network.subscribe(server_ops_topic, vec![]).await?;
    let _ = network.subscribe(workers_topic, vec![]).await?;
    let _ = network.subscribe(profiles_topic, vec![]).await?;

    info!("bootstrap node subscribed to system topics");

    // ── Public dispatch loop ────────────────────────────────────────────
    // The public listener multiplexes two concerns on ONE port:
    //   * `GET /bootstrap-id` → serves the bootstrap node's endpoint ID
    //   * everything else     → proxied to the loopback iroh-relay
    //
    // The accept loop applies per-connection I/O timeouts and is gated
    // by a semaphore so a connection-flood DoS cannot exhaust file
    // descriptors. See `willow_relay::run_proxy_listener`.
    let bootstrap_id = Arc::new(identity.endpoint_id().to_string());
    let proxy_semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_PROXY_CONNECTIONS));
    let capability_json: Arc<str> = Arc::from(capability_json);
    let capability_etag: Arc<str> = Arc::from(capability_etag);
    tokio::spawn(run_proxy_listener(
        public_listener,
        upstream_relay_addr,
        Arc::clone(&bootstrap_id),
        proxy_semaphore,
        capability_json,
        capability_etag,
    ));
    info!(
        port = public_port,
        "serving /bootstrap-id + /.well-known/willow + iroh-relay proxy"
    );

    // Spawn a task that listens for TopicAnnounce messages on _willow_server_ops
    // and dynamically subscribes to announced channel topics.
    tokio::spawn(topic_announce_listener(ops_events, network));

    info!("relay running — press Ctrl+C to stop");

    // Wait for shutdown signal or relay task failure.
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("received Ctrl+C, shutting down");
        }
        result = relay_server.task_handle() => {
            match result {
                Ok(Ok(())) => info!("relay server exited cleanly"),
                Ok(Err(e)) => tracing::error!(%e, "relay server error"),
                Err(e) => tracing::error!(%e, "relay server task panicked"),
            }
        }
    }

    // Network is moved into the listener task, so we just shut down the relay.
    if let Err(e) = relay_server.shutdown().await {
        tracing::warn!(%e, "relay server shutdown failed");
    }
    info!("shut down complete");
    Ok(())
}
