//! `<MetaRow>` — composer meta row beneath the textarea.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/composer.md`
//! §Desktop compose surface — Meta row, §Mobile compose surface, and
//! §Offline state. Source-of-truth copy lives in §Copy → Composer meta.
//!
//! Three rendered forms:
//!
//! 1. Desktop online: `lock` + `sealed with grove-keys` (`--ink-3`),
//!    `·` separator (`--ink-4`), `ear` + `hold shift to whisper`
//!    (with `shift` rendered in mono `--whisper`), flex spacer, then
//!    a `{name} is whispering` slot kept empty until whisper-mode
//!    ships (see `whisper-mode.md`).
//! 2. Mobile online: `lock` + `sealed to {N} peers in grove` (`{N}` =
//!    `peer_count` in the current channel), `·` separator, `ear` +
//!    `tap ear to whisper`.
//! 3. Offline (both shells): `hourglass` + `offline · queuing messages`
//!    rendered in `--amber`. This replaces the online forms entirely.
//!
//! `connection == ConnectionState::Reconnecting` is treated as offline
//! for meta-row purposes — the user's send still routes to the queue
//! and can't reach peers, so the queuing copy is the truthful state.
//! `Connecting` keeps the online copy as a neutral placeholder while
//! the relay session is still being established.

use leptos::prelude::*;

use crate::icons;
use crate::state::ConnectionState;

/// Composer meta row — desktop / mobile / offline variants.
#[component]
pub fn MetaRow(
    /// Live connection state. Drives the online ↔ offline form switch.
    connection: ReadSignal<ConnectionState>,
    /// Channel peer count, used in the mobile online form
    /// (`sealed to {N} peers in grove`).
    peer_count: Signal<usize>,
    /// `true` when rendering under the mobile shell (pill-small variant).
    is_mobile: Signal<bool>,
) -> impl IntoView {
    let is_offline = move || {
        matches!(
            connection.get(),
            ConnectionState::Offline | ConnectionState::Reconnecting
        )
    };

    view! {
        {move || {
            if is_offline() {
                view! { <OfflineMeta /> }.into_any()
            } else if is_mobile.get() {
                view! { <MobileOnlineMeta peer_count=peer_count /> }.into_any()
            } else {
                view! { <DesktopOnlineMeta /> }.into_any()
            }
        }}
    }
}

/// Desktop online meta — `lock · sealed with grove-keys · ear · hold
/// shift to whisper`. The trailing `{name} is whispering` status slot
/// per spec is rendered empty in v1; whisper-mode hasn't shipped yet
/// (see `whisper-mode.md`).
#[component]
fn DesktopOnlineMeta() -> impl IntoView {
    view! {
        <div class="composer__meta" aria-live="off">
            <span class="composer__meta-icon" aria-hidden="true">{icons::icon_lock()}</span>
            <span class="composer__meta-text">"sealed with grove-keys"</span>
            <span class="composer__meta-sep" aria-hidden="true">"·"</span>
            <span class="composer__meta-icon" aria-hidden="true">{icons::icon_ear()}</span>
            <span class="composer__meta-text">
                "hold "
                <kbd class="composer__kbd">"shift"</kbd>
                " to whisper"
            </span>
            // TODO(whisper-mode.md): render `{name} is whispering` here
            // with a 3-dot `willowPulse` in `--whisper` once whisper
            // peers populate. Empty slot kept so the spacing doesn't
            // shift when v2 lands.
            <span class="composer__meta-spacer" aria-hidden="true"></span>
        </div>
    }
}

/// Mobile online meta — `lock · sealed to {N} peers in grove · ear ·
/// tap ear to whisper`. `{N}` is the channel peer count per spec
/// §Mobile compose surface.
#[component]
fn MobileOnlineMeta(peer_count: Signal<usize>) -> impl IntoView {
    view! {
        <div class="composer__meta" aria-live="off">
            <span class="composer__meta-icon" aria-hidden="true">{icons::icon_lock()}</span>
            <span class="composer__meta-text">
                {move || format!("sealed to {} peers in grove", peer_count.get())}
            </span>
            <span class="composer__meta-sep" aria-hidden="true">"·"</span>
            <span class="composer__meta-icon" aria-hidden="true">{icons::icon_ear()}</span>
            <span class="composer__meta-text">"tap ear to whisper"</span>
        </div>
    }
}

/// Offline meta — `hourglass · offline · queuing messages`. Replaces
/// both desktop and mobile online forms when the device or relay has
/// dropped out (`ConnectionState::Offline | Reconnecting`).
#[component]
fn OfflineMeta() -> impl IntoView {
    view! {
        <div class="composer__meta composer__meta--offline" aria-live="polite">
            <span class="composer__meta-icon" aria-hidden="true">{icons::icon_hourglass()}</span>
            <span class="composer__meta-text">"offline · queuing messages"</span>
        </div>
    }
}
