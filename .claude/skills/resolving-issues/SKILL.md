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
- **REQUIRED for implementers when complexity gate trips (see Implementer Agent step 3):** `superpowers:brainstorming` (run automated, single-actor — never ask the human; runs are unattended), `superpowers:writing-plans` (drop a plan file when the brainstorm lands a multi-step plan).

## Master Branch Pattern

All sub-fixes land on one master branch per session. Master PR is opened **only at the end** of the run, **ready (not draft)**, so the human can't accidentally merge incomplete work mid-flight.

### Master branch setup (start of run)
1. Always create fresh master branch per session. Never reuse an open one.
2. Branch off latest `main`: `auto-fix/batch-YYYY-MM-DD-HHMMSS` (timestamp = unique per session). Empty session-open commit (`git commit --allow-empty -m "chore: open auto-fix batch ..."`) so the branch is non-empty + sub-PRs have a stable base. Push branch. **Do NOT open a PR yet.**
3. Track resolved issues locally during the run (commit subjects + a working list in coordinator memory or a scratch file). Assemble the final PR body at end-of-run from this list.

### Master PR open (end of run)
1. After all sub-PRs merge (or get parked as follow-up issues — see below), all skill edits applied, and Lessons Learned drafted, open the PR — **non-draft, ready for review**.
2. Title: `auto-fix batch YYYY-MM-DD-HHMMSS`. Base: `main`. Apply label `auto-fix-batch`.
3. PR body: running list of `Fixes #N` lines (one per resolved issue) + `## Skill Evolution` (if skill commits landed) + `## Lessons Learned` section + `## Parked` (if any issues hit a blocker mid-run; cite the follow-up issue per row).
4. **Always open the master PR with what landed**, even if some attempted issues hit blockers. The PR ships the wins; blockers move to follow-up issues so the next scheduled run picks them up automatically. Never open as draft. If literally nothing landed (zero merged sub-PRs), don't open a PR at all — close the branch out.
5. **No in-session continuation.** Don't leave a session "to be resumed" — the human must not have to chase a session. The issue queue is the durable handoff.

### Sub-PR rules
- Sub-PR base = master branch, NOT `main`.
- Sub-PR body references issue (`Refs #N`) — no `Fixes` keyword. `Fixes` lives only on master PR (assembled at end of run) so issues close when master PR merges.
- **Sub-PR base ≠ main means GH Actions workflows scoped to `pull_request: branches: [main]` won't fire.** Implementer treats local `just check` green (fmt + clippy + test + wasm) as the merge gate; do not park sub-PR open waiting for CI that won't run. Master PR (base=main, opened end-of-run) runs full CI before human merge — the load-bearing gate.
- Implementer watches CI on sub-PR ONLY when CI actually runs (rare; usually requires PR base = main). CI green → merge sub-PR into master branch. No CI run → local `just check` green is the gate, then merge.
- CI red after one fix attempt → file follow-up issue (caveman body, link blocker), close sub-PR. Move on. Do NOT leave it as a draft for someone to resume — the next scheduled run picks up the follow-up issue automatically.

## Core Loop

