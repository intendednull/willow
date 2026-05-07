# Willow docs

Master index of Willow's design specs, implementation plans, and reports. Grouped by feature area for discovery and onboarding.

For build/test/dev commands and deep architecture notes, see `../CLAUDE.md`. This file does not duplicate that content.

## Where to start (new agents and humans)

Read these in order to get the conceptual map of Willow:

1. [State authority and mutations](specs/2026-04-12-state-authority-and-mutations.md) — how every state change is authorized.
2. [Per-author Merkle DAG state](specs/2026-04-01-per-author-merkle-dag-state-design.md) — the event-sourced state model.
3. [State management model](specs/2026-04-26-state-management-model-design.md) — actors, locks, and shared mutable state.
4. [E2E test architecture](specs/2026-04-21-e2e-test-architecture-design.md) — which test tier covers which behavior.

Then skim the catalog below for the area you are working in.

## Document types

Three document types, each with one job. If a doc does not fit one of these, the type list is wrong, not the doc.

- **Spec** (`specs/`) — *what we are building toward.* Target shape of the code: types, traits, invariants, public API. Long-lived, canonical.
- **Plan** (`plans/`) — *how we get from current code to the target.* Migration steps, file-by-file changes, PR breakdown. Cites the spec it realizes. Goes stale once shipped.
- **Report** (`reports/`) — *findings from a one-shot investigation.* Audits, post-mortems. Dated, immutable.

## Status tags

Every entry below carries one of:

- `[draft]` — being written, target not yet stable.
- `[active]` — current target / in-flight migration.
- `[landed]` — realized in code; canonical reference.
- `[superseded]` — replaced; entry links to successor.

## Catalog

### State & Authority

**Specs**

_(populated in Task 4)_

**Plans**

_(populated in Task 4)_

### Networking & Sync

**Specs**

_(populated in Task 4)_

**Plans**

_(populated in Task 4)_

### Identity, Crypto & Trust

**Specs**

_(populated in Task 4)_

**Plans**

_(populated in Task 4)_

### Messaging

**Specs**

_(populated in Task 4)_

**Plans**

_(populated in Task 4)_

### Workers & Actors

**Specs**

_(populated in Task 4)_

**Plans**

_(populated in Task 4)_

### Web UI & UX

**Specs**

_(populated in Task 4)_

**Plans**

_(populated in Task 4)_

See also: [`plans/STATUS.md`](plans/STATUS.md) — point-in-time audit of which UI-phase plans have landed.

### Agent / MCP

**Specs**

_(populated in Task 4)_

**Plans**

_(populated in Task 4)_

### Testing

**Specs**

_(populated in Task 4)_

**Plans**

_(populated in Task 4)_

**Reports**

_(populated in Task 4)_

### Process & Tooling

**Specs**

_(populated in Task 4)_

**Plans**

_(populated in Task 4)_

## Reference designs

Archived design bundles (immutable inputs to specs, not specs themselves) live in [`reference-designs/`](reference-designs/README.md).

## Conventions

Cemented in [`specs/2026-05-07-docs-organization-design.md`](specs/2026-05-07-docs-organization-design.md). Mirrored on demand by the `organizing-willow-docs` skill. Summary:

### Naming

| Type | Pattern | Example |
|---|---|---|
| Spec | `specs/YYYY-MM-DD-<kebab>-design.md` | `2026-05-07-docs-organization-design.md` |
| Multi-file spec | `specs/YYYY-MM-DD-<kebab>/README.md` + children | `2026-04-19-ui-design/README.md` |
| Plan | `plans/YYYY-MM-DD-<kebab>.md` | `2026-04-21-e2e-test-architecture.md` |
| Report | `reports/YYYY-MM-DD-<kebab>.md` | `2026-04-13-test-audit.md` |

The date is when the doc was written, not the implementation target. The `-design.md` suffix on specs is what visually distinguishes specs from plans in `ls` output. Existing files predating this convention are not renamed.

### Document headers

Every new spec, plan, and report opens with:

```
**Date:** YYYY-MM-DD
**Status:** draft | active | landed | superseded
**Spec:** specs/...      (plans only — REQUIRED, points at the spec being realized)
**Supersedes:** specs/... (if applicable)
```

### Nested folders

Use a folder (`specs/YYYY-MM-DD-<topic>/README.md` + children) only when one logical document is too large for a single file *and* the children are tightly coupled. Children use kebab-case topic names (no date prefix — they inherit the parent's date). Maximum one level deep. Multiple independent docs sharing a topic stay flat — example: the `ui-phase-1a` … `ui-phase-2e` plan series.

### Adding a new spec, plan, or report

1. Pick the right type (spec = target, plan = migration, report = audit).
2. Name with `YYYY-MM-DD-<kebab>-design.md` (spec) or `YYYY-MM-DD-<kebab>.md` (plan/report).
3. Add a one-line entry to this README under the right area, with a 5–15 word summary and `[draft]` tag.
4. Plans must include `**Spec:** specs/...` in their header.
5. Multi-file specs nest under `YYYY-MM-DD-<topic>/` with a required `README.md`.
