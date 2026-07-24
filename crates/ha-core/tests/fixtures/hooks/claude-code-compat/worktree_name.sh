#!/usr/bin/env bash
# Official-style Claude Code WorktreeCreate hook: choose where the worktree
# lands.
#
# The official payload names the generated worktree `worktree_name` (Hope
# Agent's variant field is internally `name`), and the official output schema
# lets the hook answer with `hookSpecificOutput.worktreePath` to override the
# checkout location. This fixture exercises BOTH directions of the contract in
# one run: it reads the official input key and writes the official output key.
set -euo pipefail
input=$(cat)
worktree_name=$(printf '%s' "$input" | jq -r '.worktree_name // empty')
[ -n "$worktree_name" ] || { echo "WorktreeCreate payload has no .worktree_name key" >&2; exit 1; }
cat <<JSON
{"hookSpecificOutput":{"hookEventName":"WorktreeCreate","additionalContext":"worktree_name=${worktree_name}","worktreePath":"/tmp/worktrees/${worktree_name}"}}
JSON
exit 0
