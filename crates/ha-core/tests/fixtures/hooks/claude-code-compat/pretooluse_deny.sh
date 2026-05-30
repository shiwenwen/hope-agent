#!/usr/bin/env bash
# Official-style Claude Code PreToolUse guard using the JSON decision protocol
# (exit 0 + `hookSpecificOutput.permissionDecision`), as opposed to the exit-2
# shorthand. Denies any command that writes under /etc. The emitted object is
# the official PreToolUse output schema verbatim — Hope Agent must parse
# `permissionDecision: "deny"` into a hard block (G1).
set -euo pipefail
input=$(cat)
command=$(printf '%s' "$input" | jq -r '.tool_input.command // empty')
if printf '%s' "$command" | grep -q '/etc/'; then
  cat <<'JSON'
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Writes under /etc are not allowed."}}
JSON
  exit 0
fi
echo '{}'
exit 0
