//! # SVG Icon Module
//!
//! Inline Lucide-style SVG icons rendered via `<span>` elements with
//! `inner_html`. Each icon uses `currentColor` for stroke so it inherits
//! the surrounding text color, and `width="1em" height="1em"` so it scales
//! with font-size.

use leptos::prelude::*;

/// Shared SVG attributes for all icons.
const SVG_ATTRS: &str = r#"xmlns="http://www.w3.org/2000/svg" width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round""#;

/// Shared SVG attributes for Phase-1 shell icons — stroke 1.5, viewBox 24.
const SVG_ATTRS_THIN: &str = r#"xmlns="http://www.w3.org/2000/svg" width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round""#;

/// Render an inline SVG icon wrapped in a `<span class="icon {class}">`.
/// Size is controlled by the parent's font-size.
fn icon(svg: &str, class: &str) -> impl IntoView {
    view! {
        <span class=format!("icon {class}") inner_html=svg.to_string()></span>
    }
}

/// Hamburger menu icon (three horizontal lines).
pub fn icon_menu() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><line x1="4" x2="20" y1="12" y2="12"/><line x1="4" x2="20" y1="6" y2="6"/><line x1="4" x2="20" y1="18" y2="18"/></svg>"#
        ),
        "icon-menu",
    )
}

/// Hash / number sign icon (channel indicator).
pub fn icon_hash() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><line x1="4" x2="20" y1="9" y2="9"/><line x1="4" x2="20" y1="15" y2="15"/><line x1="10" x2="8" y1="3" y2="21"/><line x1="16" x2="14" y1="3" y2="21"/></svg>"#
        ),
        "icon-hash",
    )
}

/// Speaker / volume icon (voice channel indicator).
pub fn icon_volume_2() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/></svg>"#
        ),
        "icon-volume",
    )
}

/// Settings cog icon.
pub fn icon_settings() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/></svg>"#
        ),
        "icon-settings",
    )
}

/// Pin icon (for pinned messages).
pub fn icon_pin() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><line x1="12" x2="12" y1="17" y2="22"/><path d="M5 17h14v-1.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V6h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1v4.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24Z"/></svg>"#
        ),
        "icon-pin",
    )
}

/// Users / group icon (for member count).
pub fn icon_users() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>"#
        ),
        "icon-users",
    )
}

/// Microphone icon (unmuted).
pub fn icon_mic() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" x2="12" y1="19" y2="22"/></svg>"#
        ),
        "icon-mic",
    )
}

/// Microphone off icon (muted).
pub fn icon_mic_off() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><line x1="2" x2="22" y1="2" y2="22"/><path d="M18.89 13.23A7.12 7.12 0 0 0 19 12v-2"/><path d="M5 10v2a7 7 0 0 0 12 5.29"/><path d="M15 9.34V5a3 3 0 0 0-5.68-1.33"/><path d="M9 9v3a3 3 0 0 0 5.12 2.12"/><line x1="12" x2="12" y1="19" y2="22"/></svg>"#
        ),
        "icon-mic-off",
    )
}

/// Headphones icon (audio on).
pub fn icon_headphones() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3"/></svg>"#
        ),
        "icon-headphones",
    )
}

/// Headphones off icon (deafened). Uses a diagonal strike-through over the
/// headphones shape.
pub fn icon_headphones_off() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M21 14h-1a2 2 0 0 0-2 2v3a2 2 0 0 0 2 2h1a2 2 0 0 0 2-2v-7a9 9 0 0 0-18 0v7a2 2 0 0 0 2 2h1a2 2 0 0 0 2-2v-3a2 2 0 0 0-2-2H3"/><line x1="2" x2="22" y1="2" y2="22"/></svg>"#
        ),
        "icon-headphones-off",
    )
}

