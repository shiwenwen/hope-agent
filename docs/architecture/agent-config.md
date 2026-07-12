# Agent 配置与解析链

> 返回 [文档索引](../README.md) | 关联源码：[`crates/ha-core/src/agent_config.rs`](../../crates/ha-core/src/agent_config.rs)、[`crates/ha-core/src/agent_loader.rs`](../../crates/ha-core/src/agent_loader.rs)、[`crates/ha-core/src/agent/resolver.rs`](../../crates/ha-core/src/agent/resolver.rs)、[`crates/ha-core/src/agent/migration.rs`](../../crates/ha-core/src/agent/migration.rs)、命令面在 [`src-tauri/src/commands/`](../../src-tauri/src/commands/) 与 [`crates/ha-server/src/routes/agents.rs`](../../crates/ha-server/src/routes/agents.rs)

## 概述

Agent 配置子系统以**磁盘目录 `agents/{id}/` 为单一真相源**：每个 Agent 由一份 `agent.json`（结构化身份 / 能力 / 记忆 / 委派配置）加若干 markdown 文件（行为说明、人格、工具指引、记忆）共同定义。`agent_loader` 负责从磁盘装配运行时 `AgentDefinition`，`resolver` 用固定的 **7 级优先链**裁决「一次新会话该用哪个 Agent」，`migration` 提供一次性的 legacy `"default"` → `"ha-main"` 启动迁移。

该子系统**只管「Agent 是谁、用哪个 Agent」**，不直接负责 prompt 拼装（见 [prompt-system.md](prompt-system.md)）、记忆召回（见 [memory.md](memory.md)）、子 Agent 委派执行（见 [subagent.md](subagent.md)）；这些子系统从这里读取配置但实现各自闭合。Agent 解析链的逐级表此前最完整的覆盖在 [project.md](project.md) 的「Agent 解析链」整节，本文是该子系统的单一真相源、与之互链而不重复工作目录解析等项目侧细节。

提供三类能力：

- **Agent 装配**：`load_agent` 从 `agents/{id}/` 读 `agent.json` + markdown，合成 `AgentDefinition`；`list_agents` 扫目录产 `AgentSummary` 列表
- **默认 Agent 解析**：`resolve_default_agent_id_full` 7 级链按上下文（项目 / IM 各级 / 全局）裁决首个非空胜出，并返回命中级别 `AgentSource`
- **Owner CRUD**：Tauri / HTTP owner 平面读写 `agent.json` / markdown、排序、删除、人格渲染、OpenClaw 导入

## 模块结构

| 模块 | 职责 |
|---|---|
| [`agent_config.rs`](../../crates/ha-core/src/agent_config.rs) | 全部数据模型：`AgentConfig` 及其嵌套（model / personality / capabilities / memory / subagents / team / acp）+ 运行时 `AgentDefinition` / `AgentSummary` + `effective_memory_budget` |
| [`agent_loader.rs`](../../crates/ha-core/src/agent_loader.rs) | 磁盘装配与读写：`load_agent` / `list_agents` / `list_all_agents` / `ensure_default_agent` / `save_agent_*` / `get_template` / `render_persona_to_soul_md`；硬常量 `DEFAULT_AGENT_ID` |
| [`agent_lifecycle.rs`](../../crates/ha-core/src/agent_lifecycle.rs) | Agent 启停、删除预检、活动工作阻断、引用重绑、备份与可恢复回收站删除的唯一入口 |
| [`agent/resolver.rs`](../../crates/ha-core/src/agent/resolver.rs) | 7 级默认 Agent 解析链 + `AgentSource` 来源枚举 + `normalize_default_agent_id` 写归一 |
| [`agent/migration.rs`](../../crates/ha-core/src/agent/migration.rs) | legacy `"default"` → `"ha-main"` 一次性启动迁移（sentinel 短路） |

## 配置模型（`AgentConfig`）

`agent.json` 反序列化为 `AgentConfig`（**camelCase**，**所有字段 `serde default`**——缺字段 / 缺文件均回落 `AgentConfig::default()`）：

