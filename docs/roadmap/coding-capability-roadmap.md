# Hope Agent 控制平面与 Coding 能力强化总纲

> 返回 [文档索引](../README.md)
>
> 更新时间：2026-07-03

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
5. **Phase 3：coding-specific 能力**。Managed Worktree、LSP、Review Engine、Smart Verification、Context Retrieval v2、Actionable Context Loop、Coding Eval、Workflow review/verify、Repair Loop 自动化、Deep Review / Profiles / IDE Context、Trend Report / Improvement Loop 已完成。
6. **Phase 4：Learning Loop / Skill & Guidance 沉淀**。Phase 4.1 Proposal-to-Action、Phase 4.2 Draft Promotion + Workflow Retro、Phase 4.3 Dashboard 全局学习视图、Phase 4.4 Transcript Distillation + Failure Feedback 已完成：改进 proposal 可预览、应用成 eval / workflow / guidance / skill 草稿产物，并可显式晋升为正式 fixture / project guidance / active skill；Dashboard 可看全局 / 项目级 workflow、eval、review、verification、proposal、retro 趋势；Workspace 可显式从 transcript / workflow / failure feedback 提炼更高质量候选。
7. **Phase 5：任务级评测与策略效果评估**。Phase 5.1 Task-level Eval Runner、Phase 5.2 Agent Execution Runner、Phase 5.3 Gold Task Pack v1、Phase 5.4 Strategy Effect Evaluator、Phase 5.5 Gold Task Pack 全量自动化、Phase 5.6 mock tool-call 基线、Phase 5.7 Strategy Effect 趋势持久化 / Dashboard、Phase 5.8 Release Gate 与 Phase 5.9 外部模型基线 runner 已完成：可以从 task prompt 触发真实 chat engine execution，或用 deterministic fixture patch 做无模型回归，再调用真实 Review / Smart Verification / Context Retrieval / Goal evaluator，并按任务 schema 判分和记录 eval run；20 个 active gold tasks 已可批量 materialize / run，pack / strategy report history 已进入 Dashboard 质量趋势，mock Responses provider 可驱动真实 `write` 工具产出 candidate diff，策略改动前后的 pack report 已可确定性对比，release gate 可把持久化 history 转成发布质量结论，外部模型基线可显式传 provider/model 在 Gold Pack 上运行，后续继续补跨项目学习泛化。

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
- LSP 语义代码工具和被动 diagnostics 注入已在 Phase 3.2 补齐；后续缺口是项目级配置和 doctor。
- 独立 `/review` engine、verifier 三态和 Workspace 审查区块已在 Phase 3.3 补齐；Smart Verification 已在 Phase 3.4 补齐最小验证选择、后台低风险执行和 Goal validation evidence；Context Retrieval v2 已在 Phase 3.5 补齐任务感知上下文推荐、file search v2 + LSP symbols + diff/artifact/review/verification 聚合；Phase 3.6 已补齐 workflow/task/goal evidence 关联召回和候选行 focused review / focused verification；Phase 3.7 已补齐确定性 coding control-plane eval harness；Phase 3.8 已补齐 Workflow review/verify host API 与 Goal-aware eval；Phase 3.9 已补齐 bounded repair loop 自动化、受控 block 停机和 repair-loop eval；Phase 3.10 已补齐 LLM reviewer、review profiles、IDE/ACP 当前文件信号、symbol-context evidence 和 profile/IDE eval；Phase 3.11/4.1/4.2/4.3/4.4 已补齐趋势报告、proposal queue、proposal-to-action、retro、promotion、Dashboard 全局学习视图、transcript distillation 和 failure feedback。
- 已有第一层 coding eval harness、Phase 5.1 task-level scorer、Phase 5.2 agent execution runner、Phase 5.3 Gold Task Pack v1、Phase 5.4 strategy effect evaluator、Phase 5.5 Gold Task Pack 全量自动化、Phase 5.6 mock tool-call 基线、Phase 5.7 策略效果趋势持久化 / Dashboard、Phase 5.8 release gate 与 Phase 5.9 外部模型基线 runner；仍缺跨项目学习泛化。
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
- 更完整的 ACP / IDE 双向 RPC；轻量 IDE context envelope 已在 Phase 3.10 落地。

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
- 验证执行质量、历史失败模式和趋势质量进入更高层 eval。

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
- context precision / critical context recall 已进入 Phase 3.7/3.8 控制面 eval，后续继续扩展到人工 gold task 与趋势 dashboard。

### Phase 3.7：Coding Eval 控制面评测

状态：已完成。最终架构见 [Coding Eval 控制面评测](../architecture/coding-eval.md)，人工 gold task 体系继续见 [Coding Eval 体系方案](coding-eval.md)。

