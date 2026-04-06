# Plan Mode 对比分析：OpenComputer vs Claude Code vs OpenClaw

> 基线对比时间：2026-04-05 | 对应主文档章节：2.4

---

## 一、架构总览

| 维度 | OpenComputer | Claude Code | OpenClaw |
|------|-------------|-------------|----------|
| **实现语言** | Rust (后端状态机 + 工具) | TypeScript (工具 + 权限系统 + 附件注入) | 不支持 |
| **状态模型** | 六态状态机 (Off/Planning/Review/Executing/Paused/Completed) | 二态切换 (default ↔ plan) + 权限模式恢复 | N/A |
| **Agent 架构** | 双 Agent 分离 (PlanAgent + BuildAgent) | 单 Agent + 多子 Agent 并行探索 | N/A |
| **计划文件** | Markdown (`~/.opencomputer/plans/`) + 版本管理 | Markdown (`~/.claude/plans/` 或自定义) + word-slug 命名 | N/A |
| **执行追踪** | PlanStep 逐步追踪 (5 状态 + 耗时) | 无内置步骤追踪 (依赖 TodoWrite) | N/A |
| **Git 安全** | 自动创建 checkpoint 分支 + 一键 rollback | 无内置 (依赖用户手动 git) | N/A |
| **交互式问答** | plan_question 工具 (结构化选项卡 UI) | AskUserQuestion 通用工具 | N/A |
| **远程执行** | 不支持 | Ultraplan (CCR 远程会话) | N/A |

---

## 二、OpenComputer 实现

### 2.1 六态状态机

OpenComputer 的 Plan Mode 采用严格的六态有限状态机，定义在 `src-tauri/src/plan/types.rs`:

```
Off → Planning → Review → Executing → Completed
                              ↓  ↑
                            Paused
```

**状态定义：**

| 状态 | 含义 | 可用操作 |
|------|------|----------|
| `Off` | Plan Mode 未激活 | 正常对话 |
| `Planning` | 正在创建计划 | 只读探索 + plan_question + submit_plan |
| `Review` | 计划已提交，等待用户审阅 | 用户可批准/修改/退出 |
| `Executing` | 正在执行已批准的计划 | 全量工具 + update_plan_step + amend_plan |
| `Paused` | 执行暂停 | 记录 `paused_at_step`，可恢复到 Executing |
| `Completed` | 计划全部执行完毕 | 展示结果摘要 + 建议后续操作 |

状态存储在全局 `PLAN_STORE`（`OnceLock<Arc<RwLock<HashMap<String, PlanMeta>>>>`），按 session_id 隔离。状态变更通过 `set_plan_state()` 统一管理，自动处理 paused_at_step 的记录与清除。

**崩溃恢复：** 步骤状态持久化到 SQLite (`persist_steps_to_db`)，会话恢复时优先从 DB 加载，fallback 到重新解析 Markdown 文件。

### 2.2 双 Agent 分离（PlanAgent + BuildAgent）

OpenComputer 将规划与执行拆分为两个独立的 Agent 角色：

**PlanAgent（Planning/Review 阶段）：**
- 采用 **允许列表** 机制，只有白名单工具可用
- 允许的工具：`read`, `ls`, `grep`, `find`, `glob`, `web_search`, `web_fetch`, `exec`(需审批), `plan_question`, `submit_plan`, `write`(仅 plans/ 目录), `edit`(仅 plans/ 目录), `recall_memory`, `memory_get`, `subagent`
- `write`/`edit` 工具受路径限制：只能操作 `~/.opencomputer/plans/` 下的 `.md` 文件
- `exec` 工具需要用户审批
- 配置定义在 `PlanAgentConfig::default_config()`

**BuildAgent（Executing/Paused 阶段）：**
- 拥有全量工具权限
- 额外注入 `update_plan_step` 和 `amend_plan` 两个执行专用工具
- 系统提示词包含完整的计划内容 + 步骤追踪指令

**子 Agent 规划模式：**
通过 `spawn_plan_subagent()` 可将 Planning 阶段委托给子 Agent 执行。子 Agent 继承 PlanAgent 的工具限制，但系统提示词额外注入 `PLAN_SUBAGENT_CONTEXT_NOTICE`，强调计划必须自包含（因为执行 Agent 看不到探索历史）。超时时间 1 小时（因 plan_question 可等待用户 10 分钟）。

### 2.3 步骤追踪（PlanStep）

每个计划步骤用 `PlanStep` 结构追踪：

```rust
pub struct PlanStep {
    pub index: usize,
    pub phase: String,       // 所属阶段名称
    pub title: String,       // 步骤标题
    pub description: String, // 步骤描述
    pub status: PlanStepStatus, // Pending/InProgress/Completed/Skipped/Failed
    pub duration_ms: Option<u64>, // 执行耗时
}
```

