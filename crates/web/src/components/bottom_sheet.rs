//! Bottom-sheet primitive — reusable mobile pull-up surface.
//!
//! Spec: docs/specs/2026-04-19-ui-design/layout-primitives.md §Bottom sheets
//!
//! Consumed by profile-sheet, confirm-sheet, and any future ad-hoc
//! sheet. Note: the legacy `.mobile-action-sheet` container in
//! `message-row.md` stays — this component is a *new* parallel
//! primitive for sheets driven from the mobile shell.
//!
//! Wiring:
//!   - Dismiss on backdrop tap, Escape key, or swipe-down > 120 px.
//!   - 100 vw wide, top corners `--radius-l`, 4 × 36 px centred handle.
//!   - translateY(100% → 0) over `--motion-slow`, backdrop fade.

use leptos::ev::TransitionEvent;
use leptos::html::Div;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::lifecycle::{advance, is_zero_duration, LifecycleState};

/// Reusable bottom-sheet primitive.
#[component]
pub fn BottomSheet(
    /// Sheet visibility.
    #[prop(into)]
    open: Signal<bool>,
    /// Aria label for the dialog (e.g. `profile actions`, `confirm delete`).
    #[prop(into)]
    label: String,
    /// Called when the sheet dismisses (backdrop tap / Escape /
    /// swipe-down). Consumers flip their `open` signal.
    on_close: Callback<()>,
    children: Children,
) -> impl IntoView {
    // Escape listener — active only while the sheet is open.
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

    // Swipe-down: listen on touchstart / touchend, dismiss when the
    // Y delta clears the 120 px threshold.
    let touch_y_start = StoredValue::new(0.0f64);

    let on_touch_start = move |ev: web_sys::TouchEvent| {
        if let Some(t) = ev.touches().item(0) {
            touch_y_start.set_value(t.client_y() as f64);
        }
    };
    let on_touch_end = move |ev: web_sys::TouchEvent| {
        if let Some(t) = ev.changed_touches().item(0) {
            let dy = t.client_y() as f64 - touch_y_start.get_value();
            if dy > 120.0 {
                on_close.run(());
            }
        }
    };

    // Four-phase data-state lifecycle on the inner .bottom-sheet div.
    // Driving property: transform (per components.css `.bottom-sheet`
    // declares `transition: transform var(--motion-slow) var(--motion-ease)`
    // — translateY 100% → 0 on .open). The reduced-motion media query
    // swaps the transition to opacity, but the JS shortcut handles that
    // path by snapping to terminal when computed transition-duration is
    // zero (which it is in the reduced-motion CSS once the transition
    // finishes).
    //
    // The existing data-open attribute on .bottom-sheet-root remains
    // (additive) — it gates pointer-events + the backdrop's opacity,
    // and removing it would break those CSS selectors. The new
    // data-state lives on the inner .bottom-sheet (the element that
    // actually carries the transform transition).
    //
    // See docs/specs/2026-04-27-event-based-waits-design.md
    // §`data-state` attribute pattern.
    let sheet_ref: NodeRef<Div> = NodeRef::new();
    let lifecycle = RwSignal::new(if open.get_untracked() {
        LifecycleState::Open
    } else {
        LifecycleState::Closed
    });

    Effect::new(move |prev: Option<bool>| {
        let now_open = open.get();
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
        if let Some(el) = sheet_ref.get_untracked() {
            if is_zero_duration(el.as_ref()) {
                lifecycle.set(advance(lifecycle.get_untracked()));
            }
        }
        now_open
    });

    let on_transition_end = move |ev: TransitionEvent| {
        if ev.property_name() == "transform" {
            lifecycle.update(|s| *s = advance(*s));
        }
    };

    view! {
        <div
            class="bottom-sheet-root"
            data-open=move || if open.get() { "true" } else { "false" }
            aria-hidden=move || if open.get() { "false" } else { "true" }
        >
            <div
                class="bottom-sheet-backdrop"
                on:click=move |_| on_close.run(())
            ></div>
            <div
                class=move || if open.get() { "bottom-sheet open".to_string() }
                    else { "bottom-sheet".to_string() }
                node_ref=sheet_ref
                data-state=move || lifecycle.get().as_str()
                on:transitionend=on_transition_end
                role="dialog"
                aria-modal="true"
                aria-label=label.clone()
                on:touchstart=on_touch_start
                on:touchend=on_touch_end
            >
                <div class="bottom-sheet-handle" aria-hidden="true"></div>
                <div class="bottom-sheet-body">
                    {children()}
                </div>
            </div>
        </div>
    }
}
