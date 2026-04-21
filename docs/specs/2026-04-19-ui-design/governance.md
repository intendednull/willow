# Governance — authority, management, event log

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:** [`foundation.md`](foundation.md), `layout-primitives.md`, `profile-card.md`, `trust-verification.md`

## Purpose

Make the authority graph of a grove legible to the people who hold it.
Willow has no hidden server admin — the owner is the root of trust and
every other permission is a signed event that can be traced back to
them. This spec covers three admin surfaces that share the
owner/steward mental model and live behind one tab group in grove
settings:

1. **Governance** — read-mostly view of permissions, the chain of
   trust, a matrix for grants/revokes.
2. **Manage** — day-to-day: role presets, invites, files.
3. **Event log** — read-only audit timeline, filterable and exportable.

Switching tabs does not tear down layout; they share a header and a
single scroll container on mobile.

## Scope

- Reachable only from the grove rail context menu
  ("grove settings → governance"). Not in top-level nav.
- Visible to the owner and to peers with `Administrator`. Other
  members who open the deep link see a calm 403 card
  ("only stewards can see governance in this grove") linking to their
  own `profile-card.md` permissions summary.
- The member-side "what am I allowed to do here?" view lives in
  `profile-card.md`; not redefined here.
- File management lives here because files are grove-wide;
  per-message attachments stay in `files-inline.md` and pins stay in
  `reactions-pins.md`.
- The event log surfaces *that* events happened — not bodies. Content
  lives in the chat view.

## Surfacing and entry

- Grove rail → right-click / long-press → "grove settings" → settings
  popover (desktop) / full-screen sheet (mobile). Governance, Manage
  (3 sub-tabs), Event log. Tabs are hidden entirely for non-admins.
- Keyboard: `g g` enters settings; `[` / `]` cycle tabs.
- Tabs use `layout-primitives.md`: 2 px `--moss-2` bottom border when
  active; *meta* typography from `foundation.md`.

## Governance (chain of trust + permission matrix + recent changes)

**Header.** Display M italic "governance"; right-side meta Tag pills:
"{n} stewards", "{n} members", plus a presence pulse — `--ok` when
the owner is online, `--warn` when not seen on the network ≥ 72 h.

Body intro copy:

> every change to this grove is a signed event. the owner is the root.
> stewards carry delegated authority. you can trace any member's
> permissions back to the owner who granted them.

### Desktop layout (≥ 900 px)

Three columns, `320 | 1fr | 360`, gap 24 px, padding `22px 32px`:

1. **Owner & chain** — `--bg-1`, `--radius`, `--line`.
2. **Permission matrix** — `--bg-1` with `--bg-2` cells.
3. **Recent authority changes** — `--bg-0` feed, `--line-soft` left
   border.

At < 1100 px column 3 stacks below the matrix; at < 900 px column 2
also stacks and the matrix becomes a list of expand-rows.

### Column 1 — Chain of trust

Top: owner avatar (48 px, `willowPulse` on presence), display name in
display S italic, pronouns in body S, fingerprint short form in mono S.
Beneath the card, a *meta*-typography label "root of trust", then a
vertical ordered list:

```
● owner      — mira.sage                [self]
│
├─ steward   — ori.heron                 granted by mira.sage · 3w ago
│  ├─ admin  — perrin.wrack              granted by ori.heron · 12d ago
│  └─ admin  — kestrel.cairn             granted by ori.heron · 5d ago
│
└─ steward   — nico.slate                granted by mira.sage · 2w ago
```

- Each rung is keyboard-focusable. Enter opens a popover with the
  source event's hash (mono), timestamp, and a "view event" link that
  deep-links into the event log filter.
- Hovering a rung draws an `--moss-1` guide line tracing the
  delegation back to the owner.
- Clicking a rung sets a breadcrumb at the top of column 1
  ("mira.sage → ori.heron → perrin.wrack"), one Fraunces-italic hop
  per name.
