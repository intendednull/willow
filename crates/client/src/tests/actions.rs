// `share_file_inline` is `#[deprecated]` in favour of
// `upload_attachment` + `send_attachment_message`. The legacy tests in
// this module specifically pin the inline-base64 reader contract that
// stays alive for back-compat — silence the deprecation warning so
// `just check` remains zero-warning.
#![allow(deprecated)]

//! Tests for `crates/client/src/actions.rs`.
//!
//! `actions.rs` is mostly a thin pass-through layer that forwards UI
//! calls to [`crate::mutations::ClientMutations`] (whose own behaviour is
//! covered elsewhere — see `multi_peer_sync.rs`, `trust_flow.rs`, the
//! `tests` block at the bottom of `lib.rs`, and the state-machine tests
//! in `crates/state/src/tests.rs`). The only paths that warrant a
//! dedicated test at this tier are the ones that do real translation
//! work *before* delegating: validation, ID minting, derived-view
//! composition.
//!
//! What this file covers:
//!
//! * [`ClientHandle::share_file_inline`] — 256 KB size cap and
//!   `[file:NAME:BASE64]` body shape.
//! * [`ClientHandle::create_voice_channel`] — UUID minting and
//!   `ChannelKind::Voice` is recorded on the materialized channel.
//! * [`ClientHandle::set_permission`] — translates `(role, perm,
//!   granted)` into a `SetPermission` event that lands on the role's
//!   permission set, including the revoke (`granted = false`) branch.
//! * [`ClientHandle::assign_role`] — translates `(peer, role)` into an
//!   `AssignRole` event that lands on the member's role set.
//! * [`ClientHandle::pinned_message_ids`] / `pinned_messages` /
//!   `is_pinned` — channel-name → channel-id lookup, ordering,
//!   composition, and missing-channel handling.
//!
//! Pure pass-throughs (e.g. `send_message`, `create_channel`,
//! `propose_revoke_admin`, `mutate_channel_mute`, …) are intentionally
//! NOT re-tested here: their behaviour is exercised through the
//! mutation handle directly in the modules listed above.

use crate::test_client;
use willow_state::{ChannelKind, Permission};

/// `share_file_inline` rejects payloads larger than the 256 KiB cap and
/// does not enqueue any message in that case.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn share_file_inline_rejects_oversized_payload() {
    let (client, _rx) = test_client();

    // 256 KiB + 1 byte — one byte over the documented cap.
    let oversized = vec![0u8; 256 * 1024 + 1];
    let err = client
        .share_file_inline("general", "big.bin", &oversized)
        .await
        .expect_err("oversized payload must be rejected");
    assert!(
        err.to_string().contains("file too large"),
        "error must mention size limit, got: {err}"
    );

    // The rejected call must not have produced a message.
    let msgs = client.messages("general").await;
    assert!(
        msgs.is_empty(),
        "no message should have been sent for a rejected file"
    );
}

/// `share_file_inline` formats the body as `[file:NAME:BASE64(DATA)]`
/// when the payload fits, and the encoded bytes round-trip.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn share_file_inline_emits_base64_encoded_body() {
    let (client, _rx) = test_client();

    let data: &[u8] = b"hello, willow!";
    client
        .share_file_inline("general", "note.txt", data)
        .await
        .expect("inline share must succeed under cap");
    // Let the actor system apply the message event.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let msgs = client.messages("general").await;
    let body = &msgs.last().expect("message must be recorded").body;
    let expected_prefix = "[file:note.txt:";
    assert!(
        body.starts_with(expected_prefix) && body.ends_with(']'),
        "body must use [file:NAME:BASE64] shape, got: {body}"
    );
    let encoded = &body[expected_prefix.len()..body.len() - 1];
    let decoded = crate::base64::decode(encoded).expect("body payload must be valid base64");
    assert_eq!(decoded, data, "round-trip of inlined bytes must match");
}

/// `create_voice_channel` mints a fresh channel with `ChannelKind::Voice`.
/// The mutation handle exposes no `create_voice_channel`, so this whole
/// path lives in `actions.rs` and needs its own coverage.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_voice_channel_records_voice_kind() {
    let (client, _rx) = test_client();

    client
        .create_voice_channel("lounge")
        .await
        .expect("voice channel creation must succeed");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let kinds = client.channel_kinds().await;
    let lounge = kinds
        .iter()
        .find(|(name, _)| name == "lounge")
        .expect("lounge channel must exist");
    assert!(
        matches!(lounge.1, ChannelKind::Voice),
        "lounge must be a Voice channel, got {:?}",
        lounge.1
    );
}

