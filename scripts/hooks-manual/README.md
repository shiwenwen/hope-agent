# Hooks 手动端到端测试

GUI 目前没有 Test Runner(在 hooks.md Roadmap 里),所以手动验证靠:**配一条把 hook
input 落盘的 command hook → 在 app 里做真实操作 → 看落盘**。本目录提供探针脚本和
全事件配置生成器。

自动化测试(协议层 / 聚合 / runner / scope / 兼容)走 `scripts/hooks-smoke-test.sh`,
与本目录互补。

## 文件

| 文件 | 作用 |
|------|------|
| `spy.sh` | 探针:把每次触发的 hook input 摘要 + env 落盘。默认 `exit 0`(放行) |
| `gen-spy-config.sh` | 生成覆盖全部 24 个真触发事件的 hooks 配置 JSON,command 自动指向 `spy.sh` |

## 用法

```bash
# 1) 把 spy 配置合并进 user-scope 配置(需要 jq)
tmp=$(mktemp)
jq --argjson h "$(scripts/hooks-manual/gen-spy-config.sh)" '.hooks = $h' \
  ~/.hope-agent/config.json > "$tmp" && mv "$tmp" ~/.hope-agent/config.json

# 2) 重启 app(或在 Settings → Hooks 里随便存一次触发热重载)

# 3) 实时看触发
tail -f /tmp/ha-hook-spy.log
```

然后在 app 里按下表把 24 个事件点一遍,每触发一个,`ha-hook-spy.log` 会多一行。

## 24 个真触发事件 → 触发动作

| 事件 | 怎么触发 |
|------|------|
| `UserPromptSubmit` ⛔ | 发送任意消息 |
| `PreToolUse` ⛔ | 让模型调任意工具前 |
| `PostToolUse` / `PostToolUseFailure` | 工具成功 / 失败后 |
| `PostToolBatch` | 一轮多个工具跑完后 |
| `PreCompact` ⛔ / `PostCompact` | 触发压缩,或 `/compact` |
| `SessionStart` | 新会话第一条消息 |
| `SessionEnd` | `/clear`,或退出登录 |
| `Stop` / `StopFailure` | 一轮回答结束 / 中断 |
| `Notification` | 登录成功、工具进入审批等待 |
| `SubagentStart` / `SubagentStop` | `subagent` 工具起子 Agent |
| `TaskCreated` / `TaskCompleted` | `task_create` / `task_update` 终态 |
| `ConfigChange` | 改任意设置 |
| `CwdChanged` | 改会话工作目录 |
| `FileChanged` | `write` / `edit` / `apply_patch` 改文件 |
| `PermissionRequest` / `PermissionDenied` | 工具触发审批 / 拒绝 |
| `UserPromptExpansion` | 输入斜杠命令 `/xxx` |
| `Elicitation` / `ElicitationResult` | `ask_user_question` 弹问 / 答完 |

⛔ = 阻断型。把 `spy.sh` 末尾改成 `exit 2` 重测这三个,验证真能拦住(工具/提示/压缩被 Block)。

## 其它权威信号源

```bash
# 审计日志:dispatch / runner / decision / security 全埋点
sqlite3 -readonly ~/.hope-agent/logs.db \
  "SELECT ts,source,message FROM logs WHERE category='hooks' ORDER BY ts DESC LIMIT 30"

# transcript 镜像:hook 脚本能读到的会话历史
cat ~/.hope-agent/sessions/<sid>/transcript.jsonl
```

## 清理

```bash
# 把 hooks 字段清空
tmp=$(mktemp)
jq '.hooks = {}' ~/.hope-agent/config.json > "$tmp" && mv "$tmp" ~/.hope-agent/config.json
rm -f /tmp/ha-hook-spy.log
```
