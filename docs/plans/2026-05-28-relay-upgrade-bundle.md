# Relay Upgrade Bundle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Date:** 2026-05-28
**Status:** active
**Spec:** specs/2026-04-24-relay-capability-doc.md, specs/2026-04-24-negentropy-sync.md, specs/2026-04-24-history-sync-eose.md, specs/2026-04-24-outbox-relay-discovery.md

**Goal:** Realize the four `[active]` relay/sync specs as six sequential, independently-shippable PRs: a signed relay capability document, heads-based delta sync over gossip, an end-of-stored-events completion marker, and pkarr-based relay/worker discovery.

**Architecture:** The bundle layers cleanly. PRs 1–2 add a self-contained `/.well-known/willow` HTTP sidecar served by the relay (pure types + crypto, then the endpoint). PRs 3–4 replace the legacy 500-event gossip dump with the `HeadsSummary`-based delta protocol the worker path already uses, split into wire/query types (pure) then the live listener cutover. PR 5 adds the `HistorySyncComplete` EOSE marker on top of the new streaming. PR 6 composes iroh pkarr discovery + the capability doc + existing `SyncProvider` grants for relay/worker discovery. Every wire change is **additive** (new `WireMessage` enum variants); no `PROTOCOL_VERSION` bump, no new `MessageType` slot.

**Tech Stack:** Rust workspace (`willow-common`, `willow-state`, `willow-storage`, `willow-relay`, `willow-client`, `willow-worker`, `willow-replay`, `willow-network`, `willow-transport`, `willow-identity`); iroh 0.98.1 (QUIC + gossip + `discovery-pkarr-dht`); bincode wire framing; Ed25519 signing; serde_json + RFC 8785 JCS for the capability doc; SQLite (`rusqlite`) for storage.

---

## How to use this plan

