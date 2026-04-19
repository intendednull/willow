//! # Hybrid Logical Clock (HLC)
//!
//! A clock that combines wall-clock time with a logical counter to produce
//! timestamps that are:
//!
//! 1. **Monotonically increasing** on each node.
//! 2. **Consistent across peers** even when system clocks drift slightly.
//! 3. **Totally ordered** — any two timestamps can be compared.
//!
//! ## How it works
//!
//! Each [`HlcTimestamp`] has two components:
//!
//! - `millis` — milliseconds since the Unix epoch (wall-clock component).
//! - `counter` — a logical counter that breaks ties when events happen within
//!   the same millisecond.
//!
//! When generating a new timestamp ([`HLC::now`]):
//!
//! - Take `max(wall_clock, last_millis)`.
//! - If the millisecond didn't advance, increment the counter; otherwise reset
//!   it to zero.
//!
//! When receiving a remote timestamp ([`HLC::receive`]):
//!
//! - Take `max(wall_clock, last_millis, remote_millis)`.
//! - Advance the counter past both the local and remote counters if in the same
//!   millisecond.
//!
//! ## References
//!
//! Based on the algorithm from *"Logical Physical Clocks and Consistent
//! Snapshots in Globally Distributed Databases"* (Kulkarni et al., 2014).

use serde::{Deserialize, Serialize};

/// A single HLC timestamp that can be compared, serialized, and sent over the
/// network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HlcTimestamp {
    /// Wall-clock component (milliseconds since Unix epoch).
    pub millis: u64,
    /// Logical counter that breaks ties within the same millisecond.
    pub counter: u32,
}

impl HlcTimestamp {
    /// The zero timestamp — used as an initial state.
    pub const ZERO: Self = Self {
        millis: 0,
        counter: 0,
    };
}

impl PartialOrd for HlcTimestamp {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HlcTimestamp {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.millis
            .cmp(&other.millis)
            .then(self.counter.cmp(&other.counter))
    }
}

impl std::fmt::Display for HlcTimestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.millis, self.counter)
    }
}

// ───── HLC state machine ────────────────────────────────────────────────────

/// Returns the current wall-clock time in milliseconds since Unix epoch.
#[cfg(not(target_arch = "wasm32"))]
fn wall_clock_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_millis() as u64
}

/// Returns the current wall-clock time in milliseconds since Unix epoch (WASM).
#[cfg(target_arch = "wasm32")]
fn wall_clock_ms() -> u64 {
    js_sys::Date::now() as u64
}

/// Advance `counter` by one within `millis`, rolling over into the next
/// millisecond if the u32 counter would overflow.
///
/// The counter field is only 32 bits wide, so a node that generates more
/// than `u32::MAX` timestamps inside a single millisecond would otherwise
/// wrap around and break monotonicity. When that happens we borrow from
/// the millisecond field instead: bump `millis` by 1 and reset the counter
/// to 0. This keeps every generated timestamp strictly greater than the
/// previous one.
fn bump_counter(millis: u64, counter: u32) -> (u64, u32) {
    match counter.checked_add(1) {
        Some(next) => (millis, next),
        // Saturated: roll into the next millisecond to preserve monotonicity.
        None => (millis.saturating_add(1), 0),
    }
}

/// A Hybrid Logical Clock instance.
///
/// Each node in the network maintains its own `HLC`. It must be used to
/// timestamp **every** outbound event and updated on **every** inbound event.
///
/// # Examples
///
/// ```
/// use willow_messaging::hlc::HLC;
///
/// let mut clock = HLC::new();
///
/// // Generate a local timestamp.
/// let t1 = clock.now();
/// let t2 = clock.now();
/// assert!(t2 > t1);
/// ```
pub struct HLC {
    latest: HlcTimestamp,
}

impl HLC {
    /// Create a new clock starting from zero.
    pub fn new() -> Self {
        Self {
            latest: HlcTimestamp::ZERO,
        }
    }

    /// The most recent timestamp this clock has produced or observed.
    pub fn latest(&self) -> HlcTimestamp {
        self.latest
    }

