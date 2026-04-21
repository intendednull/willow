//! Unread badge — the moss pill that flags unread surfaces.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/notifications.md` §Unread badge.
//!
//! Variants (priority high→low):
//!   whisper → mentioned → announce-only → muted → default.
//!
//! Count format: `99+` beyond 99, single-digit numbers render as a
//! centred circle (same dimensions as the pill min-width). Mentioned
//! surfaces prefix the count with a 10 px `@` glyph. Muted surfaces
//! render outlined; the count still increments (muting silences
//! notifications, not attribution).
//!
//! Mobile tab-bar passes `dot: true` to render the 6×6 moss dot form.

use leptos::prelude::*;

use willow_client::views::UnreadStats;

/// Single-digit threshold where the badge renders as a circle.
const SINGLE_DIGIT_MAX: u32 = 9;
/// Number beyond which the badge renders as `99+`.
const OVERFLOW_THRESHOLD: u32 = 99;

/// The unread badge atom.
#[component]
pub fn UnreadBadge(
    /// Live stats for the surface.
    #[prop(into)]
    stats: Signal<UnreadStats>,
    /// Render as a 6×6 dot (mobile tab-bar idle variant).
    #[prop(default = false)]
    dot: bool,
) -> impl IntoView {
    let class_fn = move || {
        let s = stats.get();
        let mut c = String::from("unread-badge");
        if dot {
            c.push_str(" unread-badge--dot");
        } else if s.count <= SINGLE_DIGIT_MAX {
            c.push_str(" unread-badge--single");
        }
        // Apply the highest-priority variant class — whisper wins over
        // mentioned etc. to mirror the spec's priority table.
        if s.whisper {
            c.push_str(" unread-badge--whisper");
        } else if s.mentioned {
            c.push_str(" unread-badge--mentioned");
        } else if s.announce_only {
            c.push_str(" unread-badge--announce");
        }
        if s.muted {
            c.push_str(" unread-badge--muted");
        }
        c
    };

    let aria_label_fn = move || describe(&stats.get());

    view! {
        <span
            class=class_fn
            role="status"
            aria-label=aria_label_fn
        >
            {move || {
                if dot {
                    None
                } else {
                    let s = stats.get();
                    let count_text = if s.count > OVERFLOW_THRESHOLD {
                        "99+".to_string()
                    } else {
                        s.count.to_string()
                    };
                    Some(view! {
                        {s.mentioned.then(|| view! {
                            <span class="unread-badge__at" aria-hidden="true">"@"</span>
                        })}
                        <span class="unread-badge__count">{count_text}</span>
                    })
                }
            }}
        </span>
    }
}

/// Compose the accessible label for a surface's stats. Matches the
/// spec exactly: `"N unread"` / `"N unread, mentioned"` /
/// `"N unread whisper"` / `"N unread, muted"`.
pub fn describe(s: &UnreadStats) -> String {
    let count_text = if s.count > OVERFLOW_THRESHOLD {
        "99+".to_string()
    } else {
        s.count.to_string()
    };
    // Whisper and mentioned compose; muted is a standalone modifier.
    let mut out = format!("{count_text} unread");
    if s.whisper {
        out.push_str(" whisper");
    } else if s.mentioned {
        out.push_str(", mentioned");
    }
    if s.muted {
        out.push_str(", muted");
    }
    out
}