目标：把 Review、Smart Verification、Context Retrieval、Goal、Task、Workflow 的协同质量纳入可自动回归的 deterministic harness，先守住控制面底座，再继续做 Workflow review/verify、repair loop、task-level scorer 和真实 Agent execution eval。

已完成：

- `ha-core::coding_eval` fixture harness：临时 git repo、baseline commit、local diff、真实 session/goal/task/workflow seed。
- 集成测试 `cargo test -p ha-core --test coding_eval --locked`，加载 `crates/ha-core/tests/fixtures/coding_eval/*.json` 并聚合失败报告。
- 三组首批 fixture：Rust 控制面召回、docs-only sanity、focused scope 不扫无关文件。
- 断言 review finding、verification command、focused stats、context action path、`context_precision` 与 `critical_context_recall`。
- 不调用 LLM、不默认执行真实验证命令、不访问网络，适合默认 CI 做稳定回归；fixture 显式 `workflow.validate()` 时才执行受控验证命令。

后续增强：

- 增加 LSP diagnostics、Goal final audit / repair blocked fixture。
- 输出可选 JSON/HTML eval 报告和 release gate 摘要。
- Phase 5.1 已把候选 diff 的 task-level 成功率与确定性控制面指标串联成 improvement loop；Phase 5.2 已把 agent execution stage 接到 scorer 前；Phase 5.3 已把首批 active gold tasks 接成可批量运行的 Gold Task Pack v1；Phase 5.4 已把策略效果对比接成纯函数 owner API；Phase 5.5 已把 20 个 gold tasks 全量自动化；Phase 5.6 已补 mock tool-call 写文件基线与 `toolCalls` 指标；Phase 5.7 已把 pack / strategy history 接入 Dashboard；Phase 5.8 已把持久化 history 接入 release gate；Phase 5.9 已补外部模型基线 runner。

### Phase 3.8：Workflow Review/Verify Host API 与 Goal-aware Eval

状态：已完成。最终架构见 [Workflow 与 Execution Mode](../architecture/workflow.md)、[Review Engine 控制平面](../architecture/review-engine.md)、[Smart Verification 控制平面](../architecture/verification-engine.md)、[Coding Eval 控制面评测](../architecture/coding-eval.md)。

目标：让 workflow 不只会执行工具和验证命令，还能在脚本内发起 durable review 与 Smart Verification 计划，并把这些控制面证据稳定挂回 Goal / Context Retrieval。

已完成：

- `workflow.review({ focusPaths?, baseRef?, profiles?, ideContext?, scope? })` host API：idempotent durable op，复用 Review Engine，默认 local diff，继承 workflow `goal_id`。
- `workflow.verify({ focusPaths?, maxCommands?, scope? })` host API：idempotent durable op，复用 Smart Verification selector，只生成计划，不执行命令。
- Script Gate / permission preview 把 `workflow.review()` / `workflow.verify()` 归类为 permission-neutral coding control-plane API，静态调用可直接通过。
- Goal evidence 串联：review 继续写 `review_passed` / `review_completed` / `review_finding`，verification plan 写 `validation_completed`，workflow completion 写 `workflow_completed`。
- Coding Eval 新增 workflow-bound fixture，覆盖 workflow op、review run、verification plan、Goal evidence 与 Context Retrieval 召回。

边界：

- `workflow.verify()` 不代表验证通过；它只证明“验证计划已生成”。真正执行命令仍由 `workflow.validate()` 或 owner 面板运行 verification step。
- review / verify 不新增平行数据模型；GUI 仍读取现有 Review / Verification / Goal / Context Retrieval 控制面。

### Phase 3.9：Repair Loop 自动化

状态：已完成。最终架构见 [Workflow 与 Execution Mode](../architecture/workflow.md)、[Coding Eval 控制面评测](../architecture/coding-eval.md)。

目标：把“修复 → 验证 → 审查 → 再修复 / 停机”从提示词约定升级为 workflow runtime 的 bounded loop，让长任务失败时可控、可信、可恢复、可被 eval 证明。

已完成：

- `workflow.repairLoop({ label?, maxAttempts?, validationCommands?, focusPaths?, review?, verify?, maxVerificationCommands? }, fn)`：脚本级动态修复循环，修复动作仍由 callback 决定，不退回结构化 DSL。
- 每轮 repair attempt 自动创建用户可见 task，执行 callback，随后按配置运行 `workflow.validate()`、focused `workflow.review()` 和 `workflow.verify()`，并写入结构化 trace。
- `workflow.block({ reason?, label?, payload? })`：显式受控停机出口，写 `workflow_block_requested` event，将 run 转 `blocked`，并形成 Goal `workflow_blocked` evidence。
- attempt budget 耗尽时统一 `blocked(reason=repair_loop_attempts_exhausted)`，不会伪装 completed；原有 guarded repair stop guard 仍处理重复验证失败和无有效 diff 进展。
- GUI 目标驱动 workflow 草稿默认使用 repairLoop，而不是单次 implement + validate。
- Coding Eval 新增 `repair_loop_blocks_with_evidence` fixture，覆盖 repair loop blocked、validation_failed / workflow_blocked evidence、Context Retrieval 召回。