```rust
pub struct AgentConfig {
    pub enabled: bool,                         // 默认 true；false 时不可参与新执行
    pub name: String,                          // 缺省 "Assistant"
    pub description: Option<String>,
    pub emoji: Option<String>,
    pub avatar: Option<String>,
    pub model: AgentModelConfig,
    pub personality: PersonalityConfig,
    pub capabilities: CapabilitiesConfig,
    pub memory: MemoryConfig,
    pub openclaw_mode: bool,                   // 4 文件 markdown 模式
    pub notify_on_complete: Option<bool>,      // None = 跟随全局
    pub subagents: SubagentConfig,
    pub team: TeamAgentConfig,
    pub acp: AgentAcpConfig,                   // ACP 外部 agent 委派
}
```

**`AgentConfig` 不含 `id` 字段**——Agent id 即磁盘目录名 `agents/{id}/`，运行时 id 落在 `AgentDefinition.id`（装配时由目录名注入）。

### 模型覆盖（`AgentModelConfig`）

per-agent 覆盖主对话模型选择，是「会话 > Agent > 全局」三层覆盖的中间层：`primary` / `fallbacks`（failover 链）/ `plan_model`（Plan 期专用模型）/ `temperature` / `reasoning_effort`。空 = 继承全局 active model 与全局温度 / think 配置（见 [provider-system.md](provider-system.md) / [failover.md](failover.md)）。

### 人格（`PersonalityConfig` / `PersonaMode`）

人格有**双面创作模型**，由 `PersonaMode` 切换：

- **`Structured`（默认）**：结构化字段 `role` / `vibe` / `tone` / `traits` / `principles` / `boundaries` / `quirks` / `communication_style`，由前端表单填写、渲染进 system prompt
- **`SoulMd`**：放弃结构化字段，改由 `soul.md` 自由文本承载人格——给想完全手写人设的用户

`render_persona_to_soul_md` 把结构化 `PersonalityConfig` 渲染成一份 `SOUL.md` 草稿（**仅返回文本、不落盘**），供用户从 Structured 模式迁移到 SoulMd 模式时打底。

### 能力（`CapabilitiesConfig`）

工具 / 技能 / 审批 / 沙箱 / 运行时限的总开关：

| 字段 | 语义 |
|---|---|
| `max_tool_rounds` | 单 turn 工具循环轮数上限 |
| `sandbox` / `default_sandbox_mode` | legacy bool 与新 `SandboxMode` 枚举；`None` 时按 legacy bool 经 `effective_default_sandbox_mode` 映射 |
| `tools: FilterConfig` | **非 Core 工具**的显式开 / 关覆盖（Core 工具不受影响） |
| `skills: FilterConfig` | 技能严格白 / 黑名单 |
| `async_tool_policy: AsyncToolPolicy` | `ModelDecide`（默认）/ `AlwaysBackground` / `NeverBackground` 异步后台策略 |
| `mcp_enabled` | 是否注入该 Agent 的 MCP 工具 |
| `skill_env_check` | 技能环境变量检查 |
| `enable_custom_tool_approval` / `custom_approval_tools` | 自定义审批工具白名单（Smart 模式刻意忽略，见 [permission-system.md](permission-system.md)） |
| `default_session_permission_mode` | 该 Agent 新会话的初始权限 mode（`default \| smart \| yolo`） |

**`FilterConfig`** 是通用 `allow` / `deny` 对，`is_allowed` 走严格白 / 黑名单语义。两处用途不同：`skills` 用严格语义（不在 allow 即拒）；`tools` **仅作非 Core 工具的显式开 / 关覆盖**——Core 工具恒可用，执行层统一走 `dispatch::resolve_tool_fate`（见 [tool-system.md](tool-system.md)）。

### 记忆（`MemoryConfig` / `ActiveMemoryConfig`）

per-agent 记忆开关与提取阈值。**多数提取相关字段 `Option`，`None` = 继承全局，不是关闭**；`budget` 覆盖是**整体替换**不是 field-by-field 合并。`effective_memory_budget(agent, global)` 是唯一入口：`agent.budget` 存在则整体覆盖全局 `MemoryBudgetConfig`。

`ActiveMemoryConfig` 控制 Active Memory 预回复注入（**默认 `enabled=false`**），含 `include_claims`（Active Memory v2，把召回候选扩到 effective-active claim）。**该开关 per-agent 走 `agent.json`、不进 `ha-settings`**。记忆侧契约详见 [memory.md](memory.md) / [dreaming.md](dreaming.md)。

