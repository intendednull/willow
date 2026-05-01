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
#
# `|| true` on the grep handles the success path under the 2026-09-30
# sunset: once every spec migrates and zero matches remain, grep
# returns 1, pipefail propagates, and `set -e` would silently abort
# the script with an empty `current=`. We want this to evaluate to
# `current=0` so the ratchet correctly reports `ratchet ok` on a
# fully-migrated tree.
current=$( { grep -roh "waitForTimeout" e2e/ --include='*.ts' 2>/dev/null || true; } | wc -l | tr -d ' ')

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
