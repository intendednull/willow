//! Long-press avatar primitive.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/trust-verification.md`
//! §Long-press SAS on mobile.
//!
//! A press-and-hold on a peer avatar for ≥ 350 ms (or keyboard Enter
//! when focused) triggers `on_trigger` — usually `begin_compare`. A
//! 2 px `--moss-2` ring grows in opacity from 0 → 1 over the hold
//! duration; releasing before the threshold fades the ring without
//! triggering. `prefers-reduced-motion: reduce` collapses the growth
//! animation to opacity-only (CSS-side).

use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// The hold duration that arms the long-press (spec: 350 ms).
const HOLD_MS: i32 = 350;

/// Wrap `children` in a focusable, long-pressable surface.
///
/// When the user holds pointerdown for at least [`HOLD_MS`] or presses
/// Enter with the wrapper focused, `on_trigger` fires. A visible ring
/// animates during the hold and brightens briefly when armed.
#[component]
pub fn LongPressAvatar(
    /// Called when the press completes successfully.
    on_trigger: Callback<()>,
    /// Accessible name surfaced to SRs. Pair with the avatar's own
    /// label so screen readers read something like
    /// `open verify dialog for alice`.
    #[prop(into, default = "compare fingerprints".to_string())]
    label: String,
    children: Children,
) -> impl IntoView {
    // Ring state: None = idle, Some(false) = growing, Some(true) = armed.
    let (ring, set_ring) = signal(None::<bool>);
    // Timer handle so release can cancel it cleanly.
    let timer = StoredValue::new(None::<i32>);

    let on_trigger = StoredValue::new(on_trigger);

    let start = move || {
        set_ring.set(Some(false));
        let set_ring_for_timer = set_ring;
        let on_trigger_inner = on_trigger;
        // `once_into_js` transfers ownership to JS so the closure is
        // reclaimed by GC after fire (or after `clear_timeout_with_handle`
        // in `cancel`). `Closure::once(...).forget()` would leak one
        // closure per pointerdown / touchstart (issue #193).
        let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
            set_ring_for_timer.set(Some(true));
            // Light haptic + fire the callback. Headless browsers
            // (wasm-bindgen-test) lack `navigator.vibrate`, so
            // feature-detect before calling.
            if let Some(nav) = web_sys::window().map(|w| w.navigator()) {
                if js_sys::Reflect::has(nav.as_ref(), &"vibrate".into()).unwrap_or(false) {
                    nav.vibrate_with_duration(8);
                }
            }
            on_trigger_inner.get_value().run(());
        });
        if let Some(win) = web_sys::window() {
            if let Ok(handle) = win
                .set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), HOLD_MS)
            {
                timer.set_value(Some(handle));
            }
        }
    };

    let cancel = move || {
        if let Some(handle) = timer.get_value() {
            if let Some(win) = web_sys::window() {
                win.clear_timeout_with_handle(handle);
            }
            timer.set_value(None);
        }
        set_ring.set(None);
    };

    // Armed state hides after a beat so repeated presses still feel alive.
    Effect::new(move |_| {
        if let Some(true) = ring.get() {
            let set_ring_off = set_ring;
            // One-shot fade-off timer. `once_into_js` lets JS reclaim
            // the closure after fire — this Effect re-runs on every
            // arm, so `forget()` would leak per long-press (issue #193).
            let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                set_ring_off.set(None);
            });
            if let Some(win) = web_sys::window() {
                win.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 180)
                    .ok();
            }
        }
    });

    let on_down = move |_ev: web_sys::MouseEvent| start();
    let on_up = move |_ev: web_sys::MouseEvent| cancel();
    let on_leave = move |_ev: web_sys::MouseEvent| cancel();
    let on_touch_start = move |_ev: web_sys::TouchEvent| start();
    let on_touch_end = move |_ev: web_sys::TouchEvent| cancel();
    let on_key = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Enter" || ev.key() == " " {
            ev.prevent_default();
            on_trigger.get_value().run(());
        }
    };

    view! {
        <span
            class="long-press-avatar"
            tabindex="0"
            role="button"
            aria-label=label
            on:mousedown=on_down
            on:mouseup=on_up
            on:mouseleave=on_leave
            on:touchstart=on_touch_start
            on:touchend=on_touch_end
            on:keydown=on_key
        >
            {children()}
            {move || {
                ring.get().map(|armed| {
                    let mut class = String::from("sas-press-ring sas-press-ring--growing");
                    if armed { class.push_str(" sas-press-ring--armed"); }
                    view! { <span class=class aria-hidden="true"></span> }
                })
            }}
        </span>
    }
}