### 委派（`SubagentConfig` / `TeamAgentConfig`）

`SubagentConfig` 定义子 Agent 委派行为：`allowed_agents` / `denied_agents`（`is_agent_allowed` 判定）/ `max_concurrent` / 默认超时 / 深度 / 批量等。`TeamAgentConfig` 承载 Agent Team 能力配置。委派执行细节见 [subagent.md](subagent.md) / [agent-team.md](agent-team.md)。

## 磁盘布局与运行时装配

### 目录布局

每个 Agent 一个目录 `~/.hope-agent/agents/{id}/`（[`paths::agent_dir`](../../crates/ha-core/src/paths.rs)）：

| 文件 | 内容 | 必读 |
|---|---|---|
| `agent.json` | `AgentConfig`，**单一真相源** | 缺失则用 `default` |
| `agent.md` | Agent 行为说明（首启写默认模板） | 可选 |
| `persona.md` | 人格 / 沟通风格 | 可选 |
| `tools.md` | 自定义工具使用指引 | 可选 |
| `memory.md` | Agent 级核心记忆 | 可选 |
| `agents.md` / `identity.md` / `soul.md` | OpenClaw 4 文件模式（`openclaw_mode=true`）或 SoulMd 人格面（仅 `soul.md`）时读取 | 可选 |

外加几个非 per-agent 目录目录：`~/.hope-agent/memory.md`（全局共享核心记忆，[`paths::root_dir`](../../crates/ha-core/src/paths.rs)）、`~/.hope-agent/{id}-home/`（命名 Agent home 目录，[`paths::agent_home_dir`](../../crates/ha-core/src/paths.rs)，`load_agent` 时 ensure 存在）、`~/.hope-agent/plans/{agent}/`（Agent 维度 plan 目录，见 [plan-mode.md](plan-mode.md)）、`~/.hope-agent/avatars/default-agent-logo.png`（内置品牌 logo，默认 Agent 头像）。

### `AgentDefinition` 与 `load_agent` 流程

`AgentDefinition` 是运行时完整定义——`id` + `dir` + `config: AgentConfig` + 各 markdown 内容字段（`agent_md` / `persona` / `tools_guide` / `agents_md` / `identity_md` / `soul_md` / `global_memory_md` / `memory_md`）。`load_agent` 流程：

1. 读 `agents/{id}/agent.json`：**文件缺失才回落 `AgentConfig::default()`**；文件**存在但 JSON 解析失败则 `load_agent` 直接 `bail`（致命错误，不静默 default）**——与 `list_agents` 刻意相反，后者对解析失败的目录宽容跳过（`.ok()` + `unwrap_or(default)`）。`serde default` 只兜底「文件存在、JSON 合法、但缺字段」，不兜底「JSON 损坏」
2. 读各 markdown（按 `openclaw_mode` / `PersonaMode` 决定读 `agents.md`/`identity.md`/`soul.md` 与否）
3. 注入全局 `~/.hope-agent/memory.md` 到 `global_memory_md`
4. ensure `{id}-home/` 目录存在
5. 合成 `AgentDefinition` 返回

markdown 各文件如何进 system prompt（行为说明 / 人格 / soul / 记忆段）见 [prompt-system.md](prompt-system.md)。

### `AgentSummary` 与列表

`AgentSummary` 是前端列表用轻量摘要（含 `enabled`、`has_*` 标志与 `memory_count`）。`list_agents` 只返回可运行 Agent，供聊天、Cron、频道和委派选择；`list_all_agents` 是 owner 设置面的完整列表，包含已禁用 Agent，便于重新启用或安全删除。两者都按 `config.agent_order` 排序、主 Agent 置顶，并对解析失败目录 skip。`list_agent_ids` 只返回目录名集合，供 ID 冲突检测。

## 生命周期与生产级删除

