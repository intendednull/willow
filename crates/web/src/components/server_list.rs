use leptos::prelude::*;

/// Discord-style vertical server icon rail on the far left.
#[component]
pub fn ServerList(
    server_name: ReadSignal<String>,
    on_settings_click: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView {
    // First letter of server name as the icon
    let initial = move || {
        let name = server_name.get();
        name.chars().next().unwrap_or('W').to_uppercase().to_string()
    };

    view! {
        <div class="server-rail">
            // Active server icon
            <div class="server-icon active" title=move || server_name.get()>
                {initial}
            </div>

            <div class="server-rail-divider"></div>

            // Add server button (join via invite)
            <div
                class="server-icon add-server"
                title="Join or Create Server"
                on:click=move |_| on_settings_click(())
            >
                "+"
            </div>
        </div>
    }
}
