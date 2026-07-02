# Hope Agent 控制平面与 Coding 能力强化总纲

> 返回 [文档索引](../README.md)
>
> 更新时间：2026-07-02

## 目录

- [文档定位](#文档定位)
- [2026-07-01 路线调整](#2026-07-01-路线调整)
- [背景](#背景)
- [北极星目标](#北极星目标)
- [参考资料与调研线索](#参考资料与调研线索)
- [现状判断](#现状判断)
- [能力模型](#能力模型)
- [Dynamic Workflow、Execution Mode 与 Loop 边界](#dynamic-workflowexecution-mode-与-loop-边界)
- [阶段计划](#阶段计划)
- [30 天首个里程碑](#30-天首个里程碑)
- [验收指标](#验收指标)
- [红线与非目标](#红线与非目标)
- [后续设计文档清单](#后续设计文档清单)

## 文档定位

本文是 Hope Agent 下一阶段 Agent 控制平面与 Coding 能力建设的总纲，用来沉淀背景、目标、调研线索、参考资料和整体路线。它不是某个具体子系统的最终设计，也不直接定义数据库 schema、API 细节或 UI 交互。

后续每个大项应先拆成 `docs/roadmap/` 下的方案或 RFC，例如 `workflow`、`ToolDefinition v2`、`managed worktree`、`LSP`、`review engine`、`coding eval`。实现完成并成为稳定技术事实后，再沉淀到 `docs/architecture/` 的最终架构文档。本文只负责回答：

1. 为什么要做。
2. 参考了什么。
3. Hope 当前有什么优势和缺口。
4. 应该按什么顺序补齐。
5. 哪些边界不能碰。

## 2026-07-01 路线调整

Phase 2 已经完成 Workflow + Execution Mode 的第一版产品化：长任务可以通过 `workflow.js` 执行、审批、暂停、恢复、取消、查看 trace，并在 Workspace / Workflow Control Center 中被用户掌控。基于这个事实，后续路线不再继续把所有能力都塞进“Coding Mode”一条线里，而是调整为：

```text
通用 Agent 控制平面
  -> coding-first 产品化落地
  -> coding-specific 深水能力
```

新的优先级以 [Agent 控制平面路线图](agent-control-plane-roadmap.md) 为准：

1. **Phase 2.6：语义收口**，已完成。`/loop off|guarded|deep|autonomous` 收口为 `/mode off|guarded|deep|autonomous`。
2. **Phase 2.7：`/goal` MVP**，已完成第一版。补一等目标对象：objective、completion criteria、budget、evidence、status、final audit。
3. **Phase 2.8：Goal-driven Workflow**。Goal 派生 workflow run，失败后生成 repair run，workflow evidence 回写 goal，最终 evaluator 收口。
4. **Phase 2.9：真正 `/loop`**。只做定时、重复、轮询或条件触发，复用 cron / wakeup / automation。
5. **Phase 3：coding-specific 能力**。Managed Worktree、LSP、Review Engine、Smart Verification、Context Retrieval v2、Actionable Context Loop 已完成；后续继续做更深 review / verification / context eval。

这次调整的核心不是降低 coding 优先级，而是把 coding 能力挂到更稳的控制平面上。`/goal` 负责最终完成标准，`/workflow` 负责一次具体执行，`/mode` 负责推进强度，`/loop` 第一版负责重复触发，`/worktree` 才是 coding 场景的隔离环境。

## 背景

Hope Agent 已经具备很完整的本地 agent 底座：Tauri / HTTP / ACP 三入口、`ha-core` 核心层、Chat Engine、Plan Mode、Task、Subagent、Agent Team、Async Jobs、Hooks、Skills、Permission、Knowledge、Memory、Project、Working Directory、ACP IDE 集成等。

但这些能力目前仍然偏“通用助手平台”。要在 coding 场景中对齐甚至超过 Codex、Claude Code 一类专门 coding agent，关键不是继续加长 system prompt，而是把现有能力收束成一个 coding-first 的闭环系统：

```text
目标理解
  -> 上下文收集
  -> 动态计划
  -> 隔离执行
  -> 最小验证
  -> 独立 review
  -> 自动修复 loop
  -> trace/eval/skill 沉淀
```

这个闭环既要能在桌面 GUI 里给用户充分掌控，也要能在 server / ACP / cron 等无人值守或半无人值守场景中稳定运行。

## 北极星目标

建设 Hope Agent 的 `Coding Mode`：给定一个 issue、bug、feature、PR、测试失败或代码审查请求，Hope 能自动完成以下流程：

1. 解析任务类型和完成标准。
2. 读取项目级规则、架构文档、git 状态、相关文件、LSP 语义信息和历史上下文。
3. 生成可审批、可执行、可验证的计划。
4. 在必要时使用 subagent/team 并行探索，但写代码时通过 worktree 或任务边界隔离风险。
5. 实施最小必要改动，避免无关重构和过早抽象。
6. 根据项目约束选择最小相关验证，而不是默认跑全套检查。
7. 通过独立 review engine 找出 correctness、security、concurrency、frontend、test gap 等问题。
8. 在预算内进入 repair loop，直到验证通过、风险被解释清楚，或触发 human gate。
9. 产出 review-ready diff、验证记录、剩余风险和可复用 trace。
10. 将失败和成功案例沉淀为 eval、workflow rule、skill 或项目 guidance。

一句话目标：Hope 不只是“会写代码”，而是成为一个可审计、可恢复、可学习的本地 coding 操作系统。

## 参考资料与调研线索

### Hope Agent 当前架构

最高优先级参考是本仓已有架构与红线，尤其是：

- [Chat Engine](../architecture/chat-engine.md)：主对话入口、tool loop、streaming、foreground idle guard。
- [提示词系统](../architecture/prompt-system.md)：system prompt 分段、工具描述、working directory 注入。
- [工具系统](../architecture/tool-system.md)：ToolDefinition、工具执行、权限、deferred tool、tool_search。
- [Plan Mode](../architecture/plan-mode.md)：plan 与 task 双轨、5 态状态机、用户审批。
- [Subagent](../architecture/subagent.md)：spawn、队列、结果注入、group fan-out。
- [后台任务](../architecture/background-jobs.md)：async job、slot、approval park、completion merge。
- [Hooks 系统](../architecture/hooks.md)：28 事件、5 handler、project/local scope。
- [技能系统](../architecture/skill-system.md)：SKILL.md 渐进加载、fork 模式、预算控制。
- [ACP 协议](../architecture/acp.md)：IDE 直连、会话互通、工具与事件映射。
- [权限/审批系统](../architecture/permission-system.md)：Plan、Smart、YOLO、strict 审批、无人值守 fail-closed。

### Codex 设计线索

参考 Codex 官方手册与当前 Codex 产品形态，重点吸收这些模式：

- `AGENTS.md` 分层规则，项目规则是 durable guidance。
- Goal / Plan 工作流，把“完成标准”作为一等对象。
- Managed worktrees，用隔离工作区支持并行与后台任务。
- Skills 渐进加载，catalog 只放 name/description/path，命中后再读完整说明。
- Subagents 明确区分探索、实现、review、handoff。
- Review 命令和本地 diff 审查能力。
- Hooks / MCP / permissions 作为 agent 行为的外部控制面。
- Resume、thread、handoff 让长任务和跨设备工作不中断。

### Claude Code 线索的使用边界

本次调研读取了本地早期版本 `~/Codes/claude-code` 和提示词目录 `~/Codes/claude-code-system-prompts`。必须明确：这些材料只是历史切片和设计线索，不代表当前 Claude Code 的真实实现、产品能力或内部架构。

可借鉴的是仍然经得起推敲的工程模式：

- 工具契约包含 `read_only`、`destructive`、`concurrent_safe`、`strict`、`validate_input`、`render`、`search_hint` 等丰富元数据。
- 连续并发安全工具可以批量并行，非并发安全工具串行。
- 大工具 deferred，通过 `tool_search` 按需加载。
- Plan Mode 是只读规划状态，写代码前要有用户可审的方案。
- Fresh subagent 必须收到完整背景，不能假设继承父上下文。
- Fork subagent 用于继承上下文但隔离中间输出。
- LSP 提供 definition、references、hover、symbols、diagnostics 等语义代码能力。
- Code review 使用 candidate finding + verifier 三态确认，而不是同一个实现者自审。
- Prompt cache 需要稳定前缀，动态内容应放后面。

不应该做的是“复刻早期 Claude Code 源码”。Hope 应该把这些线索重新映射到自己的 `ha-core`、Plan、Task、Hooks、Subagent、ACP、Knowledge、Memory 和 Permission 架构中。

### 最新 agent workflow / loop 范式

参考公开资料：

- [Anthropic: Building effective agents](https://www.anthropic.com/research/building-effective-agents)
- [LangGraph Graph API](https://docs.langchain.com/oss/python/langgraph/graph-api)
- [OpenAI Agents: Running agents](https://developers.openai.com/api/docs/guides/agents/running-agents)
- [OpenAI Cookbook: Agent improvement loop](https://developers.openai.com/cookbook/examples/agents_sdk/agent_improvement_loop)

关键结论：

- Workflow 是预设代码路径，适合稳定、可审计的业务流程。
- Agent 是模型动态决定步骤和工具，适合开放式问题。
- 最强形态不是二选一，而是“静态骨架 + 动态路由”。
- `routing`、`parallelization`、`orchestrator-workers`、`evaluator-optimizer` 是 coding agent 最值得内建的四类模式。
- Loop 不只是一轮 tool call，而应覆盖 task loop、debug loop、review loop 和 improvement loop。
- Trace / eval / feedback 是长期变强的核心基础设施。

## 现状判断

### 已有优势

Hope 已经具备很多 coding agent 需要的基础能力：

- `ha-core` 是零 Tauri 依赖核心层，适合同时服务桌面、HTTP、ACP。
- Chat Engine 已经有稳定 tool loop、streaming 和上下文压缩。
- Plan Mode 已经把 plan 与 task 分离，适合承载 coding workflow。
- Subagent / Agent Team / Async Jobs 已经能做后台并行和结果注入。
- Hooks 已经对齐 Claude Code 协议风格，是天然 workflow 扩展点。
- Skills 已支持渐进加载，可承载 coding 方法论和项目模板。
- Permission 系统已有 strict、Smart、YOLO、unattended fail-closed 等安全底座。
- ACP 能连接 IDE，会成为 LSP、diagnostics、selection 上下文的重要入口。
- Knowledge / Memory / Project 能补上长期项目上下文和跨会话沉淀。

### 主要缺口

当前缺口集中在“coding 专用编排层”：

- 缺少 `Coding Mode` 这样的一等工作模式。
- 缺少 `workflow` 状态机，把 Plan、Task、Subagent、Validation、Review、Repair 串起来。
- ToolDefinition 元数据不够表达工具风险、展示、输入校验、语义分类和并发能力。
- `tool_search` 仍偏基础关键词匹配，缺少 search hint、alias、BM25、多来源 schema 组装。
- Managed Worktree 创建、恢复、归档、交接已在 Phase 3.1 补齐；后续缺口转为 detail 页面、清理策略和 review/LSP evidence 接入。
- LSP 语义代码工具和被动 diagnostics 注入已在 Phase 3.2 补齐；后续缺口是项目级配置、doctor 和 IDE context envelope。
- 独立 `/review` engine、verifier 三态和 Workspace 审查区块已在 Phase 3.3 补齐；Smart Verification 已在 Phase 3.4 补齐最小验证选择、后台低风险执行和 Goal validation evidence；Context Retrieval v2 已在 Phase 3.5 补齐任务感知上下文推荐、file search v2 + LSP symbols + diff/artifact/review/verification 聚合；Phase 3.6 已补齐 workflow/task/goal evidence 关联召回和候选行 focused review / focused verification。后续缺口是 LLM reviewer、profiles、Workflow review/verify host API、IDE/ACP 当前文件信号和 context eval。
- 缺少 coding eval harness 和系统级 improvement loop。
- 内置 coding skills 还偏“说明书”，尚未产品化为稳定 workflow policy。

## 能力模型

Coding 能力拆成 8 层建设：

| 层级 | 能力 | 目标 |
| --- | --- | --- |
| L1 Context | 项目规则、架构文档、git、文件、LSP、Knowledge、Memory | 让模型先知道自己在哪 |
| L2 Tool Contract | 工具元数据、权限、并发、输入校验、结果展示 | 让模型安全、准确地用工具 |
| L3 Planning | 任务分类、计划、critical files、reuse、verification | 动手前先形成可审设计 |
| L4 Execution | edit/apply_patch、task、async job、subagent、worktree | 控制修改范围和并行风险 |
| L5 Validation | 类型检查、单测、lint、UI smoke、diagnostics | 选择最小相关验证 |
| L6 Review | diff scan、candidate、verifier、inline finding、auto-fix | 发现实现者遗漏的问题 |
| L7 Workflow Loop | observe-plan-act-validate-review-repair | 让任务闭环完成 |
| L8 Improvement | trace、eval、retro、skill/guidance 更新 | 让系统越用越强 |

## Dynamic Workflow、Execution Mode 与 Loop 边界

语义收口详见 [Goal / Mode / Workflow / Loop 语义收口](control-plane-semantics.md)。当前 Phase 2 已落的是 `/mode` execution mode 与 `/workflow`，不是一等公民 `/goal`，也不是真正的定时/重复 `/loop`。

### 核心原则

动态工作流不是让模型随便跑，而是：

```text
静态骨架负责安全、预算、状态、审计
动态路由负责选择上下文、工具、subagent、验证和下一步
```

也就是模型可以驾驶，但道路、护栏、限速和刹车由系统控制。

### 建议新增模块

长期建议新增 `ha-core::workflow`：

```text
WorkflowRun
  id
  kind: coding.fix_bug | coding.feature | coding.review | coding.debug | research | maintenance
  state
  current_node
  execution_policy
  budget
  artifacts
  trace

WorkflowNode
  observe | classify | plan | explore | implement | validate | review | repair | ask_user | finish

WorkflowEdge
  static edge
  conditional edge
  model-routed edge
  human-gated edge
  hook-gated edge

StopPolicy
  max_iterations
  max_repair_attempts
  max_minutes
  max_cost
  no_progress_threshold
  validation_required
  human_gate_points
```

### 五类 loop

本节的 loop 指算法和任务闭环，不等同于产品命令 `/loop`。产品 `/loop` 已收口为定时、重复触发或条件轮询，第一版见 [Loop 控制平面](../architecture/loop.md)。

1. **Agent Inner Loop**

   ```text
   model -> tool calls -> tool results -> model -> final/handoff
   ```

   这是 Chat Engine 已有能力。后续要补 trace，把工具选择、失败、权限、retry、handoff 都结构化记录。

2. **Coding Task Loop**

   ```text
   Observe -> Plan -> Act -> Validate -> Review -> Repair -> Validate -> Finish
   ```

   这是 Coding Mode 的主循环。

3. **Debug Loop**

   ```text
   Reproduce -> Trace -> Hypothesis -> Minimal Fix -> Targeted Check -> Regression Guard
   ```

   没有复现、日志、测试或明确证据时，不应直接大改。

4. **Review Loop**

   ```text
   Diff Scan -> Candidate Findings -> Verifier Agents -> Confirm/Plausible/Refute -> Optional Fix
   ```

   Review agent 和 implementer 应隔离上下文，避免自己审自己。

5. **Improvement Loop**

   ```text
   Trace -> Feedback -> Eval Case -> Workflow/Skill Patch -> Re-run Eval
   ```

   这是长期超过同类工具的核心。每次失败都应该变成可回放的 eval 或 guidance 候选。

### Loop 停止条件

任何自动 loop 都必须有明确停止条件：

- 验证通过。
- review 无 P0/P1。
- 用户目标达成。
- repair 次数超限。
- 连续两轮没有有效 diff。
- 验证失败原因不变。
- 修改范围超过计划。
- 触发 protected path / dangerous action / broad refactor。
- 成本、时间、token、工具调用达到预算。

触发停止后，应进入 final 或 `ask_user`，不能无限自转。

### 用户控制面

建议新增 slash 命令或 UI 控制：

```text
/workflow
/workflow trace
/mode off
/mode guarded
/mode deep
/mode autonomous
```

语义：

| 模式 | 行为 |
| --- | --- |
| `off` | 不自动 repair，只给下一步建议 |
| `guarded` | 默认模式，允许 1-2 次低风险修复 |
| `deep` | 长任务模式，允许更多 explore/review/repair，但仍有预算 |
| `autonomous` | server/cron 场景，必须强预算、强 trace、强 human gate |

## 阶段计划

### Phase 0：Coding Baseline 与评测体系

目标：先知道现在有多强，再谈变强。

任务：

- 先建 20 个 coding eval gold task，覆盖 bugfix、frontend、test、review、repo navigation；schema 稳定后再扩展到 30-50 个。
- 为每个 task 记录输入、预期行为、允许验证、成功条件、禁止行为。
- 建 trace schema：context sources、tools、diff、tests、review findings、final outcome。
- 指标包含成功率、一次通过率、平均耗时、工具调用数、验证相关性、review 漏报、prompt cache 稳定性、审批卡点。

产物：

- [Coding Eval 体系方案](coding-eval.md)。
- [Coding Eval 首批 Gold Tasks](coding-eval-tasks.md)。
- [Coding Eval Phase 0 完成报告](coding-eval-phase0-report.md)。

### Phase 1：ToolDefinition v2 与 tool_search 升级

状态：已于 2026-06-30 完成实现与定点验证。设计与验收见 [ToolDefinition v2 RFC](tool-definition-v2.md) 和 [ToolDefinition v2 迁移 Checklist](tool-definition-v2-checklist.md)。

目标：让工具成为可推理、可搜索、可审计的对象。

任务：

- 扩展 ToolDefinition 元数据：`search_hint`、`read_only`、`destructive`、`open_world`、`strict`、`interrupt_behavior`、`path_extractor`、`validate_input`、`permission_matcher`、`render/search_text`、`auto_classifier_input`。
- 改造核心工具定义，先覆盖 `read`、`write`、`edit`、`apply_patch`、`exec`、`grep`、`find`、`tool_search`、`task_*`。
- 升级 `tool_search`：alias、search hint、BM25、`select:a,b,c`、多来源工具 schema。
- 将 token 大、低频、场景化工具默认 deferred。
- 加 prompt render diff/debug，定位 cache 失效。

产物：

- `ToolDefinition v2` RFC。
- `tool_search` v2。
- 工具迁移 checklist。

### Phase 2：Coding Mode 与原生 Skills

状态：详细方案见 [Phase 2 Coding Mode 与 Script-first Dynamic Workflow 方案](phase2-coding-mode-dynamic-workflow.md)，收口清单见 [Phase 2 完整目标与验收清单](phase2-completion-checklist.md)。2026-07-01 已完成 durable store/state machine 与 QuickJS runtime foundation；首批 host API（`task.create/update`、`fileSearch`、`tool/read/grep`、`workflow.map`、`spawnAgent/waitAll`、async job backed `validate`、`askUser`、`diff`、`trace`、`finish`）已可通过 Script Gate 后执行并 durable replay，`workflow.map` 已物化 fan-out 列表并生成嵌套位置 op-key，`spawnAgent` / `validate` / 显式 async `workflow.tool` 已具备 child_handle attach，`workflow.spawnAgent` 已补真实 subagent tool 路径 E2E（同 run id 预分配、`subagent_runs` 与 `background_jobs` 投影）和 mock-provider 回复型 fan-out E2E（两个子 Agent 真实跑过 child `run_chat_engine` + OpenAI Chat provider adapter 后由 `waitAll` 汇总），`askUser` 已复用无人值守 fail-closed 判定；permission preview / user approval 第一版已落，Draft script 会在执行前产出 preview，动态工具调用先进入 `awaiting_approval`，owner approve 后才继续；Workspace Panel 已升级为 Workflow Control Center v2，常驻 session execution mode 控制，并提供目标驱动草稿入口（coding 目标 → 观察 / 子 Agent 实现 / waitAll / 单点验证 / diff / finish 的可预检 `workflow.js`）、脚本高级编辑、创建前 Script Gate + permission preview 预检、run 总览、授权清单、审批焦点、Trace timeline、Validation 命令明细、Agents 三视图、失败恢复建议和 run draft / approve / pause / resume / cancel 操作；Tauri/HTTP owner API 已支持 preview / create / run，create 强制复用同一 preflight，approve/resume 会异步 kick runtime；`/workflow` 控制命令已接入，`/mode` 已升级为持久化 `execution_mode` 并注入 system prompt；guarded repair runtime stop guard 已落地（重复 validation fingerprint / 无有效 diff 进展 → Blocked）。外部真实 provider smoke 只作为体验抽检，不再是实现完成的唯一证据。

目标：把已有 Plan、Task、Subagent、Async Jobs、Hooks、Permission 组合成 coding-first 体验，同时不把第三方移植 skills 直接作为核心策略。

任务：

- 审计现有内置 coding skills，区分 `reference` / `vendor_optional` / `rewrite_native` / `deprecate`。
- 重写 Hope-native coding skills：`ha-coding-common`、`ha-code-review`、`ha-debug`、`ha-verify`、`ha-workflow-script` 等。
- 新增 `CodingSessionProfile` 或等价能力，按任务类型启用对应 workflow。
- 分类任务：`fix_bug`、`feature`、`review`、`debug`、`test`、`refactor`。
- Plan 输出固定包含 Context、Critical Files、Reuse、Steps、Verification、Risks。
- 加 Plan quality gate：没有关键文件、没有验证方案、没有风险说明的计划不能进入实施。
- 执行期强制 task 作为进度真相。
- Hope-native skills 才能升级为 workflow policy 候选；第三方移植 skills 只作为参考或可选 vendor skill。
- 所有验证策略遵守项目级 AGENTS。默认单点验证，不主动跑全套检查。

产物：

- `Coding Mode` / native skills 设计文档。
- skill detox 审计表。
- workflow policy registry。
- Plan quality gate。

### Phase 2.5：Script-first Dynamic Workflow + Execution Policy

状态：详细方案见 [Phase 2 Coding Mode 与 Script-first Dynamic Workflow 方案](phase2-coding-mode-dynamic-workflow.md)。

目标：让 coding 任务能通过“先写脚本、审批后执行”的动态 workflow，在预算和护栏内稳定闭环。

任务：

- 新增 durable `WorkflowRun` / `WorkflowOp` / `WorkflowEvent`，以 replay 而不是 VM snapshot 恢复长任务。
- 新增受控 `workflow.js` runtime；脚本只能调用 host API，不能直接访问 raw fs/network/process/env。
- MVP 先支持 `coding.fix_bug`、`coding.feature`、`coding.review`、`coding.debug`。
- host API 先覆盖 `tool`、`fileSearch`、`read`、`grep`、`spawnAgent`、`waitAll`、`task.create/update`、`validate`、`askUser`、`trace`、`diff`、`finish`。
- 脚本执行前做 lint / budget / permission preview / user approval（第一版已落，Workspace Panel 与 `/workflow` 控制面已接入）。
- `task_create/update` 与 workflow op 自动绑定。
- validation 失败自动生成 structured feedback，作为下一轮 repair 输入。
- 增加 `/workflow`、`/workflow trace`、`/workflow pause|resume|cancel`、`/mode` 控制面。

产物：

- script-first workflow runtime RFC。
- workflow trace viewer。
- execution policy 配置。

### Phase 2.6：控制平面语义收口

状态：已完成。详见 [Goal / Mode / Workflow / Loop 语义收口](control-plane-semantics.md)。

目标：把 Phase 2 中临时承载执行强度的 `/loop` 语义收口为 `/mode`，避免后续真正的 scheduled loop 与 execution mode 混淆。

任务：

- 删除旧 `/loop off|guarded|deep|autonomous` 执行强度入口。
- 统一用户文案为 Execution Mode / 执行模式。
- 统一代码、DB、API 为 `execution_mode` / `executionMode`。
- 明确 `/loop` 只保留给定时、重复触发、轮询或条件继续。
- 因能力尚未发布，不保留旧 alias、旧 route、旧字段兼容层。

### Phase 2.7：`/goal` MVP

状态：已完成第一版。最终架构见 [Goal 控制平面](../architecture/goal.md)，路线设计见 [Agent 控制平面路线图](agent-control-plane-roadmap.md)。

目标：补一等 Goal 对象，让长期任务有 objective、completion criteria、budget、evidence、status 和 final audit。

任务：

- 新增 goal durable store。
- 新增 `/goal <objective and completion criteria>` / `/goal status|pause|resume|clear`。
- GUI 增加 active goal strip / detail。
- Goal 记录 linked workflow runs、tasks、validation evidence。
- Goal evaluator 输出 completed / blocked，并给出 reason；`partial` 暂不写入状态机，避免状态语义漂移。
- 无痕会话不持久化 goal。

产物：

- Goal 实现与架构文档。
- Goal owner API。
- Goal UI detail / strip。
- Goal evaluator 第一版。

### Phase 2.8：Goal-driven Workflow

状态：核心已完成。Goal durable store、Workflow 绑定、validation/diff/file evidence link、GUI Goal detail、Evaluator v2、Budget v2 已落地；`/loop` 已接入 Goal evidence；Managed Worktree、LSP、Review Engine、Smart Verification、Context Retrieval v2 已分别作为 Phase 3.1-3.5 落地。详细方案见 [Goal-driven Workflow v2 路线图](goal-driven-workflow-v2.md)。

目标：让 Workflow 成为 Goal 的执行手段，而不是独立漂浮的 run。

任务：

- `workflow_runs` 增加可选 `goal_id`。
- Workflow create / repair draft 继承当前 goal。
- Workflow completion / validation / task evidence 进入 Goal audit。
- `workflow.validate` 写 `validation_passed/failed` evidence；`workflow.diff` 写 `diff_snapshot` 和最多 50 个 `file_changed` evidence。
- Goal strip 展示 linked run/task/evidence 指标；点击可展开 detail，查看 criteria、evidence、timeline、workflow/task 摘要。
- Goal evaluator 读取 workflow/evidence/budget snapshot，而不是重扫散落消息；failed validation 不会被 workflow completed 覆盖。
- Goal budget 展示 token/time/turn 使用，接近上限写 warning event，耗尽后阻止新 workflow。

产物：

- goal-workflow link 数据结构。
- linked run 指标与后续 timeline 设计。
- deterministic final audit + next evidence needed。
- budget hard stop。

### Phase 2.9：真正 `/loop`

状态：第一版已落地。最终架构见 [Loop 控制平面](../architecture/loop.md)，路线收口见 [Agent 控制平面路线图](agent-control-plane-roadmap.md#7-phase-29真正-loop)。

目标：`/loop` 只表示按时间、事件或条件重复触发，不再表示执行强度。

任务：

- 复用 cron / wakeup / automation / async jobs，不另起调度系统。
- 支持绑定 Goal 或明确 recurring prompt。
- 每个 loop 有最大次数、最大运行时长、token 预算；成本预算字段保留并暂时拒绝创建，等待 provider cost ledger。
- 每次触发都有 trace 和可审计结果。
- 支持 status / pause / resume / stop。

产物：

- Loop schedule store。
- `/loop every|until|status|pause|resume|stop`。
- Loop run trace。
- Workspace Loop 区块与 Tauri / HTTP owner API。

后续增强：

- Event-triggered loop 接入 EventBus / file watcher / CI。
- 独立 Loop detail 页面展示完整 run trace、cron log 与消息范围。
- 成本预算接入 provider cost ledger。
- Loop trigger 直接生成/运行 Goal-driven Workflow draft。

### Phase 3.1：Coding-specific 能力起点：Managed Worktree 隔离与交接

状态：已完成。worktree 已作为 Goal / Workflow / Subagent 下的 coding-specific 隔离执行面落地，而不是独立任务系统。

目标：并行写代码不污染用户当前工作区，并让长任务可隔离、可恢复、可交接。

已完成：

- `ha-core::worktree` managed worktree manager：create、list、archive、restore、handoff。
- `managed_worktrees` durable store：记录 session、purpose、base ref/sha、path、state、dirty snapshot。
- `.worktreeinclude`：复制必要 git-ignored setup，跳过 symlink。
- Workflow 绑定 `worktree_id`：运行时自动 restore，默认 cwd 切到 worktree，不可用时 fail closed/block。
- 用户可见 subagent / batch spawn 默认尝试进入隔离 worktree；内部 helper 默认不制造 worktree。
- `WorktreeCreate` / `WorktreeRemove` hooks 激活，支持企业自定义创建/清理链路。
- Tauri + HTTP owner API 对齐。
- Workspace GUI 环境面板支持创建、恢复、归档、交接；Workflow 创建区支持当前目录、新隔离工作树、已有 worktree 三种运行位置。
- 架构文档已转入 [Managed Worktree 控制平面](../architecture/worktree.md)。

后续增强：

- Worktree detail 页面：完整 diff、dirty file list、base ref、子任务/Workflow 归属和清理建议。
- 清理策略：最近 N 个、pinned/in-progress/handoff 跳过、可配置保留窗口。
- 更强 `.worktreeinclude`：支持显式 glob 预览、冲突处理和复制审计。
- Review / LSP / diagnostics evidence 进入 worktree 维度。

### Phase 3.2：LSP 与语义代码智能

状态：已完成。最终架构见 [LSP 与语义代码智能](../architecture/lsp.md)。

目标：让 Hope 不只会 grep，还能理解符号、引用和诊断。

已完成：

- `ha-core::lsp` LSP manager：按 `(workspace_root, server_id)` 缓存 stdio language server。
- 默认支持 Rust / TypeScript / Python / Go / C/C++ 的常见 language server。
- Agent 工具 `lsp`：`definition`、`references`、`hover`、`document_symbols`、`workspace_symbols`、`implementation`、`call_hierarchy`、`diagnostics`、`sync_file`、`status`。
- `write` / `edit` / `apply_patch` 成功写入后有界同步 `didOpen` / `didChange` / `didSave`，失败不影响写入结果。
- diagnostics cache + `# LSP Diagnostics` 动态 prompt 后缀，最多注入 12 条，不进入静态 prompt prefix。
- Tauri + HTTP owner API：`get_lsp_status` / `get_lsp_diagnostics`，HTTP `/api/sessions/{sid}/lsp/status` / `/api/sessions/{sid}/lsp/diagnostics`。
- EventBus `lsp:diagnostics`。
- Workspace GUI “语义诊断”区块：server 状态、错误/警告计数、最近诊断、手动刷新。
- ACP/IDE 边界已明确：open files / selection 属于动态 turn context；symbols/navigation 走 `lsp` 工具；diagnostics 走 passive cache + prompt suffix + GUI。

后续增强：

- 项目级 `.hope/lsp.json` 或插件贡献 LSP server 配置。
- LSP client restart/backoff 与 doctor。
- diagnostics 进入 Goal evidence / Workflow validation summary 的强类型链路。
- ACP IDE context envelope：open files、selection、visible diagnostics、active editor URI。

### Phase 3.3：Review Engine

状态：已完成。最终架构见 [Review Engine 控制平面](../architecture/review-engine.md)。

目标：把 review 从“提示词建议”升级为独立系统。

已完成：

- `ha-core::review` durable store：`review_runs` / `review_findings` / `review_events`。
- `/review` 能力：默认审查 uncommitted local diff，支持 status 和 finding 状态更新。
- Diff scan 复用 `load_session_git_diff`，按 session workspace scope 读取，不允许 HTTP 任意路径。
- Candidate findings：LSP diagnostics、conflict marker、possible secret、debug output、no test update、truncated diff。
- Verifier 三态：`confirmed`、`plausible`、`refuted`。
- Inline finding：file + start/end line + title/body/category/severity/verdict/status。
- Tauri + HTTP owner API：list/get/run/update finding status。
- Workspace GUI “代码审查”区块：运行审查、P0-P3 统计、finding 卡片、已修复/忽略/误报操作。
- Goal evidence：`review_passed` / `review_completed` / `review_finding`；P0/P1 open finding 阻止 Goal completed。

后续增强：

- LLM reviewer 和独立 verifier agent。
- Review profiles：correctness、security、concurrency、frontend、accessibility、tests。
- Workflow host API：`workflow.review()`。
- Auto-fix 后 focused re-review。

### Phase 3.4：Smart Verification / 智能验证选择

状态：已完成。最终架构见 [Smart Verification 控制平面](../architecture/verification-engine.md)。

目标：把“应该跑什么验证”从人工经验升级为 durable、可观察、可回写 Goal evidence 的控制平面能力。

已完成：

- `ha-core::verification` durable store：`verification_runs` / `verification_steps` / `verification_events`。
- Selector 读取当前 session diff、repo root、`AGENTS.md` / `CLAUDE.md` 项目规则。
- 推荐最小验证：Rust package check、frontend typecheck、i18n check、diff whitespace sanity。
- 全量 / 重检查作为 gated suggestion 展示，不默认自动执行。
- `run_smart_verification` 后台执行低风险 step，请求返回后仍可靠事件/轮询更新。
- Tauri + HTTP owner API：list/get/plan/run。
- Workspace GUI “验证”区块：推荐验证、运行推荐、统计、step 状态、失败输出摘要。
- Goal evidence：`validation_passed` / `validation_failed` / `validation_completed`。
- 重启时遗留 running verification run fail-closed 标记为 interrupted。

后续增强：

- 历史 trace 成功率、耗时和失败模式参与排序。
- 更细的 test impact / owner map / symbol 级验证选择。
- GUI 支持批准并运行单条 gated step。
- Workflow host API：`workflow.verify()`。
- 验证选择质量进入 coding eval。

### Phase 3.5：Context Retrieval v2 / 推荐上下文

状态：已完成。最终架构见 [Context Retrieval v2](../architecture/context-retrieval.md)。

目标：把 file search v2 从“文件名搜索”升级为任务感知上下文推荐，让用户和后续 agent 步骤能快速看到当前最该看的文件、诊断、审查项、验证项、符号和来源。

已完成：

- `ha-core::context_retrieval` 只读聚合器：Git diff、历史 artifacts、LSP diagnostics、Review findings、Verification steps、file search v2、LSP workspace symbols、URL sources。
- 统一 `ContextCandidate` 模型：`file` / `symbol` / `diagnostic` / `review_finding` / `verification_step` / `url_source`。
- 排序策略：高危 review/diagnostic/verification 与当前 diff 优先，query 作为 boost，不因搜索词隐藏高危信号。
- Tauri + HTTP owner API：`get_context_retrieval` / `GET /api/sessions/{sid}/context-retrieval`。
- Workspace GUI “推荐上下文”区块：默认推荐、关键词召回、手动刷新、事件驱动刷新；文件项复用统一文件操作策略。
- 无痕会话 fail-closed；LSP symbol 查询失败降级 warning，不阻断其它候选。

### Phase 3.6：Actionable Context Loop / 可行动上下文闭环

状态：已完成。最终架构见 [Context Retrieval v2](../architecture/context-retrieval.md)、[Review Engine 控制平面](../architecture/review-engine.md)、[Smart Verification 控制平面](../architecture/verification-engine.md)。

目标：把“推荐上下文”从只读列表升级成可行动闭环，让用户看到候选后可以直接触发最小范围的审查或验证，同时让 Goal / Task / Workflow 证据进入同一推荐排序。

已完成：

- Context Retrieval 新增 `goal_evidence` / `task` / `workflow_op` 候选类型，并统计 `goalEvidence` / `tasks` / `workflowOps`。
- 候选 metadata 支持 `actions.focusPaths` / `canReview` / `canVerify`，Workspace 候选行显示聚焦审查与聚焦验证按钮。
- `run_code_review` 支持 `focusPaths[]`，在 local diff 内收窄 changed files 与 LSP diagnostics，并在 stats/summary 中标记 focused run。
- `plan_smart_verification` / `run_smart_verification` 支持 `focusPaths[]`，在 selector 前收窄 changed files，并在计划和最终结果中保留 focused stats。
- GUI 操作复用现有 durable Review / Verification run、Goal evidence、EventBus 与 Workspace 区块，不创建平行数据模型。

后续增强：

- document symbols fallback、IDE selection envelope、ACP 当前文件信号。
- Workflow host API：`workflow.review()` / `workflow.verify()`。
- context precision / critical context recall 进入 coding eval。

### Phase 5：Review 与 Verification Engine 后续增强

目标：在 Phase 3.3 Review Engine 与 Phase 3.4 Smart Verification 的基础上，把 review 和 verification 组合成更强的闭环。

任务：

- 支持 base branch、commit range 和远程 PR review。
- Diff scan 增强到 enclosing function / symbol context。
- LLM reviewer 生成更高召回 candidate findings。
- 去重后交给独立 verifier agent，输出 `CONFIRMED`、`PLAUSIBLE`、`REFUTED`。
- 支持 review profiles：correctness、security、concurrency、frontend、accessibility、tests。
- 支持 inline finding、可选 auto-fix、fix 后 re-review。
- Verification selector 加入历史 trace、test impact、owner map 和 symbol 级影响分析。

产物：

- verifier prompt 与 result schema。
- `workflow.review()` / `workflow.verify()` host API。
- focused re-review 与 review catch-rate eval。

### Phase 6：Learning Loop 与技能沉淀

目标：让每次 coding session 都能让系统变强。

任务：

- 每次 workflow 完成后生成 lightweight retro。
- 失败案例可一键转 eval candidate。
- 成功 transcript 可抽取 workflow skill 草稿。
- 常见 failure mode 反哺工具描述、workflow policy、project guidance。
- Dashboard 展示 coding success、review catch rate、slow tools、cache invalidators、approval stalls。

产物：

- `coding improvement loop` 设计文档。
- eval backlog。
- skill/guidance draft generator。

## 30 天首个里程碑

2026-07-01 之后的首个里程碑不再是 ToolDefinition / workflow runtime foundation，它们已经进入 Phase 1 / Phase 2 已完成范围。新的 30 天目标是把控制平面补到可承载长任务：

1. 已落 `/goal` 第一版：objective、completion criteria、state、budget 字段、evidence、final audit。
2. 已在 GUI 中展示 active goal，不要求用户记 slash 命令才能掌控长期任务。
3. 已让 workflow run 可选绑定 goal，repair run 不丢 goal 归属。
4. 已让 workflow completion / validation / task evidence 回写 goal audit；validation / diff / file evidence 第一层结构化 link 已落地，Review Engine evidence 与 Smart Verification evidence 已落地，artifact/diagnostic 接入后续补。
5. 已做第一版 goal evaluator，能输出 completed / blocked + reason。
6. 后续更新 Phase 0 coding eval：新增 goal-driven 长任务场景，验证 goal evidence 与 final audit。
7. `/loop` 第一版已落地；后续增强放到 Phase 3+ 或独立 RFC。

## 验收指标

短期验收：

- coding eval 成功率有基线和趋势。
- 工具 schema token 占用下降，deferred 命中率上升。
- `tool_search` 能稳定找回核心工具和场景工具。
- Plan 中稳定出现 Critical Files、Verification、Risks。
- `/review --local` 能发现 seeded correctness issue。
- 自动验证不违反项目 AGENTS，不默认跑全套检查。

中期验收：

- 多文件 bugfix 能在 guarded loop 下完成 observe-plan-act-validate-review-repair。
- background subagent 写代码不污染本地工作区。
- LSP diagnostics 能减少明显类型错误和符号误判。
- Review verifier 能降低误报，同时保留 realistic plausible finding。
- Workflow trace 足够解释“为什么这么做”。

长期验收：

- 失败任务能自动转 eval candidate。
- 常见错误能沉淀为 workflow rule 或 skill patch。
- 同类任务重复执行时成功率、耗时、验证相关性持续改善。
- Hope 的 coding 能力不依赖单个 provider，而是由系统闭环稳定支撑。

## 红线与非目标

红线：

- 不把早期 Claude Code 源码当成当前竞品事实，只作为历史设计线索。
- 不靠无限加长 system prompt 解决系统编排问题。
- 不允许多个写代码 agent 在同一个脏工作区并行修改。
- 不让 implementer 自己作为唯一 reviewer。
- 不默认跑全套检查，必须遵守项目 AGENTS 的验证策略。
- 不绕过 permission / approval / Plan Mode / incognito / KB access 等现有安全边界。
- 不把动态日期、天气、文件清单、权限状态等内容塞进稳定 prompt 前缀破坏 cache。
- 不在无人值守场景中无限等待审批或无限 loop。

非目标：

- 本文不定义最终数据库 schema。
- 本文不定义最终前端 UI。
- 本文不一次性实现所有 coding workflow。
- 本文不追求复刻任何单一竞品。

## 后续设计文档清单

建议后续按优先级在 `docs/roadmap/` 下拆文；实现完成后再转入 `docs/architecture/`：

1. [Agent 控制平面路线图](agent-control-plane-roadmap.md)：`/goal`、`/workflow`、`/mode`、真 `/loop`、`/worktree` 的总顺序。
2. [Goal / Mode / Workflow / Loop 语义收口](control-plane-semantics.md)：产品语言与命名红线。
3. [Goal 控制平面](../architecture/goal.md)：Goal store、owner API、GUI、evaluator、evidence link。
4. [Goal-driven Workflow v2 路线图](goal-driven-workflow-v2.md)：已落地 Goal detail、validation/diff/file/review evidence、Evaluator v2、Budget v2；继续跟踪 artifact/diagnostic evidence、可选 LLM auditor 和后续系统接入。
5. [Loop 控制平面](../architecture/loop.md)：真正 `/loop` 的调度、预算、审批和 trace。
6. [Managed Worktree 控制平面](../architecture/worktree.md)：已完成的隔离工作区、handoff、UI、hooks 架构。
7. [LSP 与语义代码智能](../architecture/lsp.md)：LSP manager、tools、diagnostics pipeline。
8. [Review Engine 控制平面](../architecture/review-engine.md)：diff scan、candidate、verifier、inline finding 与 Goal evidence。
9. [Smart Verification 控制平面](../architecture/verification-engine.md)：最小验证选择、后台低风险执行、Goal validation evidence 与 Workspace 验证区块。
10. [Context Retrieval v2](../architecture/context-retrieval.md)：任务感知上下文推荐与行动入口、file search v2、LSP symbols、diff/artifact/review/verification/goal/task/workflow 聚合、focused review / verification。
11. `docs/roadmap/coding-improvement-loop.md`：retro、eval candidate、skill/guidance distillation。

这些文档完成后，再进入逐项实现。实现顺序应优先保证可评测、可回滚、可审计，而不是先堆最显眼的 UI。
