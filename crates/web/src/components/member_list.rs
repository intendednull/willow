use leptos::prelude::*;

use crate::app::WebClientHandle;
use crate::components::ConfirmDialog;

/// Right sidebar showing connected peers with trust/kick actions.
/// Accepts `(peer_id, display_name)` tuples so names update reactively.
#[component]
pub fn MemberList(
    peers: ReadSignal<Vec<(String, String, bool)>>,
    peer_id: ReadSignal<String>,
) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();

    // Kick confirmation state.
    let (show_kick_confirm, set_show_kick_confirm) = signal(false);
    let (pending_kick_peer, set_pending_kick_peer) = signal(Option::<(String, String)>::None);
    let handle_kick_confirm = handle.clone();

    view! {
        <div class="member-list">
            <h3>"Members"</h3>
            <For
                each=move || peers.get()
                key=|(id, _, _)| id.clone()
                let:peer
            >
                {
                    let (pid, name, is_online) = peer;
                    let name_for_kick = name.clone();
                    let pid_badge = pid.clone();
                    let pid_trust = pid.clone();
                    let pid_untrust = pid.clone();
                    let pid_kick = pid.clone();
                    let pid_self = pid.clone();
                    let handle_badge = handle.clone();
                    let handle_trust = handle.clone();
                    let handle_untrust = handle.clone();
                    view! {
                        <div class="member-item">
                            <div class={if is_online { "status-dot" } else { "status-dot offline" }}></div>
                            <span class="member-name">
                                {name}
                                <span class="member-peer-id">{
                                    let short = if pid.len() > 8 { format!("{}...", &pid[..8]) } else { pid.clone() };
                                    format!(" ({short})")
                                }</span>
                            </span>
                            {
                                let pb = pid_badge.clone();
                                let hb = handle_badge.clone();
                                move || {
                                    let owner = hb.server_owner();
                                    if pb == owner {
                                        Some(view! { <span class="badge owner-badge">"Owner"</span> })
                                    } else if hb.has_permission(&pb, &willow_client::willow_state::Permission::Administrator) {
                                        Some(view! { <span class="badge trusted-badge">"Trusted"</span> })
                                    } else {
                                        None
                                    }
                                }
                            }
                            <div class="member-actions">
                                {
                                    let is_self = {
                                        let p = pid_self.clone();
                                        move || peer_id.get() == p
                                    };
                                    let pt = pid_trust.clone();
                                    let ht = handle_trust.clone();
                                    let pu = pid_untrust.clone();
                                    let hu = handle_untrust.clone();
                                    let pk = pid_kick.clone();
                                    let hb2 = handle_badge.clone();
                                    move || {
                                        let is_owner = hb2.server_owner() == peer_id.get_untracked();
                                        if is_self() || !is_owner {
                                            None
                                        } else {
                                            let pt = pt.clone();
                                            let ht = ht.clone();
                                            let pu = pu.clone();
                                            let hu = hu.clone();
                                            let pk = pk.clone();
                                            {
                                                let kick_name = name_for_kick.clone();
                                                let kick_pid = pk.clone();
                                                Some(view! {
                                                    <button class="btn btn-sm" on:click=move |_| { ht.trust_peer(&pt); }>"Trust"</button>
                                                    <button class="btn btn-sm" on:click=move |_| { hu.untrust_peer(&pu); }>"Untrust"</button>
                                                    <button class="btn btn-sm btn-danger" on:click=move |_| {
                                                        set_pending_kick_peer.set(Some((kick_pid.clone(), kick_name.clone())));
                                                        set_show_kick_confirm.set(true);
                                                    }>"Kick"</button>
                                                })
                                            }
                                        }
                                    }
                                }
                            </div>
                        </div>
                    }
                }
            </For>
            {move || {
                if peers.get().is_empty() {
                    Some(view! { <div class="empty-state" style="font-size: 12px;">"No peers connected"</div> })
                } else {
                    None
                }
            }}
            <ConfirmDialog
                visible=show_kick_confirm
                title="Kick Member"
                message=Signal::derive(move || {
                    pending_kick_peer.get()
                        .map(|(_, name)| format!("Kick {}?", name))
                        .unwrap_or_default()
                })
                confirm_text="Kick"
                danger=true
                on_confirm=Callback::new(move |_| {
                    if let Some((pid, _)) = pending_kick_peer.get_untracked() {
                        let _ = handle_kick_confirm.kick_member(&pid);
                    }
                    set_pending_kick_peer.set(None);
                    set_show_kick_confirm.set(false);
                })
                on_cancel=Callback::new(move |_| {
                    set_pending_kick_peer.set(None);
                    set_show_kick_confirm.set(false);
                })
            />
        </div>
    }
}
