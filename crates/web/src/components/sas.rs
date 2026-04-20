//! SAS fingerprint grid atoms — `FingerprintGrid`, `FingerprintLabel`,
//! and the byte-exact `SAS_COPY` strings.
//!
//! See `docs/specs/2026-04-19-ui-design/trust-verification.md` §SAS
//! fingerprint grid and §Copy — exact strings. No caller is permitted
//! to render their own grid; all surfaces consume this component so the
//! security-critical layout stays consistent.

use leptos::prelude::*;

use crate::icons;

/// Copy strings for the trust-verification UX, pinned byte-exact to
/// the reference bundle. See
/// `docs/specs/2026-04-19-ui-design/trust-verification.md` §Copy.
///
/// These strings are **security UI, not marketing** — rewording changes
/// what the user thinks they are protecting. A browser test asserts
/// every field matches the spec table.
pub mod sas_copy {
    pub const TITLE: &str = "add a friend";
    pub const INTRO: &str = "compare six words on two screens. if they match, no one can impersonate either of you in this conversation, ever.";
    pub const REASSURANCE: &str = "these six words come from your shared key. if someone tried to sit between you, at least one word would be different. verification gets stronger with repetition.";
    pub const YOU_META: &str = "just now · keys created";
    pub const PEER_META: &str = "arrived via nearby share";
    pub const MATCH_CTA: &str = "they match";
    pub const NO_MATCH_CTA: &str = "they don't match";
    pub const UNSURE_CTA: &str = "not sure";
    pub const LABEL_YOU: &str = "your fingerprint — read this aloud";
    pub const LABEL_PEER: &str = "their fingerprint — do these match?";

    pub const BADGE_VERIFIED: &str = "verified peer";
    pub const BADGE_UNVERIFIED: &str =
        "unverified — compare fingerprints before you trust this peer";
    pub const BADGE_PENDING: &str = "verification pending";
    pub const BADGE_PENDING_CHIP: &str = "compare →";
    pub const BADGE_NEW_PEER: &str = "new peer";

    pub const CONFIRM_MATCH_TITLE: &str = "verified.";
    pub const CONFIRM_MATCH_BODY: &str = "verified peer — this cannot be silently downgraded by an attacker. their key is pinned; if it ever changes you'll be asked to verify again.";
    pub const CONFIRM_MISMATCH_TITLE: &str = "marked not verified.";
    pub const CONFIRM_MISMATCH_BODY: &str = "marked not-verified — we will keep this peer unverified until you compare again. you can still send messages, but whisper and device handoff stay closed until the fingerprints match.";

    pub const DOWNGRADE_TITLE: &str = "keys changed — verify again";
    pub const DOWNGRADE_BODY: &str = "this peer's key rotated or a fingerprint check failed. whisper and device handoff are paused until you compare again.";
    pub const DOWNGRADE_CTA: &str = "compare now";
    pub const DOWNGRADE_DISMISS: &str = "dismiss for now";

    pub const HOLDER_PILL: &str = "{n} holders";
    pub const HOLDER_TITLE: &str = "who can read this channel";
    pub const HOLDER_SELF_FOOTER: &str = "you · holder since {t}";
}

/// Size variant for [`FingerprintGrid`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FingerprintSize {
    /// Desktop onboarding, desktop compare flow, desktop profile card.
    #[default]
    Md,
    /// Mobile onboarding, mobile compare sheet, mobile profile sheet.
    Sm,
}

impl FingerprintSize {
    fn class(self) -> &'static str {
        match self {
            FingerprintSize::Md => "sas-grid--md",
            FingerprintSize::Sm => "sas-grid--sm",
        }
    }
}

/// State-tint variant driven by the caller. The grid never infers its
/// own variant; see spec §State tint variants.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FingerprintVariant {
    /// "your fingerprint — read this aloud" — subtle left rule.
    You,
    /// "their fingerprint — do these match?" — neutral.
    Peer,
    /// Applied after `they match` — green cell borders, check icon.
    Matched,
    /// Applied after `they don't match` — warn dashed borders.
    Mismatch,
}

