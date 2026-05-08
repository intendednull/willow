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
6. **Coordinator-direct already-fixed sweep (run BEFORE any implementer dispatch).** For each picked issue, before dispatching, scan recent merged commits + closing PRs against the issue's scope:
   - `git log --oneline <last-audit-ref>..origin/main -- <relevant paths>` to find candidate fix commits
   - `mcp__github__search_pull_requests` w/ issue keywords for closure-by-PR cases
   - cross-check against recent general-audit master tickets ("verified fixed by ..." entries)
   
   If the issue is resolved by a landed change, do the close pass directly — no implementer dispatch:
   - caveman comment on the issue citing the upstream commit / PR + the fix location
   - close `completed` (audit intent now holds) or `not_planned` (audit premise moot)
   - record under `## Already-Fixed` for master PR body
   - if closed issue is a sub-issue of a parent, run parent close-out check (see ## Parent issue close-out) — close parent too if this was its last open child
   
   Closing GH issues = metadata work, not code work; allowed under "coordinator never codes." Implementers only dispatch for *unresolved* issues. This pass typically clears audit lessons (already folded into skills), structural-deps follow-ups (still structural), and audit findings closed by intervening fixes — saves dispatch overhead on no-op work.

   **When same-day audit-to-fix gap, expect ~zero already-fixed hits.** The sweep yields most when the audit ran days/weeks before the fix run (intervening PRs land between audit and fix). When the audit was filed earlier the same day against the current `main`, no fixes can have landed between audit and fix run by definition — the sweep correctly returns empty. Don't over-invest sweep effort in this case; one quick `git log <audit-ref>..origin/main` check on each pick's path is enough to confirm.

   **Ambiguous-fix-path (audit premise real, fix is a design call) — coordinator-skip without close.** Some audit findings flag a real concern but the prescribed fix is ambiguous-by-design with ≥ 2 legitimate approaches that need a design decision the coordinator can't make unilaterally. Common shape: "audit-glob doesn't catch in-file `mod tests` — move to external test file OR update glob." Both are valid; picking either silently is wrong. **Action:** during step 6 picks, if the audit's literal fix conflicts with HEAD reality (e.g. code already satisfies the surface premise) AND the underlying concern requires a design call, **skip the issue from this run's dispatch queue without closing it**. Note it in the run-end `## Lessons Learned` so the next session has full context. Don't comment on the issue — the audit description already captures the concern; a "skipped, design call needed" comment adds noise without progress. The next run with fresh eyes / accumulated context can decide.
7. Per remaining issue, sequential, max 10 per run:
   - **Pre-dispatch sync:** before spawning each implementer, in the coordinator's checkout:
     ```bash
     git fetch origin <master-branch>
     git reset --hard origin/<master-branch>
     ```
     Prior implementers' commits must be the implementer's base; stale local state poisons the next dispatch.
   - Spawn fresh implementer agent (see ## Implementer Agent below).
   - Implementer commits directly to master branch and pushes. No worktree, no sub-PR.
   - Track `Fixes #N` for final PR body assembly.
   - Next issue.
8. Implementer finds related rot? File follow-up issue.
9. Apply Lessons Learned skill edits to `.claude/skills/resolving-issues/SKILL.md`, commit on master branch, push. (Coordinator does this directly — see ### Coordinator never codes.)
10. Open the master PR — **ready (not draft)** — base `main`, head master branch. Body: `Fixes #N` list + `## Already-Fixed` + `## Parked` + `## Skill Evolution` + `## Lessons Learned`. Master PR runs full CI; human merges when satisfied. If anything's unfinished, leave the branch un-PR'd instead of opening a draft.

## Implementer Agent

Fresh agent per issue, scoped to one issue + master branch ref. Steps:

1. Read the issue. Decide if more context needed.
2. **Research (optional, parallel OK):** spawn research subagents for codebase grep, related-file reads, spec lookups. Synthesize before coding. Re-grep cited line numbers / LOC counts at HEAD before working from issue-body literal positions — they drift fast across refactors and may be off by hundreds of lines after a recent file move.

   **Verify the audit's claimed mechanism, not just its line numbers.** Audits sometimes prescribe a fix that depends on a stated mechanism — "log via `tracing::warn!`", "the rebuild Effect recovers dropped inserts", "the existing CSP test would catch this". That stated mechanism may (a) violate a module-local constraint (e.g. a privacy contract in the module doc forbidding `tracing::*`), or (b) describe code that doesn't actually exist as cited (e.g. a "rebuild Effect" referenced in a doc-comment but not present in any `app.rs` Effect — the actual recovery path is a different shape with the same fragility). **Pre-flight read** the cited file's module-doc + the cited recovery code at HEAD before dispatching. If either check fails, the issue is an **ambiguous-fix-path** (see step 6 of the Core Loop): coordinator-skip without close, note in run-end Lessons Learned, leave for a future session with fresh eyes. Saves a wasted dispatch + a likely rejected commit. Examples surfaced this way: `crates/client/src/search/handle.rs` module-doc forbids `tracing::*` (privacy contract) — kills audit's "log the drop" prescription; the "rebuild Effect" cited in `insert`'s doc-comment is itself misleading (only an event-loop subscription exists, sharing the same `do_send` fragility).
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

   **Cargo.lock conflicts with in-flight PRs are usually additive — don't refuse the dep.** When a fix needs a new workspace dep and `Cargo.lock` is in another open PR's file list, don't abort: dep additions are strictly additive in `Cargo.lock` (your row gets appended; the other PR's rows stay untouched). Merge resolution post-PR-#X is mechanical. Note the `Cargo.lock` churn in the commit body so the human reviewer expects a small textual conflict on the master PR's merge. Refusing on this basis creates infinite "wait for PR X to merge" deadlocks that block dependency upgrades and dual-target fixes indefinitely. Same logic applies to additive-only edits in any "shared by every change" file (e.g. workspace `Cargo.toml`'s `[workspace.dependencies]` table) — coordinator-narrowed briefs should explicitly authorise additive churn.

7. **Local merge gate.** Run, in order:
   - `cargo fmt --all -- --check` (or `just fmt-check` if available)
   - `cargo clippy <scope> --all-targets -- -D warnings` — scope to touched crate(s) for speed; workspace-wide if changes ripple
   - `cargo test <scope>` — ditto on scope
   - `cargo check --target wasm32-unknown-unknown <scope>` if dual-target lib crate touched
   - `cargo clippy --target wasm32-unknown-unknown <scope> --all-targets -- -D warnings` if dual-target lib crate touched — wasm-only lints (e.g. unnecessary `as u32` on `js_sys::Date::get_month()` which already returns `u32` on wasm32, or `js_sys`-only call paths under `#[cfg(target_arch = "wasm32")]`) only fire on the wasm target. Native clippy misses them, and CI's `cargo check --target wasm32` catches compile errors but not lint warnings — the wasm-clippy gate is the only path that closes that gap locally
   - `just check` if available + scope is wide enough to warrant the full sweep
   
   Apply `superpowers:verification-before-completion` — confirm command output before claiming done.
   
   **`just` may be absent in some sandboxes.** Fall back to raw `cargo` equivalents (same gate, different binary). Note which path was used in the commit body if unusual.
   
   **Browser tests (`wasm-pack` + Firefox + geckodriver) may be unavailable.** `cargo check --target wasm32-unknown-unknown -p willow-web --tests` is the fallback gate — confirms the test compiles. Real headless run executes on master-PR CI. Flag the gap in the commit body.

   **Non-Rust file changes still need `cargo test` on related crates.** Coordinator briefs sometimes argue "CSS / HTML / JSON / YAML / `.well-known` change — no Rust touched, skip cargo test." This is a trap: integration tests (`crates/<x>/tests/*.rs`) commonly assert on the contents of static assets — e.g. `crates/web/tests/static_assets.rs` greps `index.html` for required CSP directives, manifest.json for icon refs, etc. A non-Rust change that flips an asserted substring is a CI-red landmine. **Default rule:** if you touched any file in `crates/<x>/` or anywhere those crates' tests read at runtime (HTML / CSS / JSON / TOML / YAML / sw.js), still run `cargo test -p <x>` (or at minimum `cargo test -p <x> --test <integration_test_name>` if you can identify the relevant integration suite). Cost is a few seconds; saves a master-PR CI-rescue dispatch (~5-10 min) and a noisy `wip`-shape commit on the master branch.

8. **Commit + push.** Use `caveman:caveman-commit` for the message. Conventional Commits format. `Refs #N` (NOT `Fixes` — that lives only on the master PR). Push directly to origin master branch.

   **Never push `wip:` / `chore: checkpoint` / "work-in-progress" commits to the master branch.** Make all your edits, run the local gate, then commit ONCE with the proper Conventional Commits message. Coordinator's master PR body assembles `Fixes #N` rows from per-commit messages — `wip:` rows look like junk to a human reviewer and force a finalize-implementer detour to squash + force-push (real cost: ~10–15 min of cargo lock contention while the rescue agent re-runs gates, plus a `--force-with-lease` against a branch that may have already accumulated more commits from later dispatches if the coordinator misjudges sequencing). If you genuinely need intermediate checkpoints (long sessions, sandbox interference per next bullet), make them in a Pattern B local feature branch and squash via `git merge --no-ff` when done — never push intermediate commits to master.

   **Sandbox `git reset --hard origin/<branch>` interference (known hazard).** Some sandboxed environments run a periodic `git reset --hard origin/<branch>` between tool invocations that silently rolls back uncommitted edits — visible as `Edit`/`Write` results vanishing between cargo commands, or as the working tree being clean when you expected staged changes. Detection: run `git status` after a tool call you expected to leave changes; if it's clean and the file content matches origin, the sandbox wiped it. Recovery: apply edits and `git add -A && git commit` in a tight single-shell pipeline (one `bash -c` invocation) before the next tool call lands. If you accumulate commits-as-checkpoints this way, follow the no-wip-commits rule above by squashing at the end via `git reset --soft <pre-dispatch-SHA> && git commit -m "<final message>" && git push --force-with-lease`. Note the sandbox-interference workaround in the commit body so the human can audit.

   **Pre-staged working tree at session start (expected, NOT anomalous).** The opposite of the sandbox-reset case: implementers commonly find their prescribed diff already applied as uncommitted edits in the working tree the moment they start. Observed across multiple runs (~40-50% of dispatches in PR #566's 10-dispatch run, #567's 9-dispatch run). Mechanism is unclear — possibly the harness caches edits between implementer invocations, or prior session's uncommitted work survives in the sandbox. **Action:** when you see a dirty `git status` matching the brief's prescribed diff at session start, verify the content matches the design (read the file, diff against the brief), run the local merge gate, and commit. Do NOT redo the work — that wastes cycles and risks divergence from the staged version. Do NOT panic — this is now an expected pattern, not a bug. Note the pre-staged finding in the commit body briefly ("working tree had the prescribed diff at session start; verified diff and committed") so the human has the audit trail. Distinguish from the sandbox-reset case above: pre-staged means edits *exist that you didn't make yet* (apply gate + commit); sandbox-reset means edits *vanish that you did make* (re-apply in tight pipeline).

9. **Mid-fix block** (CI red on the local gate that won't resolve, brainstorm reveals deeper structural issue, fix demands cross-cutting refactor): **abort the dispatch.** `git checkout <master-branch>` + `git reset --hard origin/<master-branch>` to drop any local work. File a follow-up GH issue (caveman body, link original + cite the blocker). Return to coordinator. The follow-up issue is the durable handoff for the next scheduled run.

10. **Already-fixed-upstream path:** if pre-flight investigation (e.g. `cargo audit`, file-state grep, `cargo tree`) shows the issue was resolved by a recently-merged upstream PR, do NOT make a no-op commit. Leave a caveman comment on the original issue naming the upstream PR + the fix location, close the issue (`completed` if the audit's intent now holds — the upstream fix solved it for us; `not_planned` if the audit's premise is moot — e.g. the targeted code was deleted), report back. Coordinator records under `## Already-Fixed` in the master PR — NOT under `Fixes`.

11. **Stale-audit-with-residual-gap path:** if pre-flight investigation shows the audit's literal premise is stale (e.g. "zero tests" — but a later PR added some) but its underlying concern is partially valid (some specific gap remains), narrow scope to the residual gap and ship that. Note the audit's stale framing + cite the upstream PR that resolved most of it in the commit body. Coordinator still records under `Fixes #N` because the audit issue is the right closer.

12. **Structural-deps follow-up family path:** dependency-multi-version audits (rand, getrandom, convert_case, bincode, etc.) often look "obvious" but are pinned by transitive crates we don't own — no workspace pin / `[patch]` can collapse them without lying about semver. The first 1–2 finds get individual follow-up trackers. On the **3rd** structural-deps follow-up in this family, file or update a single **upstream-domino meta-tracker issue** instead of another standalone TD-NN follow-up — list the holdout crates, the upstream releases that would unblock each version (e.g. `aes-gcm 0.11` stable, `derive_more 3.x`, `iroh ≥ N`), and link prior individual follow-ups under it. Future runs check the meta-tracker, don't refile the same shape.

   **Retroactive meta-tracker fill-in (coordinator-direct, no implementer dispatch).** When a run's triage finds 3+ structural-deps trackers already exist *without* a consolidating meta-tracker, the coordinator files the meta-tracker as part of the step 6 already-fixed sweep — same pattern as closing already-fixed issues, pure metadata work, falls under the "Coordinator never codes" exceptions because no source files are touched. List rows for every active tracker, link them under the meta, comment on each individual tracker citing the meta. Skill compliance is *retroactive*: fix the gap when you spot it, don't leave the next run to re-derive the consolidation. Record the new meta issue under `## Skill Evolution` in the master PR body alongside the lessons.

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
   - `## Fixes` — `Fixes #N` lines, one per resolved issue, with a 1-line summary per row. If the children listed here would close the last open sub-issue of a parent on merge, add `Fixes #<parent>` so the merge closes the parent atomically (see ## Parent issue close-out).
   - `## Already-Fixed` — issues that were already resolved upstream and closed during this run.
   - `## Parked` — issues that hit a scope blocker mid-run; cite the follow-up issue per row.
   - `## Skill Evolution` — if the skill was edited, link the commit SHA + summarize what changed.
   - `## Lessons Learned` — caveman bullets on what worked / what didn't.
   - `## Test plan` — what the master-PR CI will run + any manual smoke notes.
5. **Always open the master PR with what landed**, even if some attempted issues hit blockers. The PR ships the wins; blockers move to follow-up issues so the next scheduled run picks them up automatically. Never open as draft. If literally nothing landed (zero implementer commits), don't open a PR at all — close the branch out.
6. **No in-session continuation.** Don't leave a session "to be resumed". The issue queue is the durable handoff.

## Parent issue close-out

Any issue we resolve may be a sub-issue under a parent (general-audit master ticket, epic, tracker, meta-issue, or any other parent). Closing the last open sub-issue must close the parent — else the parent lingers as a zombie ticket.

Coordinator owns this check (metadata work, allowed under "coordinator never codes"). Two trigger points:

1. **Step 6 sweep direct close.** After closing a sub-issue via `mcp__github__issue_write`, fetch the parent's remaining sub-issues. All closed → close parent w/ caveman comment citing the child closures + master branch ref, `state=closed reason=completed` (or `not_planned` if the parent's premise is moot for the whole batch). Record parent under `## Already-Fixed` in master PR body.
2. **Step 10 master PR body assembly.** For each `Fixes #N` where N is a sub-issue, check the parent's other sub-issues — already-closed + ones this PR will close on merge. If this PR's `Fixes` list completes the parent, add `Fixes #<parent>` to the body. Master PR merge then closes parent atomically w/ children.

**Detect parent.** Use GitHub's sub-issue API (preferred — `mcp__github__sub_issue_write` family + `mcp__github__issue_read` to list a parent's sub-issues). Fall back to body-checklist parsing only when sub-issue links are absent (some older trackers list children as a markdown checklist instead of via the sub-issue API).

**Edge cases:**
- Parent has open sub-issues outside this run's scope → leave parent open. Don't force-close to tidy up; remaining children are real work.
- Sub-issue closed `not_planned` (premise moot) → still counts toward parent close. Re-evaluate parent's own close reason: if premise moot for all sub-issues, parent likely closes `not_planned`; mixed (some completed, some moot) → use `completed` w/ comment noting the moot ones.
- Follow-up issues filed mid-run (scope-creep guards, pre-existing rot, structural-deps tracker) ≠ sub-issues unless explicitly linked under the parent via the sub-issue API. Don't conflate.
- Multi-level nesting (parent → sub-parent → leaf): after closing a sub-parent, recurse the same check up the chain. Each level closes only when its own remaining sub-issues are all closed.

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

### Waiting for implementer commits without polling
- Implementer agent runs in the background. Coordinator gets a notification when the agent's tool call completes — but the agent's *commit* may land seconds before that, and the harness's stop hook fires on uncommitted changes during cargo gates.
- **Don't sleep, don't poll, don't read the agent's output file.** The system explicitly blocks long sleeps and warns against reading sub-agent transcripts (context overflow).
- **Capture the full pre-dispatch SHA via `git log -1 --format='%H' <branch>` BEFORE arming the wait.** Use that exact 40-char string in the until-condition. Never type/synthesize the hash from a short prefix (`31c7851` ≠ `31c7851bf8c6...` — the loop will exit immediately on the mismatched comparison and you'll spuriously conclude the agent committed). Pattern: capture into a shell variable, interpolate into the loop.
- **Use `Monitor` (or `Bash` background with an `until` loop) watching for HEAD to advance** past the captured SHA. One notification fires when the loop exits; the coordinator stays idle in between. Pattern:
  ```bash
  PRIOR=$(git log -1 --format='%H' <branch>)
  until [ "$(git log -1 --format='%H' <branch>)" != "$PRIOR" ]; do sleep 10; done
  git log -1 --oneline <branch>
  ```
  Set `timeout` to ~10–15 minutes for typical implementer runs (longer for cross-crate gates). The coordinator can do GitHub-API metadata work (filing follow-up issues, drafting PR body) while waiting; just don't touch files in the implementer's scope.
- **Two independent signals: SHA-advance + agent-completion notification.** They arrive on their own schedules — agent-completion can lag the commit by minutes (or vice versa, agent can complete WITHOUT committing — see "Implementer cuts off mid-flight" below). Trust whichever signal answers your immediate question:
  - "Did the implementer's work land on master?" → check git ref / wait for SHA-advance.
  - "Did the implementer agent terminate?" → wait for agent-completion notification.
  Don't treat them as redundant — and don't preemptively conclude the implementer crashed just because one arrived first.

### Implementer cuts off mid-flight (uncommitted edits, agent terminated)
- A surprisingly common failure mode: the implementer agent gets cut off (token budget, time slice, harness limit) AFTER making the substantive edits but BEFORE running the local merge gate or committing. Symptoms:
  - Agent-completion notification arrives with a truncated/incomplete summary (e.g. "Spec doc looks good. Let me check the test result." mid-thought).
  - `git status` shows uncommitted changes matching the issue's expected scope.
  - Stop hook fires complaining about uncommitted changes.
- **Don't conclude the work is wrong** — inspect the diff. If it matches the issue's intent, the edits are usually correct; the agent just ran out of runway before the gate-and-commit step.
- **Don't restart the issue from scratch.** Dispatch a thin **finalize-implementer** agent: brief it on the existing uncommitted state, hand it the gate list + commit-and-push instructions verbatim, no need to re-do brainstorming or TDD-red. The previous implementer's substance stands; the finalize agent's job is the mechanical close-out (verify gates, commit, push, report SHA).
- **Brief the finalize agent on what's already done** — paste a `git diff --stat` summary so it knows the scope and doesn't redo the work or panic at the size. Cite the previous agent's brainstorm decision (Option A/B etc.) so it carries the same reasoning into the commit body.
- **Stop-hook noise is normal mid-implementer.** A `git status` showing uncommitted changes during a cargo gate run does NOT mean the implementer crashed. Only suspect crash-before-commit if (a) sufficient time has passed for the gates to finish (typically 5–10 min for a small fix, longer for `just check` workspace-wide), AND (b) the agent-completion notification has arrived AND no commit was made. Don't preemptively dispatch a finalize agent based on stop-hook chatter alone.
- **Two false-alarm patterns to avoid:**
  1. *Wrong-SHA wait exits immediately, you assume the implementer never started.* (See "Capture the full pre-dispatch SHA" above.) Result: redundant finalize dispatch on a tree the original agent already cleanly committed.
  2. *Agent-completion notification lags the commit.* By the time you see the agent's summary, the work has been on origin for minutes. The redundant finalize agent will report "no changes to commit, gates green" — harmless but wasteful (~5 min cargo lock contention).
  Both are mitigated by: capture full SHA → arm SHA-watch → wait for either signal → on signal arrival, `git status` first to disambiguate (clean tree = real commit landed; uncommitted edits matching scope = mid-flight, dispatch finalize).

- **Coordinator-side post-dispatch push fallback (sandbox-shared filesystem).** Some harness configurations share the working tree across coordinator + implementer agents. The implementer can commit locally without its `git push` step landing on origin — coordinator's tree shows the new commit ahead of `origin/<branch>` while `git ls-remote origin <branch>` still returns the pre-dispatch SHA. Symptoms: stop-hook fires "N unpushed commit(s)"; `git status` clean; `git log origin/<branch>..HEAD` shows the implementer's commit; SHA-watch Monitor still hasn't fired because remote hasn't advanced. **Action:** push the local commit yourself — `git push origin <branch>`. Safe — if the implementer's push later beats yours to origin, your push wins and theirs reports up-to-date; if yours wins, same outcome reversed. Note the manual push in the run-end Lessons so the pattern is logged. Don't wait the full Monitor timeout assuming the implementer will push later — the implementer agent may already have exited (its push step was the casualty). Observed in PR #599's #540 dispatch: implementer's later report explicitly noted "commit was already on origin when I arrived" because the coordinator's manual push had landed it; the implementer was a no-op-validator in that case.

### Implementer-flagged out-of-scope rot
- When the implementer surfaces pre-existing rot it intentionally doesn't fix (e.g. unrelated wasm break under `--all-features`, dead-code warnings in untouched files), the coordinator files a follow-up GH issue using `mcp__github__issue_write` (metadata work, allowed under the Coordinator-never-codes rule). Cite the discovery context (which dispatch surfaced it, which commit + gate step) so the next run has full provenance.
- This is the same shape as the "implementer files follow-up" rule — coordinator just does the filing because the implementer is single-task and exits after commit.

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
