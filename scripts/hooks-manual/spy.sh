#!/usr/bin/env bash
# Hooks 手动 e2e 探针:把每次 hook 触发收到的 input(stdin JSON)摘要落盘,
# 用来观察哪些事件真的触发、payload 形状、以及 env 注入是否生效。
#
# exit 0 = 放行(观察用,不影响主流程)。
# 想验证阻断型事件(PreToolUse / UserPromptSubmit / PreCompact)能真拦住,
# 把下面的 exit 0 改成 exit 2,stderr 会作为 Block 原因。
#
# 日志路径可用 HA_HOOK_SPY_LOG 覆盖,默认 /tmp/ha-hook-spy.log。
set -uo pipefail

LOG="${HA_HOOK_SPY_LOG:-/tmp/ha-hook-spy.log}"
input=$(cat)
ts=$(date +%H:%M:%S)

if command -v jq >/dev/null 2>&1; then
  summary=$(printf '%s' "$input" | jq -c \
    '{event:.hook_event_name, tool:.tool_name, cwd:.cwd, sid:(.session_id[0:8])}' 2>/dev/null)
else
  summary=$(printf '%s' "$input" | tr -d '\n' | head -c 200)
fi

printf '%s | %s | CLAUDE_PROJECT_DIR=%s | HOPE_PROJECT_DIR=%s\n' \
  "$ts" "${summary:-<unparsable>}" "${CLAUDE_PROJECT_DIR:-<unset>}" "${HOPE_PROJECT_DIR:-<unset>}" \
  >> "$LOG"

exit 0
