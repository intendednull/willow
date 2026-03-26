use leptos::prelude::*;

/// A positioned popup menu that appears at (x, y) when `visible` is true.
///
/// Click outside or press Escape to close. Children are rendered as the
/// menu items (use `.context-menu-item` buttons).
#[component]
pub fn ContextMenu(
    visible: ReadSignal<bool>,
    x: ReadSignal<f64>,
    y: ReadSignal<f64>,
    on_close: Callback<()>,
    children: Children,
) -> impl IntoView {
    // Close on outside click via a transparent overlay.
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Escape" {
            on_close.run(());
        }
    };

    let items = children();

    view! {
        <div
            class=move || if visible.get() { "context-menu-overlay open" } else { "context-menu-overlay" }
            on:click=move |_| on_close.run(())
        ></div>
        <div
            class=move || if visible.get() { "context-menu open" } else { "context-menu" }
            style=move || {
                let cx = x.get();
                let cy = y.get();
                format!("left: {cx}px; top: {cy}px;")
            }
            tabindex="-1"
            on:keydown=on_keydown
        >
            {items}
        </div>
    }
}
