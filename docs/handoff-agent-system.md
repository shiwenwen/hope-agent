# Agent 定义系统 — 开发交接文档

> 交接时间: 2026-03-16
> 当前进度: Phase 1 进行中（用户配置已完成，Agent 定义 + 系统提示词组装待实现）

## 1. 项目背景

OpenComputer 正在实现一套 **Agent 定义系统**，允许用户通过 GUI 可视化配置来定义多个 AI Agent（身份、行为、人设、能力边界），并动态组装系统提示词。

设计方案参考了 OpenClaw（~/Codes/openclaw）的实现，详见设计文档 `docs/agent-definition-system.md`（v0.3）。

### 核心设计原则

- **结构化配置用 JSON（agent.json），自然语言用 Markdown（agent.md / persona.md / tools.md）**
- **GUI 优先**：普通用户通过表单配置，不感知底层文件格式
- **记忆系统暂不实现**：本阶段只做 Agent 定义 + 系统提示词组装

---

## 2. 已完成的工作

### 2.1 设计文档

**文件**: `docs/agent-definition-system.md`（v0.3，约 800 行）

包含完整的：
- 目录结构设计（`~/.opencomputer/agents/{id}/`）
- `agent.json` 配置格式（身份、模型、技能过滤、工具过滤、行为配置）
- Markdown 文件职责划分（agent.md / persona.md / tools.md）
- 系统提示词 10 段组装流程 + 截断策略
- Rust 数据结构定义（AgentConfig / AgentDefinition / MemoryIndex 等）
- Tauri 命令接口设计
- 前端 GUI 方向
- 4 个 Phase 的实施路线

### 2.2 用户个人配置（已完成）

#### Rust 后端

**新增文件**: `src-tauri/src/user_config.rs`

```rust
pub struct UserConfig {
    pub name: Option<String>,        // 昵称
    pub avatar: Option<String>,      // 头像路径/URL
    pub role: Option<String>,        // 角色/职业
    pub timezone: Option<String>,    // IANA 时区
    pub language: Option<String>,    // 首选语言
    pub experience: Option<String>,  // 经验水平: senior/mid/junior/student
    pub ai_experience: Option<String>, // AI经验: expert/intermediate/beginner
    pub response_style: Option<String>, // 回复风格: concise/detailed/自定义文本
    pub custom_info: Option<String>,  // 用户自由补充信息
}
```

关键函数：
- `load_user_config()` → 从 `~/.opencomputer/user.json` 读取，不存在返回默认值
- `save_user_config_to_disk(config)` → 写入 `~/.opencomputer/user.json`
- `build_user_context(config) → Option<String>` → 生成系统提示词的用户信息段（`# User\n\n- Name: ...\n- Role: ...`）

**修改文件**: `src-tauri/src/paths.rs`
- 新增 `user_config_path()` → `~/.opencomputer/user.json`

**修改文件**: `src-tauri/src/lib.rs`
- 新增 `mod user_config`
- 新增 3 个 Tauri 命令：
  - `get_user_config` → 读取用户配置
  - `save_user_config` → 保存用户配置
  - `get_system_timezone` → 获取系统 IANA 时区（读取 `/etc/localtime` 软链接）

#### 前端

**修改文件**: `src/components/SettingsView.tsx`
- 设置侧栏新增 "个人信息" 入口（User icon）
- 新增 `UserProfilePanel` 组件，包含：
  - 头部区域：头像 + 昵称/角色内联编辑
  - 经验区域：两列并排按钮选择（经验水平 + AI 使用经验）
  - 地区区域：两列并排下拉选择（时区分组 + 语言列表）
  - 回复风格：三按钮切换（简洁/详细/自定义），自定义展开 textarea
  - 补充说明：自由 textarea
  - 保存按钮（带成功状态反馈）
- 时区默认取系统时区，语言默认取当前 UI 语言

**修改文件**: `src/i18n/locales/zh.json` + `en.json`
- 新增约 30 条翻译 key（`settings.profile*` 命名空间）

### 2.3 注意：头像功能

