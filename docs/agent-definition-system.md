# OpenComputer Agent 定义系统设计文档

> 版本: 0.1 (Draft)
> 日期: 2026-03-15
> 状态: 设计阶段

## 1. 概述

设计一套基于 Markdown 文件的 Agent 定义系统，让用户可以通过编写/编辑 `.md` 文件来定义 Agent 的身份、行为、记忆和能力边界。

### 设计原则

- **Markdown 优先** — Agent 定义、人设、记忆都用 Markdown 文件，人类可读可编辑
- **Rust 原生** — 所有解析和组装逻辑在 Rust 侧完成，不依赖前端
- **渐进式复杂度** — 单个 AGENT.md 文件即可定义一个完整 Agent，复杂场景再拆分辅助文件
- **兼容现有架构** — 复用 `ProviderStore`、`SkillSystem`、`ToolSystem`，不破坏已有功能

### 参考实现

本设计参考了 [OpenClaw](https://github.com/nichochar/open-claw) 的系统提示词组装、Agent 配置和记忆系统，并根据 OpenComputer 的桌面应用场景和 Rust 技术栈做了简化适配。

---

## 2. 目录结构

```
~/.opencomputer/
├── config.json                    # 全局配置（Provider、activeModel 等，已有）
├── credentials/                   # OAuth 凭证（已有）
├── skills/                        # 技能目录（已有）
│
├── agents/                        # [新增] Agent 定义目录
│   ├── default/                   #   默认 Agent
│   │   ├── AGENT.md               #   Agent 定义文件（核心，必需）
│   │   ├── SOUL.md                #   人设/性格（可选）
│   │   ├── USER.md                #   用户信息（可选）
│   │   ├── TOOLS.md               #   工具使用指导（可选）
│   │   └── memory/                #   Agent 私有记忆
│   │       ├── MEMORY.md          #     记忆索引
│   │       └── *.md               #     记忆文件
│   │
│   ├── coder/                     #   示例：编程专家 Agent
│   │   ├── AGENT.md
│   │   ├── SOUL.md
│   │   └── memory/
│   │
│   └── researcher/                #   示例：研究助手 Agent
│       ├── AGENT.md
│       └── memory/
│
├── memory/                        # [新增] 全局共享记忆
│   ├── MEMORY.md                  #   全局记忆索引
│   ├── user_*.md                  #   用户画像记忆
│   ├── feedback_*.md              #   反馈记忆
│   ├── project_*.md               #   项目记忆
│   └── reference_*.md             #   引用记忆
│
├── home/                          # Agent 工作目录（已有）
└── share/                         # 跨 Agent 共享目录（已有）
```

### 关键约定

- 每个 Agent 是 `agents/` 下的一个子目录，目录名即 Agent ID
- `AGENT.md` 是唯一必需文件，其他文件均为可选增强
- `memory/` 目录按 Agent 隔离，全局 `memory/` 为跨 Agent 共享
- 目录名使用小写字母、数字和连字符（如 `code-reviewer`）

---

## 3. AGENT.md — Agent 定义文件

AGENT.md 是 Agent 的核心定义文件，采用 YAML frontmatter + Markdown body 的结构：

```markdown
---
# === 基本信息 ===
name: "默认助手"
description: "通用 AI 助手，擅长编程、写作和问题解决"
emoji: "🤖"
avatar: ""                          # 可选，头像文件路径或 URL

# === 模型配置（可选，覆盖全局 activeModel）===
model:
  primary: ""                       # 留空则使用全局 activeModel
  fallbacks: []                     # 备选模型列表

# === 能力过滤 ===
skills:                             # 技能过滤（全部留空 = 使用所有已启用技能）
  allow: []                         # 白名单，非空时只加载列出的技能
  deny: []                          # 黑名单，排除列出的技能

tools:                              # 工具过滤（全部留空 = 使用所有工具）
  allow: []                         # 白名单，如 ["exec", "read", "write", "edit"]
  deny: []                          # 黑名单，如 ["web_search"]

# === 记忆配置 ===
memory:
  enabled: true                     # 是否启用记忆系统
  shared: true                      # 是否加载全局共享记忆
  auto_flush: true                  # 对话接近上下文限制时自动保存记忆

# === 行为配置 ===
behavior:
  max_tool_rounds: 10               # 工具循环最大轮数
  require_approval: ["exec"]        # 需要用户审批的工具
  sandbox: false                    # 是否默认使用 Docker 沙箱执行命令
---

# 系统指令

你是一个智能助手，运行在 OpenComputer 桌面应用中。

## 核心原则
- 简洁直接，先行动再解释
- 安全第一，不执行高危操作前先确认
- 尊重用户偏好和工作习惯

## 工作风格
- 代码修改前先阅读现有代码
- 优先编辑现有文件而非创建新文件
- 保持最小改动原则
```

### 设计说明

| 部分 | 说明 |
|------|------|
| frontmatter | 结构化配置，Rust 反序列化为 `AgentConfig` |
| body（正文）| 自然语言系统指令，直接注入系统提示词 |

**所有 frontmatter 字段均为可选**，缺省时使用合理默认值（见第 7 节数据结构）。

---

## 4. 辅助定义文件

### 4.1 SOUL.md — 人设定义

定义 Agent 的性格、沟通风格和行为边界。与 AGENT.md 的系统指令互补：AGENT.md 偏向"做什么"，SOUL.md 偏向"怎么做"。

```markdown
---
name: soul
description: Agent 性格与沟通风格定义
---

## 性格
- 专业但友善
- 直接不啰嗦，不说废话
- 遇到不确定的事情坦诚说明

## 沟通风格
- 中文交流时使用简体中文
- 技术术语保持英文原文
- 代码注释使用英文

## 边界
- 不猜测用户意图，不确定时主动询问
- 不做超出请求范围的"改进"
- 不在回复末尾做重复总结
```

### 4.2 USER.md — 用户信息

存储用户画像，让 Agent 个性化响应。可由用户手动编辑，也可由 Agent 在对话中自动维护。

```markdown
---
name: user_info
description: 用户基本信息和技术偏好
---

## 基本信息
- 全栈开发者，10 年经验
- 当前主要项目: OpenComputer（Tauri + React + Rust）

## 技术偏好
- 熟悉: Rust, TypeScript, React, Go
- 编辑器: VS Code / Cursor
- 终端: iTerm2 + zsh

## 协作偏好
- 喜欢简洁的回复，不需要过多解释
- 代码修改时希望看到 diff 而非完整文件
```

### 4.3 TOOLS.md — 工具使用指导

为 Agent 提供工具使用的额外指导，补充或覆盖默认行为。

```markdown
---
name: tools_guide
description: 工具使用的自定义指导
---

## exec 工具
- 优先使用 brew 安装软件
- git 操作不要使用 --force 除非用户明确要求
- 长时间运行的命令使用 background 模式

## write/edit 工具
- 新文件使用 UTF-8 编码
- 遵循项目已有的代码风格
```

---

## 5. 记忆系统

### 5.1 设计理念

记忆系统采用**纯 Markdown 文件**存储，按类型分类，通过 `MEMORY.md` 索引管理。先期不实现向量检索，仅在会话开始时加载索引内容到系统提示词。

### 5.2 记忆类型

| 类型 | 前缀 | 用途 | 示例 |
|------|------|------|------|
| `user` | `user_` | 用户角色、目标、知识背景 | `user_role.md` |
| `feedback` | `feedback_` | 用户对 Agent 行为的纠正 | `feedback_no_summary.md` |
| `project` | `project_` | 项目状态、目标、决策 | `project_v2_migration.md` |
| `reference` | `reference_` | 外部资源指针 | `reference_jira_board.md` |

### 5.3 记忆文件格式

```markdown
---
name: user_role
description: 用户是一名全栈开发者，擅长 Rust 和 TypeScript
type: user
created: 2026-03-15
updated: 2026-03-15
---

用户是一名全栈开发者，当前主要在开发 OpenComputer 项目。
熟悉 Rust、TypeScript、React、Tauri 技术栈。
偏好简洁的代码风格和直接的沟通方式。
```

### 5.4 MEMORY.md 索引

```markdown
# Memory Index

## User
- [user_role.md](user_role.md) - 用户角色和技术背景

## Feedback
- [feedback_no_summary.md](feedback_no_summary.md) - 不需要在每次操作后总结

## Project
- [project_agent_system.md](project_agent_system.md) - Agent 定义系统设计中

## Reference
- [reference_openclaw.md](reference_openclaw.md) - OpenClaw 源码在 ~/Codes/openclaw
```

**约束**：
- `MEMORY.md` 只做索引，不存储记忆内容本身
- 索引控制在 200 行以内，超出部分在系统提示词中会被截断
- 记忆文件名使用 `{type}_{topic}.md` 格式

### 5.5 记忆操作

| 操作 | 触发方式 | 行为 |
|------|---------|------|
| 创建 | Agent 主动识别 / 用户说"记住..." | 创建 `memory/{type}_{topic}.md` + 更新 `MEMORY.md` |
| 更新 | Agent 发现信息变化 / 用户指令 | 修改已有 `.md` 文件 + 更新 frontmatter `updated` |
| 删除 | 用户说"忘记..." | 删除文件 + 从 `MEMORY.md` 移除条目 |
| 加载 | 每次会话开始 | 读取 `MEMORY.md` 内容注入系统提示词 |

### 5.6 记忆作用域

```
┌─────────────────────────────────────────┐
│           全局共享记忆                    │
│    ~/.opencomputer/memory/              │
│    所有 Agent 可见（当 shared=true）      │
├─────────────────────────────────────────┤
│    Agent 私有记忆                        │
│    ~/.opencomputer/agents/{id}/memory/  │
│    仅该 Agent 可见                       │
└─────────────────────────────────────────┘
```

加载优先级：先加载全局共享记忆索引，再加载 Agent 私有记忆索引。冲突时私有记忆优先。

### 5.7 什么不该存为记忆

- 代码模式、架构、文件路径 — 可以从代码库直接读取
- Git 历史、谁改了什么 — `git log` / `git blame` 是权威来源
- 调试方案 — 修复已在代码中，上下文在 commit message 里
- CLAUDE.md / AGENT.md 中已有的内容 — 不要重复
- 临时任务状态、当前对话上下文 — 属于会话生命周期，不是记忆

---

## 6. 系统提示词组装

### 6.1 组装流程

```
┌──────────────────────────────────────────────────┐
│              System Prompt Assembly               │
│                                                   │
│  ① 基础身份行                                     │
│     "You are {agent.name}, running in             │
│      OpenComputer on {os} {arch}."                │
│                                                   │
│  ② AGENT.md 正文                                  │
│     ← 直接注入 Markdown body                      │
│                                                   │
│  ③ SOUL.md（如存在）                               │
│     ← 人设 / 性格 / 沟通风格                       │
│                                                   │
│  ④ USER.md（如存在）                               │
│     ← 用户信息 / 偏好                              │
│                                                   │
│  ⑤ TOOLS.md（如存在）                              │
│     ← 工具使用的自定义指导                          │
│                                                   │
│  ⑥ 工具定义段                                     │
│     ← tools.rs 中启用的工具描述                    │
│     ← 根据 agent.tools.allow/deny 过滤            │
│                                                   │
│  ⑦ 技能段                                         │
│     ← skills.rs 加载的可用技能描述                  │
│     ← 根据 agent.skills.allow/deny 过滤           │
│                                                   │
│  ⑧ 记忆段（如 memory.enabled=true）                │
│     ← 全局 MEMORY.md 索引（如 shared=true）        │
│     ← Agent 私有 MEMORY.md 索引                    │
│                                                   │
│  ⑨ 运行时信息                                     │
│     ← 当前日期 / 时区 / 工作目录 / OS / Shell       │
│                                                   │
│  ⑩ 项目上下文（如在项目目录中）                     │
│     ← .opencomputer/AGENT.md（项目级追加指令）      │
│     ← CLAUDE.md（兼容加载）                        │
│                                                   │
└──────────────────────────────────────────────────┘
```

### 6.2 截断策略

借鉴 OpenClaw 的截断机制：

| 限制 | 默认值 | 说明 |
|------|--------|------|
| 单文件上限 | 20,000 字符 | 单个 .md 文件最大注入长度 |
| 总上下文预算 | 150,000 字符 | 所有注入内容的总预算 |
| 截断方式 | 头 70% + 尾 20% + 标记 | 超限文件保留头尾，中间插入 `[... truncated ...]` |
| MEMORY.md 上限 | 200 行 | 索引文件最大行数 |

### 6.3 项目级覆盖

支持在项目目录中放置 `.opencomputer/AGENT.md`，追加项目特定指令：

```
~/Projects/my-app/
└── .opencomputer/
    └── AGENT.md    # 项目级指令
```

**合并规则**：
- frontmatter 配置：项目级深度合并覆盖 Agent 级
- 正文指令：追加到 Agent 指令之后，标记为 `# Project Context`

---

## 7. Rust 数据结构

### 7.1 核心类型

```rust
// src-tauri/src/agent_config.rs

use serde::{Deserialize, Serialize};

/// Agent 定义的 frontmatter 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_name")]
    pub name: String,
    pub description: Option<String>,
    pub emoji: Option<String>,
    pub avatar: Option<String>,

    #[serde(default)]
    pub model: AgentModelConfig,
    #[serde(default)]
    pub skills: FilterConfig,
    #[serde(default)]
    pub tools: FilterConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub behavior: BehaviorConfig,
}

/// 模型选择（可选覆盖全局 activeModel）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentModelConfig {
    pub primary: Option<String>,          // "provider_id/model_id" 格式
    #[serde(default)]
    pub fallbacks: Vec<String>,
}

/// 能力过滤器（用于 skills 和 tools）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilterConfig {
    #[serde(default)]
    pub allow: Vec<String>,               // 白名单（非空时仅允许列出项）
    #[serde(default)]
    pub deny: Vec<String>,                // 黑名单（排除列出项）
}

/// 记忆配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "yes")]
    pub shared: bool,                     // 加载全局共享记忆
    #[serde(default = "yes")]
    pub auto_flush: bool,                 // 接近上下文限制时自动保存
}

/// 行为配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorConfig {
    #[serde(default = "ten")]
    pub max_tool_rounds: u32,             // 工具循环最大轮数
    #[serde(default)]
    pub require_approval: Vec<String>,    // 需审批的工具名
    #[serde(default)]
    pub sandbox: bool,                    // 默认 Docker 沙箱
}
```

### 7.2 Agent 完整定义

```rust
/// 从文件系统加载的完整 Agent 定义
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    pub id: String,                       // 目录名，如 "default", "coder"
    pub dir: PathBuf,                     // Agent 目录绝对路径
    pub config: AgentConfig,              // AGENT.md frontmatter
    pub system_instructions: String,      // AGENT.md body（Markdown 正文）
    pub soul: Option<String>,             // SOUL.md 全文
    pub user_info: Option<String>,        // USER.md 全文
    pub tools_guide: Option<String>,      // TOOLS.md 全文
    pub memory_index: Option<String>,     // memory/MEMORY.md 内容
}
```

### 7.3 记忆条目

```rust
/// 单条记忆的 frontmatter 元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMeta {
    pub name: String,
    pub description: String,
    #[serde(rename = "type")]
    pub memory_type: MemoryType,
    pub created: String,                  // ISO date, e.g. "2026-03-15"
    pub updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    User,
    Feedback,
    Project,
    Reference,
}
```

---

## 8. 模块划分

### 8.1 新增模块

```
src-tauri/src/
├── agent_config.rs       # [新增] 数据结构定义（AgentConfig, AgentDefinition, MemoryMeta）
├── agent_loader.rs       # [新增] Agent 加载器
│                         #   - load_agent(id) → AgentDefinition
│                         #   - list_agents() → Vec<AgentSummary>
│                         #   - parse_frontmatter(md) → (config, body)
│                         #   - ensure_default_agent() → 首次运行创建默认 Agent
│
├── system_prompt.rs      # [新增] 系统提示词组装器
│                         #   - build_system_prompt(definition, runtime) → String
│                         #   - build_identity_section(config) → String
│                         #   - build_tools_section(tools, filter) → String
│                         #   - build_skills_section(skills, filter) → String
│                         #   - build_memory_section(index) → String
│                         #   - build_runtime_section(env) → String
│                         #   - truncate_content(text, limit) → String
│
├── memory.rs             # [新增] 记忆系统
│                         #   - load_memory_index(agent_id) → String
│                         #   - save_memory(agent_id, meta, content) → Result
│                         #   - delete_memory(agent_id, name) → Result
│                         #   - list_memories(agent_id) → Vec<MemoryMeta>
│                         #   - update_memory_index(agent_id) → Result
```

### 8.2 需修改的现有模块

```
src-tauri/src/
├── agent.rs              # 修改: AssistantAgent 接受 AgentDefinition 构建
│                         #   - new_from_definition(definition, provider) 方法
│                         #   - 系统提示词改为由 system_prompt.rs 生成
│
├── lib.rs                # 修改: 新增 Tauri 命令注册
│                         #   - AppState 增加 current_agent_id 字段
│                         #   - 注册 Agent 管理 + 记忆管理命令
│
├── paths.rs              # 修改: 增加 agents_dir() / memory_dir() 路径函数
```

---

## 9. Tauri 命令接口

### 9.1 Agent 管理

```rust
/// 列出所有可用 Agent
#[tauri::command]
async fn list_agents(state: State<'_, AppState>) -> Result<Vec<AgentSummary>, String>;

/// 获取 Agent 完整定义
#[tauri::command]
async fn get_agent(id: String) -> Result<AgentDefinition, String>;

/// 切换当前活跃 Agent（重建 AssistantAgent + 系统提示词）
#[tauri::command]
async fn switch_agent(id: String, state: State<'_, AppState>) -> Result<(), String>;

/// 创建新 Agent（生成目录 + AGENT.md）
#[tauri::command]
async fn create_agent(id: String, config: AgentConfig) -> Result<(), String>;

/// 更新 Agent 配置（重写 AGENT.md frontmatter）
#[tauri::command]
async fn update_agent(id: String, config: AgentConfig) -> Result<(), String>;

/// 删除 Agent（移除整个目录）
#[tauri::command]
async fn delete_agent(id: String) -> Result<(), String>;
```

### 9.2 记忆管理

```rust
/// 列出记忆条目（指定 agent 私有记忆或全局记忆）
#[tauri::command]
async fn list_memories(scope: MemoryScope) -> Result<Vec<MemoryMeta>, String>;

/// 读取单条记忆内容
#[tauri::command]
async fn read_memory(scope: MemoryScope, name: String) -> Result<String, String>;

/// 保存记忆（创建或更新）
#[tauri::command]
async fn save_memory(scope: MemoryScope, meta: MemoryMeta, content: String) -> Result<(), String>;

/// 删除记忆
#[tauri::command]
async fn delete_memory(scope: MemoryScope, name: String) -> Result<(), String>;

// scope 枚举
enum MemoryScope {
    Global,                     // ~/.opencomputer/memory/
    Agent(String),              // ~/.opencomputer/agents/{id}/memory/
}
```

---

## 10. 流程变更

### 10.1 当前流程 → 重构后流程

```
当前:
  app 启动 → try_restore_session()
           → set_active_model(provider, model)
           → AssistantAgent::new(硬编码 SYSTEM_PROMPT)
           → chat()

重构后:
  app 启动 → try_restore_session()
           → agent_loader::ensure_default_agent()     # 首次运行创建默认 Agent
           → agent_loader::load_agent(current_id)      # 加载 Agent 定义
           → system_prompt::build(definition, runtime) # 组装系统提示词
           → AssistantAgent::new_from_definition(...)  # 创建 Agent 实例
           → chat()

  切换 Agent:
  switch_agent(id) → agent_loader::load_agent(id)
                   → system_prompt::build(...)
                   → AssistantAgent::new_from_definition(...)
                   → AppState.agent = new_agent
                   → 清空对话历史（新 Agent 新会话）
```

### 10.2 向后兼容

- `~/.opencomputer/agents/` 不存在时，自动创建 `default/AGENT.md`
- `config.json` 中的 `activeModel` 继续作为默认模型源
- 现有技能系统完全保留，Agent 只做过滤层
- 未配置 Agent 时行为与当前版本一致

---

## 11. 项目级 Agent 覆盖

### 11.1 机制

在项目根目录放置 `.opencomputer/AGENT.md`，可追加或覆盖当前 Agent 的指令：

```
~/Projects/my-app/
└── .opencomputer/
    └── AGENT.md    # 项目级 Agent 指令
```

### 11.2 合并规则

| 部分 | 规则 |
|------|------|
| frontmatter 配置 | 项目级深度合并覆盖 Agent 级（如 `tools.deny` 合并） |
| 正文指令 | 追加到 Agent 指令之后，用 `# Project Context` 标题分隔 |

### 11.3 加载时机

`system_prompt::build()` 在组装时检测当前工作目录是否包含 `.opencomputer/AGENT.md`，如存在则合并加载。同时兼容 `CLAUDE.md` 文件。

---

## 12. 与 OpenClaw 的对比

| 维度 | OpenClaw | OpenComputer（本设计） |
|------|---------|---------------------|
| Agent 定义格式 | JSON5 config + 多个独立 .md | AGENT.md 一体化（frontmatter + body） |
| 配置复杂度 | ~287 行类型定义，30+ 配置项 | ~50 行类型定义，核心配置精简 |
| 运行时 | TypeScript / Node.js | Rust / Tauri |
| 记忆存储 | SQLite + embeddings + 混合搜索 | 纯 Markdown 文件（先期） |
| Prompt 组装 | 20+ 独立 section 函数 | 10 个模块化 section |
| 多 Agent | 子 Agent 树、并发、session 管理 | 单活跃 Agent 切换（先期） |
| 截断策略 | 头 70% + 尾 20% | 相同 |
| Bootstrap | 7 种文件类型 + hook 系统 | 4 种文件类型（AGENT/SOUL/USER/TOOLS） |

---

## 13. 实施路线

### Phase 1: 基础框架（当前）

- [ ] `agent_config.rs` — 数据结构定义
- [ ] `agent_loader.rs` — AGENT.md 解析 + 加载
- [ ] `system_prompt.rs` — 系统提示词组装（替换硬编码）
- [ ] `paths.rs` — 新增路径函数
- [ ] `lib.rs` — 注册 `list_agents` / `switch_agent` 命令
- [ ] 默认 AGENT.md 模板生成

### Phase 2: 记忆系统

- [ ] `memory.rs` — 记忆文件 CRUD
- [ ] Agent 对话中自动识别 + 保存记忆
- [ ] 记忆加载注入系统提示词
- [ ] `lib.rs` — 注册记忆管理命令

### Phase 3: 前端 UI

- [ ] Agent 列表 / 切换 UI
- [ ] Agent 创建 / 编辑向导
- [ ] 记忆管理面板
- [ ] AGENT.md 在线编辑器

### Phase 4: 高级功能

- [ ] 项目级 `.opencomputer/AGENT.md` 覆盖
- [ ] 记忆向量检索（SQLite + embeddings）
- [ ] Agent 间共享上下文
- [ ] 子 Agent 支持

---

## 14. 开放问题

> 以下问题待讨论确认后更新文档。

1. **Agent 切换时是否保留对话历史？** 当前设计是切换即清空，但也可以按 Agent 分别保存历史。
2. **记忆自动保存的触发时机？** OpenClaw 在 compaction 前触发，我们是否需要类似机制？
3. **是否需要 Agent 导入/导出？** 让用户分享 Agent 定义（打包整个目录）。
4. **AGENT.md 的 frontmatter 解析库选择？** 考虑 `serde_yaml` + 手动 `---` 分割 vs 使用 `gray_matter` crate。
5. **记忆文件的大小限制？** 单条记忆是否应有字数上限？
