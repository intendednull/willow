//! Composer module — the input surface beneath every message list.
//!
//! Implements `docs/specs/2026-04-19-ui-design/composer.md`. Owns the
//! compose textarea (desktop + mobile pill variants), reply preview bar,
//! edit bar, keybindings, mention autocomplete popover, offline tinting,
//! placeholder copy, and the typing indicator row above the composer.
//!
//! Submodules are populated incrementally across Phase 3a tasks; T4
//! introduces the skeleton + the pure `placeholder_for` helper, T5 lights
//! up the `<Composer>` shell, and later tasks wire up the meta row,
//! reply / edit bars, typing indicator, and mention autocomplete.

pub mod composer;
pub mod edit_bar;
pub mod mention_autocomplete;
pub mod meta_row;
pub mod placeholders;
pub mod reply_bar;
pub mod typing_indicator;

pub use composer::*;
pub use placeholders::placeholder_for;
