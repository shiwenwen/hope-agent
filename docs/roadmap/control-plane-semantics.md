# Goal / Mode / Workflow / Loop 语义收口

> 返回 [路线图索引](README.md)
>
> 更新时间：2026-07-01
>
> 状态：控制平面语义收口。本文定义产品语言、命令边界和后续实现顺序；已实现细节见 `docs/architecture/`。

## 1. 结论

Phase 2 已完成的是 **Workflow + Execution Mode**，随后已补齐 **Goal 第一版**；尚未完成的是 Claude Code 语义下真正的 `/loop`。

当前统一关系：

```text
Goal        = 我要最终达成什么、完成标准是什么（已实现第一版）
Mode        = 这次会话/目标用多主动、多深的策略推进（已实现为 /mode）
Workflow    = 一次具体、可观察、可恢复的执行编排（已实现）
Task        = 用户可见进度事实（已有，workflow 内已接入）
Worktree    = 代码改动落在哪个隔离环境（后续 Phase 3，编码场景特有）
Loop        = 定时/重复触发或条件轮询（未实现，保留给真正循环任务）
```

因此旧的 `/loop off|guarded|deep|autonomous` 语义已经收口为：

```text
/mode off
/mode guarded
/mode deep
/mode autonomous
```

`/loop` 不再作为执行强度入口保留，避免把“自主性策略”和“重复触发”混成一个概念。

由于这组能力尚未发布，不保留旧 `/loop off|guarded|deep|autonomous` alias、旧 `coding-loop-mode` HTTP 路由或旧 `coding_loop_mode` 数据字段兼容层。

## 2. 参考线索

本收口参考的公开产品语义：

