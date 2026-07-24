#!/usr/bin/env bash
# Official-style Claude Code PermissionRequest hook, allow side: the same
# structured `hookSpecificOutput.decision.behavior` object with `"allow"`.
# At the PARSE level this must set the outcome's explicit-allow flag (distinct
# from the default `Allow` a context-only hook yields). Note the approval gate
# deliberately consumes PermissionRequest hooks DENY-ONLY — see the Rust-side
# comment; that policy lives in `tools/approval.rs`, not in this protocol layer.
# Reads `.command` first so a renamed payload field fails loudly.
set -euo pipefail
input=$(cat)
command=$(printf '%s' "$input" | jq -r '.command // empty')
[ -n "$command" ] || {
  echo "PermissionRequest payload has no .command key" >&2
  exit 1
}
cat <<'JSON'
{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}
JSON
exit 0