头像按钮的文件选择器尚未实现（UI 有 hover 按钮但点击无操作），标记为 `// TODO: file picker for avatar`。需要用 Tauri 的 `tauri-plugin-dialog` 的 `open` API 来选择图片文件，可以考虑将选中的图片复制到 `~/.opencomputer/` 下统一管理。

---

## 3. 待实现的工作（按优先级排序）

### 3.1 Phase 1 剩余：Agent 定义基础

以下是 Phase 1 中尚未完成的核心工作：

#### 3.1.1 `agent_config.rs` — Agent 配置数据结构

**新建文件**: `src-tauri/src/agent_config.rs`

需要定义的结构体（参考设计文档第 8 节）：

```rust
// agent.json 对应的配置
pub struct AgentConfig {
    pub name: String,                 // 默认 "Assistant"
    pub description: Option<String>,
    pub emoji: Option<String>,
    pub avatar: Option<String>,
    pub model: AgentModelConfig,      // { primary, fallbacks }
    pub skills: FilterConfig,         // { allow, deny }
    pub tools: FilterConfig,          // { allow, deny }
    pub behavior: BehaviorConfig,     // { maxToolRounds, requireApproval, sandbox }
}

// 运行时加载的完整定义
pub struct AgentDefinition {
    pub id: String,                   // 目录名
    pub dir: PathBuf,
    pub config: AgentConfig,          // agent.json
    pub agent_md: Option<String>,     // agent.md 内容
    pub persona: Option<String>,      // persona.md 内容
    pub tools_guide: Option<String>,  // tools.md 内容
}
```

注意：设计文档中有 `MemoryConfig`，但本阶段不实现记忆，可以在 `AgentConfig` 中保留字段但跳过逻辑。

#### 3.1.2 `agent_loader.rs` — Agent 加载器

**新建文件**: `src-tauri/src/agent_loader.rs`

核心功能：
1. **`ensure_default_agent()`** — 首次运行时在 `~/.opencomputer/agents/default/` 创建默认 `agent.json`
2. **`load_agent(id) → AgentDefinition`** — 读取 `agent.json` + 可选的 `.md` 文件
3. **`list_agents() → Vec<AgentSummary>`** — 遍历 `~/.opencomputer/agents/` 子目录
4. **`save_agent_config(id, config)`** — 写入 `agent.json`
5. **`save_agent_markdown(id, file, content)`** — 写入 `.md` 文件
6. **`delete_agent(id)`** — 删除整个 Agent 目录

加载路径规则：
```
~/.opencomputer/agents/{id}/
├── agent.json        ← 必需，serde_json 反序列化
├── agent.md          ← 可选，读为 String
├── persona.md        ← 可选，读为 String
└── tools.md          ← 可选，读为 String
```

#### 3.1.3 `system_prompt.rs` — 系统提示词组装器

**新建文件**: `src-tauri/src/system_prompt.rs`

**这是最核心的模块**，替换现有 `agent.rs` 中硬编码的 `SYSTEM_PROMPT`。

组装流程（10 段，按顺序拼接）：

```
① 基础身份行 — "You are {name}, running in OpenComputer on {os} {arch}."
② agent.md 内容（如存在）
③ persona.md 内容（如存在）
④ 用户信息 — user_config::build_user_context() 结果
⑤ tools.md 内容（如存在）
⑥ 工具定义段 — 现有 tools.rs 的工具描述，根据 agent.json tools.allow/deny 过滤
⑦ 技能段 — 现有 skills.rs 的技能描述，根据 agent.json skills.allow/deny 过滤
⑧ 记忆段 — 本阶段跳过
⑨ 运行时信息 — 日期、时区、工作目录、OS、Shell
⑩ 项目上下文 — .opencomputer/agent.md 或 CLAUDE.md（本阶段可延后）
```

截断策略：
- 单文件上限：20,000 字符
- 总上下文预算：150,000 字符
- 超限文件：保留头 70% + 尾 20% + 中间插入 `[... truncated ...]`

**关键参考**：现有系统提示词在 `agent.rs` 的 `SYSTEM_PROMPT` 常量和 `build_system_prompt()` 函数中（约第 81-107 行）。重构后 `agent.rs` 应调用 `system_prompt::build()` 而非自己拼接。

