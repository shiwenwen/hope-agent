#!/usr/bin/env bash
# Official-style Claude Code Stop hook: inspect the finished turn.
#
# The official Stop payload carries the turn's final assistant text as
# `last_assistant_message` plus the `stop_hook_active` guard flag — the flag a
# well-behaved official script checks FIRST so it never re-drives a turn a
# previous Stop hook already re-drove (the classic infinite-continue footgun).
# Both keys are echoed back so the Rust side asserts they arrived under the
# official names and with the values the dispatcher was handed.
set -euo pipefail
input=$(cat)
last_message=$(printf '%s' "$input" | jq -r '.last_assistant_message // empty')
stop_active=$(printf '%s' "$input" | jq -r 'if has("stop_hook_active") then (.stop_hook_active | tostring) else empty end')
[ -n "$last_message" ] || { echo "Stop payload has no .last_assistant_message key" >&2; exit 1; }
[ -n "$stop_active" ] || { echo "Stop payload has no .stop_hook_active key" >&2; exit 1; }
# Emit with `jq -nc --arg` rather than a heredoc: payload values are
# arbitrary text (quotes, newlines, backslashes), and splicing them into
# a heredoc yields invalid JSON that the host silently treats as "the hook
# contributed nothing" — indistinguishable from a missing field.
jq -nc \
  --arg ev "Stop" \
  --arg ctx "stop_last_message=${last_message}; stop_hook_active=${stop_active}" \
  '{hookSpecificOutput:{hookEventName:$ev,additionalContext:$ctx}}'
exit 0
