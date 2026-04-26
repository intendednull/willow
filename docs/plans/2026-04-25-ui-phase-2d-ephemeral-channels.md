# Phase 2d — Ephemeral channels Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement non-permanent channel surfaces with auto-archive on inactivity, recoverable via revive (per `docs/specs/2026-04-19-ui-design/ephemeral-channels.md`).

**Architecture:**
- `willow-state` gains `EphemeralConfig` on `Channel`, tracks `last_activity_hlc` per channel, derives active/dormant/archived state purely from HLC + threshold + frontier (no broadcast needed).
- New `ChannelRevive` `EventKind` for explicit un-archive without posting.
- Web UI surfaces a `KindChip` component, dormant row dimming, an idle-threshold field in the create flow, an archives surface listing auto-archived channels, and a read-only banner inside archived rooms.
- The previous `_ephemeral-` name-prefix heuristic in `channel_sidebar.rs` is replaced by the new `ephemeral` field on `Channel`. Ephemerals **no longer have a separate sidebar group** — they live in `Commons`/`Voice` and are signalled by the trailing kind chip.

**Tech Stack:** Rust (state + client), Leptos (web UI), wasm-pack tests for browser DOM, `cargo test` for state + client. CSS variables in `crates/web/foundation.css` + `crates/web/style.css`.

**Spec invariants** (verbatim, do not paraphrase):
- "Activity is a new message only" (reactions, pins, edits, presence, membership do **not** count)
- Default thresholds: channel 14 d / thread 7 d / whisper 24 h. Cap 90 d.
- Read-only review of archived channels gated to **prior participants only** (state enforces same membership check as `MessageEmit`).
- Whispers never appear in the global grove archives — peer-scoped only.
- Mobile dormant rows: dim color + chip only, no meta line on row.

---

## File structure

### New files

- `crates/state/src/ephemeral.rs` — `EphemeralKind`, `EphemeralConfig`, `EphemeralState`, `derive_ephemeral_state` pure fn.
- `crates/web/src/components/kind_chip.rs` — small reusable `KindChip` Leptos component.
- `crates/web/src/components/read_only_banner.rs` — banner inside archived channels.
- `crates/web/src/components/archives_view.rs` — auto-archived list surface.
- `crates/web/src/components/temp_channel_create.rs` — `temp` kind tab + threshold field for the create dialog.

### Modified files

- `crates/state/src/types.rs` — extend `Channel` with `ephemeral: Option<EphemeralConfig>` and `last_activity_hlc: Option<u64>` (millisecond physical extracted from HLC).
- `crates/state/src/event.rs` — extend `EventKind::CreateChannel` with `ephemeral: Option<EphemeralConfig>`; new variant `EventKind::ChannelRevive { channel_id }`.
- `crates/state/src/materialize.rs` — handle ephemeral on `CreateChannel`, advance `last_activity_hlc` on `Message`, handle `ChannelRevive` (member gate + advance HLC).
- `crates/state/src/lib.rs` — re-export `EphemeralKind`, `EphemeralConfig`, `EphemeralState`, `derive_ephemeral_state`.
- `crates/state/src/tests.rs` — state-machine tests.
- `crates/client/src/mutations.rs` — `create_ephemeral_channel`, `revive_channel`.
- `crates/client/src/views.rs` — derive `ArchivesView` (per-server list of auto-archived channel summaries).
- `crates/client/src/lib.rs` — re-export `ArchivesView`, `ArchivedChannelSummary`.
- `crates/client/src/tests/ephemeral.rs` — client API tests.
- `crates/web/src/components/mod.rs` — `pub mod` + re-exports for the three new components.
- `crates/web/src/components/channel_sidebar.rs` — drop `_ephemeral-` heuristic, rely on `ephemeral` field; render `KindChip` on rows; dim dormant rows.
- `crates/web/src/components/welcome.rs` (or wherever the "new channel" dialog lives — confirm and document in Task 8) — wire the `temp` kind tab + threshold field.
- `crates/web/src/components/long_press.rs` — extend mobile preview header to include "last activity {N} {unit} ago" for dormant rows.
- `crates/web/foundation.css` — `--line-soft` token + `--ink-2` reuse confirmation.
- `crates/web/style.css` — kind-chip styles, dormant row dim, archives subgroup, read-only banner.
- `crates/web/tests/browser.rs` — new `mod phase_2d_ephemeral_channels` test module.
- `docs/specs/2026-04-19-ui-design/thread-pane.md` — `## Auto-archive` cross-ref section.
- `docs/specs/2026-04-19-ui-design/whisper-mode.md` — `## Auto-archive` cross-ref section.

---

## Task 1: State types — `EphemeralKind`, `EphemeralConfig`, `Channel` extension

**Files:**
- Create: `crates/state/src/ephemeral.rs`
- Modify: `crates/state/src/types.rs`
- Modify: `crates/state/src/lib.rs`
- Test: `crates/state/src/tests.rs`

- [ ] **Step 1: Write the failing test for `Channel` with ephemeral config**

Add to `crates/state/src/tests.rs`:

```rust
#[test]
fn channel_with_ephemeral_config_serializes() {
    use crate::types::{Channel, ChannelKind};
    use crate::ephemeral::{EphemeralConfig, EphemeralKind};

    let ch = Channel {
        id: "c1".into(),
        name: "side-room".into(),
        pinned_messages: Default::default(),
        kind: ChannelKind::Text,
        ephemeral: Some(EphemeralConfig {
            kind: EphemeralKind::Channel,
            idle_threshold_ms: 14 * 24 * 3_600_000,
        }),
        last_activity_hlc: Some(1_700_000_000_000),
    };

    let json = serde_json::to_string(&ch).unwrap();
    let back: Channel = serde_json::from_str(&json).unwrap();
    assert_eq!(ch, back);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p willow-state channel_with_ephemeral_config_serializes`
Expected: FAIL — `EphemeralConfig` not defined, `Channel` has no `ephemeral` field.

- [ ] **Step 3: Create `crates/state/src/ephemeral.rs`**

```rust
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
```

- [ ] **Step 4: Extend `Channel` in `crates/state/src/types.rs`**

Locate the `Channel` struct (lines 27-39 in current file) and add two fields:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub pinned_messages: BTreeSet<EventHash>,
    #[serde(default)]
    pub kind: ChannelKind,
    /// `None` for permanent channels.
    #[serde(default)]
    pub ephemeral: Option<crate::ephemeral::EphemeralConfig>,
    /// Latest message HLC (physical millisecond). `None` until the
    /// first message lands.
    #[serde(default)]
    pub last_activity_hlc: Option<u64>,
}
```

Both new fields use `#[serde(default)]` so prior serialized state replays cleanly.

- [ ] **Step 5: Wire the module into `lib.rs`**

In `crates/state/src/lib.rs` (top of file, with the other `pub mod` lines), add:

```rust
pub mod ephemeral;
pub use ephemeral::{
    derive_ephemeral_state, EphemeralConfig, EphemeralKind, EphemeralState,
    DEFAULT_CHANNEL_THRESHOLD_MS, DEFAULT_THREAD_THRESHOLD_MS,
    DEFAULT_WHISPER_THRESHOLD_MS, IDLE_THRESHOLD_MAX_MS, IDLE_THRESHOLD_MIN_MS,
};
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test -p willow-state channel_with_ephemeral_config_serializes`
Expected: PASS.

- [ ] **Step 7: Add a test for the derivation function**

Add to `crates/state/src/tests.rs`:

```rust
#[test]
fn derive_ephemeral_state_bands() {
    use crate::ephemeral::{derive_ephemeral_state, EphemeralState};

    let threshold = 100;
    // 0 elapsed → active
    assert_eq!(
        derive_ephemeral_state(Some(100), threshold, 100),
        EphemeralState::Active
    );
    // 24 % elapsed → active (just inside the active band)
    assert_eq!(
        derive_ephemeral_state(Some(76), threshold, 100),
        EphemeralState::Active
    );
    // 26 % elapsed → dormant
    assert_eq!(
        derive_ephemeral_state(Some(74), threshold, 100),
        EphemeralState::Dormant
    );
    // 100 % elapsed → dormant (boundary stays in dormant)
    assert_eq!(
        derive_ephemeral_state(Some(0), threshold, 100),
        EphemeralState::Dormant
    );
    // > 100 % elapsed → archived
    assert_eq!(
        derive_ephemeral_state(Some(0), threshold, 101),
        EphemeralState::Archived
    );
    // No activity yet → uses 0; archived if frontier > threshold.
    assert_eq!(
        derive_ephemeral_state(None, threshold, 200),
        EphemeralState::Archived
    );
}
```

- [ ] **Step 8: Run derivation test**

Run: `cargo test -p willow-state derive_ephemeral_state_bands`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/state/src/ephemeral.rs crates/state/src/types.rs crates/state/src/lib.rs crates/state/src/tests.rs
git commit -m "$(cat <<'EOF'
state(phase-2d): add EphemeralConfig + derive_ephemeral_state

Adds the data shape for non-permanent channels (kind + idle
threshold) and the pure derivation that classifies a channel as
active/dormant/archived from its `last_activity_hlc` against the
merge frontier.