#### 3.1.4 修改现有模块

**`agent.rs`**：
- 现有 `AssistantAgent::new()` 系列方法内部调用 `build_system_prompt()`
- 需要新增 `new_from_definition(definition, provider, ...)` 方法
- 或者修改现有 `build_system_prompt()` 使其接受 `AgentDefinition` 参数
- 系统提示词生成逻辑迁移到 `system_prompt.rs`

**`lib.rs`**：
- `AppState` 新增 `current_agent_id: Mutex<String>` 字段（默认 "default"）
- 注册新命令：`list_agents`, `get_agent_config`, `get_agent_markdown`, `save_agent_config`, `save_agent_markdown`, `switch_agent`, `delete_agent`
- 应用启动流程调整：调用 `agent_loader::ensure_default_agent()`

**`paths.rs`**：
- 新增 `agents_dir() → ~/.opencomputer/agents/`
- 新增 `agent_dir(id) → ~/.opencomputer/agents/{id}/`
- `ensure_dirs()` 中添加 agents 目录创建

#### 3.1.5 前端 GUI

需要新增的 UI 部分：
- **Agent 列表/切换**：可以在聊天界面顶部或侧栏添加 Agent 选择器
- **Agent 编辑页**：在设置页中新增 "Agent" section，包含：
  - 基本信息 Tab（名称、描述、emoji、头像 — 表单）
  - 模型 Tab（主模型下拉选择，复用现有模型选择器）
  - 能力 Tab（技能/工具的白名单黑名单 — 多选开关）
  - 行为 Tab（maxToolRounds、审批规则、沙箱 — 表单开关）
  - Agent 说明 Tab（agent.md — textarea 编辑器）
  - 人设 Tab（persona.md — textarea 编辑器）
  - 工具指导 Tab（tools.md — textarea 编辑器）

---

## 4. 代码架构指引

### 4.1 现有模块依赖关系

```
lib.rs (Tauri 命令注册 + AppState)
  ├── agent.rs (AssistantAgent + LLM 调用 + Tool Loop)
  │   ├── tools.rs (11 个工具定义 + 执行)
  │   ├── skills.rs (技能加载 + 提示词注入)
  │   └── provider.rs (ProviderStore + 持久化)
  ├── user_config.rs [新增] (用户配置)
  ├── paths.rs (路径管理)
  ├── oauth.rs (Codex OAuth)
  ├── process_registry.rs (后台进程管理)
  └── sandbox.rs (Docker 沙箱)
```

重构后新增：
```
lib.rs
  ├── agent_config.rs [新增] (数据结构)
  ├── agent_loader.rs [新增] (Agent 文件加载)
  ├── system_prompt.rs [新增] (提示词组装)
  ├── agent.rs (修改: 接受 AgentDefinition)
  └── ... (其余不变)
```

### 4.2 关键文件位置

| 文件 | 用途 | 行数 |
|------|------|------|
| `src-tauri/src/agent.rs` | AssistantAgent + LLM 调用 | ~1473 |
| `src-tauri/src/tools.rs` | 11 个工具定义 + Provider schema 适配 | ~2927 |
| `src-tauri/src/skills.rs` | 技能发现 + 系统提示词注入 | ~395 |
| `src-tauri/src/provider.rs` | ProviderStore + 模型配置 | ~390 |
| `src-tauri/src/user_config.rs` | 用户配置 [已完成] | ~113 |
| `src-tauri/src/lib.rs` | Tauri 命令注册 + AppState | ~990 |
| `src-tauri/src/paths.rs` | 路径管理 | ~83 |
| `src/components/SettingsView.tsx` | 设置页面（含用户个人信息） | ~930 |
| `docs/agent-definition-system.md` | 完整设计文档 | ~830 |

### 4.3 现有系统提示词位置

在 `agent.rs` 中搜索 `SYSTEM_PROMPT` 或 `build_system_prompt`：

- 约第 14-80 行：`SYSTEM_PROMPT` 常量（工具描述的基础提示词）
- 约第 81-107 行：`build_system_prompt()` 函数（拼接技能段）
- 约第 109-116 行：技能注入逻辑（`skills::build_skills_prompt()`）

