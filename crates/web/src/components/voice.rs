//! # Voice Controls Component
//!
//! UI component for voice chat controls, shown when connected to a voice
//! channel. Displays mute, deafen, and disconnect buttons.

use leptos::prelude::*;

use crate::icons;

/// Voice control bar shown when connected to a voice channel.
///
/// Displays the current voice channel name, mute/deafen toggles, and a
/// disconnect button. Positioned above the user area in the sidebar.
#[component]
pub fn VoiceControls(
    /// Name of the voice channel currently connected to.
    channel_name: ReadSignal<String>,
    /// Whether the local microphone is muted.
    muted: ReadSignal<bool>,
    /// Whether the local audio output is deafened.
    deafened: ReadSignal<bool>,
    /// Called when the mute button is clicked.
    on_mute: impl Fn(()) + Send + Clone + 'static,
    /// Called when the deafen button is clicked.
    on_deafen: impl Fn(()) + Send + Clone + 'static,
    /// Called when the disconnect button is clicked.
    on_disconnect: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView {
    view! {
        <div class="voice-controls">
            <div class="voice-status">
                <span class="voice-status-icon">{icons::icon_volume_2()}</span>
                <span class="voice-channel-name">{move || channel_name.get()}</span>
            </div>
            <div class="voice-buttons">
                <button
                    class=move || if muted.get() { "voice-btn muted" } else { "voice-btn" }
                    title=move || if muted.get() { "Unmute" } else { "Mute" }
                    on:click={
                        let on_mute = on_mute.clone();
                        move |_| on_mute(())
                    }
                >
                    {move || if muted.get() { icons::icon_mic_off().into_any() } else { icons::icon_mic().into_any() }}
                </button>
                <button
                    class=move || if deafened.get() { "voice-btn deafened" } else { "voice-btn" }
                    title=move || if deafened.get() { "Undeafen" } else { "Deafen" }
                    on:click={
                        let on_deafen = on_deafen.clone();
                        move |_| on_deafen(())
                    }
                >
                    {move || if deafened.get() { icons::icon_headphones_off().into_any() } else { icons::icon_headphones().into_any() }}
                </button>
                <button
                    class="voice-btn disconnect"
                    title="Disconnect"
                    on:click={
                        let on_disconnect = on_disconnect.clone();
                        move |_| on_disconnect(())
                    }
                >
                    {icons::icon_phone_off()}
                </button>
            </div>
        </div>
    }
}
