# OpenComputer Plan Mode 架构文档

> 更新时间：2026-03-28

## 目录

- [概述](#概述)（含系统架构总览图）
- [核心概念](#核心概念)
- [六态状态机](#六态状态机)（含状态流转图）
- [后端架构](#后端架构)
  - [Plan 状态管理](#plan-状态管理)（含数据结构）
  - [工具限制与路径权限](#工具限制与路径权限)（含过滤流程图）
  - [系统提示词注入](#系统提示词注入)（5 阶段规划流程）
  - [Plan 文件持久化](#plan-文件持久化)（项目本地化存储）
  - [Markdown Checklist 解析](#markdown-checklist-解析)
  - [交互式问答（plan_question）](#交互式问答plan_question)
  - [计划提交（submit_plan）](#计划提交submit_plan)
  - [步骤进度追踪（update_plan_step）](#步骤进度追踪update_plan_step)（含事件流图）
  - [执行中计划修改（amend_plan）](#执行中计划修改amend_plan)
  - [Git Checkpoint 回滚](#git-checkpoint-回滚)
  - [计划版本管理](#计划版本管理)
  - [Plan/Build 独立模型](#planbuild-独立模型)
  - [子 Agent 安全继承](#子-agent-安全继承)
- [斜杠命令 /plan](#斜杠命令-plan)（含命令分发图）
- [Tauri 命令一览](#tauri-命令一览)
- [前端架构](#前端架构)
  - [usePlanMode Hook](#useplanmode-hook)（含状态流图）
  - [ChatInput 集成](#chatinput-集成)（含 UI 示意图）
  - [PlanCardBlock 摘要卡片](#plancardblock-摘要卡片)
  - [PlanQuestionBlock 交互问答](#planquestionblock-交互问答)
  - [PlanPanel 右侧面板](#planpanel-右侧面板)（含 UI 示意图）
  - [planParser 解析器](#planparser-解析器)
- [完整交互流程](#完整交互流程)（含全链路时序图）
- [DB Schema](#db-schema)
- [事件系统](#事件系统)
- [安全设计](#安全设计)
- [与 Claude Code / OpenCode 对比](#与-claude-code--opencode-对比)
- [文件清单](#文件清单)

---

## 概述

Plan Mode 是 OpenComputer 的可视化交互计划模式，实现六态状态机驱动的完整规划-审查-执行工作流。在 Planning 阶段，LLM 通过 5 阶段规划流程（深度探索→需求澄清→架构设计→计划编写→审查优化）充分分析需求，支持子 Agent 并行探索代码库和结构化交互问答。用户可在 Review 阶段审查计划、请求修改、浏览版本历史。批准后进入 Executing 阶段，实时追踪步骤进度，支持暂停/恢复、执行中修改计划、Git Checkpoint 一键回滚。

**核心设计原则：**

1. **零侵入集成**：复用现有 `denied_tools` + `extra_system_context` + 条件工具注入机制，不改变核心 chat 流程
2. **可视化优先**：ChatInput 工具栏按钮（主入口）+ `/plan` 斜杠命令（辅助入口），PlanCardBlock 内嵌卡片 + PlanPanel 右侧面板双视图
3. **交互式规划**：`plan_question` 结构化问答 + `submit_plan` 计划提交，LLM 与用户迭代讨论
4. **实时追踪**：`update_plan_step` + `amend_plan` 工具 + Tauri 全局事件驱动前端 UI 实时更新
5. **安全可靠**：细粒度路径权限 + 子 Agent 限制继承 + Git Checkpoint 回滚 + 步骤进度 DB 持久化
6. **会话级隔离**：Plan Mode 状态绑定到 session，不影响其他会话

### 系统架构总览

```mermaid
graph TB
    subgraph 前端
        CI[ChatInput<br/>Plan 按钮 五色状态]
        PCC[PlanCardBlock<br/>摘要卡片 可展开]
        PQB[PlanQuestionBlock<br/>交互问答 UI]
        PP[PlanPanel<br/>右侧面板 四种视图]
        UPM[usePlanMode Hook<br/>六态状态管理]
        CS[ChatScreen<br/>集成层]
    end

    subgraph 后端
        PM[plan.rs<br/>六态状态机 + 文件 I/O<br/>+ Git Checkpoint + 版本管理]
        SC[slash_commands<br/>/plan 命令 6 子命令]
        CP[commands/plan.rs<br/>11 个 Tauri 命令]
        CH[commands/chat.rs<br/>工具限制 + 路径权限<br/>+ 模型覆盖 + 提示词注入]
        TQ[tools/plan_question.rs<br/>交互问答]
        TS[tools/submit_plan.rs<br/>计划提交]
        TP[tools/plan_step.rs<br/>步骤进度更新]
        TA[tools/amend_plan.rs<br/>执行中修改]
        SA[subagent/spawn.rs<br/>限制继承]
        DB[(SessionDB<br/>plan_mode + plan_steps)]
        FS[(文件系统<br/>.opencomputer/plans/)]
    end

    CI -->|enterPlanMode| UPM
    UPM -->|invoke| CP
    CP -->|set_plan_state| PM
    PM -->|persist| DB
    PM -->|save| FS

    CS -->|/plan| SC
    SC -->|handle_plan| PM

    CH -->|get_plan_state| PM
    CH -->|denied_tools + path_allow| TQ
    CH -->|plan_executing| TP

    TQ -->|emit| PQB
    TS -->|emit| PCC
    TP -->|emit| PP
    TA -->|emit| PP

    SA -->|inherit denied_tools| PM

    UPM -->|listen events| PP
    UPM -->|listen events| PCC
    UPM -->|listen events| PQB
```

---

## 核心概念

| 概念 | 说明 |
|------|------|
| **PlanModeState** | 六态枚举：Off / Planning / Review / Executing / Paused / Completed |
| **PlanStep** | 计划中的单个步骤，包含 index、phase、title、description、status、duration |
| **PlanMeta** | 计划元数据，包含 state、steps、file_path、version、checkpoint_ref、paused_at_step 等 |
| **PlanQuestion** | 结构化问答，包含选项、多选、自定义输入、recommended 标记、template 分类 |
| **plan_question** | Planning 阶段的交互问答工具，LLM 发起结构化提问，前端渲染可视化卡片 |
| **submit_plan** | Planning 阶段的计划提交工具，触发 Planning→Review 状态转换 |
| **update_plan_step** | Executing 阶段的步骤进度工具，LLM 实时报告执行进度 |
| **amend_plan** | Executing/Paused 阶段的计划修改工具，支持插入/删除/更新步骤 |
| **Plan File** | Markdown 格式的计划文件，项目本地存储于 `.opencomputer/plans/` |
| **Git Checkpoint** | 执行前自动创建 git 分支快照，失败时可一键回滚 |

---

## 六态状态机

Plan Mode 有六个状态，按 session 隔离：

```mermaid
stateDiagram-v2
    [*] --> Off

    Off --> Planning : 点击 Plan 按钮 / /plan enter
    Planning --> Off : /plan exit / 点击退出
    Planning --> Review : LLM 调用 submit_plan

    Review --> Planning : 用户"请求修改"（反馈→LLM修订）
    Review --> Executing : 用户"批准并执行"（创建 Git Checkpoint）
    Review --> Off : 用户"退出"

    Executing --> Paused : 用户暂停
    Executing --> Completed : 所有步骤完成（自动转换）

    Paused --> Executing : 用户恢复
    Paused --> Off : 用户退出（可回滚）

    Completed --> Off : 用户退出（清理 Checkpoint）

    state Off {
        [*] : 正常模式<br/>所有工具可用
    }
    state Planning {
        [*] : 只读 + 规划模式<br/>write/edit 仅允许 plans/ 路径<br/>apply_patch/canvas 禁用<br/>exec 需审批<br/>注入 plan_question + submit_plan 工具<br/>5 阶段规划提示词<br/>可配置独立 planModel
    }
    state Review {
        [*] : 审查模式<br/>工具限制同 Planning<br/>PlanPanel 只读渲染<br/>审批/请求修改/退出/版本历史
    }
    state Executing {
        [*] : 执行模式<br/>所有工具恢复可用<br/>注入 update_plan_step + amend_plan<br/>注入计划内容到提示词<br/>Git Checkpoint 已创建
    }
    state Paused {
        [*] : 暂停模式<br/>记录 paused_at_step<br/>可恢复执行或回滚
    }
    state Completed {
        [*] : 完成模式<br/>注入完成提示词<br/>LLM 总结结果 + 建议后续
    }
```

### 各状态的行为差异

| 维度 | Off | Planning | Review | Executing | Paused | Completed |
|------|-----|----------|--------|-----------|--------|-----------|
| write/edit | 可用 | **仅 plans/ 路径** | **仅 plans/ 路径** | 可用 | 可用 | 可用 |
| apply_patch/canvas | 可用 | **禁用** | **禁用** | 可用 | 可用 | 可用 |
| exec 工具 | 按权限 | **强制审批** | **强制审批** | 按权限 | 按权限 | 按权限 |
| plan_question | 不注入 | **注入** | 不注入 | 不注入 | 不注入 | 不注入 |
| submit_plan | 不注入 | **注入** | 不注入 | 不注入 | 不注入 | 不注入 |
| update_plan_step | 不注入 | 不注入 | 不注入 | **注入** | 不注入 | 不注入 |
| amend_plan | 不注入 | 不注入 | 不注入 | **注入** | **注入** | 不注入 |
| 系统提示词 | 无额外 | 5 阶段规划指令 | 计划上下文 | 计划内容 + 执行指令 | 计划 + 暂停指令 | 完成总结指令 |
| 模型 | 主模型 | **planModel 覆盖** | 主模型 | 主模型 | 主模型 | 主模型 |
| ChatInput | 灰色按钮 | 蓝色高亮 | 紫色 | 绿色 + 进度% | 黄色 | 绿色 |

---

## 后端架构

### Plan 状态管理

**文件**：`src-tauri/src/plan.rs`

使用全局 `OnceLock<Arc<RwLock<HashMap<String, PlanMeta>>>>` 管理 per-session 状态：

```rust
// 六态枚举
pub enum PlanModeState { Off, Planning, Review, Executing, Paused, Completed }

// 步骤状态
pub enum PlanStepStatus { Pending, InProgress, Completed, Skipped, Failed }
  // is_terminal() → Completed | Skipped | Failed

// 核心元数据
pub struct PlanMeta {
    pub session_id: String,
    pub title: Option<String>,
    pub file_path: String,
    pub state: PlanModeState,
    pub steps: Vec<PlanStep>,
    pub created_at: String,
    pub updated_at: String,
    pub paused_at_step: Option<usize>,    // 暂停时记录步骤位置
    pub version: u32,                      // 版本计数器
    pub checkpoint_ref: Option<String>,    // Git 分支名
}

pub struct PlanStep {
    pub index: usize,
    pub phase: String,        // "Phase 1: 分析"
    pub title: String,        // "读取 config 文件"
    pub description: String,  // 步骤详细描述
    pub status: PlanStepStatus,
    pub duration_ms: Option<u64>,
}

// 交互问答
pub struct PlanQuestion {
    pub question_id: String,
    pub text: String,
    pub options: Vec<PlanQuestionOption>,
    pub allow_custom: bool,
    pub multi_select: bool,
    pub template: Option<String>,  // "scope" / "tech_choice" / "priority"
}

pub struct PlanQuestionOption {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
    pub recommended: bool,         // 前端显示 ★ 标记
}

pub struct PlanQuestionGroup {
    pub request_id: String,
    pub session_id: String,
    pub questions: Vec<PlanQuestion>,
    pub context: Option<String>,
}

pub struct PlanQuestionAnswer {
    pub question_id: String,
    pub selected: Vec<String>,
    pub custom_input: Option<String>,
}

pub struct PlanVersionInfo {
    pub version: u32,
    pub file_path: String,
    pub modified_at: String,
    pub is_current: bool,
}
```

**公共 API**：

```rust
// 状态管理
pub async fn get_plan_state(session_id: &str) -> PlanModeState
pub async fn set_plan_state(session_id: &str, state: PlanModeState)
pub async fn get_plan_meta(session_id: &str) -> Option<PlanMeta>
pub async fn update_plan_steps(session_id: &str, steps: Vec<PlanStep>)
pub async fn update_step_status(session_id: &str, index: usize, status: PlanStepStatus, duration_ms: Option<u64>)
pub async fn restore_from_db(session_id: &str, plan_mode_str: &str)  // 崩溃恢复

// 交互问答
pub async fn register_plan_question(request_id: String, sender: oneshot::Sender<Vec<PlanQuestionAnswer>>)
pub async fn submit_plan_question_response(request_id: &str, answers: Vec<PlanQuestionAnswer>) -> Result<()>
pub async fn cancel_pending_plan_question(request_id: &str)

// 文件 I/O
pub fn save_plan_file(session_id: &str, content: &str) -> Result<String>
pub fn load_plan_file(session_id: &str) -> Result<Option<String>>
pub fn delete_plan_file(session_id: &str) -> Result<()>
pub fn parse_plan_steps(markdown: &str) -> Vec<PlanStep>

// 版本管理
pub fn list_plan_versions(session_id: &str) -> Result<Vec<PlanVersionInfo>>
pub fn load_plan_version(file_path: &str) -> Result<String>

// Git Checkpoint
pub fn create_git_checkpoint(session_id: &str) -> Option<String>
pub async fn get_checkpoint_ref(session_id: &str) -> Option<String>
pub fn rollback_to_checkpoint(checkpoint_ref: &str) -> Result<String>
pub fn cleanup_checkpoint(checkpoint_ref: &str)

// 路径权限
pub fn is_plan_mode_path_allowed(file_path: &str) -> bool
```

**持久化策略**：三层持久化

| 层 | 存储 | 用途 |
|----|------|------|
| 内存 HashMap | `PLAN_STORE` | 快速访问，per-request 查询 |
| DB `plan_mode` + `plan_steps` 列 | SessionDB | 崩溃恢复，步骤进度持久化（JSON） |
| Plan 文件 | `.opencomputer/plans/*.md` | 计划内容持久化 + 版本历史 |

### 工具限制与路径权限

**文件**：`src-tauri/src/commands/chat.rs`

```rust
// 常量定义（plan.rs）
pub const PLAN_MODE_DENIED_TOOLS: &[&str] = &["write", "edit", "apply_patch", "canvas"];
pub const PLAN_MODE_ASK_TOOLS: &[&str] = &["exec"];
pub const PLAN_MODE_PATH_AWARE_TOOLS: &[&str] = &["write", "edit"];
```

```mermaid
flowchart TD
    A[chat 函数入口] --> B{get_plan_state}
    B -->|Off| C[正常流程]
    B -->|Planning/Review| D[工具限制]
    B -->|Executing| E[注入执行工具]
    B -->|Paused| F[注入 amend_plan]
    B -->|Completed| G[注入完成提示词]

    D --> D1["denied_tools += apply_patch, canvas"]
    D --> D2["plan_mode_allow_paths = plans/"]
    D --> D3["plan_ask_tools = exec"]
    D --> D4["plan_tools_enabled = true<br/>注入 plan_question + submit_plan"]
    D --> D5["extra_system_context = 5 阶段规划提示词"]

    D1 --> H[路径感知过滤]
    D2 --> H
    H --> H1{write/edit 目标路径?}
    H1 -->|.opencomputer/plans/*.md| H2[允许]
    H1 -->|其他路径| H3[拒绝]

    E --> E1["plan_executing = true"]
    E --> E2["注入 update_plan_step + amend_plan"]
    E --> E3["extra_system_context = 计划内容 + 执行指令"]
```

**路径检查函数**（`is_plan_mode_path_allowed`）：
- 允许：`.opencomputer/plans/` 目录下的 `.md` 文件
- 允许：`~/.opencomputer/plans/` 全局目录下的 `.md` 文件
- 拒绝：所有其他路径

### 系统提示词注入

#### Planning 阶段：5 阶段规划流程

```
# Plan Mode Active — 5-Phase Planning Workflow

## Phase 1: Deep Exploration
- Read relevant source files systematically
- Launch up to 3 subagent tasks IN PARALLEL to explore different parts of codebase
- Map dependencies and data flow

## Phase 2: Requirements Clarification
- Use plan_question tool to ask structured questions
- Support recommended options, multi_select, template categories
- Iterate until requirements are clear

## Phase 3: Design & Architecture
- Analyze patterns in existing code
- Consider edge cases and error handling
- Evaluate trade-offs between approaches

## Phase 4: Plan Composition
- Write detailed phase-based plan with checklists
- Include file paths, function names, specific changes

## Phase 5: Review & Refinement
- Self-review the plan for completeness
- Call submit_plan to submit for user review

## Plan Format
### Background
[Context and reasoning]

### Phase N: <title>
- [ ] Step description (file: path/to/file.rs)
- [ ] Step description

## Restrictions
- CANNOT modify files outside .opencomputer/plans/
- CAN read files, search code, browse the web
- Shell commands (exec) require user approval
- Use subagent for parallel code exploration
```

#### Executing 阶段

```
# Executing Plan

Follow the steps below in order.
After completing each step, call update_plan_step to mark progress:
- update_plan_step(step_index=N, status="in_progress") when starting
- update_plan_step(step_index=N, status="completed") when done

If you need to modify the plan during execution, use amend_plan tool:
- amend_plan(action="insert", title="...", after_index=N)
- amend_plan(action="delete", step_index=N)
- amend_plan(action="update", step_index=N, title="...")

## Plan Content
[完整的 Plan Markdown 内容]
```

#### Completed 阶段

```
# Plan Execution Completed

All steps have been executed. Please:
1. Summarize what was accomplished
2. Highlight any steps that failed or were skipped, with reasons
3. Suggest follow-up actions or improvements
4. Note any issues discovered during execution
```

### Plan 文件持久化

**项目本地化存储**：优先存储在项目本地 `.opencomputer/plans/`，非 git 仓库回退到全局 `~/.opencomputer/plans/`。

```
# git 仓库项目
project-root/
└── .opencomputer/
    └── plans/
        ├── plan-a1b2c3-20260328.md          # 当前版本
        ├── plan-a1b2c3-20260328-v1.md       # 历史版本 1
        ├── plan-a1b2c3-20260328-v2.md       # 历史版本 2
        └── ...

# 非 git 项目（回退）
~/.opencomputer/
└── plans/
    └── ...
```

支持 `plansDirectory` 配置项自定义路径。加载时自动查找全局目录兼容旧计划。

**文件格式**：标准 Markdown，无 frontmatter，直接存储 LLM 通过 `submit_plan` 提交的 checklist 内容。

### Markdown Checklist 解析

**函数**：`plan::parse_plan_steps(markdown) -> Vec<PlanStep>`

解析规则：
- `### Phase N: title` → phase 分组
- `- [ ] text` → Pending 步骤
- `- [x] text` → Completed 步骤
- 步骤 index 从 0 开始连续编号
- 支持步骤描述（缩进行）

### 交互式问答（plan_question）

**文件**：`src-tauri/src/tools/plan_question.rs`

**条件注入**：仅在 `plan_tools_enabled = true`（Planning 状态）时注入。

**执行流程**：

```mermaid
sequenceDiagram
    participant LLM
    participant Tool as plan_question.rs
    participant Registry as PENDING_PLAN_QUESTIONS
    participant Tauri as Tauri Events
    participant UI as PlanQuestionBlock

    LLM->>Tool: plan_question({questions: [...]})
    Tool->>Registry: register_plan_question(request_id, oneshot_tx)
    Tool->>Tauri: emit("plan_question_request", PlanQuestionGroup)
    Tauri-->>UI: 渲染可视化问答卡片

    Note over UI: 用户选择选项 / 输入自定义答案

    UI->>Tauri: invoke("respond_plan_question", answers)
    Tauri->>Registry: submit_plan_question_response(request_id, answers)
    Registry->>Tool: oneshot_rx.recv() → answers

    Tool-->>LLM: 格式化的答案文本
    Note over LLM: 基于答案继续规划
```

**超时**：10 分钟无响应自动取消。

**问答 UI 特性**：
- `recommended` 选项显示 ★ 标记
- `template` 控制图标分类（scope / tech_choice / priority）
- `multi_select` 支持多选
- `allow_custom` 支持自定义输入

### 计划提交（submit_plan）

**文件**：`src-tauri/src/tools/submit_plan.rs`

**条件注入**：仅在 `plan_tools_enabled = true`（Planning 状态）时注入。

**执行流程**：
1. 解析 `title` + `content` 参数
2. 调用 `parse_plan_steps()` 解析步骤
3. 调用 `save_plan_file()` 持久化（自动备份旧版本）
4. 持久化步骤到 DB（`save_plan_steps`）
5. 状态转换：Planning → Review
6. 发射 `plan_submitted` 事件
7. 发射 `plan_mode_changed` 事件（state: "review"）

### 步骤进度追踪（update_plan_step）

**文件**：`src-tauri/src/tools/plan_step.rs`

**条件注入**：仅在 `plan_executing = true`（Executing 状态）时注入。

**参数**：

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| step_index | integer | 是 | 步骤的零基索引 |
| status | string | 是 | "in_progress" / "completed" / "skipped" / "failed" |
| summary | string | 否 | 步骤摘要，完成时保存到结果文件 |

**工具属性**：`internal: true`（不需要用户审批）

**执行流程**：

```mermaid
sequenceDiagram
    participant LLM
    participant Tool as plan_step.rs
    participant Store as PLAN_STORE
    participant DB as SessionDB
    participant Tauri as Tauri Events
    participant UI as 前端 UI

    LLM->>Tool: update_plan_step(step_index=2, status="completed")
    Tool->>Store: update_step_status(sid, 2, Completed)
    Tool->>DB: save_plan_steps(sid, steps_json)
    Tool->>Tauri: emit("plan_step_updated", {stepIndex: 2, status: "completed"})
    Tauri-->>UI: 更新 PlanCardBlock + PlanPanel

    alt 所有步骤 is_terminal()
        Tool->>Store: set_plan_state(sid, Completed)
        Tool->>DB: update_session_plan_mode(sid, "completed")
        Tool->>Tool: cleanup_checkpoint(checkpoint_ref)
        Tool->>Tauri: emit("plan_mode_changed", {state: "completed"})
        Tauri-->>UI: 显示完成状态 + 总结提示
    end

    Tool-->>LLM: "Step 2 marked as completed."
```

### 执行中计划修改（amend_plan）

**文件**：`src-tauri/src/tools/amend_plan.rs`

**条件注入**：Executing 或 Paused 状态时注入。

**操作**：

| action | 参数 | 说明 |
|--------|------|------|
| `insert` | after_index?, title, phase?, description? | 在指定步骤后插入新步骤 |
| `delete` | step_index | 删除步骤（已完成步骤不可删除） |
| `update` | step_index, title?, description?, phase? | 更新步骤信息（终态步骤不可更新） |

**执行后**：
- 自动重编号所有步骤
- 再生成计划 Markdown 文件
- 持久化步骤到 DB
- 发射 `plan_amended` 事件通知前端刷新

### Git Checkpoint 回滚

**执行流程**：

```mermaid
sequenceDiagram
    participant User
    participant Frontend as PlanPanel
    participant Backend as plan.rs
    participant Git

    Note over Backend: Review → Executing 时自动创建
    Backend->>Git: git branch opencomputer/checkpoint-{id}-{ts}
    Backend->>Backend: checkpoint_ref = branch_name

    Note over Frontend: 步骤失败时显示回滚按钮
    User->>Frontend: 点击"回滚更改"
    Frontend->>Backend: invoke("plan_rollback", session_id)
    Backend->>Git: git reset --hard <checkpoint_ref>
    Backend->>Git: git branch -D <checkpoint_ref>
    Backend-->>Frontend: "Rolled back to checkpoint"

    Note over Backend: 计划正常完成时自动清理
    Backend->>Git: git branch -D <checkpoint_ref>
```

**分支命名**：`opencomputer/checkpoint-{short_session_id}-{timestamp}`

### 计划版本管理

每次保存计划时自动备份旧版本为 `plan-{stem}-v{N}.md`，`PlanMeta.version` 计数器自增。

**功能**：
- `list_plan_versions()` → 列出当前版本 + 所有历史版本
- `load_plan_version()` → 读取指定版本内容
- `restore_plan_version()` → 恢复旧版本（覆盖当前 + 重新解析步骤）

前端 PlanPanel 提供版本历史浏览和一键恢复。

### Plan/Build 独立模型

**配置**：`AgentModelConfig.plan_model: Option<String>`

Planning 阶段可配置便宜/快速模型（如 Sonnet），节省 60-80% 探索成本。`commands/chat.rs` 在检测到 Planning 状态时，将 `plan_model` 作为模型链首选项覆盖。

前端 Agent 设置面板提供 Plan Model 选择器。

### 子 Agent 安全继承

**文件**：`src-tauri/src/subagent/spawn.rs`

```rust
// 子 Agent 继承 Planning/Review 阶段工具限制
let parent_plan_state = plan::get_plan_state(&parent_session_id).await;
if matches!(parent_plan_state, Planning | Review) {
    for tool in PLAN_MODE_DENIED_TOOLS {
        if !denied.contains(&tool) { denied.push(tool); }
    }
}
```

防止子 Agent 绕过 Planning 阶段的文件修改限制。这是 OpenCode 已知的安全漏洞（Issue #18515），我们在架构层面已修复。

---

## 斜杠命令 /plan

**文件**：`src-tauri/src/slash_commands/handlers/plan.rs`

| 子命令 | 说明 | CommandAction |
|--------|------|---------------|
| `/plan` 或 `/plan enter` | 进入 Planning | `EnterPlanMode` |
| `/plan exit` | 退出 Plan Mode | `ExitPlanMode` |
| `/plan approve` | 批准计划并执行（创建 Checkpoint） | `ApprovePlan` |
| `/plan show` | 显示当前计划内容 | `ShowPlan` |
| `/plan pause` | 暂停执行 | `PausePlan` |
| `/plan resume` | 恢复执行 | `ResumePlan` |

```mermaid
flowchart LR
    A["/plan args"] --> B[parser.rs]
    B --> C[dispatch → plan::handle_plan]
    C -->|enter| D[Planning]
    C -->|exit| E[Off]
    C -->|approve| F[Executing + Checkpoint]
    C -->|show| G[load_plan_file]
    C -->|pause| H[Paused]
    C -->|resume| I[Executing]
```

---

## Tauri 命令一览

| 命令 | 参数 | 返回 | 说明 |
|------|------|------|------|
| `get_plan_mode` | session_id | String | 获取当前 plan mode 状态 |
| `set_plan_mode` | session_id, state | () | 设置 plan mode（含 Checkpoint 创建/清理） |
| `get_plan_content` | session_id | Option\<String\> | 读取 plan 文件内容 |
| `save_plan_content` | session_id, content | () | 保存 plan 文件 + 解析 steps |
| `get_plan_steps` | session_id | Vec\<PlanStep\> | 获取步骤列表 |
| `update_plan_step_status` | session_id, step_index, status | () | 更新步骤状态（含自动完成检测） |
| `respond_plan_question` | request_id, answers | () | 回复交互问答 |
| `get_plan_versions` | session_id | Vec\<PlanVersionInfo\> | 列出版本历史 |
| `load_plan_version_content` | file_path | String | 读取指定版本内容 |
| `restore_plan_version` | session_id, file_path | () | 恢复旧版本 |
| `plan_rollback` | session_id | String | Git Checkpoint 回滚 |
| `get_plan_checkpoint` | session_id | Option\<String\> | 获取 Checkpoint 分支名 |

---

## 前端架构

### usePlanMode Hook

**文件**：`src/components/chat/plan-mode/usePlanMode.ts`

```typescript
interface UsePlanModeReturn {
  planState: PlanModeState  // "off"|"planning"|"review"|"executing"|"paused"|"completed"
  setPlanState: Dispatch<SetStateAction<PlanModeState>>
  planSteps: PlanStep[]
  setPlanSteps: Dispatch<SetStateAction<PlanStep[]>>
  planContent: string
  setPlanContent: Dispatch<SetStateAction<string>>
  showPanel: boolean
  setShowPanel: Dispatch<SetStateAction<boolean>>
  progress: number           // 0-100
  completedCount: number
  planCardInfo: PlanCardInfo | null
  pendingQuestionGroup: PlanQuestionGroup | null
  enterPlanMode: () => Promise<void>
  exitPlanMode: () => Promise<void>
  approvePlan: () => Promise<void>
  pauseExecution: () => Promise<void>
  resumeExecution: () => Promise<void>
}
```

**事件监听**：

| 事件 | payload | 动作 |
|------|---------|------|
| `plan_step_updated` | `{sessionId, stepIndex, status, durationMs?}` | 更新 planSteps 对应步骤 |
| `plan_mode_changed` | `{sessionId, state, reason?}` | 更新 planState |
| `plan_submitted` | `{sessionId, title, stepCount, phaseCount, steps[]}` | 设置 planCardInfo + planSteps |
| `plan_amended` | `{sessionId, steps[], stepCount}` | 刷新 planSteps |
| `plan_question_request` | serialized PlanQuestionGroup | 设置 pendingQuestionGroup |
| `plan_content_updated` | `{sessionId, stepCount, content}` | 更新 planContent |

### ChatInput 集成

**文件**：`src/components/chat/ChatInput.tsx`

```
┌──────────────────────────────────────────┐
│ ┌──────────────────────────────────────┐ │
│ │ 📋 计划模式：文件修改工具已禁用...  ✕ │ │ ← 蓝色横幅（Planning/Review 时）
│ └──────────────────────────────────────┘ │
│                                          │
│  描述你想要实现的功能...                  │ ← placeholder 变化
│                                          │
├──────────────────────────────────────────┤
│ 🖼 📎 ⚡ [📋Plan] 🛡Auto  模型选择  ▶  │ ← 工具栏
└──────────────────────────────────────────┘

Plan 按钮五色状态：
  Off:       灰色图标，无文字
  Planning:  蓝色高亮 + "Plan" 文字
  Review:    紫色高亮
  Executing: 绿色高亮 + "75%" 进度
  Paused:    黄色高亮
```

### PlanCardBlock 摘要卡片

**文件**：`src/components/chat/plan-mode/PlanCardBlock.tsx`

在聊天消息流中渲染的计划摘要卡片，LLM 调用 `submit_plan` 后触发 `plan_submitted` 事件时插入。

```
┌── 📋 OpenCode Agent 调度逻辑分析文档 ── 在面板中查看 > ──┐
│  2 个阶段 · 8 个步骤                                     │
│                                                          │
│  8/8 步完成                                     100%     │
│  ████████████████████████████████████████████             │
│                                                          │
│  > Phase 1: 信息收集（已完成）              4/4          │  ← 可点击展开
│    ✅ 步骤 1                                             │    展开后显示
│    ✅ 步骤 2                                             │    PlanStepItem
│    ✅ 步骤 3                                             │
│    ✅ 步骤 4                                             │
│  > Phase 2: 文档生成                        4/4          │
│                                                          │
│  [⏸ 暂停]                                               │  ← 操作按钮
└──────────────────────────────────────────────────────────┘
```

**操作按钮**（按状态切换）：
- Review：`[▶ 批准并执行]` `[✕ 退出不执行]`
- Executing：`[⏸ 暂停]`
- Paused：`[▶ 恢复]` `[✕ 退出]`
- Completed：`✅ 计划执行完成`
- Planning：`🔄 正在规划...`

### PlanQuestionBlock 交互问答

**文件**：`src/components/chat/plan-mode/PlanQuestionBlock.tsx`

LLM 调用 `plan_question` 后在消息流中渲染的可视化问答卡片：

```
┌── 🤔 需要确认以下问题 ──────────────────────────────────┐
│                                                          │
│  Q1: 项目的目标范围是什么？                    [🎯 scope] │
│                                                          │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐      │
│  │ 仅核心功能  │  │ 核心 + 测试 │  │ 完整重构 ★  │      │  ← recommended
│  │ 最小改动    │  │ 含单元测试  │  │ 含文档更新  │      │
│  └─────────────┘  └─────────────┘  └─────────────┘      │
│                                                          │
│  [自定义输入...]                                         │
│                                                          │
│  [提交]                                                  │
└──────────────────────────────────────────────────────────┘
```

### PlanPanel 右侧面板

**文件**：`src/components/chat/plan-mode/PlanPanel.tsx`

四种视图模式：

**Review 视图**：
```
┌─────────── Plan Panel ──────────────┐
│ 📋 Plan Title                  ✕    │
├─────────────────────────────────────┤
│                                     │
│  [Markdown 只读渲染]               │  ← 计划完整内容
│                                     │
├─────────────────────────────────────┤
│  [▶ 批准并执行]                     │
│  [📝 请求修改]  [✕ 退出]           │
│  [📋 版本历史]                      │  ← 浏览 + 恢复旧版本
└─────────────────────────────────────┘
```

**Executing 视图**：
```
┌─────────── Plan Panel ──────────────┐
│ 📋 Plan Title                  ✕    │
├─────────────────────────────────────┤
│  5/8 步完成               62%      │
│  ████████████████░░░░░░░░░          │
├─────────────────────────────────────┤
│  PHASE 1: ANALYSIS                  │
│    ✅ Read config files      [12s] │
│    ✅ Analyze CSS vars        [8s] │
│  PHASE 2: IMPLEMENTATION            │
│    🔄 Add ThemeProvider  (运行中)  │
│    ⭕ Create toggle button          │
│    ⭕ Persist preference            │
├─────────────────────────────────────┤
│  🔄 正在按计划执行...               │
│  [⏸ 暂停]                          │
└─────────────────────────────────────┘
```

**Paused 视图**：增加回滚按钮（有 Checkpoint 时显示）

**Completed 视图**：显示完成状态 + 执行摘要

**布局**：`w-[400px] shrink-0 max-w-[40vw]`，与 CanvasPanel 互斥显示。

**状态图标**：

| 状态 | 图标 | 颜色 |
|------|------|------|
| pending | Circle | text-muted-foreground |
| in_progress | Loader2 (spin) | text-blue-500 |
| completed | CheckCircle | text-green-500 |
| failed | XCircle | text-red-500 |
| skipped | MinusCircle | text-gray-400 |

### planParser 解析器

**文件**：`src/components/chat/plan-mode/planParser.ts`

| 函数 | 说明 |
|------|------|
| `detectPlanContent(content)` | 检测消息是否包含 Plan 格式，返回 `{ isPlan, steps, title }` |
| `groupStepsByPhase(steps)` | 按 phase 分组，返回 `{ name, steps }[]` |
| `formatDuration(ms)` | 格式化毫秒为 "12s" / "1m30s" 等 |

---

## 完整交互流程

### 1. Planning → Review → Executing → Completed

```mermaid
sequenceDiagram
    actor User
    participant CI as ChatInput
    participant Hook as usePlanMode
    participant Backend as Rust 后端
    participant LLM
    participant Panel as PlanPanel

    Note over User,Panel: Phase 1: 进入 Planning
    User->>CI: 点击 Plan 按钮
    CI->>Hook: enterPlanMode()
    Hook->>Backend: invoke("set_plan_mode", {state: "planning"})
    Backend->>Backend: set_plan_state(Planning) + DB
    CI->>CI: 蓝色高亮 + 横幅

    Note over User,Panel: Phase 2: 交互规划（5 阶段流程）
    User->>CI: "帮我重构认证模块"
    CI->>Backend: invoke("chat")
    Backend->>Backend: Planning → denied_tools + ask_tools + allow_paths + planModel
    Backend->>Backend: 注入 plan_question + submit_plan 工具
    Backend->>LLM: API 请求（planModel + 规划提示词）

    LLM->>LLM: Phase 1 - 探索代码（read/grep/subagent）
    LLM->>Backend: tool_call: plan_question({questions: [...]})
    Backend-->>Panel: emit("plan_question_request")
    Panel->>User: 渲染 PlanQuestionBlock 问答卡片

    User->>Panel: 选择选项 / 输入答案
    Panel->>Backend: invoke("respond_plan_question", answers)
    Backend-->>LLM: 答案文本

    LLM->>LLM: Phase 2-4 - 需求澄清 → 架构设计 → 编写计划
    LLM->>Backend: tool_call: submit_plan({title, content})
    Backend->>Backend: 保存文件 + 解析步骤 + 持久化 DB
    Backend->>Backend: Planning → Review
    Backend-->>Panel: emit("plan_submitted") + emit("plan_mode_changed")

    Note over User,Panel: Phase 3: Review 审查
    Panel->>User: Markdown 只读渲染 + 审批/修改/退出按钮

    alt 请求修改
        User->>Panel: 点击"请求修改" + 输入反馈
        Panel->>Hook: Review → Planning + 发送反馈给 LLM
        LLM->>LLM: 修订计划 → 再次 submit_plan
    end

    User->>Panel: 点击"批准并执行"
    Panel->>Hook: approvePlan()
    Hook->>Backend: invoke("set_plan_mode", {state: "executing"})
    Backend->>Backend: create_git_checkpoint() → branch
    Backend->>Backend: Review → Executing

    Note over User,Panel: Phase 4: 执行
    CI->>Backend: invoke("chat", "请按计划执行")
    Backend->>Backend: Executing → plan_executing + 计划内容注入
    Backend->>LLM: API 请求（全部工具 + update_plan_step + amend_plan）

    loop 每个步骤
        LLM->>Backend: update_plan_step(N, "in_progress")
        Backend->>Backend: persist to DB
        Backend-->>Panel: 步骤 N 高亮蓝色
        LLM->>LLM: 执行工作（write/edit/exec...）
        LLM->>Backend: update_plan_step(N, "completed")
        Backend->>Backend: persist to DB
        Backend-->>Panel: 步骤 N 变绿 ✅
    end

    Note over User,Panel: Phase 5: 自动完成
    Backend->>Backend: all_terminal() → Completed
    Backend->>Backend: cleanup_checkpoint()
    Backend-->>Panel: emit("plan_mode_changed", {state: "completed"})
    Backend->>Backend: 注入 PLAN_COMPLETED_SYSTEM_PROMPT
    LLM-->>Panel: 总结执行结果 + 建议后续
```

### 2. 失败回滚流程

```mermaid
sequenceDiagram
    actor User
    participant Panel as PlanPanel
    participant Backend as plan.rs
    participant Git

    Panel->>User: 步骤 N 失败（红色）+ 回滚按钮
    Panel->>Backend: invoke("get_plan_checkpoint")
    Backend-->>Panel: "opencomputer/checkpoint-xxx"

    User->>Panel: 点击"回滚更改"
    Panel->>Backend: invoke("plan_rollback", session_id)
    Backend->>Git: git reset --hard <checkpoint>
    Backend->>Git: git branch -D <checkpoint>
    Backend-->>Panel: "Rolled back to checkpoint"
    Panel->>User: 代码恢复到执行前状态
```

### 3. 子 Agent 安全继承

```mermaid
sequenceDiagram
    participant Main as 主 Agent (Planning)
    participant Spawn as subagent/spawn.rs
    participant Sub as 子 Agent

    Main->>Spawn: subagent(spawn, task="探索代码")
    Spawn->>Spawn: 检测父 session = Planning
    Spawn->>Spawn: denied_tools += PLAN_MODE_DENIED_TOOLS
    Spawn->>Sub: spawn（工具列表不含 write/edit）
    Sub->>Sub: 只能 read/grep/web_fetch ✅
```

---

## DB Schema

### sessions 表

```sql
-- plan_mode 列
ALTER TABLE sessions ADD COLUMN plan_mode TEXT DEFAULT 'off';
-- 值域："off"|"planning"|"review"|"executing"|"paused"|"completed"

-- plan_steps 列（崩溃恢复）
ALTER TABLE sessions ADD COLUMN plan_steps TEXT;
-- JSON 序列化的 Vec<PlanStep>，每次 update_step_status 后自动写入
```

**崩溃恢复策略**：
1. `restore_from_db()` 读取 `plan_mode` 重建状态
2. 优先从 `plan_steps` 列加载步骤（JSON）
3. 回退到从 plan 文件重新解析

---

## 事件系统

| 事件名 | 触发来源 | Payload | 监听方 |
|--------|----------|---------|--------|
| `plan_step_updated` | plan_step.rs / commands/plan.rs | `{sessionId, stepIndex, status, durationMs?}` | usePlanMode → 更新步骤状态 |
| `plan_mode_changed` | submit_plan / set_plan_mode / 自动完成 | `{sessionId, state, reason?}` | usePlanMode → 状态切换 |
| `plan_submitted` | submit_plan.rs | `{sessionId, title, stepCount, phaseCount, steps[]}` | usePlanMode → 设置 planCardInfo |
| `plan_amended` | amend_plan.rs | `{sessionId, steps[], stepCount}` | usePlanMode → 刷新步骤列表 |
| `plan_question_request` | plan_question.rs | serialized PlanQuestionGroup | usePlanMode → 渲染问答 UI |
| `plan_content_updated` | save_plan_content | `{sessionId, stepCount, content}` | usePlanMode → 更新 planContent |

---

## 安全设计

### 细粒度路径权限

- Planning/Review 状态下 `write`/`edit` 工具仅允许操作 `.opencomputer/plans/` 目录下的 `.md` 文件
- `apply_patch`/`canvas` 完全禁用
- `exec` 强制用户审批
- 路径检查通过 `is_plan_mode_path_allowed()` + `plan_mode_allow_paths` 在 `ToolExecContext` 中传播

### 子 Agent 限制继承

Planning/Review 状态下 spawn 的子 Agent 自动继承 `PLAN_MODE_DENIED_TOOLS`，防止工具限制逃逸。

### 内部工具不需审批

`update_plan_step` 和 `amend_plan` 标记为 `internal: true`，不触发用户审批流程。

### 步骤进度不丢失

每次步骤状态更新后自动持久化到 DB `plan_steps` 列，崩溃后可恢复。

### Git Checkpoint 保护

执行前自动创建 git 分支快照，失败时可回滚到执行前状态，正常完成后自动清理。

---

## 与 Claude Code / OpenCode 对比

| 特性 | Claude Code | OpenCode | OpenComputer |
|------|-------------|----------|--------------|
| 入口 | Shift+Tab / CLI flag | Tab 键切换 | **工具栏按钮** + /plan 命令 |
| 状态数 | 4 (default/acceptEdits/plan/auto) | 2 (Plan/Build) | **6 (Off/Planning/Review/Executing/Paused/Completed)** |
| 规划流程 | 无结构化流程 | 系统提示词引导 | **5 阶段流程 + 结构化问答** |
| Plan 展示 | Plan 文件 + 编辑器 | 终端文本 | **PlanCardBlock + PlanPanel 双视图** |
| 交互问答 | 无 | 普通 question 工具 | **plan_question 类型化选项 + UI 卡片** |
| 进度追踪 | 无 | 无（依赖 todoread/todowrite） | **实时步骤追踪（工具 + 事件 + DB 持久化）** |
| 暂停/恢复 | 无 | 无 | **Paused 状态 + paused_at_step** |
| 执行中修改 | 无 | 无 | **amend_plan（insert/delete/update）** |
| 完成总结 | 无 | 无 | **Completed 状态 + 总结提示词** |
| 工具限制 | 文件系统级只读 | deny/ask 配置 | **路径感知权限（plans/ 路径可写）** |
| 子 Agent 安全 | N/A | **已知逃逸问题** | **继承 denied_tools** |
| Plan 存储 | `~/.claude/plans/` | `.opencode/plans/` | **项目本地 + 全局回退** |
| 版本管理 | 无 | 无 | **自动备份 + 版本历史 + 恢复** |
| Git 回滚 | 无 | 无 | **Checkpoint 分支 + 一键回滚** |
| 模型优化 | 同一模型 | Plan/Build 独立模型 | **planModel 覆盖（节省 60-80%）** |
| 审批选项 | 4 种执行模式 | 确认/取消 | **批准/请求修改/退出/版本恢复** |

---

## 文件清单

### 后端文件

| 文件 | 用途 |
|------|------|
| `src-tauri/src/plan.rs` | 核心：六态状态机 + 文件 I/O + 解析 + 常量 + 路径权限 + Git Checkpoint + 版本管理 + 问答注册 |
| `src-tauri/src/commands/plan.rs` | 11 个 Tauri 命令 |
| `src-tauri/src/slash_commands/handlers/plan.rs` | /plan 斜杠命令处理（6 子命令） |
| `src-tauri/src/tools/plan_question.rs` | plan_question 交互问答工具 |
| `src-tauri/src/tools/submit_plan.rs` | submit_plan 计划提交工具 |
| `src-tauri/src/tools/plan_step.rs` | update_plan_step 步骤进度工具 |
| `src-tauri/src/tools/amend_plan.rs` | amend_plan 执行中修改工具 |
| `src-tauri/src/subagent/spawn.rs` | 子 Agent 计划模式限制继承 |

### 前端文件

| 文件 | 用途 |
|------|------|
| `src/components/chat/plan-mode/usePlanMode.ts` | Hook：六态状态管理 + 事件监听 |
| `src/components/chat/plan-mode/PlanCardBlock.tsx` | 摘要卡片（可展开 Phase 步骤） |
| `src/components/chat/plan-mode/PlanQuestionBlock.tsx` | 交互问答 UI（选项卡片 + recommended 标记） |
| `src/components/chat/plan-mode/PlanPanel.tsx` | 右侧详情面板（四种视图 + 版本历史 + 回滚） |
| `src/components/chat/plan-mode/PlanStepItem.tsx` | 步骤行组件（状态图标 + 耗时） |
| `src/components/chat/plan-mode/PlanBlock.tsx` | 容器组件 |
| `src/components/chat/plan-mode/planParser.ts` | Markdown Plan 格式检测与解析 |

### 修改文件

| 文件 | 改动点 |
|------|--------|
| `src-tauri/src/lib.rs` | `mod plan` + 注册 11 个命令 |
| `src-tauri/src/paths.rs` | `plans_dir()` + 项目本地路径解析 |
| `src-tauri/src/agent_config.rs` | `AgentModelConfig.plan_model` 字段 |
| `src-tauri/src/session/db.rs` | 迁移（plan_mode + plan_steps 列）+ 读写方法 |
| `src-tauri/src/session/types.rs` | `SessionMeta.plan_mode` 字段 |
| `src-tauri/src/commands/chat.rs` | 六态分支：工具限制 + 路径权限 + 模型覆盖 + 提示词注入 |
| `src-tauri/src/agent/mod.rs` | plan_ask_tools + plan_executing + plan_tools_enabled + plan_mode_allow_paths |
| `src-tauri/src/agent/providers/*.rs` | 4 个 Provider 条件注入 plan 工具 |
| `src-tauri/src/tools/mod.rs` | 4 个 plan 工具模块 + 常量 |
| `src-tauri/src/tools/execution.rs` | 4 个工具 dispatch |
| `src-tauri/src/tools/definitions.rs` | 4 个工具定义 |
| `src-tauri/src/slash_commands/registry.rs` | 注册 /plan 命令 |
| `src-tauri/src/slash_commands/types.rs` | 6 个 CommandAction 变体 |
| `src/components/chat/ChatScreen.tsx` | usePlanMode + PlanPanel + handleCommandAction |
| `src/components/chat/ChatInput.tsx` | Plan 按钮五色 + 横幅 + placeholder |
| `src/i18n/locales/zh.json` | planMode.* 翻译 |
| `src/i18n/locales/en.json` | planMode.* 翻译 |
