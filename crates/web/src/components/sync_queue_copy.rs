//! Sync-queue copy constants — Phase 2b.
//!
//! One source of truth for every byte-exact string that the sync-queue
//! surfaces render. Mirrors the `Copy (exact)` table in
//! `docs/specs/2026-04-19-ui-design/sync-queue.md` §Copy. When the spec
//! changes, update this module; when a component needs a sync-queue
//! string, import from here — **do not** paraphrase or re-type.
//!
//! Format strings (`{peer}`, `{n}`, `{grove}`, `{reached}`, `{total}`)
//! are exposed as small `const fn` helpers so call sites stay
//! declarative and the spec wording is the only thing that moves when
//! copy is polished.

// ───── Timing constants ─────────────────────────────────────────────
//
// Live next to the copy because the toast + banner gate on them and
// the value is spec-driven, not ergonomic.

/// Reconnection-toast / welcome-back-banner "≥ 60 s offline" gate.
///
/// Spec: `docs/specs/2026-04-19-ui-design/sync-queue.md` §Reconnection
/// toast + §Welcome-back banner — both surface only after a long
/// offline window so first-connect and brief blips stay silent.
///
/// Typed as `u64` to match `willow_client::presence::Tick`, the units
/// carried on `QueueView::last_offline_ticks`.
pub const RECONNECT_GATE_TICKS: u64 = 60;

// ───── Offline strip ────────────────────────────────────────────────

/// `strip_default` — rendered when `queue.view.peer_count > 1`.
pub fn strip_default(peer_count: u32, depth: u32) -> String {
    format!("waiting for {peer_count} peers · {depth} messages queued")
}

/// `strip_singular` — rendered when `queue.view.peer_count == 1`.
pub fn strip_singular(peer: &str, depth: u32) -> String {
    format!("waiting for {peer} · {depth} messages queued")
}

/// `strip_relay_suffix` — appended to the strip copy when the relay is
/// unreachable.
pub const STRIP_RELAY_SUFFIX: &str = " · relay unreachable";

// ───── Queue pill ───────────────────────────────────────────────────

/// `pill_queued` for a single peer, clamped at 99 / 500 per spec edge
/// cases. Returns `queued · {n}` / `queued · 99+` / `queued · 500+`.
pub fn pill_queued(total: u32) -> String {
    if total > 500 {
        "queued · 500+".to_string()
    } else if total > 99 {
        "queued · 99+".to_string()
    } else {
        format!("queued · {total}")
    }
}

/// `pill_tooltip_out` — screen-reader label when only outbound counts.
pub fn pill_tooltip_out(out: u32, peer: &str) -> String {
    format!("you have {out} messages waiting for {peer}")
}

/// `pill_tooltip_in` — screen-reader label when only inbound counts.
pub fn pill_tooltip_in(peer: &str, inbound: u32) -> String {
    format!("{peer} has {inbound} messages pending for you")
}

/// `pill_tooltip_both` — screen-reader label when both directions.
pub fn pill_tooltip_both(out: u32, peer: &str, inbound: u32) -> String {
    format!("{out} waiting for {peer} · {inbound} pending from them")
}

// ───── Inline queue note ────────────────────────────────────────────

/// `msg_note_queued_peer` / `msg_note_queued_grove` — single copy for
/// both. Caller chooses the surface label.
pub fn msg_note_queued(peer_or_grove: &str) -> String {
    format!("queued · will send when {peer_or_grove} reachable")
}

/// `msg_note_just_delivered` — transient note on Pending → None flip.
pub const MSG_NOTE_JUST_DELIVERED: &str = "queued earlier · delivered just now";

/// `msg_note_inbound_held` — transient note for late-arrival remote
/// messages.
pub const MSG_NOTE_INBOUND_HELD: &str = "sent earlier · arrived now";

// ───── Sync-queue screen ────────────────────────────────────────────

/// `screen_title` — shipped in the screen header.
pub const SCREEN_TITLE: &str = "sync queue";

/// `screen_subtitle` — shipped below the title.
pub const SCREEN_SUBTITLE: &str = "what's pending · what's reachable";

/// `screen_card_label` — shown when the queue has depth > 0.
pub const SCREEN_CARD_REACHING_OUT: &str = "reaching out…";

/// `screen_card_drained` — shown when the queue is empty.
pub const SCREEN_CARD_DRAINED: &str = "queue drained";

/// `screen_card_count` — pluralised per-peer progress count.
pub fn screen_card_count(reached: u32, total: u32) -> String {
    format!("{reached} / {total} peers")
}

/// `screen_section_recent` — header on the recent-arrivals section.
pub const SCREEN_SECTION_RECENT: &str = "recent · arrived from queue";

/// `screen_footnote` — verbatim privacy reference footer.
pub const SCREEN_FOOTNOTE: &str =
    "willow holds unsent messages on this device and tries again automatically. nothing is stored on a server.";