重构时需要把这些逻辑迁移到 `system_prompt.rs`，并扩展为 10 段组装。

### 4.4 config.json 结构

现有 `~/.opencomputer/config.json` 是 `ProviderStore` 序列化的结果：

```json
{
  "providers": [...],       // ProviderConfig 数组
  "activeModel": { "providerId": "...", "modelId": "..." },
  "extraSkillsDirs": [...],
  "disabledSkills": [...]
}
```

Agent 系统不修改这个文件。Agent 配置独立存储在 `~/.opencomputer/agents/{id}/agent.json`。

### 4.5 编码风格

- Rust：`serde(rename_all = "camelCase")`、`anyhow::Result` 内部错误、命令边界转 `String`
- 前端：函数式组件 + hooks、Tailwind utility class、`@/` 路径别名
- i18n key：`settings.profileXxx` 格式，参考现有 `zh.json` / `en.json`
- 命名：Rust snake_case、JSON camelCase、TypeScript camelCase

---

## 5. 开发建议

### 5.1 推荐实现顺序

1. **`agent_config.rs`** — 纯数据结构，无依赖，5 分钟搞定
2. **`paths.rs`** — 加 `agents_dir()` + `agent_dir(id)`，2 分钟
3. **`agent_loader.rs`** — 文件读写，依赖 1 + 2
4. **`system_prompt.rs`** — 最复杂，依赖 1 + 3 + `user_config` + `skills` + `tools`
5. **修改 `agent.rs`** — 让 AssistantAgent 使用新的 system_prompt
6. **修改 `lib.rs`** — 注册命令 + 修改启动流程
7. **前端 Agent 管理 UI** — 最后做

### 5.2 关键注意事项

- **不要破坏现有功能**：重构系统提示词时，确保默认 Agent 的行为与当前硬编码版本一致
- **先做后端再做前端**：后端 API 稳定后前端才好对接
- **记忆系统暂不做**：设计文档中有记忆相关的结构定义，本阶段全部跳过
- **项目级覆盖暂不做**：`.opencomputer/agent.md` + `CLAUDE.md` 的加载可以延后
- **头像文件选择器**：`UserProfilePanel` 中头像更换按钮标记了 TODO，可以用 `tauri-plugin-dialog` 的 `open` 实现

### 5.3 测试验证

- 启动应用后检查 `~/.opencomputer/user.json` 是否正确读写
- 设置页面 → 个人信息 → 填写保存 → 刷新后数据应保持
- `cargo check` 确认 Rust 编译无错误
- `npx tsc --noEmit` 确认前端类型无错误

---

## 6. 相关资源

| 资源 | 位置 |
|------|------|
| 设计文档 | `docs/agent-definition-system.md` |
| 本交接文档 | `docs/handoff-agent-system.md` |
| 项目说明 | `CLAUDE.md`（根目录） |
| OpenClaw 参考源码 | `~/Codes/openclaw`（特别是 `src/agents/system-prompt.ts`） |
| 用户配置示例 | `~/.opencomputer/user.json`（运行后自动创建） |

---

## 7. 设计决策记录

| 决策 | 原因 |
|------|------|
| 配置用 JSON，描述用 Markdown | 结构化数据 JSON 更适合 GUI 读写；自然语言 Markdown 更适合人类编写 |
| 用户信息全局共享（user.json）而非 per-agent | 用户身份不随 Agent 变化，所有 Agent 共享同一份用户画像 |
| persona.md 而非 soul.md | "persona" 在 AI Agent 语境下最直觉，用户一看就懂 |
| 去掉技术栈、代码注释语言 | 这些不是通用用户信息，属于特定场景才需要的 |
| 新增 AI 经验水平字段 | 帮助 AI 判断用户对 AI 的熟悉程度，调整沟通方式 |
| 时区/语言用选择器而非输入框 | 面向普通用户，选择比输入更友好，默认取系统值 |
| 回复风格支持自定义文本 | 预设选项可能不够用，自定义给用户更多控制权 |
| 记忆系统单独设计 | 记忆功能复杂度高，需要独立的设计-实现周期 |
