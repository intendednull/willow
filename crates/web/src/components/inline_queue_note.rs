//! Inline queue note — Phase 2b sync queue.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/sync-queue.md` §Per-message
//! queue note. Renders below the message body for three states:
//!
//! - `Queued`         — "queued · will send when {peer} reachable"
//! - `JustDelivered`  — "queued earlier · delivered just now"
//! - `InboundHeld`    — "sent earlier · arrived now"
//!
//! Wired into `message.rs` below the body; the row's `aria-describedby`
//! points at the note's `id` so screen readers announce the hint
//! alongside the message.

use leptos::prelude::*;

use crate::components::sync_queue_copy;
use crate::icons;

/// Three-state variant for the inline note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineState {
    /// Local author, still pending ack from at least one recipient.
    Queued,
    /// Transitioned Pending → delivered within the last 30 s.
    JustDelivered,
    /// Remote author; message arrived late (peer was offline at
    /// authoring time).
    InboundHeld,
}

/// Inline queue-note renderer.
///
/// Props:
/// - `state` — the three-state variant.
/// - `peer_or_grove` — the name of the peer or grove to reference in
///   the `queued · will send when {peer_or_grove} reachable` copy.
/// - `message_id` — used to stamp a unique DOM id so the message row
///   can reference it via `aria-describedby`.
#[component]
pub fn InlineQueueNote(
    #[prop(into)] state: Signal<InlineState>,
    #[prop(into)] peer_or_grove: Signal<String>,
    #[prop(into)] message_id: Signal<String>,
) -> impl IntoView {
    let text = move || match state.get() {
        InlineState::Queued => sync_queue_copy::msg_note_queued(&peer_or_grove.get()),
        InlineState::JustDelivered => sync_queue_copy::MSG_NOTE_JUST_DELIVERED.to_string(),
        InlineState::InboundHeld => sync_queue_copy::MSG_NOTE_INBOUND_HELD.to_string(),
    };

    let color_class = move || match state.get() {
        InlineState::Queued => "inline-note--queued",
        InlineState::JustDelivered => "inline-note--just-delivered",
        InlineState::InboundHeld => "inline-note--inbound-held",
    };

    let icon = move || match state.get() {
        InlineState::Queued => icons::icon_hourglass_sm().into_any(),
        InlineState::JustDelivered => icons::icon_check_small().into_any(),
        InlineState::InboundHeld => icons::icon_leaf().into_any(),
    };

    let class_attr = move || format!("inline-note {}", color_class());
    let id_attr = move || format!("qn-{}", message_id.get());

    view! {
        <span class=class_attr id=id_attr role="note">
            {icon}
            <em>{text}</em>
        </span>
    }
}
