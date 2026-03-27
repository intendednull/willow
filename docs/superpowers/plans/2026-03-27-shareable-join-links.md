# Shareable Join Links Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add shareable URL-based join links that trigger automatic P2P key exchange when clicked, with a dedicated join page that serves as a welcoming front door to the app.

**Architecture:** A URL token (inviter PeerId + server ID + link ID + server name + inviter display name) is encoded in a URL fragment. When opened, the app renders a dedicated JoinPage — not the welcome screen. The join page shows the server name, inviter name, a name input, and a single "Join" button. Clicking it sends a `JoinRequest` over gossipsub. The inviter's app auto-generates a per-recipient encrypted invite and sends it back as a `JoinResponse`. The existing `accept_invite()` completes the join, and the page crossfades directly into the chat.

**Tech Stack:** Rust, Leptos 0.7, libp2p gossipsub, willow_transport serialization, localStorage persistence.

**Spec:** `docs/superpowers/specs/2026-03-27-shareable-join-links-design.md`

---

## UX Design Direction

### Join Page (joiner's first impression)

The join page is the **front door to the entire app**. It must feel intentional, calm, and welcoming — like someone opened the door for you.

**Layout:** Centered card on a subtle ambient background. No sidebar, no header, no chrome. Just the essentials.

**Visual hierarchy (top to bottom):**
1. Willow wordmark (small, understated — establishes brand)
2. Server name in large display weight (`IBM Plex Sans 600, ~28px`)
3. "Invited by **[name]**" in muted secondary text
4. "Your name" input field (pre-filled from saved profile if available)
5. "Join" button — full-width, accent color, generous padding

**States:**
- **Idle:** Button says "Join [Server Name]". Input focused on load.
- **Connecting:** Button text fades to "Connecting..." with a subtle pulse animation on the button border (accent glow). Input disabled. No spinner — the button IS the loading indicator.
- **Offline (after 15s):** Text below the button appears: "[Inviter] isn't online right now. We'll keep trying." Calm, honest. Button stays in connecting state. Retries silently with backoff.
- **Denied:** Button resets. Error text appears below: "This invite link is no longer valid." Red accent. Clear, not scary.
- **Success:** The entire join card fades out (opacity + scale down) while the chat view fades in behind it. No intermediate screen. ~300ms crossfade.

**Styling details:**
- Background: `var(--bg-main)` with a very subtle radial gradient using `var(--accent-glow)` at ~20% opacity, centered behind the card. Same technique as the call page ambient background.
- Card: `var(--bg-elevated)` with `var(--border)` border, `border-radius: 16px`, `var(--shadow-lg)`. Max-width 400px, centered vertically and horizontally.
- Button connecting state: `border: 2px solid var(--accent)` with a CSS keyframe that pulses the `box-shadow` between `0 0 0 0 var(--accent-glow)` and `0 0 0 8px transparent` (same rhythm as the call page live dot).
- Input: standard app input styling with `var(--bg-input)`, `var(--border)`, `font-family: inherit`.
- Transition to chat: use `opacity` and `transform: scale(0.98)` on the card with `transition: 0.3s cubic-bezier(0.4, 0, 0.2, 1)`. The chat view fades in simultaneously.

### Settings — Link Management (inviter side)

**Principle: Zero-friction default, power on demand.**

- A single prominent button: "Create Invite Link" — clicking it **immediately** generates a link with defaults (5 uses, no expiry) and copies to clipboard. A "Copied!" tooltip flashes for 1.5s (reuse existing `.copied-tooltip` animation).
- Below the button, a collapsed "Options" disclosure (`<details>`) with: max uses (number input), expiration (select: 1 hour, 24 hours, 7 days, never). These take effect on the **next** generated link. Most users never open this.
- Active links in a clean list below. Each row: uses badge ("3/5"), relative time ("2h ago"), expiration status, and a trash icon button. Uses the existing `.settings-section` card pattern.
- No link text shown — just the copy button. The URL is an implementation detail, not something users read.

**Styling:**
- The "Create Invite Link" button uses `var(--accent-green)` background (differentiated from accent blue which is "action" — green is "create/invite/positive").
- Uses badge: monospace pill, `var(--bg-input)` background, `var(--text-muted)` text.
- Expired links: struck-through, muted text, no trash button (auto-hidden).

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `crates/client/src/ops.rs` | Modify | `JoinToken` (with server_name + inviter_name), `JoinLink`, 3 new `WireMessage` variants |
| `crates/client/src/events.rs` | Modify | `JoinLinkResponse` and `JoinLinkDenied` ClientEvent variants |
| `crates/client/src/network.rs` | Modify | `NetworkEvent` variants, dispatch in WASM + native loops |
| `crates/client/src/lib.rs` | Modify | `create_join_link()`, `join_links()`, `delete_join_link()`, `send_join_request()`, `publish()`; handle JoinRequest in process_batch |
| `crates/client/src/storage.rs` | Modify | `save_join_links()` / `load_join_links()` |
| `crates/relay/src/lib.rs` | Modify | Add variants to relay's WireMessage + wildcard match arm |
| `crates/web/src/state.rs` | Modify | `join_token`, `join_status` signals |
| `crates/web/src/event_processing.rs` | Modify | Handle `JoinLinkResponse` / `JoinLinkDenied` |
| `crates/web/src/app.rs` | Modify | Parse `#join=` fragment, route to JoinPage before welcome/chat |
| `crates/web/src/components/join_page.rs` | Create | Full-screen join page (the front door) |
| `crates/web/src/components/settings.rs` | Modify | "Invite Links" section on Server tab |
| `crates/web/src/components/mod.rs` | Modify | Export `JoinPage` |
| `crates/web/style.css` | Modify | Join page styles, link management styles |