    /// Generate a new timestamp for a local event.
    ///
    /// Guaranteed to be strictly greater than all previously generated or
    /// received timestamps on this clock.
    pub fn now(&mut self) -> HlcTimestamp {
        let wall = wall_clock_ms();
        let millis = wall.max(self.latest.millis);

        let (millis, counter) = if millis == self.latest.millis {
            bump_counter(millis, self.latest.counter)
        } else {
            (millis, 0)
        };

        self.latest = HlcTimestamp { millis, counter };
        self.latest
    }

    /// Update the clock after receiving a remote timestamp.
    ///
    /// Returns a new local timestamp that is strictly greater than both the
    /// local clock and the remote timestamp.
    pub fn receive(&mut self, remote: HlcTimestamp) -> HlcTimestamp {
        let wall = wall_clock_ms();
        let millis = wall.max(self.latest.millis).max(remote.millis);

        let (millis, counter) = if millis == self.latest.millis && millis == remote.millis {
            bump_counter(millis, self.latest.counter.max(remote.counter))
        } else if millis == self.latest.millis {
            bump_counter(millis, self.latest.counter)
        } else if millis == remote.millis {
            bump_counter(millis, remote.counter)
        } else {
            (millis, 0)
        };

        self.latest = HlcTimestamp { millis, counter };
        self.latest
    }
}

impl Default for HLC {
    fn default() -> Self {
        Self::new()
    }
}

