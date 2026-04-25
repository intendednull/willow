//! Right rail — the 280 px `--bg-1` pane hosting members / pinned /
//! thread. Exactly one of the three children mounts at a time; the
//! rail itself slides in (transform, 180 ms) on mount and the
//! children cross-fade (opacity, 120 ms) when swapped.
//!
//! Spec: docs/specs/2026-04-19-ui-design/layout-primitives.md §Right rail

use leptos::prelude::*;
use willow_client::DisplayMessage;

use crate::components::{MemberList, PinnedPanel, RightRailWhich, SyncQueueView};

/// Wrapper that mounts exactly one of MemberList / PinnedPanel /
/// thread-stub based on `which`.
#[component]
pub fn RightRail(
    /// Which pane to show. `None` closes the rail.
    #[prop(into)]
    which: Signal<RightRailWhich>,
    /// Called when the rail wants to close (eg. Esc on an empty state).
    #[prop(optional, into)]
    on_close: Option<Callback<()>>,
    /// Members list props.
    peers: ReadSignal<Vec<(String, String, bool)>>,
    peer_id: ReadSignal<String>,
    /// Pinned list props.
    pinned_messages: ReadSignal<Vec<DisplayMessage>>,
    /// Called when the user clicks "jump" on a pinned message.
    #[prop(into)]
    on_pinned_jump: Callback<String>,
) -> impl IntoView {
    let is_open = Signal::derive(move || !matches!(which.get(), RightRailWhich::None));
    let aria_label = move || match which.get() {
        RightRailWhich::Members => "members",
        RightRailWhich::Pinned => "pinned",
        RightRailWhich::Thread => "thread",
        RightRailWhich::SyncQueue => "sync queue",
        RightRailWhich::None => "",
    };

    view! {
        <aside
            class="right-rail"
            data-open=move || if is_open.get() { "true" } else { "false" }
            role="complementary"
            aria-label=aria_label
            aria-hidden=move || if is_open.get() { "false" } else { "true" }
        >
            <div class="right-rail-inner">
                {move || match which.get() {
                    RightRailWhich::Members => {
                        let on_close_cb = on_close;
                        view! {
                            <div class="right-rail-pane" data-pane="members">
                                <MemberList
                                    peers=peers
                                    peer_id=peer_id
                                    on_close=Callback::new(move |_| {
                                        if let Some(cb) = on_close_cb { cb.run(()); }
                                    })
                                />
                            </div>
                        }.into_any()
                    },
                    RightRailWhich::Pinned => {
                        let on_jump = on_pinned_jump;
                        let on_close_cb = on_close;
                        view! {
                            <div class="right-rail-pane" data-pane="pinned">
                                <PinnedPanel
                                    messages=pinned_messages
                                    on_jump=move |id: String| on_jump.run(id)
                                    on_close=move |_| {
                                        if let Some(cb) = on_close_cb { cb.run(()); }
                                    }
                                />
                            </div>
                        }.into_any()
                    },
                    RightRailWhich::Thread => view! {
                        <div class="right-rail-pane" data-pane="thread">
                            <div class="thread-stub state-empty">
                                <div class="state-empty__headline">"no thread yet"</div>
                                <div class="state-empty__hint">"open a thread from any message."</div>
                            </div>
                        </div>
                    }.into_any(),
                    RightRailWhich::SyncQueue => view! {
                        <div class="right-rail-pane" data-pane="sync-queue">
                            <SyncQueueView/>
                        </div>
                    }.into_any(),
                    RightRailWhich::None => view! { <div class="right-rail-pane right-rail-pane--empty"></div> }.into_any(),
                }}
            </div>
        </aside>
    }
}
