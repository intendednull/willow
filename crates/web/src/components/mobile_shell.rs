//! Mobile shell — the root of the viewport-≤720 px experience.
//!
//! Owns the mobile chrome: top bar, body container (host for the
//! current primary route or the current full-screen push), bottom tab
//! bar, grove drawer, and any bottom sheet currently open.
//!
//! Architecture:
//!   - `MobileTab` (Home / Letters / Discover / You) drives the tab bar
//!     and controls which primary route renders in the body.
//!   - `push_stack: Vec<MobilePush>` models full-screen routes pushed
//!     over the primary tab (channel chat, thread, call, onboarding).
//!     Rendering the stack is a <For> — each entry is an absolutely-
//!     positioned slide-in layer.
//!   - `drawer_open` is the grove drawer visibility. Tapping the top-
//!     bar grove glyph on `home` opens it; tapping backdrop / swipe-
//!     left / grove selection / Escape close it.
//!
//! Spec: docs/specs/2026-04-19-ui-design/layout-primitives.md §Mobile layout

use leptos::prelude::*;

use crate::components::{
    BottomSheet, ChannelSidebar, ChatInput, FileShareButton, GroveDrawer, MainPaneHeader,
    MessageList, RightRailWhich, TabBar,
};
use crate::icons;
use crate::state::{AppState, AppWriteSignals, SettingsTab};
use willow_client::DisplayMessage;

/// Mobile primary routes exposed via the tab bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MobileTab {
    #[default]
    Home,
    Letters,
    Discover,
    You,
}

impl MobileTab {
    pub fn id(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Letters => "letters",
            Self::Discover => "discover",
            Self::You => "you",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Home => "groves",
            Self::Letters => "letters",
            Self::Discover => "discover",
            Self::You => "you",
        }
    }
}

/// Full-screen pushes that can be stacked over a primary tab. Each
/// push hides the tab bar and renders translated in from the right.
#[derive(Clone, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub enum MobilePush {
    /// Channel chat view (name = channel id).
    Channel(String),
    /// Thread pane (parent message id).
    Thread(String),
    /// Live voice / video call surface.
    Call,
    /// Onboarding flow pushed over home.
    Onboarding,
}