边界：

- repairLoop 不自动生成代码改动；它负责循环骨架和停机语义。具体修复仍由脚本 callback、subagent 或工具调用完成。
- `workflow.verify()` 在 loop 内仍是 planning-only；真正执行命令由 `validationCommands` / `workflow.validate()` 承担。

### Phase 3.10：Deep Review / Profiles / IDE Context

状态：已完成。最终架构见 [Review Engine 控制平面](../architecture/review-engine.md)、[Context Retrieval v2](../architecture/context-retrieval.md)、[Workflow 与 Execution Mode](../architecture/workflow.md)、[Coding Eval 控制面评测](../architecture/coding-eval.md)。

目标：把当前 deterministic review / verification 从“结构化控制面”提升到“更接近资深工程师的缺陷发现能力”，同时让当前 IDE / ACP 工作上下文成为一等信号。

已完成：

- LLM reviewer：`deep` profile 下通过 bounded side-query 生成候选 findings，超时/失败只写 warning，不阻断 deterministic review。
- Review profiles：`correctness`、`security`、`maintainability`、`tests`、`concurrency`、`frontend`、`accessibility`、`deep` 可组合选择，并写入 review run stats / Workspace 展示。
- IDE / ACP context：接入当前文件、selection、open tabs、active diagnostic、active symbol，让 Context Retrieval 和 review evidence 更贴近用户正在看的位置。
- Session IDE context owner API：Tauri + HTTP `get/save/clear_session_ide_context`，ACP `_meta.ideContext` best-effort 写入。
- Diff scan 增强：从文件级扩到 enclosing function / symbol context，finding evidence 可解释当前符号位置。
- Workflow 接入：`workflow.review({ profiles?, ideContext? })` 与 `workflow.repairLoop({ reviewProfiles? })` 支持 profile-aware review。
- GUI：Workspace Review 区块提供 profile toggles，run card 展示 active profiles、IDE context 与 Deep reviewer 状态；Context 区块展示 IDE 候选与 IDE 信号计数。
- eval 扩展：新增 `profiles_ide_context_recall` fixture，保持无 LLM 的 deterministic 控制面回归。

验收：

- Workspace 中的 Review / Context 区块能说明采用了哪些 profile 和 IDE 信号。
- Focused review 的候选文件 / 行号更准，且不会扫无关文件。
- 没有 IDE / ACP 信号时优雅降级，不影响 server / headless workflow。

### Phase 3.11：Trend Report / Improvement Loop 接口

状态：已完成。

目标：把 Phase 3 已有的 workflow、goal、review、verification、repair loop 和 eval 证据汇总成可持续改进系统，而不是只停留在单次任务完成。

已落地：

- Coding trend report：统计 coding eval 成功率、review finding/blocker、verification 选择质量、repair loop 成功率 / blocked 原因。
- Failure taxonomy：把验证失败、review blocker、权限卡点、上下文漏召回、无有效 diff 进展、repair loop exhausted、verification selection gap 等归入可比较分类。
- Eval backlog 接口：失败 bucket 生成 draft `eval_candidate` proposal，payload 包含 failure、scope、expected signals。
- Workflow / skill / guidance 候选：成功 run 或高频 blocker 可生成 `workflow_template` / `skill_candidate` / `guidance_candidate` proposal，默认只生成草案。
- GUI 报告：Workspace 「质量趋势」区块显示当前 session/project 近 30 天质量趋势、常见卡点和 proposal 草案，并可显式提炼 transcript/workflow/failure feedback 候选；Phase 4.3 已补 Dashboard 全局 / 项目级学习视图。
- Eval 覆盖：deterministic coding eval harness 增加 improvement run/check，repair-loop fixture 覆盖趋势报告与 proposal 语义。

验收：

- 单次任务结束后，用户能看到“为什么完成 / 为什么阻塞 / 下次怎么改进”。
- 趋势报告不依赖外部 LLM，至少能基于 durable 控制面数据稳定生成。
- 任何自动沉淀都必须先进入 proposal，不直接改项目规则或全局 skill。
- 最终架构见 [Coding Improvement Loop](../architecture/coding-improvement-loop.md)。

### 后续池：Review 与 Verification Engine 增强

目标：在 Phase 3.3 Review Engine 与 Phase 3.4 Smart Verification 的基础上，把 review 和 verification 组合成更强的闭环；其中 Deep Review、profiles、IDE context 已在 Phase 3.10 落地，趋势指标已前移到 Phase 3.11。

