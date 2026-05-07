# Docs Organization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Date:** 2026-05-07
**Status:** draft
**Spec:** docs/specs/2026-05-07-docs-organization-design.md

**Goal:** Realize the docs-organization spec — populate `docs/README.md` as the master index, mirror conventions in a project-local skill, fold the `docs/design/` orphan into `docs/specs/`, and update `CLAUDE.md` to point at the new entry surfaces.

**Architecture:** Three deliberately redundant discovery surfaces. `docs/README.md` is canonical and self-documenting (4 sections: orientation, primer, 9-area catalog, conventions). `.claude/skills/organizing-willow-docs/SKILL.md` mirrors the conventions for on-demand loading. `CLAUDE.md` carries a thin pointer to both. One-time cleanup folds the lone file in `docs/design/` into `docs/specs/` and removes the directory. No existing files are renamed.

**Tech Stack:** Markdown only. No build tooling required. Verification is a small bash link-checker.

---

## File Structure

Files created, modified, moved, or deleted by this plan:

| Action | Path | Responsibility |
|---|---|---|
| Create | `docs/README.md` | Master index — orientation, doc-type primer, 9-area catalog, conventions |
| Create | `.claude/skills/organizing-willow-docs/SKILL.md` | Skill mirror of conventions; loaded when adding/modifying specs/plans |
| Move | `docs/design/llm-agent-ux-spec.md` → `docs/specs/2026-04-25-llm-agent-ux-spec-design.md` | Apply naming convention to the orphan; original git-creation date is `2026-04-25` |
| Delete | `docs/design/` | Empty after the move; remove directory |
| Modify | `CLAUDE.md:25-32` | Update Repository Structure tree (drop `design/`, mention `README.md`) |
| Modify | `CLAUDE.md:292` | Replace "Specs & Plans" line in *Code Conventions* with pointer to README + skill |
| Verify only | `docs/specs/2026-04-12-state-authority-and-mutations.md` ↔ `docs/plans/2026-04-12-state-authority-and-mutations.md` | Confirm legitimate spec+plan pair, not duplicates |
| Verify only | `docs/specs/2026-04-12-willow-channel-removal.md` ↔ `docs/plans/2026-04-12-willow-channel-removal.md` | Confirm legitimate spec+plan pair, not duplicates |

The plan itself (`docs/plans/2026-05-07-docs-organization.md`) is also added as a catalog entry under *Process & Tooling* in Task 5.

---

## Task 1: Verify the suspected duplicate filenames are spec+plan pairs

The same date+kebab combo appears in both `docs/specs/` and `docs/plans/` for two topics. Under the new convention this is the correct pattern (a spec and its plan share a date), but the pre-convention origin of these files makes verification cheap and worth doing now. If either is misclassified, fix it before the catalog enumerates it.

**Files:**
- Read: `docs/specs/2026-04-12-state-authority-and-mutations.md` (first 30 lines)
- Read: `docs/plans/2026-04-12-state-authority-and-mutations.md` (first 30 lines)
- Read: `docs/specs/2026-04-12-willow-channel-removal.md` (first 30 lines)
- Read: `docs/plans/2026-04-12-willow-channel-removal.md` (first 30 lines)

- [ ] **Step 1: Read the four files and classify each**

For each pair, run:

```bash
head -30 docs/specs/2026-04-12-state-authority-and-mutations.md
head -30 docs/plans/2026-04-12-state-authority-and-mutations.md
head -30 docs/specs/2026-04-12-willow-channel-removal.md
head -30 docs/plans/2026-04-12-willow-channel-removal.md
```

Classify each file:
- **Spec** if it describes a target shape (types, traits, invariants, target API).
- **Plan** if it describes migration steps (current state, file-by-file changes, PR breakdown).

Expected: each pair has one of each. If a `docs/specs/` file is actually a plan, or vice versa, note it.

- [ ] **Step 2: If all four are correctly classified, record the finding**

Write a single-line confirmation to your scratchpad: "Both 2026-04-12 pairs are legitimate spec+plan pairs (no migration needed)." Move on to Task 2.

- [ ] **Step 3: If any file is misclassified, `git mv` it to the correct directory**

For example, if `docs/specs/2026-04-12-foo.md` is actually a plan:

```bash
git mv docs/specs/2026-04-12-foo.md docs/plans/2026-04-12-foo.md
```

Per the spec's *Non-goals*, do NOT rename the file's stem (no adding/removing `-design.md`) — only move it between directories. Existing inbound links are preserved by the directory move only if the filename is unchanged.