---

### Task 1: Wire Protocol — JoinToken, JoinLink, WireMessage

**Files:**
- Modify: `crates/client/src/ops.rs`

- [ ] **Step 1: Write failing tests**

Add to the existing `mod tests` in `ops.rs`:

```rust
#[test]
fn join_token_round_trip() {
    let token = JoinToken {
        inviter_peer_id: "12D3KooWTest".to_string(),
        server_id: "srv-1".to_string(),
        link_id: "link-abc".to_string(),
        server_name: "My Server".to_string(),
        inviter_name: "Alice".to_string(),
    };
    let encoded = token.encode();
    let decoded = JoinToken::decode(&encoded).unwrap();
    assert_eq!(decoded.inviter_peer_id, "12D3KooWTest");
    assert_eq!(decoded.server_id, "srv-1");
    assert_eq!(decoded.link_id, "link-abc");
    assert_eq!(decoded.server_name, "My Server");
    assert_eq!(decoded.inviter_name, "Alice");
}

#[test]
fn join_token_decode_invalid_returns_none() {
    assert!(JoinToken::decode("not-valid!@#$").is_none());
    assert!(JoinToken::decode("").is_none());
}

#[test]
fn join_link_is_valid_active_under_limit() {
    let link = JoinLink {
        link_id: "l1".into(), server_id: "s1".into(),
        max_uses: 5, used: 2, active: true, expires_at: None,
    };
    assert!(link.is_valid());
}

#[test]
fn join_link_is_valid_max_uses_reached() {
    let link = JoinLink {
        link_id: "l1".into(), server_id: "s1".into(),
        max_uses: 5, used: 5, active: true, expires_at: None,
    };
    assert!(!link.is_valid());
}

#[test]
fn join_link_is_valid_inactive() {
    let link = JoinLink {
        link_id: "l1".into(), server_id: "s1".into(),
        max_uses: 5, used: 0, active: false, expires_at: None,
    };
    assert!(!link.is_valid());
}

#[test]
fn join_link_is_valid_expired() {
    let link = JoinLink {
        link_id: "l1".into(), server_id: "s1".into(),
        max_uses: 5, used: 0, active: true, expires_at: Some(1),
    };
    assert!(!link.is_valid());
}

#[test]
fn wire_message_join_request_round_trip() {
    let id = Identity::generate();
    let msg = WireMessage::JoinRequest {
        link_id: "link-1".to_string(),
        peer_id: "12D3KooWJoiner".to_string(),
    };
    let data = pack_wire(&msg, &id).unwrap();
    let (decoded, signer) = unpack_wire(&data).unwrap();
    assert_eq!(signer, id.peer_id());
    match decoded {
        WireMessage::JoinRequest { link_id, peer_id } => {
            assert_eq!(link_id, "link-1");
            assert_eq!(peer_id, "12D3KooWJoiner");
        }
        _ => panic!("expected JoinRequest"),
    }
}

#[test]
fn wire_message_join_response_round_trip() {
    let id = Identity::generate();
    let msg = WireMessage::JoinResponse {
        target_peer: "12D3KooWJoiner".to_string(),
        invite_data: "base64inviteblob".to_string(),
    };
    let data = pack_wire(&msg, &id).unwrap();
    let (decoded, _) = unpack_wire(&data).unwrap();
    match decoded {
        WireMessage::JoinResponse { target_peer, invite_data } => {
            assert_eq!(target_peer, "12D3KooWJoiner");
            assert_eq!(invite_data, "base64inviteblob");
        }
        _ => panic!("expected JoinResponse"),
    }
}

#[test]
fn wire_message_join_denied_round_trip() {
    let id = Identity::generate();
    let msg = WireMessage::JoinDenied {
        target_peer: "12D3KooWJoiner".to_string(),
        reason: "link_expired".to_string(),
    };
    let data = pack_wire(&msg, &id).unwrap();
    let (decoded, _) = unpack_wire(&data).unwrap();
    match decoded {
        WireMessage::JoinDenied { target_peer, reason } => {
            assert_eq!(target_peer, "12D3KooWJoiner");
            assert_eq!(reason, "link_expired");
        }
        _ => panic!("expected JoinDenied"),
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p willow-client -- join_token`
Expected: FAIL — types don't exist.

- [ ] **Step 3: Implement types and variants**

Add before the `WireMessage` enum in `ops.rs`:

