# Spec Alignment Agent — Design

**Status:** draft (2026-04-27)
**Owner:** TBD
**Related:** `.claude/skills/spec-alignment-audit/` (to be created)

## Problem

The repository accumulates ~30 design specs in `docs/specs/`. Code evolves
faster than specs, and specs occasionally describe behavior the code never
adopted. Recent commit history (`audit/doc-drift`, `fix/docs-spec-drift-*`,
`audit/lock-actor-alignment`) shows drift detection has been a recurring
manual chore. We want a recurring agent that surfaces drift as actionable
GitHub issues so a separate executor agent can fix them.

## Goals

- Periodically (cadence is the user's choice; expected ~weekly) audit every
  spec in `docs/specs/` against the live codebase.
- Produce one GitHub issue per drift finding, including a determination of
  which side should change (docs vs code) with evidence from commit history.
- Best-effort deduplication against existing open issues and PRs.
- Tolerate noise; bias toward filing rather than suppressing.
- Never edit code or specs; never open PRs. Pure triager.

## Non-Goals

- Real-time / on-merge enforcement (handled elsewhere if needed).
- Style, prose, or typo policing.
- Automatic fixing of drift (delegated to a separate executor agent).
- Cross-repo or external-doc auditing.
- Any guarantee against duplicate issue filing — best-effort only.

## Architecture

Two-layer skill:

```
.claude/skills/spec-alignment-audit/
├── SKILL.md           # orchestrator workflow, user-invocable
└── auditor-prompt.md  # verbatim prompt passed to each sub-agent
```

**Orchestrator** (encoded in `SKILL.md`): enumerates specs, dispatches one
fresh sub-agent per spec sequentially, persists run state, prints a final
summary. Owns the worklist. Does no auditing itself.

**Auditor sub-agent** (one per spec, fresh context): reads the spec,
verifies claims against code, decides direction per finding, deduplicates
against open GitHub issues/PRs, files the survivors, returns a structured
summary.

**Cadence** is composed externally via the existing `loop` skill
(`/loop 7d /spec-alignment-audit`). Skill is also invocable on demand for
one-off audits after large merges.

### Why this shape

- **Orchestrator + sub-agents** keeps each audit's context isolated. A
  sub-agent's findings on one spec never poison the audit of another.
- **Sequential dispatch** keeps token/resource use bounded and avoids
  parallel sub-agents racing to file the same issue.
- **Sibling `auditor-prompt.md`** versions the sub-agent contract on its own
  and avoids quoting the prompt inside `SKILL.md`. Orchestrator reads it and
  passes it verbatim.
- **Skill, not slash-command-only**: matches `.claude/skills/` convention,
  composes with the existing `loop` skill, and `user-invocable: true` makes
  it directly callable.

### Rejected alternatives

- **GitHub Action cron.** Splits the system across two homes (skill + CI),
  duplicates GitHub auth concerns, and removes the user's ability to
  manually trigger after a large merge. The `loop` skill already covers
  recurrence inside Claude Code.
- **Hook on file change.** Fires too often; spec audits are inherently
  batchy.
- **Single agent doing all specs.** Context bloat after a handful of specs;
  early findings poison later audits.
- **Parallel sub-agents.** Resource cost; race conditions on issue filing.
- **One umbrella issue per spec, edited each run.** Awkward to assign; one
  drift fixed doesn't cleanly close the issue. Per-finding issues map
  better to executor-agent task units.

## Orchestrator Contract

### Inputs (skill arguments)

| Argument | Default | Purpose |
|----------|---------|---------|
| `cap=N` | 5 | Max findings per spec per run |
| `glob=...` | `docs/specs/**/*.md` | Override the target set |
| `dry-run` | false | Sub-agents skip dedup + filing; report findings only |

### Steps

1. Resolve `glob` (default `docs/specs/**/*.md`). Sort alphabetically. This
   is the worklist.
2. Check `.claude/audit-runs/` for a state file dated today with `pending`
   entries. If found, offer the user the option to resume; otherwise start a
   new run. State file path:
   `.claude/audit-runs/<ISO-timestamp>-spec-alignment.json`.
3. Persist initial state:
   ```
   {
     "run_started": "<ISO-8601>",
     "run_tag": "<ISO-timestamp>",
     "finding_cap": <N>,
     "dry_run": <bool>,
     "specs": [
       {"path": "...", "status": "pending"},
       ...
     ]
   }
   ```
4. For each spec in order, sequentially:
   a. Mark `in-progress` in state file.
   b. Read `auditor-prompt.md`.
   c. Dispatch via `Agent` tool: `subagent_type: general-purpose`, prompt =
      auditor-prompt with `SPEC_PATH`, `FINDING_CAP`, `RUN_TAG`, `DRY_RUN`
      substituted.
   d. Parse the sub-agent's returned JSON summary. On parse failure or any
      raised error, mark the spec `errored` with the reason. **Never
      retry.**
   e. On success, mark `done` and store the summary.
   f. Print a one-line status update.
5. After the last spec, print the final summary (counts per direction,
   total filed, duplicates skipped, capped/errored specs).

### State file conventions

- Per-run, not rolling — old runs serve as audit history.
- Resume only within the same calendar day (local-TZ `YYYY-MM-DD` prefix
  matches today). Older state files ignored.
- On resume, the original run's `finding_cap` and `dry_run` values are
  preserved; new skill-argument overrides are rejected with a message
  asking the user to start a fresh run.
- Add `.claude/audit-runs/` to `.gitignore`. Local artifact, not source.

## Auditor Sub-Agent Contract

Sub-agent runs in a clean context. Prompt provided verbatim from
`auditor-prompt.md`. Inputs substituted: `SPEC_PATH`, `FINDING_CAP`,
`RUN_TAG`, `DRY_RUN`.

### Phases

**A. Read the spec.** Read `SPEC_PATH` fully. Enumerate its claims: file
paths, type/function/module names, behavioral assertions, follow-up
sections, supersession markers.

**B. Audit against code.** For each claim, verify against the live tree
using `Read`, `Grep`, `Glob`. Build a draft list of findings. Stop at
`FINDING_CAP`; if reached, set `cap_hit: true`. Drop pure style/typo
findings.

For non-localizable claims (whole-subsystem invariants), sample 2–3
representative files in the named subsystem; record what was sampled in the
finding so the executor knows the basis.

**C. Direction determination.** For each finding, decide
`docs-update | code-update | ambiguous`. Method:

1. Read surrounding usages of the drifted symbol/path.
2. Run `git log --follow -p -- <file>` against both the code file and the
   spec to see when and why divergence appeared.
3. Strong signals:
   - Migration/refactor/removal commits on the code side post-dating the
     spec → likely `docs-update`.
   - Code added recently with no spec update, especially without an obvious
     reason → likely `code-update`.
   - Mixed/unclear → `ambiguous`, quote both sides.

Reasoning, with commit hashes and file:line cites, goes into the issue
body — this is the executor agent's most valuable input.

**D. Deduplicate (only after Phases A–C complete).** Touching GitHub state
earlier would poison the audit context. For each surviving finding:

1. `mcp__github__list_issues` with label `spec-drift`, state `open`.
2. `mcp__github__list_pull_requests` with state `open`.
3. Compare each open item's title and body preface against the finding (same
   spec slug, same drift target). On clear match, drop the finding and
   increment `duplicates_skipped`.

