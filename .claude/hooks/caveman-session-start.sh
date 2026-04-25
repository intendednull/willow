#!/usr/bin/env bash
# SessionStart hook — nudges Claude to invoke the caveman skill at the start
# of every new/cleared/compacted session.
#
# The nudge is self-suppressing: if `caveman` is not in the available-skills
# list (no plugin, no vendored copy), Claude is told to ignore the
# instruction. If it is in the list, the source (plugin vs vendored) doesn't
# matter — invoking the skill is idempotent.

set -euo pipefail

PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(pwd)}"
SKILL_DIR="${PROJECT_DIR}/.claude/skills/caveman"

if [ ! -d "$SKILL_DIR" ]; then
    exit 0
fi

if [ "${WILLOW_SKIP_VENDORED_SKILLS:-}" = "1" ]; then
    exit 0
fi

jq -n '{
  hookSpecificOutput: {
    hookEventName: "SessionStart",
    additionalContext: "If a `caveman` skill appears in your available-skills list, invoke it before any other action in this session. If no such skill is available, ignore this instruction."
  }
}'
