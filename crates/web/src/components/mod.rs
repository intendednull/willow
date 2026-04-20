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

mod add_server;
mod bottom_sheet;
mod call_page;
mod channel_sidebar;
mod chat;
mod command_palette;
mod confirm_dialog;
mod context_menu;
mod file_share;
mod grove_drawer;
mod grove_rail;
mod input;
mod join_page;
mod main_pane_header;
mod member_list;
mod message;
pub(crate) mod mobile_shell;
pub(crate) mod palette_actions;
mod participant_tile;
mod pinned;
mod right_rail;
mod roles;
mod settings;
mod tab_bar;
mod voice;
mod welcome;

pub use add_server::*;
pub use bottom_sheet::*;
pub use call_page::*;
pub use channel_sidebar::*;
pub use chat::*;
pub use command_palette::*;
pub use confirm_dialog::*;
pub use context_menu::*;
pub use file_share::*;
pub use grove_drawer::*;
pub use grove_rail::*;
pub use input::*;
pub use join_page::*;
pub use main_pane_header::*;
pub use member_list::*;
pub use message::*;
pub use mobile_shell::MobileShell;
#[allow(unused_imports)]
pub use mobile_shell::{MobilePush, MobileTab};
pub use participant_tile::*;
pub use pinned::*;
pub use right_rail::*;
pub use roles::*;
pub use settings::*;
pub use tab_bar::*;
pub use voice::*;
pub use welcome::*;