- [ ] **Step 4: Commit the finding (or fix)**

If no fix was needed:

```bash
# No commit — proceed to Task 2.
```

If a fix was needed:

```bash
git add -A
git commit -m "docs: reclassify <filename> as <plan|spec>

Verified during docs-organization migration (see docs/plans/2026-05-07-docs-organization.md, Task 1).
The file describes <target shape | migration steps>, so it belongs in <docs/specs/ | docs/plans/>."
```

---

## Task 2: Move the `docs/design/` orphan into `docs/specs/` and remove the directory

`docs/design/` contains exactly one file (`llm-agent-ux-spec.md`). The target layout has no `docs/design/` — design documents live in `docs/specs/`. Apply the naming convention using the file's git-creation date (`2026-04-25`).

**Files:**
- Move: `docs/design/llm-agent-ux-spec.md` → `docs/specs/2026-04-25-llm-agent-ux-spec-design.md`
- Delete: `docs/design/` directory

- [ ] **Step 1: Confirm `docs/design/` contains only the one file**

Run:

```bash
ls -la docs/design/
```

Expected: exactly one file, `llm-agent-ux-spec.md`. If anything else is there, stop and ask the user — the plan assumed a single orphan.

- [ ] **Step 2: `git mv` the file with the new name**

```bash
git mv docs/design/llm-agent-ux-spec.md docs/specs/2026-04-25-llm-agent-ux-spec-design.md
```

The `git mv` preserves history.

- [ ] **Step 3: Remove the now-empty `docs/design/` directory**

```bash
rmdir docs/design
```

Expected: command succeeds silently. If `rmdir` complains about a non-empty directory, stop — Step 1's assumption was wrong.

- [ ] **Step 4: Verify no remaining references to the old path**

```bash
git grep -F 'docs/design/llm-agent-ux-spec' || echo "no references found"
git grep -F 'docs/design/' || echo "no references found"
```

Expected: `no references found` for both. If references exist, update them in the same commit.

- [ ] **Step 5: Commit the move**

```bash
git add -A
git commit -m "docs: move llm-agent-ux-spec into docs/specs/ and remove docs/design/

Applies the naming convention from docs/specs/2026-05-07-docs-organization-design.md.
The file's git-creation date (2026-04-25) becomes its filename prefix.
docs/design/ is empty after the move and is removed."
```

---

## Task 3: Draft the master index skeleton in `docs/README.md`

Lay down the structural scaffolding for `docs/README.md` — orientation, primer, 9 empty area headers, and the conventions section. Catalog entries are populated in Task 4. This task is intentionally separated so the skeleton can be reviewed independently of the long catalog content.

**Files:**
- Create: `docs/README.md`

- [ ] **Step 1: Create `docs/README.md` with the full skeleton**

Write this exact content:

````markdown
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
````

- [ ] **Step 2: Verify the file renders**

Run:

```bash
head -30 docs/README.md
wc -l docs/README.md
```

Expected: header is intact, file is roughly 130–150 lines (skeleton size).

- [ ] **Step 3: Commit the skeleton**

```bash
git add docs/README.md
git commit -m "docs: add docs/README.md skeleton (master index scaffold)

Orientation, doc-type primer, status legend, 9 empty area headers, and the
conventions section. Catalog entries are populated in the next commit
(see docs/plans/2026-05-07-docs-organization.md, Task 4)."
```

---

## Task 4: Populate the catalog with one-line entries for every existing spec, plan, and report

For each file in `docs/specs/`, `docs/plans/`, and `docs/reports/`, produce a one-line entry under the right area. Each entry is:

```markdown
- [Title](specs/2026-04-12-state-authority-and-mutations.md) — 5–15 word summary. `[status]`
```

This task is the bulk of the work. It is parallelism-friendly — dispatch one subagent per area if executing under `subagent-driven-development`, or do them sequentially if executing inline.

**Methodology for each entry** (apply uniformly):

