//! Palette action catalog — spec layout-primitives.md §Actions catalog (v1).
//!
//! Each entry declares its label, icon thunk, availability predicate
//! (read-only over `AppState`), and a run dispatcher keyed on its stable
//! id. The palette iterates the catalog, filters by `(available)(&state)`,
//! and dispatches via `run(id, &ctx)`.

use crate::state::AppState;

/// One palette action, described declaratively.
#[derive(Clone)]
pub struct Action {
    pub id: &'static str,
    pub label: &'static str,
    pub available: fn(&AppState) -> bool,
}

/// Callbacks bundled together for dispatch without prop-drilling nine
/// separate closures through the palette component.
#[derive(Clone)]
pub struct CommandContext {
    pub on_open_tweaks: leptos::prelude::Callback<()>,
    pub on_open_settings: leptos::prelude::Callback<()>,
    pub on_new_channel: leptos::prelude::Callback<()>,
    pub on_new_letter: leptos::prelude::Callback<()>,
    pub on_create_grove: leptos::prelude::Callback<()>,
    pub on_open_sync_queue: leptos::prelude::Callback<()>,
    pub on_move_call: leptos::prelude::Callback<()>,
    pub on_toggle_theme: leptos::prelude::Callback<()>,
    pub on_sign_out: leptos::prelude::Callback<()>,
}

/// V1 action catalog. Letters and toggle-theme rows are feature-flagged
/// off and will land with `letters-dms.md` / the light theme.
pub fn catalog() -> Vec<Action> {
    vec![
        Action {
            id: "tweaks",
            label: "open tweaks",
            available: |_| true,
        },
        Action {
            id: "settings",
            label: "open settings",
            available: |_| true,
        },
        Action {
            id: "new-channel",
            label: "new channel…",
            available: can_manage_channels,
        },
        Action {
            id: "new-letter",
            label: "write a letter…",
            available: |_| true,
        },
        Action {
            id: "new-grove",
            label: "new grove…",
            available: |_| true,
        },
        Action {
            id: "sync-queue",
            label: "open sync queue",
            available: |_| true,
        },
        Action {
            id: "move-call",
            label: "move this call",
            available: is_call_active,
        },
        // toggle theme deferred until light theme ships.
        Action {
            id: "sign-out",
            label: "sign out",
            available: |_| true,
        },
    ]
}

/// Dispatch a catalog action by its stable id.
pub fn run(id: &str, ctx: &CommandContext) {
    use leptos::prelude::Callable;
    match id {
        "tweaks" => ctx.on_open_tweaks.run(()),
        "settings" => ctx.on_open_settings.run(()),
        "new-channel" => ctx.on_new_channel.run(()),
        "new-letter" => ctx.on_new_letter.run(()),
        "new-grove" => ctx.on_create_grove.run(()),
        "sync-queue" => ctx.on_open_sync_queue.run(()),
        "move-call" => ctx.on_move_call.run(()),
        "toggle-theme" => ctx.on_toggle_theme.run(()),
        "sign-out" => ctx.on_sign_out.run(()),
        _ => tracing::warn!(%id, "unknown palette action"),
    }
}

/// Proxy: `ManageChannels` permission until a per-permission signal
/// exists. Admin ↔ manages channels in v1 state.
fn can_manage_channels(state: &AppState) -> bool {
    use leptos::prelude::Get;
    let me = state.network.peer_id.get();
    if me.is_empty() {
        return false;
    }
    state.server.admin_ids.get().contains(&me)
}

/// Available only while an active call / voice channel is live.
fn is_call_active(state: &AppState) -> bool {
    use leptos::prelude::Get;
    state.voice.voice_channel.get().is_some()
}
