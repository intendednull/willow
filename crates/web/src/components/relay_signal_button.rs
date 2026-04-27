//! Relay signal button — Phase 2b sync queue.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/sync-queue.md` §Relay
//! awareness §Sync queue screen. Signal-icon button rendered in the
//! sync-queue screen header.
//!
//! - Reachable relay: `--moss-3` tint.
//! - Unreachable relay: `--amber` tint with the 2 s `willowPulse`
//!   at 40 % intensity handled entirely via CSS.
//! - Not configured: `--ink-3` tint (neutral).
//!
//! Clicking opens a lightweight popover (desktop) anchored under the
//! button — the mobile bottom-sheet variant is handled by the same
//! popover container on narrow viewports via the shared media query
//! in `style.css` (`@media (max-width: 720px) { .relay-popover { …
//! } }`). Popover body surfaces the status label, the approximate
//! count of direct-peer attempts in progress (derived from
//! `QueueView::per_peer.len()`), and a `change relay in settings`
//! link. The link is a no-op until `settings-tweaks.md` ships the
//! relay picker — it routes through the existing settings toggle so
//! the affordance is discoverable today.

use leptos::prelude::*;
use willow_client::RelayStatus;

use crate::components::sync_queue_copy;
use crate::icons;
use crate::state::{AppState, AppWriteSignals};

/// Signal-icon button with an anchored popover.
///
/// The button is non-interactive when the relay status is
/// `NotConfigured` — there is no popover to show because nothing has
/// been set up yet. That branch surfaces the plain icon without a
/// click handler, matching the spec's "idle glyph" cue.
#[component]
pub fn RelaySignalButton() -> impl IntoView {
    let app =
        use_context::<AppState>().expect("<RelaySignalButton> mounted outside an AppState context");
    let write = use_context::<AppWriteSignals>()
        .expect("<RelaySignalButton> requires AppWriteSignals in context");
    let relay = app.queue.relay_status;
    let queue_view = app.queue.view;
    let set_show_settings = write.ui.set_show_settings;

    let open = RwSignal::new(false);

    // Settings-link button receives focus when the popover opens, so
    // keyboard users land inside the dialog instead of skipping past
    // its contents. Same pattern used by `ConfirmDialog`.
    let settings_link_ref = NodeRef::<leptos::html::Button>::new();

    // Pull focus into the popover on each open transition. The global
    // keybinding stack (`crate::keybindings`) does not know about this
    // popover, so we own the initial-focus contract locally. The dialog
    // is non-modal (`aria-modal="false"`) — we do not trap focus, just
    // seed it; Tab continues to move through page chrome normally.
    Effect::new(move |prev: Option<bool>| {
        let is_open = open.get();
        let was_open = prev.unwrap_or(false);
        if is_open && !was_open {
            request_animation_frame(move || {
                if let Some(el) = settings_link_ref.get_untracked() {
                    let _ = el.focus();
                }
            });
        }
        is_open
    });

    // Local Escape handler scoped to the popover. The global handler in
    // `keybindings::install` only knows about the rail / palette / sheet
    // stack; this popover is outside that stack, so without a local
    // listener Escape would be a no-op while focus is inside the dialog.
    let on_popover_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Escape" {
            ev.stop_propagation();
            open.set(false);
        }
    };

    let class_for = move || match relay.get() {
        RelayStatus::Reachable => "relay-signal-button relay-signal-button--ok",
        RelayStatus::Unreachable => "relay-signal-button relay-signal-button--warn",
        RelayStatus::NotConfigured => "relay-signal-button relay-signal-button--idle",
    };
    let aria_for = move || match relay.get() {
        RelayStatus::Reachable => "relay reachable",
        RelayStatus::Unreachable => "relay unreachable",
        RelayStatus::NotConfigured => "no relay configured",
    };
    let popover_header = move || match relay.get() {
        RelayStatus::Reachable => "relay reachable".to_string(),
        RelayStatus::Unreachable => sync_queue_copy::RELAY_UNREACHABLE.to_string(),
        RelayStatus::NotConfigured => "no relay configured".to_string(),
    };

    // Count of direct-peer attempts in progress — we use the
    // distinct-peer count on the outbound queue as a conservative
    // proxy until the network layer exposes a real "in-flight"
    // counter. Matches spec intent ("number of direct-peer attempts
    // in progress") without false-positives for empty queues.
    let attempts_count = move || queue_view.with(|v| v.per_peer.len() as u32);

    let toggle = move |_| {
        if matches!(relay.get(), RelayStatus::NotConfigured) {
            return;
        }
        open.update(|o| *o = !*o);
    };

    view! {
        <div class="relay-signal-wrap">
            <button
                class=class_for
                type="button"
                aria-label=aria_for
                aria-expanded=move || open.get().to_string()
                aria-haspopup="dialog"
                title=aria_for
                on:click=toggle
            >
                {icons::icon_signal()}
            </button>

            <Show when=move || open.get()>
                <div
                    class="relay-popover"
                    role="dialog"
                    aria-label="relay status"
                    aria-modal="false"
                    tabindex="-1"
                    on:keydown=on_popover_keydown
                >
                    <header class="relay-popover__header">
                        {icons::icon_signal()}
                        <span class="relay-popover__status">{popover_header}</span>
                    </header>
                    <dl class="relay-popover__body">
                        <dt>"attempts in progress"</dt>
                        <dd class="relay-popover__mono">{attempts_count}</dd>
                    </dl>
                    <button
                        class="relay-popover__settings-link"
                        type="button"
                        node_ref=settings_link_ref
                        on:click=move |_| {
                            // Close the popover + open settings. The
                            // relay-picker tab lands with
                            // `settings-tweaks.md`; until then the
                            // button opens the settings dialog at the
                            // root so the affordance isn't broken.
                            open.set(false);
                            set_show_settings.set(true);
                        }
                    >
                        "change relay in settings"
                    </button>
                </div>
                <button
                    class="relay-popover__backdrop"
                    type="button"
                    aria-label="close relay popover"
                    on:click=move |_| open.set(false)
                />
            </Show>
        </div>
    }
}
