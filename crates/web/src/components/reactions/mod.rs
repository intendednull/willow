//! Reaction-strip surfaces extracted from `message.rs`.
//!
//! Phase 2a shipped the inline reactions strip rendered directly
//! inside `MessageView`. Phase 3c calls for spec geometry + a
//! reactor tooltip + a desktop-only add-reaction chip — too much to
//! keep inline. This module moves the strip surfaces into their own
//! components so the row stays readable and the strip itself is
//! easier to test.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/reactions-pins.md`
//! §Reactions.

pub mod add_chip;
pub mod strip;
pub mod tooltip;

pub use add_chip::AddReactionChip;
pub use strip::ReactionStrip;
pub use tooltip::reactor_tooltip;
