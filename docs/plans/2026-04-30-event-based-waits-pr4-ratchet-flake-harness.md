# Event-Based Waits PR-4 — Wait-Timeout Ratchet + Flake Harness

**Status:** landed (commit `5b84ef5` ratchet enforcer; `1283430` wired into `check-all`; `eeed70f` follow-up fixes) — `scripts/check-wait-timeout-count.sh` enforces the baseline; `e2e/.wait-timeout-baseline` is the source of truth; `test-e2e-flake` + `check-wait-timeout` recipes in `justfile`. Plan task boxes never got ticked but all artifacts ship.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lock in the current `waitForTimeout` count so future PRs cannot regress, ship a flake harness that runs the e2e suite N times to surface intermittent failures, and prove the ESLint rule's sunset cutoff is enforced by CI.

**Architecture:** A single shell script `scripts/check-wait-timeout-count.sh` reads the baseline count from `e2e/.wait-timeout-baseline`, recounts `waitForTimeout` in `e2e/` (specs + helpers), and exits non-zero if the count exceeds the baseline. The script also fails if the current date is past the sunset cutoff (2026-09-30 per spec) AND any `eslint-disable.*no-restricted-syntax` headers are still present in the spec tree — that's the forcing function for the migration to complete. The flake harness is a new `test-e2e-flake N=5` recipe in `justfile` that wraps `npx playwright test` in a bash loop and aggregates pass/fail across runs. Both land in `just check-all` so any regression fails CI.

**Tech Stack:** POSIX shell (bash), `just`, ripgrep (fallback to `grep -r`), Playwright 1.58.