1. **Title:** human-readable form of the topic, drawn from the doc's `# H1` header.
2. **Path:** repo-relative from `docs/`, e.g. `specs/2026-04-12-foo.md`.
3. **Summary (5–15 words):** read the doc's `## Purpose` section (or the first paragraph if none) and compress to one clause stating *what target this defines* (spec) or *what migration this enacts* (plan). Avoid restating the title.
4. **Status:** read the doc's header. If it carries `**Status:**`, use that. If not, infer:
   - **Specs** without a `Status:` line and referenced from `CLAUDE.md` Architecture Notes are `[landed]`.
   - **Plans** without a `Status:` line whose row in `docs/plans/STATUS.md` says "yes" are `[landed]`.
   - **Plans** whose STATUS row says "partial" are `[active]`.
   - Anything else: read the file's last-modified date — if older than 30 days and it has no follow-on referenced in `git log`, mark `[landed]` if the spec is realized in code, otherwise `[active]`. When in doubt, `[active]`.

**Area assignments (canonical mapping for this migration):**

```
State & Authority           specs/2026-04-12-state-authority-and-mutations.md
                            specs/2026-04-01-per-author-merkle-dag-state-design.md
                            specs/2026-04-26-state-management-model-design.md
                            specs/2026-03-31-reactive-client-state-design.md
                            plans/2026-04-12-state-authority-and-mutations.md
                            plans/2026-04-01-per-author-merkle-dag-state-plan.md

Networking & Sync           specs/2026-04-24-relay-capability-doc.md
                            specs/2026-04-24-negentropy-sync.md
                            specs/2026-04-24-outbox-relay-discovery.md
                            specs/2026-04-24-history-sync-eose.md
                            specs/2026-03-29-iroh-migration-design.md
                            plans/2026-03-29-iroh-migration.md

Identity, Crypto & Trust    specs/2026-04-24-epoch-key-rotation.md
                            specs/2026-04-24-seal-gift-wrap-dms.md
                            specs/2026-04-24-bech32-identifiers.md
                            specs/2026-03-27-shareable-join-links-design.md
                            plans/2026-03-27-shareable-join-links.md

Messaging                   specs/2026-04-12-willow-channel-removal.md
                            plans/2026-04-12-willow-channel-removal.md

Workers & Actors            specs/2026-03-29-actor-system-design.md
                            specs/2026-03-31-actor-system-library-design.md
                            specs/2026-03-27-worker-nodes-design.md
                            plans/2026-03-30-actor-system.md
                            plans/2026-03-31-actor-system-library.md
                            plans/2026-03-31-actor-library-migration.md
                            plans/2026-03-27-worker-nodes.md

Web UI & UX                 specs/2026-04-19-ui-design/   (one entry, link to README.md)
                            specs/2026-03-25-ux-navigation-improvements-design.md
                            specs/2026-03-26-screen-sharing-call-page-design.md
                            specs/2026-03-24-async-client-ui-refactor-design.md
                            plans/2026-03-24-async-client-ui-refactor.md
                            plans/2026-03-25-ux-navigation-improvements.md
                            plans/2026-03-26-video-screen-sharing-call-page.md
                            plans/2026-04-19-ui-phase-0-foundation.md
                            plans/2026-04-20-ui-phase-1a-desktop-shell.md
                            plans/2026-04-20-ui-phase-1b-mobile-shell.md
                            plans/2026-04-20-ui-phase-1c-palette-a11y.md
                            plans/2026-04-20-ui-phase-1d-trust-verification.md
                            plans/2026-04-20-ui-phase-1e-presence.md
                            plans/2026-04-20-ui-phase-1f-notifications.md
                            plans/2026-04-20-ui-phase-2a-message-row.md
                            plans/2026-04-21-ui-phase-2b-sync-queue.md
                            plans/2026-04-21-ui-phase-2c-profile-card.md
                            plans/2026-04-21-ui-phase-2e-local-search.md
                            plans/2026-04-25-ui-phase-2d-ephemeral-channels.md
                            plans/2026-05-02-issue-354-search-incremental.md

Agent / MCP                 specs/2026-03-29-agentic-peer-api-design.md
                            specs/2026-04-25-llm-agent-ux-spec-design.md   (post-Task-2 path)
                            plans/2026-04-01-agentic-peer-api.md

Testing                     specs/2026-04-21-e2e-test-architecture-design.md
                            specs/2026-04-13-test-architecture.md
                            specs/2026-04-27-event-based-waits-design.md
                            specs/2026-03-24-multi-peer-e2e-tests-design.md
                            plans/2026-04-21-e2e-test-architecture.md
                            plans/2026-03-24-multi-peer-e2e-tests.md
                            plans/2026-04-27-event-based-waits-pr1-test-hooks-foundation.md
                            plans/2026-04-28-event-based-waits-pr1-errata.md
                            plans/2026-04-29-event-based-waits-pr2-peer-wrapper.md
                            plans/2026-04-30-event-based-waits-pr3-data-state-lifecycle.md
                            plans/2026-04-30-event-based-waits-pr4-ratchet-flake-harness.md
                            reports/2026-04-13-test-audit.md

Process & Tooling           specs/2026-04-24-error-prefixes.md
                            specs/2026-05-07-docs-organization-design.md
                            plans/2026-05-07-docs-organization.md
```

