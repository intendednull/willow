---
name: resolving-issues
description: Use when running a scheduled pass over the open issue + PR queue to clear small-scope fixes, or when /resolving-issues is invoked manually
user-invocable: true
---

# Resolving Issues

You = coordinator. Subagents = workers. You dispatch + monitor, never touch files.

## When to Use

- Scheduled run: sweep open issues + PRs, dispatch subagents to fix small items in parallel.
- Manual invoke: same flow, on demand.

## Required Skills

- **REQUIRED:** `superpowers:dispatching-parallel-agents` — fan out work.
- **REQUIRED:** `superpowers:using-git-worktrees` — isolate each subagent.
- **REQUIRED:** `superpowers:caveman` — all GH comms.
- **REQUIRED for subagents:** `superpowers:test-driven-development`, `superpowers:verification-before-completion`.

## Core Loop

1. Read open issues + open PRs. Skip anything already in flight.
2. Pick small-scope fixes. `general-audit` issues = top priority.
3. Skip big features + major refactors. Out of scope here.
4. Dispatch subagent per issue, max 3 parallel via git worktrees.
5. Subagent: fix → tests → PR with `Fixes #N` and caveman body.
6. Follow-up issue filed when subagent finds related rot.
7. No work fits? Noop fine.

## Rules

### Coordinator never codes
- Read, dispatch, monitor. Subagents touch files.
- One worktree per subagent. Max 3 concurrent. Queue the rest.

### Scope filter
- Fixes + small-scope changes only.
- No big features. No major refactors. No architecture rewrites.
- Unsure too big? Draft PR + questions in body. Wait for human reply.

### PR rules
- One PR per issue (or tight batch).
- `Fixes #N` in body to auto-close on merge.
- `just check` green before marking ready.
- Blocked? Draft PR + questions. Do not force.

### GitHub comms
- All issue + PR bodies + comments in caveman mode.
- Code blocks + security warnings stay normal.

### Autonomy
- Best judgment. No hand-holding.
- Direction needed? Draft PR + question. Do not stall.
- Noop fine. Ship nothing > ship junk.

## Setup

- Pre-worktree: `git stash` or `git restore` main dir; `.claude/worktrees/` in `.gitignore`.
- Tear down worktree after PR merges or closes.

## Quality

- `just check` green before PR ready.
- Tests at lowest tier covering behavior (see `CLAUDE.md`).
- Spot-check each subagent's diff before marking PR ready.
