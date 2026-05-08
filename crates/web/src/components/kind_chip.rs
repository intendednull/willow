//! Kind chip — small mono pill rendered on ephemeral surface rows
//! (sidebar entries + archives entries) signalling non-permanence.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/ephemeral-channels.md`
//! §Sidebar treatment (Active).

use leptos::prelude::*;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum KindChipKind {
    Channel,
    Thread,
    Whisper,
}

impl KindChipKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Channel => "temp",
            Self::Thread => "thread",
            Self::Whisper => "whisper",
        }
    }

    pub fn aria_kind(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Thread => "thread",
            Self::Whisper => "whisper",
        }
    }
}

/// Render a non-permanent kind chip — `temp` / `thread` / `whisper`
/// — with an `aria-label` carrying the metaphor for screen readers.
#[component]
pub fn KindChip(kind: KindChipKind) -> impl IntoView {
    let aria = format!("non-permanent — {}", kind.aria_kind());
    view! {
        <span class="kind-chip" aria-label=aria>{kind.label()}</span>
    }
}
