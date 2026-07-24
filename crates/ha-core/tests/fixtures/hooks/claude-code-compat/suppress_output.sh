#!/usr/bin/env bash
# Official-style Claude Code hook exercising the two TOP-LEVEL output fields
# (siblings of `hookSpecificOutput`, not nested in it): `suppressOutput`, which
# asks the host to keep this hook's stdout out of the transcript, and
# `systemMessage`, the one-line note surfaced to the user. Reads `.tool_name`
# with `jq` so a renamed payload field fails loudly rather than reporting
# success on an empty payload. Running this UNMODIFIED proves both top-level
# keys survive parse → aggregate onto the outcome (G1).
set -euo pipefail
input=$(cat)
tool=$(printf '%s' "$input" | jq -r '.tool_name // empty')
[ -n "$tool" ] || {
  echo "PostToolUse payload has no .tool_name key" >&2
  exit 1
}
cat <<'JSON'
{"suppressOutput":true,"systemMessage":"hook ran quietly"}
JSON
exit 0