The above mapping is the source of truth for area assignment. If a file does not appear in the mapping, add it to the area whose subsystem it primarily affects.

**Files:**
- Modify: `docs/README.md` (replace each `_(populated in Task 4)_` placeholder with real entries)

- [ ] **Step 1: Worked example — populate State & Authority**

Open each State & Authority file and read its `# H1` and `## Purpose` (or first paragraph). Compose entries using the methodology above.

Replace:

```markdown
### State & Authority

**Specs**

_(populated in Task 4)_

**Plans**

_(populated in Task 4)_
```

With (sample — confirm summaries against actual file content; do NOT copy these summaries blind):

```markdown
### State & Authority

**Specs**

- [State authority and mutations](specs/2026-04-12-state-authority-and-mutations.md) — `apply_event` permission checks and the `required_permission` table. `[landed]`
- [Per-author Merkle DAG state](specs/2026-04-01-per-author-merkle-dag-state-design.md) — DAG event model, deterministic materialization. `[landed]`
- [State management model](specs/2026-04-26-state-management-model-design.md) — actors, lock policy, audited exceptions. `[active]`
- [Reactive client state](specs/2026-03-31-reactive-client-state-design.md) — Leptos signal model layered over the state machine. `[landed]`

**Plans**

- [State authority migration](plans/2026-04-12-state-authority-and-mutations.md) — refactor `apply_event` to consult `required_permission`. `[landed]`
- [Per-author DAG implementation](plans/2026-04-01-per-author-merkle-dag-state-plan.md) — DAG primitives, sync buffer, materialization. `[landed]`
```

The placeholder summaries above MUST be replaced with summaries derived from the actual file headers — do not paste them blind.

- [ ] **Step 2: Populate the remaining 8 areas using the same methodology**

For each area (Networking & Sync, Identity/Crypto/Trust, Messaging, Workers & Actors, Web UI & UX, Agent / MCP, Testing, Process & Tooling), repeat Step 1's procedure. Each entry is one line; one summary per file from the canonical mapping.

For the **Web UI & UX** area, the multi-file spec gets a single entry pointing at the parent `README.md`:

```markdown
- [UI target UX bundle](specs/2026-04-19-ui-design/README.md) — 20+ child specs covering desktop and mobile target UX. `[active]`
```

The `STATUS.md` link at the end of the Web UI & UX section is already in the skeleton — leave it.

For the **Process & Tooling** area, include this plan and its spec as entries:

```markdown
**Specs**

- [Error prefixes](specs/2026-04-24-error-prefixes.md) — convention for error-message prefixes across crates. `[active]`
- [Docs organization](specs/2026-05-07-docs-organization-design.md) — target structure for docs/, master index, naming, nesting. `[active]`

**Plans**

- [Docs organization](plans/2026-05-07-docs-organization.md) — populate the master index, create the skill mirror, fold orphan. `[active]`
```

- [ ] **Step 3: Run the link checker**

Save this as a one-shot bash script and run it:

```bash
bash -c '
set -e
fail=0
# Extract all relative .md links from docs/README.md
grep -oE "\(([a-zA-Z0-9._/-]+\.md)\)" docs/README.md | sed "s/^(//; s/)$//" | sort -u | while read -r link; do
    target="docs/$link"
    if [ ! -e "$target" ]; then
        echo "BROKEN: $link → $target does not exist"
        fail=1
    fi
done
exit $fail
'
```

Expected: zero output (all links resolve). If any "BROKEN:" lines appear, fix the path or filename in `docs/README.md`.

- [ ] **Step 4: Run the coverage check**

Every `.md` under `docs/specs/`, `docs/plans/`, `docs/reports/` (excluding nested-folder children) should appear exactly once in the catalog. Verify:

