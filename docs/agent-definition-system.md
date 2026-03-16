# OpenComputer Agent 定义系统设计文档

> 版本: 0.3
> 日期: 2026-03-15
> 状态: 设计阶段

## 1. 概述

设计一套 Agent 定义系统，让用户可以通过 GUI 可视化配置 + Markdown 编写来定义 Agent 的身份、行为、记忆和能力边界。

### 设计原则

- **结构化配置与自然语言分离** — 配置数据用 JSON（`agent.json`），描述性内容用 Markdown，各司其职
- **GUI 优先** — 普通用户通过可视化界面完成所有配置，高级用户可直接编辑文件
- **Rust 原生** — 所有解析和组装逻辑在 Rust 侧完成
- **渐进式复杂度** — 一个 `agent.json` 即可定义一个 Agent，Markdown 文件按需添加
- **兼容现有架构** — 复用 `ProviderStore`、`SkillSystem`、`ToolSystem`

### 参考实现

本设计参考了 [OpenClaw](https://github.com/nichochar/open-claw) 的系统提示词组装、Agent 配置和记忆系统，并根据 OpenComputer 的桌面应用场景和 Rust 技术栈做了简化适配。

---

## 2. 目录结构

```
~/.opencomputer/
├── config.json                    # 全局配置（Provider、activeModel 等，已有）
├── user.json                      # [新增] 用户个人配置（全局，跨 Agent 共享）
├── credentials/                   # OAuth 凭证（已有）
├── skills/                        # 技能目录（已有）
│
├── agents/                        # [新增] Agent 定义目录
│   ├── default/                   #   默认 Agent
│   │   ├── agent.json             #   结构化配置（核心，必需）
│   │   ├── agent.md               #   Agent 说明：干什么、如何工作（可选）
│   │   ├── persona.md             #   人设/性格/人格（可选）
│   │   ├── tools.md               #   工具使用指导（可选）
│   │   └── memory/                #   Agent 私有记忆
│   │       ├── index.json         #     记忆索引
│   │       └── *.md               #     记忆文件
│   │
│   ├── coder/                     #   示例：编程专家
│   │   ├── agent.json
│   │   ├── agent.md
│   │   └── memory/
│   │
│   └── researcher/                #   示例：研究助手
│       ├── agent.json
│       └── memory/
│
├── memory/                        # [新增] 全局共享记忆
│   ├── index.json                 #   全局记忆索引
│   └── *.md                       #   记忆文件
│
├── home/                          # Agent 工作目录（已有）
└── share/                         # 跨 Agent 共享目录（已有）
```

### 关键约定

- 每个 Agent 是 `agents/` 下的一个子目录，目录名即 Agent ID
- `agent.json` 是唯一必需文件，Markdown 文件均为可选增强
- 用户信息是**全局配置**（`user.json`），不属于单个 Agent
- 目录名使用小写字母、数字和连字符（如 `code-reviewer`）

---

## 3. agent.json — Agent 配置文件

`agent.json` 是 Agent 的核心配置，包含身份信息和所有结构化参数，全部可通过前端 GUI 可视化编辑。

```json5
{
  // === 身份信息 ===
  "name": "默认助手",
  "description": "通用 AI 助手，擅长编程、写作和问题解决",
  "emoji": "🤖",
  "avatar": "",                          // 可选，头像文件路径或 URL

  // === 模型配置（可选，覆盖全局 activeModel）===
  "model": {
    "primary": "",                       // 留空则使用全局 activeModel
    "fallbacks": []                      // 备选模型列表
  },

  // === 能力过滤 ===
  "skills": {
    "allow": [],                         // 白名单（非空时仅加载列出的技能）
    "deny": []                           // 黑名单
  },
  "tools": {
    "allow": [],                         // 白名单（如 ["exec", "read", "write"]）
    "deny": []                           // 黑名单（如 ["web_search"]）
  },

  // === 记忆配置 ===
  "memory": {
    "enabled": true,                     // 是否启用记忆系统
    "shared": true,                      // 是否加载全局共享记忆
    "autoFlush": true                    // 对话接近上下文限制时自动保存记忆
  },

  // === 行为配置 ===
  "behavior": {
    "maxToolRounds": 10,                 // 工具循环最大轮数
    "requireApproval": ["exec"],         // 需要用户审批的工具
    "sandbox": false                     // 是否默认使用 Docker 沙箱
  }
}
```

### 设计说明

- 所有字段均为可选，缺省时使用合理默认值（见第 7 节）
- 字段命名采用 camelCase，与现有 `config.json` 风格一致
- 身份信息（name / description / emoji / avatar）直接在 JSON 中配置，不需要单独文件
- 前端 GUI 读写此文件，用户无需了解 JSON 语法

---

## 4. user.json — 用户个人配置（全局）

用户信息是全局的，所有 Agent 共享同一份用户画像。存储在 `~/.opencomputer/user.json`。

```json5
{
  // === 基本信息 ===
  "name": "",                            // 用户姓名/昵称
  "role": "",                            // 角色，如 "全栈开发者"
  "timezone": "",                        // 时区，如 "Asia/Shanghai"
  "language": "",                        // 首选语言，如 "zh-CN"

  // === 技术背景（可选，帮助 Agent 调整沟通方式）===
  "techStack": [],                       // 如 ["Rust", "TypeScript", "React"]
  "experience": "",                      // 如 "senior", "junior", "student"

  // === 协作偏好 ===
  "preferences": {
    "responseStyle": "",                 // "concise" | "detailed" | ""
    "codeCommentLang": ""                // 代码注释语言，如 "en", "zh"
  }
}
```

### 设计说明

- 与 `config.json` 分离存储：`config.json` 管 Provider / 模型等应用配置，`user.json` 管用户个人信息
- 所有字段可选，空值时 Agent 不做假设
- 前端在「设置 → 个人信息」中提供表单编辑
- Agent 组装系统提示词时读取此文件，生成用户上下文段

---

## 5. Markdown 描述文件

Markdown 文件用于编写 **自然语言内容**，会被组装进系统提示词。每种文件职责明确，互不重叠。

### 5.1 agent.md — Agent 说明

描述这个 Agent 是干什么的、如何工作。这是 Agent 最核心的自然语言定义。

```markdown
# 默认助手

你是一个智能助手，运行在 OpenComputer 桌面应用中。

## 核心原则
- 简洁直接，先行动再解释
- 安全第一，不执行高危操作前先确认
- 尊重用户偏好和工作习惯

## 工作方式
- 代码修改前先阅读现有代码
- 优先编辑现有文件而非创建新文件
- 保持最小改动原则
- 遇到不确定的事情坦诚说明，主动询问

## 擅长领域
- 软件开发（编码、调试、代码审查）
- 技术写作（文档、README、注释）
- 问题分析与解决方案设计
```

### 5.2 persona.md — 人设 / 性格

定义 Agent 的人格特质和沟通风格。与 agent.md 互补：agent.md 说"做什么"，persona.md 说"以什么性格做"。

```markdown
# 性格特质

- 专业但友善，像一个靠谱的同事
- 直接不啰嗦，不说废话
- 有自己的观点，但尊重用户决策

# 沟通风格

- 中文交流时使用简体中文
- 技术术语保持英文原文
- 不在回复末尾做重复总结
- 用代码和例子说话，少用抽象描述

# 边界

- 不做超出请求范围的"改进"
- 不主动推销功能或建议
- 承认不知道的事情
```

### 5.3 tools.md — 工具使用指导

为 Agent 提供工具使用的额外指导，补充或覆盖默认行为。

```markdown
# exec 工具

- 优先使用 brew 安装软件
- git 操作不要使用 --force 除非用户明确要求
- 长时间运行的命令使用 background 模式

# write/edit 工具

- 新文件使用 UTF-8 编码
- 遵循项目已有的代码风格
```

### 文件职责对照

| 文件 | 存什么 | 谁来写 | GUI 编辑方式 |
|------|--------|--------|-------------|
| `agent.json` | 身份 + 结构化配置 | 用户（GUI 表单） | 表单 / 开关 / 下拉框 |
| `agent.md` | Agent 做什么、怎么工作 | 用户（Markdown） | Markdown 编辑器 |
| `persona.md` | Agent 的性格和沟通风格 | 用户（Markdown） | Markdown 编辑器 |
| `tools.md` | 工具使用的额外指导 | 用户（Markdown） | Markdown 编辑器 |
| `user.json` | 用户个人信息（全局） | 用户（GUI 表单） | 表单输入 |

---

## 6. 记忆系统

### 6.1 设计理念

记忆内容用 **Markdown 文件** 存储（人类可读可编辑），记忆索引用 **JSON 文件** 管理（结构化、前端可直接读写）。先期不实现向量检索，仅在会话开始时加载索引内容到系统提示词。

### 6.2 记忆类型

| 类型 | 用途 | 示例 |
|------|------|------|
| `user` | 用户角色、目标、知识背景 | 用户是全栈开发者 |
| `feedback` | 用户对 Agent 行为的纠正和偏好 | 不需要在回复末尾做总结 |
| `project` | 项目状态、目标、决策 | v2 迁移进行中，冻结非关键 PR |
| `reference` | 外部资源指针 | Bug 跟踪在 Linear "INGEST" 项目 |

### 6.3 记忆索引 — index.json

```json5
{
  "version": 1,
  "entries": [
    {
      "name": "user_role",
      "description": "用户是一名全栈开发者，擅长 Rust 和 TypeScript",
      "type": "user",
      "file": "user_role.md",
      "created": "2026-03-15",
      "updated": "2026-03-15"
    },
    {
      "name": "feedback_no_summary",
      "description": "不需要在每次操作后总结",
      "type": "feedback",
      "file": "feedback_no_summary.md",
      "created": "2026-03-15",
      "updated": "2026-03-15"
    }
  ]
}
```

### 6.4 记忆文件（Markdown）

记忆内容仍用 Markdown 存储，保持人类可读：

```markdown
用户是一名全栈开发者，当前主要在开发 OpenComputer 项目。
熟悉 Rust、TypeScript、React、Tauri 技术栈。
偏好简洁的代码风格和直接的沟通方式。
```

元数据全部存在 `index.json` 中，Markdown 文件只存纯内容，无 frontmatter。

### 6.5 记忆操作

| 操作 | 触发方式 | 行为 |
|------|---------|------|
| 创建 | Agent 主动识别 / 用户说"记住..." | 创建 `memory/{name}.md` + 更新 `index.json` |
| 更新 | Agent 发现信息变化 / 用户指令 | 修改 `.md` 文件 + 更新 `index.json` 的 `updated` |
| 删除 | 用户说"忘记..." | 删除 `.md` 文件 + 从 `index.json` 移除条目 |
| 加载 | 每次会话开始 | 读取 `index.json`，生成记忆摘要注入系统提示词 |
| 自动保存 | 对话接近上下文限制时 | Agent 自动提取关键信息写入记忆（详见 6.7） |

### 6.6 记忆作用域

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

加载优先级：先加载全局共享记忆，再加载 Agent 私有记忆。冲突时私有记忆优先。

### 6.7 记忆自动保存

当对话接近上下文窗口限制时，自动触发记忆保存：

1. **触发条件**：已用 token 数达到上下文窗口的 80%
2. **保存流程**：
   - 向 Agent 发送内部指令，要求提取当前对话中值得长期记住的信息
   - Agent 判断哪些信息属于记忆类型（user / feedback / project / reference）
   - 创建或更新对应记忆文件 + 索引
3. **去重**：保存前检查 `index.json`，已存在的同名记忆走更新而非创建
4. **频率控制**：每轮对话最多触发一次自动保存

### 6.8 什么不该存为记忆

- 代码模式、架构、文件路径 — 可以从代码库直接读取
- Git 历史、谁改了什么 — `git log` / `git blame` 是权威来源
- 调试方案 — 修复已在代码中，上下文在 commit message 里
- `agent.json` / `agent.md` 中已有的内容 — 不要重复
- 临时任务状态、当前对话上下文 — 属于会话生命周期，不是记忆

---

## 7. 系统提示词组装

### 7.1 组装流程

```
┌──────────────────────────────────────────────────┐
│              System Prompt Assembly               │
│                                                   │
│  ① 基础身份行                                     │
│     "You are {agent.name}, running in             │
│      OpenComputer on {os} {arch}."                │
│                                                   │
│  ② agent.md                                       │
│     ← Agent 说明：干什么、如何工作（如存在）         │
│                                                   │
│  ③ persona.md                                     │
│     ← 人设 / 性格 / 沟通风格（如存在）              │
│                                                   │
│  ④ 用户信息                                       │
│     ← user.json 生成的用户上下文段（全局）          │
│                                                   │
│  ⑤ tools.md                                       │
│     ← 工具使用的自定义指导（如存在）                 │
│                                                   │
│  ⑥ 工具定义段                                     │
│     ← tools.rs 中启用的工具描述                    │
│     ← 根据 agent.json tools.allow/deny 过滤       │
│                                                   │
│  ⑦ 技能段                                         │
│     ← skills.rs 加载的可用技能描述                  │
│     ← 根据 agent.json skills.allow/deny 过滤      │
│                                                   │
│  ⑧ 记忆段（如 memory.enabled=true）                │
│     ← 全局 index.json 摘要（如 shared=true）       │
│     ← Agent 私有 index.json 摘要                   │
│                                                   │
│  ⑨ 运行时信息                                     │
│     ← 当前日期 / 时区 / 工作目录 / OS / Shell       │
│                                                   │
│  ⑩ 项目上下文（如在项目目录中）                     │
│     ← .opencomputer/agent.md（项目级追加指令）      │
│     ← CLAUDE.md（兼容加载）                        │
│                                                   │
└──────────────────────────────────────────────────┘
```

### 7.2 截断策略

| 限制 | 默认值 | 说明 |
|------|--------|------|
| 单文件上限 | 20,000 字符 | 单个 .md 文件最大注入长度 |
| 总上下文预算 | 150,000 字符 | 所有注入内容的总预算 |
| 截断方式 | 头 70% + 尾 20% + 标记 | 超限文件保留头尾，中间插入 `[... truncated ...]` |
| 记忆摘要上限 | 5,000 字符 | index.json 生成的记忆摘要文本上限 |

### 7.3 项目级覆盖

在项目根目录放置 `.opencomputer/` 目录，可追加项目特定指令：

```
~/Projects/my-app/
└── .opencomputer/
    ├── agent.json             # 项目级配置覆盖（可选）
    └── agent.md               # 项目级指令追加（可选）
```

**合并规则**：

| 文件 | 规则 |
|------|------|
| `agent.json` | 项目级深度合并覆盖 Agent 级（如 `tools.deny` 合并去重） |
| `agent.md` | 追加到 Agent 指令之后，用 `# Project Context` 标题分隔 |
| `CLAUDE.md` | 兼容加载，等同于项目级 `agent.md` |

---

## 8. Rust 数据结构

### 8.1 Agent 配置（对应 agent.json）

```rust
// src-tauri/src/agent_config.rs

use serde::{Deserialize, Serialize};

/// Agent 配置，从 agent.json 反序列化
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    // 身份信息
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,

    // 结构化配置
    #[serde(default)]
    pub model: ModelConfig,
    #[serde(default)]
    pub skills: FilterConfig,
    #[serde(default)]
    pub tools: FilterConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub behavior: BehaviorConfig,
}

fn default_name() -> String { "Assistant".to_string() }

/// 模型选择
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelConfig {
    #[serde(default)]
    pub primary: Option<String>,          // "provider_id/model_id"
    #[serde(default)]
    pub fallbacks: Vec<String>,
}

/// 能力过滤器（通用，用于 skills 和 tools）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilterConfig {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

/// 记忆配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConfig {
    #[serde(default = "bool_true")]
    pub enabled: bool,
    #[serde(default = "bool_true")]
    pub shared: bool,
    #[serde(default = "bool_true")]
    pub auto_flush: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self { enabled: true, shared: true, auto_flush: true }
    }
}

/// 行为配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BehaviorConfig {
    #[serde(default = "default_max_rounds")]
    pub max_tool_rounds: u32,
    #[serde(default)]
    pub require_approval: Vec<String>,
    #[serde(default)]
    pub sandbox: bool,
}

fn default_max_rounds() -> u32 { 10 }

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            max_tool_rounds: 10,
            require_approval: vec!["exec".to_string()],
            sandbox: false,
        }
    }
}

fn bool_true() -> bool { true }
```

### 8.2 用户配置（对应 user.json）

```rust
/// 用户个人配置，全局共享
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserConfig {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub tech_stack: Vec<String>,
    #[serde(default)]
    pub experience: Option<String>,       // "senior" | "junior" | "student"
    #[serde(default)]
    pub preferences: UserPreferences,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserPreferences {
    #[serde(default)]
    pub response_style: Option<String>,   // "concise" | "detailed"
    #[serde(default)]
    pub code_comment_lang: Option<String>, // "en" | "zh"
}
```

### 8.3 Agent 完整定义（运行时）

```rust
/// 从文件系统加载的完整 Agent 定义
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    pub id: String,                       // 目录名
    pub dir: PathBuf,                     // Agent 目录绝对路径
    pub config: AgentConfig,              // agent.json
    pub agent_md: Option<String>,         // agent.md 内容
    pub persona: Option<String>,          // persona.md 内容
    pub tools_guide: Option<String>,      // tools.md 内容
    pub memory_index: Option<MemoryIndex>, // memory/index.json
}
```

### 8.4 记忆索引（对应 index.json）

```rust
/// 记忆索引文件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryIndex {
    pub version: u32,
    pub entries: Vec<MemoryEntry>,
}

/// 单条记忆的元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub name: String,
    pub description: String,
    #[serde(rename = "type")]
    pub memory_type: MemoryType,
    pub file: String,                     // 对应 .md 文件名
    pub created: String,                  // "2026-03-15"
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

## 9. 模块划分

### 9.1 新增 Rust 模块

```
src-tauri/src/
├── agent_config.rs       # [新增] 数据结构定义
│                         #   AgentConfig, UserConfig,
│                         #   AgentDefinition, MemoryIndex, MemoryEntry
│
├── agent_loader.rs       # [新增] Agent 加载器
│                         #   - load_agent(id) → AgentDefinition
│                         #   - list_agents() → Vec<AgentSummary>
│                         #   - save_agent_config(id, config) → Result
│                         #   - ensure_default_agent()
│                         #   - delete_agent(id) → Result
│
├── system_prompt.rs      # [新增] 系统提示词组装器
│                         #   - build(definition, user, runtime) → String
│                         #   - truncate(text, limit) → String
│                         #   各 section 内部构建函数
│
├── memory.rs             # [新增] 记忆系统
│                         #   - load_index(scope) → MemoryIndex
│                         #   - save_memory(scope, entry, content) → Result
│                         #   - delete_memory(scope, name) → Result
│                         #   - list_memories(scope) → Vec<MemoryEntry>
│                         #   - build_memory_summary(index) → String
│
├── user_config.rs        # [新增] 用户配置
│                         #   - load_user_config() → UserConfig
│                         #   - save_user_config(config) → Result
│                         #   - build_user_context(config) → String
```

### 9.2 需修改的现有模块

```
src-tauri/src/
├── agent.rs              # 修改: AssistantAgent 接受 AgentDefinition
│                         #   - new_from_definition() 方法
│                         #   - 系统提示词改为由 system_prompt.rs 生成
│
├── lib.rs                # 修改: 新增 Tauri 命令注册
│                         #   - AppState 增加 current_agent_id
│                         #   - 注册 Agent / 记忆 / 用户配置管理命令
│
├── paths.rs              # 修改: 增加 agents_dir() / memory_dir() / user_config_path()
```

---

## 10. Tauri 命令接口

### 10.1 Agent 管理

```rust
/// 列出所有可用 Agent（返回摘要信息）
#[tauri::command]
async fn list_agents() -> Result<Vec<AgentSummary>, String>;

/// 获取 Agent 配置
#[tauri::command]
async fn get_agent_config(id: String) -> Result<AgentConfig, String>;

/// 获取 Agent 的某个 Markdown 文件内容
#[tauri::command]
async fn get_agent_markdown(id: String, file: String) -> Result<Option<String>, String>;

/// 保存 Agent 配置（创建或更新 agent.json）
#[tauri::command]
async fn save_agent_config(id: String, config: AgentConfig) -> Result<(), String>;

/// 保存 Agent 的某个 Markdown 文件
#[tauri::command]
async fn save_agent_markdown(id: String, file: String, content: String) -> Result<(), String>;

/// 切换当前活跃 Agent
#[tauri::command]
async fn switch_agent(id: String, state: State<'_, AppState>) -> Result<(), String>;

/// 删除 Agent
#[tauri::command]
async fn delete_agent(id: String) -> Result<(), String>;

/// AgentSummary
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSummary {
    id: String,
    name: String,
    description: Option<String>,
    emoji: Option<String>,
    has_agent_md: bool,
    has_persona: bool,
    memory_count: usize,
}
```

### 10.2 用户配置

```rust
/// 获取用户配置
#[tauri::command]
async fn get_user_config() -> Result<UserConfig, String>;

/// 保存用户配置
#[tauri::command]
async fn save_user_config(config: UserConfig) -> Result<(), String>;
```

### 10.3 记忆管理

```rust
/// 列出记忆条目
#[tauri::command]
async fn list_memories(scope: MemoryScope) -> Result<Vec<MemoryEntry>, String>;

/// 读取单条记忆内容
#[tauri::command]
async fn read_memory(scope: MemoryScope, name: String) -> Result<String, String>;

/// 保存记忆（创建或更新）
#[tauri::command]
async fn save_memory(
    scope: MemoryScope,
    entry: MemoryEntry,
    content: String,
) -> Result<(), String>;

/// 删除记忆
#[tauri::command]
async fn delete_memory(scope: MemoryScope, name: String) -> Result<(), String>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum MemoryScope {
    Global,
    Agent { id: String },
}
```

---

## 11. 流程变更

### 11.1 当前流程 → 重构后流程

```
当前:
  app 启动 → try_restore_session()
           → set_active_model(provider, model)
           → AssistantAgent::new(硬编码 SYSTEM_PROMPT)
           → chat()

重构后:
  app 启动 → try_restore_session()
           → agent_loader::ensure_default_agent()     # 首次运行创建默认 Agent
           → agent_loader::load_agent(current_id)      # 加载 agent.json + .md 文件
           → user_config::load_user_config()           # 加载全局用户配置
           → system_prompt::build(definition, user, runtime)
           → AssistantAgent::new_from_definition(...)
           → chat()

  切换 Agent:
  switch_agent(id) → agent_loader::load_agent(id)
                   → system_prompt::build(...)
                   → AssistantAgent::new_from_definition(...)
                   → AppState.agent = new_agent
                   → 前端开启新对话
```

### 11.2 向后兼容

- `~/.opencomputer/agents/` 不存在时，自动创建 `default/agent.json`
- `user.json` 不存在时，使用空默认值
- `config.json` 中的 `activeModel` 继续作为默认模型源
- 现有技能系统完全保留，Agent 只做过滤层

---

## 12. 前端 GUI 设计方向

### 12.1 Agent 管理页

- **Agent 列表**：卡片式展示所有 Agent，显示 emoji + 名称 + 描述
- **快速切换**：顶部下拉或侧栏切换当前活跃 Agent
- **新建 Agent**：引导式表单，填写基本信息 → 自动生成 `agent.json`

### 12.2 Agent 编辑页

分 Tab 组织：

| Tab | 内容 | 编辑方式 |
|-----|------|---------|
| 基本信息 | 名称、描述、emoji、头像 | 表单输入 |
| 模型 | 主模型、备选模型 | 下拉选择 |
| 能力 | 技能/工具 白名单/黑名单 | 多选开关 |
| 行为 | 工具循环轮数、审批规则、沙箱 | 表单 + 开关 |
| 记忆 | 启用/共享/自动保存 | 开关 |
| Agent 说明 | agent.md 内容 | Markdown 编辑器 |
| 人设 | persona.md 内容 | Markdown 编辑器 |
| 工具指导 | tools.md 内容 | Markdown 编辑器 |

### 12.3 用户设置页

在全局设置中增加「个人信息」Tab：

- 姓名、角色、时区、语言
- 技术栈（标签输入）
- 经验水平（下拉选择）
- 协作偏好（回复风格、代码注释语言）

### 12.4 记忆管理

- **记忆列表**：按类型分组，显示 name + description + 更新时间
- **记忆详情**：查看/编辑 Markdown 内容
- **手动添加**：选择类型 → 填写名称和描述 → 编写内容
- **作用域切换**：全局记忆 / 当前 Agent 私有记忆

---

## 13. 与 OpenClaw 的对比

| 维度 | OpenClaw | OpenComputer |
|------|---------|-------------|
| 配置格式 | JSON5 集中式 config | `agent.json` 每个 Agent 独立 |
| 描述内容 | .md（带 YAML frontmatter） | .md（纯 Markdown） |
| 用户信息 | USER.md（per-agent） | user.json（全局，所有 Agent 共享） |
| 人设文件 | SOUL.md | persona.md |
| 记忆索引 | MEMORY.md | index.json |
| 配置编辑 | 手动编辑 JSON5 | GUI 可视化 + 文件直编 |
| 复杂度 | ~287 行类型，30+ 配置项 | ~80 行类型，核心配置精简 |
| 运行时 | TypeScript / Node.js | Rust / Tauri |
| 记忆存储 | SQLite + embeddings | 纯文件（先期） |
| 多 Agent | 子 Agent 树、并发 | 单活跃 Agent 切换（先期） |

---

## 14. 实施路线

### Phase 1: Agent 定义基础

- [ ] `agent_config.rs` — 数据结构（AgentConfig, AgentDefinition）
- [ ] `user_config.rs` — 用户配置（UserConfig）
- [ ] `agent_loader.rs` — agent.json + .md 文件加载
- [ ] `system_prompt.rs` — 提示词组装（替换硬编码 SYSTEM_PROMPT）
- [ ] `paths.rs` — agents_dir() / user_config_path()
- [ ] `lib.rs` — 注册 Agent / 用户配置管理命令
- [ ] 默认 agent.json 模板

### Phase 2: 记忆系统

- [ ] `memory.rs` — 记忆 CRUD + index.json 管理
- [ ] 记忆加载 → 系统提示词注入
- [ ] 记忆自动保存触发机制
- [ ] `lib.rs` — 注册记忆管理命令

### Phase 3: 前端 GUI

- [ ] Agent 列表 / 切换 UI
- [ ] Agent 配置表单（agent.json 可视化编辑）
- [ ] Markdown 编辑器（agent.md / persona.md / tools.md）
- [ ] 用户设置页（user.json 可视化编辑）
- [ ] 记忆管理面板

### Phase 4: 高级功能

- [ ] 项目级 `.opencomputer/agent.json` + `agent.md` 覆盖
- [ ] 对话历史按 Agent 保存与回溯
- [ ] 记忆容量管理（到达上限提醒用户压缩或清理）
- [ ] 记忆向量检索（SQLite + embeddings）
- [ ] Agent 间共享上下文
- [ ] 子 Agent 支持

---

## 15. 已确认的决策

| 编号 | 问题 | 决策 |
|------|------|------|
| Q1 | Agent 切换时对话历史怎么处理？ | 完整对话历史保存并可回溯，后续设计存储方案 |
| Q2 | 记忆自动保存的触发时机？ | 对话接近上下文限制时触发（80% 阈值），每轮最多一次 |
| Q3 | 是否需要 Agent 导入/导出？ | 暂不考虑 |
| Q4 | 配置数据用什么格式？ | JSON（agent.json, index.json, user.json），结构化数据统一用 JSON |
| Q5 | 记忆文件大小限制？ | 不同类型不同策略，长期记忆有上限，到达后提醒用户选择压缩或清理 |
| Q6 | 用户信息放哪里？ | 全局 user.json，不属于单个 Agent |
| Q7 | Agent 身份信息怎么存？ | 结构化字段放 agent.json（name/description/emoji/avatar） |
| Q8 | 人设文件叫什么？ | persona.md — AI Agent 语境下最直觉的命名 |

---

## 16. 待讨论的问题

1. **默认 Agent 的 agent.md 内容**：需要设计一份通用的默认系统指令模板
2. **Agent 的对话历史存储格式**：JSON / SQLite / 其他？
3. **记忆各类型的具体容量上限**：user 类 / feedback 类 / project 类分别多少？
4. **项目级覆盖的检测逻辑**：如何确定"当前项目目录"？用 cwd 还是 git root？
5. **user.json 和 config.json 是否合并**：还是保持独立更清晰？