任务：

- 支持 base branch、commit range 和远程 PR review。
- 独立 verifier agent v2：在当前本地 verifier 后追加 evidence quote / 反证 / 更细降噪。
- 支持 inline finding、可选 auto-fix、fix 后 re-review。
- Verification selector 加入历史 trace、test impact、owner map 和 symbol 级影响分析。

产物：

- verifier prompt 与 result schema。
- focused re-review 与 review catch-rate eval。
- repair loop 趋势指标、成功率和失败模式 dashboard。

### 后续池：Learning Loop 与技能沉淀

状态：Phase 4.4 Transcript Distillation + Failure Feedback 已完成；Phase 5.1 task-level eval runner、Phase 5.2 agent execution runner、Phase 5.3 Gold Task Pack v1、Phase 5.4 strategy effect evaluator、Phase 5.5 Gold Task Pack 全量自动化、Phase 5.6 mock tool-call 基线、Phase 5.7 strategy effect 趋势持久化 / Dashboard、Phase 5.8 release gate 与 Phase 5.9 外部模型基线 runner 已完成；更高质量的跨项目通用化仍属后续增强。

目标：让每次 coding session 都能让系统变强；eval backlog、workflow / skill / guidance proposal 已作为 Phase 3.11 的接口先落一层，Phase 4.1 已补上从 proposal 到草稿产物的安全落地动作，Phase 4.2 已补上 terminal workflow retro 与人工显式 promotion，Phase 4.3 已补上全局 / 项目级学习 Dashboard，Phase 4.4 已补上显式 transcript/workflow/failure feedback 蒸馏，Phase 5.1 已补上候选 diff 的任务级判分，Phase 5.2 已补上从 task prompt 到候选结果的 agent execution 阶段，Phase 5.3 已补上 active gold task pack 的批量回放入口，Phase 5.4 已补上策略效果对比，Phase 5.5 已补上 20 个 gold tasks 全量自动化，Phase 5.6 已补上 mock tool-call 写文件基线，Phase 5.7 已补上策略效果趋势持久化与 Dashboard，Phase 5.8 已补上 release gate，Phase 5.9 已补上外部模型基线 runner，后续继续增强跨项目泛化。

已落地：

- `eval_failed` 进入 failure taxonomy，失败 eval run 可生成 `eval_candidate` backlog。
- proposal 可预览 action plan：目标路径、是否已存在、内容预览。
- proposal apply 先原子 claim 到内部 `applying`，目标已存在或并发创建都 fail-closed，不覆盖；`applied` 终态不可被人工状态更新改回草案。
- `eval_candidate` 可应用为 `.hope-agent/coding-improvement/eval-candidates/*.json` 草稿。
- `workflow_template` 可应用为 `.hope-agent/coding-improvement/workflows/*.md` 草稿。
- `guidance_candidate` 可应用为 `.hope-agent/coding-improvement/guidance/*.md` 草稿。
- `skill_candidate` 可应用为 `~/.hope-agent/skills/ha-learned-*/SKILL.md` managed draft skill。
- 每次 terminal workflow 会生成 lightweight retro，retro recommendation 可进入 proposal queue。
- 已应用草稿可显式 promotion：`eval_candidate` 进入正式 coding eval fixture，`workflow_template` / `guidance_candidate` 进入 promoted project docs 并由 `AGENTS.md` managed include 引入，`skill_candidate` 激活 managed draft skill。
- Workspace 质量趋势区块支持展开详情、预览、应用、晋升、拒绝和 artifact/error 展示。
- Dashboard Learning Tab 新增 Coding Improvement 全局 / 项目级视图：overview、timeline、project signals、failure modes、proposal status、latest retros。
- `dashboard_coding_improvement` 只读 owner API 已接通 Tauri / HTTP / Transport；不生成 proposal、不 apply、不 promotion。
- `distill_coding_improvement_proposals` 已接通 Tauri / HTTP / Transport；显式扫描 transcript message、tool error、workflow ops 与 failure taxonomy，生成更高质量的 workflow / skill / guidance draft proposal。
- Workspace 质量趋势区块新增「提炼候选」动作；蒸馏候选仍走同一 proposal 队列和 preview/apply/promotion 安全链路。
- `improvement_proposal_to_action` fixture 覆盖 proposal-to-action 回归。
- `improvement_retro_and_promotion` fixture 覆盖 retro 与 promotion 回归。
- `task_level_eval_runner` fixture 覆盖任务级 scorer：changed files、required / forbidden diff、验证命令、review/context/goal 证据和 eval run 记录。
- `agent_execution_runner_fixture_patch` fixture 覆盖 execution stage 先产出 candidate diff，再进入 review / verification / context / task scoring / eval-run recording。
- mock-provider 单测覆盖 `mode="agent"` 真实调用 chat engine、创建 turn 并记录 response。
- mock Responses tool-call 单测覆盖真实 `write` 工具写入临时 repo、记录 `toolCalls` 并产出 candidate diff。
- Gold Task Pack 覆盖 20 个 active gold tasks：可通过 `list_coding_eval_gold_tasks` 查看 registry，通过 `run_coding_eval_gold_task_pack` 批量 materialize / run；case 覆盖 docs/design、Rust、TS、i18n、多文件 diff 与 review-seeded 场景。
- Strategy Effect Evaluator 覆盖策略改动前后两份 pack report 的 pass rate、task score、context recall、validation violations、scope creep 和 execution failures 对比；可通过 `evaluate_coding_eval_strategy_effect` / `POST /api/coding-eval/strategy-effects/evaluate` 调用，`recordRun=true` 时写入 strategy effect history。
- Gold Task Pack / Strategy Effect history 已持久化到 `coding_eval_pack_runs` / `coding_strategy_effect_runs`，Dashboard Learning Tab 展示 pack pass rate、strategy verdict、tool-call failure mode、validation / scope creep 趋势和 latest strategy effects。
- `dashboard::coding_improvement` 单元测试覆盖项目 rollup、pack / strategy / tool-call 聚合与 incognito 排除。

