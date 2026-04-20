//! Grove rail — leftmost desktop navigation surface.
//!
//! Replaces the legacy `ServerList`. Contents top-to-bottom:
//!   1. Letters tile (direct messages)
//!   2. Divider
//!   3. Grove tiles (one per joined grove, in willow-state order)
//!   4. New-grove tile
//!   5. Discover tile
//!   6. Flex spacer
//!   7. Settings tile (pinned bottom)
//!
//! Keyboard: roving arrow keys between tiles, Enter activates,
//! Home / End jump to ends. `aria-label="groves"`.
//!
//! Spec: docs/specs/2026-04-19-ui-design/layout-primitives.md §Grove rail

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::app::WebClientHandle;
use crate::components::{ConfirmDialog, ContextMenu};
use crate::icons;

/// Which rail tile currently has roving focus.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RailFocus {
    Letters,
    Grove(usize),
    NewGrove,
    Discover,
    Settings,
}

/// Leftmost grove / workspace rail.
#[component]
pub fn GroveRail(
    servers: ReadSignal<Vec<(String, String)>>,
    active_server_id: ReadSignal<String>,
    on_server_click: impl Fn(String) + Send + Clone + 'static,
    on_add_server_click: impl Fn(()) + Send + Clone + 'static,
    /// Called when the user picks "Server Settings" from the context menu.
    #[prop(optional, into)]
    on_open_settings: Option<Callback<()>>,
    /// Called when the user activates the settings tile (pinned bottom).
    #[prop(optional, into)]
    on_settings_tile_click: Option<Callback<()>>,
) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();

    // Context menu state.
    let (show_menu, set_show_menu) = signal(false);
    let (menu_x, set_menu_x) = signal(0.0f64);
    let (menu_y, set_menu_y) = signal(0.0f64);
    let (menu_server_id, set_menu_server_id) = signal(Option::<String>::None);

    // Leave-server confirmation dialog.
    let (show_leave_confirm, set_show_leave_confirm) = signal(false);
    let (leave_server_id, set_leave_server_id) = signal(Option::<String>::None);
    let handle_leave = handle.clone();

    // Roving keyboard focus: tracks which tile is currently the tab stop.
    let (focus_tile, set_focus_tile) = signal(RailFocus::Letters);

    // Long-press timer for mobile.
    let long_press_timer =
        send_wrapper::SendWrapper::new(std::rc::Rc::new(std::cell::Cell::new(0i32)));

    // Build keyboard handler that walks tiles. We capture the server
    // count reactively so arrow-nav follows list length.
    let servers_for_kb = servers;
    let on_click_for_kb = on_server_click.clone();
    let on_add_for_kb = on_add_server_click.clone();
    let on_settings_for_kb = on_settings_tile_click;
    let rail_keydown = move |ev: web_sys::KeyboardEvent| {
        let total = servers_for_kb.get_untracked().len();
        let ordered: Vec<RailFocus> = std::iter::once(RailFocus::Letters)
            .chain((0..total).map(RailFocus::Grove))
            .chain([
                RailFocus::NewGrove,
                RailFocus::Discover,
                RailFocus::Settings,
            ])
            .collect();
        let current_idx = ordered
            .iter()
            .position(|t| *t == focus_tile.get_untracked())
            .unwrap_or(0);
        let key = ev.key();
        let new_idx = match key.as_str() {
            "ArrowDown" | "Down" => Some((current_idx + 1).min(ordered.len() - 1)),
            "ArrowUp" | "Up" => Some(current_idx.saturating_sub(1)),
            "Home" => Some(0),
            "End" => Some(ordered.len() - 1),
            "Enter" | " " => {
                ev.prevent_default();
                match focus_tile.get_untracked() {
                    RailFocus::Grove(i) => {
                        if let Some((id, _)) = servers_for_kb.get_untracked().get(i).cloned() {
                            on_click_for_kb(id);
                        }
                    }
                    RailFocus::NewGrove => on_add_for_kb(()),
                    RailFocus::Settings => {
                        if let Some(cb) = on_settings_for_kb {
                            cb.run(());
                        }
                    }
                    _ => {}
                }
                None
            }
            _ => None,
        };
        if let Some(i) = new_idx {
            ev.prevent_default();
            if let Some(t) = ordered.get(i).copied() {
                set_focus_tile.set(t);
                // Move DOM focus to the newly selected tile so roving
                // focus is visible and screen-reader-announced.
                if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                    let selector = match t {
                        RailFocus::Letters => ".grove-rail .rail-tile--letters".to_string(),
                        RailFocus::Grove(i) => {
                            format!(".grove-rail .grove-tile[data-index=\"{i}\"]")
                        }
                        RailFocus::NewGrove => ".grove-rail .rail-tile--new-grove".to_string(),
                        RailFocus::Discover => ".grove-rail .rail-tile--discover".to_string(),
                        RailFocus::Settings => ".grove-rail .rail-tile--settings".to_string(),
                    };
                    if let Some(el) = doc
                        .query_selector(&selector)
                        .ok()
                        .flatten()
                        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
                    {
                        el.focus().ok();
                    }
                }
            }
        }
    };

    view! {
        <nav
            class="grove-rail"
            role="navigation"
            aria-label="groves"
            on:keydown=rail_keydown
        >
            <div class="grove-rail-top-spacer"></div>

            // 1. Letters tile
            <button
                class="rail-tile rail-tile--letters"
                data-state="idle"
                title="letters · direct messages"
                aria-label="letters · direct messages"
                tabindex=move || if focus_tile.get() == RailFocus::Letters { "0" } else { "-1" }
                on:click=move |_| set_focus_tile.set(RailFocus::Letters)
            >
                {icons::icon_inbox()}
            </button>

            // 2. Divider
            <div class="grove-rail-divider" aria-hidden="true"></div>

            // 3. Grove tiles
            <div class="grove-rail-scroll noscroll">
                <For
                    each=move || {
                        servers.get()
                            .into_iter()
                            .enumerate()
                            .collect::<Vec<(usize, (String, String))>>()
                    }
                    key=|(i, (id, _))| format!("{i}:{id}")
                    let:entry
                >
                    {
                        let (idx, (id, name)) = entry;
                        let id_click = id.clone();
                        let id_active = id.clone();
                        let id_ctx = id.clone();
                        let id_touch = id.clone();
                        let name_for_title = name.clone();
                        let initial = name
                            .chars()
                            .next()
                            .unwrap_or('?')
                            .to_uppercase()
                            .to_string();
                        let on_click = on_server_click.clone();

                        // Long-press for mobile context menu.
                        let lp = long_press_timer.clone();
                        let lp_end = long_press_timer.clone();
                        let lp_move = long_press_timer.clone();

                        let on_touchstart = move |ev: web_sys::TouchEvent| {
                            let id_for_timer = id_touch.clone();
                            if let Some(touch) = ev.touches().get(0) {
                                let cx = touch.client_x() as f64;
                                let cy = touch.client_y() as f64;
                                if let Some(window) = web_sys::window() {
                                    let cb = wasm_bindgen::closure::Closure::once(move || {
                                        set_menu_server_id.set(Some(id_for_timer));
                                        set_menu_x.set(cx);
                                        set_menu_y.set(cy);
                                        set_show_menu.set(true);
                                        if let Some(w) = web_sys::window() {
                                            let _ = w.navigator().vibrate_with_duration(25);
                                        }
                                    });
                                    if let Ok(timer_id) =
                                        window.set_timeout_with_callback_and_timeout_and_arguments_0(
                                            cb.as_ref().unchecked_ref(),
                                            500,
                                        )
                                    {
                                        lp.set(timer_id);
                                    }
                                    cb.forget();
                                }
                            }
                        };

                        let on_touchend = move |_: web_sys::TouchEvent| {
                            let tid = lp_end.get();
                            if tid != 0 {
                                if let Some(w) = web_sys::window() {
                                    w.clear_timeout_with_handle(tid);
                                }
                                lp_end.set(0);
                            }
                        };

                        let on_touchmove = move |_: web_sys::TouchEvent| {
                            let tid = lp_move.get();
                            if tid != 0 {
                                if let Some(w) = web_sys::window() {
                                    w.clear_timeout_with_handle(tid);
                                }
                                lp_move.set(0);
                            }
                        };

                        view! {
                            <button
                                class="grove-tile server-icon"
                                data-index=idx.to_string()
                                data-state=move || {
                                    if active_server_id.get() == id_active { "active" } else { "idle" }
                                }
                                title=name_for_title
                                aria-label=name.clone()
                                tabindex=move || if focus_tile.get() == RailFocus::Grove(idx) { "0" } else { "-1" }
                                on:click=move |_| {
                                    set_focus_tile.set(RailFocus::Grove(idx));
                                    on_click(id_click.clone());
                                }
                                on:contextmenu=move |ev: web_sys::MouseEvent| {
                                    ev.prevent_default();
                                    set_menu_server_id.set(Some(id_ctx.clone()));
                                    set_menu_x.set(ev.client_x() as f64);
                                    set_menu_y.set(ev.client_y() as f64);
                                    set_show_menu.set(true);
                                }
                                on:touchstart=on_touchstart
                                on:touchend=on_touchend
                                on:touchmove=on_touchmove
                            >
                                <span class="grove-tile-glyph">{initial}</span>
                            </button>
                        }
                    }
                </For>

                // 4. New-grove tile
                <button
                    class="rail-tile rail-tile--new-grove server-icon add-server"
                    data-state="idle"
                    title="new grove"
                    aria-label="new grove"
                    tabindex=move || if focus_tile.get() == RailFocus::NewGrove { "0" } else { "-1" }
                    on:click={
                        let on_add = on_add_server_click.clone();
                        move |_| {
                            set_focus_tile.set(RailFocus::NewGrove);
                            on_add(());
                        }
                    }
                >
                    {icons::icon_plus()}
                </button>

                // 5. Discover tile
                <button
                    class="rail-tile rail-tile--discover"
                    data-state="idle"
                    title="discover"
                    aria-label="discover"
                    tabindex=move || if focus_tile.get() == RailFocus::Discover { "0" } else { "-1" }
                    on:click=move |_| set_focus_tile.set(RailFocus::Discover)
                >
                    {icons::icon_compass()}
                </button>
            </div>

            // 6. Flex spacer
            <div class="grove-rail-spacer"></div>

            // 7. Settings tile (pinned bottom)
            <button
                class="rail-tile rail-tile--settings"
                data-state="idle"
                title="settings"
                aria-label="settings"
                tabindex=move || if focus_tile.get() == RailFocus::Settings { "0" } else { "-1" }
                on:click=move |_| {
                    set_focus_tile.set(RailFocus::Settings);
                    if let Some(cb) = on_settings_tile_click {
                        cb.run(());
                    }
                }
            >
                {icons::icon_settings()}
            </button>
        </nav>

        <ContextMenu
            visible=show_menu
            x=menu_x
            y=menu_y
            on_close=Callback::new(move |_| set_show_menu.set(false))
        >
            {
                let settings_cb = on_open_settings;
                view! {
                    <button
                        class="context-menu-item"
                        on:click=move |_| {
                            set_show_menu.set(false);
                            if let Some(ref cb) = settings_cb { cb.run(()); }
                        }
                    >
                        "Server Settings"
                    </button>
                    <button
                        class="context-menu-item danger"
                        on:click=move |_| {
                            set_show_menu.set(false);
                            set_leave_server_id.set(menu_server_id.get_untracked());
                            set_show_leave_confirm.set(true);
                        }
                    >
                        "Leave Server"
                    </button>
                }
            }
        </ContextMenu>
        <ConfirmDialog
            visible=show_leave_confirm
            title="Leave Server"
            message=Signal::derive(move || {
                "Are you sure you want to leave this server? You will lose access to its channels and messages.".to_string()
            })
            confirm_text="Leave"
            danger=true
            on_confirm=Callback::new(move |_| {
                if let Some(sid) = leave_server_id.get_untracked() {
                    let h = handle_leave.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        h.leave_server(&sid).await;
                    });
                }
                set_leave_server_id.set(None);
                set_show_leave_confirm.set(false);
            })
            on_cancel=Callback::new(move |_| {
                set_leave_server_id.set(None);
                set_show_leave_confirm.set(false);
            })
        />
    }
}