**步骤来源：** 从计划 Markdown 文件解析而来（`parse_plan_steps`），识别 `### Phase N: title` 作为阶段标题，`- [ ]`/`- [x]` 作为可追踪步骤。

**执行期间动态修改：** BuildAgent 可通过 `amend_plan` 工具在执行中修改计划：
- `insert`：在指定步骤后插入新步骤
- `delete`：删除未执行的步骤
- `update`：修改步骤标题/描述

### 2.4 Git Checkpoint

执行前自动创建 git checkpoint，实现在 `src-tauri/src/plan/git.rs`：

1. **创建**：`create_git_checkpoint()` 在当前 HEAD 创建分支 `opencomputer/checkpoint-{short_id}-{timestamp}`，不切换分支
2. **存储**：checkpoint 引用保存在 `PlanMeta.checkpoint_ref`
3. **回滚**：`rollback_to_checkpoint()` 执行 `git reset --hard <checkpoint_branch>`，然后删除 checkpoint 分支
4. **清理**：执行成功后 `cleanup_checkpoint()` 删除 checkpoint 分支

自动检测 git 仓库根目录（`git rev-parse --show-toplevel`），非 git 项目静默跳过。

### 2.5 交互式问答

Planning 阶段通过 `plan_question` 工具发送结构化问题，前端渲染为交互式 UI 卡片：

```rust
pub struct PlanQuestion {
    pub question_id: String,
    pub text: String,
    pub options: Vec<PlanQuestionOption>,  // 2-5 个建议选项
    pub allow_custom: bool,   // 允许自定义输入
    pub multi_select: bool,   // 多选模式
    pub template: Option<String>, // UI 样式模板: "scope"/"tech_choice"/"priority"
}
```

每个选项支持 `recommended: true` 标记（渲染为星标推荐）。问题分组发送（`PlanQuestionGroup`），通过 oneshot channel 等待用户响应。

### 2.6 暂停/恢复

- 进入 Paused 状态时，自动记录 `paused_at_step`（第一个 InProgress 或第一个 Pending 步骤的 index）
- 恢复到 Executing 时，清除 `paused_at_step`
- 前端可展示暂停位置的精确进度

### 2.7 计划文件管理

**存储路径：** `~/.opencomputer/plans/plan-{short_id}-{date}.md`

**版本管理：**
- 每次保存自动备份旧版本为 `plan-{name}-v{N}.md`
- `PlanMeta.version` 计数器递增
- `list_plan_versions()` 列出所有历史版本（含修改时间）
- `load_plan_version()` 可加载任意历史版本

**执行结果：** 完成后生成 `result-{short_id}-{date}.md`，包含每步执行状态、耗时统计和总结。

**5 阶段工作流：**
1. Deep Exploration — 使用子 Agent 并行探索代码库
2. Requirements Clarification — 通过 plan_question 向用户提问
3. Design & Architecture — 设计方案并权衡取舍
4. Plan Composition — 通过 submit_plan 提交计划
5. Review & Refinement — 用户审阅，支持内联注释修改

---

## 三、Claude Code 实现

### 3.1 Enter/Exit 二态模型

Claude Code 的 Plan Mode 基于**权限模式切换**，核心是两个工具：

**EnterPlanMode：**
- 无参数，调用即切换权限模式从 `default`/`auto` 到 `plan`
- 需要用户批准（显示确认对话框）
- 切换后保存 `prePlanMode` 以便退出时恢复
- 在 `--channels` 模式（Telegram/Discord）下自动禁用（避免用户不在终端时卡死）
- Agent 上下文中禁止使用

**ExitPlanMode (V2)：**
- 读取磁盘上的计划文件内容，展示给用户审批
- 支持用户在审批对话框中编辑计划
- 退出后恢复到 `prePlanMode`，处理 auto mode 的 circuit breaker 和权限恢复
- 支持 `allowedPrompts` 参数请求语义化权限（如 "run tests"）
- Teammate 模式下走 mailbox 审批流（发送给 team-lead）

**触发条件（外部用户版）：** 积极触发，包括新功能实现、多种可行方案、代码修改、架构决策、多文件变更、需求不明确、用户偏好相关等场景。

**触发条件（Anthropic 内部版）：** 更保守，仅在真正存在架构歧义、需求不清、高影响重构时触发。

### 3.2 Plan 文件约定

**文件路径：** `~/.claude/plans/{word-slug}.md`，支持通过 `settings.json` 的 `plansDirectory` 自定义路径（相对项目根目录）。

**命名策略：** 使用 word-slug 生成器产生可读文件名（非 UUID），冲突时最多重试 10 次。子 Agent 使用 `{slug}-agent-{agentId}.md`。

