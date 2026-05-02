//! # Sync-queue primitives
//!
//! Pure, unit-testable data types + derivation helpers used by the
//! [`QueueMeta`](crate::state_actors) actor, the `compute_queue_view`
//! projection, and the message-row `queue_note` projection in
//! [`views`](crate::views).
//!
//! The module is intentionally platform-agnostic — it compiles unchanged
//! on native and WASM. All inputs are value types; the helpers are pure
//! functions suitable for direct unit testing without spinning up an
//! actor or touching I/O.
//!
//! Spec: [`docs/specs/2026-04-19-ui-design/sync-queue.md`].
//!
//! Plan: [`docs/plans/2026-04-21-ui-phase-2b-sync-queue.md`] Task 1.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use willow_identity::EndpointId;
use willow_messaging::hlc::HlcTimestamp;
use willow_messaging::store::DeliveryState;

use crate::presence::Tick;

/// Per-peer outbound queue summary — produced by
/// [`compute_queue_view`](crate::views::compute_queue_view) from
/// [`QueueMeta`](crate::state_actors::QueueMeta)'s in-flight entries.
///
/// Shape mirrors spec §Data shape; durations are HLC timestamps and
/// ticks so the UI layer can render absolute + relative times without a
/// clock.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueSummary {
    /// Number of queued outbound messages waiting for this peer.
    pub outbound: u32,
    /// HLC timestamp of the oldest queued outbound message.
    pub oldest_outbound_at: Option<HlcTimestamp>,
    /// Tick at which the last retry attempt happened (for backoff UI).
    pub last_attempt_at: Option<Tick>,
    /// Last transport error message from the last retry, if any.
    pub last_attempt_error: Option<String>,
}

/// One row in the sync-queue screen's `Recent · arrived from queue`
/// section.
///
/// Bucketed by peer + contiguous arrival window in the actor so the UI
/// can render `14 messages synced overnight · from 4 peers` directly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrivedSummary {
    /// Peer whose messages just arrived.
    pub peer_id: EndpointId,
    /// Tick at which the bucket closed (youngest arrival in the window).
    pub at_tick: Tick,
    /// Number of messages in this bucket.
    pub count: u32,
    /// Optional text preview for the most recent message in the bucket.
    /// `None` on the mobile lock screen / privacy-restricted surfaces.
    pub preview: Option<String>,
}

/// Relay-reachability state used by the offline strip + relay-signal
/// button on the sync-queue screen.
///
/// See `Network::relay_status` in the [`willow_network`] crate for the
/// full derivation. The `NotConfigured` default means the client has
/// never been given a relay address — treated as "relay-unaware" by
/// the UI (no amber suffix, no signal button).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelayStatus {
    /// Relay session had a recent successful ping (< 30 s).
    Reachable,
    /// Relay configured but the last ping was > 30 s ago (unreachable).
    Unreachable,
    /// No relay configured (or relay support disabled).
    #[default]
    NotConfigured,
}

/// Derive whether a local-author message is still pending acknowledgement
/// from at least one recipient.
///
/// Returns `false` for messages authored by remote peers regardless of
/// the delivery state — remote-author pending is visible to us only as a
/// [`derive_late_arrival`] signal. Returns `false` for delivered or
/// unknown-state messages (no delivery tracking → treated as delivered).
///
/// This is the `Pending` arm of the
/// [`QueueNote`](crate::state::QueueNote) tri-state.
pub fn derive_pending(is_local_author: bool, delivery: Option<&DeliveryState>) -> bool {
    if !is_local_author {
        return false;
    }
    matches!(
        delivery,
        Some(DeliveryState::PendingAllRecipients(_))
            | Some(DeliveryState::PendingSomeRecipients { .. })
    )
}

/// Derive whether a remote-author message is a `LateArrival` — the peer
/// was unreachable near the authoring time and the message took more
/// than 30 s to reach us.
///
/// `history` is the bounded peer presence history (`(peer, tick,
/// reachable)` triples) maintained by
/// [`QueueMeta`](crate::state_actors::QueueMeta). The predicate is
/// wall-clock based (not HLC) because the spec's `inbound-held` 30-s
/// threshold is an absolute "arrived later than expected" heuristic, not
/// a logical-time property.
///
/// Returns `true` iff:
///
/// - `author` has an `unreachable=false` entry in `history`, AND
/// - `now_ms - msg_authored_at_ms > 30_000`.
pub fn derive_late_arrival(
    history: &VecDeque<(EndpointId, Tick, bool)>,
    author: EndpointId,
    msg_authored_at_ms: u64,
    now_ms: u64,
) -> bool {
    let was_offline = history
        .iter()
        .any(|(p, _, reachable)| *p == author && !*reachable);
    was_offline && now_ms.saturating_sub(msg_authored_at_ms) > 30_000
}

