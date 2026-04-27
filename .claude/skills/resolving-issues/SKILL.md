---
name: resolving-issues
description: Use when running scheduled pass over open issue + PR queue to clear small-scope fixes, or when /resolving-issues invoked manually
user-invocable: true
---

# Resolving Issues

You = coordinator. Fresh subagents = workers. Read, dispatch, monitor. Never touch files.

## When

- Scheduled: sweep open issues + PRs, fix small items sequentially.
- Manual: same flow, on demand.

## Required Skills

- **REQUIRED:** `superpowers:using-git-worktrees` — isolate each subagent.
- **REQUIRED:** `caveman` — all GH comms.
- **REQUIRED for subagents:** `superpowers:test-driven-development`, `superpowers:verification-before-completion`.

## Core Loop

1. Read open issues + open PRs. Skip anything in flight.
2. Pick small-scope fixes. `general-audit` issues = top priority.
3. Skip big features + major refactors. Out of scope.
4. Per issue, sequential, max 10 per run:
   - Spawn fresh worktree.
   - Dispatch fresh subagent. Subagent: fix → tests → PR with `Fixes #N` + caveman body.
   - Wait PR ready (or draft).
   - Tear down worktree.
   - Next issue.
5. Subagent finds related rot? File follow-up issue.
6. No work fits? Noop fine.

## Rules

### Coordinator never codes
- Read, dispatch, monitor. Subagents touch files.
- One worktree per issue. Sequential. Tear down before next.

### Sequential, not parallel
- One issue at a time. No parallel subagents.
- Save resources. Cap = 10 issues per run.

### Fresh agent per issue
- New subagent each issue. No state leak.
- Subagent gets only that issue's context.

### Scope filter
- Fixes + small-scope only.
- No big features. No major refactors. No architecture rewrites.
- Too big? Draft PR + questions in body. Wait for human reply.

### PR rules
- One PR per issue (or tight batch).
- `Fixes #N` in body to auto-close on merge.
- `just check` green before ready.
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
- Worktree per issue. Tear down after PR ready/draft/closed.

## Quality

- `just check` green before PR ready.
- Tests at lowest tier covering behavior (see `CLAUDE.md`).
- Spot-check each subagent's diff before marking PR ready.
