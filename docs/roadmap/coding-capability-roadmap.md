# Hope Agent Coding 能力强化总纲

> 返回 [文档索引](../README.md)
>
> 更新时间：2026-06-29

## 目录

- [文档定位](#文档定位)
- [背景](#背景)
- [北极星目标](#北极星目标)
- [参考资料与调研线索](#参考资料与调研线索)
- [现状判断](#现状判断)
- [能力模型](#能力模型)
- [Dynamic Workflow 与 Loop 模式](#dynamic-workflow-与-loop-模式)
- [阶段计划](#阶段计划)
- [30 天首个里程碑](#30-天首个里程碑)
- [验收指标](#验收指标)
- [红线与非目标](#红线与非目标)
- [后续设计文档清单](#后续设计文档清单)

## 文档定位

本文是 Hope Agent 下一阶段 Coding 能力建设的总纲，用来沉淀背景、目标、调研线索、参考资料和整体路线。它不是某个具体子系统的最终设计，也不直接定义数据库 schema、API 细节或 UI 交互。

后续每个大项应先拆成 `docs/roadmap/` 下的方案或 RFC，例如 `workflow`、`ToolDefinition v2`、`managed worktree`、`LSP`、`review engine`、`coding eval`。实现完成并成为稳定技术事实后，再沉淀到 `docs/architecture/` 的最终架构文档。本文只负责回答：

1. 为什么要做。
2. 参考了什么。
3. Hope 当前有什么优势和缺口。
4. 应该按什么顺序补齐。
5. 哪些边界不能碰。

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
- 缺少 managed worktree 创建、恢复、归档、交接。
- 缺少 LSP 语义代码工具和被动 diagnostics 注入。
- 缺少独立 `/review` engine 和 verifier 三态确认。
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

## Dynamic Workflow 与 Loop 模式

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
  loop_policy
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

LoopPolicy
  max_iterations
  max_repair_attempts
  max_minutes
  max_cost
  no_progress_threshold
  validation_required
  human_gate_points
```

### 五类 loop

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
/loop off
/loop guarded
/loop deep
/loop autonomous
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

状态：详细方案见 [Phase 2 Coding Mode 与 Script-first Dynamic Workflow 方案](phase2-coding-mode-dynamic-workflow.md)。

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

### Phase 2.5：Script-first Dynamic Workflow + Loop Engine

状态：详细方案见 [Phase 2 Coding Mode 与 Script-first Dynamic Workflow 方案](phase2-coding-mode-dynamic-workflow.md)。

目标：让 coding 任务能通过“先写脚本、审批后执行”的动态 workflow，在预算和护栏内稳定闭环。

任务：

- 新增 durable `WorkflowRun` / `WorkflowOp` / `WorkflowEvent`，以 replay 而不是 VM snapshot 恢复长任务。
- 新增受控 `workflow.js` runtime；脚本只能调用 host API，不能直接访问 raw fs/network/process/env。
- MVP 先支持 `coding.fix_bug`、`coding.feature`、`coding.review`、`coding.debug`。
- host API 先覆盖 `tool`、`fileSearch`、`read`、`grep`、`spawnAgent`、`waitAll`、`task.create/update`、`validate`、`askUser`、`trace`、`diff`、`finish`。
- 脚本执行前做 lint / budget / permission preview / user approval。
- `task_create/update` 与 workflow op 自动绑定。
- validation 失败自动生成 structured feedback，作为下一轮 repair 输入。
- 增加 `/workflow`、`/workflow trace`、`/workflow pause|resume|cancel`、`/loop` 控制面。

产物：

- script-first workflow runtime RFC。
- workflow trace viewer。
- loop policy 配置。

### Phase 3：Managed Worktree 隔离与交接

目标：并行写代码不污染用户当前工作区。

任务：

- 实现 worktree manager：create、list、archive、restore、handoff。
- 支持 `.worktreeinclude`，复制必要 ignored setup。
- 记录 base branch、dirty state、diff snapshot。
- background subagent 或 parallel implementation 默认进入 worktree。
- 激活 `WorktreeCreate` / `WorktreeRemove` hooks。
- UI 提供 Local / Worktree 切换、diff、restore、handoff。

产物：

- `managed-worktree` 架构文档。
- worktree owner API。
- worktree UI panel。

### Phase 4：LSP 与语义代码智能

目标：让 Hope 不只会 grep，还能理解符号、引用和诊断。

任务：

- 新增 LSP manager。
- 支持 `definition`、`references`、`hover`、`document_symbols`、`workspace_symbols`、`implementation`、`call_hierarchy`、`diagnostics`。
- 编辑后同步 `didChange` / `didSave`。
- diagnostics 被动注入下一轮。
- ACP/IDE 场景注入 open files、selection、diagnostics、symbols。

产物：

- `lsp` 架构文档。
- LSP tools。
- diagnostics attachment pipeline。

### Phase 5：Review 与 Verification Engine

目标：把 review 从“提示词建议”升级为独立系统。

任务：

- 新增 `/review` 能力，支持 uncommitted diff、base branch、commit range。
- Diff scan 读取 hunk 和 enclosing function。
- 生成 candidate findings。
- 去重后交给 verifier agent，输出 `CONFIRMED`、`PLAUSIBLE`、`REFUTED`。
- 支持 review profiles：correctness、security、concurrency、frontend、accessibility、tests。
- 支持 inline finding、可选 auto-fix、fix 后 re-review。
- Verification executor 根据 AGENTS、任务类型、改动文件选择最小相关检查。

产物：

- `review-engine` 架构文档。
- `/review --local` MVP。
- verifier prompt 与 result schema。

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

优先做投入小、收益大的基础设施：

1. 建 `coding eval harness`，先放 20 个 gold tasks。
2. 写 `ToolDefinition v2` RFC，迁移 5-8 个核心工具。
3. 升级 `tool_search`：alias、search_hint、BM25、`select:`。
4. 做 `/review --local` MVP：uncommitted diff、candidate finding、verifier 三态。
5. 给 Plan Mode 加 plan quality gate。
6. 修订内置 `code-review` skill，使其遵守项目 AGENTS 的验证策略。
7. 设计 `WorkflowRun` trace schema，但先不急着做复杂 UI。

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

1. [docs/roadmap/coding-eval.md](coding-eval.md)：评测集、指标、trace、报告格式。
2. `docs/roadmap/tool-definition-v2.md`：工具元数据、迁移策略、兼容层。
3. `docs/roadmap/tool-search-v2.md`：搜索排序、schema 返回、deferred 策略。
4. `docs/roadmap/coding-mode.md`：任务分类、profile、Plan quality gate、Skill policy。
5. `docs/roadmap/workflow.md`：WorkflowRun、节点、边、loop policy、trace。
6. `docs/roadmap/managed-worktree.md`：隔离工作区、handoff、UI、hooks。
7. `docs/roadmap/lsp.md`：LSP manager、tools、diagnostics pipeline。
8. `docs/roadmap/review-engine.md`：diff scan、candidate、verifier、inline finding。
9. `docs/roadmap/coding-improvement-loop.md`：retro、eval candidate、skill/guidance distillation。

这些文档完成后，再进入逐项实现。实现顺序应优先保证可评测、可回滚、可审计，而不是先堆最显眼的 UI。
