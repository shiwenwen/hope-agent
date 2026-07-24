#!/usr/bin/env bash
# Official-style Claude Code SessionEnd hook: log why the session ended.
#
# The official SessionEnd payload carries the end cause under `.reason`
# (`clear` / `resume` / `logout` / `prompt_input_exit` / …) — NOT under the
# Hope-Agent-internal field name `source` the variant is declared with. An
# unmodified official script therefore reads `.reason`, and a rename regression
# would leave it empty. `.source` is read back too: if it ever reappears the
# echoed context carries a `stale_source=` marker the Rust side asserts against,
# so a duplicate/legacy key fails loudly instead of passing silently.
set -euo pipefail
input=$(cat)
reason=$(printf '%s' "$input" | jq -r '.reason // empty')
stale=$(printf '%s' "$input" | jq -r '.source // empty')
[ -n "$reason" ] || { echo "SessionEnd payload has no .reason key" >&2; exit 1; }
ctx="session_end_reason=${reason}"
[ -z "$stale" ] || ctx="${ctx}; stale_source=${stale}"
cat <<JSON
{"hookSpecificOutput":{"hookEventName":"SessionEnd","additionalContext":"${ctx}"}}
JSON
exit 0
