//! Mobile grove drawer — 280 px left-edge overlay for grove switching.
//!
//! Spec: docs/specs/2026-04-19-ui-design/layout-primitives.md §Grove drawer
//!
//! Internal layout (left → right):
//!   1. 64 px grove rail column with tile glyphs.
//!   2. 216 px body column: wordmark + grove rows + me strip footer.
//!
//! Close triggers: backdrop tap, grove select, swipe-left > 60 px,
//! Escape. Swipe gesture wiring lands in task 8.

use leptos::ev::TransitionEvent;
use leptos::html::Aside;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::lifecycle::{advance, is_zero_duration, LifecycleState};
use crate::components::{PeerStatusLabel, StatusDot, StatusDotBorder, StatusDotSize};
use crate::icons;
use crate::state::AppState;

/// Left-edge drawer overlaying the current screen.
#[allow(clippy::too_many_arguments)]
#[component]
pub fn GroveDrawer(
    /// Drawer open state.
    #[prop(into)]
    open: Signal<bool>,
    /// Joined groves (id, display name).
    #[prop(into)]
    servers: Signal<Vec<(String, String)>>,
    /// Currently-active grove id.
    #[prop(into)]
    active_server_id: Signal<String>,
    /// Aggregate peer count (used in the header summary).
    #[prop(into)]
    peer_count: Signal<usize>,
    /// Self display name (rendered in the me strip).
    #[prop(into)]
    display_name: Signal<String>,
    /// Close handler — fires on backdrop tap / Escape / swipe-left.
    on_close: Callback<()>,
    /// Grove-row click. Selects the grove; drawer auto-closes.
    on_server_click: Callback<String>,
    /// "+ new grove" CTA.
    #[prop(optional, into)]
    on_new_grove: Option<Callback<()>>,
    /// Settings tile in the me strip.
    #[prop(optional, into)]
    on_open_settings: Option<Callback<()>>,
) -> impl IntoView {
    // Escape key closes the drawer while it is open.
    {
        let open_for_kb = open;
        let closure = wasm_bindgen::closure::Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(
            move |ev: web_sys::KeyboardEvent| {
                if ev.key() == "Escape" && open_for_kb.get_untracked() {
                    ev.prevent_default();
                    on_close.run(());
                }
            },
        );
        if let Some(window) = web_sys::window() {
            window
                .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref())
                .ok();
        }
        closure.forget();
    }

    // Four-phase data-state lifecycle on the inner <aside> (.grove-drawer).
    // The aside owns the `transform` transition (.grove-drawer.open
    // translates from -100% to 0); the root div only carries pointer-events
    // gating via the existing `data-open` attribute.
    //
    // - Effect mirrors `open` into `lifecycle`. On initial mount (prev.is_none())
    //   we snap to a terminal state (Open/Closed) so we don't fire a
    //   spurious Opening/Closing animation phase.
    // - on_transition_end filters on property_name() == "transform" so
    //   stray transitionend events from box-shadow / opacity / etc. are
    //   ignored (the spec's "ignore unrelated transitionend" failure mode).
    // - Reduced-motion shortcut: if computed transition-duration is 0s
    //   we snap to the terminal phase synchronously, since no
    //   transitionend will ever fire under prefers-reduced-motion: reduce.
    //
    // See docs/specs/2026-04-27-event-based-waits-design.md
    // §`data-state` attribute pattern.
    let drawer_ref: NodeRef<Aside> = NodeRef::new();
    let lifecycle = RwSignal::new(if open.get_untracked() {
        LifecycleState::Open
    } else {
        LifecycleState::Closed
    });

    Effect::new(move |prev: Option<bool>| {
        let now_open = open.get();
        // First run, or no change — snap to terminal state. Don't fire
        // Opening/Closing on initial mount.
        if prev.is_none() || prev == Some(now_open) {
            lifecycle.set(if now_open {
                LifecycleState::Open
            } else {
                LifecycleState::Closed
            });
            return now_open;
        }
        lifecycle.set(if now_open {
            LifecycleState::Opening
        } else {
            LifecycleState::Closing
        });
        // Reduced-motion shortcut: snap straight to terminal if no animation.
        if let Some(el) = drawer_ref.get_untracked() {
            if is_zero_duration(el.as_ref()) {
                lifecycle.set(advance(lifecycle.get_untracked()));
            }
        }
        now_open
    });

    let on_transition_end = move |ev: TransitionEvent| {
        // Accept either driving property: `.grove-drawer` slides via
        // `transform` by default, but the prefers-reduced-motion media
        // query in components.css swaps the transition to
        // `opacity var(--motion-slow) linear`. Both paths must advance
        // the lifecycle; `is_zero_duration` cannot short-circuit because
        // the reduced-motion duration is non-zero.
        let prop = ev.property_name();
        if prop == "transform" || prop == "opacity" {
            lifecycle.update(|s| *s = advance(*s));
        }
    };

    view! {
        <div
            class="grove-drawer-root"
            data-open=move || if open.get() { "true" } else { "false" }
            aria-hidden=move || if open.get() { "false" } else { "true" }
        >
            <div
                class="grove-drawer-backdrop"
                on:click=move |_| on_close.run(())
            ></div>
            <aside
                node_ref=drawer_ref
                class=move || if open.get() { "grove-drawer open".to_string() }
                    else { "grove-drawer".to_string() }
                data-state=move || lifecycle.get().as_str()
                on:transitionend=on_transition_end
                role="dialog"
                aria-modal="true"
                aria-label="groves"
            >
                <div class="drawer-rail" role="navigation" aria-label="grove rail">
                    <For
                        each=move || servers.get()
                        key=|(id, _)| id.clone()
                        children=move |(id, name): (String, String)| {
                            let id_for_click = id.clone();
                            let id_for_active = id.clone();
                            let glyph = name
                                .chars()
                                .next()
                                .map(|c| c.to_ascii_uppercase().to_string())
                                .unwrap_or_else(|| "·".to_string());
                            view! {
                                <button
                                    class=move || {
                                        if active_server_id.get() == id_for_active {
                                            "drawer-rail-tile active".to_string()
                                        } else {
                                            "drawer-rail-tile".to_string()
                                        }
                                    }
                                    aria-label=format!("grove {name}")
                                    on:click={
                                        let id = id_for_click.clone();
                                        move |_| on_server_click.run(id.clone())
                                    }
                                >
                                    {glyph}
                                </button>
                            }
                        }
                    />
                </div>
                <div class="drawer-body">
                    <header class="drawer-header">
                        <div class="drawer-wordmark">"willow"</div>
                        <div class="drawer-summary">
                            {move || {
                                let n = servers.get().len();
                                let m = peer_count.get();
                                format!("{n} groves · {m} peers online")
                            }}
                        </div>
                    </header>
                    <div class="drawer-grove-list">
                        <For
                            each=move || servers.get()
                            key=|(id, _)| id.clone()
                            children=move |(id, name): (String, String)| {
                                let id_for_click = id.clone();
                                let id_for_active = id.clone();
                                let glyph = name
                                    .chars()
                                    .next()
                                    .map(|c| c.to_ascii_uppercase().to_string())
                                    .unwrap_or_else(|| "·".to_string());
                                let display_name = name.clone();
                                view! {
                                    <button
                                        class=move || {
                                            if active_server_id.get() == id_for_active {
                                                "drawer-grove-row active".to_string()
                                            } else {
                                                "drawer-grove-row".to_string()
                                            }
                                        }
                                        on:click={
                                            let id = id_for_click.clone();
                                            move |_| on_server_click.run(id.clone())
                                        }
                                    >
                                        <span class="drawer-grove-glyph">{glyph}</span>
                                        <span class="drawer-grove-name">{display_name}</span>
                                    </button>
                                }
                            }
                        />
                        {move || on_new_grove.map(|cb| view! {
                            <button
                                class="drawer-grove-row drawer-grove-row--new"
                                on:click=move |_| cb.run(())
                            >
                                <span class="drawer-grove-glyph" aria-hidden="true">
                                    {icons::icon_plus()}
                                </span>
                                <span class="drawer-grove-name">"new grove"</span>
                            </button>
                        })}
                    </div>
                    <footer class="drawer-me-strip">
                        <div class="drawer-me-avatar" aria-hidden="true" style="position: relative">
                            {move || display_name.get()
                                .chars()
                                .next()
                                .map(|c| c.to_ascii_uppercase().to_string())
                                .unwrap_or_else(|| "·".to_string())}
                            {
                                // Optional — self-state pulled from AppState
                                // when the context is available. Not mandatory
                                // in test / storybook contexts.
                                use_context::<AppState>().map(|app_state| view! {
                                    <StatusDot
                                        state=app_state.presence.self_state
                                        size=StatusDotSize::MeStrip
                                        border=StatusDotBorder::Bg1
                                        ambient=true
                                    />
                                })
                            }
                        </div>
                        <div class="drawer-me-col">
                            <div class="drawer-me-name">"you"</div>
                            <div class="drawer-me-sub">
                                {use_context::<AppState>().map(|app_state| view! {
                                    <PeerStatusLabel
                                        state=app_state.presence.self_state
                                        show_dot=false
                                    />
                                })}
                            </div>
                        </div>
                        {move || on_open_settings.map(|cb| view! {
                            <button
                                class="drawer-me-settings"
                                aria-label="settings"
                                on:click=move |_| cb.run(())
                            >
                                {icons::icon_settings()}
                            </button>
                        })}
                    </footer>
                </div>
            </aside>
        </div>
    }
}
