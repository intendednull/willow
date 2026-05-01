---
name: general-audit
description: Use when running a scheduled general audit of the Willow codebase, or when /general-audit is invoked on a pull request for review
user-invocable: true
---

# General Audit

You = master orchestrator. Job = find + file findings + self-improve. Resolution of findings = separate routine.

## Required Skills

- **REQUIRED:** `superpowers:dispatching-parallel-agents` — every audit run fans out subagents.
- **REQUIRED:** `superpowers:using-git-worktrees` — one worktree per subagent AND for the lessons-PR branch.
- **REQUIRED:** `caveman` — all GH comms (issues, comments, reviews, PR title + body). Code blocks + security warnings stay normal.
- **REQUIRED for verification pass:** `superpowers:verification-before-completion` — spot-check findings before filing.

## When to Use

- Scheduled run on `main`: full-tree audit, files findings as issues, opens lessons-PR.
- `/general-audit` invoked in a PR: review the PR only — no issues filed, no lessons-PR.

## Skip Window

Skip if HEAD == commit recorded in most recent `general-audit` master issue. PR-mode rules apply for diff-only review.

## Approach

**Full sweep every run with full resources.** Never assume fewer issues since the last run. Every audit fans out subagents in parallel.

Default split — one agent per concern:
- security → sub-split: input validation/DoS, auth/permissions, web/WASM, deps/supply-chain
- tech debt / code quality
- clean architecture (diff specs vs code; pass spec paths explicitly)
- test coverage
- general review

Spawn more if an area needs depth. For very large diffs, add per-crate agents on top of per-concern agents — both axes, not one or the other.

## Audit Pass Order

Run passes in order. Sibling-of-closed FIRST — small fix-scope mismatches are a top source of real findings on every run.

### Pass 1: Sibling-of-closed (high yield)

Pre-fetch closed PRs since last audit master:

```bash
LAST_AUDIT_DATE=$(gh issue view <last-audit-master> --json createdAt -q .createdAt)
gh pr list --state merged --base main \
  --json number,title,closingIssuesReferences,files \
  --search "merged:>$LAST_AUDIT_DATE"
```

For each closed PR, check three sub-patterns:

**(a) Scope-vs-claim mismatch.** Commit-subject prefix (`fix:`/`ci:`/`feat:`) claims broader than diff. E.g. `ci:` editing only `justfile`, not `.github/workflows/`.

```bash
# `ci:`-prefixed commits not touching .github/workflows/
git log --since="$LAST_AUDIT_DATE" --grep "^ci:" --name-only --format="%H %s" \
  | awk '/^ci:/ {commit=$0; next} /^[a-z]/ {if (!/.github\/workflows\//) print commit}'

# `fix: closes #N`-style commits not touching the file path #N referenced
```

**(b) Sibling files.** PR fixed bug in file X — grep for same symptom in adjacent files (handlers vs components, routes vs pages, replay vs storage).

**(c) API-added-without-caller.** PR closed issue by adding `pub fn` — verify ≥1 production caller exists. Closure-by-API-only without integration is a finding.

```bash
# For each new pub fn introduced in a closing PR, check production callers
git diff <pr-base>..<pr-head> -- '*.rs' | rg "^\+\s*pub (async )?fn (\w+)" -or '$2'
# Then for each fn name: rg "\.<fn_name>\(" crates --glob '!**/tests*'
```

### Pass 2: Standard sweep grep set

Run as a checklist; record everything found (do NOT pre-filter against existing issues — that contaminates context and biases the sweep):

```bash
# Security
rg -n "(^|\s)unsafe\s+(impl|fn|\{)" crates --glob '!**/tests*'
rg -n "\b(dbg!|todo!\(|unimplemented!\(|FIXME|HACK)" crates --glob '!**/tests*' --glob '!**/main.rs'
rg -n "Arc<\s*(parking_lot::)?(Mutex|RwLock)<" crates --glob '!**/tests*' | grep -v "lock-ok"
rg -n "(js_sys::eval|innerHTML|set_inner_html)" crates --glob '!**/tests*'

# Observability / UX (sibling of `.ok();` swallow)
rg -n "let _ = [a-z_]+\.[a-z_]+\(.*\)\.await" crates/web/src/components/ crates/web/src/handlers.rs crates/web/src/app.rs
rg -n "\.ok\(\);" crates/client/src crates/web/src --glob '!**/tests*' | head -40

# Architecture
rg -n "use anyhow|anyhow::|anyhow!\(" crates/state/src crates/transport/src crates/identity/src crates/messaging/src crates/crypto/src crates/common/src
rg -n "topics:|deps:|participants:|peers:|members:" crates/common/src crates/transport/src crates/state/src/event.rs

