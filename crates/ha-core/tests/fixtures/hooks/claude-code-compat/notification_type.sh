#!/usr/bin/env bash
# Official-style Claude Code Notification hook: route the notification onward.
#
# The official Notification payload names the notification kind `type` (a bare
# JSON key, not `notification_type`) alongside the human-readable `message`.
# Community scripts branch on `.type` to decide whether to page a desktop
# notifier, so a drift back to the internal field name would silently make every
# such script fall into its default branch. Both keys are echoed back as
# additionalContext so the Rust side can assert the exact values arrived.
set -euo pipefail
input=$(cat)
kind=$(printf '%s' "$input" | jq -r '.type // empty')
message=$(printf '%s' "$input" | jq -r '.message // empty')
[ -n "$kind" ] || { echo "Notification payload has no .type key" >&2; exit 1; }
[ -n "$message" ] || { echo "Notification payload has no .message key" >&2; exit 1; }
cat <<JSON
{"hookSpecificOutput":{"hookEventName":"Notification","additionalContext":"notification_type=${kind}; notification_message=${message}"}}
JSON
exit 0