/// Top-level mobile shell component.
///
/// Consumers must provide everything the shell needs to wire real
/// state (channel lists, voice actions, message handlers). The
/// desktop shell already has all these — see `app.rs`.
#[allow(clippy::too_many_arguments)]
#[component]
pub fn MobileShell<FSend, FEdit, FDelete, FReact, FChannel, FServer, FVoiceJoin, FPin>(
    /// Handlers provided by app.rs so the mobile shell can send
    /// messages, edit, delete, react, switch channel, switch server,
    /// join voice, pin.
    on_send: FSend,
    on_edit_send: FEdit,
    on_delete_msg: FDelete,
    on_react: FReact,
    on_channel_click: FChannel,
    on_server_click: FServer,
    on_voice_join: FVoiceJoin,
    on_pin: FPin,
) -> impl IntoView
where
    FSend: Fn(String) + Send + Sync + Clone + 'static,
    FEdit: Fn((String, String)) + Send + Sync + Clone + 'static,
    FDelete: Fn(DisplayMessage) + Send + Sync + Clone + 'static,
    FReact: Fn((DisplayMessage, String)) + Send + Sync + Clone + 'static,
    FChannel: Fn(String) + Send + Sync + Clone + 'static,
    FServer: Fn(String) + Send + Sync + Clone + 'static,
    FVoiceJoin: Fn(String) + Send + Sync + Clone + 'static,
    FPin: Fn(DisplayMessage) + Send + Sync + Clone + 'static,
{
    let app_state = use_context::<AppState>().expect("AppState context");
    let write = use_context::<AppWriteSignals>().expect("AppWriteSignals context");

    // Local shell signals — mobile navigation is entirely client-side.
    let (active_tab, set_active_tab) = signal(MobileTab::Home);
    let (push_stack, set_push_stack) = signal(Vec::<MobilePush>::new());
    let (drawer_open, set_drawer_open) = signal(false);
    let (sheet_open, _set_sheet_open) = signal(false);

    // Derived: tab bar hides when anything is pushed.
    let tab_bar_visible = Signal::derive(move || push_stack.get().is_empty());

    // Derived: top-bar title depends on push stack vs active tab.
    let top_title = Signal::derive(move || {
        let stack = push_stack.get();
        if let Some(top) = stack.last() {
            match top {
                MobilePush::Channel(name) => name.clone(),
                MobilePush::Thread(_) => "thread".to_string(),
                MobilePush::Call => "call".to_string(),
                MobilePush::Onboarding => "welcome".to_string(),
            }
        } else {
            match active_tab.get() {
                MobileTab::Home => app_state.server.active_server_name.get(),
                MobileTab::Letters => "letters".to_string(),
                MobileTab::Discover => "discover".to_string(),
                MobileTab::You => "you".to_string(),
            }
        }
    });

    // Derived: home-tab badge aggregates all per-grove unread counts.
    let badges = Signal::derive(move || {
        let unread = app_state.server.unread.get();
        let home_total: usize = unread.values().sum();
        [
            ("home", home_total),
            ("letters", 0),
            ("discover", 0),
            ("you", 0),
        ]
        .iter()
        .filter_map(|(k, v)| {
            if *v > 0 {
                Some((k.to_string(), *v))
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
    });

    // Back chevron pops the top push. Tapping grove glyph on home
    // opens the drawer.
    let on_back = move |_: ()| {
        set_push_stack.update(|stack| {
            stack.pop();
        });
    };
    let on_left_slot = move |_: ()| {
        if push_stack.get_untracked().is_empty() {
            set_drawer_open.set(true);
        } else {
            set_push_stack.update(|stack| {
                stack.pop();
            });
        }
    };

    // Tab change: switching away from a push resets the stack so the
    // new tab starts at its primary route.
    let on_tab_change = move |tab: MobileTab| {
        set_push_stack.update(|s| s.clear());
        set_active_tab.set(tab);
    };

    // Drawer close wiring.
    let on_drawer_close = Callback::new(move |_: ()| set_drawer_open.set(false));

    // Search-button opens the local-search surface directly on mobile
    // (per `local-search.md` §Mobile — top-bar overflow search).
    // Scope defaults to `this channel` when a channel is focused; the
    // scope chip lets the user widen.
    let on_search = Callback::new(move |_: ()| {
        let ch = app_state.chat.current_channel.get_untracked();
        if !ch.is_empty() {
            write
                .search
                .set_scope
                .set(willow_client::SearchScope::ThisChannel(ch));
        }
        write.search.set_open.set(true);
    });

    // Grove-drawer grove-select handler — switches the active grove
    // and closes the drawer.
    let on_drawer_server_click = {
        let on_server_click = on_server_click.clone();
        move |id: String| {
            on_server_click(id);
            set_drawer_open.set(false);
        }
    };

    // Channel-click on home pushes the channel route over the home
    // tab — the tab bar hides while the chat stack is on screen.
    let on_channel_tap = {
        let on_channel_click = on_channel_click.clone();
        move |name: String| {
            on_channel_click(name.clone());
            set_push_stack.update(|stack| stack.push(MobilePush::Channel(name)));
        }
    };

    // Pop the whole stack (used when leaving a thread / call that was
    // itself pushed onto a channel — callers decide how deep to go).
    let _reset_push = move || set_push_stack.set(Vec::new());

    // Empty-state helper: whether the active grove has no channels.
    let no_channels = Signal::derive(move || app_state.chat.channels.get().is_empty());
    // Empty-state helper: whether the user has zero groves joined.
    let no_groves = Signal::derive(move || app_state.server.servers.get().is_empty());

    // Pre-clone handlers for the channel push body.
    let on_send = on_send.clone();
    let on_edit_send = on_edit_send.clone();
    let on_delete_msg = on_delete_msg.clone();
    let on_react = on_react.clone();
    let on_pin = on_pin.clone();
    let on_voice_join_sidebar = on_voice_join.clone();

    // Alias some signals for concise view closures.
    let current_channel = app_state.chat.current_channel;
    let messages = app_state.chat.messages;
    let replying_to = app_state.chat.replying_to;
    let editing = app_state.chat.editing;
    let channel_views = app_state.chat.channel_views;
    let pin_labels = app_state.chat.pin_labels;
    let display_name = app_state.server.display_name;
    let loading = app_state.network.loading;
    let peer_count = app_state.network.peer_count;
    let show_sidebar = app_state.ui.show_sidebar;
    let active_server_name = app_state.server.active_server_name;

    view! {
        <div class="mobile-shell" data-tab=move || active_tab.get().id()>
            // ── Top bar ────────────────────────────────────────────
            <header class="mobile-top-bar" role="banner" aria-label="top bar">
                <button
                    class="top-slot-left"
                    aria-label=move || {
                        if push_stack.get().is_empty() { "open groves".to_string() }
                        else { "back".to_string() }
                    }
                    on:click=move |_| on_left_slot(())
                >
                    {move || {
                        if push_stack.get().is_empty() {
                            let glyph = active_server_name.get()
                                .chars()
                                .next()
                                .map(|c| c.to_ascii_uppercase().to_string())
                                .unwrap_or_else(|| "·".to_string());
                            view! { <span class="top-glyph">{glyph}</span> }.into_any()
                        } else {
                            view! { <span class="top-back">{icons::icon_arrow_left()}</span> }.into_any()
                        }
                    }}
                </button>
                <div class="top-title-col">
                    <div class="top-title">{move || top_title.get()}</div>
                    <div class="top-subtitle">
                        {move || format!("{} peers", peer_count.get())}
                    </div>
                </div>
                <div class="top-slot-right">
                    <button
                        class="top-action"
                        aria-label="search (⌘K)"
                        title="search"
                        on:click=move |_| on_search.run(())
                    >
                        {icons::icon_search()}
                    </button>
                </div>
            </header>

            // ── Body: primary route or top-of-push-stack ──────────
            <div class="mobile-body">
                {move || {
                    if no_groves.get() {
                        return view! {
                            <div class="mobile-empty state-empty">
                                <div class="state-empty__headline">
                                    "no groves yet — start one"
                                </div>
                                <button class="btn mobile-empty-cta"
                                    on:click=move |_| {
                                        write.ui.set_show_add_server.set(true);
                                    }
                                >
                                    "new grove"
                                </button>
                            </div>
                        }.into_any();
                    }

                    let stack = push_stack.get();
                    if let Some(top) = stack.last() {
                        match top {
                            MobilePush::Channel(name) => {
                                let ch_name = name.clone();
                                let send = on_send.clone();
                                let edit_send = on_edit_send.clone();
                                let delete_msg = on_delete_msg.clone();
                                let react = on_react.clone();
                                let pin = on_pin.clone();
                                view! {
                                    <div class="mobile-push mobile-push--channel"
                                        data-channel=ch_name.clone()>
                                        <MainPaneHeader
                                            channel=current_channel
                                            which=Signal::derive(|| RightRailWhich::None)
                                            on_set_which=Callback::new(|_| ())
                                            on_search_click=Callback::new(move |_| {
                                                write.ui.set_show_palette.set(true);
                                            })
                                        />
                                        <MessageList
                                            messages=messages
                                            loading=Signal::from(loading)
                                            local_display_name={
                                                let s: Signal<String> = Signal::from(display_name);
                                                s
                                            }
                                            on_message_click=Callback::new(move |msg: DisplayMessage| {
                                                write.chat.set_replying_to.set(Some(msg));
                                            })
                                            on_edit=Callback::new(move |msg: DisplayMessage| {
                                                write.chat.set_editing.set(Some(msg));
                                            })
                                            on_delete=Callback::new(move |msg: DisplayMessage| {
                                                delete_msg(msg);
                                            })
                                            on_react=Callback::new(move |args: (DisplayMessage, String)| {
                                                react(args);
                                            })
                                            on_pin=Callback::new(move |msg: DisplayMessage| {
                                                pin(msg);
                                            })
                                            pin_labels=Signal::from(pin_labels)
                                            // Phase 2a Task 15 — Escape returns
                                            // focus to the composer textarea
                                            // per spec §Accessibility keyboard
                                            // path. Mirrors the desktop wiring
                                            // in `app.rs`.
                                            on_focus_composer=Callback::new(move |()| {
                                                js_sys::eval("var i=document.querySelector('.input-area input,.input-area textarea');if(i)i.focus();").ok();
                                            })
                                        />
                                        <div class="typing-indicator">
                                            {move || {
                                                let ch = current_channel.get();
                                                let views = channel_views.get();
                                                let names = views
                                                    .get(&ch)
                                                    .map(|v| v.typing.clone())
                                                    .unwrap_or_default();
                                                match names.len() {
                                                    0 => String::new(),
                                                    1 => format!("{} is typing...", names[0]),
                                                    2 => format!("{} and {} are typing...",
                                                        names[0], names[1]),
                                                    _ => "Multiple people are typing...".to_string(),
                                                }
                                            }}
                                        </div>
                                        <div class="input-row">
                                            <FileShareButton channel=current_channel />
                                            <ChatInput
                                                on_send=send
                                                replying_to=replying_to
                                                on_cancel_reply=Callback::new(move |_| {
                                                    write.chat.set_replying_to.set(None);
                                                })
                                                editing=editing
                                                on_edit_send=Callback::new(move |args: (String, String)| {
                                                    edit_send(args);
                                                })
                                                on_cancel_edit=Callback::new(move |_| {
                                                    write.chat.set_editing.set(None);
                                                })
                                                on_typing=Callback::new(|_| ())
                                            />
                                        </div>
                                    </div>
                                }.into_any()
                            }
                            MobilePush::Thread(_) => view! {
                                <div class="mobile-push mobile-push--thread">
                                    <div class="state-empty">
                                        <div class="state-empty__headline">"thread"</div>
                                    </div>
                                </div>
                            }.into_any(),
                            MobilePush::Call => view! {
                                <div class="mobile-push mobile-push--call">
                                    <div class="state-empty">
                                        <div class="state-empty__headline">"call"</div>
                                    </div>
                                </div>
                            }.into_any(),
                            MobilePush::Onboarding => view! {
                                <div class="mobile-push mobile-push--onboarding">
                                    <div class="state-empty">
                                        <div class="state-empty__headline">"welcome"</div>
                                    </div>
                                </div>
                            }.into_any(),
                        }
                    } else {
                        // No push — show the active tab's primary view.
                        match active_tab.get() {
                            MobileTab::Home => {
                                let ch_tap = on_channel_tap.clone();
                                let vj = on_voice_join_sidebar.clone();
                                view! {
                                    <div class="mobile-home" role="main" aria-label="home">
                                        {move || if no_channels.get() {
                                            Some(view! {
                                                <div class="mobile-empty state-empty">
                                                    <div class="state-empty__headline">
                                                        "this grove is quiet. say hi?"
                                                    </div>
                                                    <div class="state-empty__hint">
                                                        "add a channel from the grove menu."
                                                    </div>
                                                </div>
                                            })
                                        } else {
                                            None
                                        }}
                                        <ChannelSidebar
                                            channels=app_state.chat.channels
                                            current_channel=current_channel
                                            open=show_sidebar
                                            unread=app_state.server.unread_stats
                                            server_name=active_server_name
                                            on_channel_click={
                                                let ch_tap = ch_tap.clone();
                                                move |name: String| ch_tap(name)
                                            }
                                            on_settings_click=move |_| {
                                                write.ui.set_settings_tab.set(SettingsTab::Profile);
                                                write.ui.set_show_settings.set(true);
                                            }
                                            on_server_settings_click=move |_| {
                                                write.ui.set_settings_tab.set(SettingsTab::Server);
                                                write.ui.set_show_settings.set(true);
                                            }
                                            on_voice_join={
                                                let vj = vj.clone();
                                                move |name: String| vj(name)
                                            }
                                            voice_channel=app_state.voice.voice_channel
                                            voice_channel_name=app_state.voice.voice_channel_name
                                            voice_muted=app_state.voice.voice_muted
                                            voice_deafened=app_state.voice.voice_deafened
                                            on_voice_mute=Callback::new(|_: ()| ())
                                            on_voice_deafen=Callback::new(|_: ()| ())
                                            on_voice_disconnect=Callback::new(|_: ()| ())
                                            on_channel_created=move |_: ()| {}
                                        />
                                    </div>
                                }.into_any()
                            }
                            MobileTab::Letters => view! {
                                <div class="mobile-tab-empty state-empty"
                                    role="main" aria-label="letters">
                                    <div class="state-empty__headline">
                                        "letters come to life here"
                                    </div>
                                    <div class="state-empty__hint">
                                        "direct messages are still being built."
                                    </div>
                                </div>
                            }.into_any(),
                            MobileTab::Discover => view! {
                                <div class="mobile-tab-empty state-empty"
                                    role="main" aria-label="discover">
                                    <div class="state-empty__headline">
                                        "discover other groves"
                                    </div>
                                    <div class="state-empty__hint">
                                        "public grove discovery lands soon."
                                    </div>
                                </div>
                            }.into_any(),
                            MobileTab::You => view! {
                                <div class="mobile-tab-empty state-empty"
                                    role="main" aria-label="you">
                                    <div class="state-empty__headline">"you"</div>
                                    <div class="state-empty__hint">
                                        "profile and settings will live here."
                                    </div>
                                </div>
                            }.into_any(),
                        }
                    }
                }}
            </div>

            // ── Tab bar (hidden when something is pushed) ─────────
            <TabBar
                active=Signal::from(active_tab)
                badges=badges
                visible=tab_bar_visible
                on_tab_change=Callback::new(on_tab_change)
            />

            // ── Grove drawer (home only) ──────────────────────────
            <GroveDrawer
                open=Signal::from(drawer_open)
                servers=app_state.server.servers
                active_server_id=app_state.server.active_server_id
                peer_count=peer_count
                display_name=display_name
                on_close=on_drawer_close
                on_server_click=Callback::new(on_drawer_server_click)
                on_new_grove=Callback::new(move |_: ()| {
                    write.ui.set_show_add_server.set(true);
                    set_drawer_open.set(false);
                })
                on_open_settings=Callback::new(move |_: ()| {
                    write.ui.set_settings_tab.set(SettingsTab::Profile);
                    write.ui.set_show_settings.set(true);
                    set_drawer_open.set(false);
                })
            />

            // ── Bottom sheet host (placeholder — used by future
            //    profile-sheet / confirm-sheet consumers). Wiring
            //    lands as consumers arrive. ─────────────────────────
            <BottomSheet
                open=Signal::from(sheet_open)
                label="action sheet".to_string()
                on_close=Callback::new(move |_: ()| {})
            >
                <div />
            </BottomSheet>

            // ── Back-gesture + drawer-gesture listeners.
            //    Wired by GestureBinder below (task 8). ────────────
            <GestureBinder
                drawer_open=Signal::from(drawer_open)
                push_stack=Signal::from(push_stack)
                on_back=Callback::new(on_back)
                on_drawer_open=Callback::new(move |_: ()| {
                    if push_stack.get_untracked().is_empty() {
                        set_drawer_open.set(true);
                    }
                })
                on_drawer_close=Callback::new(move |_: ()| set_drawer_open.set(false))
            />
        </div>
    }
}

// Gesture-binder wires window-level pointer listeners so the
// mobile shell can respond to edge-swipe / back-swipe / drawer-
// close gestures without every child component opting in.
//
// Thresholds (spec §Mobile gestures):
//   - Edge-swipe open drawer: pointerdown x ≤ 20, pointerup dx > 60.
//   - Swipe-left close drawer: dx < -60 while drawer is open.
//   - Back-swipe: same edge-swipe when push stack is non-empty.
//   - Sheet-dismiss: lives in `bottom_sheet.rs`.
#[component]
fn GestureBinder(
    drawer_open: Signal<bool>,
    push_stack: Signal<Vec<MobilePush>>,
    on_back: Callback<()>,
    on_drawer_open: Callback<()>,
    on_drawer_close: Callback<()>,
) -> impl IntoView {
    use wasm_bindgen::JsCast;

    // Track pointer start position in StoredValues so the window
    // callbacks can read + write without borrowing.
    let start_x = StoredValue::new(0.0f64);
    let start_y = StoredValue::new(0.0f64);
    let is_tracking = StoredValue::new(false);

    // pointerdown: record start coords + arm tracking if the touch
    // starts on the left edge (≤ 20 px) or the drawer is already
    // open (so a left-swipe anywhere closes it).
    {
        let down = wasm_bindgen::closure::Closure::<dyn Fn(web_sys::PointerEvent)>::new(
            move |ev: web_sys::PointerEvent| {
                let x = ev.client_x() as f64;
                let y = ev.client_y() as f64;
                start_x.set_value(x);
                start_y.set_value(y);
                is_tracking.set_value(x <= 20.0 || drawer_open.get_untracked());
            },
        );
        if let Some(window) = web_sys::window() {
            window
                .add_event_listener_with_callback("pointerdown", down.as_ref().unchecked_ref())
                .ok();
        }
        down.forget();
    }

    // pointerup: classify the gesture against dx/dy and fire.
    {
        let up = wasm_bindgen::closure::Closure::<dyn Fn(web_sys::PointerEvent)>::new(
            move |ev: web_sys::PointerEvent| {
                if !is_tracking.get_value() {
                    return;
                }
                is_tracking.set_value(false);

                let dx = ev.client_x() as f64 - start_x.get_value();
                let dy = ev.client_y() as f64 - start_y.get_value();
                // Ignore mostly-vertical swipes so taps / scrolls
                // don't fire horizontal gestures.
                if dy.abs() > dx.abs() {
                    return;
                }

                let drawer = drawer_open.get_untracked();
                let has_push = !push_stack.get_untracked().is_empty();

                if dx > 60.0 {
                    if has_push {
                        on_back.run(());
                    } else if !drawer {
                        on_drawer_open.run(());
                    }
                } else if dx < -60.0 && drawer {
                    on_drawer_close.run(());
                }
            },
        );
        if let Some(window) = web_sys::window() {
            window
                .add_event_listener_with_callback("pointerup", up.as_ref().unchecked_ref())
                .ok();
        }
        up.forget();
    }

    view! { <span class="mobile-gesture-binder" aria-hidden="true"></span> }
}