**Spec:** [`docs/specs/2026-04-27-event-based-waits-design.md`](../specs/2026-04-27-event-based-waits-design.md) §"Implementation phasing" PR 4 entry + §"CI gate".
**Predecessor:** PR-3 (#496, merged in `a166f2b`).
**Tracking issue:** [#458](https://github.com/intendednull/willow/issues/458).

---

## File Structure

**Create:**
- `scripts/check-wait-timeout-count.sh` — POSIX bash script, executable.
- `e2e/.wait-timeout-baseline` — single integer line (current count).

**Modify:**
- `justfile` — add `test-e2e-flake N=5` recipe; add `check-wait-timeout-count` invocation to `check-all`.
- `e2e/README.md` — append a "Wait-timeout ratchet" section.

**Untouched:** all 7 specs that still carry `eslint-disable` headers stay un-migrated. PR-4 does NOT do migration work; it just locks the count and adds enforcement so that file-by-file migration (via #458) ratchets monotonically.

---

## Task 0: Preflight

- [ ] **Step 1: Confirm git state**

```bash
cd /home/user/willow/.worktrees/pr4
git status
git log --oneline -3
```

Expected: clean tree on `claude/event-based-waits-pr4`, head is `a166f2b` (PR-3 merge).

- [ ] **Step 2: Capture the current count exactly**

```bash
# Count waitForTimeout occurrences in e2e/, excluding generated and ignored paths.
# This is the authoritative count that goes into the baseline file.
COUNT=$(grep -roh "waitForTimeout" e2e/ --include='*.ts' | wc -l | tr -d ' ')
echo "current waitForTimeout count in e2e/*.ts: $COUNT"
```

Expected output: `54` (validated by the controller before plan-write). If the number differs, **stop and report** — the worktree may have drifted.

---

## Task 1: Write `e2e/.wait-timeout-baseline`

**Files:**
- Create: `e2e/.wait-timeout-baseline`

A single-line file containing the integer count. No trailing comment, no metadata — keep it dumb so downstream tools never get confused about how to read it. The number IS the spec.

- [ ] **Step 1: Write the file**

```bash
echo "54" > e2e/.wait-timeout-baseline
cat e2e/.wait-timeout-baseline   # confirm
```

- [ ] **Step 2: Commit (no other changes yet — script lands next)**

```bash
git add e2e/.wait-timeout-baseline
git commit -m "test(e2e): lock the waitForTimeout baseline at 54

Snapshot of the current count in e2e/*.ts at PR-4 land time. Read by
scripts/check-wait-timeout-count.sh (next commit). Future PRs that add
new waitForTimeout calls fail the ratchet; future PRs that delete
calls update this file in the same commit so it monotonically
descends to zero by the 2026-09-30 sunset (per spec §Sunset).

Per docs/specs/2026-04-27-event-based-waits-design.md §Implementation
phasing PR 4."
```

---

## Task 2: Write `scripts/check-wait-timeout-count.sh`

**Files:**
- Create: `scripts/check-wait-timeout-count.sh`

The script does three things:
1. Read the baseline integer from `e2e/.wait-timeout-baseline`.
2. Recount `waitForTimeout` in `e2e/*.ts` (specs + helpers).
3. Fail (exit 1) if the recount exceeds the baseline. Pass with a friendly note if the recount is below baseline (and prompt the author to update the baseline file in the same PR).
4. **Sunset enforcement:** if today's date is on or after `2026-09-30` AND any spec file under `e2e/*.spec.ts` still has the `eslint-disable.*no-restricted-syntax` header, fail with a pointer to issue #458.

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# scripts/check-wait-timeout-count.sh
#
# Ratchet enforcer for `page.waitForTimeout` in e2e/*.ts. Reads the
# baseline integer from e2e/.wait-timeout-baseline and fails if the
# current count exceeds it. Below-baseline counts pass with a hint
# to update the baseline file in the same PR.
#
# Also enforces the spec's sunset cutoff: after 2026-09-30, any spec
# file that still carries an `eslint-disable` header for
# `no-restricted-syntax` fails. This is the forcing function for the
# file-by-file migration tracked at:
#   https://github.com/intendednull/willow/issues/458
#
# Per docs/specs/2026-04-27-event-based-waits-design.md §CI gate.

set -euo pipefail

BASELINE_FILE="e2e/.wait-timeout-baseline"
SUNSET_DATE="2026-09-30"

if [[ ! -f "$BASELINE_FILE" ]]; then
    echo "error: $BASELINE_FILE not found — run from repo root" >&2
    exit 2
fi

baseline=$(< "$BASELINE_FILE")
if ! [[ "$baseline" =~ ^[0-9]+$ ]]; then
    echo "error: $BASELINE_FILE must contain a single non-negative integer (got: '$baseline')" >&2
    exit 2
fi

# Count occurrences across all .ts files in e2e/. Use grep -roh so we
# count every occurrence (not just files containing one) and sum to a
# single integer.
current=$(grep -roh "waitForTimeout" e2e/ --include='*.ts' 2>/dev/null | wc -l | tr -d ' ')

echo "waitForTimeout count: current=$current baseline=$baseline"

if (( current > baseline )); then
    cat >&2 <<EOF
error: waitForTimeout count regressed
  baseline (e2e/.wait-timeout-baseline): $baseline
  current (e2e/*.ts):                    $current
  delta:                                 +$((current - baseline))

The ratchet only allows the count to decrease. New tests must use
event-based waits (Peer.nextEvent / Peer.waitUntilHeadsEqual /
data-state lifecycle / page.clock) instead of waitForTimeout.

See docs/specs/2026-04-27-event-based-waits-design.md §Implementation
phasing for the migration patterns. Tracking: #458.
EOF
    exit 1
fi

if (( current < baseline )); then
    cat <<EOF

note: waitForTimeout count is below baseline ($current < $baseline).
Update e2e/.wait-timeout-baseline to $current in this PR so the
ratchet locks in the improvement. (One-line file: \`echo $current >
e2e/.wait-timeout-baseline\`.)

EOF
fi

# Sunset enforcement.
today=$(date -u +%Y-%m-%d)
if [[ "$today" > "$SUNSET_DATE" || "$today" == "$SUNSET_DATE" ]]; then
    leftover=$(grep -lE "eslint-disable.*no-restricted-syntax" e2e/*.spec.ts 2>/dev/null || true)
    if [[ -n "$leftover" ]]; then
        cat >&2 <<EOF
error: sunset cutoff $SUNSET_DATE has passed; the following specs still
carry an eslint-disable header for no-restricted-syntax:

$leftover

Per docs/specs/2026-04-27-event-based-waits-design.md §Sunset, every
spec must have migrated to event-based waits by this date. Migrate
the remaining specs (tracking: #458) and remove the eslint-disable
headers, then re-run.
EOF
        exit 1
    fi
fi

echo "ratchet ok"
```

- [ ] **Step 2: Make the script executable + verify**

```bash
chmod +x scripts/check-wait-timeout-count.sh
./scripts/check-wait-timeout-count.sh
```

Expected output (today, before sunset):
```
waitForTimeout count: current=54 baseline=54
ratchet ok
```

- [ ] **Step 3: Self-test the failure path**

```bash
# Temporarily lower the baseline by 1 to simulate a regression — must fail.
echo "53" > e2e/.wait-timeout-baseline
if ./scripts/check-wait-timeout-count.sh 2>&1; then
    echo "BUG: script should have failed (current=54, baseline=53)"
    exit 1
else
    echo "self-test ok: regression detected as expected"
fi
# Restore.
echo "54" > e2e/.wait-timeout-baseline
```

Expected: the script exits non-zero with the "regressed" error block, and the self-test echoes "self-test ok".

- [ ] **Step 4: Self-test the below-baseline path**

```bash
# Temporarily raise the baseline to simulate "we got better, please bump the file".
echo "55" > e2e/.wait-timeout-baseline
./scripts/check-wait-timeout-count.sh
# Restore.
echo "54" > e2e/.wait-timeout-baseline
```

Expected: exit 0 with the "below baseline" note printed.

- [ ] **Step 5: Commit**

```bash
git add scripts/check-wait-timeout-count.sh
git commit -m "test(e2e): add wait-timeout ratchet enforcer script

Reads e2e/.wait-timeout-baseline, recounts waitForTimeout in e2e/*.ts,
and fails if the count exceeds the baseline. Below-baseline counts
pass with a hint to update the baseline file in the same PR so future
runs lock in the improvement.

Also enforces the spec's 2026-09-30 sunset cutoff: after that date,
any spec under e2e/*.spec.ts that still has an eslint-disable header
for no-restricted-syntax fails the gate. This is the forcing function
for completing the file-by-file migration tracked at #458.

Wired into 'just check-all' in the next commit."
```

---

## Task 3: Add the flake harness recipe to `justfile`

**Files:**
- Modify: `justfile`

A new recipe `test-e2e-flake N=5` runs `npx playwright test` N times and aggregates pass/fail. The default N=5 matches the spec; the recipe accepts an override (e.g. `just test-e2e-flake N=10`).

- [ ] **Step 1: Add the recipe**

Open `justfile` and insert this recipe after the existing `test-e2e-ui` recipe (around line 95):

```just
# Run the Playwright E2E suite N times to surface intermittent flake.
# Aggregates pass/fail per run; exits non-zero if ANY run failed.
# Default N=5 (per docs/specs/2026-04-27-event-based-waits-design.md
# §Implementation phasing PR 4).
test-e2e-flake N="5" FEATURES="test-hooks":
    #!/usr/bin/env bash
    set -uo pipefail
    @just setup-e2e FEATURES={{FEATURES}}
    pass=0
    fail=0
    failures=()
    for i in $(seq 1 {{N}}); do
        echo
        echo "════════ flake harness: run $i / {{N}} ════════"
        if npx playwright test --reporter=line; then
            pass=$((pass + 1))
        else
            fail=$((fail + 1))
            failures+=("$i")
        fi
    done
    echo
    echo "════════ flake summary ════════"
    echo "passed: $pass / {{N}}"
    echo "failed: $fail / {{N}}"
    if (( fail > 0 )); then
        echo "failed runs: ${failures[*]}"
        exit 1
    fi
```

- [ ] **Step 2: Verify `just` parses the recipe**

```bash
just --list 2>&1 | grep -i flake
```

Expected: `test-e2e-flake N FEATURES` listed.

- [ ] **Step 3: Skip running it locally** — `just test-e2e-flake` requires a running dev stack and takes 5× the time of a single suite run. CI will exercise it on the next gate-all run if the user adds it to `check-all`. PR-4 doesn't add it to `check-all` by default — flake-harness runs are opt-in (heavy + non-deterministic), per spec §"PR 4 ... informational, not blockers".

- [ ] **Step 4: Commit**

```bash
git add justfile
git commit -m "test(e2e): add test-e2e-flake N=5 harness recipe

Runs 'npx playwright test' N times in a loop and aggregates pass/fail
per run. Default N=5 per docs/specs/2026-04-27-event-based-waits-design.md
§Implementation phasing PR 4.

Not wired into check-all (heavy + non-deterministic per spec). Opt-in:
  just test-e2e-flake             # 5 runs
  just test-e2e-flake N=10        # 10 runs

Use this when investigating intermittent failures or before merging
risky changes to the e2e suite. The wait-timeout ratchet (also added
in PR-4) gates compile-time regressions; this recipe surfaces
runtime-only flakes that the ratchet can't see."
```

---

## Task 4: Wire the ratchet into `just check-all`

**Files:**
- Modify: `justfile`

Add the script invocation to `check-all` so any PR that introduces a new `waitForTimeout` fails the gate. Place it AFTER `test-e2e-ui` so failures surface in the order: lint → Rust → wasm → e2e → ratchet (ratchet is fast; ordering doesn't matter for correctness, but later steps imply earlier ones already passed).

- [ ] **Step 1: Edit `check-all`**

Find this block:

```just
check-all FEATURES="test-hooks":
    #!/usr/bin/env bash
    set -euo pipefail
    just fmt
    just clippy
    just test
    just test-browser
    just test-e2e-ui FEATURES={{FEATURES}}
    ./scripts/check-no-test-hooks-in-prod.sh
```

And append one line:

```just
    ./scripts/check-wait-timeout-count.sh
```

So the block becomes:

```just
check-all FEATURES="test-hooks":
    #!/usr/bin/env bash
    set -euo pipefail
    just fmt
    just clippy
    just test
    just test-browser
    just test-e2e-ui FEATURES={{FEATURES}}
    ./scripts/check-no-test-hooks-in-prod.sh
    ./scripts/check-wait-timeout-count.sh
```

- [ ] **Step 2: Verify**

```bash
./scripts/check-wait-timeout-count.sh
```

Expected: `ratchet ok`. (Running the full `just check-all` is heavy; this single invocation proves the wiring is reachable from PATH.)

- [ ] **Step 3: Commit**

```bash
git add justfile
git commit -m "test(e2e): wire wait-timeout ratchet into just check-all

Failures here block PR merge — same precedence as fmt/clippy/test/wasm.
The ratchet is fast (single grep + integer compare) so it adds
negligible time to the gate.

Per docs/specs/2026-04-27-event-based-waits-design.md §CI gate."
```

---

## Task 5: Document the ratchet in `e2e/README.md`

**Files:**
- Modify: `e2e/README.md`

Append a short section after the existing "Anti-patterns blocked by ESLint" section (added in PR-2) so a contributor who hits the ratchet failure has somewhere to read about how to fix it.

- [ ] **Step 1: Append**

Open `e2e/README.md` and add at the end:

```markdown

## Wait-timeout ratchet

`scripts/check-wait-timeout-count.sh` enforces a monotone-decreasing
count of `page.waitForTimeout` calls in `e2e/*.ts`. The current
allowed count lives in `e2e/.wait-timeout-baseline`.

- **If you add a new `waitForTimeout`**, the ratchet fails. Migrate
  the new test to event-based waits instead — see "Event-based waits
  (Peer wrapper)" above for the pattern.
- **If you remove a `waitForTimeout`**, decrement the baseline file
  in the same PR. The ratchet's "below baseline" message tells you
  the new value to write.
- The ratchet enforces a sunset cutoff: after **2026-09-30** any spec
  that still carries an `eslint-disable.*no-restricted-syntax` header
  fails the gate. By then the file-by-file migration tracked at
  [#458](https://github.com/intendednull/willow/issues/458) must have
  removed every header.

## Flake harness

```bash
just test-e2e-flake             # 5 runs (default)
just test-e2e-flake N=10        # 10 runs
```

Runs the full Playwright suite N times in sequence and reports pass/fail
per run. Use when investigating an intermittent failure or before
merging risky changes to the e2e suite. Not wired into `just
check-all` — heavy + non-deterministic.
```

- [ ] **Step 2: Commit**

```bash
git add e2e/README.md
git commit -m "docs(e2e): document wait-timeout ratchet + flake harness

Two new sections appended to e2e/README.md:
- 'Wait-timeout ratchet' explains the script, the baseline file, and
  how to update it on improve / fail on regress, plus the 2026-09-30
  sunset cutoff.
- 'Flake harness' documents the just test-e2e-flake recipe and when
  to use it.

Per docs/specs/2026-04-27-event-based-waits-design.md §Implementation
phasing PR 4."
```

---

## Final acceptance

```bash
# The cheap subset (heavy ones run in CI):
./scripts/check-wait-timeout-count.sh   # ratchet ok
just --list | grep -E "flake|check-all" # recipes present
```

Expected:
- ratchet exits 0 with `ratchet ok`
- `just --list` shows `test-e2e-flake` and the modified `check-all`

CI runs the full `just check-all FEATURES=test-hooks` on the PR.

---

## Out of scope (deferred)

- Migration of any of the 5 specs that still carry `eslint-disable` headers (cross-browser-sync, multi-peer-mobile, mobile-actions, permissions, mobile). Each is a one-file change tracked at #458; doing one or more here would inflate PR-4 and lose the property "PR-4 has zero behavioural risk".
- Adding the flake harness to `check-all` — explicitly opt-in per spec.
- A separate ratchet for `{ timeout: 30_000 }` overrides — the spec mentions it as a runner-up but the ESLint rule already blocks new `waitForTimeout`, and the 30_000 overrides are a separate naming convention. Defer until a clear regression case appears.

---

## Cross-references

- Spec: [`docs/specs/2026-04-27-event-based-waits-design.md`](../specs/2026-04-27-event-based-waits-design.md) §"PR 4" + §"CI gate" + §"Sunset".
- Predecessors: PR-1 #454, PR-2 #495, PR-3 #496.
- Tracking issue: [#458](https://github.com/intendednull/willow/issues/458).