1. Read open issues + open PRs. Skip anything in flight.
2. Pick small-scope fixes. `general-audit` issues = top priority.
3. Skip big features + major refactors. Out of scope.
4. No in-scope issues? Noop. Skip the rest. No master branch created, no PR opened.
5. Create fresh master branch for this session (push, no PR yet — see ### Master branch setup).
6. Per issue, sequential, max 10 per run:
   - Spawn fresh implementer agent.
   - Implementer: worktree off master branch → research subagents if needed → fix → tests → sub-PR into master branch → merge gate → squash-merge.
   - Track `Fixes #N` for final PR body assembly.
   - Tear down worktree.
   - Next issue.
7. Implementer finds related rot? File follow-up issue.
8. Apply Lessons Learned skill edits to `.claude/skills/resolving-issues/SKILL.md`, commit on master branch, push.
9. Open the master PR — **ready (not draft)** — with full body (`Fixes #N` list + `## Skill Evolution` + `## Lessons Learned`). Master PR runs full CI; human merges when satisfied. If anything's unfinished, leave the branch un-PR'd instead of opening a draft.

## Implementer Agent

Fresh agent per issue, scoped to one issue + master branch ref. Steps:

1. Read the issue. Decide if more context needed.
2. **Research (optional, parallel OK):** spawn research subagents for codebase grep, related-file reads, spec lookups. Synthesize before coding.
3. **Complexity gate — automated brainstorm + plan when warranted:**
   - **Trigger any of:** issue spans > 1 crate, fix touches state machine / wire format / migration paths, ≥ 2 reasonable approaches exist, root cause not obvious from issue text, fix likely > 5 files OR > 200 LOC, "it depends" question on scope.
   - **Skip when:** issue is a one-liner / config swap / typo / clearly mechanical (single rg-pattern site) / has explicit "Suggested fix" the implementer can follow verbatim.
   - **If triggered, run automated:**
     1. Invoke `superpowers:brainstorming` self-driven — implementer plays both roles (exploration + decision). Do NOT ask the human anything; the run is unattended. Output: a written brief naming the chosen approach, the runner-up, and why rejected. Cap at 5 minutes / a few tool calls.
     2. If the brainstorm surfaces a multi-step plan, invoke `superpowers:writing-plans` to drop a `docs/plans/YYYY-MM-DD-<issue-N>-<slug>.md` on the worktree branch. Otherwise skip — small fixes don't need a plan file.
   - Fold the brainstorm + plan into the sub-PR body so the human can review the reasoning, not just the code.
4. Open worktree branched off master branch. Branch name: `auto-fix/issue-N-short-slug`.
5. Apply fix. Add tests at lowest tier covering behavior (see `CLAUDE.md`).
6. **Scope-creep guard:** if root-cause fix touches > 5 files OR > 200 LOC AND brainstorm in step 3 didn't already approve that scope, return to coordinator with a brainstorm note before pushing. Coordinator decides: split, defer, or proceed. Don't unilaterally balloon a small-scope ticket.
7. `just check` green locally before pushing.
8. Push branch. Open sub-PR with master branch as base.
9. **Merge gate:** if sub-PR CI runs (rare — only when workflow `branches: [main]` filter matches), wait for green. If CI doesn't run (sub-PR base ≠ main is the common case), local `just check` green from step 7 IS the gate. Merge with `mcp__github__merge_pull_request` `merge_method: squash`.
10. CI red after one fix attempt OR local `just check` red OR mid-fix block → **file a follow-up GH issue** (caveman body, link the original issue + cite the blocker), then **close the sub-PR** (don't leave it as a draft for someone to resume). The next scheduled run will see the follow-up issue in the queue and pick it up. Return control to coordinator.
11. Tear down worktree on merge OR on close-after-blocker.

## Lessons Learned

End of run, before opening the master PR:

1. Draft `## Lessons Learned` content with caveman bullets: what worked, what didn't, concrete suggested edits to this skill file.
2. **Apply the skill edits to `.claude/skills/resolving-issues/SKILL.md` on the master branch.** Commit + push. Editing the skill is meta-work, exempt from the "coordinator never codes" rule.
3. Then open the master PR (ready, not draft) with body containing `Fixes #N` list + `## Skill Evolution` (referencing the skill commit) + `## Lessons Learned`.

Never defer skill edits to a follow-up — they ship with the run that surfaced them, in the same PR.

## Rules

### Coordinator never codes
- Read, dispatch, monitor. Implementers touch files.
- One worktree per issue. Sequential between issues. Tear down after merge or draft-park.
- **Exception:** the master branch's own session-open commit + Lessons Learned skill edits (see ## Lessons Learned). Coordinator commits these directly to the master branch.

### Sequential between issues
- One issue at a time. No parallel implementers.
- Research subagents *inside* an implementer may run in parallel.
- Cap = 10 issues per run.

### Fresh agent per issue
- New implementer each issue. No state leak.
- Each implementer gets only its issue + master branch ref.

### Scope filter
- Fixes + small-scope only.
- No big features. No major refactors. No architecture rewrites.
- Too big? Skip. Comment caveman note on issue if scope misclassified.

### GitHub comms
- All issue + PR bodies + comments in caveman mode.
- Code blocks + security warnings stay normal.

### Autonomy
- Best judgment. No hand-holding.
- Mid-fix block? Implementer files a follow-up GH issue (caveman body, link original + cite blocker), closes the sub-PR, moves on. The follow-up issue is the durable handoff for the next scheduled run — don't leave a draft sub-PR for someone to chase.
- Noop fine. Ship nothing > ship junk.
- **No in-session continuation.** Sessions don't get resumed. If something doesn't fit in this run, file an issue.

## Setup

- Pre-worktree: `git stash` or `git restore` main dir; `.claude/worktrees/` in `.gitignore`.
- Worktree per issue, branched off master branch. Tear down after sub-PR merges or parks as draft.

## Quality

- `just check` green before sub-PR opened.
- Tests at lowest tier covering behavior (see `CLAUDE.md`).
- Sub-PR merges into master branch only after merge gate passes (see ### Sub-PR rules — local `just check` is the gate when CI doesn't run, sub-PR CI is the gate when it does).
- Master PR opened only at end of run, **non-draft**, with whatever sub-PRs landed. Master PR runs full CI when opened — the actual quality net for the run.
- Anything blocked → follow-up issue, sub-PR closed, next scheduled run picks it up. Never open the master PR as draft. Only skip opening the PR entirely if literally zero sub-PRs landed.
- No in-session continuation. The issue queue is the durable handoff.
