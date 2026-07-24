#!/usr/bin/env bash
# Official-style Claude Code SubagentStart / SubagentStop hook: track sub-agent
# activity by type.
#
# The official payload names the sub-agent's type `agent_type` — the same key
# the flattened common block would use for the PARENT agent's type. Hope Agent
# resolves the collision by keeping the variant field and hard-coding
# `common.agent_type = None`, so exactly ONE `"agent_type"` key must appear on
# the wire. `jq` would silently keep only the last of two duplicate keys, hiding
# the defect — so the raw stdin text is grepped and the count echoed back too.
set -euo pipefail
input=$(cat)
agent_type=$(printf '%s' "$input" | jq -r '.agent_type // empty')
event=$(printf '%s' "$input" | jq -r '.hook_event_name // empty')
keys=$(printf '%s' "$input" | { grep -o '"agent_type"' || true; } | wc -l | tr -d '[:space:]')
[ -n "$agent_type" ] || { echo "${event:-Subagent} payload has no .agent_type key" >&2; exit 1; }
# Emit with `jq -nc --arg` rather than a heredoc: payload values are
# arbitrary text (quotes, newlines, backslashes), and splicing them into
# a heredoc yields invalid JSON that the host silently treats as "the hook
# contributed nothing" — indistinguishable from a missing field.
jq -nc \
  --arg ev "$event" \
  --arg ctx "subagent_event=${event}; subagent_agent_type=${agent_type}; keys=${keys}" \
  '{hookSpecificOutput:{hookEventName:$ev,additionalContext:$ctx}}'
exit 0
