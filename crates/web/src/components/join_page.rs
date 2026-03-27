//! # Join Page Component
//!
//! Full-screen join page shown when a user clicks a shareable join link.
//! Displays server name, inviter, name input, and a single Join button
//! that morphs into a connecting state with auto-retry.

use leptos::prelude::*;

use crate::app::WebClientHandle;
use crate::state::{AppState, AppWriteSignals};

/// Full-screen join page — the first thing a user sees when they click
/// a join link. Shows server name, inviter, name input, and a single
/// Join button that morphs into a connecting state.
#[component]
pub fn JoinPage() -> impl IntoView {
    let state = use_context::<AppState>().unwrap();
    let write = use_context::<AppWriteSignals>().unwrap();
    let handle = use_context::<WebClientHandle>().unwrap();

    let token = state.ui.join_token;
    let status = state.ui.join_status;

    // Pre-fill name from saved profile.
    let (name, set_name) = signal(handle.display_name());

    // Retry timer: exponential backoff while status == "connecting".
    {
        let handle_retry = handle.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let mut backoff_ms: u32 = 2_000;
            const MAX_BACKOFF: u32 = 30_000;
            loop {
                gloo_timers::future::TimeoutFuture::new(backoff_ms).await;
                if status.get_untracked() != "connecting" {
                    break;
                }
                if let Some(t) = token.get_untracked() {
                    handle_retry.send_join_request(&t.link_id);
                }
                backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF);
            }
        });
    }

    let on_join = {
        let h = handle.clone();
        move |_| {
            let n = name.get_untracked();
            if !n.trim().is_empty() {
                h.set_display_name(n.trim());
            }
            write.ui.set_join_status.set("connecting".to_string());
            // Send initial JoinRequest.
            if let Some(t) = token.get_untracked() {
                h.send_join_request(&t.link_id);
            }
        }
    };

    let on_cancel = move |_| {
        write.ui.set_join_token.set(None);
        write.ui.set_join_status.set(String::new());
        if let Some(window) = web_sys::window() {
            let _ = window.history().ok().and_then(|h| {
                h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/"))
                    .ok()
            });
        }
    };

    view! {
        <div class="join-page">
            <div class="join-page-ambient"></div>
            <div class="join-card">
                <div class="join-card-brand">"willow"</div>

                {move || token.get().map(|t| {
                    let server_name = t.server_name.clone();
                    let inviter = t.inviter_name.clone();
                    view! {
                        <h1 class="join-card-server">{server_name}</h1>
                        <p class="join-card-inviter">"Invited by " <strong>{inviter}</strong></p>
                    }
                })}

                <div class="join-card-field">
                    <label>"Your name"</label>
                    <input
                        type="text"
                        placeholder="Enter your name..."
                        prop:value=move || name.get()
                        on:input=move |ev| set_name.set(event_target_value(&ev))
                        disabled=move || status.get() == "connecting"
                    />
                </div>

                {move || {
                    let s = status.get();
                    let server = token.get().map(|t| t.server_name.clone()).unwrap_or_default();
                    if s == "connecting" {
                        view! {
                            <button class="btn btn-primary join-card-btn connecting" disabled>
                                "Connecting..."
                            </button>
                            <p class="join-card-hint">
                                "Waiting for the server owner to respond."
                            </p>
                        }.into_any()
                    } else if let Some(reason) = s.strip_prefix("denied:") {
                        let msg = match reason {
                            "link_expired" => "This invite link has been fully used.",
                            "link_disabled" => "This invite link is no longer active.",
                            _ => "This invite link is no longer valid.",
                        };
                        view! {
                            <p class="join-card-error">{msg}</p>
                            <button class="btn btn-sm" on:click=on_cancel>"Back"</button>
                        }.into_any()
                    } else {
                        let join_handler = on_join.clone();
                        view! {
                            <button class="btn btn-primary join-card-btn" on:click=join_handler>
                                {format!("Join {server}")}
                            </button>
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}