/// Phone off / disconnect icon.
pub fn icon_phone_off() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M10.68 13.31a16 16 0 0 0 3.41 2.6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7 2 2 0 0 1 1.72 2v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.42 19.42 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.63A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91"/><line x1="22" x2="2" y1="2" y2="22"/></svg>"#
        ),
        "icon-phone-off",
    )
}

/// Paperclip / attachment icon.
pub fn icon_paperclip() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"/></svg>"#
        ),
        "icon-paperclip",
    )
}

/// File / document icon.
pub fn icon_file() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/></svg>"#
        ),
        "icon-file",
    )
}

/// Download arrow icon.
pub fn icon_download() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" x2="12" y1="15" y2="3"/></svg>"#
        ),
        "icon-download",
    )
}

/// Left arrow icon.
pub fn icon_arrow_left() -> impl IntoView {
    icon(
        &format!(r#"<svg {SVG_ATTRS}><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></svg>"#),
        "icon-arrow-left",
    )
}

/// Right arrow icon.
pub fn icon_arrow_right() -> impl IntoView {
    icon(
        &format!(r#"<svg {SVG_ATTRS}><path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg>"#),
        "icon-arrow-right",
    )
}

/// X / close icon.
pub fn icon_x() -> impl IntoView {
    icon(
        &format!(r#"<svg {SVG_ATTRS}><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>"#),
        "icon-x",
    )
}

/// Plus icon (for add actions).
pub fn icon_plus() -> impl IntoView {
    icon(
        &format!(r#"<svg {SVG_ATTRS}><path d="M5 12h14"/><path d="M12 5v14"/></svg>"#),
        "icon-plus",
    )
}

/// Horizontal three-dot / ellipsis icon (more actions).
pub fn icon_more_horizontal() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/></svg>"#
        ),
        "icon-more",
    )
}

/// Sun icon (light theme indicator).
pub fn icon_sun() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/></svg>"#
        ),
        "icon-sun",
    )
}

/// Moon / crescent icon (dark theme indicator).
pub fn icon_moon() -> impl IntoView {
    icon(
        &format!(r#"<svg {SVG_ATTRS}><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/></svg>"#),
        "icon-moon",
    )
}

/// Send / paper-plane icon.
pub fn icon_send() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="m22 2-7 20-4-9-9-4Z"/><path d="M22 2 11 13"/></svg>"#
        ),
        "icon-send",
    )
}

/// Trash / delete icon.
pub fn icon_trash() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/></svg>"#
        ),
        "icon-trash",
    )
}

/// Edit / pencil icon.
pub fn icon_edit() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/><path d="m15 5 4 4"/></svg>"#
        ),
        "icon-edit",
    )
}

/// Reply / corner-up-left icon.
pub fn icon_reply() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><polyline points="9 14 4 9 9 4"/><path d="M20 20v-7a4 4 0 0 0-4-4H4"/></svg>"#
        ),
        "icon-reply",
    )
}

/// Smiley face icon (for reaction picker).
pub fn icon_smile() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><circle cx="12" cy="12" r="10"/><path d="M8 14s1.5 2 4 2 4-2 4-2"/><line x1="9" x2="9.01" y1="9" y2="9"/><line x1="15" x2="15.01" y1="9" y2="9"/></svg>"#
        ),
        "icon-smile",
    )
}

/// Search / magnifying glass icon.
pub fn icon_search() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>"#
        ),
        "icon-search",
    )
}

/// Copy / clipboard icon.
pub fn icon_copy() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>"#
        ),
        "icon-copy",
    )
}

/// Check / confirmation icon.
pub fn icon_check() -> impl IntoView {
    icon(
        &format!(r#"<svg {SVG_ATTRS}><polyline points="20 6 9 17 4 12"/></svg>"#),
        "icon-check",
    )
}

/// Shield icon — used by the trust-verification badges and downgrade
/// banner. Spec reference: `docs/specs/2026-04-19-ui-design/trust-verification.md`.
pub fn icon_shield() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>"#
        ),
        "icon-shield",
    )
}

