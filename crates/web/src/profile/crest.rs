//! Procedural crest banner SVGs.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
//! §Crest banner. Three deterministic patterns seeded by peer id.

use leptos::either::EitherOf3;
use leptos::prelude::*;
use willow_state::CrestPattern;

const MOSS_2_FALLBACK: &str = "var(--moss-2)";

/// Resolve `(pattern, color)` with spec defaults.
///
/// - `pattern == None` → [`CrestPattern::Leaf`]
/// - `color == None` or malformed → `var(--moss-2)` CSS token
pub fn crest_defaults(
    pattern: Option<CrestPattern>,
    color: Option<&str>,
) -> (CrestPattern, String) {
    let resolved_color = color
        .filter(|s| s.starts_with('#') && s.len() == 7)
        .map(|s| s.to_string())
        .unwrap_or_else(|| MOSS_2_FALLBACK.to_string());
    (pattern.unwrap_or(CrestPattern::Leaf), resolved_color)
}

/// Seed a deterministic PRNG from the peer id.
fn seed_rng(peer_id: &str) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"willow-crest-v1");
    h.update(peer_id.as_bytes());
    *h.finalize().as_bytes()
}

/// Extract a bounded integer from a seed slice.
fn roll(seed: &[u8; 32], idx: usize, modulus: u32) -> u32 {
    let off = (idx * 4) % (seed.len() - 4);
    let x = u32::from_le_bytes([seed[off], seed[off + 1], seed[off + 2], seed[off + 3]]);
    x % modulus
}

/// Render the crest banner SVG.
#[component]
pub fn CrestBanner(
    #[prop(into)] pattern: Signal<Option<CrestPattern>>,
    #[prop(into)] color: Signal<Option<String>>,
    #[prop(into)] peer_id: Signal<String>,
) -> impl IntoView {
    let svg = move || {
        let (p, c) = crest_defaults(pattern.get(), color.get().as_deref());
        let pid = peer_id.get();
        let seed = seed_rng(&pid);
        match p {
            CrestPattern::Fronds => EitherOf3::A(fronds(&seed, &c)),
            CrestPattern::Rings => EitherOf3::B(rings(&seed, &c)),
            CrestPattern::Leaf => EitherOf3::C(leaf(&seed, &c)),
        }
    };
    view! {
        <div class="profile-card__banner" aria-hidden="true">
            {svg}
        </div>
    }
}

fn fronds(seed: &[u8; 32], color: &str) -> impl IntoView {
    let color = color.to_string();
    let color_strokes = color.clone();
    let strokes = (0..14)
        .map(|i| {
            let x: i32 = 12 + i * 22;
            let sway = (roll(seed, i as usize, 20) as i32) - 10;
            let mx = x + sway;
            let tx = x + sway / 2;
            view! {
                <path
                    d=format!("M {x},92 Q {mx},58 {tx},8")
                    stroke=color_strokes.clone()
                    stroke-width="1.5"
                    fill="none"
                    opacity="0.55"
                />
            }
        })
        .collect_view();
    view! {
        <svg viewBox="0 0 320 92" preserveAspectRatio="none" class="profile-card__crest">
            {banner_washes(&color)}
            {strokes}
        </svg>
    }
}

fn rings(seed: &[u8; 32], color: &str) -> impl IntoView {
    let color = color.to_string();
    let color_strokes = color.clone();
    let scattered = (0..6)
        .map(|i| {
            let cx = 24 + roll(seed, i as usize, 270);
            let cy = 16 + roll(seed, (i + 30) as usize, 60);
            let r = 8 + roll(seed, (i + 60) as usize, 16);
            view! {
                <circle
                    cx=cx.to_string()
                    cy=cy.to_string()
                    r=r.to_string()
                    stroke=color_strokes.clone()
                    stroke-width="1.5"
                    fill="none"
                    opacity="0.5"
                />
            }
        })
        .collect_view();
    view! {
        <svg viewBox="0 0 320 92" preserveAspectRatio="none" class="profile-card__crest">
            {banner_washes(&color)}
            {scattered}
            <circle cx="160" cy="46" r="18" stroke=color.clone()
                    stroke-width="1.5" fill="none" opacity="0.7"/>
            <circle cx="160" cy="46" r="10" stroke=color
                    stroke-width="1.5" fill="none" opacity="0.9"/>
        </svg>
    }
}

fn leaf(seed: &[u8; 32], color: &str) -> impl IntoView {
    let color = color.to_string();
    let color_leaves = color.clone();
    let leaves = (0..9)
        .map(|i| {
            let x = 28 + i * 32;
            let y_off = (roll(seed, i as usize, 8) as i32) + 26;
            view! {
                <path
                    d=format!("M {x},{y_off} q 8,-14 16,0 q -8,14 -16,0 z")
                    fill=color_leaves.clone()
                    opacity="0.55"
                />
            }
        })
        .collect_view();
    view! {
        <svg viewBox="0 0 320 92" preserveAspectRatio="none" class="profile-card__crest">
            {banner_washes(&color)}
            <path d="M 0,52 Q 160,12 320,52" stroke=color.clone()
                  stroke-width="1.5" fill="none" opacity="0.35"/>
            {leaves}
        </svg>
    }
}

/// Vertical accent gradient behind the pattern + horizontal ink wash over it.
fn banner_washes(color: &str) -> impl IntoView {
    let c1 = color.to_string();
    let c2 = color.to_string();
    let c3 = color.to_string();
    view! {
        <defs>
            <linearGradient id="crest-v" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0" stop-color=c1 stop-opacity="0.55"/>
                <stop offset="0.6" stop-color=c2 stop-opacity="0.18"/>
                <stop offset="1" stop-color=c3 stop-opacity="0"/>
            </linearGradient>
            <linearGradient id="crest-h" x1="0" y1="0" x2="1" y2="0">
                <stop offset="0" stop-color="var(--bg-0)" stop-opacity="0"/>
                <stop offset="0.5" stop-color="var(--bg-0)" stop-opacity="0.22"/>
                <stop offset="1" stop-color="var(--bg-0)" stop-opacity="0"/>
            </linearGradient>
        </defs>
        <rect width="320" height="92" fill="url(#crest-v)"/>
        <rect width="320" height="92" fill="url(#crest-h)"/>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crest_defaults_none_none_returns_leaf_moss() {
        let (pattern, color) = crest_defaults(None, None);
        assert_eq!(pattern, CrestPattern::Leaf);
        assert_eq!(color, "var(--moss-2)");
    }

    #[test]
    fn crest_defaults_preserves_valid_hex_color() {
        let (_, color) = crest_defaults(None, Some("#6b8e4e"));
        assert_eq!(color, "#6b8e4e");
    }

    #[test]
    fn crest_defaults_rejects_bad_hex_and_falls_back_to_moss() {
        // no `#`
        let (_, c1) = crest_defaults(None, Some("ff00aa"));
        assert_eq!(c1, "var(--moss-2)");
        // wrong length
        let (_, c2) = crest_defaults(None, Some("#ff00"));
        assert_eq!(c2, "var(--moss-2)");
    }

    #[test]
    fn crest_defaults_preserves_explicit_pattern() {
        let (p, _) = crest_defaults(Some(CrestPattern::Rings), None);
        assert_eq!(p, CrestPattern::Rings);
    }

    #[test]
    fn seed_rng_deterministic_for_same_peer_id() {
        assert_eq!(seed_rng("abc"), seed_rng("abc"));
    }

    #[test]
    fn seed_rng_differs_for_different_peers() {
        assert_ne!(seed_rng("abc"), seed_rng("def"));
    }
}