- `AgentConfig.enabled=false` 是可逆运行门：配置与数据仍可编辑，但共享 Chat Engine、ACP、Subagent/Team 等执行入口均通过 `AgentRunGuard` fail closed；默认解析器仍保持“首个非空胜出”的纯语义，绝不因禁用/读盘错误静默换 Agent。固定主 Agent `ha-main` 不允许禁用，且全量配置保存同样强制该不变量。
- 所有 Agent 文件路径先经过 `paths::validate_agent_id`，仅接受 1–64 位 ASCII 字母、数字、`-`、`_`；owner HTTP/Tauri 不得依赖前端校验。
- 删除必须先调用 `preview_agent_delete`。预检汇总全局/频道配置、Project、Cron、其他 Agent 委派列表、历史 Session/Subagent/Team、Agent 记忆，以及前台 turn、非终态 Subagent、active Team、running Cron、active background job。
- 删除执行在 lifecycle mutex 下重新预检；执行入口及 Team 成员持久化前通过 `AgentRunGuard` 在同一把锁下原子完成“可运行检查 + 活动登记”，存在任何活动工作即拒绝。随后先创建并逐字节验证目标 `agent.json` 与现有 `config.json` 的配置备份（失败即中止）、禁用目标，再把全局/频道/Project/Cron/委派引用重绑到用户选择的可运行替代 Agent。历史 Session、Subagent/Team trace 与结构化记忆不改写，保留审计语义。
- 最终不做 `remove_dir_all`：`agents/{id}` 原子移动到 `trash/agents/<id>-<timestamp>-<uuid>/agent`；Agent Home 与 Plan 尽力移入同一回收站，并写 `manifest.json`。配置、其他 Agent、Project 与 Cron 在变更前保存精确前镜像，任一步失败自动补偿回滚；成功移动后写入进程内删除墓碑，普通配置/Markdown 保存无法用陈旧请求复活目录，只有显式 create 可以重新使用该 id。

## 默认 Agent 模板与首启

`ensure_default_agent` 首启创建 `agents/ha-main/`（`agent.json` + `agent.md`），**`agent.json` 已存在即短路**。模板按系统 locale 选语言：`detect_system_locale` 探测系统 locale（macOS `defaults` / `LANG` 等），`get_template(name, locale)` 返回内置模板（`agent` / `persona` 模板含 i18n，OpenClaw 四件套模板无 i18n）。

`DEFAULT_AGENT_ID` 是硬编码主 Agent id 常量 `"ha-main"`，定义在 `agent_loader`。`is_main_agent` 判定某 id 是否主 Agent，用于工具 tier 的 `default_for_main` / `default_for_others` 富集程度（主 Agent 默认装更全的工具集）。

## 7 级默认 Agent 解析链

`resolve_default_agent_id_full` 是**单一真相源**，返回 `(id, AgentSource)`，**首个非空胜出**：

| 级别 | 来源 | `AgentSource` |
|---|---|---|
| 1 | 显式参数 | `Explicit` |
| 2 | `project.default_agent_id` | `Project` |
| 3 | `topic.agent_id` | `Topic` |
| 4 | `group.agent_id` | `Group` |
| 5 | `tg_channel.agent_id` | `ChannelOverride` |
| 6 | `channel_account.agent_id` | `ChannelAccount` |
| 7 | `AppConfig.default_agent_id` | `GlobalConfig` |
| 兜底 | 硬编码 `DEFAULT_AGENT_ID`（`"ha-main"`） | `Hardcoded` |

便捷包装：

- `resolve_default_agent_id` —— desktop / HTTP 用（只传 project + channel_account，IM 各级传 `None`）
- `resolve_default_agent_id_with_source` —— 携来源 tag，供 `/status` 调试

`AgentSource::label()` 给每级一个可读标签，让 `/status` 能告诉用户「当前会话的 Agent 是从哪一级解析出来的」。

`normalize_default_agent_id` 是写路径的统一归一入口（trim；空串 = `None` = 清除全局默认），Tauri / HTTP / `update_settings` 三处写 `AppConfig.default_agent_id` 都经它。

## 持久化

| 位置 | 内容 |
|---|---|
| `~/.hope-agent/agents/{id}/agent.json` | 每个 Agent 的 `AgentConfig`，单一真相源 |
| `~/.hope-agent/agents/{id}/{agent,persona,tools,memory,agents,identity,soul}.md` | 行为 / 人格 / 工具 / 记忆 markdown |
| `~/.hope-agent/memory.md` | 全局共享核心记忆（注入 `global_memory_md`） |
| `~/.hope-agent/{id}-home/` | 命名 Agent home 目录 |
| `~/.hope-agent/avatars/default-agent-logo.png` | 默认 Agent 头像 |
| `~/.hope-agent/plans/{agent}/` | Agent 维度 plan 目录 |
| `config.json: AppConfig.default_agent_id` | 全局默认 Agent（解析链第 7 级） |
| `config.json: AppConfig.agent_order` | `list_agents` 显示排序（`reorder_agents` 写） |
| `~/.hope-agent/.agent-id-renamed` | legacy `default`→`ha-main` 迁移 sentinel（存在即短路） |