Best-effort only. Downstream agents are expected to handle leakage.

**E. File issues.** For each surviving finding, call
`mcp__github__issue_write` with:

- **Title:** `spec-drift(<spec-slug>): <one-line summary>`
- **Labels:** `spec-drift`, `audit:<spec-slug>`,
  `direction:<docs-update|code-update|ambiguous>`
- **Body sections:**
  - **Spec reference** — path + line range
  - **Codebase reference** — file:line(s)
  - **Drift** — one paragraph
  - **Direction & reasoning** — Phase C output, with commit hashes
  - **Suggested fix / next steps** — concrete, executor-ready
  - **Confidence** — high / medium / low
  - **Audit metadata** — `<!-- run: RUN_TAG  spec: <slug> -->`

If `DRY_RUN` is true, skip phases D and E. Sub-agent returns the findings
verbatim in its summary instead of issue URLs.

**F. Report.** Return JSON:

```json
{
  "spec": "<spec-path>",
  "issues_filed": [{"url": "...", "direction": "...", "title": "..."}],
  "duplicates_skipped": <int>,
  "cap_hit": <bool>,
  "dry_run_findings": [<finding-objects>],
  "errors": [<strings>]
}
```

`dry_run_findings` is empty when `DRY_RUN` is false; `issues_filed` is
empty when `DRY_RUN` is true.

