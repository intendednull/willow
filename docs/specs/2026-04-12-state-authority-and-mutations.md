# State, Authority, and Mutations

> **One-sentence summary:** `willow-state` is the single source of truth.
> All authority checks live in `apply_event()` and the
> `required_permission()` table. Permissions are checked *before* an
> event is created — rejected events never enter the DAG.

## Single source of truth

`willow-state` owns the canonical representation of a server: the
`ServerState` struct, produced by replaying a per-author Merkle DAG of
signed events through `materialize()`. No other crate may hold
authoritative state or enforce trust decisions.

- Client holds `ServerState` directly.
- UI derives reactive views from `ServerState` fields.
- All mutations flow through the event pipeline (no direct struct
  mutation).

## Local mutation flow

Every local state change follows one path:

```
user action
  → ManagedDag::create_and_insert()
      1. permission pre-check   (reject before signing)
      2. dag.create_event()     (sign, compute hash, set seq/prev/deps)
      3. dag.insert()           (verify signature, check seq, dedup)
      4. apply_incremental()    → apply_event() → apply_mutation()
  → broadcast signed Event over gossip
  → UI observes updated ServerState
```

**Permissions are checked before the event is created.** If the author
lacks the required permission, `create_and_insert` returns an error.
No event is signed, no sequence number is advanced, and the DAG does
not grow.

This prevents a class of problems where rejected events accumulate in
the DAG. Since the author has already committed to their sequence
chain once an event is signed, a post-insert rejection would leave a
dead event in the DAG that cannot be removed without breaking the
author's chain.

## Remote event flow

```
gossip delivers signed Event
  → dag.insert()           (verify signature, check seq, dedup)
  → apply_incremental()    → apply_event()
      1. governance check   (Propose/Vote require is_admin)
      2. admin-only check   (GrantPermission, RevokePermission,
                              RenameServer, SetServerDescription)
      3. permission check   (required_permission() table)
      4. apply_mutation()   (project into ServerState)
  → UI observes updated ServerState
```

For remote events, the permission check happens after DAG insertion
because the sender has already committed to their chain. There are two
cases where a remote event is rejected:

- **Out-of-order delivery:** A permission grant hasn't arrived yet.
  The event is structurally valid but the local state doesn't reflect
  the grant. This is a sync timing issue — the sender passed the
  pre-check locally, so the permission exists in the full DAG.
- **Malicious sender:** The sender forged a chain without permission
  checks. The event stays in the DAG but does not affect state. A
  persistently-rejected author can be evicted at the network layer.

## Permission tiers

| Tier | Events | Enforcement |
|------|--------|-------------|
| **Governance (vote)** | `Propose`, `Vote` | `is_admin()` — only admins may propose or vote. Actions auto-apply when vote threshold is met. |
| **Admin-only** | `GrantPermission`, `RevokePermission`, `RenameServer`, `SetServerDescription` | `is_admin()` — any single admin can execute these directly. |
| **Permission-gated** | `Message`, `EditMessage`, `DeleteMessage`, `Reaction` → `SendMessages`; `CreateChannel`, `DeleteChannel`, `RenameChannel`, `RotateChannelKey` → `ManageChannels`; `CreateRole`, `DeleteRole`, `SetPermission`, `AssignRole` → `ManageRoles` | `has_permission()` — admins pass implicitly; non-admins need an explicit grant. |
| **Member-only (server state)** | `SetProfile`, `UpdateProfile`, `PinMessage`, `UnpinMessage` | `state.members.contains_key(&author)` — any current member can execute. `required_permission()` returns `None`, so the membership gate lives in each handler in `apply_mutation` (defense-in-depth, see issue #177). Note: this is "any current member" — distinct from "any signer" with no gate at all. |
| **Per-identity preference (no gate)** | `MuteChannel`, `MuteGrove` | No check — these are personal preferences, not shared server state. Preferences persist across kicks. |
| **Genesis** | `CreateServer` | No-op on replay; the genesis author becomes the sole initial admin. |

Admin status is tracked in `ServerState.admins` and is **not** a variant
of the `Permission` enum. It can only be granted or revoked through the
`ProposedAction::GrantAdmin` / `RevokeAdmin` vote path. This structural
separation makes it impossible to escalate to admin via a
`GrantPermission` event.

## The `required_permission()` catch-all

The `_ => None` arm in `required_permission()` silently passes any
unrecognised `EventKind` variant without a permission check. This is
the mechanism behind bug #109: a new variant that falls into the
catch-all gets zero enforcement.

**Every variant that returns `None` is intentionally unrestricted or
checked elsewhere.** The catch-all arm MUST list these variants in a
comment so reviewers notice when a new variant is missing:

- `CreateServer` — genesis, checked structurally
- `Propose`, `Vote` — governance, checked in the governance block
- `GrantPermission`, `RevokePermission` — admin-only, checked in the
  admin block
- `RenameServer`, `SetServerDescription` — admin-only, checked in the
  admin block
- `SetProfile` — any current member; membership gate enforced in
  `apply_mutation` (not in `required_permission()`). See issue #177.
- `UpdateProfile` — any current member; membership gate enforced in
  `apply_mutation`. Self-authorship is enforced structurally (only the
  author's own profile is mutated). See issue #177.
- `PinMessage`, `UnpinMessage` — any current member; membership gate
  enforced in `apply_mutation` (not in `required_permission()`). See
  issue #177.
- `MuteChannel`, `MuteGrove` — per-identity preference, never gated
  (preferences are not shared server state and survive a kick)

**"Intentionally unrestricted" still requires membership.** The
membership gate (`state.members.contains_key(&event.author)`) lives in
the per-handler block inside `apply_mutation`, not in
`required_permission()`. This is defense-in-depth: even if a future
refactor changes the permission table, the handler-local gate keeps
non-members from mutating server state.

If a variant is not in this list and not in a `required_permission()`
arm, it is a bug.

## Checklists

### Adding a new permission

1. Add a variant to `Permission` in `crates/state/src/event.rs`.
2. Add the corresponding `EventKind` → `Permission` mapping to
   `required_permission()` in `crates/state/src/materialize.rs`.
3. Implement `has_permission()` handling if the new permission needs
   special logic (admins already pass implicitly).
4. Add state-machine tests: grant, revoke, rejection without permission.
5. Update UI if the permission should be visible in settings.

### Adding a new event kind

1. Add a variant to `EventKind` in `crates/state/src/event.rs`.
2. **Decide its authority tier** — governance, admin-only,
   permission-gated, or unrestricted.
3. If permission-gated: add it to `required_permission()`.
4. If admin-only: add it to the admin-only match block in
   `apply_event()`.
5. If unrestricted: add it to the comment on the `_ => None` arm
   listing intentionally-unrestricted variants.
6. Handle it in `apply_mutation()`.
7. Add state-machine tests for application, dedup, and permission
   rejection.
