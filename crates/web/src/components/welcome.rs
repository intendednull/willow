use leptos::prelude::*;

use crate::app::WebClientHandle;
use crate::components::AddServerPanel;
use crate::icons;
use crate::util::copy_to_clipboard;

/// Welcome/onboarding screen shown when the user has no servers.
///
/// Presents a brand hero (willow mark + wordmark + tagline), a compact
/// copyable peer-id row, and a tabbed create/join flow.
#[component]
pub fn WelcomeScreen(on_done: impl Fn(()) + Send + Clone + 'static) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();
    let peer_id = handle.peer_id();
    let peer_id_short = peer_id.get(..10).unwrap_or(&peer_id).to_string();
    let (copy_label, set_copy_label) = signal("copy");

    view! {
        <div class="welcome-screen">
            <div class="welcome-card">
                <div class="welcome-hero">
                    <div class="willow-mark-lg">{icons::icon_willow_mark()}</div>
                    <h1 class="willow-wordmark">"willow"</h1>
                    <p class="tagline">
                        "a grove of your own \u{2014} small group chat that lives on your devices, not on a server."
                    </p>
                </div>

                <div class="welcome-peer-compact" title="your peer id">
                    <span class="welcome-peer-compact__label">"your id"</span>
                    <code class="peer-id-text" data-full-id={peer_id.clone()}>{peer_id_short}</code>
                    <button
                        class="btn btn-sm welcome-peer-compact__copy"
                        on:click={
                            let pid = peer_id.clone();
                            move |_| {
                                copy_to_clipboard(&pid);
                                set_copy_label.set("copied");
                                set_timeout(
                                    move || set_copy_label.set("copy"),
                                    std::time::Duration::from_secs(2),
                                );
                            }
                        }
                    >
                        {move || copy_label.get()}
                    </button>
                </div>

                <AddServerPanel on_done=on_done />
            </div>
        </div>
    }
}