写路径：`save_agent_config` 以 pretty JSON 写 `agent.json`；`save_agent_markdown` / `get_agent_markdown` 读写**受白名单约束**的 markdown（防路径穿越）；`reorder_agents` 经 `mutate_config` 持久化 `config.agent_order`；`update_agent_reasoning_effort` 校验后写 `model.reasoning_effort`。配置读写 contract 见 [config-system.md](config-system.md)。

## 对外接口面

### Tauri 命令

| 命令 | 职责 |
|---|---|
| `list_agents` | 列出 `AgentSummary` |
| `list_all_agents` | owner 设置面列出含 disabled 的全部 Agent |
| `reorder_agents` | 持久化排序 |
| `get_agent_config` / `save_agent_config_cmd` | 读 / 写 `agent.json`；显式新建传 `create=true`，普通保存受生命周期墓碑保护 |
| `get_agent_markdown` / `save_agent_markdown` | 读 / 写白名单 markdown |
| `save_agent_memory_md` | 写 Agent 级 `memory.md` |
| `preview_agent_delete` | 删除依赖、保留数据与活动工作预检 |
| `set_agent_enabled` | 启用/禁用非主 Agent |
| `delete_agent` | 传替代 Agent，执行引用重绑与可恢复删除 |
| `render_persona_to_soul_md` | 渲染 SOUL.md 草稿（不落盘） |
| `get_agent_template` | 取内置模板 |
| `scan_openclaw_agents` / `import_openclaw_agents` / `scan_openclaw_full` / `import_openclaw_full` | OpenClaw 导入扫描 + 落地 |
| `get_default_agent_id` / `set_default_agent_id` | 读 / 写 `AppConfig.default_agent_id`（解析链第 7 级） |

### HTTP 路由

| 路由 | 映射命令 |
|---|---|
| `GET /api/agents` | `list_agents` |
| `GET /api/agents/all` | `list_all_agents` |
| `POST /api/agents/reorder` | `reorder_agents` |
| `GET /api/agents/template` | `get_agent_template` |
| `GET /api/agents/{id}` | `get_agent` / `get_agent_config` |
| `PUT /api/agents/{id}` | `save_agent` / `save_agent_config_cmd` |
| `GET /api/agents/{id}/delete-preview` | `preview_agent_delete` |
| `PATCH /api/agents/{id}/enabled` | `set_agent_enabled` |
| `DELETE /api/agents/{id}?replacementAgentId=...` | `delete_agent` |
| `GET /api/agents/{id}/markdown` / `PUT /api/agents/{id}/markdown` | 读 / 写 markdown |
| `GET /api/agents/{id}/memory-md` / `PUT /api/agents/{id}/memory-md` | 读 / 写 `memory.md` |
| `POST /api/agents/{id}/persona/render-soul-md` | 渲染 SOUL.md |
| `GET /api/agents/openclaw/scan`、`POST /api/agents/openclaw/import`、`GET /api/agents/openclaw/scan-full`、`POST /api/agents/openclaw/import-full` | OpenClaw 导入 |
| `GET /api/config/default-agent` / `PUT /api/config/default-agent` | 读 / 写全局默认 Agent |

`POST /api/agents/initialize`（`initialize_agent`）是 onboarding / provider 设置入口，**非 Agent CRUD**——HTTP 与 Tauri 的 `auth::initialize_agent` 语义有差异，见 [api-reference.md](api-reference.md) §7.4。全部端点的 Tauri ↔ HTTP 对齐单一真相源也在 [api-reference.md](api-reference.md)。

### 事件

- **`agents:changed`** —— Tauri 与 HTTP owner 命令在 saved / deleted / reordered / imported 后 emit，按 `kind` 不同携不同字段（saved / deleted 带 `id`+`kind`，reordered 仅 `kind`，imported 带 `kind`+`count`），供前端刷新 Agent 列表
- **`config:changed`** —— `set_default_agent_id` / `reorder_agents` 经 `mutate_config` 自动 emit；OpenClaw 导入若顺带加 providers 时也手动 emit

