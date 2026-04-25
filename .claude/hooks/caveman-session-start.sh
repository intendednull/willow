#!/usr/bin/env bash
# SessionStart hook — nudges Claude to invoke the caveman skill at the start
# of every new/cleared/compacted session.
#
# Skips emission if the caveman plugin is already enabled at the user level
# (so we don't double-prompt) or if WILLOW_SKIP_VENDORED_SKILLS=1.

set -euo pipefail

PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(pwd)}"
SKILL_DIR="${PROJECT_DIR}/.claude/skills/caveman"

if [ ! -d "$SKILL_DIR" ]; then
    exit 0
fi

if [ "${WILLOW_SKIP_VENDORED_SKILLS:-}" = "1" ]; then
    exit 0
fi

USER_SETTINGS="${HOME}/.claude/settings.json"
if [ -f "$USER_SETTINGS" ] && command -v jq >/dev/null 2>&1; then
    if jq -e '
        (.enabledPlugins // {})
        | to_entries
        | any(.[]; (.key | startswith("caveman@")) and .value == true)
    ' "$USER_SETTINGS" >/dev/null 2>&1; then
        exit 0
    fi
fi

jq -n '{
  hookSpecificOutput: {
    hookEventName: "SessionStart",
    additionalContext: "Invoke the caveman skill before any other action in this session."
  }
}'
