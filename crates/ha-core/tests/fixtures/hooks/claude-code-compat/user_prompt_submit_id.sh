#!/usr/bin/env bash
# Official-style UserPromptSubmit hook that keys per-turn state off `prompt_id`.
#
# `prompt_id` is the id that ties every hook of one user turn together
# (UserPromptSubmit → PreToolUse → PostToolUse → Stop), which is exactly how an
# official script correlates them. UserPromptSubmit is the hard case: it fires
# from the pre-persist preflight, and on some entry points (IM, ACP) that is
# before — or entirely without — the turn being registered, so the id cannot
# come from a registry lookup. It must be handed in by the entry point. This
# fixture fails LOUDLY when the key is absent rather than echoing an empty
# string, so a regression to "no id here" cannot pass silently.
set -euo pipefail
input=$(cat)
prompt_id=$(printf '%s' "$input" | jq -r '.prompt_id // empty')
prompt=$(printf '%s' "$input" | jq -r '.prompt // empty')
[ -n "$prompt_id" ] || { echo "UserPromptSubmit payload has no .prompt_id key" >&2; exit 1; }
[ -n "$prompt" ] || { echo "UserPromptSubmit payload has no .prompt key" >&2; exit 1; }
# Emit with `jq -nc --arg` rather than a heredoc: payload values are
# arbitrary text (quotes, newlines, backslashes), and splicing them into
# a heredoc yields invalid JSON that the host silently treats as "the hook
# contributed nothing" — indistinguishable from a missing field.
jq -nc \
  --arg ev "UserPromptSubmit" \
  --arg ctx "prompt_id=${prompt_id}; prompt=${prompt}" \
  '{hookSpecificOutput:{hookEventName:$ev,additionalContext:$ctx}}'
exit 0
