use leptos::prelude::*;

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
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if ev.key() == "Escape" {
                            on_cancel.run(());
                        }
                    }
                    tabindex="-1"
                >
                    <div class="confirm-dialog">
                        <h3>{title}</h3>
                        <p>{msg}</p>
                        <div class="confirm-actions">
                            <button class="btn btn-secondary" on:click=move |_| on_cancel.run(())>
                                {cancel_text}
                            </button>
                            <button class=confirm_class on:click=move |_| on_confirm.run(())>
                                {confirm_text}
                            </button>
                        </div>
                    </div>
                </div>
            })
        }}
    }
}
