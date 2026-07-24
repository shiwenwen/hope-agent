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
# Presence checks use `has(...)`, not `// empty`: `tool_response` is legitimately
# `null` for a failed call (see the header), and jq's `//` falls through on
# `null` — so `// empty` would report a bogus "key missing" for a perfectly
# well-formed round whose first call failed. For the same reason the array-level
# check tests `has("tool_calls")` rather than the length: `null | length` is `0`,
# a non-empty string, so a `[ -n "$count" ]` guard can never fire.
has_calls=$(printf '%s' "$input" | jq -r 'has("tool_calls")')
[ "$has_calls" = "true" ] || { echo "PostToolBatch payload has no .tool_calls key" >&2; exit 1; }
count=$(printf '%s' "$input" | jq -r '.tool_calls | length')
has_first_response=$(printf '%s' "$input" | jq -r '.tool_calls[0] | has("tool_response")')
[ "$has_first_response" = "true" ] || { echo "PostToolBatch payload has no .tool_calls[0].tool_response key" >&2; exit 1; }
first_tool=$(printf '%s' "$input" | jq -r '.tool_calls[0].tool_name // empty')
[ -n "$first_tool" ] || { echo "PostToolBatch payload has no .tool_calls[0].tool_name key" >&2; exit 1; }
# `tostring` so a null response renders as the literal `null` instead of an
# empty string — the compat assertion can then tell "failed call" from "absent".
first_response=$(printf '%s' "$input" | jq -r '.tool_calls[0].tool_response | tostring')
# Emit with `jq -nc --arg` rather than a heredoc: payload values are
# arbitrary text (quotes, newlines, backslashes), and splicing them into
# a heredoc yields invalid JSON that the host silently treats as "the hook
# contributed nothing" — indistinguishable from a missing field.
jq -nc \
  --arg ev "PostToolBatch" \
  --arg ctx "batch_calls=${count}; batch_first_tool=${first_tool}; batch_first_response=${first_response}" \
  '{hookSpecificOutput:{hookEventName:$ev,additionalContext:$ctx}}'
exit 0
