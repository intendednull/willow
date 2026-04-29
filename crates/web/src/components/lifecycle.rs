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

/// Returns true if the element has no animation duration (reduced motion or
/// untransitioned). When this returns true, callers must snap to the
/// terminal lifecycle state synchronously without waiting for `transitionend`
/// — otherwise the test hangs because no event will fire.
///
/// Reads `getComputedStyle(el).transition-duration`. Empty string and "0s"
/// both count as zero. Multi-property transitions (comma-separated values)
/// are conservatively treated as non-zero unless every component is "0s",
/// matching the spec's "if computed-zero, skip wait" semantics.
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
    if duration.is_empty() {
        return true;
    }
    duration
        .split(',')
        .all(|d| matches!(d.trim(), "" | "0s" | "0ms"))
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
}
