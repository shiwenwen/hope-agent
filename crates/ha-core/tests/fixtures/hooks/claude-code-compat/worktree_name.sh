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
# Emit with `jq -nc --arg` rather than a heredoc: payload values are
# arbitrary text (quotes, newlines, backslashes), and splicing them into
# a heredoc yields invalid JSON that the host silently treats as "the hook
# contributed nothing" — indistinguishable from a missing field.
jq -nc \
  --arg ev "WorktreeCreate" \
  --arg ctx "worktree_name=${worktree_name}" \
  --arg wp "/tmp/worktrees/${worktree_name}" \
  '{hookSpecificOutput:{hookEventName:$ev,additionalContext:$ctx,worktreePath:$wp}}'
exit 0
