//! Inline attachment renderers for messages with `Content::File` payloads.
//!
//! The message row delegates to [`pick`] on a parsed [`Content::File`] to
//! decide which surface to render: an inline `<AttachmentImage>`, an
//! `<AttachmentFileCard>`, or an `<AttachmentVoiceNote>`. The decision
//! table follows `docs/specs/2026-04-19-ui-design/files-inline.md`
//! §Inline rendering rules verbatim.
//!
//! Each surface lives in its own submodule and is only mounted by the
//! message-row branch that this module's decision table picks. The
//! components themselves are pure-presentational: they read fields off
//! `Content::File` and don't carry mutable state. Single-instance
//! voice-note playback (starting one pauses the previous) is
//! coordinated separately by [`crate::voice_note_player::VoiceNotePlayer`].

pub mod file_card;
pub mod image;
pub mod voice_note;

pub use file_card::AttachmentFileCard;
pub use image::AttachmentImage;
pub use voice_note::AttachmentVoiceNote;

/// Which surface the message row should render for a given attachment.
///
/// The variants map 1-to-1 to the components exported from this module
/// — [`AttachmentImage`], [`AttachmentFileCard`], [`AttachmentVoiceNote`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    /// Inline image rendering (`<img>` wrapped in an anchor).
    Image,
    /// Generic file card with download button.
    FileCard,
    /// Voice-note playback card.
    VoiceNote,
}

/// Threshold above which images degrade to a file card so we don't
/// silently spend mobile bandwidth on a forced inline render. Spec
/// §Inline rendering rules: "Image above 4 MB degrades to a file card
/// instead of inline".
pub const IMAGE_INLINE_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Decide which surface to render for an attachment.
///
/// The argument-list shape mirrors the fields on `willow_messaging::Content::File`
/// rather than taking the enum directly so callers can extract from any
/// projection (envelope, view-model, sealed payload after decryption)
/// without reaching for the wire type. Voice-note detection is by
/// MIME prefix `audio/`. Image detection is by MIME prefix `image/`,
/// degraded to a file card above [`IMAGE_INLINE_MAX_BYTES`] or when
/// the MIME isn't recognised (the spec phrases the latter as "unknown
/// mime → file card").
///
/// Spec: `docs/specs/2026-04-19-ui-design/files-inline.md`
/// §Inline rendering rules.
pub fn pick(mime_type: &str, size_bytes: u64) -> AttachmentKind {
    let mime_lower = mime_type.to_ascii_lowercase();
    if mime_lower.starts_with("audio/") {
        return AttachmentKind::VoiceNote;
    }
    if mime_lower.starts_with("image/") {
        if size_bytes > IMAGE_INLINE_MAX_BYTES {
            return AttachmentKind::FileCard;
        }
        return AttachmentKind::Image;
    }
    AttachmentKind::FileCard
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_under_threshold_is_inline() {
        // 1 MB JPEG: well under the 4 MB image-inline cap.
        assert_eq!(
            pick("image/jpeg", 1 * 1024 * 1024),
            AttachmentKind::Image,
            "small image should render inline"
        );
    }

    #[test]
    fn image_over_threshold_degrades_to_card() {
        // 5 MB PNG: above the 4 MB cap, must degrade.
        assert_eq!(
            pick("image/png", 5 * 1024 * 1024),
            AttachmentKind::FileCard,
            "image > 4 MB must degrade to file card to avoid \
             silently spending mobile bandwidth"
        );
    }

    #[test]
    fn audio_mime_picks_voice_note_regardless_of_size() {
        // Voice notes always render as the voice-note card (spec
        // §Inline rendering rules: "Voice notes always render as the
        // voice-note card (never a plain file card), regardless of size.")
        assert_eq!(
            pick("audio/ogg", 100),
            AttachmentKind::VoiceNote,
            "small audio should be voice-note card"
        );
        assert_eq!(
            pick("audio/wav", 50 * 1024 * 1024),
            AttachmentKind::VoiceNote,
            "even huge audio stays as voice-note card per spec"
        );
    }

    #[test]
    fn unknown_mime_picks_file_card() {
        // Unknown / non-image / non-audio mime falls back to the file
        // card per spec §Inline rendering rules.
        assert_eq!(pick("application/pdf", 1024), AttachmentKind::FileCard);
        assert_eq!(pick("application/octet-stream", 1024), AttachmentKind::FileCard);
        assert_eq!(pick("", 1024), AttachmentKind::FileCard);
        assert_eq!(pick("text/plain", 1024), AttachmentKind::FileCard);
    }

    #[test]
    fn image_decision_is_case_insensitive_on_mime() {
        // MIME types are case-insensitive per RFC 6838 §4.2; peers
        // sometimes send `IMAGE/PNG` or mixed case. The decision must
        // not flip just because the case differs.
        assert_eq!(pick("IMAGE/PNG", 1024), AttachmentKind::Image);
        assert_eq!(pick("Image/Jpeg", 1024), AttachmentKind::Image);
        assert_eq!(pick("AUDIO/Ogg", 1024), AttachmentKind::VoiceNote);
    }

    #[test]
    fn image_inline_cap_boundary_is_inclusive() {
        // Exactly 4 MB is allowed inline; one byte over degrades.
        assert_eq!(pick("image/png", IMAGE_INLINE_MAX_BYTES), AttachmentKind::Image);
        assert_eq!(
            pick("image/png", IMAGE_INLINE_MAX_BYTES + 1),
            AttachmentKind::FileCard
        );
    }
}