- A broken chain (e.g. a grant from a peer whose own authority has
  since been revoked) renders a dashed `--warn` left border and the
  meta reads "granted by {peer} · authority since revoked". Willow
  never retroactively invalidates events, but the UI warns that future
  mutations from that branch will be rejected.

Empty state (brand-new grove with only the owner): "the grove is
quiet. you're the only peer with authority." in display S italic.

### Column 2 — Permission matrix

`<table>` on `--bg-1`. Rows = members. Columns = the seven
user-visible permissions (see "Copy"). Member column is 220 px;
remaining width splits evenly across permission columns. On narrow
widths the column headers rotate 90° and use mono S — same treatment
as the reference design. Header row uses *meta* typography.

Row layout:
- 40 px tall (balanced), 48 px (cozy), 32 px (dense).
- Avatar (24 px) + display name (body, `--ink-0`) + fingerprint short
  form (mono S, `--ink-3`). The fingerprint receives the verification
  badge from `trust-verification.md`: verified (filled moss check),
  unverified (amber dashed ring), pending (hourglass).
- Owner row is pinned at the top, has a 2 px `--moss-1` left border,
  and shows a "owner" Tag beside the name. Every cell on the owner
  row is a filled check in `--moss-4` on `--moss-0`, marked
  `aria-disabled="true"`, tooltip "holds every permission implicitly".

Cell states:
- **Checked, accent filled** — granted. Click toggles revoke.
- **Unchecked** — not granted. Click toggles grant with a
  `willow-pop-in` fill, then emits `GrantPermission` /
  `RevokePermission`. If the emit is rejected locally (e.g. the
  editor loses `ManageRoles` mid-edit), the cell snaps back and a
  toast reads "that grant was rejected by your permissions."
- **Pending** — optimistic state before the event applies. Hairline
  mono pulse on the border; resolves within ~180 ms typically.
- **Indeterminate** — permission comes via a role, not a direct
  grant. Cell shows `—` in mono `--ink-3` with tooltip "granted via
  role: {role name}". Click opens the Roles tab pre-filtered.

Special columns:
- **administrator** — granting admin is a governance action, not a
  direct permission. Clicking the cell opens the propose-admin sheet
  (member + rationale + quorum preview). Cell reflects the admin set
  once the proposal passes.
- **can kick members** — no persisted permission; reflects admin
  status (derived). Checkbox is disabled; per-member kick uses the
  row's overflow menu (`more-horizontal`) which opens the propose-
  kick sheet.

Self-owner revoke guard: if the logged-in peer is the owner and taps
the "administrator" cell on their own row (or otherwise attempts to
revoke their own ownership), a centred dialog on `--bg-1` with
`--shadow-2` reads "you cannot revoke your own ownership. transfer
first." Single "ok, got it" button plus a quiet "learn about
transfer" link. No event is emitted.

At ≥ 60 members the matrix virtualises (fixed-height rows, recycled
DOM), with a sticky "find a member…" search above. Search matches
display name, short fingerprint, and peer ID prefix.

### Column 3 — Recent authority changes

Last 30 days of `GrantPermission`, `RevokePermission`, `Propose`,
`Vote`, and role-scoped events. Row:

```
[icon] perrin granted administrator
       by mira.sage · 3d ago
       fp  3f82…a19c
```

Icon colour by kind: `GrantPermission` → `--moss-3`,
`RevokePermission` → `--err`, `Propose` / `Vote` → `--willow`, role
changes → `--amber`. Subject bold body; meta typography; fingerprint
mono S `--ink-3`. Row click filters the event log to that event.
Sticky Fraunces-italic day headers use the "— today —" pattern from
`message-row.md`. Empty feed: "no authority changes in the last 30
days. a steady grove."

### Mobile layout (< 640 px)

Stacked: owner & chain, permissions (collapsed), recent authority
changes. Matrix becomes a list of member rows; tapping a row expands
inline into permission toggle pills (≥ 44 × 44 each). Owner row reads
"owner · holds every permission" (non-toggleable). No independent
matrix scroll; "find a member" is the primary narrowing control.

