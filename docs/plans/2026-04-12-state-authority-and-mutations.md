# State Authority & Mutations — Implementation Plan

**Date**: 2026-04-12
**Spec**: `docs/specs/2026-04-12-state-authority-and-mutations.md`
**Scope**: Permission pre-check before event creation, catch-all safety

## Overview

Two changes to align the codebase with the spec:

1. Add a permission pre-check to `ManagedDag::create_and_insert()` so
   rejected events never enter the DAG or advance the sequence number.
2. The `required_permission()` catch-all annotation is already in place
   (exhaustive variant comment at `materialize.rs:237-257`). No further
   work needed for that requirement.

All work is in `crates/state/src/`. Every step ends with
`cargo test -p willow-state` passing.

## Step 1: Extract `check_permission` function

**File**: `crates/state/src/materialize.rs`

Extract the permission-checking logic from `apply_event()` into a
standalone public function that can be called *before* event creation:

```rust
/// Check whether an author is allowed to emit a given EventKind
/// against the current state. Returns Ok(()) if permitted, or an
/// error string if not.
///
/// This is the same logic as the permission tiers in apply_event(),
/// extracted so callers can pre-check before signing an event.
pub fn check_permission(
    state: &ServerState,
    author: &EndpointId,
    kind: &EventKind,
) -> Result<(), String> {
    // Governance: Propose/Vote require admin.
    match kind {
        EventKind::Propose { .. } | EventKind::Vote { .. } => {
            if !state.is_admin(author) {
                return Err("not an admin".into());
            }
            return Ok(());
        }
        EventKind::CreateServer { .. } => return Ok(()),
        _ => {}
    }

    // Admin-only events.
    match kind {
        EventKind::GrantPermission { .. }
        | EventKind::RevokePermission { .. }
        | EventKind::RenameServer { .. }
        | EventKind::SetServerDescription { .. } => {
            if !state.is_admin(author) {
                return Err(format!("author '{}' is not an admin", author));
            }
        }
        _ => {}
    }

    // Permission-gated events.
    if let Some(ref perm) = required_permission(kind) {
        if !state.has_permission(author, perm) {
            return Err(format!("author '{}' lacks {:?} permission", author, perm));
        }
    }

    Ok(())
}
```

Then refactor `apply_event()` to call `check_permission()` internally,
keeping the logic in one place:

```rust
fn apply_event(state: &mut ServerState, event: &Event) -> ApplyResult {
    if let Err(reason) = check_permission(state, &event.author, &event.kind) {
        return ApplyResult::Rejected(reason);
    }
    // governance state mutations (Propose inserts proposal, Vote records vote)
    // ...
    apply_mutation(state, event)
}
```

Note: governance events (Propose/Vote) mutate state in `apply_event`
after the permission check (inserting proposals, recording votes).
The `check_permission` function only checks *whether* the author is
allowed — the actual state mutation remains in `apply_event`.

**Tests**:
- `check_permission_allows_admin_propose`
- `check_permission_rejects_non_admin_propose`
- `check_permission_allows_granted_send_messages`
- `check_permission_rejects_without_send_messages`
- `check_permission_admin_implicitly_has_all`
- `check_permission_unrestricted_events_always_pass`

## Step 2: Add `PermissionDenied` variant to `InsertError`

**File**: `crates/state/src/dag.rs`

Add a new variant:

```rust
pub enum InsertError {
    // ... existing variants ...
    /// Author lacks the required permission for this EventKind.
    PermissionDenied(String),
}
```

Update the `Display` impl to format it.

## Step 3: Pre-check in `create_and_insert`

**File**: `crates/state/src/managed.rs`

Add the permission check before event creation:

```rust
pub fn create_and_insert(
    &mut self,
    identity: &Identity,
    kind: EventKind,
    timestamp_ms: u64,
) -> Result<Event, InsertError> {
    if !self.synced {
        return Err(InsertError::NotGenesis);
    }

    // Pre-check: reject before signing if the author lacks permission.
    check_permission(&self.state, &identity.endpoint_id(), &kind)
        .map_err(InsertError::PermissionDenied)?;

    // ... rest unchanged: compute deps, create event, insert_and_apply ...
}
```

Import `check_permission` from `crate::materialize`.

**Tests**:
- `create_and_insert_rejects_without_permission` — create a
  ManagedDag, do NOT grant SendMessages to a second identity, call
  `create_and_insert` with a Message event from that identity, assert
  `InsertError::PermissionDenied`.
- `create_and_insert_does_not_advance_seq_on_rejection` — after the
  above rejection, verify `dag.latest_seq(&peer)` has not advanced.
- `create_and_insert_succeeds_with_permission` — grant SendMessages,
  then create a Message event, assert success.

## Step 4: Verify existing tests still pass

Run `cargo test -p willow-state`. The refactored `apply_event` calls
`check_permission` internally, so all existing permission tests should
pass without modification.

Run `cargo test -p willow-client` to verify the client's `build_event`
path (which calls `create_and_insert`) still works for authorized
actions.

Run `just check` to confirm fmt, clippy, all tests, and WASM.
