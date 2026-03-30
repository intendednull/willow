use leptos::prelude::*;

use crate::app::WebClientHandle;
use crate::components::ConfirmDialog;
use crate::icons;

/// Parse a string peer ID into an [`willow_identity::EndpointId`], returning
/// `None` if parsing fails.
fn parse_eid(s: &str) -> Option<willow_identity::EndpointId> {
    s.parse::<willow_identity::EndpointId>().ok()
}

/// Right sidebar showing connected peers and infrastructure nodes.
///
/// Workers (peers with SyncProvider permission) are displayed in a
/// separate "Infrastructure" section with role-specific icons and
/// badges, visually distinct from regular members.
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

    let handle_split = handle.clone();
    let handle_members = handle.clone();
    let handle_for_items = handle.clone();
    let handle_empty = handle.clone();

    view! {
        <div class="member-list">
            // ── Infrastructure (worker nodes) ──────────────────────
            {
                let hs = handle_split.clone();
                move || {
                    let all = peers.get();
                    let owner_str = hs.server_owner().to_string();
                    let workers: Vec<_> = all
                        .iter()
                        .filter(|(pid, _, _)| {
                            parse_eid(pid).is_some_and(|eid| hs.has_permission(&eid, &willow_client::willow_state::Permission::SyncProvider))
                                && pid != &peer_id.get_untracked()
                                && *pid != owner_str
                        })
                        .cloned()
                        .collect();

                    if workers.is_empty() {
                        None
                    } else {
                        let _hs2 = hs.clone();
                        Some(view! {
                            <h3 class="section-header infra-header">
                                {icons::icon_server()}
                                " Infrastructure"
                            </h3>
                            <For
                                each=move || workers.clone()
                                key=|(id, _, _)| id.clone()
                                let:worker
                            >
                                {
                                    let (wpid, wname, w_online) = worker;
                                    let wpid_display = wpid.clone();
                                    view! {
                                        <div class={if w_online { "worker-item" } else { "worker-item offline" }}>
                                            <div class="worker-icon">
                                                {
                                                    // Determine role by name heuristic or just show server icon.
                                                    let name_lower = wname.to_lowercase();
                                                    if name_lower.contains("replay") {
                                                        icons::icon_refresh().into_any()
                                                    } else if name_lower.contains("storage") {
                                                        icons::icon_database().into_any()
                                                    } else {
                                                        icons::icon_server().into_any()
                                                    }
                                                }
                                            </div>
                                            <div class="worker-info">
                                                <span class="worker-name">{wname}</span>
                                                <span class="worker-peer-id">{
                                                    if wpid_display.len() > 12 {
                                                        format!("{}...", &wpid_display[..12])
                                                    } else {
                                                        wpid_display
                                                    }
                                                }</span>
                                            </div>
                                            <div class="worker-status">
                                                {if w_online {
                                                    view! {
                                                        <span class="worker-badge online">{icons::icon_activity()} " Active"</span>
                                                    }.into_any()
                                                } else {
                                                    view! {
                                                        <span class="worker-badge offline">"Offline"</span>
                                                    }.into_any()
                                                }}
                                            </div>
                                        </div>
                                    }
                                }
                            </For>
                        })
                    }
                }
            }

            // ── Members (regular peers) ────────────────────────────
            <h3>"Members"</h3>
            <For
                each=move || {
                    let all = peers.get();
                    let hs = handle_members.clone();
                    let owner_str = hs.server_owner().to_string();
                    all.into_iter()
                        .filter(|(pid, _, _)| {
                            // Exclude workers from the members section.
                            !parse_eid(pid).is_some_and(|eid| hs.has_permission(&eid, &willow_client::willow_state::Permission::SyncProvider))
                                || pid == &peer_id.get_untracked()
                                || *pid == owner_str
                        })
                        .collect::<Vec<_>>()
                }
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
                    let handle_badge = handle_for_items.clone();
                    let handle_trust = handle_for_items.clone();
                    let handle_untrust = handle_for_items.clone();
                    view! {
                        <div class="member-item">
                            <div class={if is_online { "status-dot" } else { "status-dot offline" }}></div>
                            <span class="member-name" style=format!("color: {}", super::peer_color(&pid))>
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
                                    let owner = hb.server_owner().to_string();
                                    if pb == owner {
                                        Some(view! { <span class="badge owner-badge">"Owner"</span> })
                                    } else if parse_eid(&pb).is_some_and(|eid| hb.has_permission(&eid, &willow_client::willow_state::Permission::Administrator)) {
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
                                        let is_owner = hb2.server_owner().to_string() == peer_id.get_untracked();
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
                                                    <button class="btn btn-sm" on:click=move |_| { if let Some(eid) = parse_eid(&pt) { ht.trust_peer(eid); } }>"Trust"</button>
                                                    <button class="btn btn-sm" on:click=move |_| { if let Some(eid) = parse_eid(&pu) { hu.untrust_peer(eid); } }>"Untrust"</button>
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
                let all = peers.get();
                let owner_str = handle_empty.server_owner().to_string();
                let non_worker_count = all.iter().filter(|(pid, _, _)| {
                    !parse_eid(pid).is_some_and(|eid| handle_empty.has_permission(&eid, &willow_client::willow_state::Permission::SyncProvider))
                        || pid == &peer_id.get_untracked()
                        || *pid == owner_str
                }).count();
                if non_worker_count == 0 {
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
                        if let Some(eid) = parse_eid(&pid) {
                            let _ = handle_kick_confirm.kick_member(eid);
                        }
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
