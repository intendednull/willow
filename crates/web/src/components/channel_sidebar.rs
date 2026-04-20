//! Channel sidebar — the second pane on the desktop shell.
//!
//! Replaces the legacy `Sidebar`. Top-to-bottom:
//!   1. Grove header (glyph tile + italic grove name + chip + status
//!      row + tagline + chevron)
//!   2. Channel scroll region — four canonical groups
//!      (commons / voice / ephemeral / archives)
//!   3. Me strip (self profile card: avatar + display name + fingerprint
//!      + mic + deafen)
//!   4. Net status footer
//!
//! Spec: docs/specs/2026-04-19-ui-design/layout-primitives.md §Channel sidebar

use std::collections::HashMap;

use leptos::prelude::*;

use crate::app::WebClientHandle;
use crate::components::{
    ConfirmDialog, PeerStatusLabel, PresenceMenu, StatusDot, StatusDotBorder, StatusDotSize,
    VoiceControls,
};
use crate::icons;

/// Canonical channel groups — four labels in this render order when
/// non-empty. Unknown-prefix channels fall into `Commons`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ChannelGroup {
    Commons,
    Voice,
    Ephemeral,
    Archives,
}

impl ChannelGroup {
    /// Classify a channel by name + kind. Uses the name-prefix heuristic
    /// (see plan §Ambiguity decisions) for ephemeral / archive since no
    /// first-class `ChannelKind` exists yet.
    pub fn classify(name: &str, kind: &willow_state::ChannelKind) -> Self {
        if name.starts_with("_ephemeral-") {
            Self::Ephemeral
        } else if name.starts_with("_archive-") {
            Self::Archives
        } else if matches!(kind, willow_state::ChannelKind::Voice) {
            Self::Voice
        } else {
            Self::Commons
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Commons => "commons",
            Self::Voice => "voice",
            Self::Ephemeral => "ephemeral",
            Self::Archives => "archives",
        }
    }

    pub fn css_key(self) -> &'static str {
        match self {
            Self::Commons => "commons",
            Self::Voice => "voice",
            Self::Ephemeral => "ephemeral",
            Self::Archives => "archives",
        }
    }

    /// Render order used by ChannelSidebar.
    pub const ORDER: [Self; 4] = [Self::Commons, Self::Voice, Self::Ephemeral, Self::Archives];
}

