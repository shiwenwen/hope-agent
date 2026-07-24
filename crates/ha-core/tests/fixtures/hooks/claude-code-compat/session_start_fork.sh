#!/usr/bin/env bash
# Official-style Claude Code SessionStart hook: seed context per start source.
#
# The official payload names the start cause `source` (`startup` / `resume` /
# `clear` / `compact` / `fork`) and, when one is already set, the current
# `session_title`. Official scripts branch on `.source` to decide what context
# to inject, and `fork` is the newest member of that set — a variant that failed
# to serialize as the lowercase official token would drop such a script into its
# default branch. Both keys are echoed back for assertion.
set -euo pipefail
input=$(cat)
source_kind=$(printf '%s' "$input" | jq -r '.source // empty')
session_title=$(printf '%s' "$input" | jq -r '.session_title // empty')
[ -n "$source_kind" ] || { echo "SessionStart payload has no .source key" >&2; exit 1; }
[ -n "$session_title" ] || { echo "SessionStart payload has no .session_title key" >&2; exit 1; }
cat <<JSON
{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"src=${source_kind}; session_title=${session_title}"}}
JSON
exit 0
