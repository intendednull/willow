use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::app::WebClientHandle;
use crate::components::{ConfirmDialog, ContextMenu};

/// Discord-style vertical server icon rail on the far left.
#[component]
pub fn ServerList(
    servers: ReadSignal<Vec<(String, String)>>,
    active_server_id: ReadSignal<String>,
    on_server_click: impl Fn(String) + Send + Clone + 'static,
    on_add_server_click: impl Fn(()) + Send + Clone + 'static,
    /// Called when the user picks "Server Settings" from the context menu.
    #[prop(optional, into)]
    on_open_settings: Option<Callback<()>>,
) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();

    // Context menu state.
    let (show_menu, set_show_menu) = signal(false);
    let (menu_x, set_menu_x) = signal(0.0f64);
    let (menu_y, set_menu_y) = signal(0.0f64);
    let (menu_server_id, set_menu_server_id) = signal(Option::<String>::None);

    // Leave-server confirmation dialog state.
    let (show_leave_confirm, set_show_leave_confirm) = signal(false);
    let (leave_server_id, set_leave_server_id) = signal(Option::<String>::None);
    let handle_leave = handle.clone();

    // Long-press timer for mobile.
    let long_press_timer =
        send_wrapper::SendWrapper::new(std::rc::Rc::new(std::cell::Cell::new(0i32)));

    view! {
        <div class="server-rail">
            <For
                each=move || servers.get()
                key=|(id, _)| id.clone()
                let:server
            >
                {
                    let (id, name) = server;
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

                    // Long-press setup for mobile.
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
                                    // Haptic feedback.
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
                        <div
                            class=move || {
                                if active_server_id.get() == id_active {
                                    "server-icon active"
                                } else {
                                    "server-icon"
                                }
                            }
                            title=name_for_title
                            on:click=move |_| on_click(id_click.clone())
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
                            {initial}
                        </div>
                    }
                }
            </For>

            <div class="server-rail-divider"></div>

            // Join/create server button
            <div
                class="server-icon add-server"
                title="Join or Create Server"
                on:click=move |_| on_add_server_click(())
            >
                "+"
            </div>
        </div>
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
