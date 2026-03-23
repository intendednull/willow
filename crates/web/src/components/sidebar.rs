use leptos::prelude::*;

/// Left sidebar showing the server name, channel list, and user info.
#[component]
pub fn Sidebar(
    channels: ReadSignal<Vec<String>>,
    current_channel: ReadSignal<String>,
    peer_id: ReadSignal<String>,
    on_channel_click: impl Fn(String) + Send + Clone + 'static,
    on_settings_click: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView {
    view! {
        <div class="sidebar">
            <div class="sidebar-header">"Willow"</div>
            <div class="channel-list">
                <For
                    each=move || channels.get()
                    key=|ch| ch.clone()
                    let:channel
                >
                    {
                        let ch_active = channel.clone();
                        let ch_click = channel.clone();
                        let on_click = on_channel_click.clone();
                        let active = move || current_channel.get() == ch_active;
                        view! {
                            <div
                                class=move || if active() { "channel-item active" } else { "channel-item" }
                                on:click=move |_| on_click(ch_click.clone())
                            >
                                <span>"# " {channel.clone()}</span>
                            </div>
                        }
                    }
                </For>
            </div>
            <div class="user-area">
                <div class="status-dot"></div>
                <span style="font-size: 12px; color: var(--text-muted);">
                    {move || {
                        let id = peer_id.get();
                        if id.len() > 12 { format!("{}...", &id[..12]) } else { id }
                    }}
                </span>
                <button
                    class="btn btn-sm"
                    style="margin-left: auto; background: transparent; color: var(--text-muted);"
                    on:click=move |_| on_settings_click(())
                >
                    "Settings"
                </button>
            </div>
        </div>
    }
}
