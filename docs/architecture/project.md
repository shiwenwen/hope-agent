# Project 项目系统架构

> 返回 [文档索引](../README.md) | 更新时间：2026-06-07

## 目录

- [概述](#概述)
- [数据模型](#数据模型)
- [SQLite Schema](#sqlite-schema)
- [磁盘布局](#磁盘布局)
- [核心 API](#核心-api)
- [文件浏览器 API](#文件浏览器-api)
- [Agent 解析链（7 级）](#agent-解析链7-级)
- [工作目录解析链（session > project > 默认 workspace）](#工作目录解析链session--project--默认-workspace)
- [`/project` 与 `/projects` 斜杠命令](#project-与-projects-斜杠命令)
- [System Prompt 注入](#system-prompt-注入)
- [记忆系统接入](#记忆系统接入)
- [级联删除与孤儿清理](#级联删除与孤儿清理)
- [接入层](#接入层)
- [前端 UI](#前端-ui)
- [EventBus 事件](#eventbus-事件)
- [启动顺序](#启动顺序)
- [安全约束](#安全约束)
- [关联文档](#关联文档)
- [文件清单](#文件清单)

---

## 概述

Project 是 Hope Agent 的**可选会话容器**，把多个会话聚成一个工作空间共享三样东西：

1. **项目记忆**（`MemoryScope::Project { id }`）—— 项目内可见、跨项目隔离，注入优先级最高
2. **项目指令**（`instructions`）—— 装配进每个项目内会话的 System Prompt
3. **统一工作目录** —— 每个项目有一个真实工作目录（用户显式选的目录，或默认 `~/.hope-agent/projects/{id}/workspace/`）；上传、agent 产出、文件浏览都围绕它

**项目文件 = 工作目录里的真实文件**——这是核心哲学（对齐 [Project 的「文件即真实文件」](file-operations.md)）：上传文件直接落工作目录，模型靠 System Prompt `# Working Directory` 段的顶层文件清单 + `read` 工具感知，**没有** `project_files` 表、独立 `files/`/`extracted/` 目录、文本提取注入或 `project_read_file` 工具。文件读写统一走 [文件浏览器 API](#文件浏览器-api)（`WorkspaceScope` 作用域闭合）。

`sessions.project_id = NULL` 的会话保留 pre-project 行为，完全不受影响——项目是 opt-in 容器，不是对话的必需分组。

核心设计取舍：

- **复用 `sessions.db`**：`projects` 表与 `sessions` 表同 DB（`ProjectDB` 持 `Arc<SessionDB>`），项目与会话的关系查询可在单库内完成。
- **跨 DB 内存**：项目记忆在独立的 `memory.db`，无法与 `sessions.db` 共享事务；删除时分两库执行，靠启动期 reconciler 兜底孤儿清理。
- **工作目录单一真相源**：[`session::effective_session_working_dir`](../../crates/ha-core/src/session/helpers.rs)（lazy ensure 默认 workspace），文件浏览器读写经 [`filesystem::WorkspaceScope`](../../crates/ha-core/src/filesystem/workspace.rs)（canonicalize + `starts_with` 失败闭合）。
- **无反向认领**：项目不认领 (channel, account)；IM 会话归项目靠 chat 内 `/project <id>` 显式触发（详见 [Agent 解析链](#agent-解析链7-级) 与 [im-channel.md](im-channel.md)）。

## 数据模型

### Project（[`types.rs`](../../crates/ha-core/src/project/types.rs)）

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `String` | UUID v4 主键 |
| `name` | `String` | 项目名称（trim 后不得为空） |
| `description` | `Option<String>` | 项目简介 |
| `instructions` | `Option<String>` | 自定义指令，追加到项目内每个会话的 System Prompt |
| `emoji` | `Option<String>` | 侧边栏 / 标题前缀 emoji |
| `logo` | `Option<String>` | 项目 logo data URL（`data:image/...;base64,...`），优先于 `emoji` 渲染（见 [安全约束](#安全约束)） |
| `color` | `Option<String>` | 强调色（UI 内部装饰用） |
| `default_agent_id` | `Option<String>` | 新建会话的默认 Agent（解析链第 2 级） |
| `default_model_id` | `Option<String>` | 已废弃兼容列；不参与解析。项目会话使用默认 Agent，并在 Session 创建时固定该 Agent 的运行默认值 |
| `working_dir` | `Option<String>` | 项目级默认工作目录（绝对路径）；session 未单独设置时回落到此；`NULL` = 用默认 workspace |
| `created_at` / `updated_at` | `i64` | Unix 毫秒时间戳 |
| `archived` | `bool` | 归档标志（不删除，默认列表过滤） |

### ProjectMeta（[`types.rs`](../../crates/ha-core/src/project/types.rs)）

`Project`（flatten）+ 三个聚合计数：

- `session_count` —— 项目内会话数（`ProjectDB::list` 子查询）
- `unread_count` —— 项目内非 IM 会话的未读消息数（子查询，`source != 'channel'`），`/projects` 列表红点用；`mark_project_sessions_read` 清零
- `memory_count` —— **跨 DB**，`ProjectDB::list` 置 0，由调用方（Tauri / HTTP 层）用 `backend.count_by_project(&id)` 补齐

### 输入 DTO

- `CreateProjectInput`：`name` 必填，其余可选（含 `logo` / `working_dir` 等所有可写字段）
- `UpdateProjectInput`：PATCH 语义，所有字段 `Option<_>`（`None`=不变，`Some("")`=清空）
- `working_dir` 在 update 路径走 [`util::canonicalize_working_dir`](../../crates/ha-core/src/util.rs)（空串当清空，否则 canonicalize + `is_dir` 校验，不通过 `Err`）

> 早期版本的 `ProjectFile` 类型 / `BoundChannel` 类型均已删除——文件改为工作目录真实文件，IM 反向认领已废弃。

## SQLite Schema

`projects` 表随 `SessionDB` 连接共享，由 [`ProjectDB::migrate()`](../../crates/ha-core/src/project/db.rs) 幂等建表：

```sql
CREATE TABLE IF NOT EXISTS projects (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL,
    description       TEXT,
    instructions      TEXT,
    emoji             TEXT,
    color             TEXT,
    default_agent_id  TEXT,
    default_model_id  TEXT,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    archived          INTEGER NOT NULL DEFAULT 0,
    logo              TEXT,
    working_dir       TEXT
);
CREATE INDEX IF NOT EXISTS idx_projects_archived
    ON projects(archived, updated_at DESC);
```

**遗留清理迁移**（`migrate()` 内一次性 drop，无数据迁移——破坏性直接 drop）：

- `DROP TABLE IF EXISTS project_files` + 其索引（文件改为工作目录真实文件）
- `ALTER TABLE projects DROP COLUMN bound_channel_id / bound_channel_account_id` + `idx_projects_bound_channel`（IM 反向认领废弃，需 SQLite 3.35+）

**sessions 表扩展**（[`session/db.rs`](../../crates/ha-core/src/session/db.rs)）：迁移阶段 `ALTER TABLE sessions ADD COLUMN project_id TEXT` + 建 `idx_sessions_project_id`，老库零破坏升级。

## 磁盘布局

```
~/.hope-agent/
├── sessions.db                        # projects + sessions 同一个 DB
├── memory.db                          # 项目记忆（独立 DB，MemoryScope::Project）
└── projects/
    └── {project_id}/
        └── workspace/                 # 默认工作目录（未显式选目录时）；上传/产出/浏览都在此
            └── <用户与 agent 的真实文件>
```

> 用户在项目设置里**显式选了** `working_dir` 时，工作目录指向那个外部真实目录（不在 `projects/{id}/` 内），`projects/{id}/` 可能为空。

路径由 [`paths.rs`](../../crates/ha-core/src/paths.rs) 集中管理：`projects_dir()` / `project_dir(id)` / `project_workspace_dir(id)`。工作目录解析的单一入口是 [`project::resolve_project_dir`](../../crates/ha-core/src/project/files.rs)（显式 `working_dir` 优先，否则 lazy 创建默认 workspace 并 `ensure_dir_canonical` 返回）。

## 核心 API

### ProjectDB（[`db.rs`](../../crates/ha-core/src/project/db.rs)）

| 方法 | 说明 |
|---|---|
| `create(CreateProjectInput)` → `Project` | 插入新项目 |
| `get(id)` → `Option<Project>` | 取单个项目 |
| `update(id, UpdateProjectInput)` → `Project` | 动态 SQL 部分更新；普通字段空串 → NULL |
| `delete(id)` → `()` | 单 TX 两步：① `UPDATE sessions SET project_id = NULL`（会话保留）② `DELETE FROM projects`。磁盘 / 记忆清理由 `delete_project_cascade` 在 TX 外接续 |
| `list_all_ids()` → `Vec<String>` | 轻量 id 列表，reconciler 专用 |
| `list(include_archived)` → `Vec<ProjectMeta>` | 带 `session_count` / `unread_count` 聚合子查询；`memory_count = 0` 待调用方补齐 |

项目不再有任何文件 CRUD——文件读写全在 [文件浏览器 API](#文件浏览器-api)。

### session ↔ project 绑定（[`session/db.rs`](../../crates/ha-core/src/session/db.rs)）

| 方法 | 说明 |
|---|---|
| `create_session_with_project(agent_id, project_id)` | 带项目归属创建会话 |
| `set_session_project(session_id, project_id)` | 搬迁会话到另一个项目或 unassign（`/project` IM 路由、`move_session_to_project` 共用） |
| `list_sessions_paged(agent_id, project_filter, limit, offset)` | `ProjectFilter`：`All` / `Unassigned` / `InProject(id)` |

**项目会话懒创建（desktop / HTTP 交互入口）**：进项目「新建对话」**不再**预先 `create_session_cmd` 落库，而是停在草稿态（`currentSessionId=null`），前端用 `draftProjectId` 记住项目（仿 `draftWorkingDir`），首条消息发送时通过 `chat` 命令的 `projectId` 走 `create_session_with_project` 才落库——与普通对话对称，进项目不再产生未发消息的空会话行，且草稿态走与普通对话相同的模型 / 权限模式 seeding。`chat` 在 `agent_id` 缺省时按 `project.default_agent_id` 解析 agent（对齐 `create_session_cmd`），`project_id` 与 `incognito` 互斥（后端强制 off）。**仅交互入口懒创建**——IM 入站 / cron / subagent 仍 eager `create_session_with_project`（消息必须立即落库）。前端 `effectiveProjectId = 已加载会话 meta.projectId ?? draftProjectId` 是「当前在哪个项目」的单一来源（覆盖草稿态 + 落库过渡窗口，避免 badge 闪烁与切到普通会话时的陈旧泄漏）。

项目草稿在首发前还维护 `ProjectRuntimeDraft`：默认 `local`；Git 项目在 `local` / `worktree` 两种运行位置下都可从本地/remote-tracking 分支中选择起点。切换项目保留 composer 文本、普通附件和引用，但清空草稿 KB attach、Git 缓存和运行位置。首次发送通过 `ChatStartArgs.projectBootstrap` 接入 Tauri/HTTP 共用的 `ha-core::project_bootstrap` 编排；已有 session 携带该字段、非项目草稿、归档项目、非法 ref 或非 Git 目录均 fail closed。统一目录、Bootstrap 状态机、脏改动复制、清理与恢复契约见 [Managed Worktree 控制平面](worktree.md#项目首轮-bootstrap)；Session materialize 后的 Diff、分支、提交、推送、PR 与双向 Handoff 见 [Session Git 控制平面](git-control.md)。首版不包含“环境”配置。

## 文件浏览器 API

项目文件由 workspace-scoped 文件管理 API 读写，全部经 [`filesystem::WorkspaceScope`](../../crates/ha-core/src/filesystem/workspace.rs)（`for_session` / `for_project` / `for_path` 三入口 → canonicalize 根 → 每次操作 canonicalize 目标 + `starts_with` 校验，失败闭合；`for_path` 是只读 worktree 跳转，写操作经 `resolve_writable` 一律拒绝）。核心 ops 在 [`filesystem/ops.rs`](../../crates/ha-core/src/filesystem/ops.rs)：list / read_text / extract（PDF 逐页 PNG、Office 文本+图片，复用 `file_extract`）/ write_text / delete / rename / mkdir / upload。

接入：Tauri 命令 `project_fs_*`（[`commands/project_fs.rs`](../../src-tauri/src/commands/project_fs.rs)：`list` / `read_text` / `extract` / `resolve` / `write_text` / `delete` / `rename` / `mkdir` / `upload` + `project_git_info`）+ HTTP `/api/fs/*`（[`routes/project_fs.rs`](../../crates/ha-server/src/routes/project_fs.rs)）+ Transport 双适配。`project_git_info` 是只读接口，统一返回当前分支、local/remote-tracking 分支（排除 remote HEAD 符号引用）、dirty summary 和 worktree checkout 信息，不 fetch/checkout。HTTP **写**端点（write / delete / rename / mkdir / upload）受 `filesystem.allow_remote_writes`（默认 false）闸门，桌面 Tauri 不受限。单文件上限 `MAX_PROJECT_FILE_BYTES = 20 MB`。

**preview-by-path**（按绝对路径读取 / 提取）：Tauri `preview_read_text` / `preview_extract` + 客户端 `convertFileSrc`；HTTP `GET /api/sessions/{id}/files/{read,extract,by-path}` 共用 `authorized_canonical_file_path`（被会话 tool 消息引用 ∪ 落在会话工作目录内），二者皆非的主机任意路径一律 403——远端严禁放行任意主机路径；桌面信任本机。详见 [file-operations.md](file-operations.md)。

前端组件在 [`src/components/chat/project/file-browser/`](../../src/components/chat/project/file-browser/)，挂载于项目设置 Files 标签（`stacked`）与主聊天区右侧面板（`split`），CRUD 后发 `project:fs_changed` 事件跨视图同步。详见 [api-reference.md](api-reference.md) 端点对照表。

## Agent 解析链（7 级）

新会话 `agent_id` 解析统一走 [`agent::resolver::resolve_default_agent_id_full`](../../crates/ha-core/src/agent/resolver.rs)（首个非空胜出；`_with_source` 变体携带来源 tag 供 `/status` 显示命中级别）。无 IM 上下文的 desktop / HTTP 用 `resolve_default_agent_id(project, channel_account)` 包装（只传项目 + channel-account 两级）。

| 优先级 | 来源 | 触发条件 |
|---|---|---|
| 1 | **显式参数** | 调用方在 API / Tauri 命令里直接传 `agent_id` |
| 2 | **`project.default_agent_id`** | session 落入项目，项目设置了默认 Agent |
| 3 | **IM topic** `TelegramTopicConfig.agent_id` | Telegram forum topic 级覆盖（最具体 IM scope） |
| 4 | **IM group** `TelegramGroupConfig.agent_id` | 群级覆盖 |
| 5 | **IM tg-channel** `TelegramChannelConfig.agent_id` | 广播频道级覆盖 |
| 6 | **`channel_account.agent_id`** | IM channel account per-account 软默认 |
| 7 | **`AppConfig.default_agent_id`** | 全局设置，默认 `"ha-main"` |
| — | **硬编码 `"ha-main"`** | 兜底常量（`agent_loader::DEFAULT_AGENT_ID`），保证永远返回非空 id |

> channel worker 不自写解析链——统一收敛到 resolver 单一真相源。

### 配套 API

| 入口 | 作用 |
|---|---|
| Tauri `get_default_agent_id` / `set_default_agent_id` | 读 / 写 `AppConfig.default_agent_id` |
| HTTP `GET / PUT /api/config/default-agent` | 同上 |
| `ha-settings` 工具 `category="default_agent"` | 模型可改（LOW 风险，SKILL.md 已登记） |
| `/status` 斜杠命令 | 项目会话里追加项目摘要段，标注 Agent Source 命中级别 |

## 工作目录解析链（session > project > 默认 workspace）

会话最终工作目录由 [`session::helpers::effective_session_working_dir`](../../crates/ha-core/src/session/helpers.rs)（+ `effective_working_dir_for_meta`）单一入口解析：

```
session.working_dir 非空？      → 用之（会话级）
否则 session.project_id Some？  → project 显式 working_dir，或 lazy 创建的默认 workspace
否则                            → 默认 workspace（无项目时按需创建）
```

**项目会话总有工作目录**——显式 `working_dir` 或 lazy 创建的默认 `~/.hope-agent/projects/{id}/workspace/`。**Lazy ensure**：默认 workspace 在首次解析时 `ensure_dir_canonical` 创建并返回，**不写进 DB**（`project.working_dir` 留 NULL，保持 `HA_DATA_DIR` 可迁移）。改 `working_dir` 立即对未单独设置的项目内已有会话生效（lazy resolve，不复制快照）。

### 写入校验入口

[`util::canonicalize_working_dir`](../../crates/ha-core/src/util.rs)（session / project 共用）：空串当清空（写 NULL），非空 → `canonicalize` + `is_dir` 校验，不通过 `Err`。

### 消费点

| 消费方 | 作用 |
|---|---|
| **System Prompt 渲染**（[`agent/config.rs`](../../crates/ha-core/src/agent/config.rs)） | 把合并值传给 `system_prompt::build`，注入 `# Working Directory` 段 |
| **主对话工具执行**（[`agent/mod.rs`](../../crates/ha-core/src/agent/mod.rs)） | 写入 `ToolExecContext.session_working_dir`，被 `read` / `write` / `exec` 解析相对路径 |
| **斜杠命令执行**（[`slash_commands/handlers/mod.rs`](../../crates/ha-core/src/slash_commands/handlers/mod.rs)） | 让内置命令也走合并值 |

### UI 区分两种来源

[`WorkingDirectoryButton`](../../src/components/chat/input/WorkingDirectoryButton.tsx) / `ChatTitleBar` 显示生效路径并区分：

- **会话级**（`session.working_dir` 非空）：显示路径 + clear 按钮
- **继承自项目**（`session.working_dir` 空 + 走 `project.working_dir`）：显示路径 + 标注「继承自项目」，**不渲染 clear 按钮**（避免 no-op 误操作）

## `/project` 与 `/projects` 斜杠命令

源：[`slash_commands/handlers/project.rs`](../../crates/ha-core/src/slash_commands/handlers/project.rs)。

| 形式 | 行为 |
|---|---|
| `/projects` | picker：返回 `ShowProjectPicker`，前端渲染项目选择器 |
| `/project`（无参） | 同 picker（`ShowProjectPicker`） |
| `/project <name>`（desktop / HTTP） | fuzzy 匹配 → `EnterProject` action → 前端创建项目作用域新会话 |
| `/project <name>`（IM 会话） | fuzzy 匹配 → `AssignProject` action → channel worker 调 `set_session_project` 直接 UPDATE 现有 `sessions.project_id`，**不创建新 session** |

> **IM 可用**：`/project` 在 IM 渠道**不再禁用**（早期曾因「IM 每条消息重算归属会拉回切换」而禁用，现已通过 `AssignProject` 真正落地到现有 session 解决）。当前 `IM_DISABLED_COMMANDS = ["agent", "handover"]`（[`slash_commands/registry.rs`](../../crates/ha-core/src/slash_commands/registry.rs)），不含 `project`。

## System Prompt 注入

会话挂到项目后，`system_prompt::build` 在 Memory 段之前注入 `# Current Project`，再注入 `# Working Directory`（位置：Project 段之后、Memory 段之前）。

- **`# Current Project`**（[`system_prompt/sections.rs`](../../crates/ha-core/src/system_prompt/sections.rs)）：`Description` + `## Project Instructions`（truncate 到上限），并尾随一句「本会话 `save_memory` 默认为 project scope」提示。
- **`# Working Directory`**（[`prompt-system.md`](prompt-system.md)）：路径声明 + `## Working Directory Instructions` 子节（工作目录里的 AGENTS.md / CLAUDE.md 指令）。位置在 Project 段之后、Memory 段之前。
- **`# Files in Working Directory`**（**独立顶层段，emit 在最末**——在 Memory / weather 等所有静态段之后，见 [`system_prompt/build.rs`](../../crates/ha-core/src/system_prompt/build.rs)）：顶层文件清单（非递归、只列名、名称排序、跳过隐藏与 `.git`/`node_modules`、cap ~100）。刻意拆成尾段——文件增删只 bust 这一尾块、不波及静态前缀缓存（同一目录状态产出 byte-identical 文本）。模型靠普通 `read` 工具按需读文件。

> 早期的「`# Project Files` 三层注入（目录清单 / 小文件内联 / `project_read_file`）」已整体废弃，由上面的 `# Files in Working Directory` 尾段清单 + `read` 工具取代。

## 记忆系统接入

**MemoryScope 第三变种**（[`memory/types.rs`](../../crates/ha-core/src/memory/types.rs)）：

```rust
pub enum MemoryScope {
    Global,
    Agent { id: String },
    Project { id: String },  // 仅项目内共享
}
```

- **注入优先级**（[`memory/sqlite/trait_impl.rs`](../../crates/ha-core/src/memory/sqlite/trait_impl.rs)）：`Project（最高）→ Agent → Global（最低，shared=true 时）`。Memory Budget 裁剪时越靠前越不易被丢弃，确保项目上下文优先保留。
- **自动提取作用域**（[`memory_extract.rs`](../../crates/ha-core/src/memory_extract.rs)）：项目内会话 `save_memory` 不传 scope 时默认写 `Project`；可显式 `scope='global'` / `'agent'` 打破项目边界。
- **计数跨 DB**：`ProjectDB::list` 置 `memory_count = 0`，调用方遍历 `backend.count_by_project(&id)` 补齐。

## 级联删除与孤儿清理

### delete_project_cascade（[`files.rs`](../../crates/ha-core/src/project/files.rs)）

```
1. session.db 单 TX（ProjectDB::delete）：
   ① UPDATE sessions SET project_id = NULL WHERE project_id = ?   (会话本体保留)
   ② DELETE FROM projects WHERE id = ?
2. 磁盘：purge_project_dir(id) — remove_dir_all `projects/{id}/`，带路径逃逸防护
       （用户显式选的外部 working_dir 在 projects/ 之外，永不删）
3. memory.db（独立 DB）：list(Project scope, 10_000) → delete_batch(ids)
```

**步骤 2、3 在事务外**（跨文件系统 / 跨 DB 无法共享 TX）。设计取舍：若第 1 步后崩溃 → 孤儿 = `projects/{id}/` 目录 + `memory.db` 中该 scope 的记忆行，**均对应用无害**（id 已不存在，永不会被 `list` 查出），靠启动期 reconciler 懒清理而非同步事务。

### Startup Reconciler（[`reconcile.rs`](../../crates/ha-core/src/project/reconcile.rs)）

`spawn_startup_reconciler()` 在 `app_init` 后台 `spawn_blocking` 一次性执行，失败只 `app_warn!` 绝不阻塞启动：`list_all_ids()`（alive）与 `backend.list_distinct_project_scope_ids()`（referenced）求差集 → 对每个孤儿 id `list(Project scope) → delete_batch`。项目删除频率低，无周期 timer，重启时一次扫描足够。

### purge_project_dir 防逃逸

canonicalize `dir` + canonicalize `projects_root`，`starts_with(canonical_root)` 不成立 → `app_error!` 拒绝 `remove_dir_all`。防御符号链接越界 / 遍历式 project id（虽然 id 来自 `Uuid::new_v4()` 不会构造 `..`）。

## 接入层

### Tauri 命令（[`commands/project.rs`](../../src-tauri/src/commands/project.rs)）

注册在 [`src-tauri/src/lib.rs`](../../src-tauri/src/lib.rs) `invoke_handler!`：

| 命令 | 作用 |
|---|---|
| `list_projects_cmd(include_archived?)` | 列表 + 跨 DB 补齐 memory_count |
| `get_project_cmd(id)` | 取单个 |
| `create_project_cmd(input)` | emit `project:created` |
| `update_project_cmd(id, patch)` | emit `project:updated` |
| `delete_project_cmd(id)` | 走 `delete_project_cascade`，emit `project:deleted` |
| `archive_project_cmd(id, archived)` | 等价 patch `{archived}`，emit `project:updated` |
| `list_project_sessions_cmd(id, limit?, offset?)` | 基于 `ProjectFilter::InProject`，含 `enrich_pending_interactions` |
| `move_session_to_project_cmd(session_id, project_id?)` | `project_id=None` 即 unassign |
| `mark_project_sessions_read_cmd(id)` | 清零项目 `unread_count` |
| `list_project_memories_cmd(id, limit?, offset?)` | Project scope 记忆列表 |

文件读写见 [文件浏览器 API](#文件浏览器-api) 的 `project_fs_*` 命令；会话级工作目录 / agent 切换见 [Session 系统](session.md) 的 `update_session_working_dir` / `update_session_agent`。

### HTTP 路由（[`routes/projects.rs`](../../crates/ha-server/src/routes/projects.rs)）

| 方法 | 路径 | Handler |
|---|---|---|
| `GET` | `/api/projects` | `list_projects` |
| `POST` | `/api/projects` | `create_project` |
| `GET` | `/api/projects/:id` | `get_project` |
| `PATCH` | `/api/projects/:id` | `update_project` |
| `DELETE` | `/api/projects/:id` | `delete_project` |
| `POST` | `/api/projects/:id/archive` | `archive_project` |
| `GET` | `/api/projects/:id/sessions` | `list_project_sessions` |
| `POST` | `/api/projects/:id/read` | `mark_project_sessions_read` |
| `GET` | `/api/projects/:id/memories` | `list_project_memories` |
| `PATCH` | `/api/sessions/:id/project` | `move_session_to_project` |

文件 CRUD 走 `/api/fs/*`（见 [文件浏览器 API](#文件浏览器-api)），不再有 `/api/projects/:id/files*` 路由。详见 [api-reference.md](api-reference.md)。

## 前端 UI

### 侧边栏树状渲染（[`ProjectSection.tsx`](../../src/components/chat/project/ProjectSection.tsx)）

项目是侧边栏一等节点，每个项目渲染为可折叠的 `ProjectGroup`：

- 展开后嵌套该项目下的会话列表（复用 `SessionItem`）；展开状态按单条 `localStorage` 键 `ha:project-expanded`（一条 JSON 存所有项目的展开集，`ProjectSection.tsx` 内联，非 `useTreeExpansion`）持久化
- **每个项目独立分页**（[`useProjectSessions`](../../src/components/chat/project/hooks/useProjectSessions.ts)）：展开时按需调 `list_project_sessions_cmd` 拉自己的会话（**而非**从共享全局会话数组里筛——全局数组只持最近一页，会漏掉项目里较早的会话），默认 `PROJECT_SESSION_PAGE_SIZE`（15）；底部「展开显示 / 折叠显示」按钮增减一页。采用 **window-refetch 模型**（恒 `offset:0`、`limit:windowSize`），分页 ≤15 条对本地 SQLite 成本极低，且免去 append/dedup 竞态。实时刷新复用 ChatScreen 既有机制：以该项目在全局会话数组中切片的指纹（`changeSignal`，含 id/updatedAt/pinnedAt/unread/title/pending）+ `ProjectMeta.session_count` 作为 refetch 触发，**指纹仅作触发、绝不用于渲染**
- Hover「新建对话」+「设置」；右键菜单 新建 / 设置 / 归档
- 主区 `SessionList` 的「对话 / Subagent」浏览 Tab 各自独立分页，并在后端 `LIMIT/OFFSET` 前组合 `ProjectFilter::Unassigned`、顶层/子会话类型和 Agent 过滤，避免最近项目会话占满全局页后把平铺列表截空；共享全局会话数组只作项目树刷新信号。侧边栏搜索不受浏览 Tab 限制，仍全局覆盖项目会话
- 项目名后追加 `working_dir` 摘要

### ProjectDialog（[`ProjectDialog.tsx`](../../src/components/chat/project/ProjectDialog.tsx)）

`mode="create" | "edit"` 复用同一组件，字段：name / description / instructions / emoji / logo（data URL 上传）/ color / defaultAgentId / workingDir；`defaultModelId` 仅为旧数据兼容，不在 UI 暴露且不参与会话解析。保存按钮三态（idle → saving → saved/failed）。编辑态内嵌 [`ProjectKnowledgeSection`](../../src/components/chat/project/ProjectKnowledgeSection.tsx)（项目级知识空间绑定，详见 [knowledge-base.md](knowledge-base.md)）。

### ProjectOverviewDialog（右侧 Sheet，[`ProjectOverviewDialog.tsx`](../../src/components/chat/project/ProjectOverviewDialog.tsx)）

文件名保留，UI 实为右侧 `Sheet`，3 Tab：

| Tab | 作用 |
|---|---|
| **Overview** | 元数据 + 操作 |
| **Files** | [`FileBrowserView`](../../src/components/chat/project/file-browser/)（可编辑文件浏览器：树 + 预览 + 上传 / 删除 / 重命名 / 新建目录） |
| **Instructions** | Textarea 编辑 `instructions` |

> 旧的「Sessions」Tab（会话已在侧边栏树可见）与「绑定 IM Channel」select（反向认领废弃）均已移除。

### 标题栏（`ChatTitleBar`）

- 项目会话前缀渲染**项目 chip**（点击打开设置 Sheet）
- Agent 名换成 [`AgentSwitcher`](../../src/components/chat/AgentSwitcher.tsx) dropdown，**仅 `messages.length === 0`** 时可换（前端 disabled，后端 SQL `message_count == 0` 强制校验）
- [`WorkingDirectoryButton`](../../src/components/chat/input/WorkingDirectoryButton.tsx) 显示生效路径，区分会话级 / 继承自项目

### Hooks

- [`useProjects`](../../src/components/chat/project/hooks/useProjects.ts)：加载 + CRUD 封装 + 订阅 EventBus 事件自动刷新
- [`useProjectFs`](../../src/components/chat/project/hooks/useProjectFs.ts)：文件浏览器状态（list / read / write / 上传 / 删除 / 重命名），订阅 `project:fs_changed`
- [`useFileBrowserSplit`](../../src/components/chat/project/hooks/useFileBrowserSplit.ts)：主聊天区右侧 split 文件面板开合

### i18n

项目翻译在 `project.*` 命名空间。新增 key 当次改动需 12 语齐全（`scripts/sync-i18n.mjs`）。

## EventBus 事件

| 事件名 | payload | 发射时机 |
|---|---|---|
| `project:created` | `{projectId}` | 创建成功后 |
| `project:updated` | `{projectId}` | 更新 / 归档 / `working_dir` patch 成功后 |
| `project:deleted` | `{projectId}` | `delete_project_cascade` 成功后 |
| `project:fs_changed` | `{...}` | 文件浏览器 CRUD 后，跨视图同步 |

前端 [`useProjects`](../../src/components/chat/project/hooks/useProjects.ts) 订阅前 3 个触发 `reloadProjects()`，`useProjectFs` 订阅 `project:fs_changed`。

## 启动顺序

1. `SessionDB::open()` → sessions 表 migration（含 `project_id` 列 + 索引）
2. `ProjectDB::new(session_db)` + `ProjectDB::migrate()` → 建 `projects` 表 + 遗留 drop 迁移
3. 注册全局 `ha_core::globals::PROJECT_DB`
4. `AppState.project_db` / `AppContext.project_db` 持引用
5. `app_init::start_background_tasks` → `project::reconcile::spawn_startup_reconciler()` 异步扫孤儿记忆

## 安全约束

- **工作目录写入校验**：所有写路径走 `util::canonicalize_working_dir`，`canonicalize` + `is_dir` 不通过 `Err`
- **文件浏览器作用域闭合**：`WorkspaceScope` canonicalize + `starts_with`，失败即拒；`for_path` 只读跳转写操作一律拒；HTTP 写端点叠加 `filesystem.allow_remote_writes`（默认 false）闸门
- **preview-by-path 鉴权**：HTTP 三端点共用 `authorized_canonical_file_path`（会话引用 ∪ 工作目录内），主机任意路径 403；桌面信任本机
- **删除前防逃逸**：`purge_project_dir` canonicalize 比对 `projects_root`，拒绝对其外目录 `remove_dir_all`
- **上传上限**：`MAX_PROJECT_FILE_BYTES = 20 MB`，HTTP 层前置校验 + 管道入口 bail 双重把关
- **事务边界**：`ProjectDB::delete` 单 TX 内 unassign + delete；跨 DB 的 memory 删除放 TX 外，失败走 reconciler 兜底
- **logo 校验**（[`db.rs::validate_logo`](../../crates/ha-core/src/project/db.rs)）：长度上限 512KB；必须 `data:image/...;base64,` 前缀，**拒绝任何 http(s):// URL**（避免 SSRF / 第三方追踪）+ 拒绝 `javascript:` / `file:` 等 schema；失败 `bail!` 不静默裁剪

## 关联文档

- [Session 系统](session.md) — `sessions.project_id` 列、`ProjectFilter` 枚举、会话级 working_dir / agent 切换 API
- [知识空间](knowledge-base.md) — 项目级 KB 绑定（`ProjectKnowledgeSection`，`effective_kb_access` 取 `max(session, project)`）
- [文件操作统一](file-operations.md) — 「文件即真实文件」、文件预览面板、preview-by-path 鉴权
- [IM Channel 系统](im-channel.md) — `/project <id>` IM 路由（无反向认领）
- [记忆系统](memory.md) — `MemoryScope::Project`、三级作用域预算
- [提示词系统](prompt-system.md) — `# Current Project` / `# Working Directory` 段装配顺序
- [配置系统](config-system.md) — `AppConfig.default_agent_id` 在 7 级解析链中的位置

## 文件清单

| 文件 | 职责 |
|---|---|
| [`crates/ha-core/src/project/mod.rs`](../../crates/ha-core/src/project/mod.rs) | 模块声明 + re-export |
| [`crates/ha-core/src/project/types.rs`](../../crates/ha-core/src/project/types.rs) | `Project` / `ProjectMeta` + 两个 Input DTO |
| [`crates/ha-core/src/project/db.rs`](../../crates/ha-core/src/project/db.rs) | `ProjectDB`（复用 `SessionDB` 连接）+ migrate + `validate_logo` |
| [`crates/ha-core/src/project/files.rs`](../../crates/ha-core/src/project/files.rs) | `resolve_project_dir` / `delete_project_cascade` / `purge_project_dir` 防逃逸 |
| [`crates/ha-core/src/project/reconcile.rs`](../../crates/ha-core/src/project/reconcile.rs) | 启动期跨 DB 孤儿记忆清理 |
| [`crates/ha-core/src/paths.rs`](../../crates/ha-core/src/paths.rs) | `projects_dir` / `project_dir` / `project_workspace_dir` |
| [`crates/ha-core/src/session/db.rs`](../../crates/ha-core/src/session/db.rs) | `sessions.project_id` 迁移 + `ProjectFilter` + 绑定 API |
| [`crates/ha-core/src/session/helpers.rs`](../../crates/ha-core/src/session/helpers.rs) | `effective_session_working_dir` 合并入口 |
| [`crates/ha-core/src/filesystem/workspace.rs`](../../crates/ha-core/src/filesystem/workspace.rs) | `WorkspaceScope` 作用域闭合（`for_session` / `for_project` / `for_path`） |
| [`crates/ha-core/src/filesystem/ops.rs`](../../crates/ha-core/src/filesystem/ops.rs) | 文件浏览器读写 ops |
| [`crates/ha-core/src/agent/resolver.rs`](../../crates/ha-core/src/agent/resolver.rs) | 7 级 agent 解析链 + `_with_source` 调试入口 |
| [`crates/ha-core/src/util.rs`](../../crates/ha-core/src/util.rs) | `canonicalize_working_dir`（session / project 共用写入校验） |
| [`crates/ha-core/src/slash_commands/handlers/project.rs`](../../crates/ha-core/src/slash_commands/handlers/project.rs) | `/project` / `/projects` handler（`EnterProject` / `AssignProject` / `ShowProjectPicker`） |
| [`src-tauri/src/commands/project.rs`](../../src-tauri/src/commands/project.rs) | Tauri 项目命令 + emit 事件 |
| [`src-tauri/src/commands/project_fs.rs`](../../src-tauri/src/commands/project_fs.rs) | 文件浏览器 Tauri 命令 + preview-by-path |
| [`crates/ha-server/src/routes/projects.rs`](../../crates/ha-server/src/routes/projects.rs) | HTTP 项目 Handler，跨 DB 补齐 memory_count |
| [`crates/ha-server/src/routes/project_fs.rs`](../../crates/ha-server/src/routes/project_fs.rs) | HTTP `/api/fs/*` 文件浏览器路由 |
| [`src/components/chat/project/ProjectSection.tsx`](../../src/components/chat/project/ProjectSection.tsx) | 侧边栏项目树 |
| [`src/components/chat/project/ProjectDialog.tsx`](../../src/components/chat/project/ProjectDialog.tsx) | create / edit 复用对话框（含 KB 绑定段） |
| [`src/components/chat/project/ProjectOverviewDialog.tsx`](../../src/components/chat/project/ProjectOverviewDialog.tsx) | 项目设置 Sheet（Overview / Files / Instructions 三 Tab） |
| [`src/components/chat/project/file-browser/`](../../src/components/chat/project/file-browser/) | 文件浏览器（树 / 预览 / 拖宽） |
| [`src/components/chat/project/hooks/`](../../src/components/chat/project/hooks/) | `useProjects` / `useProjectFs` / `useFileBrowserSplit` / `useTreeExpansion` |
| [`src/components/chat/input/WorkingDirectoryButton.tsx`](../../src/components/chat/input/WorkingDirectoryButton.tsx) | 工作目录按钮（区分会话级 / 继承自项目） |
| [`src/components/chat/AgentSwitcher.tsx`](../../src/components/chat/AgentSwitcher.tsx) | 标题栏 Agent dropdown（messages 非空时 disabled） |
