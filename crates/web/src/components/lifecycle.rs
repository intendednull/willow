// crates/web/src/components/lifecycle.rs
//
// Four-phase lifecycle helpers for animated components. See
// docs/specs/2026-04-27-event-based-waits-design.md §`data-state` attribute
// pattern. Apply via `RwSignal<LifecycleState>` + a `transitionend` closure
// on the component's root element (filtered on the component's specific
// driving CSS property).
//
// `data-state` lifecycle is reserved for the four animated phases
// (closed/opening/open/closing). For categorical states (online/offline/away
// on `status_dot.rs`, idle/loading/connected on `grove_rail.rs`,
// presence labels in `peer_status_label.rs`) keep using `data-state` with
// custom strings — those usages are orthogonal and pre-date this module.

use web_sys::{Element, HtmlElement};

/// Animated component's transition phase.
///
/// `Opening` and `Closing` are set imperatively when the user triggers the
/// transition; `Open` and `Closed` are flipped by the component's
/// `transitionend` listener (or the reduced-motion shortcut).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleState {
    Closed,
    Opening,
    Open,
    Closing,
}

impl LifecycleState {
    /// String form for the `data-state` attribute. Keep in sync with e2e
    /// tests asserting `toHaveAttribute('data-state', ...)`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Opening => "opening",
            Self::Open => "open",
            Self::Closing => "closing",
        }
    }
}

/// Advance the lifecycle on `transitionend`.
///
/// `Opening` → `Open`, `Closing` → `Closed`. Other states are unchanged
/// (a stray `transitionend` while already terminal is a no-op, not an error).
pub const fn advance(state: LifecycleState) -> LifecycleState {
    match state {
        LifecycleState::Opening => LifecycleState::Open,
        LifecycleState::Closing => LifecycleState::Closed,
        terminal => terminal,
    }
}

/// Maximum transition duration (in seconds) that we treat as effectively
/// zero — i.e. "skip the wait, snap synchronously". 1ms is the threshold
/// because the global reduced-motion override at `style.css` writes
/// `transition-duration: 0.01ms !important` for every element, and we must
/// honour that as the authoritative reduced-motion contract. Any user CSS
/// using ≤1ms transitions is also indistinguishable from "no transition"
/// for the purpose of `transitionend` firing reliably across engines.
const ZERO_DURATION_EPSILON_SECONDS: f64 = 0.001;

/// Parse a single CSS `<time>` value (e.g. `"0s"`, `"0.01ms"`, `"250ms"`,
/// `"0.0001s"`) into seconds. Returns `None` for empty / malformed input —
/// callers must treat that as "not zero" to preserve the wait-for-event path.
///
/// Per CSS spec the computed value of `transition-duration` is canonicalised
/// to seconds, but engines vary in serialisation (some emit `"0.0001s"`,
/// others preserve `"0.01ms"`), so we accept both `s` and `ms` suffixes.
fn parse_duration_seconds(value: &str) -> Option<f64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Order matters: check `ms` before `s` (else `s` matches `ms` suffix's `s`).
    let (number, divisor) = if let Some(num) = trimmed.strip_suffix("ms") {
        (num, 1000.0_f64)
    } else if let Some(num) = trimmed.strip_suffix('s') {
        (num, 1.0_f64)
    } else {
        return None;
    };
    number.trim().parse::<f64>().ok().map(|n| n / divisor)
}

/// Predicate form of [`is_zero_duration`] operating on the raw computed
/// `transition-duration` string. Split out for unit testing without a DOM.
///
/// Empty input (no `transition` property set at all) → zero. Any
/// comma-separated component that fails to parse → not zero (conservative:
/// keep waiting for `transitionend` rather than snap incorrectly). All
/// components must parse and be ≤ [`ZERO_DURATION_EPSILON_SECONDS`] for the
/// element to count as zero-duration.
fn is_zero_duration_str(duration: &str) -> bool {
    if duration.is_empty() {
        return true;
    }
    duration.split(',').all(|component| {
        parse_duration_seconds(component)
            .is_some_and(|seconds| seconds <= ZERO_DURATION_EPSILON_SECONDS)
    })
}