- Claude Code [Dynamic Workflows](https://code.claude.com/docs/en/workflows)：workflow 是可由脚本表达、可暂停/恢复/查看进度的执行编排。
- Claude Code [Scheduled Tasks](https://code.claude.com/docs/en/scheduled-tasks)：`/loop` 更接近 recurring prompt / polling / scheduled continuation，不是执行强度开关。
- Claude Code [Goal](https://code.claude.com/docs/en/goal)：`/goal` 是会话级完成条件，由独立评估判断是否达成。
- Claude Code [Agent Loop](https://code.claude.com/docs/en/agent-sdk/agent-loop)：agent inner loop 是 prompt → tool calls → tool results → repeat 的底层循环。
- Claude Code [Worktrees](https://code.claude.com/docs/en/worktrees)：worktree 是文件修改隔离环境，主要服务代码任务。
- OpenAI Codex [Follow a goal](https://developers.openai.com/codex/use-cases/follow-goals)：`/goal` 是长任务 objective + stopping condition + validation loop。
- OpenAI Codex [Worktrees](https://developers.openai.com/codex/app/worktrees)：worktree 支持独立任务与后台隔离。
- OpenAI Codex [Workflows](https://developers.openai.com/codex/workflows)：workflow 更像可复用工作方式和任务执行模式，不等同于定时 loop。
- OpenAI Codex [Use cases](https://developers.openai.com/codex/use-cases)：goal / workflow / approvals / skills / automations 不是纯编码概念，编码只是最强适配场景之一。

本地早期 Claude Code 源码和提示词目录仍只作为历史线索，不作为当前产品事实。

## 3. 当前实现状态

| 概念 | 当前状态 | 产品入口 | 数据 / API | 说明 |
| --- | --- | --- | --- | --- |
| Goal | 已实现第一版 | `/goal` + Workflow Control Center Goal strip | `goals` / `goal_events` / `goal_links`，HTTP `/api/sessions/{id}/goal` / `/api/goals/{id}` | 长期任务顶层对象；完整实现见 [Goal 控制平面](../architecture/goal.md)。 |
| Mode | 已实现 | `/mode` + Workflow Control Center | `sessions.execution_mode`，`get_execution_mode` / `set_execution_mode`，HTTP `/api/sessions/{id}/execution-mode` | 会话级执行策略：`off` / `guarded` / `deep` / `autonomous`。 |
| Workflow | 已实现 | `/workflow` + Workflow Control Center | `workflow_runs.execution_mode`、`workflow_ops`、`workflow_events` | 一次具体 run，保存创建时的 execution mode 快照。 |
| Task | 已有并接入 workflow | Workflow UI / `workflow.task.*` | session task store + workflow host API | 用户可见进度事实，不靠 label 定位，按 create 返回 handle 更新。 |
| Worktree | 未实现 | 后续 `/worktree` / GUI | 后续 managed worktree registry | 代码改动隔离环境，偏 coding-specific。 |
| Loop | 未实现 | 后续 `/loop` | 后续 schedule / recurrence / condition trigger store | 只用于真正重复触发、轮询或定时继续。 |

## 3.1 调整后的实施顺序

详细路线见 [Agent 控制平面路线图](agent-control-plane-roadmap.md)。当前顺序固定为：

```text
Phase 2.6  语义收口（已完成）
Phase 2.7  /goal 第一版（已完成）
Phase 2.8  Goal-driven Workflow 核心闭环（已部分落地，增强继续）
Phase 2.9  真正 /loop
Phase 3    Worktree / LSP / Review 等 coding-specific 能力
```

这意味着 worktree、LSP、review engine 暂不作为下一步顶层优先级。它们仍然重要，但应挂在 Goal / Workflow 控制平面之下，否则容易形成一组强工具，却缺少长期任务的完成标准、证据链和最终收口。

## 4. `/mode` 的准确语义

`/mode` 回答的是：

> 这次会话里，Agent 应该以多主动、多深入、多连续的方式推进？

| Mode | 用户含义 | 行为边界 |
| --- | --- | --- |
| `off` | 普通对话 | 不自动进入 repair guard；失败后主要报告下一步。 |
| `guarded` | 守护式推进 | 默认长任务策略；允许少量低风险修复，但严格预算和 stop guard。 |
| `deep` | 深入排查 | 更多 repo reconnaissance、验证和独立分析；仍需预算与停止条件。 |
| `autonomous` | 高自主推进 | 在安全边界内持续推进；不能绕过权限、审批、AGENTS 或 destructive gate。 |

命名红线：

- 用户可见文案用 **Execution Mode / 执行模式**。
- 代码、DB、API 用 `execution_mode` / `executionMode`。
- 不再使用 `coding_loop_mode`、`loop_mode`、`Coding Loop` 表示执行强度。
- `loop` 一词只用于真实循环：repair loop、agent inner loop、scheduled loop、polling loop 等。

## 5. `/workflow` 的准确语义

`/workflow` 回答的是：

> 这一次具体怎么执行？执行到哪一步了？能否审批、暂停、恢复、取消、修复？

Workflow 是可审计 run，而不是长期目标本身。一个未来的 Goal 可以派生多个 WorkflowRun；一个 WorkflowRun 也可以作为失败后的修复 run 派生出新的 WorkflowRun。

当前已落能力：

- `workflow.js` 受控 QuickJS runtime。
- durable `workflow_runs` / `workflow_ops` / `workflow_events`。
- Script Gate、permission preview、approval。
- pause / resume / cancel。
- trace / validation / agents / output budget。
- repair draft、parent run / origin 追踪。
- execution mode 快照：`workflow_runs.execution_mode`。

## 6. `/goal` 的准确语义

Goal 是已经落地的一等对象，用来把 workflow、task、validation evidence 纳入长期目标闭环。实现细节见 [Goal 控制平面](../architecture/goal.md)。

当前入口：

```text
/goal <objective and completion criteria>
/goal
/goal status
/goal pause
/goal resume
/goal evaluate
/goal clear
```

Goal 应保存：

- objective。
- completion criteria。
- current status。
- linked workflow runs。
- linked tasks。
- validation evidence。
- final audit。
- token / time / turn budget。
- last evaluator result。

完成判定不应只靠模型自说自话，而应结合：

- 用户定义的完成标准。
- workflow trace。
- validation results。
- changed files / artifacts。
- task 状态。
- 必要时的轻量 evaluator。

当前 evaluator 已升级为 deterministic rule gate：无证据、workflow failed/blocked/cancelled 且无后续修复、validation failed 且无后续 passing validation、criteria 缺少 strong evidence、budget exhausted 都会进入 `blocked`；无 blocker、无 missing 且有 workflow completed / validation passed / task completed 这类 strong evidence 时才进入 `completed`。LLM evaluator 仍是后续可选增强，只能补 rationale，不能覆盖 hard blocker。

## 7. 真正 `/loop` 的位置

真正 `/loop` 不再表示 `guarded/deep/autonomous`。它应该回答：

> 这个 prompt / goal / workflow 是否需要按时间、事件或条件重复触发？

建议后续形态：

```text
/loop every 10m: check CI and continue fixing if failing
/loop until <condition>
/loop stop
/loop status
```

实现前置条件：

- 必须有 Goal 或明确的 recurring prompt。
- 必须有预算、最大次数、最大运行时间。
- 必须有无人值守审批策略。
- 必须能暂停/停止/审计。
- 必须和 cron / automation / wakeup 统一，而不是另起一套调度系统。

## 8. 通用性边界

这些能力不是只能用于 coding：

- Goal：通用。
- Mode：通用，但当前 prompt 文案偏 coding-first。
- Workflow：通用 runtime，当前模板和 GUI 入口偏 coding-first。
- Task：通用。
- Loop：通用。

这些能力偏 coding-specific：

- Worktree。
- LSP / diagnostics。
- code review finding。
- git diff / validation commands。
- AGENTS.md coding rules。

产品策略：底座做通用，首批模板和 UI 以 coding-first 打磨。