**会话恢复：** `copyPlanForResume()` 支持三级恢复：
1. 磁盘文件直接读取
2. file_snapshot 系统消息（CCR 远程会话增量快照）
3. 消息历史中提取（ExitPlanMode 工具输入 / planContent 字段 / plan_file_reference 附件）

**会话 Fork：** `copyPlanForFork()` 为 Fork 会话生成新 slug 并复制计划文件，避免原会话和 Fork 会话互相覆盖。

### 3.3 权限模式切换

Plan Mode 激活后的权限约束通过**附件注入**（attachment）实现，而非工具层面的硬性拦截：

**系统消息注入内容：**
- 明确声明 "MUST NOT make any edits"（除计划文件外）
- 只允许只读操作
- 注入完整的工作流指导

**两种工作流模式：**

**5-Phase 工作流（默认）：**
1. Initial Understanding — 使用 explore 子 Agent 并行探索（最多 3 个）
2. Design — 使用 plan 子 Agent 设计方案（Max 订阅最多 3 个，普通 1 个）
3. Review — 阅读关键文件 + 使用 AskUserQuestion 澄清
4. Final Plan — 将计划写入文件（有 pewter_ledger 实验控制计划长度）
5. Call ExitPlanMode — 提交计划等待审批

**Interview 工作流（Anthropic 内部 + 实验组）：**
- 迭代式循环：Explore → Update plan file → Ask user → 重复
- 不强制使用子 Agent
- 更注重用户交互
- 计划文件增量更新而非一次性生成

**重入支持：** 用户在 plan mode 中继续对话时，注入精简版指令（`getPlanModeV2SparseInstructions`），避免上下文膨胀。

### 3.4 Plan 验证（VerifyPlanExecution）

VerifyPlanExecution 是一个**实验性工具**，通过环境变量 `CLAUDE_CODE_VERIFY_PLAN=true` 启用（外部构建中编译时消除）。

**触发机制：**
- 计划执行完成后，通过 `verify_plan_reminder` 附件注入提醒消息
- 提示模型调用 VerifyPlanExecution 工具（不能通过 Agent 工具间接调用）
- 用于后台验证所有计划项是否正确完成

**当前状态：** 实验阶段，默认未启用，主要用于 Anthropic 内部测试。

### 3.5 Ultra Plan

Ultraplan 是 Claude Code 的**远程增强规划**功能（`/ultraplan` 命令），将规划阶段委托给 Claude Code on the Web (CCR) 远程会话执行：

**工作流程：**
1. 本地 CLI 检查用户资质（`checkRemoteAgentEligibility`）
2. 通过 `teleportToRemote()` 创建远程 CCR 会话
3. 注入用户请求 + 规划指令（`buildUltraplanPrompt`）
4. 本地显示状态指示器（pill），30 分钟超时
5. 远程会话完成规划后，轮询检测 ExitPlanMode 调用（`pollForApprovedExitPlanMode`）
6. 用户可选择本地执行（teleport 回计划）或远程执行（CCR 直接编码，结果通过 PR 落地）

**模型选择：** 远程会话固定使用 Opus 4.6（通过 GrowthBook 特征开关 `tengu_ultraplan_model` 控制）。

**支持 seed plan：** 从 ExitPlanMode 审批对话框点击 "Ultraplan" 按钮时，已有的草案计划作为 seedPlan 传递给远程会话进行优化。

**任务管理：** 注册为 `RemoteAgentTask`，支持状态追踪（running/needs_input/completed/failed）、后台运行通知、停止（`stopUltraplan`）等。

---

## 四、OpenClaw 实现

经过对 OpenClaw 代码库（`~/Codes/openclaw/`）的全面搜索，**OpenClaw 不包含 Plan Mode 功能**。

代码库中出现的 "plan" 关键词均与 Secrets Management 系统相关（`src/secrets/plan.ts` — 密钥配置的目标路径规划），与 LLM 交互式规划无关。

OpenClaw 的定位是通道管理和运行时基础设施，没有内置 LLM 对话规划、工具权限切换或计划文件管理等功能。

---

## 五、逐项功能对比

