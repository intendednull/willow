//! Reusable emoji picker popover.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/reactions-pins.md`
//! §Emoji picker.
//!
//! The same component is reused in three places per the spec:
//!
//! - The row's add-reaction chip and action-sheet entry (Phase 3c).
//! - The row's hover-toolbar `smile` "more reactions" button (Phase 3c).
//! - The composer's emoji IconBtn (Phase 3a placeholder; Phase 3c
//!   wires it in).
//!
//! All callers thread `recent` from
//! `willow_client::ClientHandle::recent_reactions(channel)` so the
//! "recent" shelf is per-channel.

pub mod categories;
pub mod picker;

pub use picker::EmojiPicker;