/// Sprout icon — two-leaf seedling used as the "new peer" trust sigil.
/// Matches the grove/willow theme: a peer that's just taken root.
pub fn icon_sprout() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M12 22V11"/><path d="M12 13c-2.5-2.5-6-1-7.5-4 2.5-.5 5.5 0 7.5 4"/><path d="M12 11c2.5-2.5 5.5-1 7-3.5-2 0-5 0-7 3.5"/></svg>"#
        ),
        "icon-sprout",
    )
}

/// Key icon — used by the holder pill + related crypto visibility
/// surfaces.
pub fn icon_key() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><circle cx="7.5" cy="15.5" r="5.5"/><path d="M21 2l-9.6 9.6"/><path d="M15.5 7.5l3 3L22 7l-3-3"/></svg>"#
        ),
        "icon-key",
    )
}

/// Eye / reveal icon.
pub fn icon_eye() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/></svg>"#
        ),
        "icon-eye",
    )
}

/// Monitor / screen icon (rectangle with stand).
pub fn icon_monitor() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><rect width="20" height="14" x="2" y="3" rx="2"/><line x1="8" x2="16" y1="21" y2="21"/><line x1="12" x2="12" y1="17" y2="21"/></svg>"#
        ),
        "icon-monitor",
    )
}

/// Video camera icon.
pub fn icon_video() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="m16 13 5.223 3.482a.5.5 0 0 0 .777-.416V7.87a.5.5 0 0 0-.752-.432L16 10.5"/><rect width="14" height="12" x="2" y="6" rx="2"/></svg>"#
        ),
        "icon-video",
    )
}

/// Video camera off icon (with diagonal slash).
pub fn icon_video_off() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M10.66 6H14a2 2 0 0 1 2 2v2.5l5.248-3.062A.5.5 0 0 1 22 7.87v8.196"/><path d="M16 16a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h2"/><line x1="2" x2="22" y1="2" y2="22"/></svg>"#
        ),
        "icon-video-off",
    )
}

/// 2x2 grid icon.
pub fn icon_grid() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><rect width="7" height="7" x="3" y="3" rx="1"/><rect width="7" height="7" x="14" y="3" rx="1"/><rect width="7" height="7" x="14" y="14" rx="1"/><rect width="7" height="7" x="3" y="14" rx="1"/></svg>"#
        ),
        "icon-grid",
    )
}

/// Maximize / expand icon (corner arrows).
pub fn icon_maximize() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><polyline points="15 3 21 3 21 9"/><polyline points="9 21 3 21 3 15"/><line x1="21" x2="14" y1="3" y2="10"/><line x1="3" x2="10" y1="21" y2="14"/></svg>"#
        ),
        "icon-maximize",
    )
}

/// Server/infrastructure icon (stacked horizontal bars — like a server rack).
pub fn icon_server() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><rect width="20" height="8" x="2" y="2" rx="2" ry="2"/><rect width="20" height="8" x="2" y="14" rx="2" ry="2"/><line x1="6" x2="6.01" y1="6" y2="6"/><line x1="6" x2="6.01" y1="18" y2="18"/></svg>"#
        ),
        "icon-server",
    )
}

/// Refresh/sync icon (circular arrows).
pub fn icon_refresh() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M21 2v6h-6"/><path d="M3 12a9 9 0 0 1 15-6.7L21 8"/><path d="M3 22v-6h6"/><path d="M21 12a9 9 0 0 1-15 6.7L3 16"/></svg>"#
        ),
        "icon-refresh",
    )
}

/// Database/storage icon (cylinder).
pub fn icon_database() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5V19A9 3 0 0 0 21 19V5"/><path d="M3 12A9 3 0 0 0 21 12"/></svg>"#
        ),
        "icon-database",
    )
}