/// `action_retry` — primary footer action.
pub const ACTION_RETRY: &str = "retry now";

/// `action_retry_busy` — rendered while `client.retry_queue()` is
/// in-flight. The spec pins the idle label; the busy label is an
/// accessibility refinement so the button is not mute while waiting.
pub const ACTION_RETRY_BUSY: &str = "retrying…";

/// `action_mark_read` — inbound-tab footer action.
pub const ACTION_MARK_READ: &str = "mark as read locally";

/// `action_mark_read_busy` — rendered while
/// `client.mark_queue_read()` is in flight across the inbound peer set.
/// The spec pins the idle label; the busy label is an accessibility
/// refinement so the button is not mute while waiting and a parallel
/// to `ACTION_RETRY_BUSY`.
pub const ACTION_MARK_READ_BUSY: &str = "marking…";

/// `toast_mark_read_failed` — error-toast title rendered when one or
/// more peers' `mark_queue_read` calls fail. Plural-aware so the user
/// knows the partial-success shape.
pub fn toast_mark_read_failed(n: usize) -> String {
    if n == 1 {
        "failed to mark 1 peer as read".to_string()
    } else {
        format!("failed to mark {n} peers as read")
    }
}

/// `screen_pill_waiting` — per-row pill on the outbound tab.
pub const SCREEN_PILL_WAITING: &str = "waiting";

/// `screen_pill_synced` — per-row pill on the recent-arrivals section.
pub const SCREEN_PILL_SYNCED: &str = "synced";

// ───── Reconnection toast + welcome-back banner ─────────────────────

/// `toast_reconnected_many` when `n > 0`.
pub fn toast_reconnected(n: u32) -> String {
    if n > 0 {
        format!("reconnected · delivering {n} messages")
    } else {
        "reconnected".to_string()
    }
}

/// `banner_welcome_back` — single-line banner copy.
pub fn banner_welcome_back(n: u32) -> String {
    format!("willow queued {n} messages while you were away — everything arrived")
}

// ───── Relay awareness ──────────────────────────────────────────────

/// `relay_unreachable` — inline tooltip / popover header when the relay
/// has not responded inside the 30 s reachability window.
pub const RELAY_UNREACHABLE: &str = "relay unreachable — direct-peer attempts continue";

// ───── Privacy-safe notification bodies ─────────────────────────────

/// `notif_letter` — push-notification body for an inbound letter that
/// was held on another device.
pub const NOTIF_LETTER: &str = "a letter is waiting";

/// `notif_grove` — push-notification body for an inbound grove message.
pub fn notif_grove(grove: &str) -> String {
    format!("a message in {grove}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_default_pluralises_peers_and_messages() {
        assert_eq!(
            strip_default(2, 3),
            "waiting for 2 peers · 3 messages queued"
        );
    }

    #[test]
    fn strip_singular_interpolates_display_name() {
        assert_eq!(
            strip_singular("alice", 3),
            "waiting for alice · 3 messages queued"
        );
    }

    #[test]
    fn pill_queued_clamps_at_99_and_500() {
        assert_eq!(pill_queued(0), "queued · 0");
        assert_eq!(pill_queued(42), "queued · 42");
        assert_eq!(pill_queued(100), "queued · 99+");
        assert_eq!(pill_queued(501), "queued · 500+");
    }

    #[test]
    fn pill_tooltip_shapes_match_spec() {
        assert_eq!(
            pill_tooltip_out(2, "alice"),
            "you have 2 messages waiting for alice"
        );
        assert_eq!(
            pill_tooltip_in("alice", 3),
            "alice has 3 messages pending for you"
        );
        assert_eq!(
            pill_tooltip_both(2, "alice", 1),
            "2 waiting for alice · 1 pending from them"
        );
    }

    #[test]
    fn msg_note_queued_interpolates_peer_or_grove() {
        assert_eq!(
            msg_note_queued("alice"),
            "queued · will send when alice reachable"
        );
    }

    #[test]
    fn toast_reconnected_switches_on_count() {
        assert_eq!(toast_reconnected(0), "reconnected");
        assert_eq!(toast_reconnected(5), "reconnected · delivering 5 messages");
    }

    #[test]
    fn banner_welcome_back_verbatim() {
        assert_eq!(
            banner_welcome_back(12),
            "willow queued 12 messages while you were away — everything arrived"
        );
    }

    #[test]
    fn reconnect_gate_is_60s() {
        // Locks the spec-driven 60 s offline gate for the toast + banner.
        assert_eq!(RECONNECT_GATE_TICKS, 60);
    }

    #[test]
    fn toast_mark_read_failed_pluralises() {
        assert_eq!(toast_mark_read_failed(1), "failed to mark 1 peer as read");
        assert_eq!(toast_mark_read_failed(3), "failed to mark 3 peers as read");
    }
}