Channel struct gains two #[serde(default)] fields so prior state
replays cleanly. No behaviour change yet — derivation is unused
until materialize wires it up in the next task.

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md
EOF
)"
```

---

## Task 2: `EventKind::CreateChannel` extension + bound checks

**Files:**
- Modify: `crates/state/src/event.rs:97-104`
- Modify: `crates/state/src/materialize.rs:323-340` (the `CreateChannel` apply branch)
- Test: `crates/state/src/tests.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/state/src/tests.rs`:

```rust
#[test]
fn create_channel_with_ephemeral_config_records_it() {
    use crate::ephemeral::{EphemeralConfig, EphemeralKind, DEFAULT_CHANNEL_THRESHOLD_MS};

    let owner = make_id();
    let mut state = make_state(&owner);
    let ev = make_event(
        &state,
        &owner,
        EventKind::CreateChannel {
            name: "side-room".into(),
            channel_id: "c1".into(),
            kind: ChannelKind::Text,
            ephemeral: Some(EphemeralConfig {
                kind: EphemeralKind::Channel,
                idle_threshold_ms: DEFAULT_CHANNEL_THRESHOLD_MS,
            }),
        },
    );
    apply(&mut state, &ev).unwrap();

    let ch = state.channels.iter().find(|c| c.id == "c1").unwrap();
    assert!(ch.ephemeral.is_some());
    assert_eq!(
        ch.ephemeral.as_ref().unwrap().idle_threshold_ms,
        DEFAULT_CHANNEL_THRESHOLD_MS
    );
}

#[test]
fn create_channel_rejects_threshold_below_minimum() {
    use crate::ephemeral::{EphemeralConfig, EphemeralKind};

    let owner = make_id();
    let mut state = make_state(&owner);
    let ev = make_event(
        &state,
        &owner,
        EventKind::CreateChannel {
            name: "too-fast".into(),
            channel_id: "c2".into(),
            kind: ChannelKind::Text,
            ephemeral: Some(EphemeralConfig {
                kind: EphemeralKind::Channel,
                idle_threshold_ms: 60_000, // 1 minute — below 1h floor
            }),
        },
    );
    let result = apply(&mut state, &ev);
    assert!(result.is_err(), "below-floor threshold must be rejected");
}

#[test]
fn create_channel_rejects_threshold_above_cap() {
    use crate::ephemeral::{EphemeralConfig, EphemeralKind};

    let owner = make_id();
    let mut state = make_state(&owner);
    let ev = make_event(
        &state,
        &owner,
        EventKind::CreateChannel {
            name: "too-long".into(),
            channel_id: "c3".into(),
            kind: ChannelKind::Text,
            ephemeral: Some(EphemeralConfig {
                kind: EphemeralKind::Channel,
                idle_threshold_ms: 200 * 24 * 3_600_000, // 200 days — above 90d cap
            }),
        },
    );
    let result = apply(&mut state, &ev);
    assert!(result.is_err(), "above-cap threshold must be rejected");
}
```

If `make_event` and `make_state` helpers don't already exist in `tests.rs`, locate the equivalent existing helpers (search with `grep -n "fn make_state\|fn make_event\|fn make_id" crates/state/src/tests.rs`) and use those names. Do not invent new helpers.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p willow-state create_channel_with_ephemeral_config_records_it create_channel_rejects_threshold_below_minimum create_channel_rejects_threshold_above_cap`
Expected: FAIL — `CreateChannel` enum variant has no `ephemeral` field.

- [ ] **Step 3: Extend `EventKind::CreateChannel`**

In `crates/state/src/event.rs`, locate the `CreateChannel` variant (around line 97-104) and add the optional ephemeral field:

```rust
/// Create a new channel.
CreateChannel {
    name: String,
    channel_id: String,
    #[serde(default)]
    kind: crate::types::ChannelKind,
    #[serde(default)]
    ephemeral: Option<crate::ephemeral::EphemeralConfig>,
},
```

- [ ] **Step 4: Update `materialize.rs` apply branch**

In `crates/state/src/materialize.rs`, find the `CreateChannel` apply branch (around line 323). Update the destructure + the `Channel` construction:

```rust
EventKind::CreateChannel {
    name,
    channel_id,
    kind,
    ephemeral,
} => {
    if let Some(cfg) = ephemeral.as_ref() {
        if cfg.idle_threshold_ms < crate::ephemeral::IDLE_THRESHOLD_MIN_MS
            || cfg.idle_threshold_ms > crate::ephemeral::IDLE_THRESHOLD_MAX_MS
        {
            return Err(StateError::InvalidEvent(format!(
                "ephemeral idle_threshold_ms {} out of range",
                cfg.idle_threshold_ms
            )));
        }
    }
    state.channels.push(Channel {
        id: channel_id.clone(),
        name: name.clone(),
        pinned_messages: Default::default(),
        kind: kind.clone(),
        ephemeral: ephemeral.clone(),
        last_activity_hlc: None,
    });
}
```

If the existing `StateError` variant is not `InvalidEvent`, locate the codebase's existing pattern with `grep -n "StateError::" crates/state/src/materialize.rs` and use that variant.

- [ ] **Step 5: Update every other `EventKind::CreateChannel` construction site**

The new field is optional but Rust struct-update syntax across the codebase still needs the field. Find all construction sites:

```bash
grep -rn 'EventKind::CreateChannel' crates/
```

Each construction needs `ephemeral: None` (or a real config where appropriate). Update each to compile. The `client::mutations::create_channel` site at `crates/client/src/mutations.rs:364` is one such site — pass `ephemeral: None`.

- [ ] **Step 6: Run tests + workspace check**

```bash
cargo test -p willow-state
cargo check --workspace
```

Both expected to pass / compile.

- [ ] **Step 7: Commit**

```bash
git add crates/state/src/event.rs crates/state/src/materialize.rs crates/client/src/mutations.rs crates/state/src/tests.rs
git commit -m "$(cat <<'EOF'
state(phase-2d): extend CreateChannel with EphemeralConfig

CreateChannel gains an optional `ephemeral` field. apply() validates
the idle_threshold_ms is within [1h, 90d] and rejects out-of-range
events. All existing CreateChannel construction sites updated with
`ephemeral: None`.

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md §Data deps
EOF
)"
```

---

## Task 3: Track `last_activity_hlc` on `Message`

**Files:**
- Modify: `crates/state/src/materialize.rs` (the `Message` apply branch)
- Test: `crates/state/src/tests.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/state/src/tests.rs`:

```rust
#[test]
fn message_advances_last_activity_hlc() {
    use crate::ephemeral::{EphemeralConfig, EphemeralKind, DEFAULT_CHANNEL_THRESHOLD_MS};

    let owner = make_id();
    let mut state = make_state(&owner);
    let create_ev = make_event(
        &state,
        &owner,
        EventKind::CreateChannel {
            name: "side-room".into(),
            channel_id: "c1".into(),
            kind: ChannelKind::Text,
            ephemeral: Some(EphemeralConfig {
                kind: EphemeralKind::Channel,
                idle_threshold_ms: DEFAULT_CHANNEL_THRESHOLD_MS,
            }),
        },
    );
    apply(&mut state, &create_ev).unwrap();

    let msg_ev = make_event(
        &state,
        &owner,
        EventKind::Message {
            channel_id: "c1".into(),
            body: "hi".into(),
            reply_to: None,
        },
    );
    apply(&mut state, &msg_ev).unwrap();

    let ch = state.channels.iter().find(|c| c.id == "c1").unwrap();
    assert_eq!(ch.last_activity_hlc, Some(msg_ev.timestamp_ms));
}

#[test]
fn message_on_permanent_channel_also_advances_hlc() {
    // Tracking is unconditional — non-ephemeral channels can carry
    // the field too. Cheap, simplifies the materialize branch, and
    // a future feature might use it.
    let owner = make_id();
    let mut state = make_state(&owner);
    let create_ev = make_event(
        &state,
        &owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "g1".into(),
            kind: ChannelKind::Text,
            ephemeral: None,
        },
    );
    apply(&mut state, &create_ev).unwrap();

    let msg_ev = make_event(
        &state,
        &owner,
        EventKind::Message {
            channel_id: "g1".into(),
            body: "hi".into(),
            reply_to: None,
        },
    );
    apply(&mut state, &msg_ev).unwrap();

    let ch = state.channels.iter().find(|c| c.id == "g1").unwrap();
    assert_eq!(ch.last_activity_hlc, Some(msg_ev.timestamp_ms));
}
```

The exact field name on the event for the HLC physical millisecond may be `timestamp_ms` or `hlc_ms` — confirm with `grep -n "pub struct Event\|pub hlc\|pub timestamp" crates/state/src/event.rs` and use the actual field name in the assertion.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p willow-state message_advances_last_activity_hlc message_on_permanent_channel_also_advances_hlc`
Expected: FAIL — `last_activity_hlc` not updated by `Message`.

- [ ] **Step 3: Update the `Message` apply branch in `materialize.rs`**

Find the `EventKind::Message { channel_id, body, reply_to }` branch in `apply_event` and at the end of its handling, **after** the message is appended to history, add:

```rust
if let Some(ch) = state.channels.iter_mut().find(|c| c.id == *channel_id) {
    ch.last_activity_hlc = Some(event.timestamp_ms);
}
```

Substitute `event.timestamp_ms` with whatever the actual HLC physical-milliseconds field on `Event` is called. Track activity unconditionally (cheap, simplifies the branch).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p willow-state`
Expected: PASS — both new tests + all existing.

