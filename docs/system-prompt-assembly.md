# 系统提示词组装逻辑

> 文档版本: 2026-03-17
> 源码位置: `src-tauri/src/system_prompt.rs`

## 1. 概述

OpenComputer 的系统提示词（System Prompt）由 `system_prompt::build()` 函数模块化组装，最多包含 **10 个段落**，按顺序拼接，段落之间用双换行分隔。

提示词组装支持两种模式：

| 模式 | 触发条件 | 适用场景 |
|------|---------|---------|
| **结构化模式** | `useCustomPrompt = false`（默认） | 普通用户，通过 GUI 表单填写 |
| **自定义模式** | `useCustomPrompt = true` | 高级用户，完全控制 Markdown |

---

## 2. 数据来源

提示词内容来自三个层面的配置文件：

### 2.1 Agent 配置（`~/.opencomputer/agents/{id}/`）

| 文件 | 格式 | 用途 |
|------|------|------|
| `agent.json` | JSON | 结构化配置（名称、描述、性格、行为、过滤器等） |
| `agent.md` | Markdown | 身份说明 / 补充说明 |
| `persona.md` | Markdown | 人设说明 / 补充说明 |
| `tools.md` | Markdown | 工具使用指导 |

### 2.2 用户配置（`~/.opencomputer/user.json`）

全局共享，不随 Agent 变化。包含用户昵称、性别、年龄、角色、AI 经验、时区、语言、回复风格、自定义补充。

### 2.3 运行时信息

由代码动态获取：当前日期时间、工作目录、Shell 路径、操作系统、CPU 架构。

---

## 3. 组装流程

### 3.1 入口

```
agent.rs: build_system_prompt()
  → 加载 default Agent → system_prompt::build(definition)
  → 加载失败 → system_prompt::build_legacy()  （向后兼容）
```

### 3.2 结构化模式（默认）

当 `useCustomPrompt = false` 时，10 个段落按以下顺序组装：

```
┌─────────────────────────────────────────────────────────┐
│ ① 身份行 (Identity)                                     │
│    "You are {name}, a {role}, running in OpenComputer   │
│     on {os} {arch}."                                     │
│    ─ name: agent.json → name                            │
│    ─ role: agent.json → personality.role                 │
├─────────────────────────────────────────────────────────┤
│ ② 性格段 (Personality)                                   │
│    "# Personality"                                       │
│    - Vibe: {vibe}                                       │
│    - Tone: {tone}                                       │
│    - Communication style: {communicationStyle}          │
│    - Traits: {traits 逗号分隔}                           │
│    - Principles:                                         │
│      - {principle1}                                      │
│      - {principle2}                                      │
│    - Boundaries: {boundaries}                           │
│    - Quirks: {quirks}                                   │
│    ─ 来源: agent.json → personality 各字段               │
│    ─ 空字段自动跳过                                      │
├─────────────────────────────────────────────────────────┤
│ ③ agent.md — 身份补充说明                                │
│    ─ 用户在"身份"Tab 底部 textarea 填写的补充内容         │
│    ─ 截断上限: 20,000 字符                               │
├─────────────────────────────────────────────────────────┤
│ ④ persona.md — 性格补充说明                              │
│    ─ 用户在"性格"Tab 底部 textarea 填写的补充内容         │
│    ─ 截断上限: 20,000 字符                               │
├─────────────────────────────────────────────────────────┤
│ ⑤ 用户信息 (User Context)                               │
│    "# User"                                              │
│    - Name: {name}                                       │
│    - Gender: {gender}                                   │
│    - Age: {age}                                         │
│    - Role: {role}                                       │
│    - AI experience level: {aiExperience}                │
│    - Preferred language: {language}                      │
│    - Timezone: {timezone}                               │
│    - Response style: {responseStyle}                    │
│    - Additional info: {customInfo}                      │
│    ─ 来源: ~/.opencomputer/user.json                    │
│    ─ 空字段自动跳过，全空则整段省略                       │
├─────────────────────────────────────────────────────────┤
│ ⑥ tools.md — 工具使用指导                               │
│    ─ 用户在"行为"Tab 中填写的工具使用说明                 │
│    ─ 截断上限: 20,000 字符                               │
├─────────────────────────────────────────────────────────┤
│ ⑦ 工具定义 (Tool Definitions)                           │
│    内置 11 个工具的功能描述：                             │
│    exec / process / read / write / edit / ls /          │
│    grep / find / apply_patch / web_search / web_fetch   │
│    ─ 通过 FilterConfig (allow/deny) 过滤                │
│    ─ 有过滤时追加 "Only the following tools are enabled" │
│    ─ 前端可通过 list_builtin_tools 命令动态获取工具列表    │
├─────────────────────────────────────────────────────────┤
│ ⑧ 技能 (Skills)                                        │
│    "The following skills are available..."               │
│    - {skillName}: {description}                         │
│    ─ 来源: 全局技能 + 额外技能目录                       │
│    ─ 通过 FilterConfig (allow/deny) 过滤                │
│    ─ 上限: 150 条，总计 30,000 字符                      │
├─────────────────────────────────────────────────────────┤
│ ⑨ 运行时信息 (Runtime)                                  │
│    "# Runtime"                                           │
│    - Date: 2026-03-17 14:30 CST                         │
│    - Working directory: /Users/xxx/project              │
│    - Shell: /bin/zsh                                    │
├─────────────────────────────────────────────────────────┤
│ ⑩ 项目上下文 (Project Context)                          │
│    ─ 预留，尚未实现                                      │
├─────────────────────────────────────────────────────────┤
│ ⑧ᵇ 记忆 (Memory)                                       │
│    ─ 预留，尚未实现                                      │
└─────────────────────────────────────────────────────────┘
```