/// Activity/pulse icon (heartbeat line).
pub fn icon_activity() -> impl IntoView {
    icon(
        &format!(r#"<svg {SVG_ATTRS}><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>"#),
        "icon-activity",
    )
}

// ── Phase-1 shell icons (stroke 1.5) ─────────────────────────────────

/// Inbox / letters icon (grove-rail letters tile).
pub fn icon_inbox() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS_THIN}><polyline points="22 12 16 12 14 15 10 15 8 12 2 12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/></svg>"#
        ),
        "icon-inbox",
    )
}

/// Compass icon (grove-rail discover tile).
pub fn icon_compass() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS_THIN}><circle cx="12" cy="12" r="10"/><polygon points="16.24 7.76 14.12 14.12 7.76 16.24 9.88 9.88 16.24 7.76"/></svg>"#
        ),
        "icon-compass",
    )
}

/// Phone icon (main-pane header join-call action).
pub fn icon_phone() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS_THIN}><path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7A2 2 0 0 1 22 16.92z"/></svg>"#
        ),
        "icon-phone",
    )
}

/// User icon (profile / you tab).
pub fn icon_user() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS_THIN}><path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/></svg>"#
        ),
        "icon-user",
    )
}

/// Chevron-down icon (collapsible group / grove-menu trigger).
pub fn icon_chevron_down() -> impl IntoView {
    icon(
        &format!(r#"<svg {SVG_ATTRS_THIN}><polyline points="6 9 12 15 18 9"/></svg>"#),
        "icon-chevron-down",
    )
}

/// Chevron-right icon (collapsed group / drill-in hint).
pub fn icon_chevron_right() -> impl IntoView {
    icon(
        &format!(r#"<svg {SVG_ATTRS_THIN}><polyline points="9 18 15 12 9 6"/></svg>"#),
        "icon-chevron-right",
    )
}

/// Thread / message-square icon (thread pane toggle).
pub fn icon_thread() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS_THIN}><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>"#
        ),
        "icon-thread",
    )
}

/// Lock icon (e2e-encryption status row).
pub fn icon_lock() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS_THIN}><rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>"#
        ),
        "icon-lock",
    )
}

/// Hourglass icon (ephemeral channel + timer indicator).
pub fn icon_hourglass() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS_THIN}><path d="M5 22h14"/><path d="M5 2h14"/><path d="M17 22v-4.172a2 2 0 0 0-.586-1.414L12 12l-4.414 4.414A2 2 0 0 0 7 17.828V22"/><path d="M7 2v4.172a2 2 0 0 0 .586 1.414L12 12l4.414-4.414A2 2 0 0 0 17 6.172V2"/></svg>"#
        ),
        "icon-hourglass",
    )
}

/// Volume-1 icon (voice channel row — quieter than icon_volume_2).
pub fn icon_volume_1() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS_THIN}><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/></svg>"#
        ),
        "icon-volume-1",
    )
}

/// Ear glyph (11×11 stroke 1.5) — presence label prefix for `whispering`.
///
/// See `docs/specs/2026-04-19-ui-design/presence.md` §PeerStatusLabel.
pub fn icon_ear() -> impl IntoView {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M6 8.5a6.5 6.5 0 1 1 13 0c0 6-6 4-6 8.5a3 3 0 1 1-6 0"/><path d="M9 12a3 3 0 1 1 5-2.2"/></svg>"#;
    view! {
        <span class="icon icon-ear" inner_html=svg.to_string()></span>
    }
}

/// Small hourglass glyph (11×11 stroke 1.5) — presence label prefix for
/// `queued · N`. Uses its own rendering to differ from the full-size
/// icon_hourglass used in ephemeral channel chrome.
pub fn icon_hourglass_sm() -> impl IntoView {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M5 22h14"/><path d="M5 2h14"/><path d="M17 22v-4.172a2 2 0 0 0-.586-1.414L12 12l-4.414 4.414A2 2 0 0 0 7 17.828V22"/><path d="M7 2v4.172a2 2 0 0 0 .586 1.414L12 12l4.414-4.414A2 2 0 0 0 17 6.172V2"/></svg>"#;
    view! {
        <span class="icon icon-hourglass-sm" inner_html=svg.to_string()></span>
    }
}

