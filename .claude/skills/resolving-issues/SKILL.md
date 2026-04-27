---
name: resolving-issues
description: Use when running scheduled pass over open issue + PR queue to clear small-scope fixes, or when /resolving-issues invoked manually
user-invocable: true
---

# Resolving Issues

You = coordinator. Implementer subagents = workers. Read, dispatch, monitor. Never touch files.

## When

- Scheduled: sweep open issues + PRs, fix small items sequentially into one master PR.
- Manual: same flow, on demand.

## Required Skills

- **REQUIRED:** `superpowers:using-git-worktrees` — isolate each implementer.
- **REQUIRED:** `caveman` — all GH comms.
- **REQUIRED for implementers:** `superpowers:test-driven-development`, `superpowers:verification-before-completion`, `superpowers:dispatching-parallel-agents` (for research subagents).

## Master PR Pattern

All sub-fixes land in one master PR per session. Human reviews master PR holistically + merges → all linked issues auto-close.

### Master PR setup
1. Always create fresh master PR per session. Never reuse an open one.
2. Branch off latest `main`: `auto-fix/batch-YYYY-MM-DD-HHMMSS` (timestamp = unique per session). Push. Open **draft** PR titled `auto-fix batch YYYY-MM-DD-HHMMSS` targeting `main`. Apply label `auto-fix-batch`.
3. Master PR body = running list of `Fixes #N` lines, one per resolved issue. Update after each sub-PR merge.

### Sub-PR rules
- Sub-PR base = master PR branch, NOT `main`.
- Sub-PR body references issue (`Refs #N`) — no `Fixes` keyword. `Fixes` lives only on master PR so issues close on master merge.
- Implementer watches CI on sub-PR. CI green → merge sub-PR into master PR branch.
- CI red after one fix attempt → convert sub-PR to draft + caveman question. Move on.

## Core Loop

1. Read open issues + open PRs. Skip anything in flight.
2. Pick small-scope fixes. `general-audit` issues = top priority.
3. Skip big features + major refactors. Out of scope.
4. Create fresh master PR for this session.
5. Per issue, sequential, max 10 per run:
   - Spawn fresh implementer agent.
   - Implementer: worktree off master PR branch → research subagents if needed → fix → tests → sub-PR into master PR → watch CI → merge on green.
   - On merge, append `Fixes #N` to master PR body.
   - Tear down worktree.
   - Next issue.
6. Implementer finds related rot? File follow-up issue.
7. Run done? Leave master PR as draft. Human marks ready when satisfied.
8. No work fits + no commits added this run? Noop fine.

## Implementer Agent

Fresh agent per issue, scoped to one issue + master PR branch ref. Steps:

1. Read the issue. Decide if more context needed.
2. **Research (optional, parallel OK):** spawn research subagents for codebase grep, related-file reads, spec lookups. Synthesize before coding.
3. Open worktree branched off master PR branch. Branch name: `auto-fix/issue-N-short-slug`.
4. Apply fix. Add tests at lowest tier covering behavior (see `CLAUDE.md`).
5. `just check` green locally before pushing.
6. Push branch. Open sub-PR with master PR branch as base.
7. Watch CI. Flake → re-run. Real failure → one fix attempt.
8. CI green → merge sub-PR into master PR branch. Tear down worktree.
9. CI still red → draft sub-PR + caveman question in body. Return control to coordinator.

## Lessons Learned

End of run, append `## Lessons Learned` section to master PR body with caveman bullets: what worked, what didn't, concrete suggested edits to this skill file. Human (or follow-up routine) edits `.claude/skills/resolving-issues/SKILL.md` directly to incorporate.

## Rules

### Coordinator never codes
- Read, dispatch, monitor. Implementers touch files.
- One worktree per issue. Sequential between issues. Tear down after merge or draft-park.

### Sequential between issues
- One issue at a time. No parallel implementers.
- Research subagents *inside* an implementer may run in parallel.
- Cap = 10 issues per run.

### Fresh agent per issue
- New implementer each issue. No state leak.
- Each implementer gets only its issue + master PR branch ref.

### Scope filter
- Fixes + small-scope only.
- No big features. No major refactors. No architecture rewrites.
- Too big? Skip. Comment caveman note on issue if scope misclassified.

### GitHub comms
- All issue + PR bodies + comments in caveman mode.
- Code blocks + security warnings stay normal.

### Autonomy
- Best judgment. No hand-holding.
- Mid-fix block? Implementer drafts sub-PR + caveman question, moves on.
- Noop fine. Ship nothing > ship junk.

## Setup

- Pre-worktree: `git stash` or `git restore` main dir; `.claude/worktrees/` in `.gitignore`.
- Worktree per issue, branched off master PR branch. Tear down after sub-PR merges or parks as draft.

## Quality

- `just check` green before sub-PR opened.
- Tests at lowest tier covering behavior (see `CLAUDE.md`).
- Sub-PR merges into master PR only after CI green.
- Master PR stays draft for entire orchestrator run. Human marks ready when satisfied.
