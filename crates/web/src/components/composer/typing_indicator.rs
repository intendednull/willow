//! `<TypingIndicator>` — thin row above the composer textarea.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/composer.md` §Typing indicator.
//! Plan: `docs/plans/2026-04-26-ui-phase-3a-composer.md` Tasks T11–T12.
//!
//! ## Composition
//!
//! - Renders nothing when the peers list is empty (the parent's
//!   `--empty` modifier collapses padding so the composer doesn't
//!   shift on every typing-ping update).
//! - Otherwise renders a 3-dot `willowPulse` cluster (staggered
//!   0 / 200 / 400 ms via the `:nth-child` rules) followed by a
//!   localised label per the spec's pluralisation table:
//!     * 1 peer:   `{name} is writing…`
//!     * 2 peers:  `{name} and {name} are writing…`
//!     * 3 peers:  `{name}, {name}, and {name} are writing…`
//!     * 4+ peers: `{count} people are writing…`
//!
//! ## Source of typing peers
//!
//! T11 keeps the component **purely presentational**: it accepts a
//! `peers: Signal<Vec<String>>` prop. Production wiring in `app.rs`
//! reads display names off the existing `channel_views` map (filled
//! by the typing-expiry timer in `app.rs`), so the polling logic is
//! not duplicated here. Tests inject a `Signal::derive(...)` directly
//! to drive each pluralisation case without standing up a real
//! `ClientHandle`.
//!
//! Runner-up rejected: have `<TypingIndicator>` poll
//! `client.typing_in(channel)` on its own 1 Hz timer. That works but
//! duplicates the existing `app.rs` typing-expiry timer at line 544
//! and forks the source of truth for "who is typing right now". A
//! single timer + signal-prop is cleaner and matches how `<MetaRow>`
//! consumes `peer_count` rather than re-deriving it.
//!
//! T12 layers an aria-live announcement element with a 5 s debounce
//! on top — see the inline comment in [`TypingIndicator`].

use leptos::prelude::*;

/// Composer typing-indicator row.
///
/// `peers` is the list of display names currently typing in the
/// active channel, **already filtered to exclude the local peer**
/// (the underlying `Client::typing_in` accessor handles that).
#[component]
pub fn TypingIndicator(
    /// Display names of peers currently typing in the active channel.
    /// An empty vec collapses the row entirely.
    peers: Signal<Vec<String>>,
) -> impl IntoView {
    let label = Memo::new(move |_| format_typing(&peers.get()));
    let is_empty = move || peers.get().is_empty();

    view! {
        <div
            class=move || {
                let mut c = String::from("composer__typing-indicator");
                if is_empty() {
                    c.push_str(" composer__typing-indicator--empty");
                }
                c
            }
            data-empty=move || is_empty().to_string()
        >
            // Visible side — animated dots + italic label. Hidden via
            // `--empty` styling when no peers are typing so the
            // composer doesn't shift on each ping. T12 adds an
            // additional `aria-live` span next to this label that
            // throttles screen-reader announcements to once per 5 s.
            <span class="composer__typing-dots" aria-hidden="true">
                <span class="composer__typing-dot"></span>
                <span class="composer__typing-dot"></span>
                <span class="composer__typing-dot"></span>
            </span>
            <span class="composer__typing-label">{move || label.get()}</span>
        </div>
    }
}

/// Format the typing-indicator label per the spec's pluralisation
/// table. The trailing character is the Unicode horizontal ellipsis
/// `…` (`U+2026`), not three ASCII dots — matches `composer.md`
/// §Typing indicator copy.
///
/// Empty input returns the empty string. The caller is responsible
/// for hiding the row entirely when the result is empty (the
/// component does this via `composer__typing-indicator--empty`).
pub(crate) fn format_typing(names: &[String]) -> String {
    match names.len() {
        0 => String::new(),
        1 => format!("{} is writing\u{2026}", names[0]),
        2 => format!("{} and {} are writing\u{2026}", names[0], names[1]),
        3 => format!(
            "{}, {}, and {} are writing\u{2026}",
            names[0], names[1], names[2]
        ),
        n => format!("{n} people are writing\u{2026}"),
    }
}

#[cfg(test)]
mod tests {
    //! Pure unit tests for the typing-indicator helpers. These run on
    //! native `cargo test` because `format_typing` has no DOM
    //! dependency. Browser-level tests live in
    //! `crates/web/tests/browser.rs::phase_3a_composer`.

    use super::*;

    #[test]
    fn format_typing_one_name() {
        let names = vec!["alex".to_string()];
        assert_eq!(format_typing(&names), "alex is writing\u{2026}");
    }

    #[test]
    fn format_typing_two_names() {
        let names = vec!["alex".to_string(), "bo".to_string()];
        assert_eq!(format_typing(&names), "alex and bo are writing\u{2026}");
    }

    #[test]
    fn format_typing_three_names() {
        let names = vec!["alex".to_string(), "bo".to_string(), "cy".to_string()];
        assert_eq!(
            format_typing(&names),
            "alex, bo, and cy are writing\u{2026}"
        );
    }

    #[test]
    fn format_typing_four_or_more_names() {
        let names = vec![
            "alex".to_string(),
            "bo".to_string(),
            "cy".to_string(),
            "dee".to_string(),
        ];
        assert_eq!(format_typing(&names), "4 people are writing\u{2026}");

        let many: Vec<String> = (0..7).map(|i| format!("p{i}")).collect();
        assert_eq!(format_typing(&many), "7 people are writing\u{2026}");
    }

    #[test]
    fn format_typing_empty_is_blank() {
        // Empty input must return the empty string so the row can
        // collapse without a stray label leaking through.
        assert_eq!(format_typing(&[]), "");
    }
}
