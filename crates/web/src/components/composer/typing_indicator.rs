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
//! ## Aria-live debounce (T12)
//!
//! Spec §Screen reader flow: "Typing indicator announced via
//! `aria-live=\"polite\"` with debouncing (at most once per 5 s)."
//!
//! Implementation: the visible label always tracks `peers.get()` so
//! sighted users see fresh names instantly. A separate
//! visually-hidden `<span aria-live="polite">` carries the
//! announcement string. Its content is gated by [`should_announce`]
//! — a pure helper that takes `(last_announced_ms, current_ms,
//! peers_len)` and returns whether the announcement should refresh.
//! This separation lets us throttle screen-reader announcements
//! without throttling the visible row.
//!
//! Runner-up rejected: throttle the *peers signal itself* and have
//! both the visible label and the aria region read from the same
//! debounced source. Loses the spec's "user sees fresh names
//! instantly" affordance — the visible row would also lag the wire.
//! Two signals (one fresh, one throttled) is the cleaner separation.

use leptos::prelude::*;

/// Reserved capacity for the `aria-live` debounce gate. Spec
/// §Screen reader flow: "at most once per 5 s".
pub(crate) const TYPING_ARIA_DEBOUNCE_MS: f64 = 5_000.0;

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

    // Aria-live announcement state — throttled to once per 5 s per
    // spec §Screen reader flow. `RwSignal` so a single `Effect` can
    // both read the previous timestamp and update the published
    // label atomically. The first non-empty change always announces
    // (initial `last_announced_ms = None`).
    let aria_state = RwSignal::new(AriaThrottleState::default());
    Effect::new(move |_| {
        let names = peers.get();
        let now_ms = current_time_ms();
        aria_state.update(|state| {
            if should_announce(state.last_announced_ms, now_ms, names.len()) {
                state.label = format_typing(&names);
                state.last_announced_ms = Some(now_ms);
            }
        });
    });
    let aria_label = move || aria_state.get().label;

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
            // composer doesn't shift on each ping.
            //
            // `aria-hidden="true"` on the visible label is critical:
            // without it, screen readers would announce the visible
            // text on every change, defeating the 5 s debounce on the
            // companion `aria-live` span below.
            <span class="composer__typing-dots" aria-hidden="true">
                <span class="composer__typing-dot"></span>
                <span class="composer__typing-dot"></span>
                <span class="composer__typing-dot"></span>
            </span>
            <span class="composer__typing-label" aria-hidden="true">
                {move || label.get()}
            </span>
            // Screen-reader side — a visually-hidden span that is the
            // *only* aria-live region in the indicator. Its content
            // is updated at most once per 5 s by [`should_announce`]
            // so screen readers aren't flooded with "Alex is
            // writing… Alex and Bo are writing…" each ping.
            <span
                class="composer__typing-sr-only"
                aria-live="polite"
                aria-atomic="true"
            >
                {aria_label}
            </span>
        </div>
    }
}

/// Internal aria-live throttle state. Decoupled from the visible
/// label so the debounce gate only affects what screen readers
/// announce.
#[derive(Clone, Default, PartialEq)]
struct AriaThrottleState {
    /// Timestamp (ms since epoch) of the last announcement, or
    /// `None` if we have not announced yet (next non-empty change
    /// announces).
    last_announced_ms: Option<f64>,
    /// The label currently published to the aria-live region.
    label: String,
}

/// Decide whether the aria-live announcement should be refreshed.
///
/// Returns `true` when:
/// - `peers_len > 0` and we have not announced yet
///   (`last_announced_ms == None`). The empty case stays silent so
///   we never announce the "no one is typing" transition.
/// - At least [`TYPING_ARIA_DEBOUNCE_MS`] (5 s) has elapsed since
///   the previous announcement, and `peers_len > 0`.
///
/// Pure function — exposed at module scope so the unit tests below
/// can drive it with synthetic timestamps without standing up a
/// browser harness. The 5 s boundary is **inclusive** to match the
/// spec's "at most once per 5 s" wording (a refresh exactly 5 s
/// after the previous one is the next allowed announcement).
pub(crate) fn should_announce(
    last_announced_ms: Option<f64>,
    current_ms: f64,
    peers_len: usize,
) -> bool {
    if peers_len == 0 {
        // Never announce empty — that would read "" to screen readers
        // every time a stale ping expires.
        return false;
    }
    match last_announced_ms {
        None => true,
        Some(prev) => (current_ms - prev) >= TYPING_ARIA_DEBOUNCE_MS,
    }
}

/// Current epoch milliseconds. Cfg-gated so native `cargo test`
/// builds don't pull in `js_sys` (which is wasm-only). The native
/// path returns a value derived from `std::time::SystemTime`;
/// production code is wasm-only.
#[cfg(target_arch = "wasm32")]
fn current_time_ms() -> f64 {
    js_sys::Date::now()
}

#[cfg(not(target_arch = "wasm32"))]
fn current_time_ms() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1_000.0)
        .unwrap_or(0.0)
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

    // ── T12 — aria-live debounce gate ────────────────────────────
    //
    // Spec: `composer.md` §Screen reader flow (line 257-259) — the
    // aria-live announcement throttles to "at most once per 5 s".
    // `should_announce` is the pure helper that owns that decision
    // so the browser-test harness doesn't have to reach for a fake
    // clock.

    #[test]
    fn should_announce_first_change_with_peers() {
        // Initial state: never announced. First non-empty peers list
        // must announce so the screen reader hears who started
        // typing.
        assert!(should_announce(None, 1_000.0, 1));
    }

    #[test]
    fn should_announce_skips_empty_peers() {
        // Empty peers means "no one is typing right now"; we must
        // not announce that to screen readers. Empty initial state
        // *and* an active session that has just transitioned to
        // empty (typing-ping TTL expired) both stay silent.
        assert!(!should_announce(None, 1_000.0, 0));
        assert!(!should_announce(Some(0.0), 10_000.0, 0));
    }

    #[test]
    fn should_announce_throttles_within_5s() {
        // Announced at t=1000, again at t=1500 (within 5 s window).
        assert!(!should_announce(Some(1_000.0), 1_500.0, 1));
        // Edge: 4 999 ms after — still throttled.
        assert!(!should_announce(Some(1_000.0), 5_999.0, 2));
    }

    #[test]
    fn should_announce_unblocks_after_5s() {
        // Boundary inclusive per "at most once per 5 s" — a refresh
        // exactly 5 000 ms after the previous one is the next
        // allowed announcement.
        assert!(should_announce(Some(1_000.0), 6_000.0, 1));
        // Far past the window.
        assert!(should_announce(Some(1_000.0), 10_000.0, 3));
    }
}
