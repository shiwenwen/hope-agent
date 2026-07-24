#!/usr/bin/env bash
# Official-style Claude Code StopFailure hook: classify a failed turn.
#
# The official StopFailure payload names the failure category `error_type`
# (`provider_failed` / `compaction_failed` / `shutdown` / …); Hope Agent's
# variant field is internally called `reason`, so only the serde rename keeps an
# unmodified official script working. The legacy `.reason` key is read back and
# echoed only if it still exists, letting the Rust side assert the context
# carries NO stale `reason=` echo — i.e. that the rename is complete, not
# additive (a duplicated key would let both spellings silently work).
set -euo pipefail
input=$(cat)
error_type=$(printf '%s' "$input" | jq -r '.error_type // empty')
stale=$(printf '%s' "$input" | jq -r '.reason // empty')
[ -n "$error_type" ] || { echo "StopFailure payload has no .error_type key" >&2; exit 1; }
ctx="stop_error_type=${error_type}"
[ -z "$stale" ] || ctx="${ctx}; reason=${stale}"
# Emit with `jq -nc --arg` rather than a heredoc: payload values are
# arbitrary text (quotes, newlines, backslashes), and splicing them into
# a heredoc yields invalid JSON that the host silently treats as "the hook
# contributed nothing" — indistinguishable from a missing field.
jq -nc \
  --arg ev "StopFailure" \
  --arg ctx "$ctx" \
  '{hookSpecificOutput:{hookEventName:$ev,additionalContext:$ctx}}'
exit 0
