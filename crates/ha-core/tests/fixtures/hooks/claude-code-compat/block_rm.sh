#!/usr/bin/env bash
# Official-style Claude Code PreToolUse guard: block destructive `rm -rf`.
#
# Reads the hook input JSON from stdin (the Claude Code contract) and inspects
# `.tool_input.command` with `jq` exactly as the official Bash-validator example
# does. Exit 2 is the canonical "block this tool call" signal; stderr carries
# the reason shown to the model. This script is intentionally UNMODIFIED from
# the shape an official/community hook would ship — running it as-is against
# Hope Agent proves field-level payload alignment (goal G1).
set -euo pipefail
input=$(cat)
command=$(printf '%s' "$input" | jq -r '.tool_input.command // empty')
if printf '%s' "$command" | grep -qE 'rm[[:space:]]+-rf'; then
  echo "Blocked dangerous command: $command" >&2
  exit 2
fi
exit 0
