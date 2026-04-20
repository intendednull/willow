use leptos::prelude::*;

use crate::components::AddServerPanel;
use crate::icons;

/// Welcome/onboarding screen shown when the user has no servers.
///
/// Brand hero, a shared display-name input (optional, applies to whichever
/// path the user takes), then tabbed Create / Join flows.
#[component]
pub fn WelcomeScreen(on_done: impl Fn(()) + Send + Clone + 'static) -> impl IntoView {
    let (display_name, set_display_name) = signal(String::new());

    let greeting = move || {
        let n = display_name.get();
        let trimmed = n.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(format!("hi, {trimmed}"))
        }
    };

    view! {
        <div class="welcome-screen">
            <div class="welcome-card">
                <div class="welcome-hero">
                    <div class="willow-mark-lg">{icons::icon_willow_mark()}</div>
                    <h1 class="willow-wordmark">"willow"</h1>
                    <p class="tagline">"encrypted p2p chat"</p>
                    {move || greeting().map(|g| view! {
                        <p class="welcome-greeting">{g}</p>
                    })}
                </div>

                <div class="welcome-name-row">
                    <label for="welcome-display-name">"Display name · optional"</label>
                    <input
                        id="welcome-display-name"
                        type="text"
                        placeholder="what peers should call you"
                        prop:value=move || display_name.get()
                        on:input=move |ev| set_display_name.set(event_target_value(&ev))
                    />
                </div>

                <AddServerPanel on_done=on_done display_name=display_name />
            </div>
        </div>
    }
}