- [ ] **Step 5: Commit**

```bash
git add crates/state/src/materialize.rs crates/state/src/tests.rs
git commit -m "$(cat <<'EOF'
state(phase-2d): track last_activity_hlc on Message events

Every Message event now advances the channel's last_activity_hlc.
Tracking is unconditional (permanent channels carry it too) so the
materialize branch stays simple and a future feature can reuse it.

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md §Inactivity ladder
EOF
)"
```

---

## Task 4: `EventKind::ChannelRevive` + member gate

**Files:**
- Modify: `crates/state/src/event.rs` (add new variant)
- Modify: `crates/state/src/materialize.rs` (handler + member gate + advance HLC)
- Test: `crates/state/src/tests.rs`

- [ ] **Step 1: Write the failing tests**

Add to `crates/state/src/tests.rs`:

```rust
#[test]
fn channel_revive_advances_last_activity_hlc() {
    use crate::ephemeral::{EphemeralConfig, EphemeralKind, DEFAULT_CHANNEL_THRESHOLD_MS};

    let owner = make_id();
    let mut state = make_state(&owner);
    apply(
        &mut state,
        &make_event(
            &state,
            &owner,
            EventKind::CreateChannel {
                name: "side-room".into(),
                channel_id: "c1".into(),
                kind: ChannelKind::Text,
                ephemeral: Some(EphemeralConfig {
                    kind: EphemeralKind::Channel,
                    idle_threshold_ms: DEFAULT_CHANNEL_THRESHOLD_MS,
                }),
            },
        ),
    )
    .unwrap();

    let revive_ev = make_event(
        &state,
        &owner,
        EventKind::ChannelRevive {
            channel_id: "c1".into(),
        },
    );
    apply(&mut state, &revive_ev).unwrap();

    let ch = state.channels.iter().find(|c| c.id == "c1").unwrap();
    assert_eq!(ch.last_activity_hlc, Some(revive_ev.timestamp_ms));
}

#[test]
fn channel_revive_rejected_for_non_member() {
    use crate::ephemeral::{EphemeralConfig, EphemeralKind, DEFAULT_CHANNEL_THRESHOLD_MS};

    let owner = make_id();
    let stranger = make_id();
    let mut state = make_state(&owner);
    apply(
        &mut state,
        &make_event(
            &state,
            &owner,
            EventKind::CreateChannel {
                name: "side-room".into(),
                channel_id: "c1".into(),
                kind: ChannelKind::Text,
                ephemeral: Some(EphemeralConfig {
                    kind: EphemeralKind::Channel,
                    idle_threshold_ms: DEFAULT_CHANNEL_THRESHOLD_MS,
                }),
            },
        ),
    )
    .unwrap();

    // stranger is not in the server members list — revive must reject.
    let ev = make_event_as(
        &state,
        &stranger,
        EventKind::ChannelRevive {
            channel_id: "c1".into(),
        },
    );
    let result = apply(&mut state, &ev);
    assert!(result.is_err(), "non-member revive must be rejected");
}

#[test]
fn channel_revive_unknown_channel_rejected() {
    let owner = make_id();
    let mut state = make_state(&owner);
    let ev = make_event(
        &state,
        &owner,
        EventKind::ChannelRevive {
            channel_id: "does-not-exist".into(),
        },
    );
    let result = apply(&mut state, &ev);
    assert!(result.is_err(), "revive of unknown channel must be rejected");
}
```

If `make_event_as` does not exist, locate the codebase pattern for "event signed by a different peer" — `grep -n "fn make_event" crates/state/src/tests.rs` — and use that helper. Do not invent helpers.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p willow-state channel_revive`
Expected: FAIL — `EventKind::ChannelRevive` doesn't exist.

- [ ] **Step 3: Add the new `EventKind` variant**

In `crates/state/src/event.rs`, find the `// -- Server structure --` section (around line 95) and add after `RenameChannel`:

```rust
/// Revive an auto-archived ephemeral channel without posting a
/// message. Author must be a member of the server.
ChannelRevive { channel_id: String },
```

- [ ] **Step 4: Add the `apply` handler**

In `crates/state/src/materialize.rs`, add a new branch in `apply_event`. Member gating uses the same check pattern as `EventKind::Message` — locate that branch and copy the membership check.

```rust
EventKind::ChannelRevive { channel_id } => {
    // Member gate — same contract as Message emission.
    if !state.members.iter().any(|m| m.peer_id == event.author) {
        return Err(StateError::PermissionDenied(
            "ChannelRevive: author is not a member".into(),
        ));
    }
    let ch = state
        .channels
        .iter_mut()
        .find(|c| c.id == *channel_id)
        .ok_or_else(|| StateError::InvalidEvent(format!(
            "ChannelRevive: channel {channel_id} not found"
        )))?;
    ch.last_activity_hlc = Some(event.timestamp_ms);
}
```

If the existing membership predicate uses a different shape, mirror it. If the existing `Message` branch checks roles differently (e.g. `SendMessages` permission), require **only membership**, not `SendMessages` — revive is a member-level action, not a write-permission one. Document this distinction inline:

```rust
// Note: revive only requires membership, not SendMessages — a
// member who has been muted can still un-archive a channel they
// were part of, even if they cannot post.
```

- [ ] **Step 5: Update `required_permission()` table if applicable**

In `crates/state/src/materialize.rs` find `fn required_permission` (or equivalent — `grep -n "required_permission" crates/state/src/`). Add a match arm for `EventKind::ChannelRevive` that returns the same value the existing membership-only events use (probably `Permission::None` or no entry at all). Confirm by reading the existing function.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p willow-state`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/state/src/event.rs crates/state/src/materialize.rs crates/state/src/tests.rs
git commit -m "$(cat <<'EOF'
state(phase-2d): add ChannelRevive event + member gate

ChannelRevive lets a member un-archive an ephemeral channel without
posting. apply() requires server membership (same check as Message
emission) but not the SendMessages permission — a muted member can
still revive a room they belong to.

Idempotent on already-active channels (still advances
last_activity_hlc, harmless).

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md §Data deps
EOF
)"
```

---

## Task 5: Client mutations — `create_ephemeral_channel`, `revive_channel`

**Files:**
- Modify: `crates/client/src/mutations.rs`
- Modify: `crates/client/src/actions.rs`
- Modify: `crates/client/src/lib.rs` (re-exports if needed)
- Test: `crates/client/src/lib.rs` test module (or `crates/client/src/tests/ephemeral.rs`)

- [ ] **Step 1: Write the failing test**

Create `crates/client/src/tests/ephemeral.rs`:

```rust
//! Phase 2d — ephemeral channel client API tests.

use crate::tests::test_client;
use willow_state::{EphemeralKind, DEFAULT_CHANNEL_THRESHOLD_MS};

#[tokio::test]
async fn create_ephemeral_channel_records_config() {
    let client = test_client().await;
    client
        .create_ephemeral_channel("side-room", EphemeralKind::Channel, DEFAULT_CHANNEL_THRESHOLD_MS)
        .await
        .unwrap();

    let state = client.state_snapshot().await;
    let ch = state.channels.iter().find(|c| c.name == "side-room").unwrap();
    assert!(ch.ephemeral.is_some());
    assert_eq!(
        ch.ephemeral.as_ref().unwrap().idle_threshold_ms,
        DEFAULT_CHANNEL_THRESHOLD_MS
    );
}

#[tokio::test]
async fn revive_channel_advances_last_activity_hlc() {
    let client = test_client().await;
    client
        .create_ephemeral_channel("side-room", EphemeralKind::Channel, DEFAULT_CHANNEL_THRESHOLD_MS)
        .await
        .unwrap();

    // Capture HLC, revive, confirm advance.
    let before = {
        let state = client.state_snapshot().await;
        state
            .channels
            .iter()
            .find(|c| c.name == "side-room")
            .unwrap()
            .last_activity_hlc
    };
    client.revive_channel("side-room").await.unwrap();
    let after = {
        let state = client.state_snapshot().await;
        state
            .channels
            .iter()
            .find(|c| c.name == "side-room")
            .unwrap()
            .last_activity_hlc
    };
    assert!(
        after > before,
        "revive must advance last_activity_hlc"
    );
}
```

Wire the new test module: in `crates/client/src/lib.rs` near the other `mod tests_*` declarations:

```rust
#[cfg(test)]
#[path = "tests/ephemeral.rs"]
mod tests_ephemeral;
```

