# OpenClaw 导入

> 返回 [文档索引](../README.md) | 关联源码：[`crates/ha-core/src/openclaw_import/`](../../crates/ha-core/src/openclaw_import/)、Tauri 命令在 [`src-tauri/src/lib.rs`](../../src-tauri/src/lib.rs)、HTTP 路由在 [`crates/ha-server/src/routes/`](../../crates/ha-server/src/routes/)、前端在 [`src/components/settings/`](../../src/components/settings/) 的 `OpenClawImportDialog`、CLI 接入在 onboarding 步骤（见 [`cli.md`](cli.md)）

## 概述

OpenClaw 导入是一个**一次性迁移子系统**，把 OpenClaw（前身 clawdbot）桌面应用的 **providers / agents / memory** 搬进 Hope Agent。它**不是持续同步**——扫一次、导一次，导完两边各走各的，不监听源目录变化、不做增量回写。

数据流分三段：

- **全量扫描预览**（`scan_openclaw_full`）：一次扫出 providers + agents + memory 库存，返回 `OpenClawImportPreview`，**无副作用**
- **用户勾选**（`OpenClawImportRequest`）：GUI 多选 / CLI 单 yes/no 决定导哪几类、哪几个 agent、是否带记忆
- **应用导入**（`import_openclaw_full`）：按勾选载荷落库，顺序硬约束 **providers → agents → memory**

两套入口能力不对等：**GUI** 是多选粒度（逐 provider / 逐 agent / 逐类记忆勾选），**CLI** onboarding 只给单个 yes/no（全导或全不导，详见 [`cli.md`](cli.md)）。源码全在 `crates/ha-core/src/openclaw_import/`（零 Tauri 依赖），Tauri / HTTP 只做薄壳。

v1 的迁移范围有意收窄：记忆只导 **markdown 条目 + SQLite chunk 文本**，向量库的 embedding 一律丢弃（见安全红线）。

## 模块结构

| 模块 | 职责 |
|---|---|
| `mod.rs` | 子系统根、四个入口（`scan_openclaw_full` / `import_openclaw_full` + legacy shim `scan_openclaw_agents` / `import_openclaw_agents`）、状态目录解析 `resolve_openclaw_state_dir`、记忆合并段 `merge_openclaw_memory_section` 与写前备份 `backup_existing_core_memory_md`、顶层数据结构（`OpenClawImportPreview` / `OpenClawImportRequest` / `OpenClawImportSummary` / `MemoryPreview`） |
| `providers.rs` | Provider 映射核心：`build_providers` / `collect_credentials` / `map_api_type`；预览与待写入结构（`ProviderPreview` / `ResolvedProvider` / `ProviderProfilePreview` / `CredentialKind`）；OpenClaw 配置反序列化（`OpenClawConfigRoot` / `AuthProfilesFile` / `AuthCredentialEntry` / `SecretRef`） |
| `agents.rs` | Agent 映射：`build_previews` / `import_single_agent` / `build_model_lookup` / `extend_model_lookup_from_provider_configs`；OpenClaw agent 反序列化（`OpenClawAgent` / `OpenClawAgentModel`）与预览 / 请求 / 结果（`OpenClawAgentPreview` / `ImportAgentRequest` / `ImportResult` / `ProviderForModel`） |
| `memory.rs` | 记忆解析：`parse_openclaw_memory_md`（MEMORY.md → `NewMemory`）/ `parse_openclaw_sqlite_memory_db`（向量库 chunk 文本，只读） |

## 状态目录解析与配置发现

`resolve_openclaw_state_dir` 是「OpenClaw 装在哪」的唯一裁决，按优先级回退：

1. 环境变量 `OPENCLAW_STATE_DIR`（测试 / 高级用户覆盖）
2. `~/.openclaw/`（新名）
3. `~/.clawdbot/`（旧名回退）

`scan_openclaw_full` 在状态目录里发现配置文件：主配置 `~/.openclaw/openclaw.json`，旧版回退 `~/.clawdbot/clawdbot.json`，反序列化为 `OpenClawConfigRoot`（`models` + `auth` 两块）。**OpenClaw 未检测到时**（目录不存在），`scan_openclaw_full` 返回 `state_dir_present=false` 的空预览（无副作用），而 legacy shim `scan_openclaw_agents` 直接 `bail`。`resolve_openclaw_state_dir` 单独 re-export 供测试 / 诊断调用。

## 数据结构

### 预览侧（scan 返回）

