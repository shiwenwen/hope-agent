#!/usr/bin/env bash
# Official-style Claude Code hook relying on the $CLAUDE_PROJECT_DIR environment
# contract, cross-checked against the payload `.cwd`. Official scripts use
# "$CLAUDE_PROJECT_DIR" to locate repo-relative helpers; this fixture proves
# Hope Agent injects it (dual with HOPE_PROJECT_DIR) AND that it equals the
# payload cwd (G7 + field alignment). additionalContext is emitted only when
# both agree; otherwise a non-zero exit surfaces the mismatch.
set -euo pipefail
input=$(cat)
cwd=$(printf '%s' "$input" | jq -r '.cwd // empty')
if [ -n "${CLAUDE_PROJECT_DIR:-}" ] && [ "$CLAUDE_PROJECT_DIR" = "$cwd" ]; then
  cat <<JSON
{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"project_dir_ok:${CLAUDE_PROJECT_DIR}"}}
JSON
  exit 0
fi
echo "CLAUDE_PROJECT_DIR ('${CLAUDE_PROJECT_DIR:-unset}') != payload cwd ('${cwd}')" >&2
exit 1
