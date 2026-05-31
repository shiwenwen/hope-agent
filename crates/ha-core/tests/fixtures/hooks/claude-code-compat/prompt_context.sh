#!/usr/bin/env bash
# Official-style Claude Code UserPromptSubmit hook: inject additionalContext
# that the model sees alongside the user's message. Reads `.prompt` from the
# payload to prove that field is delivered, then returns the official
# UserPromptSubmit output schema with `additionalContext`. Hope Agent must fold
# that into the turn's system prompt (G1).
set -euo pipefail
input=$(cat)
prompt=$(printf '%s' "$input" | jq -r '.prompt // empty')
chars=${#prompt}
cat <<JSON
{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"[house-rules] prompt_len=${chars}; follow the project style guide."}}
JSON
exit 0