// ───── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_monotonic() {
        let mut clock = HLC::new();
        let t1 = clock.now();
        let t2 = clock.now();
        let t3 = clock.now();
        assert!(t1 < t2);
        assert!(t2 < t3);
    }

    #[test]
    fn receive_advances_past_remote() {
        let mut clock_a = HLC::new();
        let mut clock_b = HLC::new();

        let a1 = clock_a.now();
        let b1 = clock_b.receive(a1);

        // B's timestamp must be strictly after A's.
        assert!(b1 > a1);
    }

    #[test]
    fn receive_advances_past_local() {
        let mut clock = HLC::new();
        let local = clock.now();

        // Simulate a remote timestamp far in the future.
        let remote = HlcTimestamp {
            millis: local.millis + 100_000,
            counter: 5,
        };

        let after = clock.receive(remote);
        assert!(after > remote);
        assert!(after > local);
    }

    #[test]
    fn counter_resets_on_new_millisecond() {
        let mut clock = HLC::new();
        let t1 = clock.now();

        // Force the clock forward by 1ms so the counter resets.
        clock.latest = HlcTimestamp {
            millis: t1.millis + 1,
            counter: 0,
        };
        let t2 = clock.now();

        // Counter should be 0 or 1 (depending on whether wall clock caught up),
        // but definitely not accumulated from the previous millisecond.
        assert!(t2.counter <= 1);
    }

    #[test]
    fn timestamp_ordering_millis_first() {
        let a = HlcTimestamp {
            millis: 100,
            counter: 999,
        };
        let b = HlcTimestamp {
            millis: 101,
            counter: 0,
        };
        assert!(a < b, "higher millis wins regardless of counter");
    }

    #[test]
    fn timestamp_ordering_counter_breaks_tie() {
        let a = HlcTimestamp {
            millis: 100,
            counter: 0,
        };
        let b = HlcTimestamp {
            millis: 100,
            counter: 1,
        };
        assert!(a < b);
    }

    #[test]
    fn timestamp_serde_round_trip() {
        let ts = HlcTimestamp {
            millis: 1_700_000_000_000,
            counter: 42,
        };
        let bytes = willow_transport::pack(&ts).unwrap();
        let decoded: HlcTimestamp = willow_transport::unpack(&bytes).unwrap();
        assert_eq!(decoded, ts);
    }

    #[test]
    fn two_clocks_converge() {
        let mut clock_a = HLC::new();
        let mut clock_b = HLC::new();

        // Simulate interleaved events.
        let a1 = clock_a.now();
        let b1 = clock_b.receive(a1);
        let a2 = clock_a.receive(b1);
        let b2 = clock_b.receive(a2);

        // Each successive event should be strictly ordered.
        assert!(a1 < b1);
        assert!(b1 < a2);
        assert!(a2 < b2);
    }

    #[test]
    fn now_handles_backward_clock_drift() {
        let mut clock = HLC::new();

        // Simulate a node whose `latest` is far in the future relative to the
        // real wall clock — the wall clock has "drifted backward" from the HLC's
        // perspective.
        let future_millis = u64::MAX / 2;
        clock.latest = HlcTimestamp {
            millis: future_millis,
            counter: 0,
        };

        let before = clock.latest;
        let t = clock.now();

        // The returned timestamp must be strictly greater than the forced
        // `latest`, even though the real wall clock is far behind it.
        assert!(
            t > before,
            "now() must be monotonically greater than the previous latest \
             even when the wall clock is behind (got {t} <= {before})"
        );
    }

    #[test]
    fn now_rolls_into_next_millis_when_counter_saturates() {
        let mut clock = HLC::new();

        // Force the clock into a far-future millisecond with the counter
        // already at u32::MAX so the real wall clock stays below it.
        let future_millis = u64::MAX / 2;
        clock.latest = HlcTimestamp {
            millis: future_millis,
            counter: u32::MAX,
        };

        let t = clock.now();

        // The counter cannot go higher within the same millisecond without
        // wrapping, so we must have borrowed from the next millisecond.
        assert_eq!(t.millis, future_millis + 1);
        assert_eq!(t.counter, 0);

        // And monotonicity still holds.
        let t2 = clock.now();
        assert!(t2 > t);
    }

    #[test]
    fn receive_rolls_into_next_millis_when_counter_saturates() {
        let mut clock = HLC::new();

        let future_millis = u64::MAX / 2;
        clock.latest = HlcTimestamp {
            millis: future_millis,
            counter: u32::MAX,
        };

        // Remote is in the same millisecond with the same saturated counter.
        let remote = HlcTimestamp {
            millis: future_millis,
            counter: u32::MAX,
        };

        let t = clock.receive(remote);

        assert_eq!(t.millis, future_millis + 1);
        assert_eq!(t.counter, 0);
        assert!(t > remote);
    }

    #[test]
    fn now_near_overflow_preserves_monotonicity_across_boundary() {
        let mut clock = HLC::new();

        // Position the clock one step away from saturation in a future
        // millisecond the wall clock can't catch up with.
        let future_millis = u64::MAX / 2;
        clock.latest = HlcTimestamp {
            millis: future_millis,
            counter: u32::MAX - 1,
        };

        let t1 = clock.now(); // should land at counter = u32::MAX
        assert_eq!(t1.millis, future_millis);
        assert_eq!(t1.counter, u32::MAX);

        let t2 = clock.now(); // next tick must roll over, not wrap
        assert!(t2 > t1, "got {t2} <= {t1}");
        assert_eq!(t2.millis, future_millis + 1);
        assert_eq!(t2.counter, 0);
    }

    #[test]
    fn receive_when_local_is_ahead_of_remote() {
        let mut clock = HLC::new();

        // Force the local clock far ahead of real time.
        let large_millis = u64::MAX / 2;
        clock.latest = HlcTimestamp {
            millis: large_millis,
            counter: 10,
        };

        let local_before = clock.latest;

        // A remote timestamp with a much smaller millis value.
        let remote = HlcTimestamp {
            millis: 1_000,
            counter: 99,
        };

        let result = clock.receive(remote);

        // The result must be strictly greater than the local clock before the
        // call, even though the remote timestamp is behind.
        assert!(
            result > local_before,
            "receive() must advance past the local latest when local is ahead \
             of remote (got {result} <= {local_before})"
        );

        // A subsequent now() must also remain monotonically increasing.
        let next = clock.now();
        assert!(
            next > result,
            "now() after receive() must continue advancing (got {next} <= {result})"
        );
    }
}