/// `set_permission(granted = true)` adds the permission to the named
/// role's permission set; `set_permission(granted = false)` removes it.
/// `actions.rs::set_permission` builds the `SetPermission` event itself
/// (no equivalent exists on the mutation handle), so both branches need
/// direct coverage here.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_permission_grants_then_revokes_on_role() {
    let (client, _rx) = test_client();

    // Create a role we can mutate. `create_role` mints the UUID, so we
    // have to discover the assigned id from materialized state.
    client
        .create_role("Moderator")
        .await
        .expect("role creation must succeed");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let role_id = {
        let snap = client.state_snapshot().await;
        snap.roles
            .values()
            .find(|r| r.name == "Moderator")
            .expect("Moderator role must exist")
            .id
            .clone()
    };

    // Grant.
    client
        .set_permission(&role_id, Permission::ManageChannels, true)
        .await
        .expect("granting permission on owner-authored role must succeed");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let granted_perms = client
        .state_snapshot()
        .await
        .roles
        .get(&role_id)
        .expect("role must still exist")
        .permissions
        .clone();
    assert!(
        granted_perms.contains(&Permission::ManageChannels),
        "role must hold ManageChannels after grant, got {granted_perms:?}"
    );

    // Revoke (granted = false branch).
    client
        .set_permission(&role_id, Permission::ManageChannels, false)
        .await
        .expect("revoking permission must succeed");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let revoked_perms = client
        .state_snapshot()
        .await
        .roles
        .get(&role_id)
        .expect("role must still exist")
        .permissions
        .clone();
    assert!(
        !revoked_perms.contains(&Permission::ManageChannels),
        "role must lack ManageChannels after revoke, got {revoked_perms:?}"
    );
}

/// `assign_role` puts the role id into the target member's `roles` set.
/// Like `set_permission`, this entry point assembles the event itself
/// rather than delegating to a mutation-handle helper.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn assign_role_adds_role_to_member() {
    let (client, _rx) = test_client();
    let me = client.identity.endpoint_id();

    client
        .create_role("Moderator")
        .await
        .expect("role creation must succeed");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let role_id = {
        let snap = client.state_snapshot().await;
        snap.roles
            .values()
            .find(|r| r.name == "Moderator")
            .expect("Moderator role must exist")
            .id
            .clone()
    };

    client
        .assign_role(me, &role_id)
        .await
        .expect("assigning role to self (owner) must succeed");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let snap = client.state_snapshot().await;
    let member = snap
        .members
        .get(&me)
        .expect("local peer must be a member of its own server");
    assert!(
        member.roles.contains(&role_id),
        "member must have role assigned, got roles {:?}",
        member.roles
    );
}

/// `pinned_message_ids` returns an empty vec for an unknown channel
/// rather than panicking — the channel-name → channel-id lookup falls
/// back to a default-empty channel id and the subsequent `channels.get`
/// returns `None`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_message_ids_empty_for_unknown_channel() {
    let (client, _rx) = test_client();
    let ids = client.pinned_message_ids("does-not-exist").await;
    assert!(
        ids.is_empty(),
        "unknown channel must yield no pinned ids, got {ids:?}"
    );

    let msgs = client.pinned_messages("does-not-exist").await;
    assert!(
        msgs.is_empty(),
        "unknown channel must yield no pinned messages"
    );
}

/// End-to-end pin lifecycle exercises the composition chain inside
/// `actions.rs`: `pin_message` (delegated) → `pinned_message_ids`
/// (channel-name lookup + sort) → `pinned_messages` (filter messages
/// view) and `is_pinned` (membership test). Unpinning then has to
/// remove the entry from each of those derived views.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pin_message_flows_through_pinned_views() {
    let (client, _rx) = test_client();

    // Author a message we can pin.
    client.send_message("general", "pin me").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let msg = client
        .messages("general")
        .await
        .into_iter()
        .find(|m| m.body == "pin me")
        .expect("authored message must be present");
    let msg_hash: willow_state::EventHash =
        msg.id.parse().expect("DisplayMessage.id is hex EventHash");

    // Initially nothing is pinned.
    assert!(client.pinned_message_ids("general").await.is_empty());
    assert!(!client.is_pinned("general", &msg_hash).await);
    assert!(client.pinned_messages("general").await.is_empty());

    // Pin it.
    client
        .pin_message("general", &msg_hash)
        .await
        .expect("owner pin must succeed");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let ids = client.pinned_message_ids("general").await;
    assert_eq!(ids, vec![msg_hash], "pinned id list must contain the pin");
    assert!(client.is_pinned("general", &msg_hash).await);

    let pinned = client.pinned_messages("general").await;
    assert_eq!(pinned.len(), 1, "pinned messages must surface one entry");
    assert_eq!(pinned[0].body, "pin me");

    // Unpin and re-check every derived view drops it.
    client
        .unpin_message("general", &msg_hash)
        .await
        .expect("owner unpin must succeed");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert!(client.pinned_message_ids("general").await.is_empty());
    assert!(!client.is_pinned("general", &msg_hash).await);
    assert!(client.pinned_messages("general").await.is_empty());
}