| 功能 | OpenComputer | Claude Code | OpenClaw |
|------|:----------:|:-----------:|:--------:|
| **状态机复杂度** | 六态 | 二态 | N/A |
| **规划/执行分离** | 双 Agent 硬隔离 | 权限模式软约束 | N/A |
| **工具白名单** | 编译期允许列表 | 系统提示词约束 | N/A |
| **路径写入限制** | 代码级强制 (`is_plan_mode_path_allowed`) | 提示词约束 | N/A |
| **结构化问答** | plan_question 专用工具 + 选项卡 UI | AskUserQuestion 通用工具 | N/A |
| **计划文件版本管理** | 自动备份历史版本 | 无版本管理 | N/A |
| **步骤追踪** | PlanStep 5 状态 + 耗时 | 无 (依赖 TodoWrite) | N/A |
| **执行中修改计划** | amend_plan 工具 (insert/delete/update) | 无 | N/A |
| **暂停/恢复** | Paused 状态 + 断点记录 | 无 | N/A |
| **Git Checkpoint** | 自动创建 + 一键 rollback | 无 | N/A |
| **执行结果报告** | 自动生成 result.md | 无 | N/A |
| **崩溃恢复** | SQLite 持久化步骤状态 | 消息历史 + file_snapshot 恢复 | N/A |
| **并行探索** | 子 Agent 并行 (≤3) | explore Agent 并行 (≤3) | N/A |
| **并行设计** | 不支持 | plan Agent 并行 (Max 订阅 ≤3) | N/A |
| **远程规划** | 不支持 | Ultraplan (CCR) | N/A |
| **计划文件编辑** | 用户内联注释修改 | 审批时直接编辑 + /plan 命令 | N/A |
| **执行后验证** | 结果文件 + 完成提示词 | VerifyPlanExecution (实验) | N/A |
| **团队协作** | 不支持 | Teammate mailbox 审批 | N/A |
| **权限恢复** | 状态机归零 (Off) | prePlanMode 精确恢复 + auto mode circuit breaker | N/A |
| **会话 Fork 支持** | 不支持 | copyPlanForFork 独立计划文件 | N/A |
| **计划长度控制** | 提示词建议 | A/B 实验 (pewter_ledger: trim/cut/cap) | N/A |
| **Interview 模式** | plan_question 迭代 | 独立 interview 工作流 | N/A |

---

## 六、差距分析与建议

### OpenComputer 的优势

1. **执行可靠性强**
   - 六态状态机提供精确的生命周期管理
   - Git checkpoint 自动保护，执行失败可一键回滚
   - 步骤级追踪 + 耗时记录，进度可视化清晰
   - SQLite 持久化支持崩溃恢复

2. **工具隔离更严格**
   - PlanAgent 白名单在代码层面硬性限制，不依赖 LLM 遵守提示词
   - 路径写入限制通过 `is_plan_mode_path_allowed()` 函数检查，非提示词软约束

3. **交互设计更丰富**
   - plan_question 支持结构化选项、推荐标记、多选、自定义输入
   - 暂停/恢复机制适合长时间执行的计划

4. **执行中的灵活性**
   - amend_plan 工具允许在执行中动态调整计划
   - 完整的版本管理系统保留所有修改历史

### OpenComputer 的不足与建议

1. **缺少远程规划能力**
   - Claude Code 的 Ultraplan 可将探索阶段委托给远程高性能会话
   - 建议：评估是否需要云端规划场景，如需要可对接 ACP 协议实现远程 Agent 委托

2. **缺少团队协作审批**
   - Claude Code 支持 Teammate 模式下通过 mailbox 机制进行 team-lead 审批
   - 建议：如 IM Channel 场景需要，可扩展 plan 审批到多人协作流程

3. **缺少并行设计阶段**
   - Claude Code 的 Phase 2 可并行启动多个 plan Agent 从不同角度设计方案
   - 建议：利用现有 subagent 系统扩展，在 Planning 阶段支持多视角并行设计

4. **计划长度缺乏数据驱动优化**
   - Claude Code 通过 A/B 实验（pewter_ledger）量化不同计划长度对成本和拒绝率的影响
   - 建议：收集用户使用数据，分析计划详细度与执行成功率的关系

5. **会话 Fork 未支持**
   - Claude Code 的 Fork 场景下会自动复制计划文件到新 slug，避免文件冲突
   - 建议：如未来支持会话 Fork，需同步处理 plan 文件的隔离

6. **审批时计划编辑**
   - Claude Code 允许用户在审批对话框中直接编辑计划内容
   - OpenComputer 当前支持内联注释修改，但需要 LLM 重新理解并提交
   - 建议：考虑支持用户直接编辑计划 Markdown 并生效

### Claude Code 可借鉴 OpenComputer 的点

1. **步骤级执行追踪** — Claude Code 缺乏对计划步骤的结构化追踪，依赖 TodoWrite 工具模拟
2. **Git 安全网** — 自动 checkpoint 显著降低了执行失败的风险
3. **版本管理** — 计划文件的自动备份和历史对比是重要的可追溯性保障
4. **工具硬隔离** — 代码层面的白名单比提示词约束更可靠
5. **暂停/恢复** — 长时间执行的计划需要断点续做能力
