use leptos::prelude::*;

use crate::app::WebClientHandle;
use crate::components::RoleManager;
use crate::icons;
use crate::state::{AppState, SettingsTab};
use crate::util::copy_to_clipboard;

/// A single role entry: (role_id, role_name, list of granted permission strings).
type RoleEntry = (String, String, Vec<String>);

/// Tabbed settings panel combining Profile, Server, and Roles tabs.
///
/// Reads the initial tab from `default_tab` and maintains a local signal
/// for tab switching. The Profile tab shows display name and peer ID.
/// The Server tab shows invite generation and peer ID sharing. The Roles
/// tab renders the `RoleManager` (visible only for the server owner).
#[component]
pub fn SettingsPanel(
    peer_id: ReadSignal<String>,
    #[prop(into)] roles: Signal<Vec<RoleEntry>>,
    default_tab: SettingsTab,
    on_close: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();
    let app_state = use_context::<AppState>().unwrap();

    let (active_tab, set_active_tab) = signal(default_tab);
    let (status_msg, set_status_msg) = signal(String::new());

    // Determine if the local user is the server owner.
    let is_owner = move || {
        let pid = peer_id.get();
        app_state.server.server_owner.get() == pid
    };

    // Tab display name.
    let tab_name = move || match active_tab.get() {
        SettingsTab::Profile => "Profile",
        SettingsTab::Server => "Server",
        SettingsTab::Roles => "Roles",
    };

    // ── Profile tab handlers ─────────────────────────────────────────
    let (display_name, set_display_name) = signal(app_state.server.display_name.get_untracked());
    let server_name = app_state.server.active_server_name.get_untracked();

    let handle_save = handle.clone();
    let on_save = move |_| {
        let name = display_name.get_untracked();
        if !name.trim().is_empty() {
            let h = handle_save.clone();
            let name = name.trim().to_string();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = h.set_server_display_name(&name).await;
            });
        }
        set_status_msg.set("Saved.".to_string());
    };

    let on_copy_peer_id_profile = move |_| {
        let id = peer_id.get_untracked();
        copy_to_clipboard(&id);
        set_status_msg.set("Peer ID copied.".to_string());
    };

    // ── Server tab handlers ──────────────────────────────────────────
    let (invite_peer, set_invite_peer) = signal(String::new());
    let (invite_code, set_invite_code) = signal(String::new());

    let handle_invite = handle.clone();
    let on_generate_invite = move |_| {
        let recipient = invite_peer.get_untracked();
        if recipient.trim().is_empty() {
            set_status_msg.set("Enter a recipient Peer ID.".to_string());
            return;
        }
        let Ok(recipient_eid) = recipient.trim().parse::<willow_identity::EndpointId>() else {
            set_status_msg.set("Invalid Peer ID format.".to_string());
            return;
        };
        let h = handle_invite.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match h.generate_invite(&recipient_eid).await {
                Ok(code) => {
                    set_invite_code.set(code);
                    set_status_msg.set("Invite generated.".to_string());
                }
                Err(e) => {
                    set_status_msg.set(format!("Error: {e}"));
                }
            }
        });
    };

    let on_copy_invite = move |_| {
        let code = invite_code.get_untracked();
        if !code.is_empty() {
            copy_to_clipboard(&code);
            set_status_msg.set("Invite code copied.".to_string());
        }
    };

    let on_copy_peer_id_server = move |_| {
        let id = peer_id.get_untracked();
        copy_to_clipboard(&id);
        set_status_msg.set("Peer ID copied.".to_string());
    };

    // Read the actual server name reactively for the header.
    let active_server_name = app_state.server.active_server_name;

    view! {
        <div class="settings-panel">
            // Breadcrumb header: back arrow + "Settings / TabName"
            <div class="server-settings-header">
                <button class="btn btn-sm" on:click=move |_| on_close(())>
                    {icons::icon_arrow_left()} " Back"
                </button>
                <h2>"Settings / " {tab_name}</h2>
            </div>

            // Tab buttons.
            <div class="settings-tabs">
                <button
                    class=move || if active_tab.get() == SettingsTab::Profile { "tab-btn active" } else { "tab-btn" }
                    on:click=move |_| set_active_tab.set(SettingsTab::Profile)
                >"Profile"</button>
                <button
                    class=move || if active_tab.get() == SettingsTab::Server { "tab-btn active" } else { "tab-btn" }
                    on:click=move |_| set_active_tab.set(SettingsTab::Server)
                >"Server"</button>
                {
                    let owner_check = is_owner.clone();
                    move || {
                        if owner_check() {
                            Some(view! {
                                <button
                                    class=move || if active_tab.get() == SettingsTab::Roles { "tab-btn active" } else { "tab-btn" }
                                    on:click=move |_| set_active_tab.set(SettingsTab::Roles)
                                >"Roles"</button>
                            })
                        } else {
                            None
                        }
                    }
                }
            </div>

            // Status message.
            {move || {
                let msg = status_msg.get();
                if msg.is_empty() {
                    None
                } else {
                    Some(view! {
                        <div class="settings-status">{msg}</div>
                    })
                }
            }}

            // Profile tab content.
            <div class="settings-tab-content" style=move || if active_tab.get() == SettingsTab::Profile { "" } else { "display:none" }>
                <div class="settings-section">
                    <label>"Your Peer ID"</label>
                    <div class="peer-id-display">
                        <code class="peer-id-text">{move || { let id = peer_id.get(); id.get(..10).unwrap_or(&id).to_string() }}</code>
                        <button class="btn btn-sm" on:click=on_copy_peer_id_profile>"Copy"</button>
                    </div>
                </div>
                <div class="settings-section">
                    <div class="settings-server-label">
                        <span class="settings-server-prefix">"Profile for: "</span>
                        <span class="settings-server-name">{server_name}</span>
                    </div>
                    <label>"Display Name (this server)"</label>
                    <input
                        type="text"
                        placeholder="Enter display name..."
                        prop:value=move || display_name.get()
                        on:input=move |ev| set_display_name.set(event_target_value(&ev))
                    />
                </div>
                <button class="btn btn-primary" on:click=on_save>"Save"</button>
            </div>

            // Server tab content.
            <div class="settings-tab-content" style=move || if active_tab.get() == SettingsTab::Server { "" } else { "display:none" }>
                <div class="settings-section">
                    <h3>{move || active_server_name.get()}</h3>
                </div>
                <div class="settings-section invite-section">
                    <h3>"Invite a Peer"</h3>
                    <p class="settings-hint">"The recipient needs to share their Peer ID with you first."</p>
                    <label>"Recipient Peer ID"</label>
                    <input
                        type="text"
                        placeholder="12D3KooW..."
                        prop:value=move || invite_peer.get()
                        on:input=move |ev| set_invite_peer.set(event_target_value(&ev))
                    />
                    <button class="btn btn-primary" on:click=on_generate_invite>"Generate Invite"</button>
                    {move || {
                        let code = invite_code.get();
                        if code.is_empty() {
                            None
                        } else {
                            Some(view! {
                                <div class="invite-code-display">
                                    <textarea readonly prop:value=code.clone()></textarea>
                                    <button class="btn btn-sm" on:click=on_copy_invite>"Copy"</button>
                                </div>
                            })
                        }
                    }}
                </div>
                // ── Invite Links ──────────────────────────────────────
                {
                    let (link_copied, set_link_copied) = signal(false);
                    let (link_list, set_link_list) = signal(Vec::new());

                    // Load initial link list asynchronously.
                    {
                        let h = handle.clone();
                        let set_ll = set_link_list;
                        wasm_bindgen_futures::spawn_local(async move {
                            set_ll.set(h.join_links().await);
                        });
                    }

                    let handle_gen = handle.clone();
                    let set_status = set_status_msg;
                    let on_create_link = move |_| {
                        let h = handle_gen.clone();
                        let set_ll = set_link_list;
                        wasm_bindgen_futures::spawn_local(async move {
                            match h.create_join_link(5, None).await {
                                Ok(token) => {
                                    let origin = web_sys::window()
                                        .and_then(|w| w.location().origin().ok())
                                        .unwrap_or_else(|| "https://willow.intendednull.com".to_string());
                                    let url = format!("{origin}/#join={token}");
                                    copy_to_clipboard(&url);
                                    set_link_copied.set(true);
                                    let set_copied = set_link_copied;
                                    wasm_bindgen_futures::spawn_local(async move {
                                        gloo_timers::future::TimeoutFuture::new(1500).await;
                                        set_copied.set(false);
                                    });
                                    set_ll.set(h.join_links().await);
                                }
                                Err(e) => set_status.set(format!("Error: {e}")),
                            }
                        });
                    };

                    let handle_links = handle.clone();
                    view! {
                        <div class="settings-section">
                            <h3>"Invite Links"</h3>
                            <p class="settings-hint">"Share a link to let people join while you're online."</p>
                            <div style="position: relative; display: inline-block;">
                                <button class="btn btn-accent-green" on:click=on_create_link>
                                    "Create Invite Link"
                                </button>
                                {move || link_copied.get().then(|| view! {
                                    <span class="copied-tooltip">"Copied!"</span>
                                })}
                            </div>

                            <div class="invite-link-list">
                                <For
                                    each=move || link_list.get()
                                    key=|link| link.link_id.clone()
                                    let:link
                                >
                                    {
                                        let h = handle_links.clone();
                                        let lid = link.link_id.clone();
                                        let valid = link.is_valid();
                                        let set_ll = set_link_list;
                                        view! {
                                            <div class={if valid { "invite-link-item" } else { "invite-link-item expired" }}>
                                                <span class="invite-link-uses">
                                                    {format!("{}/{}", link.used, link.max_uses)}
                                                </span>
                                                <span class="invite-link-age">
                                                    {if valid { "active" } else { "expired" }}
                                                </span>
                                                <button class="btn-icon btn-icon-danger" on:click={
                                                    let h2 = h.clone();
                                                    let lid2 = lid.clone();
                                                    move |_| {
                                                        let h3 = h2.clone();
                                                        let lid3 = lid2.clone();
                                                        wasm_bindgen_futures::spawn_local(async move {
                                                            h3.delete_join_link(&lid3).await;
                                                            set_ll.set(h3.join_links().await);
                                                        });
                                                    }
                                                }>
                                                    {icons::icon_trash()}
                                                </button>
                                            </div>
                                        }
                                    }
                                </For>
                            </div>
                        </div>
                    }
                }

                <div class="settings-section">
                    <h3>"Your Peer ID"</h3>
                    <p class="settings-hint">"Share this with others so they can invite you to their servers."</p>
                    <div class="peer-id-display">
                        <code class="peer-id-text">{move || { let id = peer_id.get(); id.get(..10).unwrap_or(&id).to_string() }}</code>
                        <button class="btn btn-sm" on:click=on_copy_peer_id_server>"Copy"</button>
                    </div>
                </div>
            </div>

            // Roles tab content.
            <div class="settings-tab-content" style=move || if active_tab.get() == SettingsTab::Roles { "" } else { "display:none" }>
                <div class="settings-section role-section">
                    <RoleManager
                        peer_id=peer_id
                        roles=roles
                    />
                </div>
            </div>
        </div>
    }
}