- **`OpenClawImportPreview`** — `scan_openclaw_full` 的完整返回：`state_dir` 路径、`state_dir_present` 标记、`providers` / `agents` / `memories` 三类库存、`warnings`
- **`ProviderPreview`** — 单 provider 预览：`source_key` / `suggested_name` / `api_type` / `base_url` / `model_count` / `profiles` / `name_conflicts_existing`（与现有 config 重名）/ `api_type_warning`
- **`ProviderProfilePreview`** — 单凭据 profile 预览：`credential_kind` / `will_import`（OAuth 恒 false）/ `note` / `email`
- **`CredentialKind`** — 凭据分类枚举：`ApiKeyPlain` / `ApiKeyEnvRef` / `OAuth` / `Token` / `Missing`
- **`OpenClawAgentPreview`** — 单 agent 预览：含 `emoji` / `theme` / `avatar` / `model_info` / `available_files`（workspace markdown 清单）/ `already_exists`（经 `agent_loader::list_agent_ids` 标记）
- **`MemoryPreview`** — 记忆清单：`global_md_present` + 每 agent 可导入条目的估算数

### 请求侧（用户勾选）

- **`OpenClawImportRequest`** — 顶层勾选载荷：`import_provider_keys` / `import_agents`（逐 agent 的 `ImportAgentRequest` 列表）/ `import_global_memory` / `import_agent_memories`
- **`ImportAgentRequest`** — 单 agent 导入参数：`source_id` / `target_id` / `name` / `emoji` / `vibe` / `sandbox` / `import_files`

### 结果侧（import 返回）

- **`OpenClawImportSummary`** — 导入汇总：`providers_added`（新 provider 的 UUID 列表）/ `agents`（逐 `ImportResult`）/ `memories_added` 计数 / `warnings`
- **`ImportResult`** — 单 agent 结果：`success` / `error`

### 内部「待写入」结构

- **`ResolvedProvider`** — `build_providers` 算出的待落库 provider：`source_key` + 完整 `ProviderConfig` + `model_ids`（供 agent 模型接线查找）
- **`ProviderForModel`** — `model_id → provider_uuid` 查找项，给 agent 的 primary 模型接线用

### OpenClaw 源反序列化（deserialize-only）

- **`OpenClawConfigRoot`** — `openclaw.json` 根：`models`（含 `providers`）+ `auth`
- **`AuthProfilesFile` / `AuthCredentialEntry` / `SecretRef`** — 每 agent `auth-profiles.json` 凭据结构；`SecretRef` 表达 `env` / `file` / `exec` 三种 keyRef 引用
- **`OpenClawAgent`** — `agents.list` 单条：`id` / `name` / `workspace` / `model` / `identity` / `skills` / `tools` / `sandbox` / `params` 等
- **`OpenClawAgentModel`** — 自定义 `Deserialize`，兼容 `model` 既可是裸字符串、也可是 `{ primary }` 对象

## Provider 映射

`build_providers` 是核心：吃 raw `OpenClawConfigRoot` + 收集到的凭据，吐出 `(ProviderPreview, ResolvedProvider)` 列表。

**API 类型映射** 走 `map_api_type`（`pub`，单测覆盖），把 OpenClaw 的 `ModelApi` 翻成 Hope Agent 的 [`ApiType`](provider-system.md)，无法精确对应时 push 警告。**关键红线**：`openai-codex-responses` 必须映射成 `ApiType::OpenaiResponses` 而**不是** `ApiType::Codex`——后者是 OAuth-only，会让外部 API key 不可用。

**成本归一化**：成本值疑似 per-token 时（`< 0.01`）× 1e6 归一化为 per-million，对齐 Hope Agent 的成本口径。

**私网放行**：`base_url` 落在私网（`localhost` / `127.0.0.1` / `0.0.0.0`）时自动置 `allow_private_network=true`（Ollama 等本地后端能打通）。

**Provider 写入** 经 [`provider/crud.rs`](../../crates/ha-core/src/provider/crud.rs) 的 `add_many_providers`，`source="openclaw-import"`——遵守 [provider 写入 contract](provider-system.md)，绝不直接 `providers.push`。

## 凭据收集与导入策略

`collect_credentials` 遍历 `agents/*/agent/auth-profiles.json`，按 `(provider, profileId)` 联合凭据——同一 provider 跨 agent 的多份凭据收拢到一起。每份凭据按 `CredentialKind` 分类决定导入策略：

