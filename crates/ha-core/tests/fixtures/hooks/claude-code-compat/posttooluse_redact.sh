#!/usr/bin/env bash
# Official-style Claude Code PostToolUse redaction hook: scrub a leaked API key
# out of a tool result before it re-enters the conversation. Reads the official
# `.tool_response` field with `jq`, rewrites any `sk-…` token to REDACTED, and
# returns the official `hookSpecificOutput.updatedToolOutput` — the documented
# way a hook REPLACES a tool's output. This is the canonical shape a shipped
# secret-scrubber hook has; running it UNMODIFIED proves Hope Agent actually
# parses `updatedToolOutput` (it was a dead hard-coded `None` until recently)
# and that `.tool_response` is delivered under its official name (G1).
set -euo pipefail
input=$(cat)
response=$(printf '%s' "$input" | jq -r '.tool_response // empty')
[ -n "$response" ] || {
  echo "PostToolUse payload has no .tool_response key" >&2
  exit 1
}
redacted=$(printf '%s' "$response" | sed -E 's/sk-[A-Za-z0-9_-]+/REDACTED/g')
# `jq -nc --arg` builds the object so the redacted text is quoted safely.
jq -nc --arg out "$redacted" \
  '{hookSpecificOutput:{hookEventName:"PostToolUse",updatedToolOutput:$out}}'
exit 0
