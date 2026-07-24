#!/usr/bin/env bash
# Official-style Claude Code PermissionRequest hook: auto-DENY an approval
# before any prompt is shown, using the official STRUCTURED decision object
# `hookSpecificOutput.decision.behavior` — a different surface from the
# PreToolUse-only `permissionDecision` string, and the only one PermissionRequest
# honors. The human-readable justification rides on the top-level `reason`, per
# the official schema. Reads `.command` first so a renamed payload field fails
# loudly instead of denying blindly. Running this UNMODIFIED proves Hope Agent
# parses the nested behavior object into a hard Deny (G1).
set -euo pipefail
input=$(cat)
command=$(printf '%s' "$input" | jq -r '.command // empty')
[ -n "$command" ] || {
  echo "PermissionRequest payload has no .command key" >&2
  exit 1
}
cat <<'JSON'
{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny"}},"reason":"policy: no prod writes"}
JSON
exit 0
