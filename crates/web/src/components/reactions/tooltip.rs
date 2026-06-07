//! Reactor tooltip copy helper.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/reactions-pins.md`
//! §Reactor tooltip:
//!
//! - 3 or fewer reactors: `"mira, ori, kes reacted"`.
//! - More than 3 reactors: `"mira, ori, and 5 others reacted"`
//!   (lowercase `and`, mono `5`).
//!
//! The desktop hover tooltip uses this string as the `title`
//! attribute on the reaction pill. Mobile press-and-hold surfaces
//! the same string in a popover card; that variant is deferred per
//! `docs/plans/2026-05-08-ui-phase-3c-reactions-pins.md`
//! §Ambiguity decisions §5 and lands in a follow-up.

/// Compose the reactor-tooltip string for the given list of reactors.
///
/// Reactors are display names (already resolved by the projection in
/// `willow_client::views::compute_messages_view`). Empty input
/// returns an empty string — callers should suppress the tooltip
/// entirely in that case rather than rendering empty hover text.
pub fn reactor_tooltip(reactors: &[String]) -> String {
    match reactors.len() {
        0 => String::new(),
        1 => format!("{} reacted", reactors[0]),
        2 => format!("{} and {} reacted", reactors[0], reactors[1]),
        3 => format!(
            "{}, {}, and {} reacted",
            reactors[0], reactors[1], reactors[2]
        ),
        _ => {
            // Spec §Reactor tooltip: "More than 3 reactors:
            // `first two, and N others`". We use `first two` =
            // first two display names, `N` = count beyond two.
            let extras = reactors.len() - 2;
            format!(
                "{}, {}, and {} others reacted",
                reactors[0], reactors[1], extras
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(input: &[&str]) -> Vec<String> {
        input.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_returns_empty_string() {
        assert_eq!(reactor_tooltip(&[]), "");
    }

    #[test]
    fn one_reactor() {
        assert_eq!(reactor_tooltip(&names(&["mira"])), "mira reacted");
    }

    #[test]
    fn two_reactors() {
        assert_eq!(
            reactor_tooltip(&names(&["mira", "ori"])),
            "mira and ori reacted"
        );
    }

    #[test]
    fn three_reactors_lists_each_by_name() {
        // Spec §Reactor tooltip: "3 or fewer reactors: `mira, ori,
        // kes reacted`" — every name surfaces, no `and N others`
        // collapse.
        assert_eq!(
            reactor_tooltip(&names(&["mira", "ori", "kes"])),
            "mira, ori, and kes reacted"
        );
    }

    #[test]
    fn four_reactors_collapses_to_first_two_plus_others() {
        assert_eq!(
            reactor_tooltip(&names(&["mira", "ori", "kes", "rin"])),
            "mira, ori, and 2 others reacted"
        );
    }

    #[test]
    fn many_reactors_collapses_per_spec() {
        let many: Vec<String> = (0..10).map(|i| format!("p{i}")).collect();
        assert_eq!(reactor_tooltip(&many), "p0, p1, and 8 others reacted");
    }
}
