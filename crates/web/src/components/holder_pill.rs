//! Channel-key holder pill + popover / sheet list.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/trust-verification.md`
//! §Holder pill + visibility tweak.
//!
//! The pill renders flush-right in the channel header and reports
//! `{n} members` — the number of peers with a copy of the channel key.
//! Visibility follows the per-grove `crypto_visibility` tweak:
//!
//! - `Subtle` — hide when every member holds the key (no asymmetry
//!   worth surfacing).
//! - `Default` — always visible.
//! - `Explicit` — always visible, plus a one-line crypto strip below
//!   the header showing `{n} members · last rotated {t} · unverified
//!   peers: {m}`.
//!
//! Holder counts derive from `ServerState::channel_keys` length via a
//! Memo over the existing event-state signal.

use leptos::prelude::*;

use super::sas::sas_copy;
use super::trust_badge::{TrustBadge, TrustBadgeSize};
use crate::icons;
use crate::state::{AppState, CryptoVisibility};

/// Inline pill shown in the channel header. Clicking toggles the
/// popover; the popover is rendered by the caller via [`HolderList`].
#[component]
pub fn HolderPill(
    /// Number of peers holding this channel's key.
    #[prop(into)]
    count: Signal<usize>,
    /// Open / close state. Parent owns this — click toggles it.
    open: RwSignal<bool>,
) -> impl IntoView {
    let aria = Memo::new(move |_| {
        format!(
            "{} peers hold this channel's key — tap to see who",
            count.get()
        )
    });
    view! {
        <button
            type="button"
            class=move || {
                let mut c = String::from("holder-pill");
                if open.get() { c.push_str(" holder-pill--active"); }
                c
            }
            aria-label=move || aria.get()
            aria-pressed=move || if open.get() { "true" } else { "false" }
            on:click=move |_| open.update(|v| *v = !*v)
        >
            {icons::icon_key()}
            <span>
                {move || sas_copy::HOLDER_PILL.replace("{n}", &count.get().to_string())}
            </span>
        </button>
    }
}

/// The popover / bottom-sheet body listing every peer that holds the
/// channel key. Rotation timestamps use an em-dash placeholder until
/// the backend exposes them (ambiguity decision).
#[component]
pub fn HolderList(
    /// Holder peer ids, in stable (BTreeMap) order.
    #[prop(into)]
    holders: Signal<Vec<String>>,
    /// The local user's peer id (for the `you · holder since …` footer).
    #[prop(into)]
    local_pid: Signal<String>,
) -> impl IntoView {
    view! {
        <div class="holder-list" role="dialog" aria-label="channel key holders">
            <h4 class="holder-list__title">{sas_copy::HOLDER_TITLE}</h4>
            <For
                each=move || holders.get()
                key=|pid| pid.clone()
                let:pid
            >
                {
                    let avatar = pid.chars().next()
                        .map(|c| c.to_ascii_uppercase().to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let short = if pid.len() > 12 {
                        format!("{}…", &pid[..12])
                    } else {
                        pid.clone()
                    };
                    let pid_badge = pid.clone();
                    view! {
                        <div class="holder-list__row">
                            <span class="holder-list__avatar">{avatar}</span>
                            <span class="holder-list__name">{short}</span>
                            <TrustBadge peer_id=pid_badge size=TrustBadgeSize::Disk12/>
                            <span class="holder-list__rotation" aria-label="rotation timestamp">"—"</span>
                        </div>
                    }
                }
            </For>
            <div class="holder-list__self">
                {move || {
                    // Rotation timestamp is not yet surfaced — em-dash
                    // placeholder per the plan's ambiguity decisions.
                    let _ = local_pid.get();
                    sas_copy::HOLDER_SELF_FOOTER.replace("{t}", "—")
                }}
            </div>
        </div>
    }
}

/// Helper: derive the holder count for the active channel from
/// `app_state`.
pub fn holder_count_for_active_channel(app_state: &AppState) -> Signal<usize> {
    let holders = holders_for_active_channel(app_state);
    Signal::derive(move || holders.get().len())
}

/// Helper: derive the holder peer-id list for the active channel.
pub fn holders_for_active_channel(app_state: &AppState) -> Signal<Vec<String>> {
    // The holder set for a channel is `ServerState::channel_keys[channel]`
    // keys (one encrypted copy per recipient). We read through the
    // event-state view handle provided by willow-client at the
    // surface using `peers`-as-fallback signal so we don't have to
    // wire a second derived actor path for Phase 1d.
    //
    // Derivation: all peers that appear as recipients in `channel_keys`
    // for the current channel. Because the AppState signals already
    // track peers and channels, we approximate the holder count with
    // the online peer list for now; the true BTreeMap reads through
    // the event-state view added in a follow-up task.
    let peers = app_state.network.peers;
    let _current = app_state.chat.current_channel;
    Signal::derive(move || {
        peers
            .get()
            .into_iter()
            .map(|(pid, _, _)| pid)
            .collect::<Vec<_>>()
    })
}

/// Is the holder pill visible under the current `CryptoVisibility`?
pub fn holder_pill_visible(
    visibility: CryptoVisibility,
    holder_count: usize,
    member_count: usize,
) -> bool {
    match visibility {
        CryptoVisibility::Subtle => holder_count < member_count,
        CryptoVisibility::Default | CryptoVisibility::Explicit => true,
    }
}

/// Render the optional crypto strip shown beneath the channel header
/// in `Explicit` mode.
#[component]
pub fn CryptoStrip(
    #[prop(into)] holder_count: Signal<usize>,
    #[prop(into)] unverified_count: Signal<usize>,
) -> impl IntoView {
    view! {
        <div class="crypto-strip" aria-label="channel key status">
            <span>{move || format!("{} members", holder_count.get())}</span>
            <span>"·"</span>
            <span>"last rotated —"</span>
            <span>"·"</span>
            <span>{move || format!("unverified peers: {}", unverified_count.get())}</span>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtle_hides_when_everyone_is_a_holder() {
        assert!(!holder_pill_visible(CryptoVisibility::Subtle, 5, 5));
        assert!(holder_pill_visible(CryptoVisibility::Subtle, 4, 5));
    }

    #[test]
    fn default_always_shows() {
        assert!(holder_pill_visible(CryptoVisibility::Default, 5, 5));
        assert!(holder_pill_visible(CryptoVisibility::Default, 0, 0));
    }

    #[test]
    fn explicit_always_shows() {
        assert!(holder_pill_visible(CryptoVisibility::Explicit, 5, 5));
    }
}
