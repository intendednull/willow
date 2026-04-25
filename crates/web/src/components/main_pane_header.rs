//! Main-pane header — channel title strip + action bar.
//!
//! Spec: docs/specs/2026-04-19-ui-design/layout-primitives.md §Main pane header
//!
//! Replaces `ChannelHeader`. Left to right:
//!   1. Kind icon (hash / volume / hourglass)
//!   2. Italic channel title
//!   3. Topic text, prefixed by a divider bar (truncates)
//!   4. Flex spacer
//!   5. Action buttons in fixed order: members (with count badge) ·
//!      pinned · search · more. Members and pinned are mutually
//!      exclusive — opening one closes the other. Clicking the members
//!      button opens the right-rail member pane.

use leptos::prelude::*;

use crate::icons;

/// Which pane the right rail is currently showing. `None` means the
/// rail is closed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RightRailWhich {
    #[default]
    None,
    Members,
    Pinned,
    Thread,
    /// Phase 2b sync-queue screen (mutually exclusive with the others).
    SyncQueue,
}

/// Six-button action bar + channel title strip.
#[allow(clippy::too_many_arguments)]
#[component]
pub fn MainPaneHeader(
    channel: ReadSignal<String>,
    /// Active right-rail pane, if any. Component accepts either a
    /// `ReadSignal` (preferred) or any other Leptos-compatible Signal.
    #[prop(into)]
    which: Signal<RightRailWhich>,
    /// Called with the new value when a rail-toggle button fires.
    /// None closes the rail; Some swaps in the given pane.
    #[prop(into)]
    on_set_which: Callback<RightRailWhich>,
    /// Opens the command palette.
    #[prop(into)]
    on_search_click: Callback<()>,
    /// Opens the channel "more" menu.
    #[prop(optional, into)]
    on_more_click: Option<Callback<()>>,
    /// Channel kind — drives the kind icon (text / voice / ephemeral).
    #[prop(optional, into)]
    kind: Option<Signal<willow_state::ChannelKind>>,
    /// Topic string beside the title (truncates).
    #[prop(optional, into)]
    topic: Option<Signal<String>>,
) -> impl IntoView {
    let is_members = Signal::derive(move || which.get() == RightRailWhich::Members);
    let is_pinned = Signal::derive(move || which.get() == RightRailWhich::Pinned);

    let kind_icon_view = move || {
        let k = kind
            .map(|s| s.get())
            .unwrap_or(willow_state::ChannelKind::Text);
        let name = channel.get();
        if name.starts_with("_ephemeral-") {
            icons::icon_hourglass().into_any()
        } else if matches!(k, willow_state::ChannelKind::Voice) {
            icons::icon_volume_1().into_any()
        } else {
            icons::icon_hash().into_any()
        }
    };

    view! {
        <header
            class="main-pane-header"
            role="banner"
            aria-label="channel header"
        >
            <span class="mph-kind-icon" aria-hidden="true">
                {kind_icon_view}
            </span>
            <span class="mph-title">{move || channel.get()}</span>
            {move || {
                let t = topic.map(|s| s.get()).unwrap_or_default();
                if t.is_empty() {
                    None
                } else {
                    Some(view! {
                        <>
                            <span class="mph-sep" aria-hidden="true"></span>
                            <span class="mph-topic">{t}</span>
                        </>
                    })
                }
            }}

            <div class="mph-spacer"></div>

            <div class="mph-action-bar" role="toolbar" aria-label="channel actions">
                <button
                    class=move || if is_members.get() { "action-btn action-btn--with-count active" } else { "action-btn action-btn--with-count" }
                    aria-label="members"
                    aria-pressed=move || if is_members.get() { "true" } else { "false" }
                    title="members"
                    on:click=move |_| {
                        let next = if is_members.get() { RightRailWhich::None } else { RightRailWhich::Members };
                        on_set_which.run(next);
                    }
                >
                    {icons::icon_users()}
                    {
                        use crate::state::AppState;
                        use_context::<AppState>().map(|app_state| {
                            let member_count = Signal::derive(
                                move || app_state.network.peers.get().len(),
                            );
                            view! {
                                <span class="action-btn__count">
                                    {move || member_count.get().to_string()}
                                </span>
                            }
                        })
                    }
                </button>
                <button
                    class=move || if is_pinned.get() { "action-btn active" } else { "action-btn" }
                    aria-label="pinned"
                    aria-pressed=move || if is_pinned.get() { "true" } else { "false" }
                    title="pinned"
                    on:click=move |_| {
                        let next = if is_pinned.get() { RightRailWhich::None } else { RightRailWhich::Pinned };
                        on_set_which.run(next);
                    }
                >
                    {icons::icon_pin()}
                </button>
                <button
                    class="action-btn"
                    aria-label="search (⌘K)"
                    aria-keyshortcuts="Control+K Meta+K"
                    title="search (⌘K)"
                    on:click=move |_| on_search_click.run(())
                >
                    {icons::icon_search()}
                </button>
                <button
                    class="action-btn"
                    aria-label="more"
                    title="more"
                    on:click=move |_| {
                        if let Some(cb) = on_more_click { cb.run(()); }
                    }
                >
                    {icons::icon_more_horizontal()}
                </button>
            </div>
        </header>
    }
}