| keyRef 类型 | 处理 | will_import |
|---|---|---|
| `api_key` / `token`（明文） | 直接导入 | ✓ |
| `env` keyRef | 经 `std::env::var` 解析；解析到则导入 | 视解析结果 |
| `OAuth` | **永不导入**，强制用户在 Hope Agent 重新登录 | ✗ |
| `exec` keyRef | **出于安全拒绝**（不执行命令取密钥） | ✗ |
| `file` keyRef | **不支持** | ✗ |

OAuth / exec / file 三类都给出 `note` 提示，要求用户导入后手动粘贴 key 或重新登录。

## Agent 导入

`build_previews` 把 `agents.list` 转成 `OpenClawAgentPreview` 列表，经 `agent_loader::list_agent_ids` 标 `already_exists`——已存在不阻止导入，但前端 / CLI 默认过滤掉。

`import_single_agent` 写一个 agent：

1. **`target_id` 校验**：必须非空且仅 ASCII 字母数字与连字符，否则 `bail`
2. **写 agent.json**：经 `agent_loader::save_agent_config` 落 `~/.hope-agent/agents/{target_id}/agent.json`；`system_prompt_override → agent.md` 经 `save_agent_markdown`
3. **拷贝 workspace markdown**：把 agent workspace 下的 `AGENTS.md` / `SOUL.md` / `TOOLS.md` / `IDENTITY.md` 大写文件名映射成对应小写文件拷入新 agent 目录
4. **接线 primary 模型**：从 `model_id → provider` 查找表里解析 primary 模型挂到 provider UUID
5. **`openclaw_mode=true`**：写进 agent.json，影响 `system_prompt::build` 走 4 文件 markdown prompt 模式

**模型查找表** 先由 `build_model_lookup` 从同批导入的 provider 构建，再经 `extend_model_lookup_from_provider_configs` 用已配 provider 兜底——这是为了**防部分导入重复**：用户若早前已配过同一 provider，agent 模型接线优先复用现有 provider，而非又新建一份。

**工具开关不导入**：OpenClaw 的 tools allow/deny 设置**不迁移**，仅 push 警告让用户手动核对 Hope Agent 的工具开关。

## Memory 导入

记忆 caller（`import_openclaw_full`）在 agents 之后处理，两条来源：

**MEMORY.md**：导入路径把 markdown **原文**经 `CoreMemoryRepository` 合并进 canonical `MEMORY.md`（经合并段 + 备份，见下），并非逐条插入记忆库；`parse_openclaw_memory_md`（bullet 项 / 段落各成一条、跳过 heading、`source="import"`）仅经 `estimate_entries` 用于预览 / 计数。

**SQLite 向量库**（`parse_openclaw_sqlite_memory_db`）：以 `SQLITE_OPEN_READ_ONLY` 打开 `~/.openclaw/memory/{agentId}.sqlite`，**只读 `chunks` 表的 `text` 列**（按 `updated_at ASC, id ASC` 排序、跳过空白），**丢弃 embedding**（model / dimension / signature 契约与 Hope Agent 不同，无法复用）。这些 chunk 行作 `NewMemory`（`source="openclaw-db-import"`）经 backend `import_entries` 落库。

**写入路径**：

- 全局记忆 → `~/.hope-agent/memory/MEMORY.md`
- agent 记忆 → `~/.hope-agent/agents/{id}/memory/MEMORY.md`
- 两者都经**统一 repository + 合并段 + 写前备份**（见下「核心 MEMORY.md 合并」）
- SQLite chunk 文本经 memory backend 的 `import_entries(dedup=true)` 落记忆数据库——dedup 交给 backend，`skipped_duplicate` / `failed` / `errors` 转 `warnings`；**backend 未初始化则跳过记忆导入并 warn**

**注意区分**：workspace 下的 `MEMORY.md` / `memory.md` 是**记忆**，不是 agent markdown 文件，**不能**当 agent `.md` 文件夹处理。

### 核心 MEMORY.md 合并

`merge_openclaw_memory_section` 把导入内容包进 `BEGIN/END` 标记段，幂等替换后经统一 repository 写入核心 `MEMORY.md`：

- 标记段**幂等替换**——重复导入只更新标记段内内容，不无限堆叠
- 合并前先 `backup_existing_core_memory_md` 把现有 `MEMORY.md` 备份到 `~/.hope-agent/backups/openclaw-memory-import/<UTC时间戳>/<相对路径>`
- **绝不裸覆盖**用户现有 `MEMORY.md`
- 空内容不写（返回 `false` 不计数）

## 去重与冲突处理