后续任务:
- 跨项目学习泛化：验证 promoted guidance / skill 是否能在不同项目稳定提升，而不是只优化单个项目或单类 fixture。
- 成功 transcript 可抽取更高质量 workflow skill 草稿。
- 常见 failure mode 反哺工具描述、workflow policy、project guidance。
- Dashboard 继续补 review catch rate、slow tools、cache invalidators、approval stalls 等更细诊断。

产物：

- [Coding Improvement Loop](../architecture/coding-improvement-loop.md) 架构文档已落地；后续补更强 distillation 设计、failure feedback 反哺设计与跨项目学习泛化。
- [Coding Eval 控制面评测](../architecture/coding-eval.md) 已记录 Phase 5.1 task-level eval runner、Phase 5.2 agent execution runner、Phase 5.3 Gold Task Pack v1、Phase 5.4 strategy effect evaluator、Phase 5.5 Gold Task Pack 全量自动化、Phase 5.6 mock tool-call 基线、Phase 5.7 strategy effect 趋势持久化、Phase 5.8 release gate 与 Phase 5.9 外部模型基线 runner。
- eval / workflow / guidance / skill draft generator。

### Phase 5.1：Task-level Eval Runner（已完成）

目标：把“候选代码改动是否真的完成任务”从人工记录推进到可回归的确定性 runner，为后续真实 Agent execution benchmark 打底。

已落地：

- `fixture.task` 任务级 schema：任务 id、类型、提示词、期望/禁止行为、预期产物、允许验证和成功标准。
- `runs.task` 执行开关：默认刷新 Goal evaluator，并把结果记录到 `coding_eval_runs(suite='task_level_coding_eval')`。
- `checks.task` 判分断言：期望 outcome / 最低分、必须/禁止改动文件、必须/禁止 diff 片段、必须/禁止验证命令、最大改动文件数、review / verification / context / goal 要求、必召回上下文。
- 输出 `CodingTaskEvalReport`：`pass` / `partial` / `fail` / `blocked`、score、failure category、diff summary、validation summary、review summary、context recall、goal evidence 和逐项 checks。
- Owner API 已接通 Tauri / HTTP / Transport：`run_coding_task_eval_fixture` / `POST /api/coding-eval/task-fixtures/run`。
- `task_level_eval_runner` fixture 覆盖 docs-only 候选 diff、cheap validation、context recall、Goal evaluation、eval run 记录和 Improvement Loop 消费。

明确不包含：

- 不调用 LLM。
- 不让真实 Agent 从 prompt 开始自动执行任务；该能力已在 Phase 5.2 补上。
- 不默认执行项目验证命令；只有 fixture 显式 workflow validation 时才执行。

后续已完成：

- Phase 5.7 已补齐策略效果趋势持久化与 Dashboard；Phase 5.8 已补 release gate；Phase 5.9 已补外部模型基线 runner；后续转向跨项目学习泛化。

### Phase 5.2：Agent Execution Runner（已完成）

目标：把“从 task prompt 到候选结果”的执行阶段接入 eval harness，让产品 API 能真实驱动 agent 执行，再复用 Phase 5.1 scorer 判分。

已落地：

