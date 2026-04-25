//! Channel sidebar — the second pane on the desktop shell.
//!
//! Replaces the legacy `Sidebar`. Top-to-bottom:
//!   1. Grove header (glyph tile + grove name, clickable → grove menu)
//!   2. Channel scroll region — four canonical groups
//!      (commons / voice / ephemeral / archives)
//!   3. Me strip (profile link: avatar + display name → profile page)
//!
//! Spec: docs/specs/2026-04-19-ui-design/layout-primitives.md §Channel sidebar

use std::collections::HashMap;

use leptos::prelude::*;

use crate::app::WebClientHandle;
use crate::components::{
    ConfirmDialog, ContextMenu, StatusDot, StatusDotBorder, StatusDotSize, VoiceControls,
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
    unread: ReadSignal<HashMap<String, willow_client::views::UnreadStats>>,
    server_name: ReadSignal<String>,
    on_channel_click: impl Fn(String) + Send + Clone + 'static,
    // Phase 2c: the me-strip now opens the profile card (self variant)
    // instead of Settings directly; the card's `edit profile` button
    // takes over that hand-off. The prop is retained so call sites in
    // `app.rs` and `mobile_shell.rs` compile unchanged.
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
    // Phase 2c: `on_settings_click` is superseded by the profile-card
    // `edit profile` button. Bind it to suppress the unused-variable
    // warning while keeping the prop in the public API.
    let _ = on_settings_click;

    let handle = use_context::<WebClientHandle>().unwrap();
    let app_state = use_context::<crate::state::AppState>().unwrap();

    // Admins (including the genesis owner) can create and delete channels.
    let peer_id = app_state.network.peer_id;
    let can_manage_channels = move || app_state.server.admin_ids.get().contains(&peer_id.get());

    // Channel-create flow: picker → slot. Nothing commits to state
    // until the slot's Save fires.
    //   - `picker_open`: type dropdown is visible, awaiting a pick.
    //   - `pending_kind`: user picked a kind; slot with name input is
    //     shown and auto-focused.
    let (picker_open, set_picker_open) = signal(false);
    let (pending_kind, set_pending_kind) = signal(Option::<willow_state::ChannelKind>::None);
    let (new_name, set_new_name) = signal(String::new());
    let name_input_ref: NodeRef<leptos::html::Input> = NodeRef::new();

    // Focus + select-all on the slot input when a kind is picked.
    // Fires once per None → Some(kind) transition so typing doesn't
    // keep retriggering focus.
    Effect::new(move |prev: Option<bool>| {
        let is_some = pending_kind.get().is_some();
        let was_some = prev.unwrap_or(false);
        if is_some && !was_some {
            let input_ref = name_input_ref;
            leptos::prelude::request_animation_frame(move || {
                if let Some(el) = input_ref.get_untracked() {
                    let _ = el.focus();
                    let len = el.value().len() as u32;
                    let _ = el.set_selection_range(0, len);
                }
            });
        }
        is_some
    });

    // Channel delete confirmation state.
    let (show_del_confirm, set_show_del_confirm) = signal(false);
    let (pending_del_channel, set_pending_del_channel) = signal(Option::<String>::None);
    let handle_del_confirm = handle.clone();

    let reset_create = move || {
        set_picker_open.set(false);
        set_pending_kind.set(None);
        set_new_name.set(String::new());
    };

    let handle_create = handle.clone();
    let on_create_submit = {
        let reset = reset_create;
        move || {
            let name = new_name.get_untracked();
            let name = name.trim().to_string();
            let kind = pending_kind.get_untracked();
            if let Some(kind) = kind {
                if !name.is_empty() {
                    let h = handle_create.clone();
                    let name_owned = name.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        match kind {
                            willow_state::ChannelKind::Voice => {
                                let _ = h.create_voice_channel(&name_owned).await;
                            }
                            _ => {
                                let _ = h.create_channel(&name_owned).await;
                            }
                        }
                    });
                    on_channel_created(());
                }
            }
            reset();
        }
    };

    let on_create_keydown = {
        let submit = on_create_submit.clone();
        let reset = reset_create;
        move |ev: web_sys::KeyboardEvent| {
            if ev.key() == "Enter" {
                ev.prevent_default();
                submit();
            } else if ev.key() == "Escape" {
                reset();
            }
        }
    };

    let pick_kind = move |kind: willow_state::ChannelKind| {
        set_picker_open.set(false);
        set_pending_kind.set(Some(kind));
        set_new_name.set("new-tree".to_string());
        // Focus + select-all happens via the Effect above.
    };

    let on_plant_click = {
        let reset = reset_create;
        move |_| {
            if pending_kind.get_untracked().is_some() {
                // Already filling a slot — cancel.
                reset();
            } else {
                set_picker_open.update(|v| *v = !*v);
            }
        }
    };

    // Collapsed groups (persisted locally — defaults to all expanded).
    let (collapsed, set_collapsed) = signal(std::collections::HashSet::<&'static str>::new());

    // Self-presence signal (drives the status dot on the me-strip).
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
            <button
                class="grove-header sidebar-header"
                title="grove menu"
                aria-label="grove menu"
                on:click=move |_| on_server_settings_click(())
            >
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
                <span
                    class="grove-header-name"
                    title=move || server_name.get()
                >
                    {move || server_name.get()}
                </span>
            </button>

            // ── Channel scroll region ──────────────────────────────
            <div class="channel-list scroll" role="list">
                {move || can_manage_channels().then(|| {
                    let pick_kind_a = pick_kind;
                    let pick_kind_b = pick_kind;
                    let cancel = reset_create;
                    let on_kd = on_create_keydown.clone();
                    let submit_save = on_create_submit.clone();
                    view! {
                        <div class="tree-create">
                            <button
                                class=move || {
                                    let active = picker_open.get() || pending_kind.get().is_some();
                                    if active {
                                        "channel-add-btn channel-add-btn--active"
                                    } else {
                                        "channel-add-btn"
                                    }
                                }
                                title="plant a new tree"
                                aria-expanded=move || {
                                    if picker_open.get() { "true" } else { "false" }
                                }
                                on:click=on_plant_click
                            >
                                {icons::icon_tree()}
                                <span class="channel-add-btn__label">"new"</span>
                            </button>
                            {move || picker_open.get().then(|| {
                                let pick_t = pick_kind_a;
                                let pick_v = pick_kind_b;
                                view! {
                                    <div class="tree-kind-picker" role="menu" aria-label="choose tree type">
                                        <button
                                            class="tree-kind-picker__item"
                                            role="menuitem"
                                            on:click=move |_| pick_t(willow_state::ChannelKind::Text)
                                        >
                                            <span class="tree-kind-picker__glyph">"#"</span>
                                            <span class="tree-kind-picker__label">"text"</span>
                                            <span class="tree-kind-picker__hint">"chat channel"</span>
                                        </button>
                                        <button
                                            class="tree-kind-picker__item"
                                            role="menuitem"
                                            on:click=move |_| pick_v(willow_state::ChannelKind::Voice)
                                        >
                                            <span class="tree-kind-picker__glyph">
                                                {icons::icon_volume_2()}
                                            </span>
                                            <span class="tree-kind-picker__label">"voice"</span>
                                            <span class="tree-kind-picker__hint">"call + audio"</span>
                                        </button>
                                    </div>
                                }
                            })}
                            {move || pending_kind.get().map(|kind| {
                                let on_kd = on_kd.clone();
                                let save = submit_save.clone();
                                let glyph_view = match kind {
                                    willow_state::ChannelKind::Voice => {
                                        icons::icon_volume_2().into_any()
                                    }
                                    _ => view! {
                                        <span class="tree-slot__hash">"#"</span>
                                    }.into_any(),
                                };
                                view! {
                                    <div class="tree-slot" data-kind=match kind {
                                        willow_state::ChannelKind::Voice => "voice",
                                        _ => "text",
                                    }>
                                        <span class="tree-slot__glyph">{glyph_view}</span>
                                        <input
                                            type="text"
                                            class="tree-slot__input"
                                            node_ref=name_input_ref
                                            aria-label="Rename channel"
                                            placeholder="tree name"
                                            prop:value=move || new_name.get()
                                            on:input=move |ev| set_new_name.set(event_target_value(&ev))
                                            on:keydown=on_kd
                                        />
                                        <button
                                            class="tree-slot__save"
                                            title="plant tree"
                                            aria-label="plant tree"
                                            on:mousedown=move |ev: web_sys::MouseEvent| {
                                                ev.prevent_default();
                                                save();
                                            }
                                        >
                                            {icons::icon_tree()}
                                        </button>
                                        <button
                                            class="tree-slot__cancel"
                                            title="cancel"
                                            aria-label="cancel"
                                            on:mousedown=move |ev: web_sys::MouseEvent| {
                                                ev.prevent_default();
                                                cancel();
                                            }
                                        >"×"</button>
                                    </div>
                                }
                            })}
                        </div>
                    }
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

            // ── Me strip — profile link ───────────────────────────
            // Spec §Event-bus API: the me-strip avatar opens the
            // profile card with the self variant. The old behaviour
            // (open settings directly) is now served by the card's
            // `edit profile` button.
            <button
                class="me-strip"
                title="open profile"
                aria-label="open profile"
                on:click={
                    let peer_id_sig = app_state.network.peer_id;
                    move |ev: web_sys::MouseEvent| {
                        use wasm_bindgen::JsCast as _;
                        let anchor = ev
                            .current_target()
                            .and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok());
                        crate::profile::open_profile(&peer_id_sig.get(), anchor);
                    }
                }
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
                <span class="me-display-name">
                    {move || {
                        let name = app_state.server.display_name.get();
                        if name.is_empty() { "you".to_string() } else { name }
                    }}
                </span>
            </button>

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

/// Render a single channel row in the scroll region. Variant chosen by
/// the channel's group — text / voice / ephemeral; muted class defined
/// in components.css but not emitted yet (no state flag — see plan).
#[allow(clippy::too_many_arguments)]
fn render_channel_row(
    name: String,
    group: ChannelGroup,
    current_channel: ReadSignal<String>,
    unread: ReadSignal<HashMap<String, willow_client::views::UnreadStats>>,
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
    let name_ctx = name.clone();
    let is_voice = matches!(group, ChannelGroup::Voice);
    let is_ephemeral = matches!(group, ChannelGroup::Ephemeral);

    // Per-row context menu for mute / unmute.
    let (show_menu, set_show_menu) = signal(false);
    let (menu_x, set_menu_x) = signal(0.0f64);
    let (menu_y, set_menu_y) = signal(0.0f64);
    let handle_mute = use_context::<WebClientHandle>().unwrap();
    // Resolve the current muted state reactively — reads
    // UnreadStats.muted derived from ServerState::mute_state.
    let name_for_muted = name.clone();
    let is_muted = Signal::derive(move || {
        unread
            .get()
            .get(&name_for_muted)
            .map(|s| s.muted)
            .unwrap_or(false)
    });

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
        let cnt = unread.get().get(&class_name).map(|s| s.count).unwrap_or(0);
        if !active && cnt > 0 {
            cls.push_str(" channel-item--unread");
        }
        cls
    };

    // Trailing-slot closure — owns its own name clone.
    let trailing_name = name.clone();
    let trailing_fn = move || {
        use crate::components::UnreadBadge;
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
            let trailing_name_inner = trailing_name.clone();
            let stats_signal: Signal<willow_client::views::UnreadStats> =
                Signal::derive(move || {
                    unread
                        .get()
                        .get(&trailing_name_inner)
                        .cloned()
                        .unwrap_or_default()
                });
            let trailing_name_render = trailing_name.clone();
            let active_signal =
                Signal::derive(move || current_channel.get() == trailing_name_render);
            // Hide the badge when the channel is active (no unread on
            // the surface you're reading) or when the count is zero
            // AND the surface isn't muted (muted surfaces with zero
            // still collapse — there's nothing to say).
            Some(
                view! {
                    {move || {
                        let s = stats_signal.get();
                        let active = active_signal.get();
                        if active || s.count == 0 {
                            None
                        } else {
                            Some(view! { <UnreadBadge stats=stats_signal/> })
                        }
                    }}
                }
                .into_any(),
            )
        }
    };

    view! {
        <>
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
            on:contextmenu=move |ev: web_sys::MouseEvent| {
                ev.prevent_default();
                set_menu_x.set(ev.client_x() as f64);
                set_menu_y.set(ev.client_y() as f64);
                set_show_menu.set(true);
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
        <ContextMenu
            visible=show_menu
            x=menu_x
            y=menu_y
            on_close=Callback::new(move |_| set_show_menu.set(false))
        >
            {
                let name_for_mute = name_ctx.clone();
                let handle = handle_mute.clone();
                view! {
                    <button
                        class="context-menu-item"
                        on:click=move |_| {
                            set_show_menu.set(false);
                            let channel = name_for_mute.clone();
                            let h = handle.clone();
                            let target = !is_muted.get_untracked();
                            wasm_bindgen_futures::spawn_local(async move {
                                let _ = h.mutate_channel_mute(&channel, target).await;
                            });
                        }
                    >
                        {move || if is_muted.get() { "unmute channel" } else { "mute channel" }}
                    </button>
                }
            }
        </ContextMenu>
        </>
    }
    .into_any()
}
