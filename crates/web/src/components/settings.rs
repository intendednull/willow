use leptos::prelude::*;

/// Settings panel for display name and relay configuration.
#[component]
pub fn SettingsPanel() -> impl IntoView {
    view! {
        <div class="settings-panel">
            <h2>"Settings"</h2>
            <label>"Display Name"</label>
            <input type="text" placeholder="Enter display name..." />
            <label>"Relay Address"</label>
            <input type="text" placeholder="/ip4/1.2.3.4/tcp/9091/ws/p2p/12D3KooW..." />
            <button class="btn btn-primary">"Save & Reconnect"</button>
        </div>
    }
}
