# Docs organization — target structure

**Date:** 2026-05-07
**Status:** landed (migration plan landed 2026-05-07; README + skill + CLAUDE.md pointer in place; STATUS frontmatter backfilled across all landed plans in PRs #651 / #652)
**Implementation plan:** [`docs/plans/2026-05-07-docs-organization.md`](../plans/2026-05-07-docs-organization.md)

## Purpose

Define the target structure of `docs/` so agents and humans can find specs,
plans, and reports without grepping. Cement the conventions for what each
document type is, how it is named, where it lives, and how it appears in the
master index. Once realized, this spec is the canonical reference for any
future doc added to the project.

The migration from the current state to this target lives in a separate plan
under `docs/plans/`, per the spec/plan distinction defined below.

## Document types

Three document types, each with a single job. The split is the spine of the
whole structure — if a doc does not fit one of these, the type list is wrong,
not the doc.

### Spec — what we are building toward

Lives in `docs/specs/`. Describes the *target* shape of the code: types,
traits, invariants, public API, architectural boundaries. May briefly note
current state for contrast, but the bulk is the destination, not the journey.

Specs stay small because they do not repeat code that already exists. A spec
is canonical and long-lived; it is the reference plans and reviewers point at.
"Design rationale" and "why approach X over Y" belong here, because they
describe the destination's shape.

### Plan — how we get from current code to the target

Lives in `docs/plans/`. Describes the *migration*: current state, file-by-file
changes, ordering, risks, test strategy, PR-level breakdown. A plan cites the
spec it is realizing.

Plans go stale once shipped — they are an artifact of the journey, not the
destination. "How to refactor the existing 800-line file" belongs here, not
in the spec.

### Report — findings from a one-shot investigation

Lives in `docs/reports/`. Audits, post-mortems, performance investigations.
Dated, immutable, does not define future direction.

### Implications of the split

- A spec can have multiple plans (large target, multiple PR-sized chunks).
- A spec without a plan is fine (target known, path deferred).
- A plan without a spec is suspicious — flag it during review.
- When the target evolves, the spec is updated and a new plan is written;
  old plans are not rewritten retroactively.

## Top-level layout

```
docs/
├── README.md            master index — entry point for agents and humans
├── specs/               target state — what we are building toward
├── plans/               migration steps — how we get there
├── reports/             one-shot audits and investigations
└── reference-designs/   archived Claude Design bundles (immutable)
```

`docs/design/` does not exist in the target; design documents live in
`docs/specs/` (or as a multi-file spec folder under `docs/specs/`).

## Naming conventions

| Type | Pattern | Example |
|---|---|---|
| Spec | `docs/specs/YYYY-MM-DD-<kebab>-design.md` | `2026-05-07-docs-organization-design.md` |
| Multi-file spec | `docs/specs/YYYY-MM-DD-<kebab>/README.md` + children | `2026-04-19-ui-design/README.md` |
| Plan | `docs/plans/YYYY-MM-DD-<kebab>.md` (no `-design`) | `2026-04-21-e2e-test-architecture.md` |
| Report | `docs/reports/YYYY-MM-DD-<kebab>.md` | `2026-04-13-test-audit.md` |

The date is **when the doc was written**, not the implementation target.

The `-design.md` suffix on specs is what visually distinguishes a spec from a
plan in `ls` output. Plans omit it.

## Document headers

Every new spec, plan, and report opens with a small header. Existing files
predating this convention are not retrofitted (see *Non-goals*).

```
**Date:** YYYY-MM-DD
**Status:** draft | active | landed | superseded
**Spec:** docs/specs/...               (plans only — REQUIRED, points at the spec being realized)
**Supersedes:** docs/specs/...         (if applicable)
**Parent specs:** docs/specs/...       (optional — for specs that descend from one or more existing specs)
**Implementation plan:** docs/plans/...(optional — back-pointer when the doc is a spec and the plan is known)
```

The first four fields are the minimum. `**Parent specs:**` and
`**Implementation plan:**` are optional extensions used when the doc lives in
a richer cross-reference graph (e.g. a child spec descending from a multi-file
parent, or a freestanding spec whose realizing plan is already known).
Additional `**Key:**` extensions are allowed when they make navigation
cheaper for the next reader — keep them rare and self-explanatory.

Status semantics:

- `draft` — being written, target not yet stable
- `active` — current target / in-flight migration
- `landed` — realized in code; canonical reference
- `superseded` — replaced; header links to successor

## Nested folder convention

A spec may be split across multiple files when one logical document is too
large for a single file *and* the children are tightly coupled — they lose
meaning without the parent.

```
docs/specs/2026-04-19-ui-design/
├── README.md         REQUIRED. Parent doc: purpose, scope, non-goals, child links.
├── foundation.md     children use kebab-case, NO date prefix
├── composer.md
└── ...
```

Rules:

- The parent `README.md` is required. It states the folder's purpose and links
  every child.
- Children do not carry their own date — they inherit the parent's date. If a
  child genuinely needs its own date, it is a separate spec, not a child.
- Children are kebab-case topic names, not phase numbers. Phases imply
  ordering; children are facets of one design.
- The same nesting rule applies to plans, but use is rare — plans usually
  stay flat. A nested plan folder represents one large migration broken into
  chapters, not multiple independent plans.
- Do not nest more than one level deep. If a child grows large enough to need
  its own children, promote it to a top-level spec.

Multiple independent documents that share a topic are flat siblings, not
children — example: `ui-phase-1a-desktop-shell.md`, `ui-phase-1b-mobile-shell.md`
in `docs/plans/`. Each ships independently, each stands alone.

## Master index — `docs/README.md`

The entry point. Four sections, in this order.

### 1. Orientation

Eight or so lines that tell a new agent or human:

- What this file is.
- Where to start if new (a curated reading list of 3–5 foundational specs).
- Pointer to `CLAUDE.md` for build/test/dev commands. The README does not
  duplicate that content.

### 2. Document type primer

Three short blocks recapping the spec / plan / report distinction. Self-
contained so an agent landing here does not need to read this design doc or
`CLAUDE.md` to understand the catalog.

### 3. Catalog by feature area

Nine areas, ordered foundations-first:

1. **State & Authority** — event-sourced state machine, permissions, mutations
2. **Networking & Sync** — iroh, gossip, relay, history sync, negentropy
3. **Identity, Crypto & Trust** — Ed25519, encryption, key rotation, sealed DMs, trust verification
4. **Messaging** — message store, HLC, channel structure
5. **Workers & Actors** — actor framework, replay/storage worker nodes
6. **Web UI & UX** — Leptos client, target UX bundle, navigation, async refactor
7. **Agent / MCP** — agentic peer API, LLM agent UX
8. **Testing** — test architecture, multi-peer E2E, event-based waits
9. **Process & Tooling** — error prefixes, identifier formats, deploy

Each area is a `## ` header containing a **Specs** subsection and a **Plans**
subsection. Entries are one line each:

```markdown
- [Title](specs/YYYY-MM-DD-name-design.md) — 5–15 word summary. `[status]`
```

If a doc spans areas, it appears in its primary area only. The summary tells
the reader if it is also relevant to adjacent topics.

A nested-folder spec appears as one entry pointing at its `README.md`. The
index does not enumerate children — that is the parent README's job.

`docs/plans/STATUS.md` is linked from the **Web UI & UX** section as a one-off
snapshot audit, not enumerated as a normal plan entry.

### 4. Conventions

The cemented rules — naming, document types, when to nest, how to add a new
spec/plan/report. Same content as this design, distilled into reference form.
Includes a short "Adding a new spec/plan" checklist:

1. Pick the right type (spec = target, plan = migration, report = audit).
2. Name with `YYYY-MM-DD-<kebab>-design.md` (spec) or `YYYY-MM-DD-<kebab>.md`
   (plan/report).
3. Add an entry to `docs/README.md` under the right area with a 5–15 word
   summary and `[draft]` tag.
4. Plans must reference their spec in the header.
5. Multi-file specs nest under `YYYY-MM-DD-<topic>/` with a required `README.md`.

## Discovery surfaces

The conventions in this spec are surfaced to agents and humans through three
deliberately redundant channels. Each has a different access pattern; together
they ensure an agent always finds the rules without grepping.

### Primary — `docs/README.md`

The master index. Self-documenting: the conventions live in section 4
alongside the catalog so anyone browsing the docs sees them in passing. This
is the canonical copy. If the other two surfaces drift, the README is right.

### Skill mirror — `organizing-willow-docs`

A project-local skill at `.claude/skills/organizing-willow-docs/SKILL.md`
that mirrors the conventions and the "Adding a new spec/plan" checklist. An
agent loads this skill on demand when:

- Adding a new spec, plan, or report.
- Modifying the structure (adding a feature area, splitting a spec into a
  nested folder, deprecating/superseding a doc).
- Reorganizing the catalog.

The skill exists for two reasons:

1. **Discoverability via metadata.** Skills surface in the agent's
   available-skills list with their `description`; `docs/README.md` does not.
   An agent with no prior context can find the skill from its trigger
   description alone.
2. **Token efficiency.** The skill loads only the rules — not the catalog —
   so an agent making structural changes does not need to read 60+ catalog
   entries to find the conventions.

The skill is not the source of truth. It points back at this spec and at
`docs/README.md` for the canonical text. When the canonical text changes,
the skill is updated in the same commit.

### Pointer — `CLAUDE.md`

`CLAUDE.md` retains all build/test/dev/architecture content. The "Specs &
Plans" line in *Code Conventions* is replaced with a short pointer to both
of the surfaces above:

> **Docs entry point:** `docs/README.md` is the master index of specs and
> plans, grouped by feature area. Read it before adding any new spec or plan,
> or before searching for an existing one. The `organizing-willow-docs` skill
> mirrors the conventions for on-demand loading. Cemented in
> `docs/specs/2026-05-07-docs-organization-design.md`.

The trailing back-pointer to this spec gives the reader a one-hop path to the
canonical rules when they want more than the pointer paragraph itself
contains.

Existing CLAUDE.md sections that duplicate doc-discovery information (e.g.
the per-task "see `docs/specs/...`" pointers in *Architecture Notes*) are
left in place — they are useful inline context, not redundant with the index.

## Non-goals

- **Renaming existing files.** Files that predate this convention (e.g.
  specs without the `-design.md` suffix) stay as-is to preserve git history
  and existing links. The index resolves any ambiguity by labeling them
  explicitly. The convention applies to *new* docs only.
- **Migrating `docs/plans/STATUS.md` into the index.** STATUS is a
  point-in-time audit with a different shape; the index links to it but does
  not absorb it.
- **A status-tracking system.** The `[status]` tag is a discovery aid, not a
  project-management tool. Stale tags are tolerable; missing entries are not.
- **Per-area sub-READMEs.** Single-file index until volume justifies a split.

## Resolved questions

The migration closed out two open questions; recorded here so future readers
don't re-litigate them.

- **Same-date spec/plan pairs.** Pairs like
  `2026-04-12-state-authority-and-mutations.md` and
  `2026-04-12-willow-channel-removal.md` were verified during the migration:
  each is a genuine spec + plan pair (the plan's header cites the spec). No
  misclassifications were found. Future same-date pairs are legitimate and
  expected when a target and its migration are designed together.
- **Nine-area catalog.** The nine areas in §3 populated cleanly with no
  entries that resisted a home. The area list is stable. Future areas can be
  added if a new feature cluster justifies one — the convention does not cap
  the count.
