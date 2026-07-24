#!/usr/bin/env bash
# Official-style Claude Code PermissionRequest / PermissionDenied hook: audit
# what the approval chain was asked to gate.
#
# The official approval payloads carry `tool_name` plus the full `tool_input`
# object — the same shape PreToolUse uses — so ONE official policy script can
# gate `.tool_input.command` across the pre-tool and approval events unchanged.
# Hope Agent additionally emits a flat `command` extension; this fixture reads
# only the official pair so a regression that kept `command` alive while
# dropping `tool_input` still fails. The event name is echoed to tell the two
# dispatches apart from one fixture.
set -euo pipefail
input=$(cat)
tool_name=$(printf '%s' "$input" | jq -r '.tool_name // empty')
tool_command=$(printf '%s' "$input" | jq -r '.tool_input.command // empty')
event=$(printf '%s' "$input" | jq -r '.hook_event_name // empty')
[ -n "$tool_name" ] || { echo "${event:-Permission} payload has no .tool_name key" >&2; exit 1; }
[ -n "$tool_command" ] || { echo "${event:-Permission} payload has no .tool_input.command key" >&2; exit 1; }
cat <<JSON
{"hookSpecificOutput":{"hookEventName":"${event}","additionalContext":"perm_event=${event}; perm_tool=${tool_name}; perm_command=${tool_command}"}}
JSON
exit 0
