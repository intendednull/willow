use leptos::prelude::*;

use crate::components::AddServerPanel;
use crate::icons;

#[derive(Clone, Copy, PartialEq, Eq)]
enum WelcomeStep {
    Name,
    Action,
}

/// Welcome/onboarding screen shown when the user has no servers.
///
/// Two-step flow. Step 1: ask the user's display name (optional, enter-to-
/// continue). Step 2: brand hero with a "hi, {name}" subtitle, followed by
/// tabbed Create / Join surfaces. The name is optional — continuing with an
/// empty field lets the peer id stand in as identity.
#[component]
pub fn WelcomeScreen(on_done: impl Fn(()) + Send + Clone + 'static) -> impl IntoView {
    let (display_name, set_display_name) = signal(String::new());
    let (step, set_step) = signal(WelcomeStep::Name);

    let on_continue = move |_| set_step.set(WelcomeStep::Action);
    let on_continue_key = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Enter" {
            ev.prevent_default();
            set_step.set(WelcomeStep::Action);
        }
    };

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
                {move || match step.get() {
                    WelcomeStep::Name => view! {
                        <div class="welcome-step welcome-step--name">
                            <div class="willow-mark-lg willow-mark--small">
                                {icons::icon_willow_mark()}
                            </div>
                            <h1 class="welcome-name-heading">
                                "What do we call you?"
                            </h1>
                            <input
                                class="welcome-name-input"
                                type="text"
                                autofocus
                                placeholder="enter your name (optional)"
                                prop:value=move || display_name.get()
                                on:input=move |ev| set_display_name.set(event_target_value(&ev))
                                on:keydown=on_continue_key
                            />
                            <button
                                class="btn btn-primary welcome-continue-btn"
                                on:click=on_continue
                            >
                                "continue"
                            </button>
                        </div>
                    }.into_any(),
                    WelcomeStep::Action => view! {
                        <div class="welcome-step welcome-step--action">
                            <div class="welcome-hero">
                                <div class="willow-mark-lg">{icons::icon_willow_mark()}</div>
                                <h1 class="willow-wordmark">"willow"</h1>
                                <p class="tagline">"encrypted p2p chat"</p>
                                {move || greeting().map(|g| view! {
                                    <p class="welcome-greeting">{g}</p>
                                })}
                            </div>
                            <AddServerPanel
                                on_done=on_done.clone()
                                display_name=display_name
                            />
                        </div>
                    }.into_any(),
                }}
            </div>
        </div>
    }
}
