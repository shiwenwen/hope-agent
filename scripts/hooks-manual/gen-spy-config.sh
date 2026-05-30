#!/usr/bin/env bash
# 生成覆盖全部 24 个真触发事件的 spy hook 配置(JSON),每个事件的 command 都指向
# 本目录的 spy.sh(绝对路径,自动填好)。输出到 stdout。
#
# 用法:
#   # 1) 看一眼
#   scripts/hooks-manual/gen-spy-config.sh
#
#   # 2) 合并进 ~/.hope-agent/config.json 的 "hooks" 字段(需要 jq):
#   tmp=$(mktemp)
#   jq --argjson h "$(scripts/hooks-manual/gen-spy-config.sh)" '.hooks = $h' \
#     ~/.hope-agent/config.json > "$tmp" && mv "$tmp" ~/.hope-agent/config.json
#   # 改完重启 app,或在 Settings → Hooks 里随便存一次以触发热重载。
#
# 4 个保留事件(TeammateIdle / InstructionsLoaded / WorktreeCreate / WorktreeRemove)
# 当前永不触发,故意不在列表里。
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SPY="$DIR/spy.sh"

EVENTS=(
  SessionStart SessionEnd UserPromptSubmit UserPromptExpansion
  PreToolUse PostToolUse PostToolUseFailure PostToolBatch
  PermissionRequest PermissionDenied Stop StopFailure
  PreCompact PostCompact Notification SubagentStart SubagentStop
  TaskCreated TaskCompleted ConfigChange CwdChanged FileChanged
  Elicitation ElicitationResult
)

printf '{\n'
last=$(( ${#EVENTS[@]} - 1 ))
for i in "${!EVENTS[@]}"; do
  sep=','; [ "$i" -eq "$last" ] && sep=''
  printf '  "%s": [{"matcher":"*","hooks":[{"type":"command","shell":"bash","command":"bash \\"%s\\""}]}]%s\n' \
    "${EVENTS[$i]}" "$SPY" "$sep"
done
printf '}\n'
