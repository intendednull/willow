//! Phase 2d — `temp` kind tab + idle-threshold field.
//!
//! Surfaces inside the existing "new channel" picker. Default
//! threshold 14 days, capped at 90.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/ephemeral-channels.md`
//! §Spawn flows.

use leptos::prelude::*;

use super::kind_chip::KindChipKind;

const DEFAULT_DAYS: u32 = 14;
const CAP_DAYS: u32 = 90;
const MIN_DAYS: u32 = 1;

/// `temp`-kind create form — exposes a days-idle stepper +
/// helper-copy line. Mounts inline in the channel sidebar's
/// new-channel picker when the `temp` tab is selected.
#[component]
pub fn TempChannelCreateForm(
    #[prop(optional)] initial_kind: Option<KindChipKind>,
    #[prop(optional)] on_change: Option<Callback<u32>>,
) -> impl IntoView {
    let _ = initial_kind; // currently always `temp` in v1
    let (days, set_days) = signal(DEFAULT_DAYS);

    // Notify the parent (or just no-op when unset) so the parent
    // can pass the live threshold to `create_ephemeral_channel`
    // when the user submits.
    Effect::new(move |_| {
        if let Some(cb) = on_change {
            cb.run(days.get());
        }
    });

    let on_input = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(input) = target {
            let raw = input.value().parse::<u32>().unwrap_or(DEFAULT_DAYS);
            let clamped = raw.clamp(MIN_DAYS, CAP_DAYS);
            // Reflect the clamped value back into the DOM so the
            // user sees the cap enforce visually.
            input.set_value(&clamped.to_string());
            set_days.set(clamped);
        }
    };

    view! {
        <div class="temp-create">
            <label class="temp-create-label" for="temp-idle-threshold-days">
                "archives after"
            </label>
            <input
                id="temp-idle-threshold-days"
                name="temp-idle-threshold-days"
                type="number"
                min={MIN_DAYS as i32}
                max={CAP_DAYS as i32}
                prop:value=move || days.get().to_string()
                on:input=on_input
            />
            <span class="temp-create-suffix">"days idle"</span>
            <p class="temp-create-helper">
                {move || format!(
                    "archives if no one posts for {} days. anyone can revive it by posting again.",
                    days.get()
                )}
            </p>
        </div>
    }
}

/// Default idle threshold (in days) used by the temp create form.
pub const TEMP_DEFAULT_DAYS: u32 = DEFAULT_DAYS;
/// Cap on the days-idle slider (matches the state-layer 90-day cap).
pub const TEMP_CAP_DAYS: u32 = CAP_DAYS;