- `runs.execution` 执行阶段：在 review / verification / context / task scoring 前运行。
- `mode="agent"`：创建 user message + chat turn，调用 `run_chat_engine`，使用 fixture 显式传入的 `providers` / `modelChain`。
- `mode="fixture_patch"`：无外部 LLM 的 deterministic 替身，只在执行阶段写入 `repo.changes`，用于稳定回归，不冒充真实 agent 成功率。
- 输出 `AgentExecutionEvalReport`：mode、status、prompt、agentId、turnId、response/error、modelUsed、changedFiles、diffBytes。
- `checks.execution`：可断言 mode、status、turn、response/error、必须/禁止 changed files。
- task scorer 自动加入 `execution.completed` critical check；执行失败会让 task outcome 失败。
- `agent_execution_runner_fixture_patch` fixture 覆盖执行阶段产出 diff 后接 review / verification / context / task scoring / eval-run recording。
- mock-provider 单测覆盖 `mode="agent"` 真实调用 chat engine、创建 turn 和记录 response。

### Phase 5.3：Gold Task Pack v1（已完成）

目标：把 Phase 0 人工 gold task 的 active 子集转成可批量 materialize / run 的结构化 pack，让评测不再只能手写单个 fixture JSON。

已落地：

- `GoldTaskPackSummary` / `GoldTaskPackReport`：pack-level 汇总、case summary、pass/fail/skipped/error、总 checks 和逐 case `FixtureReport`。
- 内置 Gold Task Pack v1：首批 5 个 active gold tasks（`CE-TEST-004`、`CE-RUST-001`、`CE-REV-002`、`CE-NAV-001`、`CE-NAV-002`）已自动化。
- 每个自动化 case 会 materialize 成普通 `CodingEvalFixture`，默认 `runs.execution.mode="fixture_patch"`，再接 Review / Smart Verification / Context Retrieval / Goal / Task scorer。
- Owner API 已接通 Tauri / HTTP / Transport：`list_coding_eval_gold_tasks` / `GET /api/coding-eval/gold-tasks`，`run_coding_eval_gold_task_pack` / `POST /api/coding-eval/gold-tasks/run`。
- targeted tests 覆盖 pack summary 与两个 active cases 的批量回放。

明确不包含：

- 默认不访问外部模型；真实模型稳定基线已在 Phase 5.9 作为显式 agent pack runner 补上。

后续已完成：

- Phase 5.5：Gold Task Pack 全量自动化，把更多 Phase 0 任务纳入自动化；Phase 5.6 已补 mock tool-call 基线；Phase 5.9 已补外部模型基线 runner。

### Phase 5.4：Strategy Effect Evaluator（已完成）

目标：把 workflow policy、skill/guidance、tool contract 或 prompt 策略改动前后的质量差异，从“看几条结果的感觉”变成可回归的确定性对比。

已落地：

- `StrategyEffectEvalInput` / `StrategyEffectReport`：输入 baseline / candidate 两份 `GoldTaskPackReport`，输出总体 verdict、逐维度 delta、逐 case 对比、regressions / improvements 摘要。
- 聚合维度：pass rate、average task score、context recall、validation violations、scope creep、execution failures。
- 防假阳性规则：只用共同 case 算聚合指标；candidate 新增 case 只展示；candidate 漏掉 baseline case 记为回归风险。
- Owner API 已接通 Tauri / HTTP / Transport：`evaluate_coding_eval_strategy_effect` / `POST /api/coding-eval/strategy-effects/evaluate`。
- targeted tests 覆盖候选质量下降与候选漏跑 baseline case 两类回归。

明确不包含：

- 默认不持久化 strategy effect report；纯函数路径仍是无副作用计算。
- `recordRun=true` 时 owner API 会写入 `coding_strategy_effect_runs` 并返回 `runId`。
- 不跑模型、不执行项目命令。
- 不替代 full benchmark；它比较的是两份 pack report 的质量变化。

后续已完成：

- Phase 5.8 已接入 release gate，让持久化 strategy effect history 参与发布质量阈值。

### Phase 5.5：Gold Task Pack 全量自动化（已完成）

目标：把 Phase 0 的 20 个 gold tasks 全部从文档草案收敛到 typed registry，形成可批量回放、可比较、可进入 strategy effect 的产品级评测集。

已落地：

- 20 个任务全部为 `active` + `automated`，summary 为 `totalCases=20`、`activeCases=20`、`automatedCases=20`。
- `GoldTaskAutomation` 支持 support files、extra file changes、per-case validation command、verification title、forbidden command、forbidden changed file 和 review finding 上限。
- former draft cases 覆盖 bugfix、test_gap、frontend_ts、rust_logic、review、repo_navigation；其中 TS / i18n case 支持多文件 diff，Rust case 支持 crate-local `cargo check` fixture。
- 全 pack 默认仍走 `fixture_patch`，不访问外部模型、不默认执行项目命令，保持 CI 级确定性。
- targeted tests 覆盖 former-draft case 回放和全 20 case pack 回放，确保 skipped / failed 为 0。