## Manage → Roles

Tab 1 of Manage. Tab labels: "roles & permissions", "invites",
"files".

**Layout.** Vertical list of role cards on `--bg-1`, gap 10 px,
padding `20px 32px`.

**Role card** (64 px balanced / 80 px cozy / 48 px dense; `--bg-1`,
`--line`, `--radius`). Grid:

```
[dot]   role name          permission summary              member count   [edit]
```

- Coloured dot (10 px) comes from a foundation accent token
  (`--moss-3`, `--willow`, `--amber`, `--dusk`, `--cedar`, `--lichen`,
  `--ember`). Users pick from *those* — no arbitrary hex.
- Role name: body L, `--ink-0`.
- Permission summary: body S, `--ink-2`. Rule: if ≤ 2 permissions,
  list them ("can create invites, can send messages"); otherwise the
  two strongest plus "+n more". Power order: administrator > kick
  members > manage roles > manage channels > sync history > create
  invites > send messages.
- Member count: meta mono, `--ink-2` ("4 peers" / "1 peer").
- Edit action: `edit` icon, visible only to owner/admin.

**Default presets.**

| Key       | Name     | Colour      | Bundle                                         |
|-----------|----------|-------------|------------------------------------------------|
| `steward` | steward  | `--willow`  | administrator (via governance path)            |
| `member`  | member   | `--moss-3`  | send messages, create invites                  |
| `guest`   | guest    | `--ink-2`   | send messages                                  |

Presets are a client-side *bundle of events*, not a backend primitive.
Canonical role ids (`willow:preset:steward`, `willow:preset:member`,
`willow:preset:guest`) mark which roles are presets vs custom.

**Create / edit role sheet.** Desktop popover (360 px) / mobile bottom
sheet. Fields:

- name (required, placeholder "role name")
- colour (7-chip row of accent variants from `foundation.md`, 24 × 24
  touch target with `--focus-ring` halo on the selected chip)
- description (optional, body S, placeholder "what is this role for?")
- permission checklist (the same seven checkboxes as the matrix)
- assign members (chip multi-select with "find a member" search)

Save button: "save role". Cancel is quiet `--bg-2`. Delete (edit mode
only) opens the shared red confirm dialog from
`layout-primitives.md`.

**Apply semantics.** Save emits `CreateRole` (if new) + one
`SetPermission` per toggled permission + one `AssignRole` per added
member. Colour and description are new state — see "Data
dependencies".

**Empty state.** Only visible if every preset has been deleted: "no
roles yet — presets ship above. add one if you want a custom set."

## Manage → Invites

Tab 2. List of live invites on `--bg-1`, gap 10 px.

**Primary action strip** (top of tab): primary CTA
"create new letter of introduction" — `--moss-1` fill, `--moss-4`
text, display S italic, `plus` icon — next to a secondary info panel
(dashed `--line` border on `--bg-2`, `--ink-3` text) reading:

> invites are human-readable fingerprints. paste one from someone you
> trust, or share yours by voice.

**Invite row.** Desktop grid: `1fr 160 140 120 140 90`. Columns:

- invite code (mono L, `--ink-0`, format `willow://invite/{fp}`,
  with the full URL shown on hover/tap)
