//! Global keybindings — spec layout-primitives.md §Accessibility.
//!
//! Registers a single window-level `keydown` listener that owns:
//!   - ⌘K / Ctrl-K: toggle command palette
//!   - Escape: pop the top of the close-stack (rail → pinned → bottom
//!     sheet → grove drawer → palette)
//!   - Alt+↑ / Alt+↓: cycle groves
//!
//! Mutation goes through the provided `AppWriteSignals`; reads go
//! through `AppState` via `get_untracked()` to avoid tracking the
//! global listener inside any reactive scope.

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::state::{AppState, AppWriteSignals};

/// Install the global keydown listener. Call once during app bootstrap.
pub fn install(state: AppState, write: AppWriteSignals) {
    let closure = Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(
        move |ev: web_sys::KeyboardEvent| {
            let is_ctrl = ev.ctrl_key() || ev.meta_key();
            match ev.key().as_str() {
                "k" | "K" if is_ctrl => {
                    ev.prevent_default();
                    write.ui.set_show_palette.update(|v| *v = !*v);
                }
                "Escape" => {
                    if close_top_of_stack(state, write) {
                        ev.prevent_default();
                    }
                }
                "ArrowUp" if ev.alt_key() => {
                    ev.prevent_default();
                    switch_grove(state, write, -1);
                }
                "ArrowDown" if ev.alt_key() => {
                    ev.prevent_default();
                    switch_grove(state, write, 1);
                }
                _ => {}
            }
        },
    );
    if let Some(w) = web_sys::window() {
        let _ = w.add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
    }
    closure.forget();
}

/// Pop one layer off the modal stack; returns true if something closed.
/// Priority (top → bottom): members rail → pinned rail → palette.
fn close_top_of_stack(state: AppState, write: AppWriteSignals) -> bool {
    if state.ui.show_members.get_untracked() {
        write.ui.set_show_members.set(false);
        return true;
    }
    if state.ui.show_pinned.get_untracked() {
        write.ui.set_show_pinned.set(false);
        return true;
    }
    if state.ui.show_palette.get_untracked() {
        write.ui.set_show_palette.set(false);
        return true;
    }
    false
}

/// Cycle grove selection by `delta` (wraps). No-op when the joined
/// groves list is empty.
fn switch_grove(state: AppState, write: AppWriteSignals, delta: i32) {
    let servers = state.server.servers.get_untracked();
    if servers.is_empty() {
        return;
    }
    let active = state.server.active_server_id.get_untracked();
    let idx = servers
        .iter()
        .position(|(id, _)| id == &active)
        .unwrap_or(0) as i32;
    let len = servers.len() as i32;
    let next = (idx + delta).rem_euclid(len) as usize;
    let (id, _) = &servers[next];
    write.server.set_active_server_id.set(id.clone());
}