明确不包含：

- 不把 fixture_patch 结果冒充真实模型成功率；Phase 5.7 的 pack-level history 也必须通过 `baselineKind` 保持这条边界。

后续已完成：

- Phase 5.9 已补外部模型基线 runner，并保持 baseline kind 隔离。

### Phase 5.6：Mock Tool-call 基线与执行指标（已完成）

目标：让 agent execution runner 不只验证“模型能回文本”，还要验证真实 tool-call loop 能在隔离临时 repo 内写入候选 diff，并把工具调用事实进入 eval report。

已落地：

- `AgentExecutionEvalReport.toolCalls` 与 `FixtureReport.metrics.execution_tool_calls`，从真实 tool message 提取工具名。
- `checks.execution.expectedToolCalls` / `minToolCalls`，可断言模型至少调用了指定工具。
- 本地 mock OpenAI Responses SSE fixture：第一轮返回 `function_call(write, { path, content })`，第二轮返回最终文本，不访问外部服务。
- 端到端单测覆盖真实 `run_chat_engine`、tool dispatch、`write` 工具写入临时 repo、candidate diff snapshot 和 task-level scorer。
- `ChatEngineParams.session_db` 绑定到 `AssistantAgent`，agent-side session meta lookup 优先使用本轮 DB，避免 eval/headless 隔离 DB 的 working dir 被全局 DB 覆盖；incognito 缺行仍 fail-closed。
- coding eval 临时 DB 统一执行 `ChannelDB::migrate()`，保证 `get_session()` 的 metadata join 与生产 schema 一致。

明确不包含：

- 不把 mock provider 成功率冒充真实外部模型成功率；Phase 5.7 的 strategy effect history 也必须保留 baseline kind 边界。

后续已完成：

- Phase 5.8 已用持久化 history 补 release gate；Phase 5.9 已补外部模型基线 runner。

### Phase 5.7：Strategy Effect 趋势持久化 / Dashboard（已完成）

目标：把 pack / strategy report 从一次性响应升级为可审计质量闸，并在 Dashboard 中看见趋势。

已落地：

- 新增 `coding_eval_pack_runs`，持久化 `GoldTaskPackReport`、pack pass/fail/skipped/checks 汇总、`baselineKind`、label、source 和完整 report JSON。
- 新增 `coding_strategy_effect_runs`，持久化 `StrategyEffectReport`、verdict、共同 case 数、pass rate / task score / context recall / validation / scope creep / execution failure delta。
- `run_coding_eval_gold_task_pack` 支持 `recordPackRun`、`baselineKind`、`label`、`sessionId` / `projectId`，默认记录 pack history 并返回 `packRunId`。
- `evaluate_coding_eval_strategy_effect` 默认仍可做无副作用对比，`recordRun=true` 时写入 strategy effect history 并返回 `runId`。
- Dashboard Learning Tab 展示 Pack 成功率、Strategy Effect 数、Tool-call 缺失、latest strategy effects、pack / strategy 时间线和项目级 pack / strategy 信号。
- targeted tests 覆盖 pack / strategy history 写入、Dashboard 新字段聚合、tool-call missing failure mode 与 incognito 排除。

明确不包含：

- 不执行外部真实模型 benchmark；外部基线由 Phase 5.9 的 `executionMode="agent"` Gold Pack runner 显式运行并记录。
- 不隐式把 strategy effect verdict 变成发布阻断；发布阻断必须显式调用 Phase 5.8 release gate 并记录阈值。

### Phase 5.8：Release Gate（已完成）

目标：把持久化 pack / strategy / tool-call history 变成可自动化调用的发布质量门禁，避免“Dashboard 看得到趋势，但发布脚本无法判断能不能放行”。

已落地：

- 新增 `CodingEvalReleaseGateInput` / `CodingEvalReleaseGateReport`，输出 `passed` / `failed` / `insufficient_data` 三态结论。
- 新增 `evaluate_coding_eval_release_gate` Tauri command 与 `POST /api/coding-improvement/release-gate/evaluate` HTTP owner API。
- Gate 读取 `coding_eval_pack_runs`、`coding_strategy_effect_runs`、`coding_eval_runs(source_type='coding_task_eval')`，不跑模型、不执行项目命令、不写 DB。
- 默认阈值保守：`minPackRuns=1`、`minPackPassRate=1.0`、strategy regression / mixed / missing tool-call / validation delta / scope creep delta 全部默认 0 容忍。
- `requireExternalModelPack=true` 时要求窗口内存在 `baselineKind="external_model"` 的 pack run；deterministic / mock provider history 不会冒充真实 provider 基线。
- targeted tests 覆盖干净历史通过、策略/工具调用回归失败、外部真实模型基线缺失导致 `insufficient_data`。

明确不包含：

