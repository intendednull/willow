/// Derive a unique, vibrant color from a peer's [`EndpointId`].
///
/// Uses a hash of the endpoint id bytes to pick a hue on the color wheel,
/// producing visually distinct colors for different peers. The saturation
/// and lightness are tuned for readability on both dark and light themes.
///
/// Taking `&EndpointId` rather than `&str` makes CSS injection impossible at
/// the type level: callers cannot pass attacker-controlled strings into the
/// inline `style=` attributes that consume this output.
pub fn peer_color(peer_id: &willow_identity::EndpointId) -> String {
    // Hash the canonical base32 string form so colors stay stable across
    // call sites that historically passed `endpoint_id.to_string()`.
    let id_str = peer_id.to_string();
    let hash = id_str
        .bytes()
        .fold(2166136261u32, |h, b| h.wrapping_mul(16777619) ^ (b as u32));
    let hue = hash % 360;
    // Avoid dull mid-range; keep saturation punchy and lightness bright enough
    // for dark backgrounds while not washing out on light ones.
    let sat = 55 + (hash / 360) % 20; // 55-74%
    let lit = 65 + (hash / 7200) % 10; // 65-74%
    format!("hsl({hue}, {sat}%, {lit}%)")
}

/// Fallback color used when a peer id string cannot be parsed into an
/// [`willow_identity::EndpointId`]. Neutral mid-grey hue so it does not
/// masquerade as a real peer's identity color.
const PEER_COLOR_FALLBACK: &str = "hsl(0, 0%, 70%)";

/// Convenience wrapper for call sites that only have a peer id as a string
/// (typically because surrounding state stores it as `String`). Parses the
/// string to an [`willow_identity::EndpointId`] before delegating to
/// [`peer_color`]; returns a neutral fallback if parsing fails.
///
/// This keeps the type-safe boundary in [`peer_color`] while limiting churn
/// at the legacy string-typed call sites.
pub fn peer_color_from_str(peer_id: &str) -> String {
    match peer_id.parse::<willow_identity::EndpointId>() {
        Ok(eid) => peer_color(&eid),
        Err(_) => PEER_COLOR_FALLBACK.to_string(),
    }
}

#[cfg(test)]
mod peer_color_tests {
    use super::*;

    /// Validates the output charset to lock in the invariant that
    /// `peer_color` cannot leak attacker-controlled characters into an
    /// inline CSS context, regardless of the input.
    #[test]
    fn peer_color_output_charset_is_safe() {
        let id = willow_identity::Identity::generate().endpoint_id();
        let out = peer_color(&id);
        for ch in out.chars() {
            assert!(
                ch.is_ascii_digit() || matches!(ch, 'h' | 's' | 'l' | '(' | ')' | ',' | ' ' | '%'),
                "peer_color output contains unexpected char {ch:?}: {out}"
            );
        }
        // Sanity: shape is `hsl(<hue>, <sat>%, <lit>%)`.
        assert!(out.starts_with("hsl("));
        assert!(out.ends_with(')'));
    }

    #[test]
    fn peer_color_is_deterministic_for_same_id() {
        let id = willow_identity::Identity::generate().endpoint_id();
        assert_eq!(peer_color(&id), peer_color(&id));
    }

    #[test]
    fn peer_color_from_str_matches_typed_for_valid_id() {
        let id = willow_identity::Identity::generate().endpoint_id();
        assert_eq!(peer_color(&id), peer_color_from_str(&id.to_string()));
    }

    #[test]
    fn peer_color_from_str_returns_fallback_for_garbage() {
        assert_eq!(
            peer_color_from_str("not a valid endpoint id"),
            PEER_COLOR_FALLBACK
        );
    }
}

mod add_friend;
mod add_server;
mod archives_view;
mod bottom_sheet;
mod call_page;
mod channel_sidebar;
mod chat;
mod command_palette;
pub mod composer;
mod confirm_dialog;
mod context_menu;
mod downgrade_banner;
mod file_share;
mod grove_drawer;
mod grove_rail;
mod holder_pill;
mod inline_queue_note;
mod join_page;
mod kind_chip;
pub mod lifecycle;
mod long_press;
mod main_pane_header;
mod member_list;
mod message;
pub mod message_row;
pub(crate) mod mobile_shell;
mod offline_strip;
pub(crate) mod palette_actions;
mod participant_tile;
mod peer_status_label;
mod pinned;
mod presence_menu;
mod profile_card;
mod profile_popover;
mod profile_sheet;
mod queue_pill;
mod read_only_banner;
mod reconnection_toast;
mod relay_signal_button;
mod right_rail;
mod roles;
mod sas;
pub mod search;
mod settings;
mod status_dot;
pub mod sync_queue_copy;
mod sync_queue_view;
mod tab_bar;
mod temp_channel_create;
mod toast;
mod trust_badge;
mod unread_badge;
mod voice;
mod welcome;
mod welcome_back_banner;

pub use add_friend::*;
pub use add_server::*;
pub use archives_view::ArchivesPane;
pub use bottom_sheet::*;
pub use call_page::*;
pub use channel_sidebar::*;
pub use chat::*;
pub use command_palette::*;
pub use composer::*;
pub use confirm_dialog::*;
pub use context_menu::*;
pub use downgrade_banner::*;
pub use file_share::*;
pub use grove_drawer::*;
pub use grove_rail::*;
pub use holder_pill::*;
pub use inline_queue_note::*;
pub use join_page::*;
pub use kind_chip::{KindChip, KindChipKind};
pub use long_press::*;
pub use main_pane_header::*;
pub use member_list::*;
pub use message::*;
pub use message_row::{
    day_bucket, parse_code_segments, CodeSegment, DayBucket, DaySeparator, FencedCodeBlock,
    InlineCodePill, JumpToLatestPill, MentionPill,
};
pub use mobile_shell::MobileShell;
#[allow(unused_imports)]
pub use mobile_shell::{MobilePush, MobileTab};
pub use offline_strip::*;
pub use participant_tile::*;
pub use peer_status_label::*;
pub use pinned::*;
pub use presence_menu::*;
pub use profile_card::*;
pub use profile_popover::*;
pub use profile_sheet::*;
pub use queue_pill::*;
pub use read_only_banner::ReadOnlyBanner;
pub use reconnection_toast::*;
pub use relay_signal_button::*;
pub use right_rail::*;
pub use roles::*;
pub use sas::sas_copy;
pub use sas::*;
pub use search::{RecentsList, ResultRow, ResultsList, ScopeChip, SearchInput, SearchSurface};
pub use settings::*;
pub use status_dot::*;
pub use sync_queue_view::*;
pub use tab_bar::*;
pub use temp_channel_create::{TempChannelCreateForm, TEMP_CAP_DAYS, TEMP_DEFAULT_DAYS};
pub use toast::*;
pub use trust_badge::*;
pub use unread_badge::*;
pub use voice::*;
pub use welcome::*;
pub use welcome_back_banner::*;