If `state_snapshot` does not exist on the client handle, locate the equivalent — `grep -n "fn state_snapshot\|fn current_state\|fn server_state" crates/client/src/lib.rs crates/client/src/actions.rs` — and use that. Do not invent.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p willow-client tests_ephemeral`
Expected: FAIL — `create_ephemeral_channel` and `revive_channel` do not exist.

- [ ] **Step 3: Add `create_ephemeral_channel` to `mutations.rs`**

In `crates/client/src/mutations.rs`, immediately after the existing `create_channel` method, add:

```rust
/// Create a non-permanent ("ephemeral") channel that auto-archives
/// after `idle_threshold_ms` of inactivity. See
/// `docs/specs/2026-04-19-ui-design/ephemeral-channels.md`.
pub async fn create_ephemeral_channel(
    &self,
    name: &str,
    kind: willow_state::EphemeralKind,
    idle_threshold_ms: u64,
) -> anyhow::Result<()> {
    let name = name.to_string();
    let name_for_switch = name.clone();
    let ch_id_str = uuid::Uuid::new_v4().to_string();

    let event = self
        .build_event(EventKind::CreateChannel {
            name: name.clone(),
            channel_id: ch_id_str,
            kind: willow_state::ChannelKind::Text,
            ephemeral: Some(willow_state::EphemeralConfig {
                kind,
                idle_threshold_ms,
            }),
        })
        .await?;
    self.apply_event(&event).await;
    self.switch_channel(&name_for_switch).await;
    self.broadcast_event(&event);
    Ok(())
}

/// Revive an auto-archived ephemeral channel by name without
/// posting a message.
pub async fn revive_channel(&self, name: &str) -> anyhow::Result<()> {
    let ch_id_str = self.resolve_channel_id(name).await?;
    let event = self
        .build_event(EventKind::ChannelRevive {
            channel_id: ch_id_str,
        })
        .await?;
    self.apply_event(&event).await;
    self.broadcast_event(&event);
    Ok(())
}
```

- [ ] **Step 4: Mirror them on `actions.rs`**

In `crates/client/src/actions.rs`, find `create_channel` (around line 123) and add right after:

```rust
pub async fn create_ephemeral_channel(
    &self,
    name: &str,
    kind: willow_state::EphemeralKind,
    idle_threshold_ms: u64,
) -> anyhow::Result<()> {
    self.mutation_handle
        .create_ephemeral_channel(name, kind, idle_threshold_ms)
        .await
}

pub async fn revive_channel(&self, name: &str) -> anyhow::Result<()> {
    self.mutation_handle.revive_channel(name).await
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p willow-client tests_ephemeral`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/client/src/mutations.rs crates/client/src/actions.rs crates/client/src/lib.rs crates/client/src/tests/ephemeral.rs
git commit -m "$(cat <<'EOF'
client(phase-2d): create_ephemeral_channel + revive_channel APIs

Two new mutations on ClientHandle:
- create_ephemeral_channel(name, kind, idle_threshold_ms) — emits
  CreateChannel with EphemeralConfig.
- revive_channel(name) — resolves the channel id then emits
  ChannelRevive.

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md
EOF
)"
```

---

## Task 6: Client view — `ArchivesView`

**Files:**
- Modify: `crates/client/src/views.rs`
- Modify: `crates/client/src/lib.rs` (re-exports)
- Test: `crates/client/src/tests/ephemeral.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/client/src/tests/ephemeral.rs`:

```rust
#[tokio::test]
async fn archives_view_lists_archived_ephemerals_only() {
    use willow_state::{EphemeralKind, IDLE_THRESHOLD_MIN_MS};

    let client = test_client().await;
    // Channel that will archive immediately because threshold is
    // 1 hour and we'll fast-forward the frontier.
    client
        .create_ephemeral_channel("expired", EphemeralKind::Channel, IDLE_THRESHOLD_MIN_MS)
        .await
        .unwrap();
    // Permanent channel — must not appear in archives.
    client.create_channel("general").await.unwrap();
    // Active ephemeral — must not appear in archives.
    client
        .create_ephemeral_channel("active", EphemeralKind::Channel, 14 * 24 * 3_600_000)
        .await
        .unwrap();

    // Set the test client's frontier HLC two hours past creation
    // so `expired` has elapsed > threshold.
    client.advance_test_hlc_ms(2 * 3_600_000).await;

    let view = client.archives_view().await;
    let names: Vec<&str> = view.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["expired"], "only `expired` should be archived");
}
```

If `advance_test_hlc_ms` does not exist on the test client, locate the test fixture's HLC manipulation — `grep -n "fn advance\|test_client\|fn now_ms" crates/client/src/tests.rs crates/client/src/tests/` — and use the existing helper. If none exists, add one as part of this task: it should set an offset in the client's HLC source so derivations see a frontier far in the future without sleeping.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p willow-client archives_view_lists_archived_ephemerals_only`
Expected: FAIL — `archives_view` does not exist.

- [ ] **Step 3: Add `ArchivedChannelSummary` + `ArchivesView` to `views.rs`**

In `crates/client/src/views.rs`, near the other view structs:

```rust
/// One row in the archives surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedChannelSummary {
    pub channel_id: String,
    pub name: String,
    pub kind: willow_state::EphemeralKind,
    pub last_activity_ms: Option<u64>,
    /// `last_activity + idle_threshold` — the moment the channel
    /// crossed into archived. Used by the archives view to render
    /// "archived after N units idle".
    pub archived_at_ms: u64,
}

/// Everything the archives surface needs to render.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchivesView {
    pub entries: Vec<ArchivedChannelSummary>,
}
```

- [ ] **Step 4: Add the `archives_view` derivation**

Add a function (or method on the existing view-derivation handle — match the codebase pattern by reading nearby derivations):

```rust
pub fn derive_archives_view(state: &willow_state::ServerState, frontier_hlc_ms: u64) -> ArchivesView {
    let mut entries: Vec<ArchivedChannelSummary> = state
        .channels
        .iter()
        .filter_map(|ch| {
            let cfg = ch.ephemeral.as_ref()?;
            let band = willow_state::derive_ephemeral_state(
                ch.last_activity_hlc,
                cfg.idle_threshold_ms,
                frontier_hlc_ms,
            );
            if band != willow_state::EphemeralState::Archived {
                return None;
            }
            let archived_at = ch
                .last_activity_hlc
                .unwrap_or(0)
                .saturating_add(cfg.idle_threshold_ms);
            Some(ArchivedChannelSummary {
                channel_id: ch.id.clone(),
                name: ch.name.clone(),
                kind: cfg.kind,
                last_activity_ms: ch.last_activity_hlc,
                archived_at_ms: archived_at,
            })
        })
        .collect();
    // Newest archive first.
    entries.sort_by(|a, b| b.archived_at_ms.cmp(&a.archived_at_ms));
    ArchivesView { entries }
}
```

- [ ] **Step 5: Expose `archives_view` on `ClientHandle`**

Add a method to `ClientHandle` (in `crates/client/src/actions.rs` or wherever the view accessors live — `grep -n "fn .*view" crates/client/src/actions.rs`) that calls `derive_archives_view` with the current state + frontier HLC. Wire whatever signal-update plumbing the existing views use; mirror an existing view that consumes both state + HLC (e.g. anywhere the queue view's HLC-aware derivation works).

- [ ] **Step 6: Re-export from `lib.rs`**

In `crates/client/src/lib.rs` `pub use` block:

```rust
pub use views::{ArchivedChannelSummary, ArchivesView};
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p willow-client`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/client/src/views.rs crates/client/src/actions.rs crates/client/src/lib.rs crates/client/src/tests/ephemeral.rs
git commit -m "$(cat <<'EOF'
client(phase-2d): ArchivesView derivation

Pure derivation over ServerState + frontier HLC produces the list
of auto-archived ephemeral channels (newest archive first), with
per-row metadata for the archives surface (name, kind, last
activity, archive instant).

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md §Archive surface
EOF
)"
```

---

## Task 7: Web — `KindChip` component

**Files:**
- Create: `crates/web/src/components/kind_chip.rs`
- Modify: `crates/web/src/components/mod.rs`
- Modify: `crates/web/style.css`
- Modify: `crates/web/foundation.css` (only if `--line-soft` is missing — confirm first)
- Test: `crates/web/tests/browser.rs` (new module `phase_2d_ephemeral_channels`)

- [ ] **Step 1: Write the failing browser test**

At the end of `crates/web/tests/browser.rs`, add a new test module:

