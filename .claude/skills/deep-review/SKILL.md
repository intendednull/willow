---
name: deep-review
description: Deep iterative code review — flags issues, spawns parallel analysis agents, presents findings, and loops until clean
user-invocable: true
---

# Deep Review

Perform a deep, iterative review of all changes in the current branch. This
skill finds potential issues, investigates them in parallel, presents a
structured table for triage, fixes selected issues, and repeats until clean.

## Workflow

### Phase 1: Discover Changes

Determine the diff to review:

```bash
# Find the merge base with main
BASE=$(git merge-base HEAD main 2>/dev/null || echo "HEAD~10")
git diff "$BASE"..HEAD --stat
git diff "$BASE"..HEAD
```

If there are no changes vs main, fall back to the last 5 commits:

```bash
git diff HEAD~5..HEAD
```

### Phase 2: Flag Potential Issues

Read through ALL changed files completely (not just the diff — full file
context matters). Build a numbered list of **potential issues**, including
but not limited to:

| Category | What to look for |
|---|---|
| **Bug risks** | Off-by-one, null/None mishandling, race conditions, missing error handling at boundaries |
| **Code smells** | Duplicated logic, overly complex functions (high cyclomatic complexity), unclear naming |
| **Security** | Input validation gaps, unsafe crypto usage, injection vectors, leaked secrets |
| **Correctness** | Logic errors, incorrect state transitions, unhandled edge cases |
| **Performance** | Unnecessary allocations, O(n^2) where O(n) is possible, missing caching |
| **Concurrency** | Data races, deadlock potential, missing synchronization |
| **API misuse** | Wrong trait bounds, incorrect lifetimes, misused library APIs |
| **Test gaps** | Changed logic with no corresponding test coverage |
| **WASM compat** | `std::fs`, `std::time::SystemTime`, `std::thread` in library crates (per CLAUDE.md) |

For each flagged area, record:
- **ID**: sequential number (1, 2, 3...)
- **File**: path and line range
- **Category**: from table above
- **Summary**: one-line description
- **Severity estimate**: Critical / High / Medium / Low

### Phase 3: Deep Investigation

After presenting the flagged issues, ask the user:

> Would you like me to spawn parallel sub-agents to investigate each issue
> in depth, or should I investigate them myself in this session?
> (Enter "agents" for parallel sub-agents, or "no" to investigate inline)

#### Option A: Parallel Sub-Agent Investigation (if user chose "agents")

For EACH flagged issue, spawn a parallel Agent to investigate it independently.

Each agent receives:
1. The full content of the relevant file(s)
2. The specific flag (category, summary, location)
3. Surrounding context (callers, related types, tests)

Agents should be thorough — read the actual code, trace call paths, check
for existing tests, and verify whether the concern is real or a false positive.

#### Option B: Inline Investigation (if user chose "no")

Investigate each flagged issue sequentially in the current session. For each
issue, read the relevant code, trace call paths, check for existing tests,
and determine whether the concern is real or a false positive.

#### Investigation Output Format

Each issue (whether investigated by agent or inline) MUST produce a
structured analysis in this exact format:

```
ISSUE_ID: <number>
FILE: <path>:<line_range>
CATEGORY: <category>
SEVERITY: Critical | High | Medium | Low
STATUS: Confirmed | False Positive | Needs Discussion
SUMMARY: <one-line summary>
ANALYSIS: <2-5 sentence detailed explanation>
SUGGESTED_FIX: <brief description of the fix, or "N/A" if false positive>
EFFORT: Trivial | Small | Medium | Large
```

### Phase 4: Present Results Table

Compile all agent results into a single markdown table and present it to
the user:

```
| # | File | Category | Severity | Status | Summary | Effort |
|---|------|----------|----------|--------|---------|--------|
| 1 | path:L10-20 | Bug risk | High | Confirmed | ... | Small |
| 2 | path:L55 | Code smell | Low | False Positive | ... | N/A |
```

Below the table, show the full analysis for each **Confirmed** or
**Needs Discussion** item.

Then ask the user:

> Which issues should I fix? Enter the issue numbers (e.g., "1, 3, 5"),
> "all" for all confirmed issues, or "none" to skip.

### Phase 5: Triage

- **Selected by user**: Move to fix phase
- **Not selected**: Mark as **Backlog** in the tracking table
- **False Positives**: Mark as **Ignored**

Maintain a running backlog table across iterations that shows all issues
from all passes with their current status.

### Phase 6: Fix

For each issue the user selected:
1. Implement the fix
2. Run relevant tests (`just test` or the appropriate crate-level test)
3. If tests fail, fix until they pass

After all fixes are applied, commit the changes with a descriptive message.

### Phase 7: Re-review (Loop)

After fixes are committed, start over from **Phase 1** — but this time
the diff includes the new fix commits. The review loop continues until:

- **No new issues are found**, OR
- **The user marks all remaining issues as Backlog/Ignored**

When the loop terminates, present the final cumulative table showing all
issues across all passes and their resolutions.

## Output Format

Always prefix status updates so the user can follow progress:

```
[deep-review] Phase 1: Analyzing changes (14 files, +320/-45 lines)
[deep-review] Phase 2: Flagged 7 potential issues
[deep-review] Phase 3: Investigating 7 issues in parallel...
[deep-review] Phase 4: Results ready — see table below
```

## Important Notes

- Read FULL files, not just diffs — bugs often hide in the interaction
  between new and existing code.
- Respect CLAUDE.md conventions: WASM compat, `Arc` not `Rc`, etc.
- When fixing, follow the project's testing strategy — add tests at the
  lowest level that covers the behavior.
- Do NOT fix issues the user hasn't approved — backlog them.
- Keep the cumulative table updated across all iterations.