```rust
/// Token embedded in a shareable join URL. Contains enough
/// context to show the user what they're joining before connecting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinToken {
    pub inviter_peer_id: String,
    pub server_id: String,
    pub link_id: String,
    /// Human-readable server name for the join page.
    pub server_name: String,
    /// Display name of whoever generated the link.
    pub inviter_name: String,
}

impl JoinToken {
    /// Encode to a URL-safe base64 string.
    pub fn encode(&self) -> String {
        let bytes = willow_transport::pack(self).unwrap_or_default();
        crate::base64::encode(&bytes)
    }

    /// Decode from a base64 string.
    pub fn decode(s: &str) -> Option<Self> {
        let bytes = crate::base64::decode(s)?;
        willow_transport::unpack(&bytes).ok()
    }
}

/// Metadata for a generated join link, stored locally by the inviter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinLink {
    pub link_id: String,
    pub server_id: String,
    pub max_uses: u32,
    pub used: u32,
    pub active: bool,
    /// Timestamp in ms. None = never expires.
    pub expires_at: Option<u64>,
    /// When this link was created (ms since epoch).
    pub created_at: u64,
}

impl JoinLink {
    /// Check if this link can accept another join.
    pub fn is_valid(&self) -> bool {
        if !self.active || self.used >= self.max_uses {
            return false;
        }
        if let Some(expires) = self.expires_at {
            if crate::util::current_time_ms() > expires {
                return false;
            }
        }
        true
    }
}
```

Add three variants to the `WireMessage` enum:

```rust
    /// A peer is requesting to join via a shareable link.
    JoinRequest {
        link_id: String,
        peer_id: String,
    },
    /// The inviter's response with an encrypted invite for the requester.
    JoinResponse {
        target_peer: String,
        invite_data: String,
    },
    /// The inviter denied the join request.
    JoinDenied {
        target_peer: String,
        reason: String,
    },
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p willow-client -- join_token && cargo test -p willow-client -- join_link && cargo test -p willow-client -- wire_message_join`
Expected: All PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/client/src/ops.rs
git commit -m "feat: add JoinToken, JoinLink types and JoinRequest/Response/Denied wire messages"
```

---

### Task 2: Storage — Persist Join Links

**Files:**
- Modify: `crates/client/src/storage.rs`

- [ ] **Step 1: Add save/load functions**

Follow the existing `save_profile` / `load_profile` pattern:

```rust
/// Persisted join link data for a server.
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct SavedJoinLinks(pub Vec<crate::ops::JoinLink>);

pub fn save_join_links(server_id: &str, links: &[crate::ops::JoinLink]) {
    let saved = SavedJoinLinks(links.to_vec());
    if let Ok(bytes) = willow_transport::pack(&saved) {
        save_raw(&format!("join_links_{server_id}"), &bytes);
    }
}

pub fn load_join_links(server_id: &str) -> Vec<crate::ops::JoinLink> {
    load_raw(&format!("join_links_{server_id}"))
        .and_then(|bytes| willow_transport::unpack::<SavedJoinLinks>(&bytes).ok())
        .map(|s| s.0)
        .unwrap_or_default()
}
```

- [ ] **Step 2: Verify**

Run: `cargo test -p willow-client`
Expected: All pass.

- [ ] **Step 3: Commit**

```bash
git add crates/client/src/storage.rs
git commit -m "feat: add join link persistence"
```

---

### Task 3: Network Events — Dispatch

**Files:**
- Modify: `crates/client/src/network.rs`
- Modify: `crates/client/src/events.rs`

- [ ] **Step 1: Add NetworkEvent variants**

In `network.rs`, add to `NetworkEvent`:

```rust
    /// A peer wants to join via a shareable link.
    JoinLinkRequested { link_id: String, peer_id: String },
    /// A join link response was received (targeted at us).
    JoinLinkResponseReceived { invite_data: String },
    /// A join link request was denied.
    JoinLinkDenied { reason: String },
```

- [ ] **Step 2: Add ClientEvent variants**

In `events.rs`, add:

```rust
    /// A join-via-link response was received — auto-join can proceed.
    JoinLinkResponse { invite_data: String },
    /// A join-via-link request was denied.
    JoinLinkDenied { reason: String },
```

- [ ] **Step 3: Dispatch in WASM network loop**

In `run_network_wasm`'s WireMessage match (after VoiceSignal):

```rust
crate::ops::WireMessage::JoinRequest { link_id, peer_id } => {
    let _ = event_tx.unbounded_send(NetworkEvent::JoinLinkRequested { link_id, peer_id });
}
crate::ops::WireMessage::JoinResponse { target_peer, invite_data } => {
    if target_peer == local_peer_id {
        let _ = event_tx.unbounded_send(NetworkEvent::JoinLinkResponseReceived { invite_data });
    }
}
crate::ops::WireMessage::JoinDenied { target_peer, reason } => {
    if target_peer == local_peer_id {
        let _ = event_tx.unbounded_send(NetworkEvent::JoinLinkDenied { reason });
    }
}
```

- [ ] **Step 4: Same dispatch in native network loop**

Same three match arms in `run_network`.

- [ ] **Step 5: Build**

Run: `cargo build -p willow-client`
Expected: Clean.

- [ ] **Step 6: Commit**

```bash
git add crates/client/src/network.rs crates/client/src/events.rs
git commit -m "feat: add JoinLink network events and dispatch"
```

---

### Task 4: Client Logic — Link Creation, JoinRequest Handling

**Files:**
- Modify: `crates/client/src/lib.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn create_join_link_returns_token_with_server_info() {
    let (client, _rx) = test_client();
    let token_str = client.create_join_link(5, None).unwrap();
    let token = crate::ops::JoinToken::decode(&token_str).unwrap();
    assert_eq!(token.inviter_peer_id, client.peer_id());
    assert!(!token.link_id.is_empty());
    assert!(!token.server_name.is_empty());
}

