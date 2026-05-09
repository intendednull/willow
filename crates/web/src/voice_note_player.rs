//! Single-instance voice-note playback coordinator.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/files-inline.md` §Voice
//! note: "Playback persists across other audio (one voice note at a
//! time; starting another pauses the previous)."
//!
//! One [`VoiceNotePlayer`] is provided into Leptos context at the
//! app shell. Each [`crate::components::AttachmentVoiceNote`] writes
//! its own id to [`VoiceNotePlayer::active`] when it starts playing,
//! and watches the same signal to pause itself when somebody else
//! takes over. Cards key off the message-author hash + filename so
//! each row in the chat has a stable identity that survives
//! re-renders of the message list.

use leptos::prelude::*;

/// Shared, app-scoped voice-note player. Cloneable + Copy because
/// the only mutable field is a Leptos signal handle (Copy in 0.8).
#[derive(Clone, Copy)]
pub struct VoiceNotePlayer {
    /// Currently-playing voice-note id. `None` when nothing is
    /// playing. Cards write their own id on play and clear it on
    /// pause / end.
    pub active: RwSignal<Option<String>>,
}

impl VoiceNotePlayer {
    pub fn new() -> Self {
        Self {
            active: RwSignal::new(None),
        }
    }

    /// Mark `id` as the active player. Other cards subscribing to
    /// `active` will see the change and pause themselves.
    pub fn claim(&self, id: String) {
        self.active.set(Some(id));
    }

    /// Clear `active` if and only if the active id is `id`. Used by
    /// the owning card on `pause` / `ended` so a stale clear from a
    /// paused card can't pre-empt a different card that already
    /// claimed the slot.
    pub fn release_if_active(&self, id: &str) {
        self.active.update(|cur| {
            if cur.as_deref() == Some(id) {
                *cur = None;
            }
        });
    }
}

impl Default for VoiceNotePlayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Read the player from context, providing a fresh one if absent
/// (matches `use_upload_queue` for consistency with the other
/// shared-state contexts in this crate).
pub fn use_voice_note_player() -> VoiceNotePlayer {
    use_context::<VoiceNotePlayer>().unwrap_or_else(|| {
        let player = VoiceNotePlayer::new();
        provide_context(player);
        player
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_only_clears_when_we_are_active() {
        let p = VoiceNotePlayer::new();
        p.claim("alpha".to_string());
        // Different id can't preempt our active claim by accident.
        p.release_if_active("beta");
        assert_eq!(p.active.get_untracked().as_deref(), Some("alpha"));
        // Our own release does clear.
        p.release_if_active("alpha");
        assert_eq!(p.active.get_untracked(), None);
    }

    #[test]
    fn second_claim_overrides_first() {
        let p = VoiceNotePlayer::new();
        p.claim("alpha".to_string());
        p.claim("beta".to_string());
        assert_eq!(p.active.get_untracked().as_deref(), Some("beta"));
    }
}
