# Agent 定义系统 — 开发交接文档

> 交接时间: 2026-03-17
> 当前进度: Phase 1 后端已完成，下一步是前端 Agent 管理 UI

## 1. 项目背景

OpenComputer 正在实现一套 **Agent 定义系统**，允许用户通过 GUI 可视化配置来定义多个 AI Agent（身份、行为、人设、能力边界），并动态组装系统提示词。

设计文档: `docs/agent-definition-system.md`（v0.3）

### 核心设计原则

- **结构化配置用 JSON（agent.json），自然语言用 Markdown（agent.md / persona.md / tools.md）**
- **GUI 优先**：用户通过表单配置，不感知底层文件格式
- **记忆系统暂不实现**：将单独设计

---

## 2. 已完成的工作

### 2.1 用户个人配置（完整）

**后端** `src-tauri/src/user_config.rs`

```rust
pub struct UserConfig {
    pub name: Option<String>,           // 昵称/姓名
    pub avatar: Option<String>,         // 头像路径/URL
    pub gender: Option<String>,         // 性别: male/female/自定义文本
    pub age: Option<u32>,               // 年龄
    pub role: Option<String>,           // 角色/职业
    pub timezone: Option<String>,       // IANA 时区（null=跟随系统）
    pub language: Option<String>,       // 首选语言（null=跟随系统）
    pub ai_experience: Option<String>,  // AI经验: expert/intermediate/beginner
    pub response_style: Option<String>, // 回复风格: concise/detailed/自定义文本
    pub custom_info: Option<String>,    // 自由补充信息
}
```

- `load_user_config()` / `save_user_config_to_disk()` — 读写 `~/.opencomputer/user.json`
- `build_user_context()` — 生成系统提示词用户段

**Tauri 命令**: `get_user_config` / `save_user_config` / `get_system_timezone`

**前端** `src/components/SettingsView.tsx` — `UserProfilePanel` 组件：
- 头像（圆形，居中，Camera 图标，文件选择器 TODO）
- 昵称/姓名、性别（男/女/自定义）、年龄、角色
- AI 使用经验（列表选择 + Check 图标）
- 时区/语言（均有"跟随系统"选项，时区有多语言显示名）
- 回复风格（简洁/详细/自定义 textarea）
- 补充说明（textarea）
- 保存按钮固定右下角
- 所有文本输入支持 IME 中文输入（compositionStart/End 处理）

**侧边栏**: `src/App.tsx` — 个人信息按钮在主题切换上方

### 2.2 Agent 定义后端（完整）

#### `src-tauri/src/agent_config.rs` — 数据结构

```rust
pub struct AgentConfig {          // 对应 agent.json
    pub name: String,             // 默认 "Assistant"
    pub description: Option<String>,
    pub emoji: Option<String>,
    pub avatar: Option<String>,
    pub model: AgentModelConfig,  // { primary, fallbacks }
    pub skills: FilterConfig,     // { allow, deny }
    pub tools: FilterConfig,      // { allow, deny }
    pub behavior: BehaviorConfig, // { maxToolRounds, requireApproval, sandbox }
}

pub struct AgentDefinition {      // 运行时完整定义
    pub id: String,               // 目录名
    pub dir: PathBuf,
    pub config: AgentConfig,
    pub agent_md: Option<String>,     // agent.md
    pub persona: Option<String>,      // persona.md
    pub tools_guide: Option<String>,  // tools.md
}

pub struct AgentSummary { ... }   // 前端列表用
pub struct FilterConfig { allow, deny, is_allowed() }
pub struct BehaviorConfig { max_tool_rounds, require_approval, sandbox }
```

#### `src-tauri/src/agent_loader.rs` — 文件操作

| 函数 | 作用 |
|------|------|
| `ensure_default_agent()` | 启动时创建 `~/.opencomputer/agents/default/`（agent.json + agent.md） |
| `load_agent(id)` | 加载 agent.json + 可选 .md 文件 → AgentDefinition |
| `list_agents()` | 遍历 agents/ 返回 Vec<AgentSummary>，default 排首位 |
| `save_agent_config(id, config)` | 写 agent.json |
| `save_agent_markdown(id, file, content)` | 写 agent.md/persona.md/tools.md（有路径校验） |
| `get_agent_markdown(id, file)` | 读 .md 文件 |
| `delete_agent(id)` | 删除 Agent 目录（拒绝删 default） |

#### `src-tauri/src/system_prompt.rs` — 提示词组装

**`build(definition)` — 10 段模块化组装**:
```
① "You are {name}, running in OpenComputer on {os} {arch}."
② agent.md 内容
③ persona.md 内容
④ 用户信息（user_config::build_user_context）
⑤ tools.md 内容
⑥ 工具定义（内置 11 个工具描述，根据 FilterConfig 过滤）
⑦ 技能（根据 FilterConfig 过滤）
⑧ 记忆（预留，未实现）
⑨ 运行时信息（日期/cwd/shell）
⑩ 项目上下文（预留，未实现）
```

- `build_legacy()` — 向后兼容旧版提示词
- `truncate(text, max)` — 头 70% + 尾 20% + 截断标记

#### 修改的现有模块

- **`agent.rs`**: `build_system_prompt()` 委托给 `system_prompt` 模块，删除了 `SYSTEM_PROMPT_BASE` 常量
- **`lib.rs`**: AppState 增加 `current_agent_id: Mutex<String>`，注册 6 个 Agent 命令，启动时调用 `ensure_default_agent()`
- **`paths.rs`**: 新增 `agents_dir()` / `agent_dir(id)`，`ensure_dirs()` 创建 agents/