### What the sub-agent does NOT do

- Does not edit specs, code, or any other tracked file.
- Does not create branches, commits, or PRs.
- Does not close, label, or comment on existing issues beyond what is
  required to read them for dedup.

## GitHub Issue Conventions

Established by this skill (and reused by the executor agent):

- **Label `spec-drift`** — applied to every issue this skill files.
- **Label `audit:<spec-slug>`** — slug = spec filename with `.md` and
  trailing `-design` stripped (e.g. `2026-04-01-per-author-merkle-dag-state-design.md`
  → `audit:2026-04-01-per-author-merkle-dag-state`). For nested specs like
  `2026-04-19-ui-design/index.md`, the slug is the directory name. Lets the
  executor query all drift for a single spec.
- **Label `direction:<docs-update|code-update|ambiguous>`** — pre-triages
  the work for the executor.
- **Title prefix `spec-drift(<spec-slug>):`** — supports text-search dedup.

These labels do not need to pre-exist; GitHub creates them on first use.

## Permissions

The skill calls: `Agent`, `Read`, `Grep`, `Glob`, `Bash` (for `git log`),
and `mcp__github__list_issues | list_pull_requests | issue_write`. SKILL.md
ends with a pre-approval note telling the user which permissions to add to
`.claude/settings.local.json` if running unattended via `/loop`. The skill
does not auto-modify settings.

## Failure Modes

| Failure | Behavior |
|---------|----------|
| Sub-agent crashes / returns invalid JSON | Mark spec `errored`, log reason, continue. No retry. |
| GitHub API call fails inside sub-agent | Sub-agent surfaces the error in `errors[]`; orchestrator records it. Findings for that spec may be lost for this run. |
| State file write fails | Orchestrator aborts the run; partial state on disk is still inspectable. |
| User Ctrl+Cs mid-run | State file holds `in-progress` for the current spec; today's resume offer picks up at the next pending spec. |
| Spec contains malformed frontmatter / unreadable | Sub-agent reports `errors[]` with parse details; spec marked `errored`. |
| Two `loop`-driven runs overlap | Second run sees today's state file, offers resume. If user accepts, runs are serialized. |

## Testing

This is a prompt-driven skill, not Rust code, so no `cargo test` coverage.
Validation strategy:

1. **Dry-run smoke test.** Run `/spec-alignment-audit dry-run=true` on the
   current tree; inspect the run state file. Verify each spec was visited,
   findings look reasonable, no GitHub calls were made.
2. **Single-spec test.** Run with `glob=docs/specs/2026-04-12-*.md` to
   audit a single known-stale spec; verify direction determinations match
   human judgment.
3. **Dedup test.** Run twice in a row (non-dry); second run should report
   high `duplicates_skipped` and file zero or near-zero new issues.
4. **Failure-injection test.** Manually corrupt one spec's frontmatter;
   verify the orchestrator marks it `errored` and continues.

These are manual, run before merge. No automated harness.

## Open Questions / Follow-ups

- Whether to teach the executor agent to consume `audit:<slug>` labels for
  scheduling. Out of scope for this design; tracked separately.
- Whether long-running runs should checkpoint state mid-spec (current
  design only checkpoints between specs). Defer until we observe a
  sub-agent failure that loses meaningful work.
- Whether dry-run output should also be written as a markdown report under
  `docs/reports/` for offline review. Defer; state file is sufficient
  initially.