```bash
bash -c '
# Files that exist on disk (excluding nested-spec children and STATUS.md)
disk=$(find docs/specs docs/plans docs/reports -maxdepth 1 -type f -name "*.md" | sed "s|^docs/||" | sort)
# Plus the one nested-spec parent
disk=$(echo "$disk"; echo "specs/2026-04-19-ui-design/README.md")
disk=$(echo "$disk" | grep -v "^plans/STATUS.md$" | sort -u)

# Files referenced in the catalog (between "## Catalog" and "## Reference designs")
referenced=$(awk "/^## Catalog/,/^## Reference designs/" docs/README.md \
    | grep -oE "\(([a-zA-Z0-9._/-]+\.md)\)" \
    | sed "s/^(//; s/)$//" \
    | sort -u)

echo "On disk but not in catalog:"
comm -23 <(echo "$disk") <(echo "$referenced")
echo "---"
echo "In catalog but not on disk:"
comm -13 <(echo "$disk") <(echo "$referenced")
'
```

Expected: both lists are empty (after the closing `---` line). Anything in "On disk but not in catalog" must be added; anything in the second list is a typo to fix.

- [ ] **Step 5: Commit the populated catalog**

```bash
git add docs/README.md
git commit -m "docs: populate master index catalog (all 9 feature areas)

One-line entries for every spec, plan, and report under the existing
docs/specs/, docs/plans/, docs/reports/. Status tags derived from
existing **Status:** headers, docs/plans/STATUS.md rows, or the
inference rules in docs/plans/2026-05-07-docs-organization.md Task 4.

Verified by link-checker and coverage-checker scripts (see plan)."
```

---

## Task 5: Create the `organizing-willow-docs` skill

Mirror the conventions from `docs/README.md` (sections "Document types", "Status tags", and "Conventions") into a project-local skill so an agent adding a new spec or plan can load just the rules without the catalog. The skill is NOT the source of truth — it points back at the spec and the README.

**Files:**
- Create: `.claude/skills/organizing-willow-docs/SKILL.md`

- [ ] **Step 1: Create the skill directory**

```bash
mkdir -p .claude/skills/organizing-willow-docs
```

- [ ] **Step 2: Write the skill file**

Create `.claude/skills/organizing-willow-docs/SKILL.md` with this exact content:

````markdown
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
````

- [ ] **Step 3: Verify the skill is well-formed**

```bash
head -3 .claude/skills/organizing-willow-docs/SKILL.md
```

Expected: starts with `---` and includes `name: organizing-willow-docs` and `description:` lines.

```bash
wc -l .claude/skills/organizing-willow-docs/SKILL.md
```

Expected: roughly 80–110 lines.

- [ ] **Step 4: Commit the skill**

```bash
git add .claude/skills/organizing-willow-docs/SKILL.md
git commit -m "skill: add organizing-willow-docs

Project-local skill mirroring the docs conventions from
docs/specs/2026-05-07-docs-organization-design.md and docs/README.md.
Loaded on demand when an agent adds or modifies a spec/plan/report,
or restructures the catalog. README is the source of truth; this
skill is updated in the same commit when conventions change."
```

---

## Task 6: Update CLAUDE.md with the pointer to README and skill

Two small edits in `CLAUDE.md`. No content is removed beyond the redundant lines that the README now owns.

**Files:**
- Modify: `CLAUDE.md:25-32` (Repository Structure tree)
- Modify: `CLAUDE.md:292` (Specs & Plans line in Code Conventions)

- [ ] **Step 1: Update the Repository Structure tree**

Replace the current tree (lines 25–32, the `docs/` block):

```
docs/
├── plans/              — Implementation plans for features (YYYY-MM-DD-<name>.md)
├── specs/              — Design specs and technical specifications (YYYY-MM-DD-<name>-design.md)
├── design/             — Long-form design documents (UX specs, etc.)
├── reference-designs/  — Exploratory UI / design references
└── reports/            — Ad-hoc audit and investigation reports
```

With:

```
docs/
├── README.md           — Master index of specs, plans, and reports (start here)
├── specs/              — Target state — what we are building toward (YYYY-MM-DD-<name>-design.md)
├── plans/              — Migration steps — how we get to the target (YYYY-MM-DD-<name>.md)
├── reports/            — One-shot audits and investigations
└── reference-designs/  — Archived design bundles (immutable)
```

Use the Edit tool with the exact old/new strings.

- [ ] **Step 2: Update the Specs & Plans line in Code Conventions**

Replace this line (line 292):

```markdown
- **Specs & Plans**: Design specs in `docs/specs/` named `YYYY-MM-DD-<feature-name>-design.md`. Plans in `docs/plans/` named `YYYY-MM-DD-<feature-name>.md`.
```