**已注册的 Tauri 命令**:
- `list_agents` / `get_agent_config` / `get_agent_markdown`
- `save_agent_config_cmd` / `save_agent_markdown` / `delete_agent`

---

## 3. 下一步：前端 Agent 管理 UI

### 3.1 设置页新增 "Agent" section

在 `SettingsView.tsx` 的 `SECTIONS` 数组和侧栏中添加 Agent 入口。

需要新增的面板组件（参考现有 `SkillsPanel` / `UserProfilePanel` 风格）：

**Agent 列表页**:
- 调用 `list_agents` 获取 AgentSummary 数组
- 卡片展示：emoji + 名称 + 描述
- 「新建 Agent」按钮
- 点击进入编辑页

**Agent 编辑页**（分 Tab 或分区）:

| 区块 | 数据来源 | 编辑方式 |
|------|---------|---------|
| 基本信息 | agent.json: name/description/emoji/avatar | 表单 |
| 模型 | agent.json: model.primary | 下拉选择（复用现有模型选择器） |
| 能力 | agent.json: skills/tools allow/deny | 多选开关 |
| 行为 | agent.json: behavior | 表单 + 开关 |
| Agent 说明 | agent.md | textarea |
| 人设 | persona.md | textarea |
| 工具指导 | tools.md | textarea |

调用的命令：
- 读取: `get_agent_config(id)` + `get_agent_markdown(id, "agent.md")` 等
- 保存: `save_agent_config_cmd(id, config)` + `save_agent_markdown(id, file, content)`
- 删除: `delete_agent(id)`

### 3.2 聊天界面 Agent 切换

在聊天界面顶部或侧栏添加 Agent 选择器。可以延后，先做设置页管理。

### 3.3 后续 Phase

- **Phase 2**: 记忆系统（单独设计）
- **Phase 3**: 项目级覆盖（`.opencomputer/agent.json` + `agent.md`）
- **Phase 4**: 对话历史按 Agent 保存、子 Agent

---

## 4. 代码架构

```
lib.rs (Tauri 命令 + AppState)
  ├── agent.rs          (AssistantAgent + LLM 调用，build_system_prompt 委托给↓)
  ├── system_prompt.rs  [新] 提示词组装（build / build_legacy / truncate）
  ├── agent_config.rs   [新] 数据结构（AgentConfig / AgentDefinition / AgentSummary）
  ├── agent_loader.rs   [新] Agent 文件 CRUD
  ├── user_config.rs    [新] 用户配置
  ├── tools.rs          11 个工具定义 + 执行
  ├── skills.rs         技能加载 + 提示词注入
  ├── provider.rs       ProviderStore + config.json 持久化
  ├── paths.rs          路径管理
  ├── oauth.rs          Codex OAuth
  ├── process_registry.rs  后台进程管理
  └── sandbox.rs        Docker 沙箱
```

### 关键文件行数参考

| 文件 | 行数 |
|------|------|
| `agent.rs` | ~1450 |
| `tools.rs` | ~2927 |
| `system_prompt.rs` | ~210 |
| `agent_config.rs` | ~175 |
| `agent_loader.rs` | ~210 |
| `user_config.rs` | ~115 |
| `SettingsView.tsx` | ~1000 |

### 编码风格

- Rust: `serde(rename_all = "camelCase")`、`anyhow::Result`
- 前端: 函数式组件 + hooks、Tailwind、`@/` 别名
- i18n: `settings.profileXxx` / `settings.agentXxx` 命名空间
- JSON: camelCase

---

## 5. 注意事项

- **IME 输入**: 所有文本 input/textarea 需用 `textInputProps()` 辅助函数（compositionStart/End），否则无法输入中文。已有实现在 `UserProfilePanel` 中可参考
- **头像**: 文件选择器未实现，标记 `// TODO: file picker for avatar`
- **默认 Agent**: 首次启动自动创建 `~/.opencomputer/agents/default/`，不可删除
- **向后兼容**: `build_system_prompt()` 先尝试加载 default agent definition，失败则 fallback 到 `build_legacy()`
- **记忆系统**: 设计文档中有 MemoryConfig，本阶段全部跳过

---

## 6. 设计决策记录

| 决策 | 原因 |
|------|------|
| 配置 JSON + 描述 Markdown | 结构化数据 GUI 友好；自然语言 Markdown 人类友好 |
| 用户信息全局 user.json | 用户身份不随 Agent 变，所有 Agent 共享 |
| persona.md 命名 | AI Agent 语境下最直觉 |
| 去掉经验水平，保留 AI 经验 | 经验水平不通用，AI 经验帮助调整沟通方式 |
| 增加性别/年龄 | 通用个人信息，性别支持自定义 |
| 时区/语言支持跟随系统 | 默认无需手动设置 |
| 回复风格可自定义 | 预设不够用时有兜底 |
| 隐私提示 | 所有字段可选，由用户自行决策 |
| 记忆系统单独设计 | 复杂度高，独立周期 |

---

## 7. 相关资源

| 资源 | 位置 |
|------|------|
| 设计文档 | `docs/agent-definition-system.md` |
| 本交接文档 | `docs/handoff-agent-system.md` |
| 项目说明 | `CLAUDE.md` |
| OpenClaw 参考 | `~/Codes/openclaw`（`src/agents/system-prompt.ts`） |
