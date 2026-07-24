#!/usr/bin/env bash
# Official-style Claude Code veto shorthand: the two-line "exit 2 = block, stderr
# = reason" contract, with no JSON at all. This is the shape most community
# hooks ship for a hard policy stop, so running it UNMODIFIED against the newly
# gate-capable events (TaskCreated / TaskCompleted / UserPromptExpansion /
# PermissionRequest / PostToolBatch) proves the exit-code protocol reaches those
# call sites — not just PreToolUse. It deliberately reads NO payload field:
# `parse.rs` discards stdout on exit 2, so a field echo-back is unobservable
# here; field-level alignment is covered by the payload-echo fixtures.
# stdin is drained (not parsed) so the runner never sees SIGPIPE.
set -euo pipefail
cat >/dev/null
echo "vetoed by policy" >&2
exit 2