With:

```markdown
- **Docs entry point**: `docs/README.md` is the master index of specs and plans, grouped by feature area. Read it before adding any new spec or plan, or before searching for an existing one. The `organizing-willow-docs` skill mirrors the conventions for on-demand loading. Cemented in `docs/specs/2026-05-07-docs-organization-design.md`.
```

Use the Edit tool with the exact old/new strings.

- [ ] **Step 3: Verify CLAUDE.md still parses cleanly**

```bash
grep -n "Repository Structure" CLAUDE.md
grep -n "Docs entry point" CLAUDE.md
grep -n "Specs & Plans" CLAUDE.md
```

Expected: first two greps return one line each. Third grep returns nothing — the old "Specs & Plans" line is gone.

- [ ] **Step 4: Commit the CLAUDE.md update**

```bash
git add CLAUDE.md
git commit -m "docs(claude): point CLAUDE.md at docs/README.md and the org skill

Replaces the inline specs/plans naming line with a pointer to the
master index and the organizing-willow-docs skill. Repository
Structure tree drops docs/design/ (folded into docs/specs/ in
Task 2) and adds docs/README.md.

The two existing Architecture-Notes-section pointers to specific
specs (state-management-model, e2e-test-architecture) are left
in place — they are inline context, not redundant with the index."
```

---

## Task 7: Cross-surface consistency check

Verify that the three discovery surfaces tell the same story.

**Files (read only):**
- `docs/README.md`
- `.claude/skills/organizing-willow-docs/SKILL.md`
- `CLAUDE.md`

- [ ] **Step 1: Conventions parity check**

Both surfaces (the README's `## Conventions` section and the skill's body) should agree on naming patterns, status tags, header fields, and nesting rules.

```bash
# Naming patterns must appear in both
grep -F "YYYY-MM-DD-<kebab>-design.md" docs/README.md
grep -F "YYYY-MM-DD-<kebab>-design.md" .claude/skills/organizing-willow-docs/SKILL.md

# Status tags must appear in both
for tag in draft active landed superseded; do
    grep -qE "\b${tag}\b" docs/README.md && grep -qE "\b${tag}\b" .claude/skills/organizing-willow-docs/SKILL.md \
        || echo "MISSING: status tag '$tag' not found in both surfaces"
done
```

Expected: each grep prints at least one match. The for-loop prints nothing (silence = parity).

- [ ] **Step 2: CLAUDE.md pointer check**

```bash
grep -F "docs/README.md" CLAUDE.md
grep -F "organizing-willow-docs" CLAUDE.md
```

Expected: both greps print at least one matching line.

- [ ] **Step 3: No-orphan check**

```bash
test -d docs/design && echo "FAIL: docs/design still exists" || echo "OK: docs/design removed"
test -f docs/specs/2026-04-25-llm-agent-ux-spec-design.md && echo "OK: orphan moved" || echo "FAIL: orphan not at expected new path"
```

Expected: two `OK:` lines.

- [ ] **Step 4: Final link-checker pass**

Re-run the link checker from Task 4 Step 3:

```bash
bash -c '
set -e
fail=0
grep -oE "\(([a-zA-Z0-9._/-]+\.md)\)" docs/README.md | sed "s/^(//; s/)$//" | sort -u | while read -r link; do
    target="docs/$link"
    if [ ! -e "$target" ]; then
        echo "BROKEN: $link → $target does not exist"
        fail=1
    fi
done
exit $fail
'
```

Expected: zero output.

- [ ] **Step 5: Run `just check` to confirm nothing else regressed**

```bash
just check
```

Expected: passes (this is a docs-only change, but `just check` is the standard pre-commit gate per CLAUDE.md).

- [ ] **Step 6: Final push**

If any earlier task left unpushed commits:

```bash
git push -u origin claude/organize-docs-index-ojI2V
```

---

## Done

After Task 7 passes:

- `docs/README.md` exists, is the entry point for all docs.
- `.claude/skills/organizing-willow-docs/SKILL.md` exists, mirrors the conventions.
- `CLAUDE.md` points at both.
- `docs/design/` is gone; its lone file is at `docs/specs/2026-04-25-llm-agent-ux-spec-design.md`.
- The 2026-04-12 spec/plan pairs are verified.
- Status of this plan: change `**Status:**` from `draft` → `landed` in `docs/plans/2026-05-07-docs-organization.md` and update the README catalog entry for it from `[active]` → `[landed]`.