- inviter (20 px avatar + body S name, meta "created {time}" below)
- uses (mono, `x/y uses`; unlimited is `∞ uses`)
- expiry (mono `--ink-2`; `--amber` at < 24 h, `--err` when "expired")
- role (Tag in the role's preset colour, copy "grants {role}")
- revoke action (quiet `--bg-2` button, `--err` text)

Revoke flow: desktop swaps the button inline for
"really revoke? / cancel"; mobile opens a confirm sheet. On confirm
the row animates to the burned style (opacity 0.55, dashed border)
and the revoke action is replaced by the meta "revoked {time}". Other
terminal states: "exhausted", "expired".

**Create letter of introduction sheet.** Fields:

- max uses (dropdown: 1, 3, 5, 10, unlimited — default 1)
- expires (dropdown: 1h, 24h, 7d, 30d, never — default 7d)
- attached role (role-preset dropdown + "don't attach a role" —
  default "member"; preview Tag below)
- optional note (placeholder "for kes's sister")

Footer button "seal this letter". Success state swaps the sheet for a
one-screen result: mono L invite code, "copy" button, "share" button
(mobile only — system share sheet), meta "this letter was signed by
{self}. when they arrive, compare fingerprints." No auto-dismiss.

**Mobile invite row.** Stacked: code (full-width, wrapping mono L);
inviter + uses row; role + expiry row; revoke button pinned right.
Minimum row height 120 px.

## Manage → Files

Tab 3. Body copy: "files shared in this grove". Every file reference
in the grove's state, across channels.

**Header strip.** Left: filter pills (`Tag`-sized) — channel (dropdown
of all channels, default "any channel"); uploader (avatar-prefixed
member dropdown, default "anyone"); kind (multi-select pill group:
image, audio, doc, other); search (input with `search` icon,
placeholder "search filenames"). Right: meta pills — "{n} files",
"content-addressed · sha-256", "chunked at 256 KiB", "keys per chunk"
with a `lock` icon.

**File row.** Desktop grid:
`40 | 1fr | 90 | 80 | 170 | 130 | 130 | 90`. Row height 64 px.
Columns:

- icon tile (34 px, `--bg-2`, `--line`, `--radius-s`; colour by kind —
  image `--moss-3`, audio `--willow`, doc `--ink-1`, other `--ink-2`;
  ephemeral overrides to `--whisper`)
- filename (body, `--ink-0`, ellipsis) and hash
  ("sha-256 · {short-short}", mono S, `--ink-3`)
- size (mono `--ink-1`)
- chunks (mono S `--ink-2`, "1 chunk" / "78 chunks")
- uploader (18 px avatar + body S name + soft time)
- channel (Tag, clickable)
- key info (Tag: "grove-keys · ep 14" / "custom · 3 holders" /
  "ephemeral · 3d"; ephemeral uses `--whisper`)
- action ("fetch" not-yet-downloaded / "open" once cached /
  `more-horizontal` meatball for copy hash, open in channel, delete
  reference)

**Delete reference.** Owner-admin action. Confirm dialog copy:

> delete this file from the grove's references?
> holders will purge their cached copy on next sync. content-addressed
> data cannot be unpublished from peers who already have it.

Confirm button is `--err`. Emits the new `DereferenceFile` event.

**Not-available state.** When every holder has purged the chunk, the
row goes to opacity 0.55 and the action slot reads
"not available · last seen {when}" (meta typography, `--ink-3`). The
fetch button becomes "ask the archive for it" (pings SyncProvider
peers).

**Mobile.** Two-line rows: filename + kind icon on line 1; size +
uploader + soft time on line 2. Filters collapse into a single
"filter" button that opens a bottom sheet.

## Event log

**Purpose.** Read-only, filterable, exportable audit of every signed
state event in the grove.

**Header.** Display M italic "event log". Right-side meta strip:
live counter pill "live · {n} events/min" while receiving (swaps to
"caught up" when local head == network head); mono "HLC ordering";
mono "ed25519 · signed". Intro copy:

> willow is event-sourced. this grove's state — channels, roles,
> keys, invites, messages — is a replay of signed events. HLC
> timestamps merge concurrent writes from different peers without a
> central clock.

### Desktop layout (≥ 900 px)

Two-column grid `1fr 260`: timeline on the left, sync-state sidebar on
the right.

**Timeline.** Sticky header row (mono S, uppercase, `--ink-3`):
`SEQ  HLC  KIND  BY  META  STATE`. Data rows: mono M, 32 px tall,
`--line-soft` bottom border:

- `#{seq}` in `--ink-3`
- HLC in `--ink-2`
- Kind coloured by the table below
- By = 16 px avatar + lowercase display name; system events render
  italic `system` in `--ink-3`
- Meta: event-kind-specific payload summary, `--ink-2`, truncated
- State: Tag "ok" (`--moss-3`) or "merged" (`--amber`) for HLC-
  reordered events

Kind colours:

| EventKind                                         | Colour        |
|---------------------------------------------------|---------------|
| `Message`                                         | `--ink-1`     |
| `EditMessage`                                     | `--amber`     |
| `DeleteMessage`                                   | `--err`       |
| `Reaction`                                        | `--moss-3`    |
| `Propose`, `Vote`                                 | `--willow`    |
| `GrantPermission`                                 | `--moss-3`    |
| `RevokePermission`                                | `--err`       |
| `CreateChannel`, `RenameChannel`                  | `--moss-3`    |
| `DeleteChannel`                                   | `--err`       |
| `CreateRole`, `DeleteRole`, `SetPermission`, `AssignRole` | `--amber` |
| `RotateChannelKey`                                | `--whisper`   |
| `PinMessage`, `UnpinMessage`                      | `--moss-3`    |
| `CreateServer`, `RenameServer`, `SetServerDescription` | `--willow` |
| `SetProfile`                                      | `--ink-2`     |

Day grouping uses the sticky Fraunces-italic day separators from
`message-row.md`.

**Expand-to-raw-payload.** Click or Enter on a focused row expands it
to a mono JSON preview of the full `Event` struct (`hash`, `author`,
`seq`, `prev`, `deps`, `kind`, `sig`, `timestamp_hint_ms`). Long
fields truncate with a "copy full value" affordance. A focused row's
`,` key copies the hash to clipboard.

**Virtualisation.** Logs ≥ 500 events use fixed-row recycling; the
`shimmer` keyframe from foundation renders while paginating older
events (fetched lazily on scroll-to-bottom, 1000 rows initial).

### Right column — Sync state

Three grouped sections (mono, `--ink-2`):

1. **sync state** — local head seq, network head seq, status
   ("in sync" `--moss-3`; "{n} behind" `--amber`; "divergent" `--err`).
2. **peers** — each known peer with head seq and connection kind;
   colours per reference: direct `--moss-3`, storage `--willow`,
   relay `--ink-3`, queued `--amber`.
3. **worker nodes** — replay, storage, relay workers with presence
   dots (`--moss-3` online / `--amber` degraded / `--ink-4` absent).

Hidden on mobile; reachable via a "sync state" pill in the header
that opens a bottom sheet.

### Filters

Filter bar above the sticky column headers (36 px, `--bg-0`,
`--line-soft` bottom border). As a `<form>` with
`aria-label="event log filters"`:

- kind (multi-select pill group, grouped: "authority", "structure",
  "chat", "crypto", "identity"; "more…" popover exposes each
  EventKind individually)
- author (chip dropdown + "find a peer" search)
- date range (last hour / today / this week / last 30 days / custom
  two-calendar picker)
- free-text search (`role="searchbox"`, placeholder
  "search event meta…", debounced 120 ms)

Filters are AND across types / OR within. Right-side "clear all"
link resets the form.

### Export

Right edge of the filter bar: mono button "export this log as JSONL"
with the `copy` icon. Exports the *currently filtered* set, HLC-
ordered, newline-delimited. Filename:
`{grove-id}-eventlog-{iso-date}.jsonl`. Streamed for large exports
with a thin `--moss-2` progress bar at the bottom of the timeline
(tap to cancel). Events that fail signature verification are omitted;
a JSON meta line at the top reports the count.

### Mobile layout

Single-column. Filter bar collapses to a "filter" button → bottom
sheet. Sync state lives in the header pill. Rows are 2-line cards;
tap expands the payload inline with `--motion-slow`.

## Copy (exact)

All strings are normative; variables in braces.

**Section headers / tabs.** "governance", "manage", "event log",
"roles & permissions", "invites", "files".

**Role labels.** "owner", "steward", "member", "guest",
"root of trust", "holds every permission implicitly".

**Invite.** "letter of introduction",
"create new letter of introduction", "seal this letter",
"grants {role}", "{x}/{y} uses", "∞ uses", "revoke",
"revoked {time}", "exhausted", "expired", "for {note}",
"invites are human-readable fingerprints. paste one from someone you
trust, or share yours by voice.", "this letter was signed by {self}.
when they arrive, compare fingerprints."

**Governance.** "every change to this grove is a signed event. the
owner is the root. stewards carry delegated authority. you can trace
any member's permissions back to the owner who granted them.",
"granted by {peer} · {time}", "authority since revoked",
"you cannot revoke your own ownership. transfer first.",
"learn about transfer", "the grove is quiet. you're the only peer
with authority.", "no authority changes in the last 30 days. a
steady grove.", "only stewards can see governance in this grove.",
"that grant was rejected by your permissions."

**Permission human-names** (used in matrix headers, role summaries,
role-edit sheet, invite role previews, profile-card permission line):

- "can sync history" — `Permission::SyncProvider`
- "can manage channels" — `Permission::ManageChannels`
- "can manage roles" — `Permission::ManageRoles`
- "can kick members" — derived (`ProposedAction::KickMember`)
- "can send messages" — `Permission::SendMessages`
- "can create invites" — `Permission::CreateInvite`
- "administrator" — derived (`ProposedAction::GrantAdmin`)

**Files.** "files shared in this grove",
"content-addressed · sha-256", "chunked at 256 KiB",
"keys per chunk", "fetch", "open",
"delete this file from the grove's references?",
"holders will purge their cached copy on next sync. content-addressed
data cannot be unpublished from peers who already have it.",
"not available · last seen {when}", "ask the archive for it".

**Event log.** "willow is event-sourced. this grove's state —
channels, roles, keys, invites, messages — is a replay of signed
events. HLC timestamps merge concurrent writes from different peers
without a central clock.", "live · {n} events/min", "caught up",
"HLC ordering", "ed25519 · signed", "export this log as JSONL",
"clear all", "search event meta…", "ok", "merged", "system".

## Data dependencies

Flagged [existing] (already in `willow-state`) or [new].

- [existing] `EventKind::GrantPermission` / `RevokePermission` —
  matrix toggle and the "granted by" authorship in the chain view.
  `apply_event` records the granter as the event author.
- [existing] `ProposedAction::GrantAdmin` / `RevokeAdmin` —
  governance path for the "administrator" column; surfaces via the
  propose sheet.
- [existing] `ProposedAction::KickMember` — "can kick members" column
  and the per-row kick action.
- [existing] `EventKind::CreateRole`, `DeleteRole`, `SetPermission`,
  `AssignRole` — role create/edit save path.
- [existing] `EventKind::CreateChannel`, `DeleteChannel`,
  `RenameChannel`, `RotateChannelKey`, `PinMessage`, `UnpinMessage`,
  `Propose`, `Vote`, `SetProfile`, `RenameServer`,
  `SetServerDescription` — rendered in the event log.
- [new] **role presets** — `steward`/`member`/`guest` labels and
  bundled permissions are a *UX convention* over existing events. No
  new EventKind. Canonical role ids (`willow:preset:{name}`) mark
  presets; the client seeds them idempotently on first Manage visit
  via `CreateRole` + `SetPermission` only if missing.
- [new] **custom role colour** — add optional `colour` field on
  `Role` plus `EventKind::SetRoleColour { role_id, colour }`. Colour
  values are foundation accent token names.
- [new] **role description** — `description: Option<String>` on
  `Role` plus `EventKind::SetRoleDescription { role_id, description }`.
- [new] **invite role attachment** — thread an optional `role_id`
  through the invite container; on accept, emit `AssignRole`. Must
  be co-specified with `onboarding.md` (which owns the invite
  container).
- [new] **invite note** — UI-only annotation in the invite container.
- [new] **file index** — pure projection over `Message` events with
  attachments; derived `FileRef` index in `willow-state`. No event.
- [new] **`EventKind::DereferenceFile { file_hash }`** — owner-admin
  delete for the files tab. Holders purge on next sync; distributed
  chunks cannot be unpublished.
- [new] **file "last seen" signal** — runtime signal from
  `willow-network` reporting the last holder advertisement; not an
  event. Required for "not available" rows.
- [new] **chain-of-trust projection** — cached `grant_paths:
  BTreeMap<EndpointId, Vec<EndpointId>>` on `ServerState`, derived
  from `GrantPermission` event authors. Projection only.
- [new] **event log JSONL export** — purely client-side; PGP-signed
  export deferred.

## Edge cases

- **Owner offline long-term.** Governance still renders; above the
  matrix a soft `--amber` banner reads "permission changes will sync
  when the owner returns" — but only if the pending edit is on the
  governance path. Direct `GrantPermission` edits remain immediate.
  The banner dismisses when owner presence resumes.
- **Orphan admin.** The self-revoke guard prevents zero-owner state
  via UI; this only occurs from hand-rolled event injection.
  Governance then shows a red header card: "this grove has no owner.
  its event log is preserved, but no further admin actions can be
  taken." Matrix edits are disabled.
- **Revoke-your-own-administrator (non-owner admin).** Allowed only
  if ≥ 1 other admin besides the owner remains. Sole-extra-admin
  case: `--warn`-bordered tooltip "there is only one other steward.
  add another before stepping down."
- **Broken delegation chain.** Admin keeps permissions; the chain
  view shows dashed `--warn` border + "authority since revoked"
  meta. Future mutations from that branch verify against current
  authority and may be rejected.
- **Extremely long event log.** ≥ 100k events virtualise; filters
  narrow before the recycler instantiates; initial load fetches the
  most recent 1000, older events page lazily with `shimmer`.
- **Purged file references.** Zero-holder rows render at opacity
  0.55 with "not available · last seen {when}"; delete is removed,
  only "ask the archive for it" and "copy hash" remain.
- **Unknown role on an invite.** Deleted-role invites render the
  Tag "grants {role id} · deleted" (`--ink-3`), tooltip "this role
  no longer exists. accepting this letter still joins the grove but
  assigns no role."
- **Concurrent grant + revoke.** HLC wins; the matrix snaps to the
  final state with a subtle pop-in. No dedicated conflict UI — the
  event log is the ground truth.
- **Signature verification failure in export.** Events that fail
  verification are omitted; a JSON meta line at the head reads
  `{"# omitted": n, "reason": "signature verification failed"}`.

## Accessibility

- **Permission matrix** is a `<table>` with `<th scope="col">` /
  `<th scope="row">`. Each cell is a labelled checkbox; aria-label
  "grant {permission} to {member}" (or "revoke …" when checked).
  Owner row: `aria-disabled="true"`, aria-label "owner — holds every
  permission implicitly".
- **Chain of trust** is `<ol aria-label="chain of trust from owner">`
  with nested `<ol>` for nested grants.
- **Recent authority changes** is `role="feed"` with `aria-busy`
  toggled during live updates; each row is an `<article>` with
  `aria-posinset`/`aria-setsize`.
- **Event log** is `role="feed"`. Each row is an `<article>` with
  accessible name "{author} {kind-copy} {meta}". Expanded payload is
  `<pre aria-label="raw event payload">` that traps Tab while open.
- **Filter controls** live inside `<form aria-label="event log
  filters">`; free-text search uses `role="searchbox"`; "clear all"
  is the form reset.
- **Export** is a `<button>` with accessible name "export this log as
  JSONL". Progress uses `role="progressbar"` with standard ARIA
  value attributes; a sibling cancel button exists.
- **Role cards** are `<article>`. The colour dot is aria-hidden; the
  role's colour name is a visible text node, so colour is never the
  sole signifier.
- **Invite rows** use proper row/cell roles. Invite codes are real
  text (mono-typed); screen readers voice the words naturally.
- **Keyboard.** Matrix/feed/event-log rows are tabindex-0. Enter
  activates the primary action; Shift+Enter opens overflow; Space
  expands the raw payload in the event log.
- **Focus ring.** `--focus-ring` on every focusable element.
- **Reduced motion.** Row expand/collapse uses opacity only;
  `willowPulse` on the owner avatar and `willow-pop-in` cell fill
  both collapse to opacity.
- **Time strings.** Soft-time primary; full ISO timestamp available
  via each row's `aria-describedby` tooltip.

## Acceptance criteria

- [ ] Governance, Manage, and Event log tabs are hidden for peers
      without `Administrator` or owner; deep-link shows the 403 card.
- [ ] Governance renders the three-column layout at ≥ 900 px and
      stacks correctly below.
- [ ] Chain of trust traces every non-owner admin to the owner;
      broken chains display the `--warn` dashed border and
      "authority since revoked" meta.
- [ ] Toggling a non-admin, non-kick matrix cell emits
      `GrantPermission` or `RevokePermission` with the correct
      variant.
- [ ] Clicking "administrator" or a matrix-row kick action opens the
      propose sheet (not a direct toggle).
- [ ] Self-owner revoke opens the soft-warning dialog with exact
      copy and emits no event.
- [ ] Manage → Roles shows the three default presets with preset
      colours where the canonical preset ids exist.
- [ ] Creating a custom role emits `CreateRole` + per-permission
      `SetPermission` + (when supplied) `SetRoleColour` /
      `SetRoleDescription`.
- [ ] Manage → Invites creates a letter with attached role, expiry,
      max uses, and note; revoke transitions the row to burned.
- [ ] Manage → Files lists every file reference from `Message`
      events, filters as declared, renders "not available" when zero
      holders remain.
- [ ] Deleting a file reference emits `DereferenceFile`.
- [ ] Event log renders the most recent 1000 events in HLC order
      with the kind-colour table; scroll-to-bottom lazy-loads older
      events with `shimmer`.
- [ ] Expanding a row shows the full raw JSON payload; collapsing
      restores it.
- [ ] Filter form narrows the rendered list without refetching.
- [ ] Export produces JSONL of the filtered set in HLC order with
      the filename pattern `{grove-id}-eventlog-{iso-date}.jsonl`;
      failing-signature events are omitted with a meta line.
- [ ] Every "Copy (exact)" string appears verbatim in the UI.
- [ ] The seven permission human-names appear consistently in the
      matrix, role summaries, role-edit sheet, invite previews, and
      the profile-card permission line.
- [ ] WCAG AA contrast holds on the default theme.
- [ ] Reduced-motion users see no transform animations on row
      expand, matrix toggle, or chain-of-trust hover guides.
- [ ] Matrix virtualises at ≥ 60 rows and the log at ≥ 500 events;
      scroll stays at 60 fps on a mid-range laptop.

## Open questions

- **Owner transfer.** Self-revoke points at a transfer flow; transfer
  itself is out of scope. Proposal:
  `ProposedAction::TransferOwnership { new_owner }` with a two-person
  SAS confirmation, as its own child spec (`owner-transfer.md`) or an
  addition to `settings-tweaks.md`.
- **Role colour / description persistence.** Confirm the
  `SetRoleColour` / `SetRoleDescription` event path before
  implementation. Alternative: name-convention encoding.
- **Invite protocol extension.** Attached-role and note fields
  require coordination with `onboarding.md`, which owns the invite
  container.
- **Full propose-sheet surface.** Only entry points are specified
  here. Vote bar, ballot, and discussion thread belong in a
  dedicated `proposals.md` (modelled on the reference design's
  `ProposalCard`).
- **PGP-signed JSONL export.** Deferred; requires deterministic
  canonical serialisation and a client-held signing key.
- **Custom fine-grained permissions.** Future permissions (e.g.
  "can manage invites" distinct from `CreateInvite`) add columns; no
  matrix redesign needed.
- **Per-grove accent.** If `foundation.md`'s per-grove accent
  override lands, column-1 connector lines and matrix fill should
  honour it; whisper violet stays reserved for whisper mode.