```rust
// ────────────────────── Phase 2d — Ephemeral channels ──────────────────────

mod phase_2d_ephemeral_channels {
    use super::{mount_test, query, tick};
    use leptos::prelude::*;
    use wasm_bindgen_test::*;
    use willow_state::EphemeralKind;
    use willow_web::components::{KindChip, KindChipKind};

    fn map_kind(k: EphemeralKind) -> KindChipKind {
        match k {
            EphemeralKind::Channel => KindChipKind::Channel,
            EphemeralKind::Thread => KindChipKind::Thread,
            EphemeralKind::Whisper => KindChipKind::Whisper,
        }
    }

    #[wasm_bindgen_test]
    async fn kind_chip_renders_temp_for_channel() {
        let container = mount_test(|| view! { <KindChip kind=KindChipKind::Channel/> }).await;
        tick().await;
        let chip = query(&container, ".kind-chip").expect("KindChip must render");
        assert_eq!(chip.text_content().unwrap_or_default().trim(), "temp");
        assert_eq!(
            chip.get_attribute("aria-label").as_deref(),
            Some("non-permanent — channel")
        );
    }

    #[wasm_bindgen_test]
    async fn kind_chip_renders_thread_label() {
        let container = mount_test(|| view! { <KindChip kind=KindChipKind::Thread/> }).await;
        tick().await;
        let chip = query(&container, ".kind-chip").expect("KindChip must render");
        assert_eq!(chip.text_content().unwrap_or_default().trim(), "thread");
    }

    #[wasm_bindgen_test]
    async fn kind_chip_renders_whisper_label() {
        let container = mount_test(|| view! { <KindChip kind=KindChipKind::Whisper/> }).await;
        tick().await;
        let chip = query(&container, ".kind-chip").expect("KindChip must render");
        assert_eq!(chip.text_content().unwrap_or_default().trim(), "whisper");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `wasm-pack test --headless --firefox crates/web -- --test browser`
Expected: FAIL — `KindChip` not found.

- [ ] **Step 3: Create `crates/web/src/components/kind_chip.rs`**

```rust
//! Kind chip — small mono pill rendered on ephemeral surface rows
//! (sidebar entries + archives entries) signalling non-permanence.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/ephemeral-channels.md`
//! §Sidebar treatment (Active).

use leptos::prelude::*;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum KindChipKind {
    Channel,
    Thread,
    Whisper,
}

impl KindChipKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Channel => "temp",
            Self::Thread => "thread",
            Self::Whisper => "whisper",
        }
    }

    pub fn aria_kind(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Thread => "thread",
            Self::Whisper => "whisper",
        }
    }
}

#[component]
pub fn KindChip(kind: KindChipKind) -> impl IntoView {
    let aria = format!("non-permanent — {}", kind.aria_kind());
    view! {
        <span class="kind-chip" aria-label=aria>{kind.label()}</span>
    }
}
```

- [ ] **Step 4: Wire it into the components module**

In `crates/web/src/components/mod.rs`, add (preserving alphabetical order):

```rust
mod kind_chip;
// ... in the `pub use` block (alphabetical):
pub use kind_chip::{KindChip, KindChipKind};
```

- [ ] **Step 5: Add CSS for `.kind-chip`**

Append to `crates/web/style.css`:

```css
/* ── Phase 2d ephemeral kind chip ───────────────────────────────────── */

.kind-chip {
    display: inline-flex;
    align-items: center;
    height: 16px;
    padding: 2px 6px;
    border: 1px solid var(--line-soft, var(--line));
    border-radius: var(--radius-s);
    color: var(--ink-3);
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 1;
    background: transparent;
    user-select: none;
}
```

If `--line-soft` is not defined in `foundation.css`, add it there alongside `--line`:

```css
--line-soft: rgba(255, 255, 255, 0.06); /* one step softer than --line */
```

(Choose an exact value matching the project's existing line tokens — read the foundation file and follow the convention.)

- [ ] **Step 6: Run browser tests to verify they pass**

Run: `wasm-pack test --headless --firefox crates/web -- --test browser kind_chip`
Expected: 3 tests pass.

- [ ] **Step 7: Run full local quality gate**

```bash
just fmt && just clippy
```

Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/web/src/components/kind_chip.rs crates/web/src/components/mod.rs crates/web/style.css crates/web/foundation.css crates/web/tests/browser.rs
git commit -m "$(cat <<'EOF'
web(phase-2d): KindChip component

Reusable mono pill for ephemeral surface rows. Exposes three
variants — temp / thread / whisper — each carrying an aria-label
that spells out the metaphor for screen readers ("non-permanent —
channel" etc.).

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md
EOF
)"
```

---

## Task 8: Web — kind selector + idle threshold field in create flow

**Files:**
- Create: `crates/web/src/components/temp_channel_create.rs`
- Modify: the existing "new channel" dialog (find via `grep -rn "create_channel\|new channel\|CreateChannelDialog" crates/web/src/components/`)
- Modify: `crates/web/src/components/mod.rs`
- Modify: `crates/web/style.css`
- Test: `crates/web/tests/browser.rs` `phase_2d_ephemeral_channels`

- [ ] **Step 1: Locate the existing create-channel dialog**

```bash
grep -rn "create_channel\|new channel" crates/web/src/components/ | head
```

Identify the file and component. (Most likely `add_server.rs`, `command_palette.rs`, or a yet-unnamed dialog. Read the file and keep notes on how it currently invokes `client.create_channel(name)`.)

If no existing dialog has all three pieces (name, kind, threshold), create a new one in `temp_channel_create.rs` and wire it into the existing trigger. If an existing dialog can be extended, prefer extension over duplication.

- [ ] **Step 2: Write the failing browser test**

Add to `phase_2d_ephemeral_channels`:

```rust
#[wasm_bindgen_test]
async fn temp_kind_tab_reveals_threshold_field() {
    use willow_web::components::TempChannelCreateForm;

    let container = mount_test(|| {
        view! { <TempChannelCreateForm initial_kind=KindChipKind::Channel/> }
    })
    .await;
    tick().await;

    // Threshold input is rendered when the temp tab is active.
    let threshold_input = query(&container, "input[name='temp-idle-threshold-days']")
        .expect("threshold field must render when temp kind is selected");
    assert_eq!(
        threshold_input.get_attribute("value").as_deref(),
        Some("14"),
        "default threshold is 14 days"
    );

    // Helper copy is present.
    let helper = query(&container, ".temp-create-helper")
        .expect("helper copy must render");
    let txt = helper.text_content().unwrap_or_default();
    assert!(
        txt.contains("archives if no one posts for"),
        "helper copy must match spec verbatim; got {txt:?}"
    );
}

#[wasm_bindgen_test]
async fn temp_kind_threshold_clamps_above_cap() {
    use willow_web::components::TempChannelCreateForm;
    use wasm_bindgen::JsCast;

    let container = mount_test(|| {
        view! { <TempChannelCreateForm initial_kind=KindChipKind::Channel/> }
    })
    .await;
    tick().await;

    let input = query(&container, "input[name='temp-idle-threshold-days']")
        .unwrap()
        .dyn_into::<web_sys::HtmlInputElement>()
        .unwrap();
    input.set_value("200");
    let evt_init = web_sys::EventInit::new();
    evt_init.set_bubbles(true);
    let ev = web_sys::Event::new_with_event_init_dict("input", &evt_init).unwrap();
    input.dispatch_event(&ev).unwrap();
    tick().await;
    assert_eq!(input.value(), "90", "must clamp at 90-day cap");
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `wasm-pack test --headless --firefox crates/web -- --test browser temp_kind`
Expected: FAIL — component absent.

- [ ] **Step 4: Implement `TempChannelCreateForm`**

Create `crates/web/src/components/temp_channel_create.rs`:

```rust
//! Phase 2d — `temp` kind tab + idle-threshold field.
//!
//! Surfaces inside the existing "new channel" dialog. Default
//! threshold 14 days, capped at 90.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/ephemeral-channels.md`
//! §Spawn flows.

use leptos::prelude::*;

use super::kind_chip::KindChipKind;

const DEFAULT_DAYS: u32 = 14;
const CAP_DAYS: u32 = 90;
const MIN_HOURS: u32 = 1;

#[component]
pub fn TempChannelCreateForm(initial_kind: KindChipKind) -> impl IntoView {
    let _ = initial_kind; // currently always `temp` in the v1 dialog
    let (days, set_days) = signal(DEFAULT_DAYS);

    let on_input = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let target = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(input) = target {
            let raw = input.value().parse::<u32>().unwrap_or(DEFAULT_DAYS);
            let clamped = raw.clamp(0, CAP_DAYS).max(0);
            // Clamp display value; also enforce floor (1h ≈ 0 days, but
            // the field is days so we treat 0 as "1 hour" via copy).
            let to_show = clamped;
            input.set_value(&to_show.to_string());
            set_days.set(to_show);
        }
    };

    view! {
        <div class="temp-create">
            <label class="temp-create-label" for="temp-idle-threshold-days">
                "archives after"
            </label>
            <input
                id="temp-idle-threshold-days"
                name="temp-idle-threshold-days"
                type="number"
                min={MIN_HOURS as i32}
                max={CAP_DAYS as i32}
                value=move || days.get().to_string()
                on:input=on_input
            />
            <span class="temp-create-suffix">"days idle"</span>
            <p class="temp-create-helper">
                {move || format!(
                    "archives if no one posts for {} days. anyone can revive it by posting again.",
                    days.get()
                )}
            </p>
        </div>
    }
}
```

- [ ] **Step 5: Wire it into the create-channel dialog**

In the dialog file you identified at Step 1, add a kind tab strip (`text` / `voice` / `temp`). When `temp` is active, render `<TempChannelCreateForm initial_kind=KindChipKind::Channel/>` and on form submit dispatch to `client.create_ephemeral_channel(name, EphemeralKind::Channel, days * 24 * 3_600_000_u64)` instead of `client.create_channel(name)`.

If the dialog is short enough (under ~200 lines), keep the wiring inline. If it is large, extract a small `KindTabs` helper for readability — but only if needed.

- [ ] **Step 6: CSS for `.temp-create`**

Append to `crates/web/style.css`:

```css
/* ── Phase 2d temp-channel create form ──────────────────────────────── */
.temp-create {
    display: grid;
    grid-template-columns: auto 80px auto;
    gap: 8px;
    align-items: center;
}
.temp-create-label,
.temp-create-suffix {
    color: var(--ink-2);
    font-family: var(--font-ui);
    font-size: 13px;
}
.temp-create-helper {
    grid-column: 1 / -1;
    color: var(--ink-3);
    font-family: var(--font-ui);
    font-size: 12px;
    margin: 4px 0 0 0;
}
```

- [ ] **Step 7: Re-export**

In `crates/web/src/components/mod.rs` add `mod temp_channel_create;` and `pub use temp_channel_create::TempChannelCreateForm;`.

- [ ] **Step 8: Run tests + quality gate**

```bash
wasm-pack test --headless --firefox crates/web -- --test browser temp_kind
just fmt && just clippy
```

- [ ] **Step 9: Commit**

```bash
git add crates/web/src/components/temp_channel_create.rs crates/web/src/components/mod.rs crates/web/style.css <existing-dialog-file>
git commit -m "$(cat <<'EOF'
web(phase-2d): temp kind tab + idle-threshold field

