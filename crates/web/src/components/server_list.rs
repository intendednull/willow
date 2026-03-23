use leptos::prelude::*;

/// Discord-style vertical server icon rail on the far left.
#[component]
pub fn ServerList(
    servers: ReadSignal<Vec<(String, String)>>,
    active_server_id: ReadSignal<String>,
    on_server_click: impl Fn(String) + Send + Clone + 'static,
    on_settings_click: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView {
    view! {
        <div class="server-rail">
            <For
                each=move || servers.get()
                key=|(id, _)| id.clone()
                let:server
            >
                {
                    let (id, name) = server;
                    let id_click = id.clone();
                    let id_active = id.clone();
                    let initial = name
                        .chars()
                        .next()
                        .unwrap_or('?')
                        .to_uppercase()
                        .to_string();
                    let on_click = on_server_click.clone();
                    view! {
                        <div
                            class=move || {
                                if active_server_id.get() == id_active {
                                    "server-icon active"
                                } else {
                                    "server-icon"
                                }
                            }
                            title=name.clone()
                            on:click=move |_| on_click(id_click.clone())
                        >
                            {initial}
                        </div>
                    }
                }
            </For>

            <div class="server-rail-divider"></div>

            // Join/create server button
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