This is a **bundle plan** spanning four specs. Each "PR" section below is a self-contained unit of work that lands CI-green and merges to `main` **before the next begins** (per the project's sequential-PR workflow). PR 1 is fully TDD-coded because it is pure, deterministic library code. Later PRs give exact files, type shapes, ordered TDD steps, and per-tier test assertions; a small number of steps are explicitly **measure-then-encode** (the gossip byte-budget constant) or **verify-then-call** (the iroh 0.98.1 pkarr builder method) — those are real TDD steps, not placeholders: the test that measures/verifies is written first and pins the value.

Read [Cross-spec pinned decisions](#cross-spec-pinned-decisions) before writing any code — those choices are locked across all six PRs.

> **Provenance:** PR ordering and pinned decisions were derived from a full read of all four specs plus a parallel per-spec gap-analysis of the current code (workflow `relay-upgrade-bundle-plan`, 4 analyzers + 1 synthesizer). Line refs below were reported by that analysis against the worktree at the time of writing; re-confirm with `grep`/`Read` before editing, since unrelated commits may shift them.

---

## Cross-spec pinned decisions

These were resolved by cross-cutting analysis of all four specs. **Do not relitigate during implementation.**

1. **No new `MessageType` slot in this bundle.** `MessageType` (`crates/transport/src/lib.rs`, `Chat=0 … Ping=6`) is unchanged. `SyncRequestV2`, `SyncBatchV2`, and `HistorySyncComplete` are all additive `WireMessage` variants riding the existing `MessageType::Channel` envelope. Add a code comment in `crates/transport/src/lib.rs` reserving **slot 7 = EOSE/HistorySyncComplete** and **slot 8 = Sync** for a *future* promotion, so the two specs can never collide on a number. Allocate neither now. (Rationale: an additive `WireMessage` variant confines an old-peer decode failure to that one variant; a new `MessageType` would risk every message kind via `Envelope::validate_version`.)

2. **No `PROTOCOL_VERSION` bump.** Legacy `WireMessage::SyncRequest`/`SyncBatch` stay defined and decodable for one release cycle so old and new peers interoperate. A version bump would reject the entire envelope on mismatch, breaking every message kind. Removal of the legacy variants is a follow-up release, out of this bundle's scope.

3. **`supported_features` canonical tags** live as `pub const` strings in `willow-common` next to `WillowRelayInfo`, so producer (relay) and consumer (client) agree by construction: `"gossip"`, `"history"`, `"blobs"`, `"voice-signal"`, `"invite-gate"`, `"payment-gate"`, plus `"history-eose"` (advertised once PR 5 lands) and **`"seq-vector-sync"`** (advertised once PR 4 lands). The sync tag is `"seq-vector-sync"`, **not** `"negentropy"` — the spec abandoned Negentropy/RBSR for per-author seq cursors, so advertising `"negentropy"` would misrepresent the wire protocol.

4. **`SyncProvider` serving-gate lands as the final step of PR 4**, gating only the new `SyncRequestV2` responder: a peer serves a delta only if **it** holds `SyncProvider` for that server. Requesters are never gated. The legacy 500-event responder stays **ungated** (it is deleted later with the legacy variants). (`SyncProvider` already exists at `crates/state/src/event.rs`.) **Updated 2026-05-30 (PR #664):** the gate honors the owner/admins' **implicit** `SyncProvider` — it uses `ServerState::is_sync_provider` (= `has_permission(SyncProvider)`), **not** `has_explicit_permission`. The owner is the root of all permissions and admins inherit every permission, so the owner serves its own server's state to a joining member; an explicit grant-holder serves too; a regular member without the grant still refuses. The original explicit-only reading made a 2-peer server (nobody holds an explicit grant) unsyncable, which broke member-to-member backfill. See `specs/2026-04-24-negentropy-sync.md` (serving-gate note) and `reports/2026-05-30-heads-sync-owner-serve-and-eose-emission.md`.

5. **`SyncCompleted` vs `HistorySyncComplete` → Option B (additive).** Introduce a new `ClientEvent::HistorySynced { topic, provider, still_pending }` (PR 5). Keep `ClientEvent::SyncCompleted { ops_applied }` as session-wide per-batch progress; do **not** repurpose its emission point. They answer different questions (per-topic boundary vs session-wide batch). Avoids a silent behavior change to existing consumers (`crates/agent/src/notifications.rs`).

6. **`stream_generation` is a random `u64`** (from the existing `rand` dep / `ChaCha20Rng`), not a persisted counter — equality-based dedup needs no ordering and randomness avoids the "did I bump it?" bug class.

7. **`last_event_hash` stays `Option`** (`None` = provider had zero stored events) — cleanly distinguishes "empty store" from "done streaming N events".

8. **`request_id` is `String`** on the new gossip variants, matching the worker path's `WorkerWireMessage::Request/Response { request_id: String }`, so one demux table covers both paths. Generate it with the workspace `uuid` crate (v4, dual-target WASM-safe), **not** `std` RNG.

## Conflicts resolved

- **heads-sync ↔ history-eose:** the `e2e/history-sync.spec.ts` "`HistorySyncComplete` fires" assertion belongs to **PR 5**, not PR 4. PR 4's e2e is limited to a byte-count diff assertion. Worker-class markers (PR 5) do **not** require peer-to-peer sync; the peer-to-peer marker is a documented follow-up.
- **capability-doc ↔ heads-sync:** sync-algorithm feature tag is `"seq-vector-sync"` (see decision 3). The capability-doc spec's cross-spec table is updated to drop the `"negentropy"` ambiguity.
- **capability-doc ↔ pkarr-discovery:** the client-side `fetch_relay_info(url)` helper (fetch + verify signature + protocol intersection) is defined **once** by the capability-doc consumer work and **reused** by pkarr-discovery ranking. Sequencing (PR 6 after PR 2) guarantees it exists first.

## Shared prerequisites (land in the PR that first needs them, never duplicate)

- `WillowRelayInfo` + JCS canonicalization + Ed25519 signing live in **`willow-common`** (PR 1) so both the relay producer and the client consumer (PR 6) import the *identical* canonicalization code. Reimplementing per side would silently break signature verification.
- `SyncFilter` + the `request_id: String` correlation convention (PR 3) are shared by the gossip responder (PR 4); the worker path already uses the same `request_id` type.
- The **64 KiB-minus-framing byte-budget helper** (PR 3/4) is measured empirically against a real `Envelope + WireMessage + SignedMessage` round-trip and used by both the gossip responder and the worker `Sync` arms.

## PR sequence (dependency graph)

```
PR1 (cap-doc types+JCS+sign, willow-common)        ── no deps
 └─ PR2 (cap-doc relay endpoint)                    ── needs PR1
     └─ PR6 (pkarr + JoinToken + client ranking)    ── needs PR2

PR3 (heads-sync wire types + state/storage queries) ── no deps
 └─ PR4 (heads-sync client+worker cutover + gate)   ── needs PR3
     └─ PR5 (HistorySyncComplete EOSE marker)        ── needs PR4
```

PR1→PR2→PR6 (capability/discovery track) and PR3→PR4→PR5 (sync track) are independent tracks; they may be developed in either interleaving but each PR still merges before its dependents start.

---

## PR 1 — Capability document: types + JCS canonicalization + signing

**Spec:** `specs/2026-04-24-relay-capability-doc.md`
**Depends on:** nothing. Pure library code in `willow-common`; no network I/O, no wire change.

**Files:**
- Create: `crates/common/src/relay_info.rs`
- Modify: `crates/common/src/lib.rs` (add `pub mod relay_info;` + re-exports)
- Modify: `crates/common/Cargo.toml` (add `sha2`, `serde_jcs`; `serde`, `serde_json`, `hex` already present via workspace)
- Modify: root `Cargo.toml` `[workspace.dependencies]` (add `sha2 = "0.10"`, `serde_jcs = "0.1"`)

**New types** (all in `crates/common/src/relay_info.rs`, `#[derive(Debug, Clone, Serialize, Deserialize)]`):

```rust
/// Capability document served at `GET /.well-known/willow`.
/// Clients MUST ignore unknown top-level fields (serde does this by default).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct WillowRelayInfo {
    #[serde(skip_serializing_if = "Option::is_none")] pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub contact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub admin_pubkey: Option<String>,
    /// REQUIRED. Hex Ed25519 public half of the relay's own key (verifier for `signature`).
    pub pubkey: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub software: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub terms_of_service: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub privacy_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub icon: Option<String>,
    /// REQUIRED. Wire-protocol versions accepted, sorted highest-first, no duplicates.
    pub protocol_versions: Vec<u16>,
    #[serde(default)] pub supported_features: Vec<String>,
    /// REQUIRED. Detached Ed25519 signature over RFC 8785 JCS of this object
    /// with `signature` removed. Lowercase hex.
    pub signature: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub limitation: Option<Limitation>,
    #[serde(skip_serializing_if = "Option::is_none")] pub retention: Option<Retention>,
    #[serde(skip_serializing_if = "Option::is_none")] pub payments_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub invites_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub status_detail: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Limitation {
    #[serde(skip_serializing_if = "Option::is_none")] pub max_message_bytes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")] pub max_topic_len: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")] pub max_topics: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")] pub max_connections: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")] pub max_blob_bytes: Option<u64>,
    #[serde(default)] pub invite_required: bool,
    #[serde(default)] pub payment_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")] pub hlc_lower_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub min_client_version: Option<u16>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Retention {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub max_events_per_author: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")] pub max_age_seconds: Option<u64>,
    #[serde(default)] pub channel_key_escrow: bool,
}

/// Canonical v1 `supported_features` tag strings (decision 3).
pub mod features {
    pub const GOSSIP: &str = "gossip";
    pub const HISTORY: &str = "history";
    pub const BLOBS: &str = "blobs";
    pub const VOICE_SIGNAL: &str = "voice-signal";
    pub const INVITE_GATE: &str = "invite-gate";
    pub const PAYMENT_GATE: &str = "payment-gate";
    pub const HISTORY_EOSE: &str = "history-eose";       // advertised once PR5 lands
    pub const SEQ_VECTOR_SYNC: &str = "seq-vector-sync"; // advertised once PR4 lands
}

#[derive(Debug, thiserror::Error)]
pub enum CapabilityError {
    #[error("missing required field: {0}")] MissingField(&'static str),
    #[error("canonicalization failed: {0}")] Canon(String),
    #[error("signature verification failed")] BadSignature,
}
```

### Task 1.1: Add workspace + crate dependencies

- [ ] **Step 1: Add to root `Cargo.toml` `[workspace.dependencies]`**

```toml
sha2 = "0.10"
serde_jcs = "0.1"
```

- [ ] **Step 2: Add to `crates/common/Cargo.toml` `[dependencies]`**

```toml
serde_json = { workspace = true }
sha2 = { workspace = true }
serde_jcs = { workspace = true }
hex = { workspace = true }
thiserror = { workspace = true }
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p willow-common`
Expected: PASS (no usages yet).

- [ ] **Step 4: Commit** — `git commit -am "build(common): add sha2 + serde_jcs for capability doc"`

> **Note on `serde_jcs`:** it produces RFC 8785 canonical JSON from any `Serialize` value. If a reviewer objects to the dependency, the fallback is a ~60-line inline canonicalizer (serialize via `serde_json::Value`, recursively sort object keys into a `BTreeMap`, emit with no whitespace, ASCII-escape per RFC 8785 §3.2.2). Prefer the crate — hand-rolled number/unicode escaping is the documented source of cross-implementation canonicalization bugs.

### Task 1.2: Struct definitions + serde round-trip

- [ ] **Step 1: Write the failing test** in `crates/common/src/relay_info.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> WillowRelayInfo {
        WillowRelayInfo {
            name: Some("Test Relay".into()),
            description: None, contact: None, admin_pubkey: None,
            pubkey: "ab".repeat(32),
            software: None, version: None, terms_of_service: None,
            privacy_policy: None, icon: None,
            protocol_versions: vec![2],
            supported_features: vec![features::GOSSIP.into(), features::HISTORY.into()],
            signature: String::new(),
            limitation: None, retention: None,
            payments_url: None, invites_url: None,
            status: Some("ok".into()), status_detail: None,
        }
    }

    #[test]
    fn full_round_trip() {
        let info = sample();
        let json = serde_json::to_string(&info).unwrap();
        let back: WillowRelayInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, back);
    }

    #[test]
    fn minimal_doc_parses_with_none_fields() {
        let json = r#"{"protocol_versions":[1],"pubkey":"aa","signature":"bb"}"#;
        let info: WillowRelayInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.protocol_versions, vec![1]);
        assert!(info.name.is_none() && info.limitation.is_none());
        assert!(info.supported_features.is_empty());
    }

    #[test]
    fn unknown_top_level_field_is_ignored() {
        let json = r#"{"protocol_versions":[2],"pubkey":"aa","signature":"bb","future_field":42}"#;
        let info: WillowRelayInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.protocol_versions, vec![2]);
    }
}
```

- [ ] **Step 2: Run to verify it fails** — `cargo test -p willow-common relay_info::tests::full_round_trip` → FAIL (types undefined).
- [ ] **Step 3: Add the struct definitions** (the `WillowRelayInfo`/`Limitation`/`Retention`/`features`/`CapabilityError` block above) and `pub mod relay_info;` + `pub use relay_info::{WillowRelayInfo, Limitation, Retention, CapabilityError};` to `crates/common/src/lib.rs`.
- [ ] **Step 4: Run to verify it passes** — `cargo test -p willow-common relay_info` → PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(common): WillowRelayInfo capability-doc types + serde"`

### Task 1.3: JCS canonicalization (two forms) + ETag

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn canonical_is_field_order_independent() {
    // Two JSON strings, same logical doc, different key order → identical canonical bytes.
    let a: WillowRelayInfo = serde_json::from_str(
        r#"{"pubkey":"aa","protocol_versions":[2],"signature":"sig"}"#).unwrap();
    let b: WillowRelayInfo = serde_json::from_str(
        r#"{"signature":"sig","protocol_versions":[2],"pubkey":"aa"}"#).unwrap();
    assert_eq!(canonical_json(&a, true).unwrap(), canonical_json(&b, true).unwrap());
}

#[test]
fn canonical_signed_excludes_signature() {
    let mut info = sample();
    info.signature = "DEADBEEF".into();
    let signed_form = canonical_json(&info, false).unwrap();
    assert!(!String::from_utf8_lossy(&signed_form).contains("DEADBEEF"));
    assert!(String::from_utf8_lossy(&canonical_json(&info, true).unwrap()).contains("DEADBEEF"));
}

#[test]
fn etag_is_deterministic_sha256_hex() {
    let bytes = canonical_json(&sample(), true).unwrap();
    let e1 = capability_etag(&bytes);
    assert_eq!(e1, capability_etag(&bytes));
    assert_eq!(e1.len(), 64); // SHA-256 hex
}
```

- [ ] **Step 2: Run to verify it fails** — `cargo test -p willow-common relay_info::tests::canonical` → FAIL.
- [ ] **Step 3: Implement** in `crates/common/src/relay_info.rs`

```rust
use sha2::{Digest, Sha256};

/// RFC 8785 (JCS) canonical bytes of the document. When `include_signature`
/// is false, the `signature` field is removed first (the bytes the signature
/// covers, `CANON_SIGNED`); when true, the signature is included (`CANON_ETAG`).
pub fn canonical_json(info: &WillowRelayInfo, include_signature: bool) -> Result<Vec<u8>, CapabilityError> {
    let mut value = serde_json::to_value(info).map_err(|e| CapabilityError::Canon(e.to_string()))?;
    if !include_signature {
        if let Some(obj) = value.as_object_mut() { obj.remove("signature"); }
    }
    serde_jcs::to_vec(&value).map_err(|e| CapabilityError::Canon(e.to_string()))
}

/// Strong ETag = lowercase hex SHA-256 over the `CANON_ETAG` form
/// (canonical JSON with `signature` included).
pub fn capability_etag(canonical_etag_bytes: &[u8]) -> String {
    let digest = Sha256::digest(canonical_etag_bytes);
    hex::encode(digest)
}
```

- [ ] **Step 4: Run to verify it passes** — `cargo test -p willow-common relay_info` → PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(common): JCS canonicalization + SHA-256 ETag for capability doc"`

### Task 1.4: Sign + verify (Ed25519 over `CANON_SIGNED`)

> `willow-common` already depends on `willow-identity` (it imports `Identity` in `wire.rs`). Use `Identity::sign(&[u8]) -> Vec<u8>` and `Identity::verify(data, sig) -> bool` (`crates/identity/src/lib.rs`). Verification by raw pubkey hex uses the identity crate's verifying-key API — confirm the exact helper when implementing; if only `Identity::verify` (self-keyed) exists, add a free function `verify_detached(pubkey_hex, data, sig_hex) -> bool` in `willow-identity` reconstructing an `ed25519_dalek::VerifyingKey` from the hex pubkey.

- [ ] **Step 1: Write the failing test**

```rust
use willow_identity::Identity;

#[test]
fn sign_then_verify_roundtrip() {
    let id = Identity::generate();
    let mut info = sample();
    info.pubkey = hex::encode(id.endpoint_id().as_bytes()); // exact accessor: see endpoint_id()
    sign_capability_doc(&mut info, &id).unwrap();
    assert_eq!(info.signature.len(), 128); // 64-byte Ed25519 sig as hex
    assert!(verify_capability_doc(&info).unwrap());
}

#[test]
fn tampering_breaks_verification() {
    let id = Identity::generate();
    let mut info = sample();
    info.pubkey = hex::encode(id.endpoint_id().as_bytes());
    sign_capability_doc(&mut info, &id).unwrap();
    info.status = Some("read_only".into()); // attacker flips a field
    assert!(!verify_capability_doc(&info).unwrap());
}
```

- [ ] **Step 2: Run to verify it fails** — `cargo test -p willow-common relay_info::tests::sign` → FAIL.
- [ ] **Step 3: Implement**

```rust
/// Sign the document in place: canonicalize without the signature, sign with
/// the relay's Ed25519 key, store lowercase hex in `signature`.
pub fn sign_capability_doc(info: &mut WillowRelayInfo, identity: &willow_identity::Identity)
    -> Result<(), CapabilityError>
{
    let bytes = canonical_json(info, false)?;
    info.signature = hex::encode(identity.sign(&bytes));
    Ok(())
}

/// Verify the detached signature against the document's own `pubkey` field.
/// A document whose signature does not verify MUST be treated as a 404 by callers.
pub fn verify_capability_doc(info: &WillowRelayInfo) -> Result<bool, CapabilityError> {
    if info.pubkey.is_empty() { return Err(CapabilityError::MissingField("pubkey")); }
    if info.signature.is_empty() { return Err(CapabilityError::MissingField("signature")); }
    let bytes = canonical_json(info, false)?;
    let sig = hex::decode(&info.signature).map_err(|_| CapabilityError::BadSignature)?;
    let pk = hex::decode(&info.pubkey).map_err(|_| CapabilityError::BadSignature)?;
    Ok(willow_identity::verify_detached(&pk, &bytes, &sig)) // add this helper if absent
}
```

- [ ] **Step 4: Run to verify it passes** — `cargo test -p willow-common relay_info` → PASS.
- [ ] **Step 5: Run full crate checks** — `just test-crate willow-common && just clippy`. Expected: zero warnings.
- [ ] **Step 6: Commit** — `git commit -am "feat(common): Ed25519 sign/verify for capability doc"`

### PR 1 self-review
- Spec coverage: field schema ✓, JCS two forms ✓, signing MUST ✓, ETag from `CANON_ETAG` ✓, ignore-unknown-fields ✓, feature-tag table ✓. CORS/caching/dispatch are PR 2 (endpoint).
- Type consistency: `canonical_json(info, include_signature: bool)`, `capability_etag(bytes)`, `sign_capability_doc(&mut, &id)`, `verify_capability_doc(&info)` are the names every later task uses.

---

## PR 2 — Capability endpoint: dispatch + GET/OPTIONS handlers + CORS

**Spec:** `specs/2026-04-24-relay-capability-doc.md`
**Depends on:** PR 1. Relay-crate-only network code; no client/state/worker impact.

**Files:**
- Modify: `crates/relay/src/lib.rs` (rename constant; add path constants, dispatch branch, two handlers)
- Modify: `crates/relay/src/main.rs` (build + sign `WillowRelayInfo` at startup; thread pre-rendered JSON+ETag into the listener; add operator-metadata CLI args)
- Modify: `crates/relay/Cargo.toml` (depend on `willow-common`, `serde_json`)
- Create: `crates/relay/tests/capability_endpoint.rs` (mirrors `bootstrap_endpoint.rs`)

**Verified current state (re-confirm line numbers before editing):**
- `dispatch_connection()` in `crates/relay/src/lib.rs` routes `GET /bootstrap-id` to a local handler, everything else proxies to the upstream iroh-relay.
- `request_line_matches_bootstrap_id()` + `handle_bootstrap_request_after_line()` (emits ACAO only, text/plain, no OPTIONS).
- `MAX_CONCURRENT_BOOTSTRAP_CONNECTIONS` gates the **whole** proxy listener (stale name).
- Relay `Identity` loaded in `crates/relay/src/main.rs`; `--relay-port` arg present.

### Decisions for PR 2 (resolved from spec open questions)
- **`protocol_versions` value:** advertise `vec![willow_transport::PROTOCOL_VERSION]` = `vec![2]`. The spec's `[1]` examples are placeholders predating the bump. Source from the live constant.
- **`status` in v1:** static `"ok"` (operator-overridable via CLI). Dynamic `degraded`/`read_only` health-check wiring to workers is deferred (spec lists `degraded` as SHOULD). Document that two-tier `Cache-Control` is therefore effectively single-tier until dynamic status lands.
- **Operator metadata source:** optional CLI args (`--relay-name`, `--relay-contact`, `--relay-description`, `--relay-tos`, …) with env-var fallback, defaulting to `None`. Matches the existing `--relay-port` pattern; no config-file parser.
- **Build-once:** construct + sign `WillowRelayInfo` at startup, pre-render `(info_json, etag)`, and serve cached copies — no per-request crypto.

### Task 2.1: Rename the stale semaphore constant
- [ ] **Step 1:** Rename `MAX_CONCURRENT_BOOTSTRAP_CONNECTIONS` → `MAX_CONCURRENT_PROXY_CONNECTIONS` in `crates/relay/src/lib.rs` and update all references (`lib.rs`, `main.rs`). `grep -rn MAX_CONCURRENT_BOOTSTRAP_CONNECTIONS` first; also `grep -rn` it in `docker/`, deploy configs, and docs.
- [ ] **Step 2: Verify** — `cargo check -p willow-relay`. Expected: PASS.
- [ ] **Step 3: Commit** — `git commit -am "refactor(relay): rename proxy-connection semaphore constant"`

### Task 2.2: Integration test for GET (write first — TDD)
- [ ] **Step 1: Write the failing test** in `crates/relay/tests/capability_endpoint.rs`, reusing the harness pattern from `bootstrap_endpoint.rs` (`spawn_listener_with_capacity`, write raw HTTP, `read_full_response`). Assert: `GET /.well-known/willow` → `200`; `Content-Type: application/willow+json; charset=utf-8`; headers `Access-Control-Allow-Origin: *`, `Access-Control-Allow-Methods: GET, OPTIONS`, `Access-Control-Allow-Headers: Accept, Content-Type, If-None-Match`; an `ETag` header; body parses as `WillowRelayInfo` and `verify_capability_doc()` returns `true`.
- [ ] **Step 2: Run to verify it fails** — `cargo test -p willow-relay --test capability_endpoint` → FAIL (404 from upstream proxy, no branch yet).

### Task 2.3: Path constants + matcher + GET handler
- [ ] **Step 1:** Add constants after `BOOTSTRAP_DRAIN_BUFFER_CAP`: `const CAPABILITY_PATH: &str = "/.well-known/willow";` and `const CAPABILITY_CONTENT_TYPE: &str = "application/willow+json; charset=utf-8";`.
- [ ] **Step 2:** Add `fn request_line_matches_capability_doc(line: &[u8]) -> Option<Method>` after `request_line_matches_bootstrap_id()` returning whether the request line is `GET`/`OPTIONS` on `CAPABILITY_PATH`.
- [ ] **Step 3:** Add `async fn handle_capability_request_after_line(client, already_read, info_json: &str, etag: &str)` modeled on `handle_bootstrap_request_after_line` (reuse `BOOTSTRAP_IO_TIMEOUT`, the drain logic, and `BOOTSTRAP_DRAIN_BUFFER_CAP`). Parse `If-None-Match`: if it equals `etag`, write `304 Not Modified` (no body); else write `200` with `Content-Type: application/willow+json; charset=utf-8`, `ETag: "<etag>"`, the three CORS headers, `Cache-Control: public, max-age=300` (steady-state `ok`), `Connection: close`, then the JSON body.
- [ ] **Step 4: Run** the GET test from Task 2.2 → it still fails until dispatch is wired (Task 2.5). That's expected; proceed.

### Task 2.4: OPTIONS preflight handler + its test
- [ ] **Step 1: Write the failing test:** `OPTIONS /.well-known/willow` → `204`, empty body, all three CORS headers present.
- [ ] **Step 2:** Add `async fn handle_capability_options(client, already_read)` writing `204 No Content` with ACAO/ACAM/ACAH and `Connection: close`.

### Task 2.5: Wire the dispatch branch
- [ ] **Step 1:** In `dispatch_connection()`, **before** the bootstrap-id check, match the parsed request line: `OPTIONS /.well-known/willow` → `handle_capability_options(...)` and early-return; `GET /.well-known/willow` → `handle_capability_request_after_line(..., info_json, etag)` and early-return. Then the existing bootstrap-id branch; then the upstream fallthrough.
- [ ] **Step 2:** Thread `info_json: Arc<str>` and `etag: Arc<str>` from `run_proxy_listener()` (built in `main.rs`) down into `dispatch_connection`.
- [ ] **Step 3: Run** Tasks 2.2 + 2.4 tests → PASS.
- [ ] **Step 4: Commit** — `git commit -am "feat(relay): serve signed /.well-known/willow capability doc"`

> **Risk to check (from analysis):** the relay proxies *all* non-bootstrap traffic upstream. iroh-relay uses `/relay` and `/ping`; confirm no collision with `/.well-known/willow`. Add a test asserting an unrelated path (e.g. `GET /relay`) still proxies upstream unchanged.

### Task 2.6: Build + sign the doc at startup
- [ ] **Step 1:** Add optional CLI args + env fallback for operator metadata to `main.rs` (mirroring `--relay-port`).
- [ ] **Step 2:** After the identity is loaded, construct `WillowRelayInfo` (pubkey = hex of `identity.endpoint_id()`, `protocol_versions = vec![willow_transport::PROTOCOL_VERSION]`, `supported_features = vec![features::GOSSIP.into(), features::HISTORY.into(), features::BLOBS.into()]` — add `SEQ_VECTOR_SYNC`/`HISTORY_EOSE` only after PRs 4/5 land; `status = Some("ok")`, `limitation`/`retention` from the live relay constants). Call `sign_capability_doc(&mut info, &identity)`, then `serde_json::to_string(&info)` and `capability_etag(&canonical_json(&info, true)?)`. Pass both into `run_proxy_listener`.
- [ ] **Step 3: Integration test:** `If-None-Match` replay with the served `ETag` → `304`.
- [ ] **Step 4: Verify end-to-end** — `just test-relay && just clippy && just fmt`. Expected: zero warnings.
- [ ] **Step 5: Commit** — `git commit -am "feat(relay): build + sign capability doc from operator config at startup"`

### Task 2.7: Browser test (client pre-connect mismatch banner)
- [ ] **Step 1:** In `crates/web/tests/browser.rs`, stub `fetch` to return a doc whose `protocol_versions` does not intersect the client's; mount the connect flow; assert the connect button is disabled and a version-mismatch banner renders. (This exercises `fetch_relay_info` only if it exists yet; if the web connect flow does not consume the doc until PR 6, scope this test to PR 6 instead and note it here.)

### PR 2 self-review
- Spec coverage: dispatch surgery ✓, GET 200 + content-type ✓, OPTIONS 204 ✓, CORS ACAO/ACAM/ACAH ✓, ETag + `If-None-Match` 304 ✓, `Cache-Control` ✓, sign-at-startup ✓, constant rename ✓. Dynamic `degraded` status explicitly deferred + documented.

---

## PR 3 — Heads-based sync: wire types + `SyncFilter` + state/storage query helpers

**Spec:** `specs/2026-04-24-negentropy-sync.md`
**Depends on:** nothing. Pure types + query logic + serde; no live listener rewiring.

**Files:**
- Modify: `crates/common/src/wire.rs` (two additive `WireMessage` variants + `SyncFilter`; extend `max_size`/`variant_name` arms)
- Modify: `crates/state/src/dag.rs` (`events_since` gains optional `SyncFilter`; add `known_authors`)
- Modify: `crates/state/src/sync.rs` (filter helper + tests)
- Modify: `crates/storage/src/store.rs` (`known_authors(server_id)`; restructure `sync_since`; new index migration)
- Modify: `crates/transport/src/lib.rs` (reserve-slot comment only — decision 1)

**Verified current state:** `WireMessage` (`crates/common/src/wire.rs`) has `Event`, `SyncRequest { state_hash, topic }`, `SyncBatch { events }`, `Worker(..)`, plus `max_size()`/`variant_name()` methods. `HeadsSummary { heads: BTreeMap<EndpointId, AuthorHead{seq,hash}> }` + `compare_chains` in `crates/state/src/sync.rs`. `EventDag::heads_summary()` + `events_since(their_heads, limit)` in `crates/state/src/dag.rs`. `StorageEventStore::sync_since(server_id, heads)` + `idx_events_author_seq` in `crates/storage/src/store.rs`. `WorkerResponse::SyncBatch { events }` (no `more`) in `crates/common/src/worker_types.rs`. `SYNC_BATCH_LIMIT` in `crates/common/src/lib.rs`. Gossip cap `max_message_size(65536)` in `crates/network/src/iroh.rs`.

**New types** (`crates/common/src/wire.rs`):

```rust
// Additive — legacy SyncRequest/SyncBatch stay untouched (decision 2).
SyncRequestV2 { request_id: String, heads: willow_state::HeadsSummary, filter: SyncFilter },
SyncBatchV2   { request_id: String, events: Vec<willow_state::Event>, more: bool },

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncFilter {
    pub server_id: String,                              // required (DAG genesis hash hex)
    pub channels: Option<Vec<String>>,                 // narrows chat-shaped kinds only
    pub authors: Option<Vec<willow_identity::EndpointId>>,
    pub event_kinds: Option<Vec<u8>>,                  // EventKind discriminant bytes
    pub since_ms: Option<u64>,                          // advisory pre-filter only
}
```

### Task 3.1: Reserve MessageType slots (comment only)
- [ ] **Step 1:** Add a doc comment above `MessageType` in `crates/transport/src/lib.rs`: `// Reserved for future promotion (do not allocate now): 7 = HistorySyncComplete/EOSE, 8 = Sync. See docs/plans/2026-05-28-relay-upgrade-bundle.md.`
- [ ] **Step 2: Commit** — `git commit -am "docs(transport): reserve MessageType slots 7/8 for future sync promotion"`

### Task 3.2: Wire variants + `SyncFilter` round-trip (incl. legacy still works)
- [ ] **Step 1: Write the failing test** in `crates/common/src/wire.rs` tests module: round-trip `SyncRequestV2`/`SyncBatchV2` via `pack_wire`/`unpack_wire`; **assert the legacy `SyncRequest`/`SyncBatch` still round-trip** (existing round-trip tests must keep passing); assert a serialized `SyncBatchV2` envelope holding ~N events stays ≤ 64 KiB.
- [ ] **Step 2: Run to verify it fails** — `cargo test -p willow-common wire` → FAIL.
- [ ] **Step 3:** Add the two variants + `SyncFilter` struct; add their `max_size()` arms (~65,300 bytes) and `variant_name()` arms.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(common): additive SyncRequestV2/SyncBatchV2 + SyncFilter wire types"`

### Task 3.3: Empirical framing-budget helper (measure-then-encode)
- [ ] **Step 1: Write a test** in `crates/common/src/wire.rs` that builds a real `SyncBatchV2` with one representative `Event`, packs it with `pack_wire`, and measures total framing overhead (`Envelope` + `WireMessage` tag + `SignedMessage` 32B key + 64B sig + length prefixes). Assert overhead < 200 bytes; bind `pub const SYNC_ENVELOPE_BUDGET: usize = 65_536 - 256;` and assert a greedily-packed batch never exceeds it after `pack_wire`.
- [ ] **Step 2:** Implement an incremental size accumulator (add each event's own serialized length) to decide when to start a new batch — **not** a per-candidate whole-batch re-serialization (O(n²)).
- [ ] **Step 3: Run** → PASS. **Commit** — `git commit -am "feat(common): empirical 64KiB sync-envelope byte budget"`

### Task 3.4: `EventDag` filter + `known_authors`
- [ ] **Step 1: Write failing tests** in `crates/state/src/sync.rs`: `events_since` with a `SyncFilter` respects `channels`/`authors`/`event_kinds`/`since_ms`; structural events (`GrantPermission`, `CreateChannel`, `RotateChannelKey`) ignore the `channels` filter; `known_authors()` returns every author in `chains`. Keep the existing `events_since_unknown_author` test green.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3:** Extend `EventDag::events_since` to accept `Option<&SyncFilter>` (or add `events_since_filtered`), and add `EventDag::known_authors(&self) -> Vec<EndpointId>` from `self.chains.keys()`.
- [ ] **Step 4: Run** → PASS. **Commit** — `git commit -am "feat(state): SyncFilter-aware events_since + known_authors"`

### Task 3.5: Storage query restructure + index migration
- [ ] **Step 1: Write failing tests** in `crates/storage/src/store.rs` tests: `known_authors(server_id)` via `SELECT DISTINCT author`; restructured `sync_since` emitting `(author = ? AND seq > ?)` per known author returns identical results to the old `author NOT IN (...)` path (port the existing `sync_since` tests and add the new shape).
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3:** Add migration `CREATE INDEX idx_events_server_author_seq ON events(server_id, author, seq);` (new migration number; do **not** drop `idx_events_author_seq` in the same migration — reversibility). Add `known_authors`. Restructure `sync_since` to enumerate locally-known-but-unmentioned authors up front and emit `(author = ? AND seq > 0)` for them, so every disjunct is a per-`(server, author)` range scan on the new index.
- [ ] **Step 4: Run** → `just test-crate willow-storage` PASS. **Commit** — `git commit -am "feat(storage): server-author-seq index + per-author sync_since restructure"`

### PR 3 self-review
- Spec coverage: additive variants ✓, `SyncFilter` ✓, per-author tail query helpers (`events_since`/`known_authors`) ✓, storage index + restructure ✓, 64 KiB budget measured ✓, legacy variants intact ✓. Responder/receiver wiring + `SyncProvider` gate are PR 4.

---

## PR 4 — Heads-based sync: client + worker responder/receiver cutover + `SyncProvider` gate

**Spec:** `specs/2026-04-24-negentropy-sync.md`
**Depends on:** PR 3.

**Files:**
- Modify: `crates/client/src/listeners.rs` (responder for `SyncRequestV2`; receiver for `SyncBatchV2` with `request_id` correlation + `more:false` termination; **replace** the legacy 500-event dump but **keep** a legacy `SyncRequest` responder for the migration window; keep `SYNC_BATCH_LIMIT`/`MAX_SYNC_BATCH_SIZE` as defense-in-depth)
- Modify: `crates/common/src/worker_types.rs` (`WorkerResponse::SyncBatch` gains `more: bool`)
- Modify: `crates/worker/src/actors/sync.rs`, `crates/replay/src/role.rs`, `crates/storage/src/role.rs` — byte-budgeted streaming + `more` flag
- Modify: `crates/client/src/listeners.rs` (responder `SyncProvider` gate — final step)

### Task 4.1: `more` field on the worker response (additive)
- [ ] **Step 1: Write failing test** in `crates/common/src/worker_types.rs`: `WorkerResponse::SyncBatch { events, more }` round-trips.
- [ ] **Step 2:** Add `more: bool` to the variant; update all construction sites in `worker`/`replay`/`storage` to set `more: false` initially (compile-driven).
- [ ] **Step 3: Run** → PASS. **Commit** — `git commit -am "feat(common): WorkerResponse::SyncBatch gains more flag"`

> **Risk (verify):** adding a field to a bincode enum variant changes its layout. Confirm there is no persisted/cross-version `WorkerResponse` on disk; if a worker and client at different versions can exchange it during rollout, document the worker upgrade ordering.

### Task 4.2: Client responder for `SyncRequestV2`
- [ ] **Step 1: Write failing integration test** in `crates/client/src/tests/` against `willow_network::mem::MemNetwork`: peer A holds authors `{x:1..100}`, peer B sends `SyncRequestV2` with empty heads; B receives the full chain across one-or-more `SyncBatchV2` envelopes ending `more:false`; B's DAG converges to A's.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3:** Implement the responder: compute the delta via the PR 3 `events_since(filter)` over the requester's `heads`, greedily pack into `SyncBatchV2` using `SYNC_ENVELOPE_BUDGET`, set `more:true` on all but the last, `more:false` on the last; reuse the request's `request_id`.
- [ ] **Step 4: Run** → PASS. **Commit**.

### Task 4.3: Client receiver + termination + legacy replacement
- [ ] **Step 1: Write failing tests:** three-peer convergence (C empty syncs from A then B → both chains complete); edge cases (empty store; requester up-to-date → single `more:false` zero-event batch; single missing event; author unknown to requester); authority events (grant/kick/create-channel) sync without special-casing.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3:** Implement the receiver: buffer batches by `request_id`, apply each event via the existing `try_insert_event` (`crates/client/src/listeners.rs`), detect `more:false` termination. **Replace** the legacy 500-event topological dump with a `SyncRequestV2` emission on join; **keep** a handler that still answers an inbound legacy `SyncRequest` with the old 500-event response (migration window). Keep the receiver-side count cap as documented defense-in-depth.
- [ ] **Step 4: Run** → PASS. **Commit**.

### Task 4.4: Worker / replay / storage streaming
- [ ] **Step 1: Write failing tests** (`just test-workers`): replay and storage `Sync` arms stream byte-budgeted batches with correct `more` flags; final batch `more:false`.
- [ ] **Step 2:** Update `crates/worker/src/actors/sync.rs`, `crates/replay/src/role.rs`, `crates/storage/src/role.rs` to byte-budget batches (reuse `SYNC_ENVELOPE_BUDGET`) and set `more`. Demote `SYNC_BATCH_LIMIT` to a documented OOM guard.
- [ ] **Step 3: Run** → PASS. **Commit**.

### Task 4.5: `SyncProvider` responder gate (final step — decision 4)
- [ ] **Step 1: Write failing tests:** a peer **without** `SyncProvider` for the server refuses to serve a `SyncRequestV2` (responds with nothing / a typed refusal); a peer **with** the grant serves normally; requesters are never gated; the **legacy** `SyncRequest` responder stays **ungated**.
- [ ] **Step 2:** Gate only the `SyncRequestV2` responder path on the server-DAG check that the local peer holds `SyncProvider` (`crates/state/src/event.rs`).
- [ ] **Step 3: Run** → PASS.
- [ ] **Step 4: Full check** — `just test-client && just test-workers && just check`. Zero warnings.
- [ ] **Step 5: e2e** — add the byte-count diff assertion to `e2e/history-sync.spec.ts` (offline reconnect transfers only the diff). The "`HistorySyncComplete` fires" assertion belongs to PR 5 (conflict resolution). **Commit**.

### PR 4 self-review
- Spec coverage: gossip responder + receiver ✓, streaming `more` ✓, worker `more` ✓, legacy path retained ✓, `SyncProvider` gate ✓, byte budget reused ✓, MemNetwork convergence ✓. `request_id` is `String` everywhere (decision 8).

---

## PR 5 — History-sync EOSE: `HistorySyncComplete` marker + worker emission + client event

**Spec:** `specs/2026-04-24-history-sync-eose.md`
**Depends on:** PR 4 (the marker is meaningful only once streaming defines a clean end-of-stream). Worker provider classes only; **peer-to-peer marker is a documented follow-up.**

**Files:**
- Modify: `crates/common/src/wire.rs` (`HistorySyncComplete` variant)
- Modify: `crates/client/src/events.rs` (`ClientEvent::HistorySynced` — keep `SyncCompleted`)
- Modify: `crates/worker/src/runtime.rs` (name the SERVER_OPS sender, currently `_ops_sender`)
- Modify: `crates/replay/src/role.rs`, `crates/storage/src/role.rs` (emit marker after serving sync)
- Modify: `crates/client/src/listeners.rs` + `crates/client/src/lib.rs` (handle marker: trust-gate, dedup by `(provider, stream_generation)`, first-trusted-wins, `still_pending` count)
- Modify: `crates/web/src/...` (loading spinner hides on `HistorySynced`)
- Modify: `crates/relay/tests/` (relay forwards the marker unchanged)

**New types:**

```rust
// crates/common/src/wire.rs (additive WireMessage variant)
HistorySyncComplete {
    topic_id: [u8; 32],
    last_event_hash: Option<willow_state::EventHash>, // None = empty store (decision 7)
    stream_generation: u64,                            // random (decision 6)
},

// crates/client/src/events.rs
HistorySynced { topic: String, provider: willow_identity::EndpointId, still_pending: usize },
```

### Task 5.1: Wire variant round-trip
- [ ] **Step 1: Write failing test** (`crates/common/src/wire.rs`): `HistorySyncComplete` round-trips via `pack_wire`/`unpack_wire`; serialized size ~80 bytes (assert < 256 KB cap). Provider identity is **not** in the payload — it is the verified envelope signer.
- [ ] **Step 2:** Add the variant + `max_size()`/`variant_name()` arms. **Run** → PASS. **Commit**.

### Task 5.2: `ClientEvent::HistorySynced`
- [ ] **Step 1: Write failing test** (`crates/client/src/lib.rs` test module, MemNetwork): a marker from a `SyncProvider`-granted peer produces exactly one `HistorySynced` per `(topic, provider)`; a marker from an **untrusted** peer produces **no** event; reconnect with a new `stream_generation` re-emits, same generation does not; a `last_event_hash` mismatch logs a warning and does **not** produce a false completion.
- [ ] **Step 2:** Add the variant + handler (dedup map keyed by `(provider, stream_generation)`; first-trusted-wins; compute `still_pending` from connected trusted providers). **Run** → PASS. **Commit**.

### Task 5.3: Worker emission (SERVER_OPS broadcast handle)
- [ ] **Step 1: Write failing test** (`just test-workers`): after a replay/storage worker serves a `WorkerRequest::Sync`, it broadcasts exactly one `HistorySyncComplete` on `_willow_server_ops` with the last served event's hash (or `None`) and a per-run `stream_generation`; it sends at most one marker per neighbor per generation.
- [ ] **Step 2:** In `crates/worker/src/runtime.rs` rename `_ops_sender` to a real binding and thread it to the emitting actor. Emit after the `Sync` reply in `crates/replay/src/role.rs` and `crates/storage/src/role.rs`. **Run** → PASS.

> **Risk (from analysis):** SERVER_OPS was ingest-only for workers; broadcasting now risks a worker echoing its own server-ops events into a loop. Filter self-authored frames on the worker's SERVER_OPS receive path.

- [ ] **Step 3: Commit** — `git commit -am "feat(worker): broadcast HistorySyncComplete on SERVER_OPS after sync"`

### Task 5.4: Relay passthrough + browser spinner + feature tag
- [ ] **Step 1:** `crates/relay/tests/` — assert the relay forwards a `HistorySyncComplete`-bearing envelope unchanged (pins the content-agnostic contract against a future size filter).
- [ ] **Step 2:** `crates/web/tests/browser.rs` — the channel-history loading spinner hides on `HistorySynced` for the active topic and stays hidden across subsequent live events.
- [ ] **Step 3:** Advertise `features::HISTORY_EOSE` in the relay capability doc construction (PR 2 `main.rs`).
- [ ] **Step 4: Full check** — `just check`. **Commit**.
- [ ] **Step 5: e2e** — add the "`HistorySyncComplete` fires" assertion to `e2e/history-sync.spec.ts` (the assertion deferred from PR 4).

### PR 5 self-review
- Spec coverage: marker wire format ✓, provider-from-signer ✓, per-(topic,provider,generation) dedup ✓, first-trusted-wins + untrusted ignored ✓, `last_event_hash` truncation detection ✓, worker emission + SERVER_OPS handle ✓, relay passthrough ✓, spinner ✓. Peer-to-peer provider class explicitly deferred.

---

## PR 6 — Relay discovery: pkarr enablement + `JoinToken` bootstrap IDs + client ranking

**Spec:** `specs/2026-04-24-outbox-relay-discovery.md`
**Depends on:** PR 2 (the capability endpoint must exist for the client to fetch/rank). Highest external-API uncertainty → sequenced last so its risk blocks nothing else.

**Files:**
- Modify: `crates/network/src/iroh.rs` (enable pkarr discovery on the `Endpoint` builder, currently `empty_builder()`)
- Modify: `crates/client/src/ops.rs` (`JoinToken` gains `bootstrap_endpoint_ids: Vec<EndpointId>`, serde-defaulted)
- Modify: `crates/client/src/lib.rs` (`SyncProvider` grant enumeration; `fetch_relay_info(url)` consumer; pkarr→capability→trust ranking)
- Modify: `crates/web/src/app.rs` (`DEFAULT_RELAY_URL` stays as fallback; join flow resolves bootstrap IDs via pkarr first)

> **Correction to spec analysis:** iroh is already declared with `features = ["discovery-pkarr-dht"]` in the root `Cargo.toml`. PR 6 is **builder wiring**, not a Cargo feature change.

### Task 6.1: Verify the iroh 0.98.1 pkarr builder API (verify-then-call)
- [ ] **Step 1:** `cargo doc -p iroh --open` (or read the `iroh` source under `~/.cargo`) to find the exact builder method on iroh 0.98.1 — candidates: `Endpoint::builder().discovery_dht()`, `.discovery_n0()`, or `.add_discovery(...)`. The `discovery-pkarr-dht` feature gates `discovery_dht()`. **Write down the verified method name in the PR description.**
- [ ] **Step 2: Write a failing test** in `crates/network/src/iroh.rs` (or an integration test): construct an `IrohNetwork` and assert the endpoint reports a discovery service configured (use whatever introspection iroh exposes; if none, assert the builder call compiles and the path no longer uses bare `empty_builder()` without discovery).
- [ ] **Step 3:** Replace `Endpoint::empty_builder()` with the verified discovery-enabled builder; keep `Config::relay_url` as the fallback when DHT resolution fails (decision: fall back to configured relay, surface error only if both fail).
- [ ] **Step 4: Run** — `just test-crate willow-network`. **Commit** — `git commit -am "feat(network): enable iroh pkarr DHT discovery on the endpoint"`

> **Privacy note to add to the PR:** enabling DHT publish leaks relay/worker addresses to the BitTorrent mainline DHT. Operators in restricted/private deployments may want an opt-out; pkarr publication is opt-in per iroh defaults — document the behavior.

### Task 6.2: `JoinToken.bootstrap_endpoint_ids` (backward-compatible)
- [ ] **Step 1: Write failing test** in `crates/client/src/ops.rs`: a `JoinToken` with `bootstrap_endpoint_ids` round-trips through the base64 pack/unpack; an **old** token lacking the field still decodes (serde `#[serde(default)]`).
- [ ] **Step 2:** Add `#[serde(default)] pub bootstrap_endpoint_ids: Vec<EndpointId>` to `JoinToken`; update encode/decode.
- [ ] **Step 3: Run** → PASS. **Commit**.

> **Risk:** each `EndpointId` adds ~52 base64 chars to the share URL (~+150 for 3). Acceptable; note it. `bootstrap_endpoint_ids` signing by the inviter is deferred — `EndpointId` is self-certifying, so a tampered bootstrap list causes connection failure, not state forgery (document the threat model).

### Task 6.3: `SyncProvider` enumeration + `fetch_relay_info` + ranking
- [ ] **Step 1: Write failing tests** (`crates/client/src/lib.rs`): enumerate live (un-revoked) `SyncProvider` grants from a replayed DAG → correct `EndpointId` set (revoked excluded); `fetch_relay_info(url)` fetches + verifies signature + checks protocol intersection and **refuses** a doc whose signature fails or whose `protocol_versions` don't intersect; the ranker refuses to treat a non-`SyncProvider` endpoint as authoritative.
- [ ] **Step 2:** Implement `fetch_relay_info(url) -> Result<WillowRelayInfo>` in `willow-client` (reuses PR 1 `verify_capability_doc`); add `SyncProvider` grant enumeration; add latency-ranked worker selection (pkarr resolve → fetch capability doc → rank). This is the single definition reused by the web join flow.
- [ ] **Step 3: Run** → PASS. **Commit**.

### Task 6.4: Web join flow + e2e
- [x] **Step 1:** `crates/web/src/app.rs` — join flow resolves `bootstrap_endpoint_ids` via pkarr first, falls back to `DEFAULT_RELAY_URL`. Show progress during DHT resolution (latency is seconds). Landed via `resolve_bootstrap_peers` (token IDs lead, relay node always appended as fallback) + a `"resolving"` `join_status` rendered by `JoinPage`. The inviter side (`create_join_link`) was completed here too — it now populates `bootstrap_endpoint_ids` from live `SyncProvider` grants (deferred from Task 6.3, which only added the `sync_providers()` accessor). `ParsedJoinToken` threads the IDs through. See `docs/specs/2026-05-29-web-pkarr-join-flow-design.md`.
- [x] **Step 2: e2e** (`e2e/multi-peer-sync.spec.ts`): share-link join with `bootstrap_endpoint_ids`, fallback when the first bootstrap is unreachable (via the `prepend_unreachable_bootstrap` test-hook); + authority/provider change mid-session with continued convergence and **no** page reload. Write-only — these run in CI (real iroh/relay), not locally.
- [ ] **Step 3: Full check** — `just check-all` (PR gate, includes Playwright). **Commit**.

### PR 6 self-review
- Spec coverage: pkarr Layer 1 ✓, capability-doc Layer 2 consumed not duplicated ✓, `SyncProvider` Layer 3 ✓, `JoinToken` bootstrap IDs ✓, fallback policy ✓, no `EventKind::RelayList` ✓ (no state-machine change — none was added). Bootstrap-signing + privacy opt-out documented as deferred.

---

## Bundle-level self-review

**Spec coverage across all four specs:**
- `relay-capability-doc`: types/JCS/sign (PR 1), endpoint/CORS/cache/dispatch (PR 2), client fetch+verify (PR 6). ✓
- `negentropy-sync`: wire+`SyncFilter`+queries (PR 3), responder/receiver/worker cutover+gate (PR 4). ✓
- `history-sync-eose`: marker+worker emission+client event (PR 5); peer-to-peer provider class deferred per spec. ✓
- `outbox-relay-discovery`: pkarr+`JoinToken`+ranking (PR 6); no new `EventKind`, so no state-machine change. ✓

**Type consistency:** `canonical_json`/`capability_etag`/`sign_capability_doc`/`verify_capability_doc` (PR 1 → PR 2, PR 6); `SyncFilter`/`SyncRequestV2`/`SyncBatchV2`/`SYNC_ENVELOPE_BUDGET` (PR 3 → PR 4); `WorkerResponse::SyncBatch { more }` (PR 4); `HistorySyncComplete`/`HistorySynced` (PR 5); `JoinToken.bootstrap_endpoint_ids`/`fetch_relay_info` (PR 6). `request_id: String` everywhere.

**Deferred / out of scope (documented, not silently dropped):** legacy `SyncRequest`/`SyncBatch` variant removal (follow-up release); dynamic relay `degraded`/`read_only` status; `MessageType` slot promotion (7/8 reserved); ~~peer-to-peer `HistorySyncComplete` provider class~~ (**landed 2026-05-30, PR #664** — the gossip `SyncRequestV2` responder emits the marker after a successful serve; worker-only emission was unobservable by gossip clients); `JoinToken` inviter-signing; `SyncRequestV2.heads` chunking above ~900 authors; Negentropy/RBSR fallback for cross-author interior gaps; pkarr DHT publish opt-out.

## Execution

This plan is ready to execute **one PR at a time**, CI-green → merged → `main` rebased before the next (per the project's sequential-PR workflow). Recommended: **subagent-driven-development** — dispatch a fresh subagent per task with two-stage review between tasks, starting with PR 1 (pure, deterministic, unblocks the capability track).
