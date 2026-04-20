//! # Presence derivation
//!
//! Pure functions and types that map a bundle of presence inputs —
//! reachability, last-heartbeat tick, voice membership, whisper / queue
//! hints, and a local self-override — to a single [`PresenceState`]
//! value per peer.
//!
//! Presence in phase 1e is **derived, not event-sourced**. Inputs flow
//! in from [`willow-network`](willow_network), the client's voice state,
//! and stub signals for whisper/queue/invisibility. This module is
//! `no_std`-friendly: zero I/O, zero allocation beyond what [`PresenceState`]
//! itself carries, so it compiles identically on native and WASM.
//!
//! Spec: [`docs/specs/2026-04-19-ui-design/presence.md`].

use std::collections::HashSet;

use willow_identity::EndpointId;

/// Logical tick count. 1 tick = 1 second in phase 1e.
///
/// Swapped for the HLC timestamp field in a later phase; the
/// `saturating_sub` semantics carry over unchanged.
pub type Tick = u64;

/// Default idle threshold — 6 minutes.
pub const DEFAULT_IDLE_TICKS: Tick = 360;
/// Default gone threshold — 48 hours.
pub const DEFAULT_GONE_TICKS: Tick = 172_800;

/// The seven-state presence catalog plus an `Unknown` bottom.
///
/// See §State catalog in the spec. Shapes, colours, icons and copy are
/// assigned at the *atom* layer; this enum is pure data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresenceState {
    /// Peer reachable + heartbeat fresh.
    Here,
    /// Peer reachable but idle (no heartbeat for ≥ idle threshold).
    Away,
    /// Peer is in an active whisper session.
    Whispering,
    /// Peer is in a voice call (and not whispering).
    InCall,
    /// Peer unreachable AND we hold ≥ 1 queued message for them.
    Queued(u32),
    /// Peer unreachable for ≥ gone threshold.
    Gone,
    /// Peer is invisible to us (mutual hide).
    Invisible,
    /// Haven't observed the peer yet — used before any inputs arrive.
    Unknown,
}

impl PresenceState {
    /// Canonical label from the state catalog. Caller decides whether to
    /// render the label (surfaces like the member rail hide it).
    pub fn label(&self) -> String {
        match self {
            Self::Here => "here".to_string(),
            Self::Away => "away".to_string(),
            Self::Whispering => "whispering".to_string(),
            Self::InCall => "in a call".to_string(),
            Self::Queued(n) => {
                if *n > 99 {
                    "queued · 99+".to_string()
                } else {
                    format!("queued · {n}")
                }
            }
            Self::Gone => "gone".to_string(),
            Self::Invisible => "invisible".to_string(),
            Self::Unknown => "here".to_string(),
        }
    }

    /// Short state id used as the `data-state` attribute and CSS modifier.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Here | Self::Unknown => "here",
            Self::Away => "away",
            Self::Whispering => "whispering",
            Self::InCall => "in-a-call",
            Self::Queued(_) => "queued",
            Self::Gone => "gone",
            Self::Invisible => "invisible",
        }
    }
}

/// Local self-override — a *sticky* preference stored per device.
///
/// Matches the four menu entries in §Self-presence + manual override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PresenceOverride {
    /// Auto-derived — honour the computed state.
    #[default]
    Auto,
    /// Force `away`.
    Away,
    /// Force `gone`.
    Gone,
    /// Force `invisible` (mutual hide signalled).
    Invisible,
}

/// Inputs bundled together for a single peer's presence derivation.
///
/// All fields use the same `Tick` clock. `last_seen` is the tick at which
/// the last heartbeat (or reachability probe) was observed; `now` is the
/// current tick from the [`PresenceMeta`] actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresenceInputs {
    pub now: Tick,
    pub last_seen: Tick,
    pub reachable: bool,
    pub in_call: bool,
    pub whispering: bool,
    pub queue_depth: u32,
    pub invisible_to_me: bool,
    pub idle_ticks: Tick,
    pub gone_ticks: Tick,
}

