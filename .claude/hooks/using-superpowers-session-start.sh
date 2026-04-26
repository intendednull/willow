#!/usr/bin/env bash
# SessionStart hook — preloads the using-superpowers skill content into
# every new/cleared/compacted session so the meta-skill is always primed.
#
# Vendored from github.com/obra/superpowers (hooks/session-start), simplified
# for the Claude Code runtime only and pointed at the in-repo skill copy.
#
# The emitted block is self-suppressing: if `superpowers:using-superpowers`
# already appears in the available-skills list (because the user has the
# plugin enabled at the user/CLI level), Claude is instructed to treat the
# block as a no-op. This avoids brittle config-path detection and works
# across Claude Code, Copilot CLI, Gemini CLI, etc.

set -euo pipefail

PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(pwd)}"
SKILL_PATH="${PROJECT_DIR}/.claude/skills/using-superpowers/SKILL.md"

if [ ! -f "$SKILL_PATH" ]; then
    exit 0
fi

if [ "${WILLOW_SKIP_VENDORED_SKILLS:-}" = "1" ]; then
    exit 0
fi

skill_content="$(cat "$SKILL_PATH")"

preamble="If \`superpowers:using-superpowers\` already appears in your available-skills list, treat this entire <EXTREMELY_IMPORTANT> block as a no-op — the plugin version is authoritative. Otherwise, the block below is the vendored fallback copy of that skill; treat it as primed meta-skill content."

context="$(printf '%s\n\n<EXTREMELY_IMPORTANT>\nYou have superpowers.\n\n**Below is the full content of your '\''superpowers:using-superpowers'\'' skill - your introduction to using skills. For all other skills, use the '\''Skill'\'' tool:**\n\n%s\n</EXTREMELY_IMPORTANT>' "$preamble" "$skill_content")"

jq -n --arg ctx "$context" '{
  hookSpecificOutput: {
    hookEventName: "SessionStart",
    additionalContext: $ctx
  }
}'
