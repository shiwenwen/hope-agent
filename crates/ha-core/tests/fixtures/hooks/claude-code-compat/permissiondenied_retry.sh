#!/usr/bin/env bash
# Official-style Claude Code PermissionDenied hook: after the user (or policy)
# refused a tool call, ask the model to try again by returning the official
# `hookSpecificOutput.retry` flag. Reads `.reason` (`user_declined` / `policy`)
# with `jq` to prove the denial payload arrived under its official field name,
# then emits the retry object. Running this UNMODIFIED proves Hope Agent parses
# `retry` onto the outcome (it was a dead hard-coded `false` until recently).
set -euo pipefail
input=$(cat)
reason=$(printf '%s' "$input" | jq -r '.reason // empty')
[ -n "$reason" ] || {
  echo "PermissionDenied payload has no .reason key" >&2
  exit 1
}
cat <<'JSON'
{"hookSpecificOutput":{"hookEventName":"PermissionDenied","retry":true}}
JSON
exit 0
