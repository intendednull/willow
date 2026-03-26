//! # SVG Icon Module
//!
//! Inline Lucide-style SVG icons rendered via `<span>` elements with
//! `inner_html`. Each icon uses `currentColor` for stroke so it inherits
//! the surrounding text color, and `width="1em" height="1em"` so it scales
//! with font-size.

use leptos::prelude::*;

/// Shared SVG attributes for all icons.
const SVG_ATTRS: &str = r#"xmlns="http://www.w3.org/2000/svg" width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round""#;

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
        &format!(
            r#"<svg {SVG_ATTRS}><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></svg>"#
        ),
        "icon-arrow-left",
    )
}

/// Right arrow icon.
pub fn icon_arrow_right() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg>"#
        ),
        "icon-arrow-right",
    )
}

/// X / close icon.
pub fn icon_x() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>"#
        ),
        "icon-x",
    )
}

/// Plus icon (for add actions).
pub fn icon_plus() -> impl IntoView {
    icon(
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M5 12h14"/><path d="M12 5v14"/></svg>"#
        ),
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
        &format!(
            r#"<svg {SVG_ATTRS}><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/></svg>"#
        ),
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
