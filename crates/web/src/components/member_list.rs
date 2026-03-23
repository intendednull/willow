use leptos::prelude::*;

use crate::app::ClientHandle;

/// Right sidebar showing connected peers with trust/kick actions.
#[component]
pub fn MemberList(
    peers: ReadSignal<Vec<String>>,
    client: ClientHandle,
    peer_id: ReadSignal<String>,
) -> impl IntoView {
    view! {
        <div class="member-list">
            <h3>"Online"</h3>
            <For
                each=move || peers.get()
                key=|p| p.clone()
                let:peer
            >
                {
                    let peer_display = peer.clone();
                    let peer_trust = peer.clone();
                    let peer_untrust = peer.clone();
                    let peer_kick = peer.clone();
                    let peer_badge = peer.clone();
                    let client_trust = client.clone();
                    let client_untrust = client.clone();
                    let client_kick = client.clone();
                    let client_badge = client.clone();
                    view! {
                        <div class="member-item">
                            <div class="status-dot"></div>
                            <span class="member-name">{
                                if peer_display.len() > 12 {
                                    format!("{}...", &peer_display[..12])
                                } else {
                                    peer_display.clone()
                                }
                            }</span>
                            {
                                let pb = peer_badge.clone();
                                let cb = client_badge.clone();
                                move || {
                                    let c = cb.borrow();
                                    let owner = c.state().server.server.as_ref()
                                        .map(|s| s.owner.to_string())
                                        .unwrap_or_default();
                                    if pb == owner {
                                        Some(view! { <span class="badge owner-badge">"Owner"</span> })
                                    } else if c.state().op_log.trusted_peers.contains(&pb) {
                                        Some(view! { <span class="badge trusted-badge">"Trusted"</span> })
                                    } else {
                                        None
                                    }
                                }
                            }
                            <div class="member-actions">
                                {
                                    let is_self = {
                                        let p = peer_display.clone();
                                        move || peer_id.get() == p
                                    };
                                    let pt = peer_trust.clone();
                                    let ct = client_trust.clone();
                                    let pu = peer_untrust.clone();
                                    let cu = client_untrust.clone();
                                    let pk = peer_kick.clone();
                                    let ck = client_kick.clone();
                                    move || {
                                        if is_self() {
                                            None
                                        } else {
                                            let pt = pt.clone();
                                            let ct = ct.clone();
                                            let pu = pu.clone();
                                            let cu = cu.clone();
                                            let pk = pk.clone();
                                            let ck = ck.clone();
                                            Some(view! {
                                                <button
                                                    class="btn btn-sm"
                                                    title="Trust peer"
                                                    on:click=move |_| {
                                                        ct.borrow_mut().trust_peer(&pt);
                                                    }
                                                >"Trust"</button>
                                                <button
                                                    class="btn btn-sm"
                                                    title="Untrust peer"
                                                    on:click=move |_| {
                                                        cu.borrow_mut().untrust_peer(&pu);
                                                    }
                                                >"Untrust"</button>
                                                <button
                                                    class="btn btn-sm btn-danger"
                                                    title="Kick member"
                                                    on:click=move |_| {
                                                        let _ = ck.borrow_mut().kick_member(&pk);
                                                    }
                                                >"Kick"</button>
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
