use leptos::prelude::*;

use crate::app::ClientHandle;

/// Right sidebar showing connected peers with trust/kick actions.
/// Accepts `(peer_id, display_name)` tuples so names update reactively.
#[component]
pub fn MemberList(
    peers: ReadSignal<Vec<(String, String, bool)>>,
    client: ClientHandle,
    peer_id: ReadSignal<String>,
) -> impl IntoView {
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
                    let pid_badge = pid.clone();
                    let pid_trust = pid.clone();
                    let pid_untrust = pid.clone();
                    let pid_kick = pid.clone();
                    let pid_self = pid.clone();
                    let client_badge = client.clone();
                    let client_trust = client.clone();
                    let client_untrust = client.clone();
                    let client_kick = client.clone();
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
                                let cb = client_badge.clone();
                                move || {
                                    let c = cb.borrow();
                                    let owner = c.state().event_state.owner.clone();
                                    if pb == owner {
                                        Some(view! { <span class="badge owner-badge">"Owner"</span> })
                                    } else if c.state().active()
                                        .map(|ctx| ctx.op_log.trusted_peers.contains(&pb))
                                        .unwrap_or(false)
                                    {
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
                                    let ct = client_trust.clone();
                                    let pu = pid_untrust.clone();
                                    let cu = client_untrust.clone();
                                    let pk = pid_kick.clone();
                                    let ck = client_kick.clone();
                                    move || {
                                        let is_owner = {
                                            let c = client_badge.borrow();
                                            c.state().event_state.owner == peer_id.get_untracked()
                                        };
                                        if is_self() || !is_owner {
                                            None
                                        } else {
                                            let pt = pt.clone();
                                            let ct = ct.clone();
                                            let pu = pu.clone();
                                            let cu = cu.clone();
                                            let pk = pk.clone();
                                            let ck = ck.clone();
                                            Some(view! {
                                                <button class="btn btn-sm" on:click=move |_| { ct.borrow_mut().trust_peer(&pt); }>"Trust"</button>
                                                <button class="btn btn-sm" on:click=move |_| { cu.borrow_mut().untrust_peer(&pu); }>"Untrust"</button>
                                                <button class="btn btn-sm btn-danger" on:click=move |_| { let _ = ck.borrow_mut().kick_member(&pk); }>"Kick"</button>
                                            })
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
        </div>
    }
}
