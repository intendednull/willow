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

use leptos::prelude::*;
use wasm_bindgen::JsCast;

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
