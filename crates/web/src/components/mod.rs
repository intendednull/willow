/// Derive a unique, vibrant color from a peer ID string.
///
/// Uses a hash of the peer ID bytes to pick a hue on the color wheel,
/// producing visually distinct colors for different peers. The saturation
/// and lightness are tuned for readability on both dark and light themes.
pub fn peer_color(peer_id: &str) -> String {
    let hash = peer_id
        .bytes()
        .fold(2166136261u32, |h, b| h.wrapping_mul(16777619) ^ (b as u32));
    let hue = hash % 360;
    // Avoid dull mid-range; keep saturation punchy and lightness bright enough
    // for dark backgrounds while not washing out on light ones.
    let sat = 55 + (hash / 360) % 20; // 55-74%
    let lit = 65 + (hash / 7200) % 10; // 65-74%
    format!("hsl({hue}, {sat}%, {lit}%)")
}

mod add_friend;
mod add_server;
mod bottom_sheet;
mod call_page;
mod channel_sidebar;
mod chat;
mod command_palette;
mod confirm_dialog;
mod context_menu;
mod downgrade_banner;
mod file_share;
mod grove_drawer;
mod grove_rail;
mod holder_pill;
mod inline_queue_note;
mod input;
mod join_page;
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
mod toast;
mod trust_badge;
mod unread_badge;
mod voice;
mod welcome;
mod welcome_back_banner;

pub use add_friend::*;
pub use add_server::*;
pub use bottom_sheet::*;
pub use call_page::*;
pub use channel_sidebar::*;
pub use chat::*;
pub use command_palette::*;
pub use confirm_dialog::*;
pub use context_menu::*;
pub use downgrade_banner::*;
pub use file_share::*;
pub use grove_drawer::*;
pub use grove_rail::*;
pub use holder_pill::*;
pub use inline_queue_note::*;
pub use input::*;
pub use join_page::*;
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
pub use toast::*;
pub use trust_badge::*;
pub use unread_badge::*;
pub use voice::*;
pub use welcome::*;
pub use welcome_back_banner::*;