impl Default for PresenceInputs {
    fn default() -> Self {
        Self {
            now: 0,
            last_seen: 0,
            reachable: false,
            in_call: false,
            whispering: false,
            queue_depth: 0,
            invisible_to_me: false,
            idle_ticks: DEFAULT_IDLE_TICKS,
            gone_ticks: DEFAULT_GONE_TICKS,
        }
    }
}

/// Derive a peer's presence state from its inputs.
///
/// Precedence (highest → lowest):
///   `invisible` > `whispering` > `in a call` > `queued · N` > `gone` >
///   `away` > `here`.
///
/// See the spec §State catalog and §Auto transitions.
pub fn derive_peer_presence(inputs: &PresenceInputs) -> PresenceState {
    if inputs.invisible_to_me {
        return PresenceState::Invisible;
    }
    let elapsed = inputs.now.saturating_sub(inputs.last_seen);

    if inputs.reachable {
        if inputs.whispering {
            return PresenceState::Whispering;
        }
        if inputs.in_call {
            return PresenceState::InCall;
        }
        // Reachable peer with queued outbound is an edge case — spec
        // says queued only when unreachable, so we clear the queue
        // visually and fall through to here/away.
        if elapsed >= inputs.idle_ticks {
            return PresenceState::Away;
        }
        return PresenceState::Here;
    }

    // Unreachable branch.
    if inputs.queue_depth > 0 && elapsed < inputs.gone_ticks {
        return PresenceState::Queued(inputs.queue_depth);
    }
    if elapsed >= inputs.gone_ticks {
        return PresenceState::Gone;
    }
    PresenceState::Away
}

/// Derive the self-presence state.
///
/// Self has its own derivation: queue depth is never shown (you don't
/// queue to yourself), and manual overrides win over the auto result.
pub fn derive_self_presence(
    override_: PresenceOverride,
    reachable: bool,
    in_call: bool,
    whispering: bool,
) -> PresenceState {
    match override_ {
        PresenceOverride::Away => return PresenceState::Away,
        PresenceOverride::Gone => return PresenceState::Gone,
        PresenceOverride::Invisible => return PresenceState::Invisible,
        PresenceOverride::Auto => {}
    }
    if whispering {
        PresenceState::Whispering
    } else if in_call {
        PresenceState::InCall
    } else if reachable {
        PresenceState::Here
    } else {
        PresenceState::Away
    }
}

/// The presence meta — mutable per-peer tick / queue / session metadata
/// held in the [`state_actors::PresenceMeta`](crate::state_actors::PresenceMeta)
/// actor. Kept in this module so the derivation helpers have direct
/// access to field defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceSnapshot {
    pub last_seen: std::collections::HashMap<EndpointId, Tick>,
    pub queue_depth: std::collections::HashMap<EndpointId, u32>,
    pub whispering_with: HashSet<EndpointId>,
    pub invisible_to_me: HashSet<EndpointId>,
    pub self_override: PresenceOverride,
    pub in_call_peers: HashSet<EndpointId>,
    pub reachable_peers: HashSet<EndpointId>,
    pub now: Tick,
    pub idle_ticks: Tick,
    pub gone_ticks: Tick,
}

impl Default for PresenceSnapshot {
    fn default() -> Self {
        Self {
            last_seen: Default::default(),
            queue_depth: Default::default(),
            whispering_with: Default::default(),
            invisible_to_me: Default::default(),
            self_override: PresenceOverride::Auto,
            in_call_peers: Default::default(),
            reachable_peers: Default::default(),
            now: 0,
            idle_ticks: DEFAULT_IDLE_TICKS,
            gone_ticks: DEFAULT_GONE_TICKS,
        }
    }
}

impl PresenceSnapshot {
    /// Build [`PresenceInputs`] for a given peer from the snapshot.
    pub fn inputs_for(&self, peer: &EndpointId) -> PresenceInputs {
        PresenceInputs {
            now: self.now,
            last_seen: self.last_seen.get(peer).copied().unwrap_or(0),
            reachable: self.reachable_peers.contains(peer),
            in_call: self.in_call_peers.contains(peer),
            whispering: self.whispering_with.contains(peer),
            queue_depth: self.queue_depth.get(peer).copied().unwrap_or(0),
            invisible_to_me: self.invisible_to_me.contains(peer),
            idle_ticks: self.idle_ticks,
            gone_ticks: self.gone_ticks,
        }
    }