## legacy `"default"` → `"ha-main"` 一次性迁移

早期版本主 Agent id 是字面量 `"default"`，现行版本统一为 `"ha-main"`。`migrate_default_agent_id_to_ha_main` 在启动期一次性把所有 `"default"` 引用 rename 到 `"ha-main"`：

- **磁盘目录**：`agents/default/` / `default-home/` / `plans/default/`
- **`agent.json` 内嵌**：`subagents.allowedAgents` / `deniedAgents`
- **SQLite agent_id 列**：`sessions.db` 的 sessions / team_members / `teams.lead_agent_id` / `subagent_runs.{parent,child}_agent_id` / `projects.default_agent_id`；外加各自独立 DB 文件 `logs.db`（logs）/ `background_jobs.db`（background_jobs，仅文件已存在时 best-effort）/ `canvas.db`（canvas_projects，同 best-effort）
- **`memory.db`**：`memories.scope_agent_id` + `memory_claims.scope_id` + `memory_profile_snapshots.scope_id`（均限 `scope_type='agent'` 的行；后两表通常 no-op）
- **`cron_jobs.payload_json`** 内嵌的 agent_id
- **`config.json`**：`default_agent_id` / `recap.analysisAgent` / channel 各级 agent_id

完成后落 sentinel `~/.hope-agent/.agent-id-renamed`，后续启动短路。**每步独立 idempotent、崩溃可恢复**：sentinel 仅在磁盘目录 rename **干净完成后**才写；当 `agents/default/` 与 `agents/ha-main/` **同时存在**（用户手动建过 ha-main）时迁移**整体放弃**——不写 sentinel、不动 DB / config，下次启动重试。

**入口契约**：`init_runtime`（初始化 SESSION_DB / CRON_DB / LOG_DB 与 config）**必须早于 `ensure_default_agent()`**——后者会预创空 `agents/ha-main/` 模板，吞掉 rename（让上面的「同时存在」判定误触发放弃）。desktop（[`src-tauri/src/lib.rs`](../../src-tauri/src/lib.rs)）与 server（[`src-tauri/src/main.rs`](../../src-tauri/src/main.rs)）都须遵守此序。

## 安全 / 红线

- **7 级解析链顺序固定**：显式参数 → `project.default_agent_id` → `topic.agent_id` → `group.agent_id` → `tg_channel.agent_id` → `channel_account.agent_id` → `AppConfig.default_agent_id` → 硬编码 `"ha-main"`，首个非空胜出。**channel worker 不得自写解析链**，统一收敛到 `resolve_default_agent_id_full` 单一真相源
- **字面量 agent id 一律走 `DEFAULT_AGENT_ID`**（前端走 `@/types/tools` 的 `DEFAULT_AGENT_ID` / `isMainAgent`），**禁止重新引入 `"default"` 硬编码**
- **迁移入口契约**：`init_runtime` 必须早于 `ensure_default_agent()`；迁移幂等、崩溃可恢复，sentinel 仅在磁盘 rename 干净完成后写，`default` 与 `ha-main` 同存时整体放弃
- **`is_main_agent` 与 `AppConfig.default_agent_id` 正交**：即便用户改了全局默认 Agent，字面量 `"ha-main"` 仍是「主 Agent」（决定工具 tier 富集程度）；`set_default_agent_id` 写的是解析链第 7 级、**不影响 `is_main_agent`**
- **生命周期红线**：主 Agent 不可禁用/删除（包括全量 `agent.json` 保存）；删除必须有不同且可运行的 replacement；活动工作非零 fail closed；ACP 与 Team 成员创建不得绕过 `AgentRunGuard`；普通写入不得清除删除墓碑；禁止重新引入 owner 可达的裸 `remove_dir_all(agent_dir)`
- **Agent ID 路径红线**：所有 `agent_dir` / `agent_home_dir` 写删路径必须经 `validate_agent_id`，不得只靠 GUI slug 校验
- **markdown 白名单**：`save` / `get_agent_markdown` 仅允许 `agent.md` / `persona.md` / `tools.md` / `agents.md` / `identity.md` / `soul.md`，防路径穿越
- **`AgentConfig` 全字段 `serde default` ≠ 解析失败兜底**：合法 JSON 缺字段 → 字段级 `default`（`load` 不因缺字段失败）；**文件缺失** → `load_agent` 回落整体 `AgentConfig::default()`；**文件存在但 JSON 解析失败** → `load_agent` `bail`（致命、不 default），而 `list_agents` 对解析失败的目录 skip（两者刻意不同档，别照搬「load 永不失败」）
- **`tools.allow/deny` 仅是非 Core 工具显式覆盖**，Core 工具不受影响（执行层走 `dispatch::resolve_tool_fate`）；`skills` 用严格白 / 黑名单语义
- **mode 字段只影响新会话**：`default_session_permission_mode` / `default_sandbox_mode` 仅决定新会话初始 mode，**已有会话不受改动影响**；`default_sandbox_mode=None` 时按 legacy sandbox bool 经 `effective_default_sandbox_mode` 映射
- **记忆继承语义**：`MemoryConfig` 提取相关字段 `None` = 继承全局**不是关闭**；`agent.budget` 覆盖是整体替换不是 field-by-field 合并
- **Active Memory v2 不进 `ha-settings`**：`ActiveMemoryConfig.include_claims` 与 v2 per-agent 走 `agent.json`
- **`normalize_default_agent_id` 是写归一统一入口**（Tauri / HTTP / `update_settings` 三处），空串 = 清除全局默认、resolver 回退硬编码

