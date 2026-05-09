//! Per-channel reaction-recency context.
//!
//! Phase 3c.2 (PR #635) shipped `<EmojiPicker>` callsites with an
//! empty recent shelf because threading
//! `WebClientHandle::recent_reactions(channel)` through every picker
//! mount via prop-drilling would have meant touching the composer +
//! message-row + their callers in app.rs and chat.rs. This module
//! gives the recency a single Leptos context provided once at the
//! app shell layer, consumed by every `<EmojiPicker>` mount.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/reactions-pins.md`
//! §Quick reactions — "Override [the default] with the five most
//! recent reactions used in *this channel*".
//!
//! ## Refresh strategy
//!
//! The provided context derives from a Leptos `Resource` keyed on the
//! current channel + a manual "react tick" signal. The Resource
//! re-fires (a) when the active channel changes and (b) whenever a
//! caller bumps the tick after a successful `react()`. Bumping is
//! optional — the channel-change refresh alone keeps the recency
//! correct across navigation; the tick path is what makes a
//! freshly-clicked emoji appear at the top of the picker without
//! waiting for a channel switch.

use leptos::prelude::*;

/// Context carrier for the per-channel recency signal.
///
/// `Copy` because the inner [`Signal`] is itself cheap to clone (it's
/// already an `Arc`-based handle) — putting it in a wrapper struct
/// just lets `use_context::<ReactionRecency>()` disambiguate against
/// any other ambient `Signal<Vec<String>>` providers.
#[derive(Clone, Copy)]
pub struct ReactionRecency(pub Signal<Vec<String>>);

/// Spec default reaction shelf, mirrored from
/// `willow_client::state_actors::REACTION_RECENCY_DEFAULT`. Used as
/// the fallback when no `ReactionRecency` context has been provided
/// (e.g. in unit-test mounts that don't construct a full app shell).
pub const REACTION_RECENCY_DEFAULT: &[&str] = &["👍", "❤️", "🍃", "💚", "👀"];

/// Read the recency from context, falling back to the spec default
/// when no context has been provided. Always returns a non-empty
/// `Signal<Vec<String>>` so the picker's "recent" row never collapses.
pub fn use_recent_reactions() -> Signal<Vec<String>> {
    use_context::<ReactionRecency>()
        .map(|r| r.0)
        .unwrap_or_else(|| {
            Signal::derive(|| {
                REACTION_RECENCY_DEFAULT
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect()
            })
        })
}