# Deps / supply chain
cargo audit -n --ignore $(grep -oE "RUSTSEC-[0-9]+-[0-9]+" .github/workflows/ci.yml | tr '\n' ' ' | sed 's/ /,/g')
grep -rn "cargo install [^-][^- ]*" docker/ .github/workflows/
grep -rn "FROM [^@]*$" docker/   # any unpinned base image?
```

### Pass 3: cargo-audit ignore-list drift

After `cargo audit` completes, diff the CI `--ignore` list against the current advisory DB. Stale RUSTSEC IDs that no longer match should be surfaced as low-priority cleanup ("N stale RUSTSEC IDs in ci.yml — can be pruned"). Cosmetic but accumulates.

### Pass 4: timeout backfill

If any agent timed out without writing findings, the orchestrator MUST manually sweep that concern before declaring the audit complete. Gaps from timed-out agents compound across runs.

## Synthesis

Audit passes finish first. Collect raw findings list. THEN — and only then — dedup against existing issues.

**No existing-issue lookups during audit passes.** Pre-fetched issue lists contaminate the orchestrator/agent context with prior framings and bias the sweep toward known patterns. Dedup happens **after** the fresh sweep is complete, in a separate dedup step.

### Dedup step (post-sweep, fresh subagent)

Spawn a **dedup subagent** (general-purpose, fresh context — orchestrator does NOT load issue lists into its own context). Pass it:
- the raw findings list
- a query budget for `mcp__github__search_issues`

Dedup subagent's job:
1. For each finding, query `mcp__github__search_issues` narrowly — by file path, symbol name, or RUSTSEC ID. **Use `search_issues` (not `list_issues`)** — `list_issues` exceeds the 78k-char tool-result cap when a label has many open issues.
   ```text
   mcp__github__search_issues "is:open repo:intendednull/willow <file-or-symbol>"
   mcp__github__search_issues "is:open repo:intendednull/willow label:audit <keyword>"
   ```
2. For each finding, return: `kept` | `dup of #N` | `superseded by #N`.
3. Return only the verdict list to the orchestrator. Do NOT echo issue bodies.

Orchestrator drops `dup`/`superseded` findings from the file-list. Survivors get filed.

### Verification step (fresh subagent)

Second fresh subagent verifies surviving findings real via grep/rg for exact patterns cited. Drop any finding whose verification grep returns 0 hits.

### File the issues

1. **Master issue** = commit hash + survivors list.
2. **Child issue** per surviving finding.
3. **Wire children as sub-issues of master** via `mcp__github__sub_issue_write` — surfaces children in master's UI panel without manual cross-ref.
4. **Lessons issue** titled `general-audit lessons: YYYY-MM-DD` (caveman body): what worked, what didn't, concrete suggested edits to this skill file. Label: `audit`, `lessons`.
5. **Open lessons-PR** (next section).

## Lessons-Learned PR (self-improvement loop)

After the lessons issue is filed, the audit MUST open a PR that folds the lesson's suggested edits into this SKILL.md file. The human reviews + merges as they see fit.

Steps:

1. Create worktree (use `superpowers:using-git-worktrees`) on branch `claude/general-audit-lessons-YYYY-MM-DD` from `main`.
2. Edit `.claude/skills/general-audit/SKILL.md` applying **each** "Suggested edits to `.claude/skills/general-audit/SKILL.md`" item from the lessons issue body. Apply mechanically — do not invent new edits, do not skip items because they "feel risky."
3. If a suggested edit is ambiguous or directly contradicts an earlier (still-applicable) lesson, leave it out and note `skipped: <reason>` for that item in the PR body so the human can adjudicate.
4. Commit with caveman subject, e.g. `chore(skill): fold audit lessons #<lesson-issue-number>`. Body lists each edit applied (or skipped + why).
5. Push the branch with `-u origin <branch-name>`. Retry up to 4× w/ exponential backoff (2s/4s/8s/16s) on network errors only.
6. Open PR via `mcp__github__create_pull_request`. Title: `general-audit: fold lessons #<lesson-issue-number>`. Body (caveman):
   - link to lessons issue (`Closes #<lesson-issue>` if the issue is fully addressed; `Refs #<lesson-issue>` otherwise)
   - bullet per edit applied
   - bullet per edit skipped + reason
   - footer: `Auto-generated by /general-audit. Human review required before merge.`
7. **Do NOT auto-merge.** Do NOT enable auto-merge. Do NOT request reviewers. Human merges as they see fit.
8. Report PR URL in the audit's terminal output / final message.

This closes the loop: each audit run feeds the next.

## Hard Rules

### Scope
- Audit full tree always. Never scope to diff (PR mode is the only exception, and PR mode files no issues).
- **No existing-issue lookups during audit passes.** Orchestrator + agents stay blind to the open-issue list. Pre-fetching contaminates context and biases the sweep toward known framings. Dedup is a separate post-sweep step in a fresh subagent (see Synthesis).
- File findings only. **No PRs to fix findings.** No auto-fix. No closing existing issues. **Sole exception: the lessons-learned PR above** — it edits the skill file, never application code.

### Agent prompts (mandatory fields)

- Time budget: 6 min, stop+save if exceeded.
- **Write findings in small chunks. Never big batches.** Big batched writes are the dominant timeout cause — stream-idle timeouts hit at the final large-write step and lose the entire run's work. Append each finding **as soon as it's identified** — one rg hit + one Read confirmation = one append. One finding per write. Do NOT accumulate findings in memory and dump them at the end.
- Scaffold report file before 2nd tool call. Then append one finding per write thereafter.
- Per-finding entry stays small: file:line, severity (split: security = confidentiality/integrity; robustness = availability/DoS), Obvious? yes/no. One short paragraph max. If a finding's evidence is large (a long grep result, a code block), summarise it in the entry and link to file:line — don't paste it inline.
- **Hard cap: > 5 tool calls without appending one finding ⇒ STOP exploring, write the strongest finding seen so far, then continue.** No exception.
- Count/ratio claims: verify w/ a second grep cmd proving count.
- Use general-purpose agent (Explore can't Write).
- Architecture agents: skip cargo tree/cargo clippy; rg + ls + reads.
- GitHub comms in caveman mode.

### Setup
- Pre-worktree: `git stash` or `git restore` main dir; `.claude/worktrees/` in `.gitignore`. One worktree per subagent AND one for the lessons-PR. Tear down audit worktrees after report submitted; lessons-PR worktree stays until PR merges/closes.
- `cargo install --locked cargo-audit` upfront (or verify); orchestrator runs `cargo audit` directly — no agent needed. Yank-check 403s are harmless noise.

### Quality
- Quality > speed. Always thorough path.
- Independently spot-check every filed finding.
