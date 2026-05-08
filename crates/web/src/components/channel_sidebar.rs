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
    ConfirmDialog, ContextMenu, StatusDot, StatusDotBorder, StatusDotSize, TempChannelCreateForm,
    ToastStack, VoiceControls, TEMP_DEFAULT_DAYS,
};
use crate::icons;

/// Canonical channel groups — three labels in this render order when
/// non-empty. Unknown-prefix channels fall into `Commons`.
///
/// Per Phase 2d (`docs/specs/2026-04-19-ui-design/ephemeral-channels.md`),
/// ephemerals share the `Commons` group with permanent channels — the
/// kind chip on the row carries the "non-permanent" signal, not group
/// membership. The legacy `_ephemeral-` name-prefix heuristic was
/// dropped at the same time.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ChannelGroup {
    Commons,
    Voice,
    Archives,
}

impl ChannelGroup {
    /// Classify a channel by name + kind. Voice channels go to `Voice`;
    /// the legacy `_archive-` name prefix still routes to `Archives`;
    /// everything else (including ephemerals) lands in `Commons`.
    pub fn classify(name: &str, kind: &willow_state::ChannelKind) -> Self {
        if name.starts_with("_archive-") {
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
            Self::Archives => "archives",
        }
    }

    pub fn css_key(self) -> &'static str {
        match self {
            Self::Commons => "commons",
            Self::Voice => "voice",
            Self::Archives => "archives",
        }
    }

    /// Render order used by ChannelSidebar.
    pub const ORDER: [Self; 3] = [Self::Commons, Self::Voice, Self::Archives];
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
    let (pending_temp, set_pending_temp) = signal(false);
    let (temp_days, set_temp_days) = signal(TEMP_DEFAULT_DAYS);
    let (new_name, set_new_name) = signal(String::new());
    let name_input_ref: NodeRef<leptos::html::Input> = NodeRef::new();

    // Focus + select-all on the slot input when a kind is picked.
    // Fires once per None → Some(kind) transition so typing doesn't
    // keep retriggering focus.
    Effect::new(move |prev: Option<bool>| {
        let is_some = pending_kind.get().is_some() || pending_temp.get();
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
        set_pending_temp.set(false);
        set_temp_days.set(TEMP_DEFAULT_DAYS);
        set_new_name.set(String::new());
    };

    let handle_create = handle.clone();
    let on_create_submit = {
        let reset = reset_create;
        move || {
            let name = new_name.get_untracked();
            let name = name.trim().to_string();
            let kind = pending_kind.get_untracked();
            let is_temp = pending_temp.get_untracked();
            if !name.is_empty() {
                if is_temp {
                    let h = handle_create.clone();
                    let name_owned = name.clone();
                    let days = temp_days.get_untracked() as u64;
                    let threshold_ms = days.saturating_mul(24 * 3_600_000);
                    wasm_bindgen_futures::spawn_local(async move {
                        let _ = h
                            .create_ephemeral_channel(
                                &name_owned,
                                willow_state::EphemeralKind::Channel,
                                threshold_ms,
                            )
                            .await;
                    });
                    on_channel_created(());
                } else if let Some(kind) = kind {
                    let h = handle_create.clone();
                    let name_owned = name.clone();
                    // Capture toast stack on the outer reactive frame —
                    // `spawn_local` strips the owner so `use_context`
                    // inside the async block would return None.
                    let toasts = use_context::<ToastStack>();
                    wasm_bindgen_futures::spawn_local(async move {
                        match kind {
                            willow_state::ChannelKind::Voice => {
                                if let Err(e) = h.create_voice_channel(&name_owned).await {
                                    crate::handlers::warn_and_toast_with(
                                        "create voice channel",
                                        &e,
                                        toasts.as_ref(),
                                    );
                                }
                            }
                            _ => {
                                if let Err(e) = h.create_channel(&name_owned).await {
                                    crate::handlers::warn_and_toast_with(
                                        "create channel",
                                        &e,
                                        toasts.as_ref(),
                                    );
                                }
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
        set_pending_temp.set(false);
        set_new_name.set("new-tree".to_string());
        // Focus + select-all happens via the Effect above.
    };

    let pick_temp = move |_| {
        set_picker_open.set(false);
        set_pending_kind.set(None);
        set_pending_temp.set(true);
        set_temp_days.set(TEMP_DEFAULT_DAYS);
        set_new_name.set("new-tree".to_string());
    };

    let on_plant_click = {
        let reset = reset_create;
        move |_| {
            if pending_kind.get_untracked().is_some() || pending_temp.get_untracked() {
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
                                let pick_e = pick_temp;
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
                                        <button
                                            class="tree-kind-picker__item"
                                            role="menuitem"
                                            on:click=move |_| pick_e(())
                                        >
                                            <span class="tree-kind-picker__glyph">"~"</span>
                                            <span class="tree-kind-picker__label">"temp"</span>
                                            <span class="tree-kind-picker__hint">"auto-archives"</span>
                                        </button>
                                    </div>
                                }
                            })}
                            {
                                let on_kd_a = on_kd.clone();
                                let save_a = submit_save.clone();
                                move || pending_kind.get().map(|kind| {
                                let on_kd = on_kd_a.clone();
                                let save = save_a.clone();
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
                            })
                            }
                            {
                                let on_kd_b = on_kd.clone();
                                let save_b = submit_save.clone();
                                move || pending_temp.get().then(|| {
                                let on_kd = on_kd_b.clone();
                                let save = save_b.clone();
                                let on_days = Callback::new(move |d: u32| set_temp_days.set(d));
                                view! {
                                    <div class="tree-slot tree-slot--temp" data-kind="temp">
                                        <span class="tree-slot__glyph">
                                            <span class="tree-slot__hash">"~"</span>
                                        </span>
                                        <input
                                            type="text"
                                            class="tree-slot__input"
                                            node_ref=name_input_ref
                                            aria-label="Name temp channel"
                                            placeholder="tree name"
                                            prop:value=move || new_name.get()
                                            on:input=move |ev| set_new_name.set(event_target_value(&ev))
                                            on:keydown=on_kd
                                        />
                                        <button
                                            class="tree-slot__save"
                                            title="plant temp tree"
                                            aria-label="plant temp tree"
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
                                        <TempChannelCreateForm
                                            on_change=on_days
                                        />
                                    </div>
                                }
                            })
                            }
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

                        // Phase 2d: skip ephemeral channels that have
                        // crossed their idle threshold — they live in
                        // the archives pane, not the active sidebar.
                        let eph_meta = app_state.server.ephemeral_meta.get();
                        let frontier = js_sys::Date::now() as u64;
                        let archived: std::collections::HashSet<String> = eph_meta
                            .iter()
                            .filter_map(|(name, _, last, threshold)| {
                                let band = willow_state::derive_ephemeral_state(
                                    *last, *threshold, frontier,
                                );
                                if band == willow_state::EphemeralState::Archived {
                                    Some(name.clone())
                                } else {
                                    None
                                }
                            })
                            .collect();

                        let mut grouped: HashMap<ChannelGroup, Vec<String>> = HashMap::new();
                        for name in &ch_list {
                            if archived.contains(name) {
                                continue;
                            }
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
                        let toasts = use_context::<ToastStack>();
                        wasm_bindgen_futures::spawn_local(async move {
                            if let Err(e) = h.delete_channel(&name).await {
                                crate::handlers::warn_and_toast_with(
                                    "delete channel",
                                    &e,
                                    toasts.as_ref(),
                                );
                            }
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

    // Phase 2d: ephemeral status comes from the channel's
    // EphemeralConfig, no longer from the legacy `_ephemeral-` prefix
    // / sidebar-group heuristic. Pull the kind + threshold + last
    // activity from AppState so each row signals non-permanence with
    // the correct kind chip.
    let app_state_for_eph = use_context::<crate::state::AppState>().unwrap();
    let name_for_eph = name.clone();
    let ephemeral_kind = Signal::derive(move || {
        app_state_for_eph
            .server
            .ephemeral_meta
            .get()
            .into_iter()
            .find(|(n, _, _, _)| n == &name_for_eph)
            .map(|(_, k, _, _)| k)
    });
    let name_for_eph_meta = name.clone();
    let ephemeral_meta_for_row = Signal::derive(move || {
        app_state_for_eph
            .server
            .ephemeral_meta
            .get()
            .into_iter()
            .find(|(n, _, _, _)| n == &name_for_eph_meta)
            .map(|(_, _, last, threshold)| (last, threshold))
    });
    let is_ephemeral = move || ephemeral_kind.get().is_some();
    // Dormant when activity has elapsed past 25% of the threshold but
    // has not yet crossed it. Uses `js_sys::Date::now()` as the
    // frontier on WASM — close enough to the real HLC frontier for UI
    // styling purposes; archived rows are filtered out by the active
    // sidebar (Task 10) so we only need to differentiate active vs
    // dormant here.
    let is_dormant = move || {
        if let Some((last, threshold)) = ephemeral_meta_for_row.get() {
            let frontier = js_sys::Date::now() as u64;
            let band = willow_state::derive_ephemeral_state(last, threshold, frontier);
            band == willow_state::EphemeralState::Dormant
        } else {
            false
        }
    };

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

    // The kind icon needs to be reactive to the ephemeral signal —
    // wrap it in a `move ||` so subsequent ephemeral-state changes
    // re-render the row glyph.
    let kind_icon_view = move || -> AnyView {
        if is_voice {
            icons::icon_volume_1().into_any()
        } else if is_ephemeral() {
            icons::icon_hourglass().into_any()
        } else {
            icons::icon_hash().into_any()
        }
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
    } else if is_ephemeral() {
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
        if is_ephemeral() {
            cls.push_str(" channel-item--ephemeral");
        }
        if is_dormant() {
            cls.push_str(" channel-item--dormant");
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
        } else if is_ephemeral() {
            // Phase 2d: render the kind chip + (when dormant) a meta
            // line. Spec: §Sidebar treatment.
            let kind_for_chip = ephemeral_kind.get().map(|k| match k {
                willow_state::EphemeralKind::Channel => crate::components::KindChipKind::Channel,
                willow_state::EphemeralKind::Thread => crate::components::KindChipKind::Thread,
                willow_state::EphemeralKind::Whisper => crate::components::KindChipKind::Whisper,
            });
            Some(
                view! {
                    {kind_for_chip.map(|k| view! { <crate::components::KindChip kind=k/> })}
                }
                .into_any(),
            )
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

    // Phase 2d dormant meta — surfaces "last activity {N} {unit} ago"
    // via the row's `title` attribute (long-press preview on mobile,
    // tooltip on desktop) per spec §Sidebar treatment §Mobile compact
    // form.
    let title_for_row = move || {
        if is_dormant() {
            if let Some((last, _)) = ephemeral_meta_for_row.get() {
                let now = js_sys::Date::now() as u64;
                let elapsed = now.saturating_sub(last.unwrap_or(0));
                let phrase = crate::util::humanise_elapsed_ms(elapsed);
                return format!("{} — last activity {phrase}", name_title);
            }
        }
        name_title.clone()
    };

    view! {
        <>
        <div
            class=class_fn
            role="listitem"
            title=title_for_row
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
                            // Capture toast stack on the outer reactive frame —
                            // `spawn_local` strips the owner so `use_context`
                            // inside the async block would return None.
                            let toasts = use_context::<ToastStack>();
                            wasm_bindgen_futures::spawn_local(async move {
                                if let Err(e) = h.mutate_channel_mute(&channel, target).await {
                                    crate::handlers::warn_and_toast_with(
                                        "mute channel",
                                        &e,
                                        toasts.as_ref(),
                                    );
                                }
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