### 3.3 自定义提示词模式

当 `useCustomPrompt = true` 时，跳过结构化字段，直接使用 Markdown 文件：

```
┌─────────────────────────────────────────────────────────┐
│ ① 身份行（仅名称，无 role）                              │
│    "You are {name}, running in OpenComputer             │
│     on {os} {arch}."                                     │
├─────────────────────────────────────────────────────────┤
│ ② agent.md — 完整自定义身份说明                          │
│    ─ 用户在"自定义提示词"Tab 中直接编写                   │
├─────────────────────────────────────────────────────────┤
│ ③ persona.md — 完整自定义人设说明                        │
│    ─ 用户在"自定义提示词"Tab 中直接编写                   │
├─────────────────────────────────────────────────────────┤
│ ④ ~ ⑩ 与结构化模式相同                                  │
│    用户信息 → tools.md → 工具定义 → 技能 → 运行时        │
└─────────────────────────────────────────────────────────┘
```

**关键区别**：
- 结构化模式：身份行包含 `role`，自动生成 `# Personality` 段，agent.md / persona.md 作为补充
- 自定义模式：身份行仅含 `name`，不生成 Personality 段，agent.md / persona.md 作为主体内容

---

## 4. 过滤机制

### 4.1 FilterConfig

工具和技能均使用相同的 allow/deny 过滤逻辑：

```
FilterConfig {
    allow: Vec<String>,  // 白名单（非空时，仅允许列出的项）
    deny:  Vec<String>,  // 黑名单（排除列出的项）
}
```

**判断规则** (`is_allowed`):
1. 如果 `allow` 非空 且 名称不在 `allow` 中 → **拒绝**
2. 如果名称在 `deny` 中 → **拒绝**
3. 其他情况 → **允许**

### 4.2 工具过滤

- 无过滤配置时：输出完整的 11 个工具描述
- 有过滤配置时：输出完整描述 + 追加 `"Note: Only the following tools are enabled: exec, read, ..."`

### 4.3 技能过滤

- 全局禁用列表（`config.json` → `disabledSkills`）先过滤
- Agent 级别 FilterConfig 再过滤
- 最终结果传入 `build_skills_prompt()` 生成描述

---

## 5. 截断策略

对 Markdown 文件（agent.md / persona.md / tools.md）应用截断保护：

| 参数 | 值 |
|------|-----|
| 最大字符数 | 20,000 |
| 头部保留 | 70% |
| 尾部保留 | 20% |
| 中间标记 | `[... truncated {N} characters ...]` |

