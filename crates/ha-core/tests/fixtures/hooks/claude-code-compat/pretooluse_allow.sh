#!/usr/bin/env bash
# Official-style Claude Code PreToolUse hook returning `permissionDecision:
# "allow"` — the deliberate auto-approve that lets a trusted command skip the
# user prompt. Hope Agent must distinguish this from the DEFAULT allow a
# context-only hook produces, so the explicit-allow flag on the outcome is the
# thing under test (its sibling fixture `pretooluse_context_only.sh` pins the
# negative side). Reads `.tool_input.command` so a renamed field fails loudly.
set -euo pipefail
input=$(cat)
command=$(printf '%s' "$input" | jq -r '.tool_input.command // empty')
[ -n "$command" ] || {
  echo "PreToolUse payload has no .tool_input.command key" >&2
  exit 1
}
cat <<'JSON'
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow"}}
JSON
exit 0