/// Channel sidebar — grove header + channel groups + me strip + footer.
#[allow(clippy::too_many_arguments)]
#[component]
pub fn ChannelSidebar(
    channels: ReadSignal<Vec<String>>,
    current_channel: ReadSignal<String>,
    open: ReadSignal<bool>,
    unread: ReadSignal<HashMap<String, usize>>,
    connection_status: ReadSignal<String>,
    peer_count: ReadSignal<usize>,
    server_name: ReadSignal<String>,
    on_channel_click: impl Fn(String) + Send + Clone + 'static,
    on_settings_click: impl Fn(()) + Send + Clone + 'static,
    on_server_settings_click: impl Fn(()) + Send + Clone + 'static,
    on_voice_join: impl Fn(String) + Send + Clone + 'static,
    on_channel_created: impl Fn(()) + Send + Clone + 'static,
    /// Voice channel the user is currently in, if any.
    #[prop(optional)]
    voice_channel: Option<ReadSignal<Option<String>>>,
    /// Name of the current voice channel.
    #[prop(optional)]
    voice_channel_name: Option<ReadSignal<String>>,
    /// Whether the local mic is muted.
    #[prop(optional)]
    voice_muted: Option<ReadSignal<bool>>,
    /// Whether the local audio output is deafened.
    #[prop(optional)]
    voice_deafened: Option<ReadSignal<bool>>,
    #[prop(optional)] on_voice_mute: Option<Callback<()>>,
    #[prop(optional)] on_voice_deafen: Option<Callback<()>>,
    #[prop(optional)] on_voice_disconnect: Option<Callback<()>>,
) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();
    let app_state = use_context::<crate::state::AppState>().unwrap();

    // Admins (including the genesis owner) can create and delete channels.
    let peer_id = app_state.network.peer_id;
    let can_manage_channels = move || app_state.server.admin_ids.get().contains(&peer_id.get());

    // Channel-create input state (kept verbatim from the legacy Sidebar).
    let (creating, set_creating) = signal(false);
    let (new_name, set_new_name) = signal(String::new());
    let (create_voice, set_create_voice) = signal(false);

    // Channel delete confirmation state.
    let (show_del_confirm, set_show_del_confirm) = signal(false);
    let (pending_del_channel, set_pending_del_channel) = signal(Option::<String>::None);
    let handle_del_confirm = handle.clone();

    let handle_create = handle.clone();
    let on_create_submit = move || {
        let name = new_name.get_untracked();
        let name = name.trim().to_string();
        let is_voice = create_voice.get_untracked();
        if !name.is_empty() {
            let h = handle_create.clone();
            let name = name.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if is_voice {
                    let _ = h.create_voice_channel(&name).await;
                } else {
                    let _ = h.create_channel(&name).await;
                }
            });
            on_channel_created(());
        }
        set_new_name.set(String::new());
        set_creating.set(false);
        set_create_voice.set(false);
    };

    let on_create_keydown = {
        let submit = on_create_submit.clone();
        move |ev: web_sys::KeyboardEvent| {
            if ev.key() == "Enter" {
                ev.prevent_default();
                submit();
            } else if ev.key() == "Escape" {
                set_creating.set(false);
                set_new_name.set(String::new());
            }
        }
    };

    // Collapsed groups (persisted locally — defaults to all expanded).
    let (collapsed, set_collapsed) = signal(std::collections::HashSet::<&'static str>::new());

    // Presence menu open/close + self-state signal.
    let (presence_menu_open, set_presence_menu_open) = signal(false);
    let self_presence = app_state.presence.self_state;

    view! {
        <aside
            class=move || {
                if open.get() {
                    "channel-sidebar sidebar open"
                } else {
                    "channel-sidebar sidebar"
                }
            }
            role="navigation"
            aria-label="channels"
        >
            // ── Grove header ───────────────────────────────────────
            <div class="grove-header sidebar-header">
                <div class="grove-header-top">
                    <div class="grove-header-glyph" aria-hidden="true">
                        {move || {
                            server_name.get()
                                .chars()
                                .next()
                                .unwrap_or('?')
                                .to_uppercase()
                                .to_string()
                        }}
                    </div>
                    <div class="grove-header-name-col">
                        <div class="grove-header-row">
                            <span
                                class="grove-header-name"
                                title=move || server_name.get()
                            >
                                {move || server_name.get()}
                            </span>
                            <span
                                class="grove-chip"
                                title="a grove is a small private network of peers — no central server"
                            >
                                "grove"
                            </span>
                        </div>
                        <div class="grove-header-status">
                            {icons::icon_users()}
                            <span class="grove-status-peers">
                                {move || format!("{} peers", peer_count.get())}
                            </span>
                            <span class="grove-status-sep">"·"</span>
                            {icons::icon_lock()}
                            <span class="grove-status-e2e">"e2e"</span>
                        </div>
                    </div>
                    <button
                        class="grove-menu-chevron server-gear-btn"
                        title="grove menu"
                        aria-label="grove menu"
                        on:click=move |_| on_server_settings_click(())
                    >
                        {icons::icon_chevron_down()}
                    </button>
                </div>
                <div class="grove-tagline">"not a server — held between us"</div>
            </div>

            // ── Channel scroll region ──────────────────────────────
            <div class="channel-list scroll" role="list">
                {move || {
                    if creating.get() {
                        Some(view! {
                            <div class="channel-create-input">
                                <div class="channel-type-toggle">
                                    <button
                                        class=move || if !create_voice.get() { "type-btn active" } else { "type-btn" }
                                        on:mousedown=move |ev: web_sys::MouseEvent| {
                                            ev.prevent_default();
                                            set_create_voice.set(false);
                                        }
                                    >"# Text"</button>
                                    <button
                                        class=move || if create_voice.get() { "type-btn active" } else { "type-btn" }
                                        on:mousedown=move |ev: web_sys::MouseEvent| {
                                            ev.prevent_default();
                                            set_create_voice.set(true);
                                        }
                                    >{icons::icon_volume_2()} " Voice"</button>
                                </div>
                                <input
                                    type="text"
                                    placeholder=move || if create_voice.get() { "voice channel name" } else { "channel name" }
                                    prop:value=move || new_name.get()
                                    on:input=move |ev| set_new_name.set(event_target_value(&ev))
                                    on:keydown=on_create_keydown.clone()
                                />
                            </div>
                        })
                    } else {
                        None
                    }
                }}
                {move || can_manage_channels().then(|| view! {
                    <button
                        class="channel-add-btn"
                        title="create channel"
                        on:click=move |_| set_creating.set(true)
                    >
                        {icons::icon_plus()} " new channel"
                    </button>
                })}

                // Four canonical groups in ORDER; empty groups hide entirely.
                {
                    let ch_click = on_channel_click.clone();
                    let on_voice = on_voice_join.clone();
                    move || {
                        let ch_list = channels.get();
                        let kinds = app_state.server.channel_kinds.get();
                        let mut kind_map: HashMap<String, willow_state::ChannelKind> = HashMap::new();
                        for (n, k) in kinds {
                            kind_map.insert(n, k);
                        }

                        let mut grouped: HashMap<ChannelGroup, Vec<String>> = HashMap::new();
                        for name in &ch_list {
                            let default_kind = willow_state::ChannelKind::Text;
                            let kind = kind_map.get(name).unwrap_or(&default_kind);
                            grouped.entry(ChannelGroup::classify(name, kind)).or_default().push(name.clone());
                        }

                        if ch_list.is_empty() {
                            return view! {
                                <div class="channel-empty-state">
                                    <div class="channel-empty-headline">"this grove is quiet."</div>
                                    <div class="channel-empty-sub">"add a channel from the grove menu."</div>
                                </div>
                            }.into_any();
                        }

                        let sections: Vec<_> = ChannelGroup::ORDER.iter().filter_map(|group| {
                            let rows = grouped.remove(group).unwrap_or_default();
                            if rows.is_empty() {
                                return None;
                            }
                            let group_copy = *group;
                            let ch_click = ch_click.clone();
                            let on_voice = on_voice.clone();
                            let is_collapsed = move || collapsed.get().contains(group_copy.css_key());
                            Some(view! {
                                <div class="channel-group" data-group=group_copy.css_key()>
                                    <button
                                        class="channel-group-label"
                                        aria-expanded=move || if is_collapsed() { "false" } else { "true" }
                                        on:click=move |_| {
                                            set_collapsed.update(|s| {
                                                if s.contains(group_copy.css_key()) {
                                                    s.remove(group_copy.css_key());
                                                } else {
                                                    s.insert(group_copy.css_key());
                                                }
                                            });
                                        }
                                    >
                                        <span class="channel-group-chevron">
                                            {move || if is_collapsed() {
                                                icons::icon_chevron_right().into_any()
                                            } else {
                                                icons::icon_chevron_down().into_any()
                                            }}
                                        </span>
                                        <span class="channel-group-name">{group_copy.label()}</span>
                                        {matches!(group_copy, ChannelGroup::Ephemeral).then(|| view! {
                                            <span class="channel-group-meta" aria-hidden="true">
                                                {icons::icon_hourglass()}
                                            </span>
                                        })}
                                    </button>
                                    {move || {
                                        if is_collapsed() {
                                            None
                                        } else {
                                            let rows = rows.clone();
                                            let ch_click = ch_click.clone();
                                            let on_voice = on_voice.clone();
                                            Some(view! {
                                                <div class="channel-group-rows">
                                                    {rows.into_iter().map(|name| {
                                                        render_channel_row(
                                                            name,
                                                            group_copy,
                                                            current_channel,
                                                            unread,
                                                            can_manage_channels,
                                                            ch_click.clone(),
                                                            on_voice.clone(),
                                                            set_pending_del_channel,
                                                            set_show_del_confirm,
                                                        )
                                                    }).collect::<Vec<_>>()}
                                                </div>
                                            })
                                        }
                                    }}
                                </div>
                            })
                        }).collect();

                        view! { <div class="channel-groups">{sections}</div> }.into_any()
                    }
                }
            </div>

            // ── Me strip ───────────────────────────────────────────
            <div
                class="me-strip"
                style="position: relative"
                on:click=move |_| on_settings_click(())
            >
                <div class="me-avatar">
                    <div class="me-avatar-glyph">
                        {move || {
                            app_state.server.display_name.get()
                                .chars().next().unwrap_or('?')
                                .to_uppercase().to_string()
                        }}
                    </div>
                    <StatusDot
                        state=self_presence
                        size=StatusDotSize::MeStrip
                        border=StatusDotBorder::Bg1
                        ambient=true
                    />
                </div>
                <div class="me-identity">
                    <span class="me-display-name">
                        {move || {
                            let name = app_state.server.display_name.get();
                            if name.is_empty() { "you".to_string() } else { name }
                        }}
                        {move || {
                            let pid = peer_id.get();
                            if pid.is_empty() {
                                None
                            } else {
                                Some(view! {
                                    <super::TrustBadge
                                        peer_id=pid
                                        size=super::TrustBadgeSize::Disk12
                                    />
                                })
                            }
                        }}
                    </span>
                    <span class="me-fingerprint">
                        {move || short_fingerprint(&peer_id.get())}
                    </span>
                    <button
                        class="presence-menu-trigger"
                        aria-haspopup="menu"
                        aria-live="polite"
                        aria-label=move || format!(
                            "change your status · currently {}",
                            self_presence.get().label()
                        )
                        on:click=move |ev: web_sys::MouseEvent| {
                            ev.stop_propagation();
                            set_presence_menu_open.update(|v| *v = !*v);
                        }
                    >
                        <PeerStatusLabel state=self_presence show_dot=false/>
                        {icons::icon_chevron_down()}
                    </button>
                </div>
                {move || {
                    if presence_menu_open.get() {
                        Some(view! {
                            <PresenceMenu
                                open=presence_menu_open
                                on_close=Callback::new(move |_| set_presence_menu_open.set(false))
                            />
                        })
                    } else {
                        None
                    }
                }}
                <div class="me-actions" on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()>
                    {move || {
                        let muted = voice_muted.map(|s| s.get()).unwrap_or(false);
                        let deafened = voice_deafened.map(|s| s.get()).unwrap_or(false);
                        view! {
                            <button
                                class=if muted { "me-mic muted" } else { "me-mic" }
                                title="mic"
                                aria-label="mic"
                                on:click=move |_| {
                                    if let Some(cb) = on_voice_mute { cb.run(()); }
                                }
                            >
                                {if muted { icons::icon_mic_off().into_any() } else { icons::icon_mic().into_any() }}
                            </button>
                            <button
                                class=if deafened { "me-deafen muted" } else { "me-deafen" }
                                title="deafen"
                                aria-label="deafen"
                                on:click=move |_| {
                                    if let Some(cb) = on_voice_deafen { cb.run(()); }
                                }
                            >
                                {if deafened { icons::icon_headphones_off().into_any() } else { icons::icon_headphones().into_any() }}
                            </button>
                        }
                    }}
                </div>
            </div>

            // ── Voice controls (active call) ───────────────────────
            {move || {
                let vc = voice_channel.and_then(|s| s.get());
                if vc.is_some() {
                    let vcn = voice_channel_name.expect("voice_channel_name required");
                    let vm = voice_muted.expect("voice_muted required");
                    let vd = voice_deafened.expect("voice_deafened required");
                    let on_m = on_voice_mute.expect("on_voice_mute required");
                    let on_d = on_voice_deafen.expect("on_voice_deafen required");
                    let on_dc = on_voice_disconnect.expect("on_voice_disconnect required");
                    Some(view! {
                        <VoiceControls
                            channel_name=vcn
                            muted=vm
                            deafened=vd
                            on_mute=move |v| on_m.run(v)
                            on_deafen=move |v| on_d.run(v)
                            on_disconnect=move |v| on_dc.run(v)
                        />
                    })
                } else {
                    None
                }
            }}

            // ── Net status footer ──────────────────────────────────
            <div class="net-status-footer">
                {move || {
                    let status = connection_status.get();
                    let n = peer_count.get();
                    let offline = status != "connected";
                    if offline {
                        view! {
                            <>
                                <span class="pulse-dot pulse-dot--offline" aria-hidden="true"></span>
                                <span class="net-offline">"queued · waiting for peers"</span>
                            </>
                        }.into_any()
                    } else {
                        view! {
                            <>
                                <span class="pulse-dot" aria-hidden="true"></span>
                                <span class="net-peer-count">
                                    {if n == 1 { "1 peer".to_string() } else { format!("{n} peers") }}
                                </span>
                                <span class="net-sep">"·"</span>
                                <span class="net-relay">"relay"</span>
                            </>
                        }.into_any()
                    }
                }}
            </div>

            <ConfirmDialog
                visible=show_del_confirm
                title="Delete Channel"
                message=Signal::derive(move || {
                    pending_del_channel.get()
                        .map(|n| format!("Delete #{}?", n))
                        .unwrap_or_default()
                })
                confirm_text="Delete"
                danger=true
                on_confirm=Callback::new(move |_| {
                    if let Some(name) = pending_del_channel.get_untracked() {
                        let h = handle_del_confirm.clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            let _ = h.delete_channel(&name).await;
                        });
                    }
                    set_pending_del_channel.set(None);
                    set_show_del_confirm.set(false);
                })
                on_cancel=Callback::new(move |_| {
                    set_pending_del_channel.set(None);
                    set_show_del_confirm.set(false);
                })
            />
        </aside>
    }
}