#[test]
fn join_links_returns_created_links() {
    let (client, _rx) = test_client();
    assert!(client.join_links().is_empty());
    client.create_join_link(3, None).unwrap();
    let links = client.join_links();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].max_uses, 3);
    assert_eq!(links[0].used, 0);
    assert!(links[0].active);
}

#[test]
fn delete_join_link_removes_it() {
    let (client, _rx) = test_client();
    client.create_join_link(5, None).unwrap();
    let link_id = client.join_links()[0].link_id.clone();
    client.delete_join_link(&link_id);
    assert!(client.join_links().is_empty());
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p willow-client -- join_link`
Expected: FAIL.

- [ ] **Step 3: Add `join_links` to SharedState**

Add to `SharedState`:

```rust
    pub join_links: Vec<ops::JoinLink>,
```

Initialize in `ClientHandle::new()`:

```rust
    join_links: storage::load_join_links(first_server_id.as_deref().unwrap_or("")),
```

**Also update `test_client()`** — add `join_links: Vec::new()` to its `SharedState` construction.

- [ ] **Step 4: Add helper methods to ClientHandle**

```rust
    /// Publish raw data on a gossipsub topic.
    pub fn publish(&self, topic: &str, data: Vec<u8>) {
        let _ = self.cmd_tx.unbounded_send(network::NetworkCommand::Publish {
            topic: topic.to_string(),
            data,
        });
    }

    /// Send a JoinRequest for a link ID on the server ops topic.
    pub fn send_join_request(&self, link_id: &str) {
        let shared = self.shared.borrow();
        let msg = ops::WireMessage::JoinRequest {
            link_id: link_id.to_string(),
            peer_id: shared.identity.peer_id().to_string(),
        };
        if let Some(data) = ops::pack_wire(&msg, &shared.identity) {
            let _ = self.cmd_tx.unbounded_send(network::NetworkCommand::Publish {
                topic: ops::SERVER_OPS_TOPIC.to_string(),
                data,
            });
        }
    }

    /// Create a join link for the active server. Returns the encoded token string.
    /// Requires `CreateInvite` permission (owner has this implicitly).
    pub fn create_join_link(
        &self,
        max_uses: u32,
        expires_at: Option<u64>,
    ) -> anyhow::Result<String> {
        let mut shared = self.shared.borrow_mut();
        let server_id = shared.state.active_server.clone()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        // Permission check.
        let peer_id = shared.identity.peer_id().to_string();
        if !shared.state.event_state.has_permission(
            &peer_id,
            &willow_state::types::Permission::CreateInvite,
        ) {
            return Err(anyhow::anyhow!("missing CreateInvite permission"));
        }

        let server_name = shared.state.active().map(|c| c.server.name.clone()).unwrap_or_default();
        let inviter_name = shared.state.profiles.names.get(&peer_id).cloned().unwrap_or_default();

        let link = ops::JoinLink {
            link_id: uuid::Uuid::new_v4().to_string(),
            server_id: server_id.clone(),
            max_uses,
            used: 0,
            active: true,
            expires_at,
            created_at: util::current_time_ms(),
        };

        let token = ops::JoinToken {
            inviter_peer_id: peer_id,
            server_id,
            link_id: link.link_id.clone(),
            server_name,
            inviter_name,
        };

        shared.join_links.push(link);
        if shared.config.persistence {
            storage::save_join_links(
                shared.state.active_server.as_deref().unwrap_or(""),
                &shared.join_links,
            );
        }

        Ok(token.encode())
    }

    /// Return all join links for the active server.
    pub fn join_links(&self) -> Vec<ops::JoinLink> {
        self.shared.borrow().join_links.clone()
    }

    /// Delete a join link by ID.
    pub fn delete_join_link(&self, link_id: &str) {
        let mut shared = self.shared.borrow_mut();
        shared.join_links.retain(|l| l.link_id != link_id);
        if shared.config.persistence {
            storage::save_join_links(
                shared.state.active_server.as_deref().unwrap_or(""),
                &shared.join_links,
            );
        }
    }
```

- [ ] **Step 5: Handle JoinLinkRequested in process_batch**

In the `process_batch` method, add:

```rust
network::NetworkEvent::JoinLinkRequested { link_id, peer_id } => {
    let mut shared = self.shared.borrow_mut();
    let link = shared.join_links.iter_mut().find(|l| l.link_id == link_id);
    match link {
        Some(link) if link.is_valid() => {
            link.used += 1;
            tracing::info!(%link_id, used = link.used, max = link.max_uses, %peer_id, "processing join link request");
            if shared.config.persistence {
                storage::save_join_links(
                    shared.state.active_server.as_deref().unwrap_or(""),
                    &shared.join_links,
                );
            }
            // Collect data needed for invite generation, then drop borrow.
            let identity = shared.identity.clone();
            drop(shared);
            // Generate invite for the requesting peer.
            let handle = ClientHandle {
                shared: Rc::clone(&self.shared),
                cmd_tx: self.cmd_tx.clone(),
                deferred_channels: None,
            };
            match handle.generate_invite(&peer_id) {
                Ok(invite_data) => {
                    let msg = ops::WireMessage::JoinResponse {
                        target_peer: peer_id,
                        invite_data,
                    };
                    if let Some(data) = ops::pack_wire(&msg, &identity) {
                        let _ = self.cmd_tx.unbounded_send(
                            network::NetworkCommand::Publish {
                                topic: ops::SERVER_OPS_TOPIC.to_string(),
                                data,
                            },
                        );
                    }
                }
                Err(e) => tracing::warn!(%e, "failed to generate invite for join link"),
            }
        }
        Some(_) => {
            // Link exists but invalid.
            let shared = self.shared.borrow();
            let msg = ops::WireMessage::JoinDenied {
                target_peer: peer_id,
                reason: "link_expired".to_string(),
            };
            if let Some(data) = ops::pack_wire(&msg, &shared.identity) {
                let _ = self.cmd_tx.unbounded_send(
                    network::NetworkCommand::Publish {
                        topic: ops::SERVER_OPS_TOPIC.to_string(),
                        data,
                    },
                );
            }
        }
        None => {} // Unknown link — ignore.
    }
}
network::NetworkEvent::JoinLinkResponseReceived { invite_data } => {
    events.push(ClientEvent::JoinLinkResponse { invite_data });
}
network::NetworkEvent::JoinLinkDenied { reason } => {
    events.push(ClientEvent::JoinLinkDenied { reason });
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p willow-client`
Expected: All pass.

- [ ] **Step 7: Commit**

```bash
git add crates/client/src/lib.rs
git commit -m "feat: join link creation, deletion, and JoinRequest handler"
```

---

### Task 5: Relay — Pass-Through

**Files:**
- Modify: `crates/relay/src/lib.rs`

- [ ] **Step 1: Add variants to relay's WireMessage**

The relay has its **own** `WireMessage` enum (separate from the client's). Add:

```rust
    JoinRequest { link_id: String, peer_id: String },
    JoinResponse { target_peer: String, invite_data: String },
    JoinDenied { target_peer: String, reason: String },
```

**Critical:** Add a wildcard arm to the `match wire_msg` block in `handle_gossipsub_message`, after the `SyncBatch` arm:

```rust
                    // Ephemeral messages (join links, typing, voice) — relay forwards via
                    // gossipsub automatically, no storage needed.
                    _ => {}
```

- [ ] **Step 2: Build and test**

Run: `cargo test -p willow-relay`
Expected: All 3 pass.

- [ ] **Step 3: Commit**

```bash
git add crates/relay/src/lib.rs
git commit -m "feat: relay pass-through for join link messages"
```

---

### Task 6: Web Signals & Event Processing

**Files:**
- Modify: `crates/web/src/state.rs`
- Modify: `crates/web/src/event_processing.rs`

- [ ] **Step 1: Add signals**

In `UiState`, add:

```rust
    pub join_token: ReadSignal<Option<crate::join_token::ParsedJoinToken>>,
    pub join_status: ReadSignal<String>,  // "", "connecting", "denied:<reason>"
```

In `UiWriteSignals`, add:

```rust
    pub set_join_token: WriteSignal<Option<crate::join_token::ParsedJoinToken>>,
    pub set_join_status: WriteSignal<String>,
```

Create a simple type in a new file `crates/web/src/join_token.rs` (or inline in state.rs):

```rust
/// Parsed join token data for the UI. Avoids depending on willow_client::ops in signals.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedJoinToken {
    pub raw: String,          // original base64 for re-encoding
    pub link_id: String,
    pub server_name: String,
    pub inviter_name: String,
}
```

In `create_signals()`, add the signal pairs and wire them in.

- [ ] **Step 2: Handle events in event_processing.rs**

In `process_event_batch`, add:

```rust
ClientEvent::JoinLinkResponse { invite_data } => {
    match handle.accept_invite(&invite_data) {
        Ok(()) => {
            crate::event_processing::refresh_all_signals(handle, write);
            write.ui.set_join_token.set(None);
            write.ui.set_join_status.set(String::new());
            // Clear URL fragment to prevent re-trigger on refresh.
            if let Some(window) = web_sys::window() {
                let _ = window.history().ok().and_then(|h| {
                    h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/")).ok()
                });
            }
        }
        Err(e) => {
            tracing::error!(%e, "join link auto-accept failed");
            write.ui.set_join_status.set(format!("denied:{e}"));
        }
    }
}
ClientEvent::JoinLinkDenied { reason } => {
    write.ui.set_join_status.set(format!("denied:{reason}"));
}
```

- [ ] **Step 3: Build WASM**

Run: `cargo check --target wasm32-unknown-unknown -p willow-web`
Expected: Clean.

- [ ] **Step 4: Commit**

```bash
git add crates/web/src/state.rs crates/web/src/event_processing.rs
git commit -m "feat: join link signals and event processing"
```

---

### Task 7: URL Fragment Detection & Routing

**Files:**
- Modify: `crates/web/src/app.rs`

- [ ] **Step 1: Parse fragment and set initial signal state**

In `App()`, after `provide_context(write)`, add:

```rust
    // Detect join link from URL fragment.
    {
        let join_token_value = web_sys::window()
            .and_then(|w| w.location().hash().ok())
            .and_then(|hash| hash.strip_prefix("#join=").map(|s| s.to_string()));

        if let Some(ref token_str) = join_token_value {
            if let Some(token) = willow_client::ops::JoinToken::decode(token_str) {
                write.ui.set_join_token.set(Some(crate::join_token::ParsedJoinToken {
                    raw: token_str.clone(),
                    link_id: token.link_id,
                    server_name: token.server_name,
                    inviter_name: token.inviter_name,
                }));
                // New users go straight to connecting; existing users see JoinPage too
                // (the page handles both cases — name field is pre-filled for existing users).
                write.ui.set_join_status.set(String::new()); // idle until they click Join
            }
        }
    }
```

- [ ] **Step 2: Route to JoinPage before welcome/chat**

In the view closure, add a check **before** the `srv.is_empty()` check:

```rust
    // Join link takes priority over everything.
    let join_token_signal = app_state.ui.join_token;
    // ... in the view:
    if join_token_signal.get().is_some() {
        return view! { <JoinPage /> }.into_any();
    }
    // ... existing welcome/chat routing below
```

- [ ] **Step 3: Send JoinRequest on connection (for users who clicked Join)**

In the event processing spawn, after `process_event_batch`, add:

```rust
let has_connect = batch.iter().any(|e| matches!(e,
    willow_client::ClientEvent::PeerConnected(_) | willow_client::ClientEvent::Listening(_)
));
if has_connect {
    let status = state_for_events.ui.join_status.get_untracked();
    if status == "connecting" {
        if let Some(token) = state_for_events.ui.join_token.get_untracked() {
            handle_for_events.send_join_request(&token.link_id);
        }
    }
}
```

- [ ] **Step 4: Build WASM**

Run: `cargo check --target wasm32-unknown-unknown -p willow-web`

- [ ] **Step 5: Commit**

```bash
git add crates/web/src/app.rs
git commit -m "feat: URL fragment detection and JoinPage routing"
```

---

### Task 8: JoinPage Component (the front door)

**Files:**
- Create: `crates/web/src/components/join_page.rs`
- Modify: `crates/web/src/components/mod.rs`

- [ ] **Step 1: Create the JoinPage component**

```rust
use leptos::prelude::*;
use crate::app::WebClientHandle;
use crate::state::{AppState, AppWriteSignals};

/// Full-screen join page — the first thing a user sees when they click
/// a join link. Shows server name, inviter, name input, and a single
/// Join button that morphs into a connecting state.
#[component]
pub fn JoinPage() -> impl IntoView {
    let state = use_context::<AppState>().unwrap();
    let write = use_context::<AppWriteSignals>().unwrap();
    let handle = use_context::<WebClientHandle>().unwrap();

    let token = state.ui.join_token;
    let status = state.ui.join_status;

    // Pre-fill name from saved profile.
    let (name, set_name) = signal(handle.display_name());

    // Retry timer: exponential backoff while status == "connecting".
    {
        let handle_retry = handle.clone();
        let status_retry = status;
        let token_retry = token;
        wasm_bindgen_futures::spawn_local(async move {
            let mut backoff_ms: u32 = 2_000;
            const MAX_BACKOFF: u32 = 30_000;
            loop {
                gloo_timers::future::TimeoutFuture::new(backoff_ms).await;
                if status_retry.get_untracked() != "connecting" { break; }
                if let Some(t) = token_retry.get_untracked() {
                    handle_retry.send_join_request(&t.link_id);
                }
                backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF);
            }
        });
    }

    let on_join = {
        let h = handle.clone();
        move |_| {
            let n = name.get_untracked();
            if !n.trim().is_empty() {
                let _ = h.set_display_name(n.trim());
            }
            write.ui.set_join_status.set("connecting".to_string());
            // Send initial JoinRequest.
            if let Some(t) = token.get_untracked() {
                h.send_join_request(&t.link_id);
            }
        }
    };

    let on_cancel = move |_| {
        write.ui.set_join_token.set(None);
        write.ui.set_join_status.set(String::new());
        if let Some(window) = web_sys::window() {
            let _ = window.history().ok().and_then(|h| {
                h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/")).ok()
            });
        }
    };

    view! {
        <div class="join-page">
            <div class="join-page-ambient"></div>
            <div class="join-card">
                <div class="join-card-brand">"willow"</div>

                {move || token.get().map(|t| {
                    let server_name = t.server_name.clone();
                    let inviter = t.inviter_name.clone();
                    view! {
                        <h1 class="join-card-server">{server_name}</h1>
                        <p class="join-card-inviter">"Invited by " <strong>{inviter}</strong></p>
                    }
                })}

                <div class="join-card-field">
                    <label>"Your name"</label>
                    <input
                        type="text"
                        placeholder="Enter your name..."
                        prop:value=move || name.get()
                        on:input=move |ev| set_name.set(event_target_value(&ev))
                        disabled=move || status.get() == "connecting"
                    />
                </div>

                {move || {
                    let s = status.get();
                    let server = token.get().map(|t| t.server_name.clone()).unwrap_or_default();
                    if s == "connecting" {
                        view! {
                            <button class="btn btn-primary join-card-btn connecting" disabled>
                                "Connecting..."
                            </button>
                            <p class="join-card-hint">
                                "Waiting for the server owner to respond."
                            </p>
                        }.into_any()
                    } else if let Some(reason) = s.strip_prefix("denied:") {
                        let msg = match reason {
                            "link_expired" => "This invite link has been fully used.",
                            "link_disabled" => "This invite link is no longer active.",
                            _ => "This invite link is no longer valid.",
                        };
                        view! {
                            <p class="join-card-error">{msg}</p>
                            <button class="btn btn-sm" on:click=on_cancel>"Back"</button>
                        }.into_any()
                    } else {
                        view! {
                            <button class="btn btn-primary join-card-btn" on:click=on_join.clone()>
                                {format!("Join {server}")}
                            </button>
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}
```

- [ ] **Step 2: Export from mod.rs**

```rust
mod join_page;
pub use join_page::JoinPage;
```

- [ ] **Step 3: Add CSS**

Add to `crates/web/style.css`:

```css
/* ── Join Page ─────────────────────────────────────────────────────── */

.join-page {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-main);
    z-index: 100;
    transition: opacity 0.3s cubic-bezier(0.4, 0, 0.2, 1),
                transform 0.3s cubic-bezier(0.4, 0, 0.2, 1);
}
.join-page.leaving {
    opacity: 0;
    transform: scale(0.98);
    pointer-events: none;
}
.join-page-ambient {
    position: absolute;
    inset: 0;
    background:
        radial-gradient(ellipse at 50% 40%, var(--accent-glow) 0%, transparent 60%);
    opacity: 0.5;
    pointer-events: none;
}
.join-card {
    position: relative;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 16px;
    padding: 40px 32px 32px;
    max-width: 400px;
    width: 90%;
    text-align: center;
    box-shadow: var(--shadow-lg);
    animation: scale-in 0.2s ease;
}
.join-card-brand {
    font-family: 'IBM Plex Mono', monospace;
    font-size: 13px;
    letter-spacing: 0.08em;
    color: var(--text-muted);
    text-transform: lowercase;
    margin-bottom: 24px;
}
.join-card-server {
    font-size: 28px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0 0 4px;
    line-height: 1.2;
}
.join-card-inviter {
    color: var(--text-muted);
    font-size: 14px;
    margin: 0 0 28px;
}
.join-card-inviter strong {
    color: var(--text-secondary);
}
.join-card-field {
    text-align: left;
    margin-bottom: 20px;
}
.join-card-field label {
    display: block;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-bottom: 6px;
}
.join-card-field input {
    width: 100%;
    padding: 10px 12px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 8px;
    color: var(--text-primary);
    font-family: inherit;
    font-size: 15px;
    box-sizing: border-box;
    outline: none;
    transition: border-color var(--transition-fast);
}
.join-card-field input:focus {
    border-color: var(--accent);
}
.join-card-field input:disabled {
    opacity: 0.5;
}
.join-card-btn {
    width: 100%;
    padding: 12px;
    font-size: 15px;
    font-weight: 600;
    border-radius: 8px;
    cursor: pointer;
    transition: all var(--transition-normal);
}
.join-card-btn.connecting {
    cursor: default;
    position: relative;
    border: 2px solid var(--accent);
    background: transparent;
    color: var(--accent);
    animation: join-pulse 2s ease-in-out infinite;
}
@keyframes join-pulse {
    0%, 100% { box-shadow: 0 0 0 0 var(--accent-glow); }
    50% { box-shadow: 0 0 0 8px transparent; }
}
.join-card-hint {
    color: var(--text-muted);
    font-size: 13px;
    margin: 12px 0 0;
}
.join-card-error {
    color: var(--danger);
    font-size: 14px;
    margin: 0 0 12px;
}
```

- [ ] **Step 4: Build WASM**

Run: `cargo check --target wasm32-unknown-unknown -p willow-web`

- [ ] **Step 5: Commit**

```bash
git add crates/web/src/components/join_page.rs crates/web/src/components/mod.rs crates/web/style.css
git commit -m "feat: JoinPage component — the front door experience"
```

---

### Task 9: Settings UI — Link Management

**Files:**
- Modify: `crates/web/src/components/settings.rs`

- [ ] **Step 1: Add the Invite Links section to the Server tab**

After the existing "Invite a Peer" section, add:

```rust
// ── Invite Links ──────────────────────────────────────────
let (link_copied, set_link_copied) = signal(false);
let (link_list, set_link_list) = signal(handle.join_links());

// "Create Invite Link" button — immediately generates + copies.
let handle_gen = handle.clone();
let on_create_link = move |_| {
    match handle_gen.create_join_link(5, None) {
        Ok(token) => {
            let url = format!("https://willow.intendednull.com/#join={token}");
            // Copy to clipboard.
            if let Some(w) = web_sys::window() {
                let _ = w.navigator().clipboard().write_text(&url);
            }
            set_link_copied.set(true);
            // Flash "Copied!" for 1.5s.
            let set_copied = set_link_copied;
            wasm_bindgen_futures::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(1500).await;
                set_copied.set(false);
            });
            set_link_list.set(handle_gen.join_links());
        }
        Err(e) => set_status.set(format!("Error: {e}")),
    }
};
```

View:

```rust
<div class="settings-section">
    <h3>"Invite Links"</h3>
    <p class="settings-hint">"Share a link to let people join while you're online."</p>
    <div style="position: relative; display: inline-block;">
        <button class="btn btn-accent-green" on:click=on_create_link>
            "Create Invite Link"
        </button>
        {move || link_copied.get().then(|| view! {
            <span class="copied-tooltip">"Copied!"</span>
        })}
    </div>

    // Advanced options in a <details> disclosure.
    // (Implement with local signals for max_uses and expires_at,
    //  updating the next create_join_link call.)

    // Active links list.
    <div class="invite-link-list">
        <For
            each=move || link_list.get()
            key=|link| link.link_id.clone()
            let:link
        >
            {
                let h = handle.clone();
                let lid = link.link_id.clone();
                let valid = link.is_valid();
                view! {
                    <div class=("invite-link-item", move || if valid { "" } else { "expired" })>
                        <span class="invite-link-uses">
                            {format!("{}/{}", link.used, link.max_uses)}
                        </span>
                        <span class="invite-link-age">
                            // Relative time from link.created_at — implement with a
                            // helper that computes "2h ago", "3d ago", etc.
                            "active"
                        </span>
                        <button class="btn-icon btn-icon-danger" on:click={
                            let h2 = h.clone();
                            let lid2 = lid.clone();
                            move |_| {
                                h2.delete_join_link(&lid2);
                                set_link_list.set(h2.join_links());
                            }
                        }>
                            {crate::icons::icon_trash()}
                        </button>
                    </div>
                }
            }
        </For>
    </div>
</div>
```

- [ ] **Step 2: Add CSS for link management**

```css
/* ── Invite Link Management ────────────────────────────────────────── */

.btn-accent-green {
    background: var(--accent-green);
    color: white;
    border: none;
    padding: 10px 20px;
    border-radius: 8px;
    font-weight: 600;
    cursor: pointer;
    transition: background var(--transition-fast);
}
.btn-accent-green:hover {
    background: var(--accent-green-hover);
}
.invite-link-list {
    margin-top: 16px;
    display: flex;
    flex-direction: column;
    gap: 6px;
}
.invite-link-item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
    background: var(--bg-input);
    border-radius: 8px;
    font-size: 13px;
}
.invite-link-item.expired {
    opacity: 0.4;
    text-decoration: line-through;
}
.invite-link-uses {
    font-family: 'IBM Plex Mono', monospace;
    font-size: 12px;
    background: var(--bg-main);
    padding: 2px 8px;
    border-radius: 4px;
    color: var(--text-muted);
}
.invite-link-age {
    flex: 1;
    color: var(--text-muted);
    font-size: 12px;
}
```

- [ ] **Step 3: Build WASM**

Run: `cargo check --target wasm32-unknown-unknown -p willow-web`

- [ ] **Step 4: Commit**

```bash
git add crates/web/src/components/settings.rs crates/web/style.css
git commit -m "feat: invite link management in server settings"
```

---

### Task 10: E2E Test

**Files:**
- Create: `e2e/join-links.spec.ts`

- [ ] **Step 1: Write E2E test**

```typescript
import { test, expect } from '@playwright/test';
import { freshStart, createServer, sendMessage, waitForMessage, openSidebar } from './helpers';

test.describe('Join via shareable link', () => {
  test('peer joins via link URL and sees messages', async ({ browser }) => {
    const ctxA = await browser.newContext();
    const pageA = await ctxA.newPage();
    await freshStart(pageA);
    await createServer(pageA, 'Link Test', 'Alice');

    // Generate join link from settings.
    await openSidebar(pageA);
    await pageA.locator('.server-gear-btn').click();
    await pageA.waitForTimeout(500);
    await pageA.locator('button', { hasText: 'Create Invite Link' }).click();
    await pageA.waitForTimeout(500);

    // The link was copied to clipboard — read it.
    const clipboardUrl = await pageA.evaluate(() => navigator.clipboard.readText());
    expect(clipboardUrl).toContain('#join=');

    // Go back to chat.
    await pageA.locator('text=Back').click();

    // Peer B opens the join link URL.
    const ctxB = await browser.newContext();
    const pageB = await ctxB.newPage();
    await freshStart(pageB);
    await pageB.goto(clipboardUrl);

    // Should see the JoinPage with server name.
    await expect(pageB.locator('.join-card-server')).toContainText('Link Test');
    await expect(pageB.locator('.join-card-inviter')).toContainText('Alice');

    // Enter name and click Join.
    await pageB.locator('.join-card-field input').fill('Bob');
    await pageB.locator('.join-card-btn').click();

    // Wait for join to complete (join page disappears, chat appears).
    await pageB.waitForSelector('.sidebar, .app', { timeout: 30000 });
    await pageB.waitForTimeout(5000);

    // Verify B sees the server.
    await openSidebar(pageB);
    await expect(pageB.locator('.channel-item', { hasText: 'general' }))
      .toBeVisible({ timeout: 15000 });

    // A sends a message.
    await sendMessage(pageA, 'Welcome Bob!');

    // B should see it.
    await waitForMessage(pageB, 'Welcome Bob!', 30000);

    await ctxA.close();
    await ctxB.close();
  });
});
```

- [ ] **Step 2: Run test**

Run: `npx playwright test e2e/join-links.spec.ts --project=desktop-chrome`

- [ ] **Step 3: Commit**

```bash
git add e2e/join-links.spec.ts
git commit -m "test: E2E test for join-via-shareable-link"
```

---

### Task 11: Full Verification

- [ ] **Step 1:** `cargo test -p willow-client` — all pass
- [ ] **Step 2:** `cargo test -p willow-relay` — all pass
- [ ] **Step 3:** `just check-wasm` — clean
- [ ] **Step 4:** `cargo clippy --workspace -- -D warnings` — no warnings
- [ ] **Step 5:** `cargo fmt --check` — clean
