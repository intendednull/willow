---
name: resolving-issues
description: Use when running scheduled pass over open issue + PR queue to clear small-scope fixes, or when /resolving-issues invoked manually
user-invocable: true
---

# Resolving Issues

You = coordinator. Implementer subagents = workers. Read, dispatch, monitor. Never touch files (with two narrow exceptions — see ### Coordinator never codes).

## When

- Scheduled: sweep open issues + PRs, fix small items sequentially into one master PR.
- Manual: same flow, on demand.

## Required Skills

- **REQUIRED:** `caveman` — all GH comms.
- **REQUIRED for implementers:** `superpowers:test-driven-development`, `superpowers:verification-before-completion`, `superpowers:dispatching-parallel-agents` (for research subagents inside an implementer).
- **REQUIRED for implementers when complexity gate trips (see Implementer Agent step 3):** `superpowers:brainstorming` (run automated, single-actor — never ask the human; runs are unattended), `superpowers:writing-plans` (drop a plan file when the brainstorm lands a multi-step plan).

## Workflow Shape

**One master branch per session. All fixes land on it sequentially. No sub-PRs. No worktrees.** Master PR is opened at the end with everything in it.

Why this shape:

- **Sequential between issues** means no parallel writers, so no isolation needed → no worktrees.
- **No sub-PRs** means no "PR #N base ≠ main → CI silently doesn't run, but we squash-merged anyway" foot-gun, no out-of-order merges, no half-merged batches confusing the human. Git history stays clean.
- **One master PR at the end** is the only GitHub artifact the human reviews. Single CI run. Single merge.

## Master Branch Setup (start of run)

1. Fresh master branch per session, branched off latest `main`. Name: `auto-fix/batch-YYYY-MM-DD-HHMMSS`. (If the harness pre-assigned a session branch like `claude/<name>`, use that — it's the master branch for this session.)
2. Empty session-open commit so the branch is non-empty:
   ```bash
   git commit --allow-empty -m "chore: open auto-fix batch <branch-name>"
   git push -u origin <branch-name>
   ```
3. **Do NOT open a PR yet.** PR opens only at end of run.
4. Track resolved issues in coordinator memory or a scratch file — assemble final PR body at end.

## Core Loop

1. Read open issues + open PRs. Skip anything in flight.
2. Pick small-scope fixes. `general-audit` issues = top priority.
3. Skip big features + major refactors. Out of scope.
4. No in-scope issues? Noop. Skip the rest. No master branch created, no PR opened.
5. Create fresh master branch (see ## Master Branch Setup).
6. Per issue, sequential, max 10 per run:
   - **Pre-dispatch sync:** before spawning each implementer, in the coordinator's checkout:
     ```bash
     git fetch origin <master-branch>
     git reset --hard origin/<master-branch>
     ```
     Prior implementers' commits must be the implementer's base; stale local state poisons the next dispatch.
   - Spawn fresh implementer agent (see ## Implementer Agent below).
   - Implementer commits directly to master branch and pushes. No worktree, no sub-PR.
   - Track `Fixes #N` for final PR body assembly. **Already-fixed-upstream** issues go under `## Already-Fixed`, not `Fixes`.
   - Next issue.
7. Implementer finds related rot? File follow-up issue.
8. Apply Lessons Learned skill edits to `.claude/skills/resolving-issues/SKILL.md`, commit on master branch, push. (Coordinator does this directly — see ### Coordinator never codes.)
9. Open the master PR — **ready (not draft)** — base `main`, head master branch. Body: `Fixes #N` list + `## Already-Fixed` + `## Parked` + `## Skill Evolution` + `## Lessons Learned`. Master PR runs full CI; human merges when satisfied. If anything's unfinished, leave the branch un-PR'd instead of opening a draft.

## Implementer Agent

Fresh agent per issue, scoped to one issue + master branch ref. Steps:

1. Read the issue. Decide if more context needed.
2. **Research (optional, parallel OK):** spawn research subagents for codebase grep, related-file reads, spec lookups. Synthesize before coding. Re-grep cited line numbers / LOC counts at HEAD before working from issue-body literal positions — they drift fast across refactors and may be off by hundreds of lines after a recent file move.
3. **Complexity gate — automated brainstorm + plan when warranted:**
   - **Trigger any of:** issue spans > 1 crate, fix touches state machine / wire format / migration paths, ≥ 2 reasonable approaches exist, root cause not obvious from issue text, fix likely > 5 files OR > 200 LOC, "it depends" question on scope.
   - **Skip when:** issue is a one-liner / config swap / typo / clearly mechanical (single rg-pattern site) / has explicit "Suggested fix" the implementer can follow verbatim.
   - **If triggered, run automated:**
     1. Invoke `superpowers:brainstorming` self-driven — implementer plays both roles (exploration + decision). Do NOT ask the human anything; the run is unattended. Output: a written brief naming the chosen approach, the runner-up, and why rejected. Cap at 5 minutes / a few tool calls.
     2. If the brainstorm surfaces a multi-step plan, invoke `superpowers:writing-plans` to drop a `docs/plans/YYYY-MM-DD-<issue-N>-<slug>.md` on the master branch. Otherwise skip — small fixes don't need a plan file.
   - Fold the brainstorm + plan summary into the commit body so the human can review reasoning, not just code.
4. **Work directly on the master branch.** Two valid patterns; pick whichever keeps history cleanest:
   - **Pattern A — direct commit on master.** `git checkout <master-branch>` (already there from coordinator's pre-dispatch sync), apply changes, commit, push. Best for one logical commit per issue.
   - **Pattern B — local feature branch then merge back, no PR.** `git checkout -b auto-fix/issue-N-slug` off master, do the work in any number of local commits, then `git checkout <master-branch>` + `git merge --no-ff auto-fix/issue-N-slug -m "<conventional-commit-style merge message>"` + push master + delete local feature branch. Use when the work spans several commits worth keeping (e.g. "tests then fix"); the merge commit becomes the per-issue summary.
   - **Either pattern, no GitHub PR is opened.** No `mcp__github__create_pull_request` for sub-fixes. The master PR (end of run) is the only GitHub artifact.
5. Apply fix. Add tests at lowest tier covering behavior (see `CLAUDE.md` decision tree).
6. **Scope-creep guard:** if root-cause fix touches > 5 files OR > 200 LOC AND brainstorm in step 3 didn't already approve that scope, return to coordinator with a brainstorm note before pushing. Coordinator decides: split, defer, or proceed. Don't unilaterally balloon a small-scope ticket.

   **Mechanical call-site migration is part of the fix, not scope creep.** If the fix changes a small API (e.g. swapping `map.insert(k, v)` for `lru.insert(k, v)` to make a new cap take effect), every call-site rewrite is load-bearing — without them the cap is dead code. Count them in the LOC delta but don't abort just because they push past 200. Justify the count in the brainstorm + commit body so the human can see why the fan-out was unavoidable. Real scope creep = unrelated cleanup, drive-by refactors, "while I'm in here" tweaks — those still abort.
7. **Local merge gate.** Run, in order:
   - `cargo fmt --all -- --check` (or `just fmt-check` if available)
   - `cargo clippy <scope> --all-targets -- -D warnings` — scope to touched crate(s) for speed; workspace-wide if changes ripple
   - `cargo test <scope>` — ditto on scope
   - `cargo check --target wasm32-unknown-unknown <scope>` if dual-target lib crate touched
   - `just check` if available + scope is wide enough to warrant the full sweep
   
   Apply `superpowers:verification-before-completion` — confirm command output before claiming done.
   
   **`just` may be absent in some sandboxes.** Fall back to raw `cargo` equivalents (same gate, different binary). Note which path was used in the commit body if unusual.
   
   **Browser tests (`wasm-pack` + Firefox + geckodriver) may be unavailable.** `cargo check --target wasm32-unknown-unknown -p willow-web --tests` is the fallback gate — confirms the test compiles. Real headless run executes on master-PR CI. Flag the gap in the commit body.

8. **Commit + push.** Use `caveman:caveman-commit` for the message. Conventional Commits format. `Refs #N` (NOT `Fixes` — that lives only on the master PR). Push directly to origin master branch.

9. **Mid-fix block** (CI red on the local gate that won't resolve, brainstorm reveals deeper structural issue, fix demands cross-cutting refactor): **abort the dispatch.** `git checkout <master-branch>` + `git reset --hard origin/<master-branch>` to drop any local work. File a follow-up GH issue (caveman body, link original + cite the blocker). Return to coordinator. The follow-up issue is the durable handoff for the next scheduled run.

10. **Already-fixed-upstream path:** if pre-flight investigation (e.g. `cargo audit`, file-state grep, `cargo tree`) shows the issue was resolved by a recently-merged upstream PR, do NOT make a no-op commit. Leave a caveman comment on the original issue naming the upstream PR + the fix location, close the issue (`completed` if the audit's intent now holds — the upstream fix solved it for us; `not_planned` if the audit's premise is moot — e.g. the targeted code was deleted), report back. Coordinator records under `## Already-Fixed` in the master PR — NOT under `Fixes`.

11. **Stale-audit-with-residual-gap path:** if pre-flight investigation shows the audit's literal premise is stale (e.g. "zero tests" — but a later PR added some) but its underlying concern is partially valid (some specific gap remains), narrow scope to the residual gap and ship that. Note the audit's stale framing + cite the upstream PR that resolved most of it in the commit body. Coordinator still records under `Fixes #N` because the audit issue is the right closer.

12. **Structural-deps follow-up family path:** dependency-multi-version audits (rand, getrandom, convert_case, bincode, etc.) often look "obvious" but are pinned by transitive crates we don't own — no workspace pin / `[patch]` can collapse them without lying about semver. The first 1–2 finds get individual follow-up trackers. On the **3rd** structural-deps follow-up in this family, file or update a single **upstream-domino meta-tracker issue** instead of another standalone TD-NN follow-up — list the holdout crates, the upstream releases that would unblock each version (e.g. `aes-gcm 0.11` stable, `derive_more 3.x`, `iroh ≥ N`), and link prior individual follow-ups under it. Future runs check the meta-tracker, don't refile the same shape.

13. **Report back** to coordinator: commit SHA on master branch, sites touched, anything unusual.

## Lessons Learned

End of run, before opening the master PR:

1. Draft `## Lessons Learned` content with caveman bullets: what worked, what didn't, concrete suggested edits to this skill file.
2. **Apply the skill edits** to `.claude/skills/resolving-issues/SKILL.md` directly on the master branch. Commit + push. Editing the skill is meta-work, exempt from the "coordinator never codes" rule.
3. Open the master PR (ready, not draft) with body containing `Fixes #N` list + `## Already-Fixed` + `## Parked` + `## Skill Evolution` (referencing the skill commit SHA) + `## Lessons Learned`.

Never defer skill edits to a follow-up — they ship with the run that surfaced them, in the same PR.

## Master PR Open (end of run)

1. After all implementers have committed (or been parked as follow-up issues), and skill edits + Lessons Learned are committed, open the PR — **non-draft, ready for review**.
2. Title: `auto-fix batch <master-branch-name>` (or include date for clarity).
3. Base: `main`. Head: master branch. Apply label `auto-fix-batch` if it exists.
4. PR body sections:
   - `## Fixes` — `Fixes #N` lines, one per resolved issue, with a 1-line summary per row.
   - `## Already-Fixed` — issues that were already resolved upstream and closed during this run.
   - `## Parked` — issues that hit a scope blocker mid-run; cite the follow-up issue per row.
   - `## Skill Evolution` — if the skill was edited, link the commit SHA + summarize what changed.
   - `## Lessons Learned` — caveman bullets on what worked / what didn't.
   - `## Test plan` — what the master-PR CI will run + any manual smoke notes.
5. **Always open the master PR with what landed**, even if some attempted issues hit blockers. The PR ships the wins; blockers move to follow-up issues so the next scheduled run picks them up automatically. Never open as draft. If literally nothing landed (zero implementer commits), don't open a PR at all — close the branch out.
6. **No in-session continuation.** Don't leave a session "to be resumed". The issue queue is the durable handoff.

## Rules

### Coordinator never codes
- Read, dispatch, monitor. Implementers touch code files.
- One implementer at a time. Sequential between issues.
- **Two narrow exceptions where the coordinator commits directly:**
  1. The session-open empty commit on the master branch (start of run).
  2. Lessons Learned skill edits to `.claude/skills/resolving-issues/SKILL.md` on the master branch (end of run).
- **Webhook subscriptions are informational.** When `<github-webhook-activity>` arrives for the master PR after it opens, the master-PR CI is the authoritative quality net for the run — but the coordinator only acts on review comments, not on raw CI status. Acknowledge briefly, address review comments per the harness instructions, otherwise keep waiting.

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
- Mid-fix block? Implementer files a follow-up GH issue (caveman body, link original + cite blocker), aborts the dispatch with a hard reset to origin master, moves on.
- Noop fine. Ship nothing > ship junk.
- **No in-session continuation.** Sessions don't get resumed. If something doesn't fit in this run, file an issue.

## Quality

- Local cargo gate (fmt + clippy + test + wasm-check on touched crates) green before each implementer pushes.
- Tests at lowest tier covering behavior (see `CLAUDE.md` decision tree).
- Master PR runs full CI when opened — the load-bearing quality net for the run.
- Anything blocked mid-run → follow-up issue, dispatch aborted, next scheduled run picks it up. Never open the master PR as draft. Only skip opening the PR entirely if literally zero implementer commits landed.
- No in-session continuation. The issue queue is the durable handoff.
