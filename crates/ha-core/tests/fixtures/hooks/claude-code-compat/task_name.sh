#!/usr/bin/env bash
# Official-style Claude Code TaskCreated / TaskCompleted hook: mirror the task
# list into an external tracker.
#
# Both official payloads name the task text `task_name`; Hope Agent's variant
# field is internally `content` on BOTH variants, so a single serde rename per
# variant is all that keeps an unmodified official script working — and one
# script must therefore work for both events unchanged. The event name is echoed
# alongside so the Rust side can tell the two dispatches apart from one fixture.
set -euo pipefail
input=$(cat)
task_name=$(printf '%s' "$input" | jq -r '.task_name // empty')
event=$(printf '%s' "$input" | jq -r '.hook_event_name // empty')
[ -n "$task_name" ] || { echo "${event:-Task} payload has no .task_name key" >&2; exit 1; }
# Emit with `jq -nc --arg` rather than a heredoc: payload values are
# arbitrary text (quotes, newlines, backslashes), and splicing them into
# a heredoc yields invalid JSON that the host silently treats as "the hook
# contributed nothing" — indistinguishable from a missing field.
jq -nc \
  --arg ev "$event" \
  --arg ctx "task_event=${event}; task_name=${task_name}" \
  '{hookSpecificOutput:{hookEventName:$ev,additionalContext:$ctx}}'
exit 0