技能描述也有独立的上限：

| 参数 | 值 |
|------|-----|
| 最大条目数 | 150 |
| 最大总字符数 | 30,000 |

---

## 6. Legacy 兼容模式

当 default Agent 加载失败（如首次迁移、文件损坏）时，`build_legacy()` 提供降级兼容：

```
① 固定身份："You are OpenComputer, a personal AI assistant..."
② 用户信息
③ 完整工具描述（无过滤）
④ 全部技能（仅全局禁用过滤）
⑤ 运行时信息
```

无 Agent 配置、无 Markdown 文件、无 Agent 级过滤。

---

## 7. 组装示例

### 结构化模式示例

假设 Agent 配置：
- name: "小助手", role: "全栈开发助手"
- vibe: "耐心友善", tone: "专业但不死板"
- traits: ["细心", "善于解释"]
- principles: ["代码质量优先", "安全第一"]

用户配置：
- name: "张三", role: "前端开发者", language: "zh-CN"

组装结果：

```
You are 小助手, a 全栈开发助手, running in OpenComputer on macos aarch64.

# Personality

- Vibe: 耐心友善
- Tone: 专业但不死板
- Traits: 细心, 善于解释
- Principles:
  - 代码质量优先
  - 安全第一

（agent.md 补充内容...）

（persona.md 补充内容...）

# User

- Name: 张三
- Role: 前端开发者
- Preferred language: zh-CN

Available tools: ...（11 个工具描述）

The following skills are available...（技能列表）

# Runtime

- Date: 2026-03-17 14:30 CST
- Working directory: /Users/zhangsan/project
- Shell: /bin/zsh
```

### 自定义模式示例

```
You are 小助手, running in OpenComputer on macos aarch64.

（agent.md 用户自己写的完整身份说明...）

（persona.md 用户自己写的完整人设说明...）

# User

- Name: 张三
- Role: 前端开发者
- Preferred language: zh-CN

Available tools: ...

The following skills are available...

# Runtime

- Date: 2026-03-17 14:30 CST
- Working directory: /Users/zhangsan/project
- Shell: /bin/zsh
```

---

## 8. 数据结构参考

### AgentConfig（agent.json）

```json
{
  "name": "小助手",
  "description": "全栈开发助手",
  "emoji": "🤖",
  "avatar": "/path/to/avatar.png",
  "useCustomPrompt": false,
  "personality": {
    "role": "全栈开发助手",
    "vibe": "耐心友善",
    "tone": "专业但不死板",
    "traits": ["细心", "善于解释"],
    "principles": ["代码质量优先", "安全第一"],
    "boundaries": "不执行危险的系统命令",
    "quirks": "喜欢用类比来解释概念",
    "communicationStyle": "先给结论再展开"
  },
  "behavior": {
    "maxToolRounds": 10,    // 0 = 不限制轮数
    "requireApproval": ["exec"],
    "sandbox": false
  },
  "tools": { "allow": [], "deny": [] },
  "skills": { "allow": [], "deny": [] },
  "model": { "primary": null, "fallbacks": [] }
}
```

### UserConfig（user.json）

```json
{
  "name": "张三",
  "gender": "male",
  "age": 28,
  "role": "前端开发者",
  "aiExperience": "expert",
  "language": "zh-CN",
  "timezone": "Asia/Shanghai",
  "responseStyle": "concise",
  "customInfo": "偏好 TypeScript 和 React"
}
```

---

## 9. 相关源码

| 文件 | 职责 |
|------|------|
| `system_prompt.rs` | 提示词组装主逻辑（build / build_legacy / 各段构建函数） |
| `agent_config.rs` | 数据结构定义（AgentConfig / PersonalityConfig / FilterConfig / BehaviorConfig） |
| `agent_loader.rs` | Agent 文件加载 / 多语言模板 |
| `user_config.rs` | 用户配置加载 + build_user_context() |
| `skills.rs` | 技能加载 + build_skills_prompt() |
| `agent.rs` | build_system_prompt() 入口委托，动态读取 maxToolRounds 配置 |
| `lib.rs` | `list_builtin_tools` 命令 — 前端获取内置工具列表（名称 + 描述） |
