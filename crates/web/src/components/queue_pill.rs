//! Per-peer queue pill — Phase 2b sync queue.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/sync-queue.md` §Per-peer
//! badge. Amber `queued · {n}` pill rendered on letter rows + member
//! rows. Suppressed when the peer is `PeerTrust::PendingVerify` — the
//! verify prompt takes precedence and the pending count surfaces in
//! the tooltip instead.

use leptos::prelude::*;
use willow_client::trust::PeerTrust;
use willow_identity::EndpointId;

use crate::icons;
use crate::state::AppState;

/// Amber `queued · {n}` pill for a single peer row.
///
/// Props:
/// - `peer_id` — the peer the pill belongs to.
/// - `display_name` — used in the disambiguated aria-label.
#[component]
pub fn QueuePill(
    #[prop(into)] peer_id: Signal<EndpointId>,
    #[prop(into)] display_name: Signal<String>,
) -> impl IntoView {
    let app = use_context::<AppState>().expect("<QueuePill> mounted outside an AppState context");
    let queue_view = app.queue.view;
    let trust_map = app.trust.trust_map;

    // Hide pill when the peer is still in the trust-verify gate.
    let suppress = move || {
        let pid_str = peer_id.get().to_string();
        matches!(
            trust_map.get().get(&pid_str),
            Some(PeerTrust::PendingVerify)
        )
    };

    let counts = move || {
        let v = queue_view.get();
        let pid = peer_id.get();
        let out = v.per_peer.get(&pid).map(|s| s.outbound).unwrap_or(0);
        let inb = v.inbound_per_peer.get(&pid).copied().unwrap_or(0);
        (out, inb)
    };

    let show = move || {
        let (o, i) = counts();
        (o > 0 || i > 0) && !suppress()
    };

    let pill_text = move || {
        let (out, inb) = counts();
        let n = out.saturating_add(inb);
        if n > 500 {
            "queued · 500+".to_string()
        } else if n > 99 {
            "queued · 99+".to_string()
        } else {
            format!("queued · {n}")
        }
    };

    let aria_label = move || {
        let (out, inb) = counts();
        let name = display_name.get();
        match (out, inb) {
            (o, 0) => format!("you have {o} messages waiting for {name}"),
            (0, i) => format!("{name} has {i} messages pending for you"),
            (o, i) => format!("{o} waiting for {name} · {i} pending from them"),
        }
    };

    view! {
        <Show when=show>
            <button
                class="queue-pill"
                aria-label=aria_label
                type="button"
                title=aria_label
            >
                {icons::icon_hourglass_sm()}
                <span aria-hidden="true">{pill_text}</span>
            </button>
        </Show>
    }
}