/// Returns true if the element has no animation duration (reduced motion or
/// untransitioned). When this returns true, callers must snap to the
/// terminal lifecycle state synchronously without waiting for `transitionend`
/// — otherwise the test hangs because no event will fire.
///
/// Reads `getComputedStyle(el).transition-duration` and treats any value
/// ≤ 1ms as zero. The global `prefers-reduced-motion` rule in `style.css`
/// sets `transition-duration: 0.01ms !important` (≈ `0.0001s` once
/// canonicalised), so a strict `"0s" | "0ms"` check would miss it and the
/// caller would hang waiting for a `transitionend` event that never fires.
/// Multi-property transitions (comma-separated values) require *every*
/// component to be ≤ 1ms; an unparseable component is treated as non-zero.
pub fn is_zero_duration(element: &Element) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Ok(Some(computed)) = window.get_computed_style(element) else {
        return false;
    };
    let Ok(duration) = computed.get_property_value("transition-duration") else {
        return false;
    };
    is_zero_duration_str(&duration)
}

/// Convenience: convert an `&HtmlElement` to `&Element` for `is_zero_duration`.
pub fn is_zero_duration_html(html: &HtmlElement) -> bool {
    is_zero_duration(html.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_opening_to_open() {
        assert_eq!(advance(LifecycleState::Opening), LifecycleState::Open);
    }

    #[test]
    fn advance_closing_to_closed() {
        assert_eq!(advance(LifecycleState::Closing), LifecycleState::Closed);
    }

    #[test]
    fn advance_terminal_states_idempotent() {
        assert_eq!(advance(LifecycleState::Open), LifecycleState::Open);
        assert_eq!(advance(LifecycleState::Closed), LifecycleState::Closed);
    }

    #[test]
    fn as_str_round_trip() {
        for state in [
            LifecycleState::Closed,
            LifecycleState::Opening,
            LifecycleState::Open,
            LifecycleState::Closing,
        ] {
            // No round-trip parser by design — `as_str` is for the DOM attribute.
            assert!(!state.as_str().is_empty());
        }
    }

    #[test]
    fn parse_duration_seconds_handles_units() {
        assert_eq!(parse_duration_seconds("0s"), Some(0.0));
        assert_eq!(parse_duration_seconds("0ms"), Some(0.0));
        assert_eq!(parse_duration_seconds("1s"), Some(1.0));
        assert_eq!(parse_duration_seconds("250ms"), Some(0.25));
        assert_eq!(parse_duration_seconds("0.0001s"), Some(0.0001));
        assert_eq!(parse_duration_seconds("0.01ms"), Some(0.000_01));
        // Whitespace tolerated (comma-split leaves it).
        assert_eq!(parse_duration_seconds(" 100ms "), Some(0.1));
    }

    #[test]
    fn parse_duration_seconds_rejects_malformed() {
        assert_eq!(parse_duration_seconds(""), None);
        assert_eq!(parse_duration_seconds("100"), None); // no unit
        assert_eq!(parse_duration_seconds("abc"), None);
        assert_eq!(parse_duration_seconds("ms"), None); // no number
    }

    #[test]
    fn is_zero_duration_str_recognises_explicit_zero() {
        assert!(is_zero_duration_str(""));
        assert!(is_zero_duration_str("0s"));
        assert!(is_zero_duration_str("0ms"));
    }

    #[test]
    fn is_zero_duration_str_recognises_reduced_motion_override() {
        // The reduced-motion media query forces 0.01ms !important; engines
        // may serialise that as either form. Both must count as zero —
        // this is the regression case for issue #515.
        assert!(is_zero_duration_str("0.01ms"));
        assert!(is_zero_duration_str("0.0001s"));
    }

    #[test]
    fn is_zero_duration_str_treats_sub_millisecond_as_zero() {
        assert!(is_zero_duration_str("0.5ms"));
        assert!(is_zero_duration_str("1ms"));
    }

    #[test]
    fn is_zero_duration_str_rejects_real_durations() {
        assert!(!is_zero_duration_str("100ms"));
        assert!(!is_zero_duration_str("1.5ms"));
        assert!(!is_zero_duration_str("0.25s"));
        assert!(!is_zero_duration_str("1s"));
    }

    #[test]
    fn is_zero_duration_str_multi_value_all_zero() {
        assert!(is_zero_duration_str("0.01ms, 0s"));
        assert!(is_zero_duration_str("0s, 0ms, 0.0001s"));
    }

    #[test]
    fn is_zero_duration_str_multi_value_mixed_is_not_zero() {
        assert!(!is_zero_duration_str("100ms, 0s"));
        assert!(!is_zero_duration_str("0s, 250ms"));
    }

    #[test]
    fn is_zero_duration_str_unparseable_is_not_zero() {
        // Conservative: if we can't parse, keep waiting for transitionend.
        assert!(!is_zero_duration_str("garbage"));
        assert!(!is_zero_duration_str("0s, garbage"));
    }
}
