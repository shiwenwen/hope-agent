#!/usr/bin/env bash
# Official-style Claude Code PostToolBatch hook: audit one API round's tool
# calls.
#
# The official payload carries `tool_calls[]`, each entry a `{ tool_name,
# tool_input, tool_response }` summary (`tool_response` is null for a call that
# failed). Hope Agent additionally emits a flat `tool_names[]` extension, so a
# script reading only that would keep passing even if `tool_calls` regressed —
# this fixture deliberately reads ONLY the official array: its length, and the
# first entry's `tool_name` / `tool_response`.
set -euo pipefail
input=$(cat)
count=$(printf '%s' "$input" | jq -r '.tool_calls | length')
first_tool=$(printf '%s' "$input" | jq -r '.tool_calls[0].tool_name // empty')
first_response=$(printf '%s' "$input" | jq -r '.tool_calls[0].tool_response // empty')
[ -n "$count" ] || { echo "PostToolBatch payload has no .tool_calls key" >&2; exit 1; }
[ -n "$first_tool" ] || { echo "PostToolBatch payload has no .tool_calls[0].tool_name key" >&2; exit 1; }
[ -n "$first_response" ] || { echo "PostToolBatch payload has no .tool_calls[0].tool_response key" >&2; exit 1; }
cat <<JSON
{"hookSpecificOutput":{"hookEventName":"PostToolBatch","additionalContext":"batch_calls=${count}; batch_first_tool=${first_tool}; batch_first_response=${first_response}"}}
JSON
exit 0
