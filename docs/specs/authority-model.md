# Authority Model

> **One-sentence summary:** All authority checks live in
> `willow-state::materialize::apply_unchecked` and the
> `required_permission()` table. Every other crate is untrusted plumbing.

## Single source of truth

`willow-state` owns the canonical representation of a server: the
`ServerState` struct, produced by replaying a per-author Merkle DAG of
signed events through `materialize()`. No other crate may hold
authoritative state or enforce trust decisions.

`willow-channel` is **deprecated and scheduled for removal.** It was an
earlier data-model crate whose types (`Server`, `Channel`, `Role`,
`Member`) duplicated what `willow-state` already provides. The client
currently holds a `willow_channel::Server` alongside a `ServerState` and
syncs them via `reconcile_topic_map()` — this is tech debt. The target
architecture is:

- Client holds `ServerState` directly (the single source of truth).
- UI derives reactive views from `ServerState` fields.
- All mutations flow through the event pipeline (no direct struct mutation).
- Any types still needed from `willow-channel` (e.g. `ChannelKind`,
  invite helpers) move into `willow-state` or `willow-client`.

## Event flow

Every state mutation follows one path:

```
user action
  → Client emits signed Event (EventKind variant)
  → gossip broadcast to peers
  → peer receives Event
  → dag.insert() (signature + sequence verification)
  → apply_incremental() → apply_unchecked()
      1. governance check   (Propose/Vote require is_admin)
      2. admin-only check   (GrantPermission, RevokePermission,
                              RenameServer, SetServerDescription)
      3. permission check   (required_permission() table)
      4. apply_mutation()   (project into ServerState)
  → UI observes updated ServerState
```

**Direct struct mutation is not in this path and does not constitute
enforcement.** Any code that mutates a `Server` or `ServerState` outside
`apply_unchecked` is bypassing authority.

## Permission tiers

| Tier | Events | Enforcement |
|------|--------|-------------|
| **Governance (vote)** | `Propose`, `Vote` | `is_admin()` — only admins may propose or vote. Actions auto-apply when vote threshold is met. |
| **Admin-only** | `GrantPermission`, `RevokePermission`, `RenameServer`, `SetServerDescription` | `is_admin()` — any single admin can execute these directly. |
| **Permission-gated** | `Message`, `EditMessage`, `DeleteMessage`, `Reaction` → `SendMessages`; `CreateChannel`, `DeleteChannel`, `RenameChannel`, `RotateChannelKey` → `ManageChannels`; `CreateRole`, `DeleteRole`, `SetPermission`, `AssignRole` → `ManageRoles` | `has_permission()` — admins pass implicitly; non-admins need an explicit grant. |
| **Unrestricted** | `SetProfile`, `PinMessage`, `UnpinMessage` | No check — any member can execute. |
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
- `SetProfile` — intentionally unrestricted
- `PinMessage`, `UnpinMessage` — intentionally unrestricted

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
   `apply_unchecked()`.
5. If unrestricted: add it to the comment on the `_ => None` arm
   listing intentionally-unrestricted variants.
6. Handle it in `apply_mutation()`.
7. Add state-machine tests for application, dedup, and permission
   rejection.
