use leptos::prelude::*;

/// Right sidebar showing connected peers.
#[component]
pub fn MemberList(peers: ReadSignal<Vec<String>>) -> impl IntoView {
    view! {
        <div class="member-list">
            <h3>"Online"</h3>
            <For
                each=move || peers.get()
                key=|p| p.clone()
                let:peer
            >
                <div class="member-item">
                    <div class="status-dot"></div>
                    <span>{
                        if peer.len() > 12 { format!("{}...", &peer[..12]) } else { peer.clone() }
                    }</span>
                </div>
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