/// Small tree glyph (24 viewBox, stroke 1.5) — sits inside buttons and
/// pills at 1em. Crown + trunk silhouette meant to pair with the moss
/// palette.
pub fn icon_tree() -> impl IntoView {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3 6.5 10h3L5 16h4l-2 3h10l-2-3h4l-4.5-6h3z"/><path d="M12 19v2"/></svg>"#;
    view! {
        <span class="icon icon-tree" inner_html=svg.to_string()></span>
    }
}

/// Single leaf glyph (24 viewBox, stroke 1.5) — rendered at 48 × 48 for
/// the never-had-messages empty state in the message list.
///
/// TODO(illustration): replace with the final designer leaf SVG when it
/// ships. The path below is a placeholder shaped like a closed-tip
/// leaf with a central vein — spec calls for "open-leaf SVG", but the
/// real asset hasn't landed yet.
pub fn icon_leaf() -> impl IntoView {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M11 3C7 3 4 6 4 10c0 6 5 11 11 11 0-4-3-7-7-7"/><path d="M4 10c4 0 8 4 8 8"/></svg>"#;
    view! {
        <span class="icon icon-leaf" inner_html=svg.to_string()></span>
    }
}

/// Arrow-up-right glyph — 12 × 12, stroke 1.5 — used by the search
/// results surface's right-column affordance (spec §Row anatomy:
/// "signalling open-in-place"). Rendered at 1em so the caller can
/// size via font-size.
pub fn icon_arrow_up_right() -> impl IntoView {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M7 17L17 7"/><path d="M7 7h10v10"/></svg>"#;
    view! {
        <span class="icon icon-arrow-up-right" inner_html=svg.to_string()></span>
    }
}

/// Signal / radio-waves glyph (stroke 1.5). Used by the sync-queue
/// surfaces (offline strip suffix + relay-signal button + per-row
/// relay-only marker). Three concentric arcs + a centre dot.
pub fn icon_signal() -> impl IntoView {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 12.5a12 12 0 0 1 17 0"/><path d="M6.5 15.5a8 8 0 0 1 11 0"/><path d="M9.5 18.5a4 4 0 0 1 5 0"/><circle cx="12" cy="21" r="0.8" fill="currentColor" stroke="none"/></svg>"#;
    view! {
        <span class="icon icon-signal" inner_html=svg.to_string()></span>
    }
}

/// Check icon (small). Used by the sync-queue footer + recent-arrivals
/// pill + delivery-flash badge.
pub fn icon_check_small() -> impl IntoView {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>"#;
    view! {
        <span class="icon icon-check-small" inner_html=svg.to_string()></span>
    }
}

/// Willow brand mark — three drooping fronds with leaf tips.
/// Rendered with its own viewBox (48) and stroke width (1.5) to match the
/// foundation iconography rules for brand surfaces.
pub fn icon_willow_mark() -> impl IntoView {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1em" height="1em" viewBox="0 0 48 48" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M24 6c-5 12-8 22-10 36"/><path d="M24 6c0 14 0 24 0 36"/><path d="M24 6c5 12 8 22 10 36"/><ellipse cx="14" cy="42" rx="3" ry="1.4" transform="rotate(-24 14 42)" fill="currentColor" stroke="none"/><ellipse cx="24" cy="44" rx="3" ry="1.4" fill="currentColor" stroke="none"/><ellipse cx="34" cy="42" rx="3" ry="1.4" transform="rotate(24 34 42)" fill="currentColor" stroke="none"/><circle cx="24" cy="6" r="1.4" fill="currentColor" stroke="none"/></svg>"#;
    view! {
        <span class="icon icon-willow-mark" inner_html=svg.to_string()></span>
    }
}
