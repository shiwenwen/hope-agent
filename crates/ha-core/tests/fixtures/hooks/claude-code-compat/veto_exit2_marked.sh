#!/usr/bin/env bash
# Official-style Claude Code veto shorthand ("exit 2 = block, stderr = reason"),
# plus a run marker appended to $1.
#
# Same protocol shape as veto_exit2.sh, but it records that it EXECUTED. That is
# what a negative control needs: on an observation-only event the block is
# downgraded to Allow, and `parse.rs` discards stdout on exit 2, so "the
# downgrade worked" and "the hook never ran at all" are otherwise the same
# observation — a mis-wired install would pass the control silently.
# stdin is drained (not parsed) so the runner never sees SIGPIPE. No jq needed.
set -euo pipefail
cat >/dev/null
printf 'ran\n' >>"$1"
echo "vetoed by policy" >&2
exit 2
