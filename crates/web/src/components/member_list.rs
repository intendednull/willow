use leptos::prelude::*;

use crate::app::WebClientHandle;
use crate::components::{ConfirmDialog, TrustBadge, TrustBadgeSize};
use crate::icons;
use crate::state::AppState;

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
    let app_state = use_context::<AppState>().unwrap();

    // Kick confirmation state.
    let (show_kick_confirm, set_show_kick_confirm) = signal(false);
    let (pending_kick_peer, set_pending_kick_peer) = signal(Option::<(String, String)>::None);
    let handle_kick_confirm = handle.clone();

    let handle_for_items = handle.clone();

    view! {
        <aside class="member-list" role="complementary" aria-label="members">
            // ── Infrastructure (worker nodes) ──────────────────────
            {
                move || {
                    let all = peers.get();
                    let owner_str = app_state.server.server_owner.get();
                    let sync_providers = app_state.server.sync_provider_ids.get();
                    let workers: Vec<_> = all
                        .iter()
                        .filter(|(pid, _, _)| {
                            sync_providers.contains(pid)
                                && pid != &peer_id.get()
                                && *pid != owner_str
                        })
                        .cloned()
                        .collect();

                    if workers.is_empty() {
                        None
                    } else {
                        Some(view! {
                            <h3 class="section-header infra-header">
                                {icons::icon_server()}
                                " Infrastructure"
                            </h3>
                            <For
                                each=move || workers.clone()
                                key=|(id, name, online)| format!("{id}:{name}:{online}")
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
                    let owner_str = app_state.server.server_owner.get();
                    let sync_providers = app_state.server.sync_provider_ids.get();
                    all.into_iter()
                        .filter(|(pid, _, _)| {
                            // Exclude workers from the members section.
                            !sync_providers.contains(pid)
                                || pid == &peer_id.get()
                                || *pid == owner_str
                        })
                        .collect::<Vec<_>>()
                }
                key=|(id, name, online)| format!("{id}:{name}:{online}")
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
                            <TrustBadge peer_id=pid.clone() size=TrustBadgeSize::Disk12/>
                            {
                                let pb = pid_badge.clone();
                                move || {
                                    let owner = app_state.server.server_owner.get();
                                    let admins = app_state.server.admin_ids.get();
                                    if pb == owner {
                                        Some(view! { <span class="badge owner-badge">"Owner"</span> })
                                    } else if admins.contains(&pb) {
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
                                    move || {
                                        let is_admin = app_state.server.admin_ids.get().contains(&peer_id.get());
                                        if is_self() || !is_admin {
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
                                                    <button class="btn btn-sm" on:click=move |_| {
                                                        if let Some(eid) = parse_eid(&pt) {
                                                            let ht = ht.clone();
                                                            wasm_bindgen_futures::spawn_local(async move {
                                                                ht.propose_grant_admin(eid).await.ok();
                                                            });
                                                        }
                                                    }>"Trust"</button>
                                                    <button class="btn btn-sm" on:click=move |_| {
                                                        if let Some(eid) = parse_eid(&pu) {
                                                            let hu = hu.clone();
                                                            wasm_bindgen_futures::spawn_local(async move {
                                                                hu.propose_revoke_admin(eid).await.ok();
                                                            });
                                                        }
                                                    }>"Untrust"</button>
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
                let owner_str = app_state.server.server_owner.get();
                let sync_providers = app_state.server.sync_provider_ids.get();
                let non_worker_count = all.iter().filter(|(pid, _, _)| {
                    !sync_providers.contains(pid)
                        || pid == &peer_id.get()
                        || *pid == owner_str
                }).count();
                if non_worker_count == 0 {
                    Some(view! {
                        <div class="state-empty member-list-empty">
                            <div class="state-empty__headline">"just you so far"</div>
                            <div class="state-empty__hint">"invite someone"</div>
                        </div>
                    })
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
                            let hk = handle_kick_confirm.clone();
                            wasm_bindgen_futures::spawn_local(async move {
                                hk.propose_kick_member(eid).await.ok();
                            });
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
        </aside>
    }
}
