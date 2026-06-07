use leptos::prelude::*;
use willow_client::presence::PresenceState;

use crate::app::WebClientHandle;
use crate::components::{
    ConfirmDialog, StatusDot, StatusDotBorder, StatusDotSize, TrustBadge, TrustBadgeSize,
};
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
    /// Called when the user clicks the rail collapse button.
    #[prop(optional, into)]
    on_close: Option<Callback<()>>,
) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();
    let app_state = use_context::<AppState>().unwrap();

    // Kick confirmation state.
    let (show_kick_confirm, set_show_kick_confirm) = signal(false);
    let (pending_kick_peer, set_pending_kick_peer) = signal(Option::<(String, String)>::None);
    let handle_kick_confirm = handle.clone();

    let handle_for_items = handle.clone();

    let connection_status = app_state.network.connection_status;
    let peer_count = app_state.network.peer_count;

    view! {
        <aside class="member-list" role="complementary" aria-label="members">
            <button
                class="rail-close-btn"
                title="close"
                aria-label="close rail"
                on:click=move |_| {
                    if let Some(cb) = on_close { cb.run(()); }
                }
            >
                "×"
            </button>
            // ── Network (collapsed by default) ─────────────────────
            <details class="rail-section rail-section--net">
                <summary class="rail-section__header">
                    <span class="rail-section__title">"Network"</span>
                    <span class=move || {
                        if connection_status.get() == "connected" {
                            "rail-section__chip rail-section__chip--ok"
                        } else {
                            "rail-section__chip rail-section__chip--warn"
                        }
                    }>
                        {move || {
                            let n = peer_count.get();
                            if connection_status.get() != "connected" {
                                "queued".to_string()
                            } else if n == 1 {
                                "1 peer".to_string()
                            } else {
                                format!("{n} peers")
                            }
                        }}
                    </span>
                </summary>
                <div class="rail-section__body net-detail">
                    <div class="net-detail-row">
                        <span class="net-detail-label">"connection"</span>
                        <span class=move || {
                            if connection_status.get() == "connected" {
                                "net-detail-value net-detail-value--ok"
                            } else {
                                "net-detail-value net-detail-value--warn"
                            }
                        }>
                            {move || connection_status.get()}
                        </span>
                    </div>
                    <div class="net-detail-row">
                        <span class="net-detail-label">"peers"</span>
                        <span class="net-detail-value">
                            {move || peer_count.get().to_string()}
                        </span>
                    </div>
                    <div class="net-detail-row">
                        <span class="net-detail-label">"relay"</span>
                        <span class="net-detail-value net-detail-value--mono">
                            "ws://localhost:3340"
                        </span>
                    </div>
                    <div class="net-detail-row">
                        <span class="net-detail-label">"encryption"</span>
                        <span class="net-detail-value">"e2e · chacha20-poly1305"</span>
                    </div>
                </div>
            </details>

            // ── Infrastructure (collapsed, hides if empty) ─────────
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
                        let worker_count = workers.len();
                        Some(view! {
                            <details class="rail-section rail-section--infra">
                                <summary class="rail-section__header">
                                    <span class="rail-section__title">
                                        {icons::icon_server()}
                                        " Infrastructure"
                                    </span>
                                    <span class="rail-section__chip">
                                        {if worker_count == 1 {
                                            "1 node".to_string()
                                        } else {
                                            format!("{worker_count} nodes")
                                        }}
                                    </span>
                                </summary>
                                <div class="rail-section__body">
                                    <For
                                        each=move || workers.clone()
                                        key=|(id, name, online)| format!("{id}:{name}:{online}")
                                        let:worker
                                    >
                                        {
                                            let (wpid, wname, w_online) = worker;
                                            let wpid_display = wpid.clone();
                                            let role_label = {
                                                let name_lower = wname.to_lowercase();
                                                if name_lower.contains("replay") {
                                                    "replay"
                                                } else if name_lower.contains("storage") {
                                                    "storage"
                                                } else {
                                                    "worker"
                                                }
                                            };
                                            view! {
                                                <div class={if w_online { "worker-item" } else { "worker-item offline" }}>
                                                    <div class="worker-icon">
                                                        {
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
                                                        <span class="worker-role">{role_label}</span>
                                                        <span class="worker-peer-id">{
                                                            if wpid_display.len() > 16 {
                                                                format!("{}…", &wpid_display[..16])
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
                                </div>
                            </details>
                        })
                    }
                }
            }

            // ── Members (expanded by default) ──────────────────────
            <details class="rail-section rail-section--members" open>
                <summary class="rail-section__header">
                    <span class="rail-section__title">"Members"</span>
                    <span class="rail-section__chip">
                        {move || {
                            let all = peers.get();
                            let owner_str = app_state.server.server_owner.get();
                            let sync_providers = app_state.server.sync_provider_ids.get();
                            let n = all.iter().filter(|(pid, _, _)| {
                                !sync_providers.contains(pid)
                                    || pid == &peer_id.get()
                                    || *pid == owner_str
                            }).count();
                            if n == 1 { "1".to_string() } else { format!("{n}") }
                        }}
                    </span>
                </summary>
                <div class="rail-section__body">
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
                    let pid_dot = pid.clone();
                    let pid_tooltip = pid.clone();
                    let handle_trust = handle_for_items.clone();
                    let handle_untrust = handle_for_items.clone();
                    // Presence state for this peer — derived from AppState
                    // presence map. Falls back to Here/Gone depending on
                    // reachability so reloading the app before the tick
                    // driver fires never shows a stale dot.
                    let presence_state = Signal::derive(move || {
                        app_state
                            .presence
                            .per_peer
                            .get()
                            .get(&pid_dot)
                            .copied()
                            .unwrap_or(if is_online {
                                PresenceState::Here
                            } else {
                                PresenceState::Gone
                            })
                    });
                    let tooltip_label = Signal::derive(move || {
                        app_state
                            .presence
                            .per_peer
                            .get()
                            .get(&pid_tooltip)
                            .copied()
                            .unwrap_or(if is_online {
                                PresenceState::Here
                            } else {
                                PresenceState::Gone
                            })
                            .label()
                    });
                    let pid_for_click = pid.clone();
                    let on_open_profile = move |ev: web_sys::MouseEvent| {
                        use wasm_bindgen::JsCast as _;
                        let anchor = ev
                            .current_target()
                            .and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok());
                        crate::profile::open_profile(&pid_for_click, anchor);
                    };
                    view! {
                        <div class="member-item">
                            <span class="member-status" title=tooltip_label>
                                <StatusDot
                                    state=presence_state
                                    size=StatusDotSize::Rail
                                    border=StatusDotBorder::Bg1
                                    ambient=true
                                />
                            </span>
                            <button
                                class="member-name member-name-btn"
                                type="button"
                                aria-label=format!("{} — open profile", name)
                                style=format!("color: {}", super::peer_color_from_str(&pid))
                                on:click=on_open_profile
                            >
                                {name.clone()}
                                <span class="member-peer-id">{
                                    let short = if pid.len() > 8 { format!("{}...", &pid[..8]) } else { pid.clone() };
                                    format!(" ({short})")
                                }</span>
                            </button>
                            <TrustBadge peer_id=pid.clone() size=TrustBadgeSize::Disk12/>
                            {
                                // Phase 2b — per-peer queue pill. Suppresses
                                // itself when the peer has no queued
                                // outbound / inbound traffic, so mounting
                                // it here unconditionally is zero-cost for
                                // idle rows.
                                let pid_for_pill = pid.clone();
                                let name_for_pill = name.clone();
                                move || {
                                    parse_eid(&pid_for_pill).map(|eid| {
                                        let name = name_for_pill.clone();
                                        view! {
                                            <crate::components::QueuePill
                                                peer_id=Signal::derive(move || eid)
                                                display_name=Signal::derive(move || name.clone())
                                            />
                                        }
                                    })
                                }
                            }
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
                                    // Phase 2d ad-hoc spawn: any member can
                                    // start a temp channel from a peer's row.
                                    // The channel is created with the default
                                    // 14-day threshold; member-seeding is
                                    // deferred to a manual AddMember (or a
                                    // future seed-on-create extension), per
                                    // the plan's v1 cut.
                                    let pid_for_temp = pid_self.clone();
                                    let h_for_temp = handle_for_items.clone();
                                    let is_self_temp = {
                                        let p = pid_for_temp.clone();
                                        move || peer_id.get() == p
                                    };
                                    move || (!is_self_temp()).then(|| {
                                        let h = h_for_temp.clone();
                                        let pid = pid_for_temp.clone();
                                        view! {
                                            <button
                                                class="btn btn-sm member-start-temp"
                                                title="start temp channel…"
                                                on:click=move |_| {
                                                    let h = h.clone();
                                                    let short: String =
                                                        pid.chars().take(6).collect();
                                                    let name = format!("side-{short}");
                                                    wasm_bindgen_futures::spawn_local(async move {
                                                        if let Err(e) = h.create_ephemeral_channel(
                                                            &name,
                                                            willow_state::EphemeralKind::Channel,
                                                            willow_state::DEFAULT_CHANNEL_THRESHOLD_MS,
                                                        ).await {
                                                            tracing::warn!(?e, "create_ephemeral_channel failed");
                                                        }
                                                    });
                                                }
                                            >
                                                "start temp channel…"
                                            </button>
                                        }
                                    })
                                }
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
                </div>
            </details>
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
