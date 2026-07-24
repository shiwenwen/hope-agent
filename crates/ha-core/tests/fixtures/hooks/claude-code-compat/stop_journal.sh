#!/usr/bin/env bash
# Official-style Claude Code Stop hook ("don't stop yet, keep working") that
# ALSO journals what the engine actually sent it.
#
# Stop INVERTS `block`: the action being vetoed IS stopping, so exit 2 + a
# stderr reason means "prevent stopping, run another turn". This script blocks
# UNCONDITIONALLY, so the only thing that can decide whether a turn is actually
# re-driven is the engine's own `status == "completed"` gate — which is exactly
# what the integration test measures.
#
# `$1` is a journal file: every invocation appends its raw stdin payload as one
# compact JSON line, so the test can read the REAL `status` / `stop_hook_active`
# values back out instead of inferring them from behaviour. Appended BEFORE
# validation so a drifted payload leaves evidence rather than vanishing.
# stdin is drained first so the runner never sees SIGPIPE.
set -euo pipefail
log="$1"
input=$(cat)
printf '%s\n' "$input" >>"$log"
status=$(printf '%s' "$input" | jq -r '.status // empty')
[ -n "$status" ] || {
  echo "Stop payload has no .status key" >&2
  exit 1
}
# `.stop_hook_active` is a bool, so `// empty` can't distinguish `false` from a
# missing key — `has()` can.
active=$(printf '%s' "$input" | jq -r 'if has("stop_hook_active") then .stop_hook_active else empty end')
[ -n "$active" ] || {
  echo "Stop payload has no .stop_hook_active key" >&2
  exit 1
}
echo "keep going (status=$status stop_hook_active=$active)" >&2
exit 2
