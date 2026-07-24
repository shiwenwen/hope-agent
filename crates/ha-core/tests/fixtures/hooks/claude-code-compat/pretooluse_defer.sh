#!/usr/bin/env bash
# Official-style Claude Code PreToolUse hook returning `permissionDecision:
# "defer"` — "I have no opinion, hand this back to the normal permission flow".
# The official protocol lists `allow | deny | ask | defer`; `defer` is the one
# value a host can silently swallow (falling through to Allow) without any
# visible symptom, which is exactly what makes an unmodified-script test worth
# having. Reads `.tool_input.command` so a renamed payload field fails loudly.
set -euo pipefail
input=$(cat)
command=$(printf '%s' "$input" | jq -r '.tool_input.command // empty')
[ -n "$command" ] || {
  echo "PreToolUse payload has no .tool_input.command key" >&2
  exit 1
}
cat <<'JSON'
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"defer"}}
JSON
exit 0