- 不主动执行外部真实模型 benchmark；它只消费已经记录到 history 的结果。
- 不替代 Dashboard 的学习视图；gate 是机器可读发布判断，Dashboard 是人可读趋势分析。

### Phase 5.9：外部模型基线 runner（已完成）

目标：让 Gold Task Pack 不只支持 deterministic fixture，也能在受控输入下调用真实 provider，从 task prompt 产出候选 diff，并把真实模型成功率与 mock / fixture 基线分开记录。

已落地：

- `GoldTaskPackRunInput` 新增 `executionMode`、`providers`、`modelChain`、`compactConfig`、`reasoningEffort`、`extraSystemContext`、`deniedTools`、`autoApproveTools`。
- `executionMode="agent"` 时，每个 gold task materialize 成 agent execution fixture：prompt 来自 task prompt，模型必须通过真实工具调用修改临时 repo，再进入 Review / Smart Verification / Context Retrieval / Task scorer。
- `executionMode` 默认仍是 `fixture_patch`；只有传 provider/model 或 `baselineKind="external_model" | "mock_provider"` 时才进入 agent 路径。
- `baselineKind="external_model"` / `mock_provider` 必须配 agent execution；`agent` 不能记录成 `deterministic_mock`，防止只改标签伪装基线。
- owner API / Transport 沿用 `run_coding_eval_gold_task_pack`，无需新端点；报告仍写入 `coding_eval_pack_runs`，Release Gate 可消费 `external_model` pack run。
- targeted tests 使用本地 mock Responses provider 覆盖完整 agent pack runner、真实 `write` tool-call、外部基线 history 记录和 fail-fast 校验。

明确不包含：

- 不在 CI 默认访问外部网络或真实 provider；真实外部 smoke 由调用方显式传 provider/model 后运行。
- 不保证所有 20 个 gold tasks 在真实模型上立刻通过；该阶段交付的是可审计 runner 和基线记录边界。

## 30 天首个里程碑

2026-07-01 之后的首个里程碑不再是 ToolDefinition / workflow runtime foundation，它们已经进入 Phase 1 / Phase 2 已完成范围。新的 30 天目标是把控制平面补到可承载长任务：

1. 已落 `/goal` 第一版：objective、completion criteria、state、budget 字段、evidence、final audit。
2. 已在 GUI 中展示 active goal，不要求用户记 slash 命令才能掌控长期任务。
3. 已让 workflow run 可选绑定 goal，repair run 不丢 goal 归属。
4. 已让 workflow completion / validation / task evidence 回写 goal audit；validation / diff / file evidence 第一层结构化 link 已落地，Review Engine evidence 与 Smart Verification evidence 已落地，artifact/diagnostic 接入后续补。
5. 已做第一版 goal evaluator，能输出 completed / blocked + reason。
6. 已更新 Coding Eval：Phase 3.7 验证 review / verification / context / goal / task / workflow 协同，Phase 5.1 增加 task-level scorer，Phase 5.2 增加 agent execution runner，Phase 5.3 增加 Gold Task Pack v1 批量回放入口，Phase 5.4 增加 strategy effect evaluator，Phase 5.5 增加 20 个 gold tasks 全量自动化，Phase 5.6 增加 mock tool-call 写文件基线与 `toolCalls` 指标，Phase 5.7 增加 pack / strategy history Dashboard，Phase 5.8 增加 release gate，Phase 5.9 增加外部模型基线 runner。
7. `/loop` 第一版已落地；后续增强放到 Phase 3+ 或独立 RFC。

## 验收指标

短期验收：

- coding eval 成功率有基线和趋势。
- 控制面 eval 至少覆盖 focused review、focused verification、goal/task/workflow context recall。
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
11. [Coding Eval 控制面评测](../architecture/coding-eval.md)：Phase 3.7 deterministic fixture harness、context precision / critical recall、控制面回归、Phase 5.1 task-level eval runner、Phase 5.2 agent execution runner、Phase 5.3 Gold Task Pack v1、Phase 5.4 strategy effect evaluator、Phase 5.5 Gold Task Pack 全量自动化、Phase 5.6 mock tool-call 基线、Phase 5.7 strategy effect 趋势持久化、Phase 5.8 release gate 与 Phase 5.9 外部模型基线 runner。
12. [Coding Improvement Loop](../architecture/coding-improvement-loop.md)：已落地 trend report、failure taxonomy、proposal 队列、proposal-to-action、workflow retro、draft promotion、Dashboard 全局学习视图、Transcript Distillation、Failure Feedback、pack / strategy history Dashboard、release gate 与 external model baseline；后续继续设计跨项目学习泛化。

这些文档完成后，再进入逐项实现。实现顺序应优先保证可评测、可回滚、可审计，而不是先堆最显眼的 UI。
