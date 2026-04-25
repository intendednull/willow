//! Ephemeral surface configuration + derivation.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/ephemeral-channels.md`. The
//! contract: a channel marked ephemeral records its kind + idle
//! threshold; materialize tracks `last_activity_hlc` on every
//! message; `derive_ephemeral_state` returns the active/dormant/
//! archived band purely from those values vs the merge frontier.

use serde::{Deserialize, Serialize};

/// Subkind of an ephemeral surface. Channels, threads, and whispers
/// share the auto-archive mechanic but use different default
/// thresholds and surface placements.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EphemeralKind {
    Channel,
    Thread,
    Whisper,
}

/// Per-channel ephemeral configuration. Set at creation; cap is
/// enforced at apply-time (`[1h, 90d]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EphemeralConfig {
    pub kind: EphemeralKind,
    pub idle_threshold_ms: u64,
}

/// The derived band a channel sits in. Pure function of
/// `last_activity_hlc`, `idle_threshold_ms`, and frontier HLC.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EphemeralState {
    Active,
    Dormant,
    Archived,
}

/// Lower bound for `idle_threshold_ms` (1 hour).
pub const IDLE_THRESHOLD_MIN_MS: u64 = 3_600_000;
/// Upper bound for `idle_threshold_ms` (90 days).
pub const IDLE_THRESHOLD_MAX_MS: u64 = 90 * 24 * 3_600_000;

/// Default thresholds per kind.
pub const DEFAULT_CHANNEL_THRESHOLD_MS: u64 = 14 * 24 * 3_600_000;
pub const DEFAULT_THREAD_THRESHOLD_MS: u64 = 7 * 24 * 3_600_000;
pub const DEFAULT_WHISPER_THRESHOLD_MS: u64 = 24 * 3_600_000;

/// Compute the band from raw values. Active when activity is in the
/// most recent 25 % of the threshold; dormant in the 25–100 %
/// window; archived past it.
pub fn derive_ephemeral_state(
    last_activity_hlc_ms: Option<u64>,
    idle_threshold_ms: u64,
    frontier_hlc_ms: u64,
) -> EphemeralState {
    let last = last_activity_hlc_ms.unwrap_or(0);
    let elapsed = frontier_hlc_ms.saturating_sub(last);
    let dormant_floor = idle_threshold_ms / 4; // 25 %
    if elapsed > idle_threshold_ms {
        EphemeralState::Archived
    } else if elapsed > dormant_floor {
        EphemeralState::Dormant
    } else {
        EphemeralState::Active
    }
}
