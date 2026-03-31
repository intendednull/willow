use leptos::prelude::*;

use crate::icons;
use crate::state::AppState;

/// Result item category for display grouping.
#[derive(Clone, PartialEq)]
enum PaletteCategory {
    Channel,
    Server,
    Member,
}

/// A single search result item.
#[derive(Clone)]
struct PaletteItem {
    label: String,
    /// Secondary identifier (channel name, server id, peer id).
    id: String,
    category: PaletteCategory,
    /// Whether this is a voice channel (only relevant for Channel category).
    is_voice: bool,
}

/// Build the filtered result list from the current query and pre-fetched data.
fn build_results(
    channels: &[String],
    channel_kinds: &[(String, String)],
    servers: &[(String, String)],
    members: &[(String, String, bool)],
    query: &str,
) -> Vec<PaletteItem> {
    let q = query.to_lowercase();
    let mut items: Vec<PaletteItem> = Vec::new();

    // Channels.
    for ch in channels {
        let is_voice = channel_kinds.iter().any(|(n, k)| n == ch && k == "voice");
        if q.is_empty() || ch.to_lowercase().contains(&q) {
            items.push(PaletteItem {
                label: ch.clone(),
                id: ch.clone(),
                category: PaletteCategory::Channel,
                is_voice,
            });
        }
    }

    // Servers.
    for (id, name) in servers {
        if q.is_empty() || name.to_lowercase().contains(&q) {
            items.push(PaletteItem {
                label: name.clone(),
                id: id.clone(),
                category: PaletteCategory::Server,
                is_voice: false,
            });
        }
    }

    // Members.
    for (pid, name, _online) in members {
        if q.is_empty() || name.to_lowercase().contains(&q) || pid.to_lowercase().contains(&q) {
            items.push(PaletteItem {
                label: name.clone(),
                id: pid.clone(),
                category: PaletteCategory::Member,
                is_voice: false,
            });
        }
    }

    items
}

/// Command palette overlay triggered by Ctrl+K / Cmd+K.
///
/// Provides fuzzy search across channels, servers, and members.
/// Arrow keys navigate, Enter selects, Escape closes.
#[component]
pub fn CommandPalette(
    on_close: Callback<()>,
    on_switch_channel: Callback<String>,
    on_switch_server: Callback<String>,
    on_open_members: Callback<()>,
) -> impl IntoView {
    let app_state = use_context::<AppState>().unwrap();

    let (query, set_query) = signal(String::new());
    let (selected_index, set_selected_index) = signal(0usize);

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        let channels = app_state.chat.channels.get_untracked();
        let channel_kinds = app_state.server.channel_kinds.get_untracked();
        let servers = app_state.server.servers.get_untracked();
        let members = app_state.network.peers.get_untracked();
        let items = build_results(&channels, &channel_kinds, &servers, &members, &query.get_untracked());
        let len = items.len();
        match ev.key().as_str() {
            "Escape" => {
                ev.prevent_default();
                on_close.run(());
            }
            "ArrowDown" => {
                ev.prevent_default();
                if len > 0 {
                    set_selected_index.update(|i| *i = (*i + 1) % len);
                }
            }
            "ArrowUp" => {
                ev.prevent_default();
                if len > 0 {
                    set_selected_index.update(|i| {
                        *i = if *i == 0 { len - 1 } else { *i - 1 };
                    });
                }
            }
            "Enter" => {
                ev.prevent_default();
                let idx = selected_index.get_untracked();
                if idx < items.len() {
                    let item = &items[idx];
                    match item.category {
                        PaletteCategory::Channel => on_switch_channel.run(item.id.clone()),
                        PaletteCategory::Server => on_switch_server.run(item.id.clone()),
                        PaletteCategory::Member => on_open_members.run(()),
                    }
                    on_close.run(());
                }
            }
            _ => {}
        }
    };

    view! {
        <div class="palette-overlay" on:click=move |_| on_close.run(())>
            <div class="palette" on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()>
                <input
                    class="palette-input"
                    type="text"
                    placeholder="Search channels, servers, members..."
                    prop:value=move || query.get()
                    on:input=move |ev| {
                        set_query.set(event_target_value(&ev));
                        set_selected_index.set(0);
                    }
                    on:keydown=on_keydown
                    autofocus=true
                />
                <div class="palette-results">
                    {move || {
                        let channels = app_state.chat.channels.get();
                        let channel_kinds = app_state.server.channel_kinds.get();
                        let servers = app_state.server.servers.get();
                        let members = app_state.network.peers.get();
                        let items = build_results(&channels, &channel_kinds, &servers, &members, &query.get());
                        if items.is_empty() {
                            return view! {
                                <div class="palette-empty">"No results found"</div>
                            }.into_any();
                        }
                        let sel = selected_index.get();
                        let views: Vec<_> = items.iter().enumerate().map(|(i, item)| {
                            let item_for_click = item.clone();
                            let class = if i == sel {
                                "palette-item selected"
                            } else {
                                "palette-item"
                            };
                            let icon_view = match item.category {
                                PaletteCategory::Channel => {
                                    if item.is_voice {
                                        icons::icon_volume_2().into_any()
                                    } else {
                                        icons::icon_hash().into_any()
                                    }
                                }
                                PaletteCategory::Server => {
                                    let initial = item.label.chars().next().unwrap_or('?').to_uppercase().to_string();
                                    view! { <span style="font-weight: 600;">{initial}</span> }.into_any()
                                }
                                PaletteCategory::Member => icons::icon_users().into_any(),
                            };
                            let cat_label = match item.category {
                                PaletteCategory::Channel => "Channel",
                                PaletteCategory::Server => "Server",
                                PaletteCategory::Member => "Member",
                            };
                            let label = item.label.clone();
                            view! {
                                <div
                                    class=class
                                    on:click=move |_| {
                                        match item_for_click.category {
                                            PaletteCategory::Channel => on_switch_channel.run(item_for_click.id.clone()),
                                            PaletteCategory::Server => on_switch_server.run(item_for_click.id.clone()),
                                            PaletteCategory::Member => on_open_members.run(()),
                                        }
                                        on_close.run(());
                                    }
                                    on:mouseenter=move |_| set_selected_index.set(i)
                                >
                                    <span class="icon">{icon_view}</span>
                                    <span class="palette-item-label">{label}</span>
                                    <span class="palette-item-category">{cat_label}</span>
                                </div>
                            }
                        }).collect();
                        view! { <div>{views}</div> }.into_any()
                    }}
                </div>
                <div class="palette-hint">
                    <span><kbd>"Enter"</kbd>" to select"</span>
                    <span><kbd>"↑↓"</kbd>" to navigate"</span>
                    <span><kbd>"Esc"</kbd>" to close"</span>
                </div>
            </div>
        </div>
    }
}
