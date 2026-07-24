#!/usr/bin/env bash
# Official-style Claude Code PreCompact hook: observe an upcoming compaction.
#
# The official PreCompact payload carries the user's configured
# `custom_instructions` (the free-text guidance the summarizer should honor)
# alongside the `trigger` (`manual` / `auto` / `tool_loop`). Official scripts
# read both to decide whether to veto or annotate the compaction; an omitted or
# renamed `custom_instructions` would silently drop the user's guidance. Both
# keys are echoed back so the Rust side asserts they arrived verbatim.
set -euo pipefail
input=$(cat)
trigger=$(printf '%s' "$input" | jq -r '.trigger // empty')
custom=$(printf '%s' "$input" | jq -r '.custom_instructions // empty')
[ -n "$trigger" ] || { echo "PreCompact payload has no .trigger key" >&2; exit 1; }
[ -n "$custom" ] || { echo "PreCompact payload has no .custom_instructions key" >&2; exit 1; }
cat <<JSON
{"hookSpecificOutput":{"hookEventName":"PreCompact","additionalContext":"compact_trigger=${trigger}; compact_instructions=${custom}"}}
JSON
exit 0