    /// Derive a peer's presence state from the snapshot.
    pub fn peer(&self, peer: &EndpointId) -> PresenceState {
        derive_peer_presence(&self.inputs_for(peer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(now: Tick, last_seen: Tick, reachable: bool) -> PresenceInputs {
        PresenceInputs {
            now,
            last_seen,
            reachable,
            ..PresenceInputs::default()
        }
    }

    #[test]
    fn reachable_fresh_peer_is_here() {
        let i = inputs(100, 90, true);
        assert_eq!(derive_peer_presence(&i), PresenceState::Here);
    }

    #[test]
    fn reachable_stale_peer_is_away_after_idle_threshold() {
        // 360 ticks = 6 minutes default idle threshold.
        let mut i = inputs(1_000, 100, true);
        i.idle_ticks = 360;
        assert_eq!(derive_peer_presence(&i), PresenceState::Away);
    }

    #[test]
    fn unreachable_with_queue_is_queued() {
        let mut i = inputs(100, 50, false);
        i.queue_depth = 3;
        i.gone_ticks = 10_000;
        assert_eq!(derive_peer_presence(&i), PresenceState::Queued(3));
    }

    #[test]
    fn unreachable_past_gone_threshold_is_gone() {
        let mut i = inputs(200_000, 0, false);
        i.queue_depth = 0;
        i.gone_ticks = 172_800;
        assert_eq!(derive_peer_presence(&i), PresenceState::Gone);
    }

    #[test]
    fn invisibility_wins_over_everything() {
        let mut i = inputs(100, 90, true);
        i.whispering = true;
        i.in_call = true;
        i.queue_depth = 9;
        i.invisible_to_me = true;
        assert_eq!(derive_peer_presence(&i), PresenceState::Invisible);
    }

    #[test]
    fn whisper_wins_over_call_when_both_active() {
        let mut i = inputs(100, 90, true);
        i.in_call = true;
        i.whispering = true;
        assert_eq!(derive_peer_presence(&i), PresenceState::Whispering);
    }

    #[test]
    fn self_override_sticks_regardless_of_auto_signals() {
        // Away override persists even when we're reachable + in a call.
        assert_eq!(
            derive_self_presence(PresenceOverride::Away, true, true, false),
            PresenceState::Away,
        );
        // Gone override also sticks through whispering.
        assert_eq!(
            derive_self_presence(PresenceOverride::Gone, true, false, true),
            PresenceState::Gone,
        );
        // Invisible is sticky too.
        assert_eq!(
            derive_self_presence(PresenceOverride::Invisible, true, false, false),
            PresenceState::Invisible,
        );
    }

    #[test]
    fn self_auto_matches_expected_auto_table() {
        // Auto + all quiet + reachable = Here.
        assert_eq!(
            derive_self_presence(PresenceOverride::Auto, true, false, false),
            PresenceState::Here,
        );
        // Auto + unreachable = Away (not Gone — self never auto-goes gone).
        assert_eq!(
            derive_self_presence(PresenceOverride::Auto, false, false, false),
            PresenceState::Away,
        );
        // Auto + in_call = InCall.
        assert_eq!(
            derive_self_presence(PresenceOverride::Auto, true, true, false),
            PresenceState::InCall,
        );
        // Auto + whispering beats in_call.
        assert_eq!(
            derive_self_presence(PresenceOverride::Auto, true, true, true),
            PresenceState::Whispering,
        );
    }

    #[test]
    fn queued_label_caps_at_99_plus() {
        assert_eq!(PresenceState::Queued(1).label(), "queued · 1");
        assert_eq!(PresenceState::Queued(99).label(), "queued · 99");
        assert_eq!(PresenceState::Queued(100).label(), "queued · 99+");
        assert_eq!(PresenceState::Queued(9_999).label(), "queued · 99+");
    }
}
