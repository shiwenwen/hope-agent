# Project 项目系统架构

> 返回 [文档索引](../README.md) | 更新时间：2026-04-28

## 目录

- [概述](#概述)
- [数据模型](#数据模型)
- [SQLite Schema](#sqlite-schema)
- [Migration 反例：bound_channel 启动 panic 修复](#migration-反例bound_channel-启动-panic-修复)
- [磁盘布局](#磁盘布局)
- [核心 API](#核心-api)
- [文件上传管道](#文件上传管道)
- [Bound Channel：项目绑定 IM 渠道](#bound-channel项目绑定-im-渠道)
- [Agent 解析链（5 级）](#agent-解析链5-级)
- [工作目录解析链（session > project）](#工作目录解析链session--project)
- [`/project` 斜杠命令](#project-斜杠命令)
- [System Prompt 三层注入](#system-prompt-三层注入)
- [`project_read_file` 工具](#project_read_file-工具)
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

Project 是 Hope Agent 的**可选会话容器**，将多个会话聚成一个工作空间以共享：

1. **项目记忆**（`MemoryScope::Project { id }`）— 项目内可见，跨项目隔离
2. **项目指令**（`instructions`）— 装配进每个项目内会话的 System Prompt
3. **上传文件** — 三层注入给 LLM（目录清单 / 小文件内联 / on-demand 读取）

`sessions.project_id = NULL` 的会话保留 pre-project 行为，完全不受影响。项目是 opt-in 容器，而不是对话的必需分组。

核心设计取舍：

- **复用 `sessions.db`**：`projects` / `project_files` 表与 `sessions` 表同 DB（`ProjectDB` 持 `Arc<SessionDB>`），SQLite FK CASCADE 自然处理文件元数据
- **跨 DB 内存**：项目记忆在独立的 `memory.db` 中，无法共享 TX；通过启动期 reconciler 兜底孤儿清理
- **防御式路径校验**：`project_read_file` 工具和 `purge_project_files_dir` 都两次 canonicalize 后白名单比对，防符号链接逃逸

## 数据模型

### Project ([types.rs:14-56](../../crates/ha-core/src/project/types.rs))

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `String` | UUID v4 主键 |
| `name` | `String` | 项目名称（trim 后不得为空） |
| `description` | `Option<String>` | 项目简介 |
| `instructions` | `Option<String>` | 自定义指令，追加到项目内每个会话的 System Prompt |
| `emoji` | `Option<String>` | 侧边栏 / 标题前缀 emoji |
| `logo` | `Option<String>` | 项目 logo data URL（`data:image/...;base64,...`），优先于 `emoji` 渲染。详见 [安全约束](#安全约束) |
| `color` | `Option<String>` | 强调色（目前 UI 内部装饰用） |
| `default_agent_id` | `Option<String>` | 新建会话时的默认 Agent |
| `default_model_id` | `Option<String>` | 新建会话时的默认模型 |
| `working_dir` | `Option<String>` | 项目级默认工作目录（绝对路径）；session 未单独设置时回落到此 |
| `bound_channel` | `Option<BoundChannel>` | 项目对一个 IM channel account 的绑定，详见 [Bound Channel](#bound-channel项目绑定-im-渠道) |
| `created_at` / `updated_at` | `i64` | Unix 毫秒时间戳 |
| `archived` | `bool` | 归档标志（不删除，默认列表过滤） |

### BoundChannel ([types.rs:59-65](../../crates/ha-core/src/project/types.rs))

```rust
pub struct BoundChannel {
    pub channel_id: String,   // e.g. "telegram", "wechat", "discord"
    pub account_id: String,   // 该 channel 下的 account 标识
}
```

同一对 (channel_id, account_id) 在所有项目中**最多被一个项目绑定**，由 `idx_projects_bound_channel` 索引和 `create` / `update` 的 conflict 校验共同保证（见下方 SQL）。

### ProjectMeta ([types.rs:40-49](../../crates/ha-core/src/project/types.rs#L40-L49))

`Project` + 聚合计数：`session_count`、`file_count`、`memory_count`。

`session_count` / `file_count` 由 `ProjectDB::list` 的子查询得出；`memory_count` 跨 DB，需调用方在 Tauri / HTTP 层用 `backend.count_by_project(&id)` 补齐（[projects.rs:105-111](../../crates/ha-server/src/routes/projects.rs#L105-L111)）。

### ProjectFile ([types.rs:100-128](../../crates/ha-core/src/project/types.rs#L100-L128))

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `String` | UUID v4 |
| `project_id` | `String` | 所属项目 FK |
| `name` | `String` | 用户可编辑的显示名 |
| `original_filename` | `String` | 上传时原始文件名 |
| `mime_type` | `Option<String>` | MIME 类型 |
| `size_bytes` | `i64` | 字节数 |
| `file_path` | `String` | 相对 `projects_dir()` 的原文件路径 |
| `extracted_path` | `Option<String>` | 相对 `projects_dir()` 的提取文本路径（二进制 / 提取失败为 `None`） |
| `extracted_chars` | `Option<i64>` | 提取文本的字符数，内联预算决策用 |
| `summary` | `Option<String>` | 预留的 LLM 一句话摘要（当前未使用） |

### 输入 DTO

- `CreateProjectInput`：`name` 必填，其余可选；包含 `logo` / `working_dir` / `bound_channel` 等所有可写字段
- `UpdateProjectInput`：PATCH 语义。绝大多数字段是 `Option<_>`（None=不变，Some(空串)=清空），**唯独 `bound_channel` 是 double-Option**：
  ```rust
  pub bound_channel: Option<Option<BoundChannel>>,  // 自定义 deserialize_bound_channel_patch
  ```
  - `None`（字段缺省）→ 不改
  - `Some(None)`（JSON `null`）→ 清空绑定
  - `Some(Some(_))`（JSON 对象）→ 设置绑定
  
  普通 serde 默认会把"字段缺省"和"字段为 null"都解成 `None`，无法区分；`deserialize_bound_channel_patch` 强制把"字段出现"的两种值都包成 `Some(...)` 来恢复 patch 语义（[types.rs:144-151](../../crates/ha-core/src/project/types.rs#L144)）。AGENTS.md 把这条单独点出来强调，因为是该模块少见的 patch 语义陷阱
- `working_dir` 在 update 路径走 `crate::util::canonicalize_working_dir`（空串当清空，否则 canonicalize + is_dir 校验），不通过直接 `Err`

## SQLite Schema

两张表随 `SessionDB` 的连接共享，由 `ProjectDB::migrate()` 幂等建表（[db.rs:27-71](../../crates/ha-core/src/project/db.rs#L27-L71)）。

```sql
CREATE TABLE IF NOT EXISTS projects (
    id                          TEXT PRIMARY KEY,
    name                        TEXT NOT NULL,
    description                 TEXT,
    instructions                TEXT,
    emoji                       TEXT,
    logo                        TEXT,                       -- data URL
    color                       TEXT,
    default_agent_id            TEXT,
    default_model_id            TEXT,
    working_dir                 TEXT,                       -- 项目级默认工作目录
    bound_channel_id            TEXT,                       -- IM channel 绑定（成对存在）
    bound_channel_account_id    TEXT,
    created_at                  INTEGER NOT NULL,
    updated_at                  INTEGER NOT NULL,
    archived                    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_projects_archived
    ON projects(archived, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_projects_bound_channel
    ON projects(bound_channel_id, bound_channel_account_id);

CREATE TABLE IF NOT EXISTS project_files (
    id                 TEXT PRIMARY KEY,
    project_id         TEXT NOT NULL,
    name               TEXT NOT NULL,
    original_filename  TEXT NOT NULL,
    mime_type          TEXT,
    size_bytes         INTEGER NOT NULL,
    file_path          TEXT NOT NULL,
    extracted_path     TEXT,
    extracted_chars    INTEGER,
    summary            TEXT,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_project_files_project
    ON project_files(project_id);
```

**sessions 表扩展**（[session/db.rs:232-239](../../crates/ha-core/src/session/db.rs)）：`SessionDB::open` 迁移阶段 `ALTER TABLE sessions ADD COLUMN project_id TEXT` + 建 `idx_sessions_project_id` 索引，老库零破坏升级。

## Migration 反例：bound_channel 启动 panic 修复

> commit `223943cb fix: prevent startup panic for installs upgrading to bound_channel schema`

bound_channel 列引入时第一版 migration 把 `ALTER TABLE` 和 `CREATE INDEX` 放在同一个 `execute_batch` 里。SQLite 在准备 batch 时会**同时**编译所有语句的执行计划——`CREATE INDEX … ON projects(bound_channel_id, ...)` 在 ALTER 还没生效的瞬态视图里看不到这两个新列，prepared plan 失败 → 启动 panic。

修正后的 contract（[db.rs:90-110](../../crates/ha-core/src/project/db.rs)）：

```rust
// ① 先单独 ALTER（如果列不存在）
if !has_bound_channel {
    conn.execute_batch(
        "ALTER TABLE projects ADD COLUMN bound_channel_id TEXT;
         ALTER TABLE projects ADD COLUMN bound_channel_account_id TEXT;",
    )?;
}

// ② 列已确保存在后，单独 CREATE INDEX
conn.execute_batch(
    "CREATE INDEX IF NOT EXISTS idx_projects_bound_channel
         ON projects(bound_channel_id, bound_channel_account_id);",
)?;
```

通用规则：**SQLite migration 中，`CREATE INDEX` / 任何引用某列的 DDL 不能与添加该列的 `ALTER TABLE` 共用一个 batch**。回归测试入口 [`migrate_pre_bound_channel_schema`](../../crates/ha-core/src/project/db.rs#L777) 模拟旧库升级路径，新增涉及 schema 的列时建议参照写一份反向场景测试。

## 磁盘布局

```
~/.hope-agent/
├── sessions.db                        # projects + project_files + sessions 同一个 DB
├── memory.db                          # 项目记忆（独立 DB，MemoryScope::Project）
└── projects/
    └── {project_id}/
        ├── files/{uuid8}_{safe_name}  # 原始字节
        └── extracted/{uuid}.txt       # 提取文本（仅文本型文件）
```

路径由 [`paths.rs:244-261`](../../crates/ha-core/src/paths.rs#L244-L261) 集中管理：`projects_dir()` / `project_dir(id)` / `project_files_dir(id)` / `project_extracted_dir(id)`。

## 核心 API

### ProjectDB ([db.rs](../../crates/ha-core/src/project/db.rs))

**项目 CRUD：**

| 方法 | 说明 |
|---|---|
| `create(CreateProjectInput)` | 插入新项目，返回 `Project`；`bound_channel` 非 None 时先查冲突，已被其他项目占用直接 `bail!`；`logo` 走 `validate_logo`（见安全约束），`working_dir` 走 `canonicalize_working_dir` |
| `get(id)` | 取单个项目 |
| `update(id, UpdateProjectInput)` | 动态 SQL 部分更新；普通字段空串 → NULL；`bound_channel` 走 double-Option patch；强制 bump `updated_at` |
| `delete(id)` → `Vec<ProjectFile>` | `IMMEDIATE` 事务内：① `SELECT` 快照文件行作为返回值 ② `UPDATE sessions SET project_id = NULL` ③ `DELETE FROM projects`（FK CASCADE 顺带删 `project_files`）。返回的文件列表供调用者清理磁盘 |
| `list_all_ids()` | 轻量级 id 列表，reconciler 专用 |
| `list(include_archived)` → `Vec<ProjectMeta>` | 带 `session_count` / `file_count` 聚合子查询；`memory_count = 0` 待调用方补齐 |
| `find_by_bound_channel(channel_id, account_id)` → `Option<Project>` | **channel worker 路由必经入口**：每条入站消息进 `resolve_or_create_session` 时先反查项目，命中则把 `project_id` 注入新 session，agent_id 走 5 级解析链。源：[db.rs:508](../../crates/ha-core/src/project/db.rs#L508) |

**文件 CRUD：**

| 方法 | 说明 |
|---|---|
| `add_file(&ProjectFile)` | 插入元数据行 |
| `list_files(project_id)` | 按 `created_at DESC` 返回 |
| `get_file(project_id, file_id)` | 精确定位 |
| `find_file_by_name(project_id, name)` | `project_read_file` 工具的 fallback 入口 |
| `rename_file(file_id, new_name)` | 只改 `name`，不动磁盘路径 |
| `delete_file(file_id)` → `Option<ProjectFile>` | 返回删除前的行以供磁盘清理 |

### session ↔ project 绑定（[session/db.rs:642-724](../../crates/ha-core/src/session/db.rs#L642-L724)）

| 方法 | 说明 |
|---|---|
| `create_session_with_project(agent_id, project_id: Option<&str>)` | 带项目归属创建会话 |
| `set_session_project(session_id, project_id: Option<&str>)` | 搬迁会话到另一个项目或 unassign |
| `clear_project_from_sessions(project_id)` | 批量 unassign，由 `ProjectDB::delete` 内部使用 |
| `list_sessions_paged(agent_id, project_filter, limit, offset)` | 新增 `ProjectFilter` 参数：`All` / `Unassigned` / `InProject(id)` |

## 文件上传管道

`upload_project_file` ([files.rs:39-138](../../crates/ha-core/src/project/files.rs#L39-L138)) 执行 8 步，任一步失败都通过 `scopeguard` 清理已写入的字节，避免孤儿文件：

1. **大小 / 名称校验** — 大小 ≤ `MAX_PROJECT_FILE_BYTES = 20 MB`（[files.rs:17](../../crates/ha-core/src/project/files.rs#L17)），非空
2. **项目存在性检查** — 防止写入悬空项目
3. **建目录** — `project_files_dir` / `project_extracted_dir` 幂等 `create_dir_all`
4. **生成安全名** — `uuid` 前 8 位 + `_` + `sanitize_filename(原名)`
5. **写字节** — 挂上 `scopeguard` 失败时删文件
6. **文本提取** — 调 `file_extract::extract(path, filename, mime)`。提取成功写 `extracted/{uuid}.txt`，记录 `extracted_chars`；失败（二进制 / 不支持格式）`extracted_path = None`，**非致命**
7. **插行** — `ProjectDB::add_file`；失败时手动删 extracted 侧边文件，让 guard drop 删原文件
8. **解除 guard** — 成功则保留磁盘字节

**异步边界**：上传管道内部全同步（`file_extract::extract` 对大文件 I/O 密集），Tauri 命令（[commands/project.rs:168-183](../../src-tauri/src/commands/project.rs#L168-L183)）和 HTTP 路由（[projects.rs:248-260](../../crates/ha-server/src/routes/projects.rs#L248-L260)）均用 `tokio::task::spawn_blocking` 包裹，避免阻塞 tokio runtime。

## Bound Channel：项目绑定 IM 渠道

`Project.bound_channel` 让一个项目"认领"一对 (channel_id, account_id)：之后这个 IM 账户产生的所有新会话被自动归到该项目下，agent_id 也按项目偏好选择。

### 路由流程（channel worker 收到入站消息）

```
ChannelWorker (channel/db.rs::ensure_conversation/resolve_or_create_session)
   ├── ① find_by_bound_channel(channel_id, account_id)
   │      └── Some(Project) → project_id 注入新 session；agent_id 解析链命中"项目级 default_agent"
   │      └── None         → project_id 留空；agent_id 解析链跳过项目级
   ├── ② create_session_with_project(...)
   │      └── 入口防御：project_id 存在时强制 incognito = false
   └── ③ 后续消息 / 工具审批走标准流程，与桌面端无差异
```

### 唯一性约束

数据库索引 `idx_projects_bound_channel(bound_channel_id, bound_channel_account_id)` + `create` / `update` 内 conflict 校验双层保证：

```rust
// pseudo-code
let conflict: Option<String> = conn.query_row(
    "SELECT id FROM projects
     WHERE bound_channel_id = ?1 AND bound_channel_account_id = ?2
       AND id != ?3 LIMIT 1",
    ...,
)?;
if let Some(other) = conflict {
    bail!("channel binding already claimed by project {}", other);
}
```

→ 用户 UI 上尝试把已被项目 A 认领的 channel 绑给项目 B 时，前端拿到清晰的冲突错误，不会出现"两个项目同时认领同一个渠道"的不一致状态。

### 与无痕 / 项目互斥的强化

- `update_session_incognito` 对 `project_id.is_some()` 或 `channel_info.is_some()` 直接 `Err`
- `create_session_with_project` 在 `project_id` 存在时强制 `incognito = false`
- `channel/db.rs::ensure_conversation` 入口对 IM 路径会话强制 `incognito = false`（防御式清零）

## Agent 解析链（5 级）

新会话 `agent_id` 解析顺序统一为 5 级，由 [`crate::agent::resolver::resolve_default_agent_id`](../../crates/ha-core/src/agent/resolver.rs) 实现（带来源 tag 的 `_with_source` 版本供 `/status` 显示链路命中位置）：

| 优先级 | 来源 | 触发条件 |
|---|---|---|
| 1 | **显式参数** | 调用方在 API / Tauri 命令里直接传 `agent_id` |
| 2 | **`project.default_agent_id`** | session 落入项目（含 IM bound_channel 命中），项目设置了默认 Agent |
| 3 | **`channel_account.agent_id`** | IM channel account 配了 per-account agent override |
| 4 | **`AppConfig.default_agent_id`** | 全局设置，默认 `Some("default")` |
| 5 | **硬编码 `"default"`** | 兜底常量 |

### 配套 API

| 入口 | 作用 |
|---|---|
| Tauri `get_default_agent_id` / `set_default_agent_id` | 读 / 写 `AppConfig.default_agent_id` |
| HTTP `GET / PUT /api/config/default-agent` | 同上 |
| `oc-settings` 工具 `category="default_agent"` | 模型可改（LOW 风险，AGENTS.md 已登记） |
| `/status` 斜杠命令 | 在项目会话里追加项目摘要段，标注 Agent Source 命中级别 |

## 工作目录解析链（session > project）

会话最终的"工作目录"由 [`session::helpers::effective_session_working_dir`](../../crates/ha-core/src/session/helpers.rs#L79) 单一入口解析：

```
session.working_dir 非空？ → 用之（会话级）
否则 session.project_id Some？ → 取 project.working_dir（继承自项目）
否则 → 不注入 # Working Directory 段
```

**Lazy resolve（不复制快照）**：项目改 `working_dir` 立即对未单独设置的项目内已有会话生效。

### 写入校验入口

[`crate::util::canonicalize_working_dir`](../../crates/ha-core/src/util.rs)（session / project 共用）：
- 空串当清空（写 NULL）
- 非空 → `canonicalize` + `is_dir` 校验，不通过返回 `Err`

### 系统提示注入位置

`system_prompt::build` 装配顺序：
```
... → # Current Project（含 instructions） → # Project Files
                       ↓
              # Working Directory（注入 effective working dir）
                       ↓
              # Memory → ...
```

`# Working Directory` 段位置在 Project 段之后、Memory 段之前。该值同时被工具执行 context 消费（`exec` 默认 cwd、`read` / `write` 相对路径解析、`write_file` 路径白名单），保证模型视图与工具运行时不会偏离。

### UI 区分两种来源

`ChatTitleBar` 与 `WorkingDirectoryButton` 都显示生效路径，并区分：

- **会话级**（`session.working_dir` 非空）：显示路径 + clear 按钮
- **继承自项目**（`session.working_dir` 空 + 走 `project.working_dir`）：显示路径 + 标注"继承自项目"，**不渲染 clear 按钮**（避免 no-op 误操作）

### v1 范围

当前只做**提示词注入**，不改 `exec` / `read_file` 等工具内部 `cwd` 解析逻辑——工具仍以"绝对路径优先 + ctx 提供的 session_working_dir"为准。这与 prompt 注入一致，只是侧重点不同。

## `/project` 斜杠命令

在桌面 / HTTP 通道，用户可输入 `/project [name]` 切换项目：

| 形式 | 行为 |
|---|---|
| `/project`（无参） | 返回 `CommandAction::ShowProjectPicker`，前端渲染为 markdown event 消息 + 项目选择器 |
| `/project <name>` | fuzzy 匹配项目名 → 命中则 `EnterProject` action → 前端 `handleNewChatInProject(project_id)` 创建项目作用域新会话 |

### 在 IM channel 中禁用

与 `/agent` 同类禁用（commit `48fa4986` + `0fe6ec0a`）：

- `slash_commands/registry.rs` 常量 `IM_DISABLED_COMMANDS = &["project", "agent"]`
- 同步阶段过滤：Discord / Telegram 命令同步过程中跳过这两个命令的注册
- handler 内自检兜底：`session.channel_info.is_some()` 时直接拒绝（不下发 IM 用户）

**禁用原因**：IM 渠道 session 的 `project_id` / `agent_id` 由 channel-account / topic / group 在每条入站消息里**重新计算**——`/project` / `/agent` 切换会被立刻拉回，是无意义的"幻觉切换"，禁用比留 broken UX 好。

源：[handlers/project.rs](../../crates/ha-core/src/slash_commands/handlers/project.rs)、[handlers/agent.rs](../../crates/ha-core/src/slash_commands/handlers/agent.rs)。

## System Prompt 三层注入

会话挂到项目后，`system_prompt::build` 在 Memory 段之前注入 `#Current Project` 和 `# Project Files`（[build.rs:244-264](../../crates/ha-core/src/system_prompt/build.rs#L244-L264)）。

**Layer 1 — 目录清单**（总是注入，成本 ~100 bytes/文件）

来自 `build_project_files_section` ([sections.rs:497-510](../../crates/ha-core/src/system_prompt/sections.rs#L497-L510))：每个文件一行，包含 emoji 图标（按 MIME 类型分类）、文件名、大小 KB、提取字符数或"binary"标记、`file_id`。

**Layer 2 — 小文件内联**（预算 8KB，单文件上限 4096 字符）

循环 `project_files`，跳过二进制和 > 4096 字符的文件，累加字节数不超出 `DEFAULT_PROJECT_FILES_INLINE_BUDGET = 8 * 1024`（[build.rs:16](../../crates/ha-core/src/system_prompt/build.rs#L16)），命中的读盘内联进 `## Inlined Small Files` 代码块。

**Layer 3 — on-demand 读取**

LLM 看到目录但没被内联的文件时，调 `project_read_file(file_id, offset?, limit?)` 按需拉取。

**openclaw_mode 互斥**：openclaw 模式（AGENTS.md / SOUL.md / IDENTITY.md / TOOLS.md 四文件 prompt pack）自带 `# Project Context` 段，跳过此注入避免双重 heading。

**项目指令** 同段注入：`# Current Project` → `Description` → `## Project Instructions`（truncate 到 `MAX_FILE_CHARS`），并尾随一句"本会话 `save_memory` 默认为 project scope"的提示（[sections.rs:463-469](../../crates/ha-core/src/system_prompt/sections.rs#L463-L469)）。

## project_read_file 工具

内置工具定义 ([core_tools.rs:131-160](../../crates/ha-core/src/tools/definitions/core_tools.rs#L131-L160))：

- `internal: true` — UI 隐藏，不可关闭
- `deferred: false, always_load: false` — 非延迟加载，随 Layer 1 catalog 才有意义
- 参数：`file_id` / `name`（二选一）+ `offset`（1-based，默认 1）+ `limit`（默认 2000，上限 10000）

执行逻辑 ([tools/project_read_file.rs](../../crates/ha-core/src/tools/project_read_file.rs))：

1. 从 `ctx.session_id` 反查 session → `project_id`，非项目会话返回"use standard `read` tool"
2. 先按 `file_id` 精确查，fallback 到 `find_file_by_name`
3. 拒绝无 `extracted_path` 的二进制文件
4. **双层路径白名单校验（失败闭合）**：`project_extracted_dir(project_id).canonicalize()` 与 `full_path.canonicalize()` 比对 `starts_with`，任一 canonicalize 失败都拒绝读取，不 fallback 到原始路径
5. 复用 [`read.rs::read_text_page`](../../crates/ha-core/src/tools/read.rs) 做行级分页，输出与 `read` 工具一致

## 记忆系统接入

**MemoryScope 第三变种** ([memory/types.rs:49-61](../../crates/ha-core/src/memory/types.rs#L49-L61))：

```rust
pub enum MemoryScope {
    Global,
    Agent { id: String },
    Project { id: String },  // 仅项目内共享
}
```

**注入优先级**（[sqlite/trait_impl.rs:446-478](../../crates/ha-core/src/memory/sqlite/trait_impl.rs#L446-L478)）：

```
Project（最高）→ Agent → Global（最低，若 shared=true）
```

`load_prompt_candidates_with_project(agent_id, project_id, shared)` 按此顺序拼接候选集。Memory Budget 裁剪时越靠前越不容易被丢弃，确保项目上下文优先保留。

**自动提取作用域**（[memory_extract.rs:20-31](../../crates/ha-core/src/memory_extract.rs#L20-L31)）：

```rust
fn resolve_extract_scope(session_id, agent_id) -> MemoryScope {
    // 读 session → 若 session.project_id Some(pid) → Project { id: pid }
    // 否则 → Agent { id: agent_id }
}
```

用户在项目内会话调 `save_memory` 不传 scope 时默认写 `Project`；可显式传 `scope='global'` 或 `scope='agent'` 打破项目边界。

**计数跨 DB**：`ProjectDB::list` 将 `memory_count` 置 0，由调用方（Tauri `list_projects_cmd` / HTTP `list_projects`）遍历 `backend.count_by_project(&id)` 补齐。

## 级联删除与孤儿清理

### delete_project_cascade 四步 ([files.rs:218-248](../../crates/ha-core/src/project/files.rs#L218-L248))

```
1. session.db IMMEDIATE TX（ProjectDB::delete）：
   ① SELECT 快照 project_files 行 → 作为返回值给调用者用于磁盘清理
   ② UPDATE sessions SET project_id = NULL WHERE project_id = ?   (会话本体保留)
   ③ DELETE FROM projects WHERE id = ?
      └─ FK ON DELETE CASCADE 自动删 project_files               (同 TX 原子)
2. 磁盘：purge_project_files_dir(id) — remove_dir_all 带路径逃逸防护
3. memory.db（独立 DB）：list(Project scope, limit=10_000) → delete_batch(ids)
```

**步骤 2 和 3 在事务外**，因为跨文件系统 / 跨 DB 无法共享 TX。设计取舍：

- 如果第 1 步完成后崩溃 → 孤儿 = `projects/{id}/` 目录 + `memory.db` 中 `scope_project_id = id` 的记忆行
- 孤儿目录 **对应用无害**（id 已不存在，永远不会被访问）
- 孤儿记忆行 **对应用无害**（MemoryScope::Project { id } 也永远不会被 `list` 查出）
- 靠启动期 reconciler 懒清理，而不是同步事务（来源：[reconcile.rs:1-16](../../crates/ha-core/src/project/reconcile.rs#L1-L16) 注释）

### Startup Reconciler ([reconcile.rs](../../crates/ha-core/src/project/reconcile.rs))

`spawn_startup_reconciler()` 在 `app_init::start_background_tasks` 调 `tokio::task::spawn_blocking` 一次性执行，失败只 `app_warn!` 绝不阻塞启动：

1. `project_db.list_all_ids()` → `HashSet<String> alive`
2. `backend.list_distinct_project_scope_ids()` → `referenced`
3. 差集 `referenced \ alive` = 孤儿 id 列表
4. 对每个孤儿 `list(Project scope, 10_000)` → `delete_batch(ids)`
5. 成功 → `app_info!` 日志 `"Reaped N orphan project-scoped memory rows across K dead projects"`

项目删除频率低，没引入周期性 timer，重启时一次扫描就够。

### purge_project_files_dir 防逃逸 ([files.rs:164-208](../../crates/ha-core/src/project/files.rs#L164-L208))

- canonicalize `dir` + canonicalize `projects_root`
- `starts_with(canonical_root)` 不成立 → `app_error!` 拒绝 `remove_dir_all`
- 防御对象：符号链接越界 / 遍历式 project id（虽然 id 来自 `Uuid::new_v4()` 不会构造 `..`）

## 接入层

### Tauri 命令 ([src-tauri/src/commands/project.rs](../../src-tauri/src/commands/project.rs))

注册在 [`src-tauri/src/lib.rs:350-364`](../../src-tauri/src/lib.rs) `invoke_handler!`：

| 命令 | 作用 |
|---|---|
| `list_projects_cmd(include_archived?)` | 列表 + 跨 DB 补齐 memory_count |
| `get_project_cmd(id)` | 取单个 |
| `create_project_cmd(input)` | emit `project:created` |
| `update_project_cmd(id, patch)` | emit `project:updated` |
| `delete_project_cmd(id)` | 走 `delete_project_cascade`，emit `project:deleted` |
| `archive_project_cmd(id, archived)` | 等价于 patch `{archived}`，emit `project:updated` |
| `list_project_sessions_cmd(id, limit?, offset?)` | 基于 `ProjectFilter::InProject`，含 `enrich_pending_interactions` |
| `move_session_to_project_cmd(session_id, project_id?)` | project_id=None 即 unassign |
| `list_project_files_cmd(project_id)` | 按 created_at DESC（commit 014477e0 把参数对齐了 sibling 文件命令） |
| `upload_project_file_cmd(project_id, file_name, mime_type?, data)` | `spawn_blocking`，emit `project:file_uploaded` |
| `delete_project_file_cmd(project_id, file_id)` | `spawn_blocking`，emit `project:file_deleted` |
| `rename_project_file_cmd(project_id, file_id, name)` | 只改显示名 |
| `read_project_file_content_cmd(project_id, file_id, offset?, limit?)` | UI 预览 extracted 文本 |
| `list_project_memories_cmd(id, limit?, offset?)` | Project scope 记忆列表 |
| `update_session_working_dir_cmd(session_id, working_dir?)` | 设置/清空会话级工作目录，走 `canonicalize_working_dir` |
| `update_session_agent_cmd(session_id, agent_id)` | 切换会话 Agent；SQL 层 `message_count == 0` 强制校验 |
| `get_default_agent_id` / `set_default_agent_id` | 读 / 写 `AppConfig.default_agent_id`（5 级解析链 4 级源） |

### HTTP 路由 ([crates/ha-server/src/routes/projects.rs](../../crates/ha-server/src/routes/projects.rs))

在 `ha-server::lib` [`router`](../../crates/ha-server/src/lib.rs) 注册：

| 方法 | 路径 | Handler |
|---|---|---|
| `GET` | `/api/projects` | `list_projects` |
| `POST` | `/api/projects` | `create_project`（body: `{input: CreateProjectInput}`） |
| `GET` | `/api/projects/:id` | `get_project` |
| `PATCH` | `/api/projects/:id` | `update_project`（body: `{patch: UpdateProjectInput}`） |
| `DELETE` | `/api/projects/:id` | `delete_project` |
| `POST` | `/api/projects/:id/archive` | `archive_project`（body: `{archived: bool}`） |
| `GET` | `/api/projects/:id/sessions` | `list_project_sessions` |
| `PATCH` | `/api/sessions/:id/project` | `move_session_to_project`（body: `{projectId?: string}`） |
| `GET` | `/api/projects/:id/files` | `list_project_files` |
| `POST` | `/api/projects/:id/files` | `upload_project_file_route`（multipart: file / fileName / mimeType） |
| `DELETE` | `/api/projects/:id/files/:fid` | `delete_project_file_route` |
| `PATCH` | `/api/projects/:id/files/:fid` | `rename_project_file_route` |
| `GET` | `/api/projects/:id/files/:fid/content` | `read_project_file_content`（offset/limit 行分页） |
| `GET` | `/api/projects/:id/memories` | `list_project_memories` |
| `PATCH` | `/api/sessions/:id/working-dir` | `update_session_working_dir`（body: `{workingDir?: string \| null}`） |
| `PATCH` | `/api/sessions/:id/agent` | `update_session_agent`（body: `{agentId: string}`，message_count!=0 时 400） |
| `GET` | `/api/filesystem/list-dir` | `list_dir`（Bearer + query params；`ServerDirectoryBrowser` Dialog 在 HTTP 模式下选目录用） |
| `GET` | `/api/config/default-agent` | 读全局 `AppConfig.default_agent_id` |
| `PUT` | `/api/config/default-agent` | 写全局默认 agent |

上传复用 `routes::helpers::parse_file_upload` 取 multipart 字段，前置校验 `MAX_PROJECT_FILE_BYTES` 让 oversize 在触盘前得到清晰错误。

## 前端 UI

### 侧边栏树状渲染（commit 0fe6ec0a）

项目升级为侧边栏一等节点，每个项目渲染为可折叠的 `ProjectGroup`：

- 每个 `ProjectGroup` 展开后嵌套该项目下的会话列表（复用 `SessionItem`）
- 展开/折叠状态按 `localStorage` 的 `ha:project-expanded:<id>` 持久化
- Hover 显示 **「新建对话」+「设置」** 两个按钮（复用 `AgentSection.tsx` 的 `group/agent` hover 模式）
- 右键菜单：新建对话 / 设置 / 归档（复用 `ContextMenu` 模式）
- 主区域 `SessionList` 自动**排除 `projectId` 非空**的会话，避免与树状项目下的会话重复展示
- 项目名后追加 `working_dir` 摘要（commit 8f94b4d1，把项目工作目录显示在项目名后面）

### ProjectDialog ([ProjectDialog.tsx](../../src/components/chat/project/ProjectDialog.tsx))

`mode="create" | "edit"` 复用同一组件：

- 空白态 → `onCreate(CreateProjectInput)`
- 预填态 → `onUpdate(UpdateProjectInput)`
- 字段：name / description / instructions / emoji / **logo（data URL 上传）** / color / defaultAgentId / defaultModelId / **workingDir** / **bound_channel select**
- 保存按钮三态（idle → saving → saved/failed），对齐 AGENTS.md UI 约定

### ProjectOverviewDialog → 右侧 Sheet 重构

文件名保留 `ProjectOverviewDialog.tsx`（避免大量 import 改动），但 UI 实际是右侧 `Sheet`（来自 `src/components/ui/sheet.tsx`）。**Sessions tab 已删除**（项目内会话已在侧边栏树状视图直接可见，再开 tab 是冗余），当前 3 Tab：

| Tab | 作用 |
|---|---|
| **Overview** | 元数据 + 4 操作 + 内置「绑定 IM Channel」select |
| **Files** | `ProjectFilesPanel`（拖拽 / 点击上传，20MB、删除、重命名） |
| **Instructions** | Textarea 编辑 `instructions` |

### 标题栏（`ChatTitleBar`）

- 项目会话前缀渲染**项目 chip**（点击打开设置 sheet）
- Agent 名换成 [`AgentSwitcher`](../../src/components/chat/AgentSwitcher.tsx) dropdown，**仅 `messages.length === 0`** 时可换（前端 disabled，后端 SQL message_count==0 强制校验）
- [`WorkingDirectoryButton`](../../src/components/chat/WorkingDirectoryButton.tsx) 显示生效路径，区分会话级 / 继承自项目（继承态不渲染 clear 按钮）

### Hooks

- [`useProjects`](../../src/components/chat/project/hooks/useProjects.ts)：加载 + CRUD 封装 + 订阅五个 EventBus 事件自动刷新
- [`useProjectFiles`](../../src/components/chat/project/hooks/useProjectFiles.ts)：按 project_id 加载，订阅 `project:file_uploaded` / `project:file_deleted`

### i18n

项目相关翻译在 `src/i18n/locales/{zh,en}.json` 的 `project.*` 命名空间，覆盖按钮、表单、Tab 标题、确认文案（例如 `deleteConfirm.body` 明示"sessions 变 unassigned，不会被删除；项目记忆与文件永久删除"）。按 AGENTS.md 约定新增 key 只需 zh+en，其余 11 种语言由 `scripts/sync-i18n.mjs --apply` 补齐。

## EventBus 事件

所有事件 payload 均为 `{projectId: string}`，文件事件额外含 `fileId`：

| 事件名 | 发射时机 | 发射点 |
|---|---|---|
| `project:created` | 项目创建成功后 | Tauri `create_project_cmd` / HTTP `create_project` |
| `project:updated` | 更新 / 归档 / `bound_channel` patch / `working_dir` patch 成功后 | `update_project_cmd` / `archive_project_cmd` / 对应 HTTP handler |
| `project:deleted` | `delete_project_cascade` 返回 true 后 | `delete_project_cmd` / `delete_project` |
| `project:file_uploaded` | 文件插行成功后 | `upload_project_file_cmd` / `upload_project_file_route` |
| `project:file_deleted` | `delete_project_file` 返回 true 后 | `delete_project_file_cmd` / `delete_project_file_route` |

前端 [`useProjects`](../../src/components/chat/project/hooks/useProjects.ts#L73-L77) 统一订阅前 5 个事件触发 `reloadProjects()`，实现跨窗口 / 跨 transport 的实时刷新。

## 启动顺序

1. `SessionDB::open()` → 执行 sessions 表 migration（含 `project_id` 列 + 索引）
2. `ProjectDB::new(session_db)` + `ProjectDB::migrate()` → 建 `projects` / `project_files` 表
3. 注册全局：`ha_core::globals::PROJECT_DB.set(Arc::new(project_db))`
4. `AppState.project_db` / `AppContext.project_db` 分别持引用
5. `app_init::start_background_tasks` → `project::reconcile::spawn_startup_reconciler()` 异步扫孤儿

## 安全约束

- **路径白名单**：`project_read_file` 执行前两次 canonicalize，允许根 = `project_extracted_dir(id).canonicalize()`，**失败闭合**（绝不 fallback 原路径）
- **删除前防逃逸**：`purge_project_files_dir` 同样 canonicalize 比对，拒绝对 `projects_root` 之外的目录 `remove_dir_all`
- **大小硬上限**：`MAX_PROJECT_FILE_BYTES = 20 MB`，在 HTTP 层前置校验 + 管道入口 bail 双重把关（Tauri 命令无前置检查，依赖管道兜底）
- **空上传拒绝**：`data.is_empty()` 或 `original_filename.trim().is_empty()` 立即 bail
- **安全文件名**：`sanitize_filename` 剥离路径分隔符和控制字符，落盘名前缀 uuid 8 位避冲突
- **事务边界**：`ProjectDB::delete` 在单 `IMMEDIATE` TX 内 snapshot → unassign → delete；跨 DB 的 memory 删除放 TX 外，失败走 reconciler 兜底而非回滚 session 侧
- **logo 校验（[`validate_logo`](../../crates/ha-core/src/project/db.rs#L715)）**：
  - 长度上限 `MAX_LOGO_BYTES = 512 * 1024`（512KB）
  - 必须以 `data:image/...;base64,` 前缀开头，**拒绝任何 http(s):// URL**（避免 SSRF / 第三方追踪）
  - 拒绝其他 schema（避免 `javascript:` / `file:` 等）
  - 失败直接 `bail!`，不静默裁剪
- **bound_channel 唯一性**：DB 索引 + create/update conflict 校验双层把关；同一对 (channel_id, account_id) 至多一个项目认领
- **working_dir 写入校验**：所有写路径走 `crate::util::canonicalize_working_dir`，`canonicalize` + `is_dir` 不通过 `Err`

## 关联文档

- [Session 系统](session.md) — `sessions.project_id` 列、`ProjectFilter` 枚举、会话级 working_dir / agent 切换 API
- [IM Channel 系统](im-channel.md) — `bound_channel` 路由 + `/agent` IM 禁用 + `IM_DISABLED_COMMANDS` 双层防御
- [斜杠命令](slash-commands.md) — `/project` 命令在 IM 渠道禁用与原因
- [记忆系统](memory.md) — `MemoryScope::Project`、三级作用域预算、`scope_project_id` 索引
- [提示词系统](prompt-system.md) — System Prompt 装配顺序，`# Working Directory` 段位置
- [工具系统](tool-system.md) — `project_read_file` 工具注册、权限校验层级
- [配置系统](config-system.md) — `AppConfig.default_agent_id` 在 5 级 agent 解析链中的位置

## 文件清单

| 文件 | 职责 |
|---|---|
| [`crates/ha-core/src/project/mod.rs`](../../crates/ha-core/src/project/mod.rs) | 模块声明 + re-export（`ProjectDB` / `MAX_PROJECT_FILE_BYTES` 等） |
| [`crates/ha-core/src/project/types.rs`](../../crates/ha-core/src/project/types.rs) | `Project` / `ProjectMeta` / `ProjectFile` + 两个 Input DTO |
| [`crates/ha-core/src/project/db.rs`](../../crates/ha-core/src/project/db.rs) | `ProjectDB` 主实现，复用 `SessionDB` 连接 |
| [`crates/ha-core/src/project/files.rs`](../../crates/ha-core/src/project/files.rs) | 上传管道、删除、`delete_project_cascade`、目录 purge 防逃逸 |
| [`crates/ha-core/src/project/reconcile.rs`](../../crates/ha-core/src/project/reconcile.rs) | 启动期跨 DB 孤儿记忆清理 |
| [`crates/ha-core/src/paths.rs`](../../crates/ha-core/src/paths.rs#L244-L261) | `projects_dir` / `project_dir` / `project_files_dir` / `project_extracted_dir` |
| [`crates/ha-core/src/session/db.rs`](../../crates/ha-core/src/session/db.rs) | `sessions.project_id` 迁移 + `ProjectFilter` + 绑定 API |
| [`crates/ha-core/src/system_prompt/build.rs`](../../crates/ha-core/src/system_prompt/build.rs#L40-L264) | 把 `project` + `project_files` 接入装配链 |
| [`crates/ha-core/src/system_prompt/sections.rs`](../../crates/ha-core/src/system_prompt/sections.rs#L424-L575) | `build_project_context_section` + `build_project_files_section` + 图标映射 |
| [`crates/ha-core/src/tools/project_read_file.rs`](../../crates/ha-core/src/tools/project_read_file.rs) | 工具执行体，含路径白名单校验 |
| [`crates/ha-core/src/tools/definitions/core_tools.rs`](../../crates/ha-core/src/tools/definitions/core_tools.rs#L131-L160) | `project_read_file` 工具 schema 注册 |
| [`crates/ha-core/src/memory/types.rs`](../../crates/ha-core/src/memory/types.rs#L49-L61) | `MemoryScope::Project` 变种 |
| [`crates/ha-core/src/memory/sqlite/trait_impl.rs`](../../crates/ha-core/src/memory/sqlite/trait_impl.rs#L446-L478) | `load_prompt_candidates_with_project` 三层优先级 |
| [`crates/ha-core/src/memory_extract.rs`](../../crates/ha-core/src/memory_extract.rs#L20-L31) | 自动提取作用域推断 |
| [`crates/ha-core/src/agent/resolver.rs`](../../crates/ha-core/src/agent/resolver.rs) | 5 级 agent 解析链 + `_with_source` 调试入口 |
| [`crates/ha-core/src/util.rs`](../../crates/ha-core/src/util.rs) | `canonicalize_working_dir`（session/project 共用写入校验） |
| [`crates/ha-core/src/session/helpers.rs`](../../crates/ha-core/src/session/helpers.rs#L79) | `effective_session_working_dir` 合并入口 |
| [`crates/ha-core/src/channel/db.rs`](../../crates/ha-core/src/channel/db.rs) | `ensure_conversation` / `resolve_or_create_session` 反查 bound_channel + 注入 project_id |
| [`crates/ha-core/src/slash_commands/handlers/project.rs`](../../crates/ha-core/src/slash_commands/handlers/project.rs) | `/project` 命令 handler（含 IM 禁用兜底） |
| [`src-tauri/src/commands/project.rs`](../../src-tauri/src/commands/project.rs) | Tauri 命令，spawn_blocking + emit 事件 |
| [`src-tauri/src/commands/session.rs`](../../src-tauri/src/commands/session.rs) | `update_session_working_dir_cmd` / `update_session_agent_cmd` |
| [`crates/ha-server/src/routes/projects.rs`](../../crates/ha-server/src/routes/projects.rs) | HTTP Handler，multipart 上传，跨 DB 补齐 memory_count |
| [`crates/ha-server/src/routes/filesystem.rs`](../../crates/ha-server/src/routes/filesystem.rs) | `GET /api/filesystem/list-dir`（HTTP 模式 ServerDirectoryBrowser 后端） |
| [`src/components/chat/project/ProjectSection.tsx`](../../src/components/chat/project/ProjectSection.tsx) | 侧边栏项目树（含 ProjectGroup 嵌套） |
| [`src/components/chat/project/ProjectDialog.tsx`](../../src/components/chat/project/ProjectDialog.tsx) | create / edit 复用对话框 |
| [`src/components/chat/project/ProjectOverviewDialog.tsx`](../../src/components/chat/project/ProjectOverviewDialog.tsx) | 项目设置 Sheet（保留旧文件名，UI 已是 Sheet 三 Tab） |
| [`src/components/chat/project/ProjectFilesPanel.tsx`](../../src/components/chat/project/ProjectFilesPanel.tsx) | 文件上传 / 列表 / 删除 / 重命名 UI |
| [`src/components/chat/AgentSwitcher.tsx`](../../src/components/chat/AgentSwitcher.tsx) | 标题栏 Agent dropdown（messages 非空时 disabled） |
| [`src/components/chat/WorkingDirectoryButton.tsx`](../../src/components/chat/WorkingDirectoryButton.tsx) | 工作目录按钮（区分会话级 / 继承自项目） |
| [`src/components/chat/project/hooks/useProjects.ts`](../../src/components/chat/project/hooks/useProjects.ts) | 项目列表状态 + CRUD + EventBus 订阅 |
| [`src/components/chat/project/hooks/useProjectFiles.ts`](../../src/components/chat/project/hooks/useProjectFiles.ts) | 单项目文件列表状态 |
