//! # Profile card content
//!
//! The 17-field profile card leaf. Used inside both the desktop
//! [`ProfilePopover`](super::profile_popover::ProfilePopover) and the
//! mobile [`ProfileSheet`](super::profile_sheet::ProfileSheet).
//!
//! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
//! §Field inventory.

use std::sync::Arc;

use leptos::prelude::*;
use willow_client::presence::PresenceState;
use willow_client::{NicknameStoreHandle, ProfileView};

use super::peer_color;
use super::peer_status_label::PeerStatusLabel;
use super::status_dot::{StatusDot, StatusDotBorder, StatusDotSize};
use crate::icons;
use crate::profile::{copy as pcopy, CrestBanner};
use crate::state::{AppState, AppWriteSignals, SettingsTab};

/// Peer vs. self variant flags.
///
/// Spec §Self view describes the variant flags that flip between the
/// two rendered shells.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProfileVariant {
    /// Peer view — every action row, nickname editor, block link.
    Peer,
    /// Self view — `edit profile` CTA, `this is you` caption, no
    /// nickname editor.
    Self_,
}

/// 17-field profile card content.
///
/// Renders every field from spec §Peer view in DOM order. Hidden
/// fields (pronouns/bio/tagline/pinned/elsewhere/since) are omitted
/// entirely on the peer card — the spec is explicit that the peer
/// card never shows empty-state rows for unset fields. The self card
/// shows a `no pinned fragment` caption when `pinned` is unset.
#[component]
pub fn ProfileCardContent(
    /// Merged profile view built by `profile_view_of`.
    #[prop(into)]
    view: Signal<Arc<ProfileView>>,
    /// Peer or self variant.
    #[prop(default = ProfileVariant::Peer)]
    variant: ProfileVariant,
    /// Fired when any close affordance triggers (Escape, close button,
    /// navigating action dispatch).
    #[prop(into)]
    on_close: Callback<()>,
) -> impl IntoView {
    let app_state = use_context::<AppState>().expect("AppState in context");
    let write = use_context::<AppWriteSignals>().expect("AppWriteSignals in context");
    let nickname_store = use_context::<NicknameStoreHandle>();

    // Presence lookup — prefer the per-peer map; default to Here.
    let pres_state = app_state.presence.per_peer;
    let view_for_pres = view;
    let presence = Signal::derive(move || {
        pres_state
            .get()
            .get(&view_for_pres.get().peer_id)
            .copied()
            .unwrap_or(PresenceState::Here)
    });

    // Nickname signal tied to the store's version counter so the UI
    // re-reads after `set` / `clear`.
    let store_clone = nickname_store.clone();
    let view_for_nick = view;
    let nickname = Signal::derive(move || {
        store_clone
            .as_ref()
            .and_then(|s| s.get(&view_for_nick.get().peer_id))
    });
    let (editing_nickname, set_editing_nickname) = signal(false);
    let (nickname_draft, set_nickname_draft) = signal(String::new());

    // Aria label on the root: `profile — <display name>`.
    let view_for_aria = view;
    let aria_label = move || format!("profile — {}", view_for_aria.get().display_name);

    // Badge click handoff: stuffing `compare_target` into AppState
    // opens the existing `<AddFriendDialog>`.
    let view_for_badge = view;
    let set_compare_target = write.trust.set_compare_target;
    let on_close_for_badge = on_close;
    let on_badge_click = move |_| {
        let pid = view_for_badge.get().peer_id.clone();
        set_compare_target.set(Some(pid));
        on_close_for_badge.run(());
    };

    // `edit profile` handler (self variant).
    let set_settings_tab = write.ui.set_settings_tab;
    let set_show_settings = write.ui.set_show_settings;
    let on_close_for_edit = on_close;
    let on_edit_profile = move |_| {
        set_settings_tab.set(SettingsTab::Profile);
        set_show_settings.set(true);
        on_close_for_edit.run(());
    };

    // `copy fingerprint` — does NOT close the card per spec §Desktop
    // popover dismissal.
    let view_for_copy = view;
    let on_copy_fingerprint = move |_| {
        let fp = view_for_copy.get().fingerprint_full.clone();
        crate::util::copy_to_clipboard(&fp);
    };

    // Close button (desktop — hidden on mobile via CSS).
    let on_close_for_x = on_close;
    let on_close_click = move |_| on_close_for_x.run(());

    // Nickname editor save/cancel handlers. `save_nickname_fn` is an
    // `Arc<dyn Fn>` so we can hand clones into two different event
    // handlers (blur + keydown) without moving-the-only-copy issues.
    let nick_store_save = nickname_store.clone();
    let view_for_save = view;
    let save_nickname_fn: Arc<dyn Fn() + Send + Sync> = {
        let store = nick_store_save.clone();
        Arc::new(move || {
            if let Some(s) = &store {
                let pid = view_for_save.get().peer_id.clone();
                let draft = nickname_draft.get();
                s.set(&pid, &draft);
            }
            set_editing_nickname.set(false);
        })
    };

    view! {
        <div
            class="profile-card"
            class:profile-card--self=move || variant == ProfileVariant::Self_
            role="dialog"
            aria-label=aria_label
            tabindex="-1"
        >
            <CrestBanner
                pattern=Signal::derive({ let v = view; move || v.get().crest_pattern })
                color=Signal::derive({ let v = view; move || v.get().crest_color.clone() })
                peer_id=Signal::derive({ let v = view; move || v.get().peer_id.clone() })
            />

            // Verification badge on banner (top-left). Click hands off
            // to the compare-fingerprints flow (spec §Badges + trust
            // surfacing). Copy + aria-label resolve from the pcopy
            // module so byte-exact tests pin the strings.
            <button
                class="profile-card__badge"
                type="button"
                on:click=on_badge_click
                aria-label=pcopy::UNVERIFIED_TOOLTIP
                title=pcopy::UNVERIFIED_TOOLTIP
            >
                {icons::icon_shield()}
                <span>"unverified"</span>
            </button>

            // Desktop close button (top-right). Hidden on mobile via CSS.
            <button
                class="profile-card__close"
                type="button"
                aria-label="close profile"
                on:click=on_close_click
            >
                {icons::icon_x()}
            </button>

            // Avatar + presence.
            <div
                class="profile-card__avatar"
                style=move || format!("background: {}", peer_color(&view.get().peer_id))
            >
                <span class="profile-card__avatar-initial">
                    {move || {
                        view.get()
                            .display_name
                            .chars()
                            .next()
                            .unwrap_or('?')
                            .to_uppercase()
                            .to_string()
                    }}
                </span>
                <StatusDot
                    state=presence
                    size=StatusDotSize::Profile
                    border=StatusDotBorder::Bg1
                    ambient=false
                />
            </div>

            // Display name.
            <div class="profile-card__name" title=move || view.get().display_name.clone()>
                {move || view.get().display_name.clone()}
            </div>

            // Pronouns pill (optional).
            {move || {
                view.get().pronouns.clone().map(|p| {
                    view! { <span class="profile-card__pronouns">{p}</span> }
                })
            }}

            // Handle + "you call them …" line.
            {move || {
                let v = view.get();
                let handle = v.handle.clone();
                let nick = nickname.get();
                view! {
                    <div class="profile-card__handle">
                        <span class="profile-card__handle-id">{handle}</span>
                        {match (variant, &nick) {
                            (ProfileVariant::Peer, Some(name)) => Some(view! {
                                <>
                                    <span class="profile-card__handle-sep">" · "</span>
                                    <span class="profile-card__handle-nick-label">
                                        {pcopy::KNOWN_AS_PREFIX}
                                    </span>
                                    <span class="profile-card__handle-nick">" "{name.clone()}</span>
                                </>
                            }),
                            _ => None,
                        }}
                    </div>
                }
            }}

            // Status pill.
            <div class="profile-card__status">
                <PeerStatusLabel state=presence show_dot=true/>
            </div>

            // Bio (optional).
            {move || {
                view.get().bio.clone().map(|b| {
                    view! { <div class="profile-card__bio">{b}</div> }
                })
            }}

            // Tagline (optional, mono-S with middle-dot prefix).
            {move || {
                view.get().tagline.clone().map(|t| {
                    view! { <div class="profile-card__tagline">"· "{t}</div> }
                })
            }}

            // Pinned fragment (optional).
            {move || {
                match (variant, view.get().pinned.clone()) {
                    (_, Some(p)) => {
                        let body = p.body;
                        let quote = matches!(p.kind, willow_state::PinnedKind::Quote);
                        let rendered_body = if quote {
                            format!("\u{201c}{body}\u{201d}")
                        } else {
                            body
                        };
                        Some(view! {
                            <div class="profile-card__pinned">
                                <span class="profile-card__pinned-label">
                                    {pcopy::PINNED_LABEL}
                                </span>
                                <blockquote class="profile-card__pinned-body">
                                    {rendered_body}
                                </blockquote>
                            </div>
                        }
                        .into_any())
                    }
                    (ProfileVariant::Self_, None) => Some(view! {
                        <div class="profile-card__pinned profile-card__pinned--empty">
                            {pcopy::EMPTY_PINNED}
                        </div>
                    }
                    .into_any()),
                    _ => None,
                }
            }}

            // Elsewhere chips (optional).
            {move || {
                let entries = view.get().elsewhere.clone();
                (!entries.is_empty()).then(|| {
                    view! {
                        <div class="profile-card__chips profile-card__chips--elsewhere">
                            <span class="profile-card__chips-label">
                                {pcopy::ELSEWHERE_LABEL}
                            </span>
                            <For
                                each=move || entries.clone()
                                key=|e| e.clone()
                                let:entry
                            >
                                <span class="profile-card__chip">{entry}</span>
                            </For>
                        </div>
                    }
                })
            }}

            // Meta: since + fingerprint.
            <div class="profile-card__meta">
                {move || {
                    view.get().since.clone().map(|s| {
                        view! {
                            <div class="profile-card__meta-row">
                                <span class="profile-card__meta-label">
                                    {pcopy::SINCE_LABEL}
                                </span>
                                <span class="profile-card__meta-value">{s}</span>
                            </div>
                        }
                    })
                }}
                <div class="profile-card__meta-row">
                    <span class="profile-card__meta-label">
                        {pcopy::FINGERPRINT_LABEL}
                    </span>
                    <span
                        class="profile-card__meta-fingerprint"
                        title=move || view.get().fingerprint_full.clone()
                    >
                        {move || view.get().fingerprint_short.clone()}
                    </span>
                </div>
            </div>

            // Primary action row.
            {move || match variant {
                ProfileVariant::Peer => view! {
                    <div class="profile-card__actions-primary">
                        <button class="profile-card__action" type="button"
                                aria-label=pcopy::MESSAGE>
                            {icons::icon_send()}
                            <span>{pcopy::MESSAGE}</span>
                        </button>
                        <button class="profile-card__action" type="button"
                                aria-label=pcopy::CALL>
                            {icons::icon_phone()}
                            <span>{pcopy::CALL}</span>
                        </button>
                        <button class="profile-card__action" type="button"
                                aria-label=pcopy::WHISPER>
                            {icons::icon_ear()}
                            <span>{pcopy::WHISPER}</span>
                        </button>
                        <button class="profile-card__action" type="button"
                                aria-label="more actions">
                            {icons::icon_more_horizontal()}
                        </button>
                    </div>
                }
                .into_any(),
                ProfileVariant::Self_ => view! {
                    <div class="profile-card__actions-primary profile-card__actions-primary--self">
                        <button
                            class="profile-card__action profile-card__action--primary"
                            type="button"
                            aria-label=pcopy::EDIT_PROFILE
                            on:click=on_edit_profile
                        >
                            {icons::icon_edit()}
                            <span>{pcopy::EDIT_PROFILE}</span>
                        </button>
                    </div>
                }
                .into_any(),
            }}

            // Secondary row.
            {move || match variant {
                ProfileVariant::Peer => {
                    let nick_label = if nickname.get().is_some() {
                        pcopy::CHANGE_NICKNAME
                    } else {
                        pcopy::SET_NICKNAME
                    };
                    let view_for_nick_click = view;
                    let set_editing_for_click = set_editing_nickname;
                    let set_draft_for_click = set_nickname_draft;
                    let nick_for_click = nickname;
                    let on_nick_click = move |_| {
                        set_draft_for_click.set(nick_for_click.get().unwrap_or_default());
                        set_editing_for_click.set(true);
                        let _ = &view_for_nick_click;
                    };
                    view! {
                        <div class="profile-card__actions-secondary">
                            <button class="profile-card__link" type="button"
                                    on:click=on_copy_fingerprint>
                                {pcopy::COPY_FINGERPRINT}
                            </button>
                            {move || (!editing_nickname.get()).then(|| view! {
                                <button
                                    class="profile-card__link"
                                    type="button"
                                    on:click=on_nick_click
                                >
                                    {nick_label}
                                </button>
                            })}
                            {
                                let save_for_key = save_nickname_fn.clone();
                                let save_for_blur = save_nickname_fn.clone();
                                move || editing_nickname.get().then(|| {
                                    let save_key = save_for_key.clone();
                                    let save_blur = save_for_blur.clone();
                                    view! {
                                        <input
                                            class="nickname-editor__input"
                                            aria-label=move || format!(
                                                "nickname for {}",
                                                view.get().display_name,
                                            )
                                            prop:value=move || nickname_draft.get()
                                            on:input=move |ev| set_nickname_draft
                                                .set(event_target_value(&ev))
                                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                                match ev.key().as_str() {
                                                    "Enter" => (save_key)(),
                                                    "Escape" => set_editing_nickname.set(false),
                                                    _ => {}
                                                }
                                            }
                                            on:blur=move |_| (save_blur)()
                                            maxlength="32"
                                        />
                                    }
                                })
                            }
                            <button class="profile-card__link profile-card__link--danger"
                                    type="button"
                                    aria-label=pcopy::BLOCK>
                                {pcopy::BLOCK}
                            </button>
                        </div>
                    }
                    .into_any()
                }
                ProfileVariant::Self_ => {
                    let caption = move || {
                        format!(
                            "{} · {}",
                            pcopy::SELF_CAPTION,
                            view.get().fingerprint_short,
                        )
                    };
                    view! {
                        <div class="profile-card__actions-secondary profile-card__actions-secondary--self">
                            <span class="profile-card__self-caption">{caption}</span>
                        </div>
                    }
                    .into_any()
                }
            }}
        </div>
    }
}

/// Deprecated compatibility shim for phase-1e callsites that used
/// `ProfileCardStub`. Thin wrapper over [`ProfileCardContent`] that
/// constructs a minimal [`ProfileView`] from the peer_id + display
/// name and renders it.
///
/// New callers should dispatch through `open_profile` instead and let
/// the controller render the full card.
#[deprecated(note = "Use ProfileCardContent via the profile event bus (open_profile) instead.")]
#[component]
pub fn ProfileCardStub(
    #[prop(into)] peer_id: Signal<String>,
    #[prop(into)] display_name: Signal<String>,
) -> impl IntoView {
    let pid = peer_id;
    let dn = display_name;
    let view = Signal::derive(move || {
        Arc::new(ProfileView {
            peer_id: pid.get(),
            handle: pid.get().chars().take(8).collect(),
            display_name: dn.get(),
            ..ProfileView::default()
        })
    });
    view! {
        <ProfileCardContent
            view=view
            variant=ProfileVariant::Peer
            on_close=Callback::new(|_| {})
        />
    }
}
