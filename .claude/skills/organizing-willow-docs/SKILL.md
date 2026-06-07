---
name: organizing-willow-docs
description: Conventions for adding, naming, and organizing specs, plans, and reports in docs/. Use when creating a new spec/plan/report, modifying the docs structure (adding a feature area, splitting into nested folder, superseding a doc), or reorganizing the catalog. Mirrors docs/README.md and docs/specs/2026-05-07-docs-organization-design.md.
---

# Organizing Willow docs

Project-local skill mirroring the cemented conventions for the `docs/` tree.

**Source of truth:** [`docs/README.md`](../../../docs/README.md) (master index, self-documenting) and [`docs/specs/2026-05-07-docs-organization-design.md`](../../../docs/specs/2026-05-07-docs-organization-design.md) (the design spec). If this skill ever drifts from those files, the README and spec are right. Update this skill in the same commit when conventions change.

## When to use this skill

Invoke before:

- Adding a new spec, plan, or report.
- Modifying the docs structure (adding a feature area to the catalog, splitting a spec into a nested folder, superseding or deprecating a doc).
- Reorganizing the catalog.

If you are *reading* docs (not changing them), go to `docs/README.md` directly — that is the catalog.

## Document types

Three types, each with one job. If a doc does not fit one of these, the type list is wrong, not the doc.

- **Spec** (`docs/specs/`) — *what we are building toward.* Target shape of the code: types, traits, invariants, public API, architectural boundaries. May briefly note current state for contrast, but the bulk is the destination, not the journey. Long-lived, canonical.
- **Plan** (`docs/plans/`) — *how we get from current code to the target.* Migration steps, file-by-file changes, ordering, risks, test strategy, PR-level breakdown. Cites the spec it realizes. Goes stale once shipped.
- **Report** (`docs/reports/`) — *findings from a one-shot investigation.* Audits, post-mortems, performance investigations. Dated, immutable.

Implications:

- A spec can have multiple plans (large target, multiple PR-sized chunks).
- A spec without a plan is fine (target known, path deferred).
- A plan without a spec is suspicious — flag it during review.

## Naming

| Type | Pattern | Example |
|---|---|---|
| Spec | `docs/specs/YYYY-MM-DD-<kebab>-design.md` | `2026-05-07-docs-organization-design.md` |
| Multi-file spec | `docs/specs/YYYY-MM-DD-<kebab>/README.md` + children | `2026-04-19-ui-design/README.md` |
| Plan | `docs/plans/YYYY-MM-DD-<kebab>.md` (no `-design`) | `2026-04-21-e2e-test-architecture.md` |
| Report | `docs/reports/YYYY-MM-DD-<kebab>.md` | `2026-04-13-test-audit.md` |

The date is **when the doc was written**, not the implementation target. The `-design.md` suffix on specs is what visually distinguishes specs from plans in `ls` output. Plans omit it.

**Existing files predating these rules are not renamed.** The convention applies to new docs only; the master index labels older entries explicitly so the missing suffix does not affect discovery.

## Document headers

Every new spec, plan, and report opens with:

```
**Date:** YYYY-MM-DD
**Status:** draft | active | landed | superseded
**Spec:** docs/specs/...      (plans only — REQUIRED, points at the spec being realized)
**Supersedes:** docs/specs/... (if applicable)
```

Status semantics:

- `draft` — being written, target not yet stable.
- `active` — current target / in-flight migration.
- `landed` — realized in code; canonical reference.
- `superseded` — replaced; header links to successor.

The status tag is a discovery aid, not a project-management tool. Stale tags are tolerable; missing entries in the master index are not.

## Nested folders

Use a folder (`docs/specs/YYYY-MM-DD-<topic>/`) only when one logical document is too large for a single file *and* its children are tightly coupled — they lose meaning without the parent.

Rules:

- The parent `README.md` is **required**. It states the folder's purpose and links every child.
- Children use kebab-case topic names with **no date prefix** — they inherit the parent's date.
- Children are facets of one design, not phase numbers. Phases imply ordering; children do not.
- Maximum one level deep. If a child needs its own children, promote it to a top-level spec.

Multiple independent documents that share a topic are flat siblings, not children — example: `docs/plans/2026-04-20-ui-phase-1a-desktop-shell.md`, `…1b-mobile-shell.md`, etc. Each ships independently.

## Adding a new spec, plan, or report

1. **Pick the right type.** Spec = target. Plan = migration. Report = audit.
2. **Name it.** `YYYY-MM-DD-<kebab>-design.md` (spec) or `YYYY-MM-DD-<kebab>.md` (plan/report). Date is today.
3. **Write the header.** All four fields where applicable. `Spec:` is required for plans.
4. **Add a catalog entry.** One line under the right area in `docs/README.md`:
    ```markdown
    - [Title](specs/YYYY-MM-DD-name-design.md) — 5–15 word summary. `[draft]`
    ```
5. **Pick the area.** State & Authority, Networking & Sync, Identity/Crypto/Trust, Messaging, Workers & Actors, Web UI & UX, Agent / MCP, Testing, or Process & Tooling. If a doc spans areas, file it under its primary area.
6. **Commit the doc and the README entry together.** The catalog must not lag the file.

## Modifying the structure

- **Adding a feature area:** rare. Adds an `### ` header to the catalog plus the area name to step 5 above. Update the spec at `docs/specs/2026-05-07-docs-organization-design.md` and this skill in the same commit.
- **Promoting a spec to a nested folder:** rename `<topic>-design.md` → `<topic>/README.md`. Children are added later as kebab-case files (no date). Update the catalog entry to point at the folder's `README.md`.
- **Superseding a doc:** add `**Supersedes:**` to the new doc's header and `[superseded]` plus a link to the successor in the old doc's catalog entry. Do NOT delete the old doc.
