//! Peer-trust badge.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/trust-verification.md` §Badges
//! and §Placement rules by surface.
//!
//! Five visual states — verified, unverified, pending-verify, new peer,
//! downgraded — each with a shape cue (not colour alone) and an exact
//! `aria-label` from the copy table. Click / focus + Enter opens the
//! compare-fingerprints dialog by writing the peer id into
//! `trust.compare_target`.

use leptos::prelude::*;
use willow_client::trust::PeerTrust;

use super::sas::sas_copy;
use crate::icons;
use crate::state::{AppState, AppWriteSignals};

/// Size variant. `Disk12` is the default for inline badges next to a
/// display name; `Disk14` is the letter-row size; `Pill` is the
/// crest variant for profile-card headers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TrustBadgeSize {
    Disk12,
    #[default]
    Disk14,
    Pill,
}

impl TrustBadgeSize {
    fn size_class(self) -> &'static str {
        match self {
            TrustBadgeSize::Disk12 => "trust-badge--disk-12",
            TrustBadgeSize::Disk14 => "trust-badge--disk",
            TrustBadgeSize::Pill => "trust-badge--pill",
        }
    }
}

/// Whether the surface mounting this badge is a participant tile. Adds
/// the `trust-badge--tile-corner` background so the badge reads against
/// the live video.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TrustBadgeContext {
    #[default]
    Inline,
    TileCorner,
}

/// Render the trust badge for a peer. Focus + Enter opens the compare
/// flow. The badge never self-soft-hides: callers must always mount it
/// on every peer-identifying surface per spec.
#[component]
pub fn TrustBadge(
    /// Peer ID string (Ed25519 public key base32).
    #[prop(into)]
    peer_id: String,
    #[prop(default = TrustBadgeSize::default())] size: TrustBadgeSize,
    #[prop(default = TrustBadgeContext::default())] context: TrustBadgeContext,
) -> impl IntoView {
    let app_state = use_context::<AppState>().expect("AppState in context");
    let write = use_context::<AppWriteSignals>().expect("AppWriteSignals in context");
    let trust_map = app_state.trust.trust_map;
    let set_compare_target = write.trust.set_compare_target;

    let peer_id_for_lookup = peer_id.clone();
    let peer_id_for_click = peer_id.clone();

    let state_memo = Memo::new(move |_| {
        trust_map
            .get()
            .get(&peer_id_for_lookup)
            .cloned()
            .unwrap_or_default()
    });

    let size_class = size.size_class();
    let corner_class = match context {
        TrustBadgeContext::Inline => "",
        TrustBadgeContext::TileCorner => " trust-badge--tile-corner",
    };

    view! {
        {move || {
            let trust = state_memo.get();
            let (state_class, aria, body) = match &trust {
                PeerTrust::Verified { .. } => (
                    "trust-badge--verified",
                    sas_copy::BADGE_VERIFIED.to_string(),
                    view! {
                        <span class="trust-badge__icon" aria-hidden="true">
                            {icons::icon_check()}
                        </span>
                    }.into_any(),
                ),
                PeerTrust::Unverified { .. } => (
                    "trust-badge--unverified",
                    sas_copy::BADGE_UNVERIFIED.to_string(),
                    view! { <span class="trust-badge__glyph" aria-hidden="true">"?"</span> }.into_any(),
                ),
                PeerTrust::DowngradedFromVerified { .. } => (
                    "trust-badge--downgrade",
                    sas_copy::BADGE_UNVERIFIED.to_string(),
                    view! {
                        <span class="trust-badge__icon" aria-hidden="true">
                            {icons::icon_shield()}
                        </span>
                    }.into_any(),
                ),
                PeerTrust::PendingVerify => (
                    "trust-badge--pending",
                    sas_copy::BADGE_PENDING.to_string(),
                    view! { <span class="trust-badge__glyph" aria-hidden="true">"?"</span> }.into_any(),
                ),
                PeerTrust::Unknown => (
                    "trust-badge--new",
                    sas_copy::BADGE_NEW_PEER.to_string(),
                    view! { <span class="trust-badge__text">{sas_copy::BADGE_NEW_PEER}</span> }
                        .into_any(),
                ),
            };

            let full_class = format!("trust-badge {size_class} {state_class}{corner_class}");
            let peer_id_click = peer_id_for_click.clone();
            view! {
                <button
                    type="button"
                    class=full_class
                    aria-label=aria
                    data-trust-state=match &trust {
                        PeerTrust::Verified { .. } => "verified",
                        PeerTrust::Unverified { .. } => "unverified",
                        PeerTrust::DowngradedFromVerified { .. } => "downgrade",
                        PeerTrust::PendingVerify => "pending",
                        PeerTrust::Unknown => "new",
                    }
                    on:click=move |ev| {
                        ev.stop_propagation();
                        set_compare_target.set(Some(peer_id_click.clone()));
                    }
                >
                    {body}
                </button>
            }
        }}
    }
}
