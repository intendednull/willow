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
2. Branch off latest `main`: `auto-fix/batch-YYYY-MM-DD-HHMMSS` (timestamp = unique per session). Empty session-open commit (`git commit --allow-empty -m "chore: open auto-fix batch ..."`) so the draft PR can be opened before any sub-PR lands. Push. Open **draft** PR titled `auto-fix batch YYYY-MM-DD-HHMMSS` targeting `main`. Apply label `auto-fix-batch`.
3. Master PR body = running list of `Fixes #N` lines, one per resolved issue. Update after each sub-PR merge.

### Sub-PR rules
- Sub-PR base = master PR branch, NOT `main`.
- Sub-PR body references issue (`Refs #N`) — no `Fixes` keyword. `Fixes` lives only on master PR so issues close on master merge.
- **Sub-PR base ≠ main means GH Actions workflows scoped to `pull_request: branches: [main]` won't fire.** Implementer treats local `just check` green (fmt + clippy + test + wasm) as the merge gate; do not park sub-PR open waiting for CI that won't run. Master PR (base=main) re-runs full CI on every merge — the load-bearing gate.
- Implementer watches CI on sub-PR ONLY when CI actually runs (rare; usually requires PR base = main). CI green → merge sub-PR into master PR branch. No CI run → local `just check` green is the gate, then merge.
- CI red after one fix attempt → convert sub-PR to draft + caveman question. Move on.

## Core Loop

1. Read open issues + open PRs. Skip anything in flight.
2. Pick small-scope fixes. `general-audit` issues = top priority.
3. Skip big features + major refactors. Out of scope.
4. No in-scope issues? Noop. Skip the rest. No master PR opened.
5. Create fresh master PR for this session.
6. Per issue, sequential, max 10 per run:
   - Spawn fresh implementer agent.
   - Implementer: worktree off master PR branch → research subagents if needed → fix → tests → sub-PR into master PR → watch CI → merge on green.
   - On merge, append `Fixes #N` to master PR body.
   - Tear down worktree.
   - Next issue.
7. Implementer finds related rot? File follow-up issue.
8. Run done? Append Lessons Learned section to master PR body. Leave master PR as draft. Human marks ready when satisfied.

## Implementer Agent

Fresh agent per issue, scoped to one issue + master PR branch ref. Steps:

1. Read the issue. Decide if more context needed.
2. **Research (optional, parallel OK):** spawn research subagents for codebase grep, related-file reads, spec lookups. Synthesize before coding.
3. Open worktree branched off master PR branch. Branch name: `auto-fix/issue-N-short-slug`.
4. Apply fix. Add tests at lowest tier covering behavior (see `CLAUDE.md`).
5. **Scope-creep guard:** if root-cause fix touches > 5 files OR > 200 LOC, return to coordinator with a brainstorm note before pushing. Coordinator decides: split, defer, or proceed. Don't unilaterally balloon a small-scope ticket.
6. `just check` green locally before pushing.
7. Push branch. Open sub-PR with master PR branch as base.
8. **Merge gate:** if sub-PR CI runs (rare — only when workflow `branches: [main]` filter matches), wait for green. If CI doesn't run (sub-PR base ≠ main is the common case), local `just check` green from step 6 IS the gate. Merge with `mcp__github__merge_pull_request` `merge_method: squash`.
9. CI red after one fix attempt OR local `just check` red → mark sub-PR as draft + caveman question in body. Return control to coordinator.
10. Tear down worktree on merge.

## Lessons Learned

End of run, append `## Lessons Learned` section to master PR body with caveman bullets: what worked, what didn't, concrete suggested edits to this skill file.

**Apply the skill edits in the same master PR.** Don't defer to a follow-up — coordinator commits the edits to `.claude/skills/resolving-issues/SKILL.md` on the master PR branch directly so the run lands one self-contained PR (fixes + skill evolution). Editing the skill is meta-work, exempt from the "coordinator never codes" rule.

## Rules

### Coordinator never codes
- Read, dispatch, monitor. Implementers touch files.
- One worktree per issue. Sequential between issues. Tear down after merge or draft-park.
- **Exception:** the master PR's own session-open commit + Lessons Learned skill edits (see ## Lessons Learned). Coordinator commits these directly to the master PR branch.

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
- Mid-fix block? Implementer parks work as draft sub-PR + caveman question, moves on.
- Noop fine. Ship nothing > ship junk.

## Setup

- Pre-worktree: `git stash` or `git restore` main dir; `.claude/worktrees/` in `.gitignore`.
- Worktree per issue, branched off master PR branch. Tear down after sub-PR merges or parks as draft.

## Quality

- `just check` green before sub-PR opened.
- Tests at lowest tier covering behavior (see `CLAUDE.md`).
- Sub-PR merges into master PR only after merge gate passes (see ### Sub-PR rules — local `just check` is the gate when CI doesn't run, sub-PR CI is the gate when it does).
- Master PR (base=main) runs full CI on every merge — that's the actual quality net for the run.
- Master PR stays draft for entire orchestrator run. Human marks ready when satisfied.