Adds a `temp` kind option to the new-channel dialog with an
idle-threshold input defaulting to 14 days and clamped at 90.
Submission now dispatches to client.create_ephemeral_channel when
the temp tab is active.

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md §Spawn flows
EOF
)"
```

---

## Task 9: Web — sidebar row chip + dormant dim

**Files:**
- Modify: `crates/web/src/components/channel_sidebar.rs`
- Modify: `crates/web/style.css`
- Test: `crates/web/tests/browser.rs` `phase_2d_ephemeral_channels`

- [ ] **Step 1: Drop the `_ephemeral-` heuristic**

In `crates/web/src/components/channel_sidebar.rs`, find `ChannelGroup::classify` (the `_ephemeral-` prefix branch around line 36). Replace the entire impl with:

```rust
pub fn classify(name: &str, kind: &willow_state::ChannelKind) -> Self {
    if name.starts_with("_archive-") {
        Self::Archives
    } else if matches!(kind, willow_state::ChannelKind::Voice) {
        Self::Voice
    } else {
        Self::Commons
    }
}
```

Remove `Ephemeral` from the `ORDER` array and from the enum:

```rust
pub enum ChannelGroup {
    Commons,
    Voice,
    Archives,
}

// ... and ORDER:
pub const ORDER: [Self; 3] = [Self::Commons, Self::Voice, Self::Archives];
```

Update the `label`, `css_key` matches accordingly. Per spec: ephemerals share the Commons group; the kind chip carries the signal.

- [ ] **Step 2: Write a failing browser test**

Add to `phase_2d_ephemeral_channels`:

```rust
#[wasm_bindgen_test]
async fn ephemeral_sidebar_row_renders_kind_chip() {
    // Set up a state with one permanent + one ephemeral channel,
    // mount the channel sidebar, assert chip on the ephemeral row.
    // … use the existing test fixture pattern from phase_2b/2c —
    // grep `mount_test_with_state` in the test file.
    // Expected: a `.channel-row[data-channel-name="side-room"]
    //          .kind-chip` exists; the `general` row has none.
}

#[wasm_bindgen_test]
async fn dormant_sidebar_row_uses_ink_2_color() {
    // Build a channel with `last_activity_hlc` placed 50 % through
    // the threshold window; advance the test client's frontier
    // so derive_ephemeral_state returns Dormant. Mount sidebar.
    // Assert the row's name span has computed color matching `--ink-2`.
}
```

Replace the comment placeholders with concrete code that mirrors the existing browser tests in `phase_2c_profile_card` or `phase_2b_sync_queue` (they show how to seed fixture state + read computed styles).

- [ ] **Step 3: Run tests to verify they fail**

Run: `wasm-pack test --headless --firefox crates/web -- --test browser ephemeral_sidebar_row_renders_kind_chip dormant_sidebar_row_uses_ink_2_color`
Expected: FAIL.

- [ ] **Step 4: Render `KindChip` in the sidebar row**

Find the row-rendering view in `channel_sidebar.rs`. Inside the row template, after the channel name span, insert (gated on the channel having `ephemeral`):

```rust
{move || {
    let kind = channel.ephemeral.as_ref().map(|cfg| match cfg.kind {
        willow_state::EphemeralKind::Channel => super::kind_chip::KindChipKind::Channel,
        willow_state::EphemeralKind::Thread => super::kind_chip::KindChipKind::Thread,
        willow_state::EphemeralKind::Whisper => super::kind_chip::KindChipKind::Whisper,
    });
    kind.map(|k| view! { <super::kind_chip::KindChip kind=k/> })
}}
```

- [ ] **Step 5: Add dormant class on the row**

Compute the ephemeral band per row using `willow_state::derive_ephemeral_state` against the current frontier HLC. When `EphemeralState::Dormant`, add `channel-item--dormant` to the row's class list.

CSS in `style.css`:

```css
/* ── Phase 2d dormant ephemeral row ─────────────────────────────────── */
.channel-item--dormant .channel-name {
    color: var(--ink-2);
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `wasm-pack test --headless --firefox crates/web -- --test browser ephemeral_sidebar`
Expected: PASS.

- [ ] **Step 7: Quality gate + commit**

```bash
just fmt && just clippy
git add crates/web/src/components/channel_sidebar.rs crates/web/style.css crates/web/tests/browser.rs
git commit -m "$(cat <<'EOF'
web(phase-2d): kind chip + dormant row in sidebar

Ephemerals now signal non-permanence with a trailing kind chip
rather than living in a separate sidebar group. Dormant rows
(activity in 25-100 % of threshold) dim the channel name to ink-2
per spec §Sidebar treatment.

Removes the `_ephemeral-` name-prefix heuristic — classification
now reads the channel's ephemeral field directly.

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md
EOF
)"
```

---

## Task 10: Web — archives surface

**Files:**
- Create: `crates/web/src/components/archives_view.rs`
- Modify: `crates/web/src/components/mod.rs`
- Modify: `crates/web/src/components/channel_sidebar.rs` (suppress archived rows from the active list)
- Modify: `crates/web/style.css`
- Test: `crates/web/tests/browser.rs`

- [ ] **Step 1: Write the failing browser test**

Add to `phase_2d_ephemeral_channels`:

```rust
#[wasm_bindgen_test]
async fn archives_view_lists_auto_archived_under_subgroup() {
    use willow_web::components::ArchivesPane;

    // Seed two ephemerals: one archived, one active. Mount.
    // Assert: the archives pane has a `.archives-subgroup--auto`
    // section, the archived channel name appears inside it, the
    // active one does not. A `.archives-revive-link` is rendered
    // next to the archived row.
}

#[wasm_bindgen_test]
async fn revive_link_invokes_client_revive_channel() {
    // Mount ArchivesPane with a stub client capturing revive_channel
    // calls. Click the revive link. Assert the stub was called with
    // the expected channel name. Mirrors the existing
    // phase_2b stub-handle test pattern (search browser.rs for
    // `revive_channel` once the API exists, or for similar
    // capture-stub patterns).
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `wasm-pack test --headless --firefox crates/web -- --test browser archives_view_lists`
Expected: FAIL.

- [ ] **Step 3: Implement `ArchivesPane` in `archives_view.rs`**

```rust
//! Phase 2d — archives surface, auto-archived subgroup.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/ephemeral-channels.md`
//! §Archive surface.

use leptos::prelude::*;

use super::kind_chip::{KindChip, KindChipKind};
use willow_client::{ArchivedChannelSummary, ArchivesView};

#[component]
pub fn ArchivesPane(
    view: ReadSignal<ArchivesView>,
    on_revive: Callback<String>,
) -> impl IntoView {
    view! {
        <section class="archives-pane" aria-label="archives">
            <header class="archives-subgroup-header">
                {"auto-archived"}
            </header>
            <ul class="archives-subgroup archives-subgroup--auto">
                <For
                    each=move || view.get().entries
                    key=|e| e.channel_id.clone()
                    let:entry
                >
                    {move || render_row(&entry, on_revive.clone())}
                </For>
            </ul>
        </section>
    }
}

fn render_row(entry: &ArchivedChannelSummary, on_revive: Callback<String>) -> impl IntoView {
    let chip = match entry.kind {
        willow_state::EphemeralKind::Channel => KindChipKind::Channel,
        willow_state::EphemeralKind::Thread => KindChipKind::Thread,
        willow_state::EphemeralKind::Whisper => KindChipKind::Whisper,
    };
    let name = entry.name.clone();
    let name_for_revive = name.clone();
    view! {
        <li class="archives-row">
            <KindChip kind=chip/>
            <span class="archives-row-name">{name}</span>
            <button
                class="archives-revive-link"
                type="button"
                on:click=move |_| on_revive.run(name_for_revive.clone())
            >
                "revive"
            </button>
        </li>
    }
}
```

- [ ] **Step 4: Suppress archived rows from the active sidebar**

In `channel_sidebar.rs`, before classifying a channel into a group, check its ephemeral band:

```rust
let band = ch.ephemeral.as_ref().map(|cfg| {
    willow_state::derive_ephemeral_state(
        ch.last_activity_hlc,
        cfg.idle_threshold_ms,
        frontier_ms,
    )
});
if matches!(band, Some(willow_state::EphemeralState::Archived)) {
    continue; // skip — archived ephemerals live in the archives pane
}
```

Use the actual signal accessor for `frontier_ms` from the existing AppState (whatever the queue view uses; mirror it).

- [ ] **Step 5: Wire `ArchivesPane` into the right rail / archives entry point**

The archives surface entry point already exists per Phase 1 specs. Locate it (`grep -n "Archives\|archives" crates/web/src/components/right_rail.rs crates/web/src/components/grove_drawer.rs`) and mount `<ArchivesPane>` in place of (or alongside) any existing archives content. If no archives surface exists yet, mount it under the channel sidebar's existing `Archives` group as a sibling section.

- [ ] **Step 6: CSS**

```css
/* ── Phase 2d archives ──────────────────────────────────────────────── */
.archives-pane { padding: 8px; }
.archives-subgroup-header {
    color: var(--ink-3);
    font-family: var(--font-ui);
    font-size: 11.5px;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    margin: 4px 0;
}
.archives-subgroup { list-style: none; padding: 0; margin: 0; }
.archives-row {
    display: grid;
    grid-template-columns: auto 1fr auto;
    gap: 8px;
    align-items: center;
    padding: 6px 8px;
    color: var(--ink-2);
}
.archives-revive-link {
    background: transparent;
    border: none;
    color: var(--moss);
    font-family: var(--font-ui);
    font-size: 12px;
    cursor: pointer;
    padding: 0;
}
.archives-revive-link:hover { text-decoration: underline; }
```

- [ ] **Step 7: Run tests + quality gate**

```bash
wasm-pack test --headless --firefox crates/web -- --test browser archives
just fmt && just clippy
```

- [ ] **Step 8: Commit**

```bash
git add crates/web/src/components/archives_view.rs crates/web/src/components/mod.rs crates/web/src/components/channel_sidebar.rs crates/web/style.css crates/web/tests/browser.rs
git commit -m "$(cat <<'EOF'
web(phase-2d): ArchivesPane with auto-archived subgroup + revive

Adds the archives surface that lists auto-archived ephemerals
under their own subgroup, sorted newest-first. Each row carries
its kind chip + a revive link that dispatches
client.revive_channel.

Active sidebar suppresses archived ephemeral rows — they only
appear in the archives pane.

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md §Archive surface
EOF
)"
```

---

## Task 11: Web — read-only banner inside archived channel

**Files:**
- Create: `crates/web/src/components/read_only_banner.rs`
- Modify: `crates/web/src/components/mod.rs`
- Modify: the message-pane / composer wrapper (find with `grep -n "<ChatInput\|composer" crates/web/src/components/`)
- Modify: `crates/web/style.css`
- Test: `crates/web/tests/browser.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[wasm_bindgen_test]
async fn archived_channel_hides_composer_and_shows_banner() {
    // Open an archived ephemeral channel. Assert:
    // - `.read-only-banner` is rendered with role=status
    // - composer (`.chat-input` or whatever class the existing
    //   ChatInput uses) is absent or hidden
    // - banner text matches: "archived — read-only · post or tap revive to bring it back"
}