/// Three-word lowercase fingerprint hint, format `word·word·word`, from
/// a peer id. Falls back to a truncated id when the id is too short.
fn short_fingerprint(peer_id: &str) -> String {
    if peer_id.is_empty() {
        return String::new();
    }
    const WORDS: &[&str] = &[
        "willow", "moss", "cedar", "bark", "lichen", "quiet", "ember", "amber", "fern", "thistle",
        "dusk", "pine", "birch", "stone", "river", "rook",
    ];
    let bytes = peer_id.as_bytes();
    let a = WORDS[(bytes.first().copied().unwrap_or(0) as usize) % WORDS.len()];
    let b = WORDS[(bytes.get(3).copied().unwrap_or(0) as usize) % WORDS.len()];
    let c = WORDS[(bytes.get(7).copied().unwrap_or(0) as usize) % WORDS.len()];
    format!("{a}·{b}·{c}")
}

/// Render a single channel row in the scroll region. Variant chosen by
/// the channel's group — text / voice / ephemeral; muted class defined
/// in components.css but not emitted yet (no state flag — see plan).
#[allow(clippy::too_many_arguments)]
fn render_channel_row(
    name: String,
    group: ChannelGroup,
    current_channel: ReadSignal<String>,
    unread: ReadSignal<HashMap<String, usize>>,
    can_manage_channels: impl Fn() -> bool + Send + Copy + 'static,
    on_channel_click: impl Fn(String) + Send + Clone + 'static,
    on_voice_join: impl Fn(String) + Send + Clone + 'static,
    set_pending_del_channel: WriteSignal<Option<String>>,
    set_show_del_confirm: WriteSignal<bool>,
) -> AnyView {
    let name_click = name.clone();
    let name_delete = name.clone();
    let name_title = name.clone();
    let name_render = name.clone();
    let is_voice = matches!(group, ChannelGroup::Voice);
    let is_ephemeral = matches!(group, ChannelGroup::Ephemeral);

    let on_text = on_channel_click.clone();
    let on_vc = on_voice_join.clone();

    let kind_icon_view = if is_voice {
        icons::icon_volume_1().into_any()
    } else if is_ephemeral {
        icons::icon_hourglass().into_any()
    } else {
        icons::icon_hash().into_any()
    };

    // Listener count — voice channels read the voice_participants_map
    // from AppState context and render a pulse chip when active > 0.
    let app_state = use_context::<crate::state::AppState>().unwrap();
    let listeners = {
        let name = name.clone();
        move || {
            if !is_voice {
                return 0usize;
            }
            app_state
                .voice
                .voice_participants_map
                .get()
                .get(&name)
                .map(|v| v.len())
                .unwrap_or(0)
        }
    };

    // Aria label: "<kind> channel <name>" plus suffix.
    let aria_kind = if is_voice {
        "voice"
    } else if is_ephemeral {
        "ephemeral"
    } else {
        "text"
    };
    let aria = format!("{aria_kind} channel {name}");

    // Class closure — must clone name each time the closure runs.
    let class_name = name.clone();
    let class_fn = move || {
        let mut cls = String::from("channel-item");
        if is_voice {
            cls.push_str(" channel-item--voice voice-channel");
        }
        if is_ephemeral {
            cls.push_str(" channel-item--ephemeral");
        }
        if matches!(group, ChannelGroup::Archives) {
            cls.push_str(" channel-item--archive");
        }
        let active = current_channel.get() == class_name;
        if active {
            cls.push_str(" channel-item--current active");
        }
        let cnt = unread.get().get(&class_name).copied().unwrap_or(0);
        if !active && cnt > 0 {
            cls.push_str(" channel-item--unread");
        }
        cls
    };

    // Trailing-slot closure — owns its own name clone.
    let trailing_name = name.clone();
    let trailing_fn = move || {
        if is_voice {
            let n = listeners();
            if n > 0 {
                Some(
                    view! {
                        <span class="listener-chip">
                            <span class="listener-pulse"></span>
                            <span class="listener-count">{format!("{n} listening")}</span>
                        </span>
                    }
                    .into_any(),
                )
            } else {
                None
            }
        } else if is_ephemeral {
            Some(view! { <span class="ephemeral-timer">"--h --m"</span> }.into_any())
        } else {
            let cnt = unread.get().get(&trailing_name).copied().unwrap_or(0);
            let active = current_channel.get() == trailing_name;
            if !active && cnt > 0 {
                let display = if cnt > 99 {
                    "99+".to_string()
                } else {
                    cnt.to_string()
                };
                Some(view! { <span class="unread-pill unread-badge">{display}</span> }.into_any())
            } else {
                None
            }
        }
    };

    view! {
        <div
            class=class_fn
            role="listitem"
            title=name_title
            aria-label=aria
            on:click=move |_| {
                if is_voice {
                    on_vc(name_click.clone());
                } else {
                    on_text(name_click.clone());
                }
            }
        >
            <span class="channel-row-bar" aria-hidden="true"></span>
            <span class="channel-row-icon">{kind_icon_view}</span>
            <span class="channel-row-name">{name_render}</span>
            <span class="channel-row-trailing">
                {trailing_fn}
                {
                    let name_for_del = name_delete.clone();
                    move || {
                        let name_for_del = name_for_del.clone();
                        can_manage_channels().then(|| view! {
                            <button
                                class="delete-btn"
                                title="Delete channel"
                                on:click=move |ev| {
                                    ev.stop_propagation();
                                    set_pending_del_channel.set(Some(name_for_del.clone()));
                                    set_show_del_confirm.set(true);
                                }
                            >
                                "x"
                            </button>
                        })
                    }
                }
            </span>
        </div>
    }
    .into_any()
}