// ───── Tests ─────────────────────────────────────────────────────────────

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use willow_identity::Identity;

    fn peer() -> EndpointId {
        Identity::generate().endpoint_id()
    }

    #[test]
    fn derive_pending_false_when_remote_author() {
        let mut set = HashSet::new();
        set.insert(peer());
        let delivery = DeliveryState::PendingAllRecipients(set);
        assert!(!derive_pending(false, Some(&delivery)));
    }

    #[test]
    fn derive_pending_true_when_local_and_pending_all() {
        let mut set = HashSet::new();
        set.insert(peer());
        let delivery = DeliveryState::PendingAllRecipients(set);
        assert!(derive_pending(true, Some(&delivery)));
    }

    #[test]
    fn derive_pending_true_when_local_and_pending_some() {
        let mut acked = HashSet::new();
        acked.insert(peer());
        let mut pending = HashSet::new();
        pending.insert(peer());
        let delivery = DeliveryState::PendingSomeRecipients { acked, pending };
        assert!(derive_pending(true, Some(&delivery)));
    }

    #[test]
    fn derive_pending_false_when_local_and_delivered() {
        assert!(!derive_pending(true, Some(&DeliveryState::Delivered)));
    }

    #[test]
    fn derive_pending_false_when_delivery_unknown() {
        // No delivery tracking — treat as delivered (§Defaults permissive).
        assert!(!derive_pending(true, None));
    }

    #[test]
    fn derive_late_arrival_true_when_author_was_offline_and_delay() {
        let author = peer();
        let mut history = VecDeque::new();
        history.push_back((author, 10, false));
        assert!(derive_late_arrival(&history, author, 1_000_000, 1_050_000));
    }

    #[test]
    fn derive_late_arrival_false_when_author_was_online() {
        let author = peer();
        let mut history = VecDeque::new();
        history.push_back((author, 10, true));
        assert!(!derive_late_arrival(&history, author, 1_000_000, 1_050_000));
    }

    #[test]
    fn derive_late_arrival_false_when_delay_under_30s() {
        let author = peer();
        let mut history = VecDeque::new();
        history.push_back((author, 10, false));
        assert!(!derive_late_arrival(&history, author, 1_000_000, 1_020_000));
    }

    #[test]
    fn derive_late_arrival_false_when_history_empty() {
        let author = peer();
        let history: VecDeque<(EndpointId, Tick, bool)> = VecDeque::new();
        assert!(!derive_late_arrival(&history, author, 1_000_000, 1_050_000));
    }

    #[test]
    fn derive_late_arrival_false_when_other_peer_offline_only() {
        let author = peer();
        let other = peer();
        let mut history = VecDeque::new();
        history.push_back((other, 10, false));
        assert!(!derive_late_arrival(&history, author, 1_000_000, 1_050_000));
    }

    #[test]
    fn derive_late_arrival_saturates_when_msg_newer_than_now() {
        // HLC regression case: message timestamp beats now. The
        // predicate must not underflow; it must return false because
        // there is no delay to report.
        let author = peer();
        let mut history = VecDeque::new();
        history.push_back((author, 10, false));
        assert!(!derive_late_arrival(&history, author, 1_050_000, 1_000_000));
    }

    #[test]
    fn queue_summary_roundtrip_serialize() {
        let sum = QueueSummary {
            outbound: 3,
            oldest_outbound_at: Some(HlcTimestamp {
                millis: 12_345,
                counter: 7,
            }),
            last_attempt_at: Some(42),
            last_attempt_error: Some("timeout".into()),
        };
        let bytes = willow_transport::pack(&sum).unwrap();
        let decoded: QueueSummary = willow_transport::unpack(&bytes).unwrap();
        assert_eq!(decoded, sum);
    }

    #[test]
    fn relay_status_default_is_not_configured() {
        assert_eq!(RelayStatus::default(), RelayStatus::NotConfigured);
    }
}