| 冲突 | 处理 |
|---|---|
| Provider 名与现有 config 重名 | 加 `" (Imported)"` / `" (Imported N)"` 后缀，`name_conflicts_existing=true` |
| Agent `target_id` 已存在 | `already_exists=true`，不阻止导入但前端 / CLI 默认过滤 |
| source agent 找不到 | 该 agent 失败（`ImportResult.error`），整体继续 |
| 同一 model id 出现在多个 provider | 经查找表仲裁，已配 provider 兜底防部分导入重复 |
| 记忆 backend dedup | 交给 `import_entries(dedup=true)`，重复跳过转 warning |

## 对外接口面

| 平面 | 入口 |
|---|---|
| Tauri 命令 | `scan_openclaw_agents`（legacy）/ `import_openclaw_agents`（legacy）/ `scan_openclaw_full` / `import_openclaw_full` |
| HTTP 路由 | `GET /api/agents/openclaw/scan`（legacy）/ `POST /api/agents/openclaw/import`（legacy）/ `GET /api/agents/openclaw/scan-full` / `POST /api/agents/openclaw/import-full` |
| 前端 | `OpenClawImportDialog`（多选粒度勾选） |
| CLI | onboarding 步骤「import-openclaw」（step 2，单条 yes/no 全导或全不导；排在 mode 步骤之前故不受 remote 模式短路影响，见 [`cli.md`](cli.md)） |

四条 Tauri ↔ HTTP 对齐行登记在 [`api-reference.md`](api-reference.md)。**legacy shim**（`scan_openclaw_agents` / `import_openclaw_agents`）只覆盖 agents——`scan` 仅返回 full scan 的 agents 部分，`import` 仅导 agents（不含 providers / memory），保留旧入口兼容。

## 持久化

### 读取源（OpenClaw）

| 路径 | 内容 |
|---|---|
| `~/.openclaw/openclaw.json`（旧 `~/.clawdbot/clawdbot.json` 回退） | 主配置：`models` + `auth` |
| `~/.openclaw/agents/{agentId}/agent/auth-profiles.json` | 每 agent 凭据 |
| `~/.openclaw/agents/{agentId}/agent/MEMORY.md` | agent 记忆 markdown |
| `~/.openclaw/agents/{agentId}/workspace/{MEMORY,memory}.md` | workspace 记忆 |
| `~/.openclaw/MEMORY.md` | 全局记忆 |
| `~/.openclaw/memory/{agentId}.sqlite` | 向量库，仅读 `chunks` 表 `text` 列，`SQLITE_OPEN_READ_ONLY` |
| agent workspace 下 `AGENTS.md` / `SOUL.md` / `TOOLS.md` / `IDENTITY.md` | agent markdown，大写 → 小写映射拷贝 |

### 写入目标（Hope Agent）

| 目标 | 经手 |
|---|---|
| Provider 列表 → `config.json` | `provider::add_many_providers`（`source="openclaw-import"`） |
| `~/.hope-agent/agents/{target_id}/agent.json` | `agent_loader::save_agent_config` |
| 对应 `.md`（`system_prompt_override → agent.md`） | `save_agent_markdown` |
| 全局记忆 → `memory/MEMORY.md` | 统一 repository + 合并段 + 写前备份 |
| agent 记忆 → `agents/{id}/memory/MEMORY.md` | 统一 repository + 合并段 + 写前备份 |
| SQLite chunk 文本 → 记忆数据库 | memory backend `import_entries(dedup=true)`（`source="openclaw-db-import"`；MEMORY.md 解析 `source="import"`） |
| `~/.hope-agent/backups/openclaw-memory-import/<UTC时间戳>/<相对路径>` | `backup_existing_core_memory_md` |

`AgentConfig.openclaw_mode=true` 写入 agent.json，影响 [`system_prompt`](prompt-system.md)::build 走 4 文件 markdown prompt 模式。

**环境变量**：`OPENCLAW_STATE_DIR` 覆盖状态目录（测试 / 高级用户）；`env` keyRef 经 `std::env::var` 解析。

## 事件

| 事件 | 触发 |
|---|---|
| `agents:changed` | agent 导入后，让前端刷新 agent 列表 |
| `config:changed` | provider 写入经 `mutate_config` 自动 emit（见 [config 写入 contract](config-system.md)） |

## 导入顺序（硬约束）

`import_openclaw_full` 顺序固定 **providers → agents → memory**，不可重排：