## 与相邻子系统的关系

| 子系统 | 关系 |
|---|---|
| [Project](project.md) | `project.default_agent_id` 是解析链第 2 级；当前最完整的 7 级链覆盖在 project.md，本文为单一真相源、与之互链 |
| [Config](config-system.md) | `AppConfig.default_agent_id`（第 7 级）+ `agent_order`；写经 `mutate_config` + `config:changed` |
| [Prompt System](prompt-system.md) | `AgentDefinition` 的 markdown（agent.md / persona / soul 等）如何拼进 system prompt |
| [Memory](memory.md) / [Dreaming](dreaming.md) | `MemoryConfig` / `ActiveMemoryConfig` / `effective_memory_budget` 的记忆侧契约 |
| [Subagent](subagent.md) | `SubagentConfig`（allowed / denied / max_concurrent 等）委派侧契约 |
| [Agent Team](agent-team.md) | `TeamAgentConfig` |
| [Provider System](provider-system.md) / [Failover](failover.md) | `AgentModelConfig` 的 primary / fallbacks / plan_model / temperature / reasoning_effort 覆盖 |
| [Permission System](permission-system.md) | `default_session_permission_mode` / `custom_approval_tools` / sandbox 默认 |
| [IM Channel](im-channel.md) | 解析链第 3–6 级（topic / group / tg_channel / channel_account 的 agent_id） |
| [API Reference](api-reference.md) | `/api/agents` 全端点 Tauri ↔ HTTP 对齐表 + §7.4 initialize 语义差异 |

## 关键文件索引

| 文件 | 角色 |
|---|---|
| [`crates/ha-core/src/agent_config.rs`](../../crates/ha-core/src/agent_config.rs) | 全部数据模型 + `AgentDefinition` / `AgentSummary` + `effective_memory_budget` |
| [`crates/ha-core/src/agent_loader.rs`](../../crates/ha-core/src/agent_loader.rs) | 磁盘装配 + CRUD + 模板 + `DEFAULT_AGENT_ID` / `is_main_agent` |
| [`crates/ha-core/src/agent/resolver.rs`](../../crates/ha-core/src/agent/resolver.rs) | 7 级解析链 + `AgentSource` + `normalize_default_agent_id` |
| [`crates/ha-core/src/agent/migration.rs`](../../crates/ha-core/src/agent/migration.rs) | legacy `default`→`ha-main` 一次性迁移 |
| [`crates/ha-core/src/paths.rs`](../../crates/ha-core/src/paths.rs) | `agent_dir` / `agent_home_dir` / `avatars_dir` / `root_dir` 入口 |
| [`crates/ha-server/src/routes/agents.rs`](../../crates/ha-server/src/routes/agents.rs) | `/api/agents/*` HTTP 路由 |
