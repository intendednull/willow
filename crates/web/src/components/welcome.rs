use leptos::prelude::*;

use crate::app::ClientHandle;
use crate::components::AddServerPanel;
use crate::util::copy_to_clipboard;

/// Welcome/onboarding screen shown when the user has no servers.
///
/// Presents two options: create a new server or join an existing one via
/// invite code. The create/join form is delegated to `AddServerPanel` to
/// avoid duplicating logic.
#[component]
pub fn WelcomeScreen(
    client: ClientHandle,
    on_done: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView {
    let peer_id = client.borrow().peer_id();
    let (copy_label, set_copy_label) = signal("Copy");

    view! {
        <div class="welcome-screen">
            <div class="welcome-card">
                <h1>"Welcome to Willow"</h1>
                <p class="tagline">
                    "P2P encrypted chat \u{2014} no accounts, no servers, no middlemen."
                </p>
                <div class="settings-section" style="margin-bottom: 16px;">
                    <label>"Your Peer ID"</label>
                    <p class="welcome-hint">
                        "Share this with a server owner so they can invite you."
                    </p>
                    <div class="welcome-peer-id">
                        <code class="peer-id-text">{peer_id.clone()}</code>
                        <button
                            class="btn btn-sm"
                            on:click={
                                let pid = peer_id.clone();
                                move |_| {
                                    copy_to_clipboard(&pid);
                                    set_copy_label.set("Copied!");
                                    set_timeout(
                                        move || set_copy_label.set("Copy"),
                                        std::time::Duration::from_secs(2),
                                    );
                                }
                            }
                        >
                            {move || copy_label.get()}
                        </button>
                    </div>
                </div>
                <AddServerPanel client=client on_done=on_done />
            </div>
        </div>
    }
}
