#!/usr/bin/env bash
# Official-style Claude Code FileChanged hook: react to a watched-file edit.
#
# The official FileChanged payload names the changed file `file_path` (Hope
# Agent's variant field is internally `path`) plus an `action` verb
# (`create`/`edit`/`delete`/`patch`). Official scripts scope themselves with a
# regex matcher such as `.*\.rs$` — matchers are UNANCHORED per the protocol, so
# the regex is tested against the whole path rather than requiring a full match.
# Echoing both keys back proves the payload names AND (via the matcher) that a
# non-matching path never reaches this script at all.
set -euo pipefail
input=$(cat)
file_path=$(printf '%s' "$input" | jq -r '.file_path // empty')
action=$(printf '%s' "$input" | jq -r '.action // empty')
[ -n "$file_path" ] || { echo "FileChanged payload has no .file_path key" >&2; exit 1; }
[ -n "$action" ] || { echo "FileChanged payload has no .action key" >&2; exit 1; }
# Emit with `jq -nc --arg` rather than a heredoc: payload values are
# arbitrary text (quotes, newlines, backslashes), and splicing them into
# a heredoc yields invalid JSON that the host silently treats as "the hook
# contributed nothing" — indistinguishable from a missing field.
jq -nc \
  --arg ev "FileChanged" \
  --arg ctx "changed_file=${file_path}; changed_action=${action}" \
  '{hookSpecificOutput:{hookEventName:$ev,additionalContext:$ctx}}'
exit 0
