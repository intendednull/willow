#!/usr/bin/env bash
# SessionStart hook — preloads the using-superpowers skill content into
# every new/cleared/compacted session so the meta-skill is always primed.
#
# Vendored from github.com/obra/superpowers (hooks/session-start), simplified
# for the Claude Code runtime only and pointed at the in-repo skill copy.

set -euo pipefail

PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(pwd)}"
SKILL_PATH="${PROJECT_DIR}/.claude/skills/using-superpowers/SKILL.md"

if [ ! -f "$SKILL_PATH" ]; then
    exit 0
fi

skill_content="$(cat "$SKILL_PATH")"

context="$(printf '<EXTREMELY_IMPORTANT>\nYou have superpowers.\n\n**Below is the full content of your '\''superpowers:using-superpowers'\'' skill - your introduction to using skills. For all other skills, use the '\''Skill'\'' tool:**\n\n%s\n</EXTREMELY_IMPORTANT>' "$skill_content")"

jq -n --arg ctx "$context" '{
  hookSpecificOutput: {
    hookEventName: "SessionStart",
    additionalContext: $ctx
  }
}'