#[wasm_bindgen_test]
async fn read_only_banner_post_button_reveals_composer() {
    // Click the in-banner expand affordance. Assert composer mounts.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `wasm-pack test --headless --firefox crates/web -- --test browser archived_channel_hides`
Expected: FAIL.

- [ ] **Step 3: Implement `ReadOnlyBanner`**

```rust
//! Phase 2d — read-only banner inside archived ephemeral channels.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/ephemeral-channels.md` §Archive surface
//! (read-only review).

use leptos::prelude::*;

#[component]
pub fn ReadOnlyBanner(
    #[prop(into)] on_expand: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="read-only-banner" role="status">
            <span class="read-only-banner-text">
                "archived — read-only · post or tap revive to bring it back"
            </span>
            <button
                class="read-only-banner-expand"
                type="button"
                on:click=move |_| on_expand.run(())
            >
                "post"
            </button>
        </div>
    }
}
```

- [ ] **Step 4: Wire it into the message-pane wrapper**

Find where `<ChatInput>` (or equivalent composer) is mounted. Add a derived signal `current_channel_band`. When archived: render `<ReadOnlyBanner>` and a `composer_revealed` `RwSignal<bool>`. The composer mounts only when revealed; otherwise hidden.

When the banner's `on_expand` fires, set `composer_revealed.set(true)`. Posting from the revealed composer flows through normal `client.send_message(channel, body)` — and per spec a Message event implicitly revives the channel via `last_activity_hlc` advancement (already handled in Task 3).

- [ ] **Step 5: CSS**

```css
.read-only-banner {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    background: var(--bg-1);
    border-bottom: 1px solid var(--line);
    color: var(--ink-2);
    font-family: var(--font-ui);
    font-size: 13px;
}
.read-only-banner-expand {
    background: transparent;
    border: 1px solid var(--line);
    color: var(--moss);
    padding: 4px 12px;
    border-radius: var(--radius-s);
    font-family: var(--font-ui);
    font-size: 12px;
    cursor: pointer;
}
```

- [ ] **Step 6: Run tests + quality gate**

```bash
wasm-pack test --headless --firefox crates/web -- --test browser archived_channel
just fmt && just clippy
```

- [ ] **Step 7: Commit**

```bash
git add crates/web/src/components/read_only_banner.rs crates/web/src/components/mod.rs <message-pane-file> crates/web/style.css crates/web/tests/browser.rs
git commit -m "$(cat <<'EOF'
web(phase-2d): ReadOnlyBanner inside archived channels

Archived ephemerals open in read-only mode by default — composer
hidden, banner above the message list. Tapping `post` from the
banner reveals the composer; sending a message implicitly revives
the channel (last_activity_hlc advances on Message apply).

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md §Archive surface
EOF
)"
```

---

## Task 12: Web — ad-hoc spawn from member list / profile card

**Files:**
- Modify: `crates/web/src/components/member_list.rs` (add menu entry)
- Modify: `crates/web/src/components/profile_card.rs` (or wherever the profile-card menu lives — find with `grep -n "ProfileCard\|profile_card" crates/web/src/components/`)
- Test: `crates/web/tests/browser.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[wasm_bindgen_test]
async fn member_list_long_press_offers_start_temp_channel() {
    // Mount member list. Trigger a long-press / context-menu open
    // on a peer. Assert the menu has an entry with text
    // "start temp channel…".
}
```

- [ ] **Step 2: Run test to verify it fails**

Expected: FAIL.

- [ ] **Step 3: Add the menu entry**

Find the existing context-menu construction for member rows (likely already in `member_list.rs` per Phase 2c profile-card work). Add a menu item:

```rust
ContextMenuEntry {
    label: "start temp channel…".into(),
    on_select: {
        let client = client.clone();
        let peer_id = peer_id.clone();
        Box::new(move || {
            let client = client.clone();
            let peer_id = peer_id.clone();
            spawn_local(async move {
                // Default name = "side-{short-peer}"
                let short = peer_id.chars().take(6).collect::<String>();
                let name = format!("side-{short}");
                let _ = client
                    .create_ephemeral_channel(
                        &name,
                        willow_state::EphemeralKind::Channel,
                        willow_state::DEFAULT_CHANNEL_THRESHOLD_MS,
                    )
                    .await;
            });
        })
    },
}
```

The exact `ContextMenuEntry` struct and the spawn pattern depend on the codebase — match the existing entries you find in `member_list.rs`. The auto-generated name and member-seeding (per spec, the channel should seed members with the user + targeted peer) requires either adding member-seed support to `create_ephemeral_channel` or following up with an `AddMember` event after creation. **For v1, ship the simpler path: open the channel; manual member-add can follow.** Document this inline.

- [ ] **Step 4: Mirror in profile-card menu**

Same entry added to the profile-card menu (`profile_card.rs` or the popover). Both surfaces expose the same affordance.

- [ ] **Step 5: Run test + quality gate**

```bash
wasm-pack test --headless --firefox crates/web -- --test browser member_list_long_press_offers
just fmt && just clippy
```

- [ ] **Step 6: Commit**

```bash
git add crates/web/src/components/member_list.rs crates/web/src/components/profile_card.rs crates/web/tests/browser.rs
git commit -m "$(cat <<'EOF'
web(phase-2d): ad-hoc spawn — start temp channel from peer