impl FingerprintVariant {
    fn class(self) -> &'static str {
        match self {
            FingerprintVariant::You => "sas-grid--you",
            FingerprintVariant::Peer => "sas-grid--peer",
            FingerprintVariant::Matched => "sas-grid--matched",
            FingerprintVariant::Mismatch => "sas-grid--mismatch",
        }
    }
}

/// Which section label to render above the grid.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FingerprintLabelWhich {
    You,
    Peer,
}

impl FingerprintLabelWhich {
    fn text(self) -> &'static str {
        match self {
            FingerprintLabelWhich::You => sas_copy::LABEL_YOU,
            FingerprintLabelWhich::Peer => sas_copy::LABEL_PEER,
        }
    }
}

/// Section label rendered above every [`FingerprintGrid`].
#[component]
pub fn FingerprintLabel(
    which: FingerprintLabelWhich,
    /// Matching size for the grid beneath — drives type scale.
    #[prop(default = FingerprintSize::Md)]
    size: FingerprintSize,
    /// If `Some`, renders a small status icon (check / shield) next to
    /// the label text — used by the matched / mismatch variants.
    #[prop(optional)]
    status_icon: Option<FingerprintVariant>,
) -> impl IntoView {
    let class = match size {
        FingerprintSize::Md => "sas-label sas-label--md",
        FingerprintSize::Sm => "sas-label sas-label--sm",
    };
    let variant_class = match which {
        FingerprintLabelWhich::You => " sas-label--you",
        FingerprintLabelWhich::Peer => " sas-label--peer",
    };
    let full_class = format!("{class}{variant_class}");
    let text = which.text();
    let icon = status_icon.map(|v| match v {
        FingerprintVariant::Matched => view! {
            <span class="sas-label__icon sas-label__icon--matched" aria-hidden="true">
                {icons::icon_check()}
            </span>
        }
        .into_any(),
        FingerprintVariant::Mismatch => view! {
            <span class="sas-label__icon sas-label__icon--mismatch" aria-hidden="true">
                {icons::icon_shield()}
            </span>
        }
        .into_any(),
        _ => view! { <span class="sas-label__icon" aria-hidden="true"></span> }.into_any(),
    });

    view! {
        <div class=full_class>
            <span class="sas-label__text">{text}</span>
            {icon}
        </div>
    }
}

/// The six-word SAS fingerprint grid.
///
/// Renders a 3-column × 2-row table with 1-indexed numbers and
/// lowercase words. Accessibility: `role="table"` with
/// `aria-label="your six-word fingerprint"`; each cell exposes
/// `aria-label="word {n}, {word}"`.
#[component]
pub fn FingerprintGrid(
    /// The six words in reading order (left-to-right, top-to-bottom).
    #[prop(into)]
    words: Signal<[String; 6]>,
    #[prop(default = FingerprintSize::Md)] size: FingerprintSize,
    #[prop(default = FingerprintVariant::Peer)] variant: FingerprintVariant,
    /// Screen-reader label for the grid as a whole. Defaults per spec.
    #[prop(default = "your six-word fingerprint".to_string(), into)]
    aria_label: String,
) -> impl IntoView {
    let base_class = format!("sas-grid {} {}", size.class(), variant.class());
    view! {
        <div
            class=base_class
            role="table"
            aria-label=aria_label
            data-size=move || match size {
                FingerprintSize::Md => "md",
                FingerprintSize::Sm => "sm",
            }
            data-variant=move || match variant {
                FingerprintVariant::You => "you",
                FingerprintVariant::Peer => "peer",
                FingerprintVariant::Matched => "matched",
                FingerprintVariant::Mismatch => "mismatch",
            }
        >
            {move || {
                let ws = words.get();
                (0..6usize)
                    .map(|i| {
                        let w = ws[i].clone();
                        let w_for_aria = w.clone();
                        let n = i + 1;
                        view! {
                            <div
                                class="sas-cell"
                                role="cell"
                                aria-label=format!("word {n}, {w_for_aria}")
                            >
                                <span class="sas-cell__num" aria-hidden="true">{n.to_string()}</span>
                                <span class="sas-cell__word">{w}</span>
                            </div>
                        }
                    })
                    .collect::<Vec<_>>()
            }}
        </div>
    }
}