1. **providers** 先落库，拿到新 provider 的 UUID
2. **agents** 接线 primary 模型时依赖同批导入的 provider UUID——`build_model_lookup` 先用同批 provider 建表，再 `extend_model_lookup_from_provider_configs` 用已配 provider 兜底（防部分导入重复 provider）
3. **memory** 最后处理（合并段 + 备份 + backend dedup）

## 安全 / 红线

- **OAuth 永不导入**（`will_import=false`），强制用户在 Hope Agent 重新登录；`map_api_type` 把 `openai-codex-responses` 映射成 `OpenaiResponses` 而非 `ApiType::Codex`（后者 OAuth-only），否则外部 API key 不可用
- **exec keyRef 拒绝**（不执行命令取密钥）；**file keyRef 不支持**——二者都要求用户导入后手动粘贴 key
- **导入顺序硬约束** providers → agents → memory：agent primary 模型接线依赖同批 provider UUID，`extend_model_lookup_from_provider_configs` 用已配 provider 兜底防部分导入重复
- **MEMORY.md 写入必须经** `merge_openclaw_memory_section`（BEGIN/END 幂等替换段）+ `CoreMemoryRepository` + 写前 `backup_existing_core_memory_md`，**绝不裸覆盖**用户现有 `MEMORY.md`；空内容不写
- **SQLite 向量库只导 chunk 文本、丢弃 embedding**（model / dimension / signature 契约不同），只读打开；workspace 的 `MEMORY.md` 是记忆**不是** agent markdown，不能当 agent `.md` 文件夹处理
- **`target_id` 校验**：必须非空且仅 ASCII 字母数字与连字符，否则 `bail`；source agent 找不到则该 agent 失败但整体继续
- **provider 名冲突** 加 `" (Imported)"` / `" (Imported N)"` 后缀，`name_conflicts_existing=true`；agent `already_exists` 不阻止但默认过滤
- **记忆 dedup 交给 backend** `import_entries(dedup=true)`，`skipped_duplicate` / `failed` / `errors` 转 warnings；backend 未初始化则跳过记忆导入并 warn
- **私网 base_url** 自动置 `allow_private_network=true`；成本疑似 per-token（`< 0.01`）× 1e6 归一化 per-million
- **工具 allow/deny 不导入**，仅 push 警告让用户手动核对 Hope Agent 工具开关

## 已知限制

- **legacy agents-only 入口**（`scan_openclaw_agents` / `import_openclaw_agents`）只迁 agent，不含 providers / memory，为旧入口兼容保留
- **OAuth 重登**：OAuth provider 导入后需用户在 Hope Agent 重新登录
- **file / exec keyRef 手填**：这两类 keyRef 不自动解析，需用户导入后手动粘贴 key

## 跨子系统

| 子系统 | 关系 |
|---|---|
| [Provider](provider-system.md) | provider 写入经 `add_many_providers`；`ApiType` 映射；私网 / 成本归一化 |
| [Agent 解析链](backend-separation.md) | agent 经 `agent_loader` 落库；`openclaw_mode` 影响 prompt 构建 |
| [Memory](memory.md) | 记忆经 backend `import_entries(dedup=true)`；丢弃 embedding |
| [Prompt System](prompt-system.md) | `openclaw_mode=true` 走 4 文件 markdown prompt 模式 |
| [Config](config-system.md) | provider 写入经 `mutate_config` + `config:changed` + autosave（`source="openclaw-import"`） |
| [CLI](cli.md) | onboarding 步骤「import-openclaw」（step 2）单 yes/no；排在 mode 步骤之前，remote 短路只跳后续 provider/server 等步骤、不跳本步 |

## 关键文件索引

| 文件 | 角色 |
|---|---|
| [`crates/ha-core/src/openclaw_import/mod.rs`](../../crates/ha-core/src/openclaw_import/mod.rs) | 子系统根 + 四入口 + 状态目录解析 + 记忆合并段 / 备份 + 顶层数据结构 |
| [`crates/ha-core/src/openclaw_import/providers.rs`](../../crates/ha-core/src/openclaw_import/providers.rs) | provider 映射 + 凭据收集 + `map_api_type` + 反序列化结构 |
| [`crates/ha-core/src/openclaw_import/agents.rs`](../../crates/ha-core/src/openclaw_import/agents.rs) | agent 映射 + 单 agent 导入 + 模型查找表 |
| [`crates/ha-core/src/openclaw_import/memory.rs`](../../crates/ha-core/src/openclaw_import/memory.rs) | MEMORY.md 解析 + SQLite chunk 文本只读导入 |