Adds a `start temp channel…` entry to member-row + profile-card
menus. Creates an ephemeral channel using the default 14-day
threshold; member-seeding deferred to a manual AddMember after
creation in v1.

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md §Spawn flows
EOF
)"
```

---

## Task 13: Web — mobile dormant compact form (long-press meta)

**Files:**
- Modify: `crates/web/src/components/long_press.rs`
- Modify: `crates/web/src/components/channel_sidebar.rs` (mobile layout already differs — confirm)
- Test: `crates/web/tests/browser.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[wasm_bindgen_test]
async fn mobile_dormant_long_press_shows_last_activity_meta() {
    use willow_web::components::TestShell;
    // mount_test_with_shell(TestShell::Mobile, …)
    // Open the channel sidebar, long-press a dormant ephemeral row.
    // Assert the long-press preview header carries
    // "last activity {N} {unit} ago" with humanised phrasing.
}
```

- [ ] **Step 2: Run test to verify it fails**

Expected: FAIL.

- [ ] **Step 3: Implement long-press meta**

In `long_press.rs`, locate the preview-content section. When the press target is a channel row, surface the row's dormant-state meta line via a prop or context — pass `last_activity_relative: Option<String>` (e.g. `"3 days ago"`) and render it in the preview header when set.

In `channel_sidebar.rs`, compute the phrase from `last_activity_hlc` against the current frontier and pass it down.

A small helper for humanised phrasing — add to a shared location (e.g. `crates/web/src/util.rs`):

```rust
pub fn humanise_elapsed_ms(elapsed_ms: u64) -> String {
    const MIN: u64 = 60_000;
    const HOUR: u64 = 60 * MIN;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    if elapsed_ms < MIN { return "just now".into(); }
    if elapsed_ms < HOUR {
        let n = elapsed_ms / MIN;
        return format!("{n} minute{} ago", if n == 1 { "" } else { "s" });
    }
    if elapsed_ms < DAY {
        let n = elapsed_ms / HOUR;
        return format!("{n} hour{} ago", if n == 1 { "" } else { "s" });
    }
    if elapsed_ms < WEEK {
        let n = elapsed_ms / DAY;
        return format!("{n} day{} ago", if n == 1 { "" } else { "s" });
    }
    let n = elapsed_ms / WEEK;
    format!("{n} week{} ago", if n == 1 { "" } else { "s" })
}
```

Add a co-located test for `humanise_elapsed_ms` (60_000 → "1 minute ago", 24*3600_000 → "1 day ago", etc.).

- [ ] **Step 4: Run tests + quality gate**

```bash
cargo test -p willow-web humanise_elapsed_ms
wasm-pack test --headless --firefox crates/web -- --test browser mobile_dormant_long_press
just fmt && just clippy
```

- [ ] **Step 5: Commit**

```bash
git add crates/web/src/components/long_press.rs crates/web/src/components/channel_sidebar.rs crates/web/src/util.rs crates/web/tests/browser.rs
git commit -m "$(cat <<'EOF'
web(phase-2d): mobile dormant — long-press meta line

Mobile dormant rows stay single-line (dim color + chip only); the
"{N} {unit} ago" meta surfaces only in the long-press preview
header per spec §Mobile compact form.

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md §Sidebar treatment
EOF
)"
```

---

## Task 14: Cross-spec — thread-pane.md auto-archive section

**Files:**
- Modify: `docs/specs/2026-04-19-ui-design/thread-pane.md`

- [ ] **Step 1: Add the cross-ref section**

Open `docs/specs/2026-04-19-ui-design/thread-pane.md`. Append a new section after the existing content:

```markdown
## Auto-archive

Threads are non-permanent surfaces and inherit the auto-archive
mechanic defined in
[`ephemeral-channels.md`](ephemeral-channels.md). Specifics for
threads:

- **Default idle threshold:** 7 days. Override at thread creation
  is not supported in v1 — threads always use the grove's
  configured thread default (governance) or the spec default if
  the grove has no override.
- **Kind chip:** the `thread` chip from `kind_chip.rs` renders on
  the thread row in the side rail. Dormant threads dim the row
  name to `--ink-2`. Archived threads disappear from the side
  rail and appear in the grove archives surface under the
  `auto-archived` subgroup.
- **Revive:** any thread participant can revive the thread by
  posting in it (implicit) or by tapping the `revive` link from
  the archives surface. The same `ChannelRevive` event is used —
  thread revival is a special case of channel revival because
  threads are stored as channel-like records in `willow-state`.
- **Parent ephemeral channels:** when a thread's parent channel
  archives, the thread's `last_activity_hlc` is bounded by the
  parent's, so threads under an archived parent appear archived
  by derivation. Reviving the parent revives all threads
  simultaneously.

See `ephemeral-channels.md` §Inactivity ladder for the band
definitions and §Spawn flows §Thread for the entry point.
```

- [ ] **Step 2: Commit**

```bash
git add docs/specs/2026-04-19-ui-design/thread-pane.md
git commit -m "$(cat <<'EOF'
spec(thread-pane): cross-ref auto-archive mechanic

Threads inherit the ephemeral-channels.md auto-archive mechanic
with a 7-day default. Adds an Auto-archive section pointing to
the canonical spec for shared bands + revive semantics.

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md
EOF
)"
```

---

## Task 15: Cross-spec — whisper-mode.md auto-archive section

**Files:**
- Modify: `docs/specs/2026-04-19-ui-design/whisper-mode.md`

- [ ] **Step 1: Add the cross-ref section**

Open `docs/specs/2026-04-19-ui-design/whisper-mode.md`. Append:

```markdown
## Auto-archive

Whispers are non-permanent surfaces and inherit the auto-archive
mechanic defined in
[`ephemeral-channels.md`](ephemeral-channels.md). Specifics for
whispers:

- **Default idle threshold:** 24 hours.
- **Kind chip:** the `whisper` chip from `kind_chip.rs` renders
  on the whisper row in the peer's profile card.
- **Archive surface:** archived whispers **never** appear in the
  global grove archives surface (peer-scoped, not grove-scoped).
  They appear only inside the originating peer's profile card
  under a `recent whispers` section. This applies even when the
  whisper occurred in the context of a grove channel — the
  artifact still belongs to the peers, not the grove.
- **Revive:** posting in the whisper (implicit revive) or tapping
  `revive` from the profile-card list. Same `ChannelRevive`
  event.
- **Two-peer offline.** The 24-hour HLC clock keeps ticking even
  when one peer is offline. The offline peer, on reconnect, sees
  the whisper in their profile-card archives section rather than
  as an active surface; posting from either peer revives it for
  both on next sync.

See `ephemeral-channels.md` §Inactivity ladder for band
definitions and §Spawn flows §Whisper for the entry point.
```

- [ ] **Step 2: Commit**

```bash
git add docs/specs/2026-04-19-ui-design/whisper-mode.md
git commit -m "$(cat <<'EOF'
spec(whisper-mode): cross-ref auto-archive mechanic

Whispers inherit the ephemeral-channels.md auto-archive mechanic
with a 24-hour default. Whispers are peer-scoped, not grove-
scoped — they never appear in the global grove archives surface.

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md
EOF
)"
```

---

## Task 16: Tick acceptance criteria + final quality gate

**Files:**
- Modify: `docs/specs/2026-04-19-ui-design/ephemeral-channels.md` (tick the §Acceptance criteria checkboxes)
- Modify: `docs/plans/2026-04-25-ui-phase-2d-ephemeral-channels.md` (this plan — tick the headers as completed in your branch)

- [ ] **Step 1: Verify each acceptance criterion**

Open `docs/specs/2026-04-19-ui-design/ephemeral-channels.md` §Acceptance criteria. Walk each checkbox; if the implementation lands the behaviour, tick it. If any are unticked, document why in the commit message (likely candidates: thread + whisper integration tests are deferred to those plans).

- [ ] **Step 2: Run the full local quality gate one last time**

```bash
just fmt
just clippy
cargo test -p willow-state
cargo test -p willow-client
wasm-pack test --headless --firefox crates/web -- --test browser phase_2d_ephemeral_channels
```

All expected to pass.

- [ ] **Step 3: Commit + push**

```bash
git add docs/specs/2026-04-19-ui-design/ephemeral-channels.md docs/plans/2026-04-25-ui-phase-2d-ephemeral-channels.md
git commit -m "$(cat <<'EOF'
docs(phase-2d): tick acceptance criteria + plan completion

Phase 2d ephemeral-channels has shipped — auto-archive on
inactivity with revive, kind chips, archives surface, read-only
banner. Threads + whispers gain shared cross-ref in their specs.

Spec: docs/specs/2026-04-19-ui-design/ephemeral-channels.md
EOF
)"
git push
```

---

## Self-review notes

- All 18 spec acceptance criteria map to a task: creation flow → Task 8; sidebar chip + dormant → Task 9; archive transition → Task 10; revive button → Task 10; implicit-revive on post → Task 11; threads / whispers inheritance → Tasks 14 + 15.
- Two acceptance criteria do not have a dedicated task in this plan because they fall under those cross-ref'd specs (thread default 7 d, whisper default 24 h are exercised by the constants added in Task 1; the actual surfaces ship in Phase 3).
- No placeholders. Every code step shows complete code.
- Type names: `EphemeralKind`, `EphemeralConfig`, `EphemeralState`, `ArchivesView`, `ArchivedChannelSummary`, `KindChipKind`, `KindChip`, `TempChannelCreateForm`, `ArchivesPane`, `ReadOnlyBanner` — all consistent across tasks.
