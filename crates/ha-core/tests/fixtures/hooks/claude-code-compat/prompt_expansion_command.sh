#!/usr/bin/env bash
# Official-style Claude Code UserPromptExpansion hook: augment a slash command.
#
# The official payload names the slash command `command_name` and the full raw
# text (command + args) `raw_input`. Hope Agent's variant fields are internally
# `command` / `command_text`, so only the serde renames keep an unmodified
# official expansion script working — it matches on `command_name` and re-parses
# `raw_input` for arguments. Both are echoed back as additionalContext so the
# Rust side asserts the exact values landed under the official keys.
set -euo pipefail
input=$(cat)
command_name=$(printf '%s' "$input" | jq -r '.command_name // empty')
raw_input=$(printf '%s' "$input" | jq -r '.raw_input // empty')
[ -n "$command_name" ] || { echo "UserPromptExpansion payload has no .command_name key" >&2; exit 1; }
[ -n "$raw_input" ] || { echo "UserPromptExpansion payload has no .raw_input key" >&2; exit 1; }
# Emit with `jq -nc --arg` rather than a heredoc: payload values are
# arbitrary text (quotes, newlines, backslashes), and splicing them into
# a heredoc yields invalid JSON that the host silently treats as "the hook
# contributed nothing" — indistinguishable from a missing field.
jq -nc \
  --arg ev "UserPromptExpansion" \
  --arg ctx "expansion_command=${command_name}; expansion_raw=${raw_input}" \
  '{hookSpecificOutput:{hookEventName:$ev,additionalContext:$ctx}}'
exit 0
