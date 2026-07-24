#!/usr/bin/env bash
# Official-style Claude Code PreToolUse hook that ONLY injects context: it
# returns `additionalContext` and no decision at all — the overwhelmingly common
# shape for advisory/lint hooks. Such a hook must NOT be read as an approval:
# the outcome's explicit-allow flag has to stay false, otherwise every advisory
# hook in the wild would silently auto-approve tool calls. Echoes the observed
# `.tool_input.command` back inside the context so the payload round-trip is
# provable from the Rust side; a renamed field fails loudly instead of echoing
# an empty string.
set -euo pipefail
input=$(cat)
command=$(printf '%s' "$input" | jq -r '.tool_input.command // empty')
[ -n "$command" ] || {
  echo "PreToolUse payload has no .tool_input.command key" >&2
  exit 1
}
jq -nc --arg cmd "$command" \
  '{hookSpecificOutput:{hookEventName:"PreToolUse",additionalContext:("[advisory] observed command: " + $cmd)}}'
exit 0
