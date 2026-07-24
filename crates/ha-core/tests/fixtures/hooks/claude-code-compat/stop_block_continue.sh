#!/usr/bin/env bash
# Official-style Claude Code Stop hook: "don't stop yet, keep working".
#
# Stop INVERTS the usual meaning of `decision:"block"` — for every other event
# block vetoes the action, but for Stop the action IS stopping, so block means
# "prevent stopping, run another turn" and `reason` is the feedback injected
# before that turn. This is the exact JSON an official continue-until-green
# hook emits (exit 0, body on stdout), so Hope Agent must surface it through
# `HookOutcome::stop_wants_continue()`. Do NOT "fix" this to a non-blocking
# decision — that would silently disable every official Stop hook.
# stdin is drained (not parsed) so the runner never sees SIGPIPE.
set -euo pipefail
cat >/dev/null
printf '%s' '{"decision":"block","reason":"tests still failing — keep going"}'
exit 0
