# Phase 6: Agent 调度增强

## 概述

Phase 6 新增 `spawn_and_wait` 子 Agent 动作，实现前台/后台自动切换：短任务同步返回结果，长任务自动转后台执行。

## 升级前后对比

| 特性 | 升级前 | 升级后 | claude-code 参考 |
|------|--------|--------|-----------------|
| 子 Agent 等待模式 | `spawn` 立即返回 run_id，需手动 `check(wait=true)` 轮询 | `spawn_and_wait` 自动等待，超时自动转后台 | 类似 foreground/background 任务切换 |
| 超时处理 | 手动管理，`check` 的 `wait_timeout` 最长 300s | `foreground_timeout`（默认 30s，上限 120s）内完成则内联返回，否则无缝转后台 | 类似 auto-background timer |
| 结果注入 | 后台注入系统已有（`inject_and_run_parent`），但需手动 `check` 触发获取 | `spawn_and_wait` 超时后自动衔接现有注入流程，完成后结果自动推送 | 类似 task completion notification |
| 用户体验 | 模型需要 spawn → check → result 三步操作 | 模型只需 `spawn_and_wait` 一步，快任务同步、慢任务异步 | 更简洁的 API |

## spawn_and_wait 工作流

```
subagent(action="spawn_and_wait", task="...", foreground_timeout=30)
  │
  ├─ 30s 内完成 → 返回 { mode: "foreground", result: "..." }
  │                （内联结果，如同步调用）
  │
  └─ 30s 超时   → 返回 { mode: "background", run_id: "..." }
                   （子 Agent 继续运行）
                   → 完成时 inject_and_run_parent() 自动注入
```

### 关键特性

1. **零配置降级**：超时后 spawn 任务继续运行，完成时利用现有的 `inject_and_run_parent` 注入系统，无需额外基础设施
2. **foreground_timeout 可配置**：默认 30s，上限 120s，适应不同任务时长预期
3. **标记已获取**：内联返回时调用 `mark_run_fetched()`，防止后续重复注入
4. **向后兼容**：`spawn` 和 `check` 行为不变，`spawn_and_wait` 是新增动作

## 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `action` | string | — | `"spawn_and_wait"` |
| `task` | string | — | 子 Agent 任务描述 |
| `agent_id` | string | `"default"` | 目标 Agent |
| `foreground_timeout` | integer | 30 | 前台等待秒数（上限 120） |
| `timeout_secs` | integer | 300 | 子 Agent 总超时 |
| `model` | string | — | 模型覆盖 |

## 关键文件

- `src-tauri/src/tools/subagent.rs` — `action_spawn_and_wait()` 实现
- `src-tauri/src/tools/definitions.rs` — subagent 工具 schema 新增 `spawn_and_wait` 动作 + `foreground_timeout` 参数
- `src-tauri/src/system_prompt.rs` — `TOOL_DESC_SUBAGENT` 更新
