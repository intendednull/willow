use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// Reusable modal confirmation dialog with Cancel / Confirm buttons.
///
/// Shows an overlay with backdrop blur and a centered card. The confirm
/// button turns red when `danger` is `true`. Pressing Escape closes the
/// dialog via a keydown handler on the overlay.
#[component]
pub fn ConfirmDialog(
    /// Whether the dialog is visible.
    visible: ReadSignal<bool>,
    /// Dialog title.
    #[prop(into)]
    title: String,
    /// Descriptive message body.
    #[prop(into)]
    message: Signal<String>,
    /// Label for the confirm button (e.g. "Delete", "Leave").
    #[prop(into)]
    confirm_text: String,
    /// Label for the cancel button. Defaults to `Cancel`.
    #[prop(into, default = "Cancel".to_string())]
    cancel_text: String,
    /// When true the confirm button uses the danger (red) style.
    #[prop(default = false)]
    danger: bool,
    /// Called when the user confirms.
    on_confirm: Callback<()>,
    /// Called when the user cancels (or presses Escape).
    on_cancel: Callback<()>,
) -> impl IntoView {
    let confirm_class = if danger {
        "btn btn-danger"
    } else {
        "btn btn-primary"
    };

    let confirm_button_ref = NodeRef::<leptos::html::Button>::new();
    let cancel_button_ref = NodeRef::<leptos::html::Button>::new();

    // Auto-focus the confirm button when the dialog becomes visible so
    // keyboard users are pulled into the modal and Escape/Tab work as
    // expected per WAI-ARIA APG.
    Effect::new(move |prev: Option<bool>| {
        let is_visible = visible.get();
        let was_visible = prev.unwrap_or(false);
        if is_visible && !was_visible {
            leptos::prelude::request_animation_frame(move || {
                if let Some(el) = confirm_button_ref.get_untracked() {
                    let _ = el.focus();
                }
            });
        }
        is_visible
    });

    view! {
        {move || {
            if !visible.get() {
                return None;
            }
            let title = title.clone();
            let msg = message.get();
            let confirm_text = confirm_text.clone();
            let cancel_text = cancel_text.clone();
            Some(view! {
                <div
                    class="confirm-overlay"
                    role="dialog"
                    aria-modal="true"
                    aria-labelledby="confirm-dialog-title"
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if ev.key() == "Escape" {
                            on_cancel.run(());
                            return;
                        }
                        if ev.key() == "Tab" {
                            // Simple focus trap: wrap between confirm and
                            // cancel buttons so focus stays inside the
                            // dialog while it is open.
                            let target = ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok());
                            let Some(target_el) = target else { return; };
                            if ev.shift_key() {
                                if let Some(cancel_el) = cancel_button_ref.get_untracked() {
                                    let cancel_node: &web_sys::Element = &cancel_el;
                                    if cancel_node.is_same_node(Some(&target_el)) {
                                        if let Some(confirm_el) = confirm_button_ref.get_untracked() {
                                            ev.prevent_default();
                                            let _ = confirm_el.focus();
                                        }
                                    }
                                }
                            } else if let Some(confirm_el) = confirm_button_ref.get_untracked() {
                                let confirm_node: &web_sys::Element = &confirm_el;
                                if confirm_node.is_same_node(Some(&target_el)) {
                                    if let Some(cancel_el) = cancel_button_ref.get_untracked() {
                                        ev.prevent_default();
                                        let _ = cancel_el.focus();
                                    }
                                }
                            }
                        }
                    }
                    tabindex="-1"
                >
                    <div class="confirm-dialog">
                        <h3 id="confirm-dialog-title">{title}</h3>
                        <p>{msg}</p>
                        <div class="confirm-actions">
                            <button
                                class="btn btn-secondary"
                                node_ref=cancel_button_ref
                                on:click=move |_| on_cancel.run(())
                            >
                                {cancel_text}
                            </button>
                            <button
                                class=confirm_class
                                node_ref=confirm_button_ref
                                on:click=move |_| on_confirm.run(())
                            >
                                {confirm_text}
                            </button>
                        </div>
                    </div>
                </div>
            })
        }}
    }
}
