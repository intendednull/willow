use leptos::prelude::*;

/// Chat input field. Sends a message on Enter (without Shift).
#[component]
pub fn ChatInput(on_send: impl Fn(String) + Send + Clone + 'static) -> impl IntoView {
    let (input_text, set_input_text) = signal(String::new());

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Enter" && !ev.shift_key() {
            ev.prevent_default();
            let text = input_text.get_untracked();
            if !text.trim().is_empty() {
                on_send(text);
                set_input_text.set(String::new());
            }
        }
    };

    view! {
        <div class="input-area">
            <input
                type="text"
                placeholder="Message #channel"
                prop:value=move || input_text.get()
                on:input=move |ev| {
                    set_input_text.set(event_target_value(&ev));
                }
                on:keydown=on_keydown
            />
        </div>
    }
}
