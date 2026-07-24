#!/usr/bin/env bash
# Official-style Claude Code hook reading the COMMON payload block: the
# per-turn correlation id `prompt_id` and the `effort` object (`{ "level": … }`).
#
# Official scripts key their per-turn state off `prompt_id` and branch on
# `effort.level`; the same level is ALSO published as the `$CLAUDE_EFFORT`
# environment variable, so the two must agree or a script that reads one and
# writes the other corrupts its own state. This fixture cross-checks payload
# against env in the same style as projectdir_env.sh's $CLAUDE_PROJECT_DIR
# check, emitting an `env_ok` marker only when they match.
set -euo pipefail
input=$(cat)
prompt_id=$(printf '%s' "$input" | jq -r '.prompt_id // empty')
effort=$(printf '%s' "$input" | jq -r '.effort.level // empty')
[ -n "$prompt_id" ] || { echo "PreToolUse payload has no .prompt_id key" >&2; exit 1; }
[ -n "$effort" ] || { echo "PreToolUse payload has no .effort.level key" >&2; exit 1; }
if [ "${CLAUDE_EFFORT:-}" = "$effort" ]; then
  env_marker="env_ok"
else
  env_marker="env_mismatch(CLAUDE_EFFORT='${CLAUDE_EFFORT:-unset}')"
fi
# Emit with `jq -nc --arg` rather than a heredoc: payload values are
# arbitrary text (quotes, newlines, backslashes), and splicing them into
# a heredoc yields invalid JSON that the host silently treats as "the hook
# contributed nothing" — indistinguishable from a missing field.
jq -nc \
  --arg ev "PreToolUse" \
  --arg ctx "prompt_id=${prompt_id}; effort=${effort}; ${env_marker}" \
  '{hookSpecificOutput:{hookEventName:$ev,additionalContext:$ctx}}'
exit 0
