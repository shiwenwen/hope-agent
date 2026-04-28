# Project 项目系统架构

> 返回 [文档索引](../README.md) | 更新时间：2026-04-28

## 目录

- [概述](#概述)
- [数据模型](#数据模型)
- [SQLite Schema](#sqlite-schema)
- [磁盘布局](#磁盘布局)
- [核心 API](#核心-api)
- [文件上传管道](#文件上传管道)
- [System Prompt 三层注入](#system-prompt-三层注入)
- [默认工作目录](#默认工作目录)
- [IM Channel 绑定](#im-channel-绑定)
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

### Project ([types.rs:14-38](../../crates/ha-core/src/project/types.rs#L14-L38))

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `String` | UUID v4 主键 |
| `name` | `String` | 项目名称（trim 后不得为空） |
| `description` | `Option<String>` | 项目简介 |
| `instructions` | `Option<String>` | 自定义指令，追加到项目内每个会话的 System Prompt |
| `emoji` | `Option<String>` | 侧边栏 / 标题前缀 emoji（无 `logo` 时使用） |
| `logo` | `Option<String>` | `data:image/...;base64,...` 内联图标。**优先级高于 `emoji`**，写入走 `validate_logo` 校验：必须是 `data:image/` 前缀、`<= 512 KB`（前端期望 ~256px 下采样到 ~20 KB）。拒绝 http/file URL 防 SSRF |
| `color` | `Option<String>` | 强调色（目前 UI 内部装饰用） |
| `default_agent_id` | `Option<String>` | 新建会话时的默认 Agent，参与 [Agent 解析链](#关联文档) |
| `default_model_id` | `Option<String>` | 新建会话时的默认模型 |
| `working_dir` | `Option<String>` | 项目内会话的默认工作目录（绝对路径）。session 自身未设 `working_dir` 时回落到此值；详见[默认工作目录](#默认工作目录) |
| `bound_channel` | `Option<BoundChannel>` | 绑定一个 IM channel account，新会话自动落到此项目；详见[IM Channel 绑定](#im-channel-绑定) |
| `created_at` / `updated_at` | `i64` | Unix 毫秒时间戳 |
| `archived` | `bool` | 归档标志（不删除，默认列表过滤） |

**`BoundChannel`**（[types.rs:57-62](../../crates/ha-core/src/project/types.rs#L57-L62)）：`{ channel_id: String, account_id: String }`，落 SQLite 时拆成 `bound_channel_id` / `bound_channel_account_id` 两列 + 复合索引。

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

- `CreateProjectInput`：`name` 必填，其余可选
- `UpdateProjectInput`：全字段 `Option<_>`，PATCH 语义。**空串正规化为 NULL**（[db.rs push_str_field](../../crates/ha-core/src/project/db.rs#L159-L175)），让调用方能显式清空某个可选字段
- `working_dir` 写入路径（`create` / `update` 共用）必经 [`crate::util::canonicalize_working_dir`](../../crates/ha-core/src/util.rs#L193-L206)：trim 后空串 → `None`（视为清空），非空值 `canonicalize` + `is_dir` 校验，失败 `Err` 抛回调用方。session 级 `update_session_working_dir` 走同一入口，保证两侧错误措辞和解析语义对齐
- `logo` 写入路径必经 [`validate_logo`](../../crates/ha-core/src/project/db.rs#L715-L737)：trim 后空串 → 清空、`data:image/` 前缀强制、`<= 512 KB`，否则 `bail!`
- `bound_channel` 在 `UpdateProjectInput` 用 **double-Option**（`Option<Option<BoundChannel>>`，自定义 deserializer）：字段缺省=不变，JSON `null`=解绑，对象=设置；`CreateProjectInput` 单 Option 无此区分

## SQLite Schema

两张表随 `SessionDB` 的连接共享，由 `ProjectDB::migrate()` 幂等建表（[db.rs:27-71](../../crates/ha-core/src/project/db.rs#L27-L71)）。

```sql
CREATE TABLE IF NOT EXISTS projects (
    id                         TEXT PRIMARY KEY,
    name                       TEXT NOT NULL,
    description                TEXT,
    instructions               TEXT,
    emoji                      TEXT,
    color                      TEXT,
    default_agent_id           TEXT,
    default_model_id           TEXT,
    created_at                 INTEGER NOT NULL,
    updated_at                 INTEGER NOT NULL,
    archived                   INTEGER NOT NULL DEFAULT 0,
    logo                       TEXT,
    working_dir                TEXT,
    bound_channel_id           TEXT,
    bound_channel_account_id   TEXT
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

**sessions 表扩展**（[session/db.rs:232-239](../../crates/ha-core/src/session/db.rs#L232-L239)）：`SessionDB::open` 迁移阶段 `ALTER TABLE sessions ADD COLUMN project_id TEXT` + 建 `idx_sessions_project_id` 索引，老库零破坏升级。

**老库迁移**（[db.rs:76-110](../../crates/ha-core/src/project/db.rs#L76-L110)）：`migrate()` 对每个后加列做"探测式 ALTER"——`SELECT <col> LIMIT 1` 失败即增列，幂等可在每次启动重跑。当前覆盖 `logo` / `working_dir` / `bound_channel_id` / `bound_channel_account_id`。

> **回归注意**：`bound_channel_*` 增列与 `idx_projects_bound_channel` 索引必须分两个 `execute_batch` 跑——历史曾因放在同一 batch 里、SQLite 在前置 ALTER 提交前评估到 `CREATE INDEX … no such column` 触发 `migrate_pre_bound_channel_schema` 测试用例所覆盖的崩溃（[db.rs:771-826](../../crates/ha-core/src/project/db.rs#L771-L826)）。

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
| `create(CreateProjectInput)` | 插入新项目，返回 `Project` |
| `get(id)` | 取单个项目 |
| `update(id, UpdateProjectInput)` | 动态 SQL 部分更新，空串 → NULL，强制 bump `updated_at` |
| `delete(id)` → `Vec<ProjectFile>` | `IMMEDIATE` 事务内：① `SELECT` 快照文件行作为返回值 ② `UPDATE sessions SET project_id = NULL` ③ `DELETE FROM projects`（FK CASCADE 顺带删 `project_files`）。返回的文件列表供调用者清理磁盘 |
| `list_all_ids()` | 轻量级 id 列表，reconciler 专用 |
| `list(include_archived)` → `Vec<ProjectMeta>` | 带 `session_count` / `file_count` 聚合子查询；`memory_count = 0` 待调用方补齐 |
| `find_by_bound_channel(channel_id, account_id)` | 反查认领某 IM channel account 的项目（活跃 + 归档都返回），channel worker 创建会话时使用 |

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

## 默认工作目录

`Project.working_dir` 是项目内会话的**默认工作目录**——session 级 `working_dir` 覆盖项目级，未设则回落到项目级，两边都没有则不注入。**lazy resolve**（不复制快照）：改项目工作目录立即对所有未单独设置的已有会话生效，不需要重写 sessions 表。

### 合并逻辑唯一入口

[`crates/ha-core/src/session/helpers.rs::effective_session_working_dir`](../../crates/ha-core/src/session/helpers.rs#L74-L91)：

```rust
pub fn effective_session_working_dir(session_id: Option<&str>) -> Option<String> {
    let meta = lookup_session_meta(session_id)?;
    if let Some(wd) = meta.working_dir.filter(|s| !s.trim().is_empty()) {
        return Some(wd);                          // session 级优先
    }
    let pid = meta.project_id?;
    crate::get_project_db()?
        .get(&pid).ok().flatten()?
        .working_dir
        .filter(|s| !s.trim().is_empty())         // 项目级回落
}
```

这是 system prompt 渲染和工具运行时**共同**的真相源——两边消费同一个值，模型在 prompt 里看到的"current working directory"和 `read` / `write` / `exec` 的相对路径解析永远一致。

### 消费点

| 消费方 | 入口 | 作用 |
|---|---|---|
| **System Prompt 渲染** | [`agent/config.rs:341-344`](../../crates/ha-core/src/agent/config.rs#L341-L344) | 把合并值传给 [`system_prompt::build`](../../crates/ha-core/src/system_prompt/build.rs#L54)，在 Memory 段之前注入 `# Working Directory` 段（含路径 + Working Directory Instructions）|
| **主对话工具执行** | [`agent/mod.rs:1224-1230`](../../crates/ha-core/src/agent/mod.rs#L1224-L1230) | 写入 `ToolExecContext.session_working_dir`，被 `read` / `write` / `exec` 解析相对路径、`write_file` 路径白名单消费 |
| **斜杠命令执行** | [`slash_commands/handlers/mod.rs:181`](../../crates/ha-core/src/slash_commands/handlers/mod.rs#L181) | 同上，让 `/run`、`/edit` 等内置命令也走合并值 |

### 写入校验

session 和 project 两侧的写入都过 [`crate::util::canonicalize_working_dir`](../../crates/ha-core/src/util.rs#L193-L206) 单一入口：

- `None` / 空串 / 全空白 → `Ok(None)`，调用方解读为"清空选择"
- 非空值 → `path.canonicalize()` + `is_dir` 校验；失败返回带原始路径的 `anyhow!` 错误供 UI 展示
- 校验通过返回**绝对规范化路径**写入 DB，保证模型 prompt 和 tool ctx 的路径形态一致

### Project / Incognito / Channel 正交

`Project.working_dir` 与 IM channel 绑定、incognito、agent override 全部正交：incognito 会话依然能拿到合并值，channel 自动落项目时一起继承项目 default。

## IM Channel 绑定

`Project.bound_channel = Some(BoundChannel { channel_id, account_id })` 让一个项目**认领**一个 IM channel account——之后该 (channel, account) 的所有新会话都自动落到这个项目，无需用户在每条消息后手动归类。

### 唯一性约束

**同一 `(channel_id, account_id)` 对最多被一个项目认领。** 双重防御：

1. **DB 索引**：`idx_projects_bound_channel(bound_channel_id, bound_channel_account_id)` 加速反查
2. **写入冲突检测**：[`ProjectDB::create`](../../crates/ha-core/src/project/db.rs#L138-L155) 与 `update`（[`db.rs:312-347`](../../crates/ha-core/src/project/db.rs#L312-L347)）插入/修改前先 `SELECT id … WHERE bound_channel_id=? AND bound_channel_account_id=? AND id != ?` 探测，命中即 `bail!("channel binding already claimed by project {}…")` 让前端提示用户先解绑

### Channel Worker 自动落项目

入口 [`channel/db.rs::resolve_or_create_session`](../../crates/ha-core/src/channel/db.rs#L75-L141)：

```text
1. 命中 channel_conversations 已有映射 → 复用旧 session（不重判项目）
2. 否则反查 projects WHERE bound_channel_id=? AND bound_channel_account_id=? AND archived=0
3. 命中：用 project.default_agent_id（非空 trim）覆盖 caller 传入的 channel agent_id
        + 创建会话时 project_id = 命中项目
4. 未命中：保持 caller agent_id，project_id=NULL
```

> **归档=失效绑定**：`archived = 0` 过滤让"归档项目"的绑定自动失效，新消息进来就走未命中分支创建无项目会话——避免归档项目继续吞流量。

### Agent 解析链 5 级

新会话 agent_id 解析顺序统一在 [`agent::resolver::resolve_default_agent_id`](../../crates/ha-core/src/agent/resolver.rs)（详见 AGENTS.md「Agent 解析链」段）：

```text
显式参数 → project.default_agent_id → channel_account.agent_id
        → AppConfig.default_agent_id → 硬编码 "default"
```

`bound_channel` 命中时 `project.default_agent_id` 排在 `channel_account.agent_id` 之前——也就是说项目级 agent 覆盖 channel 级 agent。

### `/status` 摘要

项目内会话执行 `/status` 时，斜杠 handler 在尾部追加项目摘要段并标注 Agent 实际命中的来源（哪一层 5 级链解析到的），方便用户排查"为什么这个 channel 的会话用了别的 agent"。

### 与 Incognito 互斥

`bound_channel` / `project_id` 与 `sessions.incognito = true` 互斥（详见 AGENTS.md「会话级无痕对话」段）：项目会话强制 `incognito=false`，反之亦然。`channel/db.rs::ensure_conversation` 入口防御式清零。

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
| `list_project_files_cmd(id)` | 按 created_at DESC |
| `upload_project_file_cmd(project_id, file_name, mime_type?, data)` | `spawn_blocking`，emit `project:file_uploaded` |
| `delete_project_file_cmd(project_id, file_id)` | `spawn_blocking`，emit `project:file_deleted` |
| `rename_project_file_cmd(project_id, file_id, name)` | 只改显示名 |
| `read_project_file_content_cmd(project_id, file_id, offset?, limit?)` | UI 预览 extracted 文本 |
| `list_project_memories_cmd(id, limit?, offset?)` | Project scope 记忆列表 |

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

上传复用 `routes::helpers::parse_file_upload` 取 multipart 字段，前置校验 `MAX_PROJECT_FILE_BYTES` 让 oversize 在触盘前得到清晰错误。

## 前端 UI

### 侧边栏 ProjectSection ([ProjectSection.tsx](../../src/components/chat/project/ProjectSection.tsx))

在 [`ChatSidebar.tsx:317-325`](../../src/components/chat/sidebar/ChatSidebar.tsx#L317-L325) 位于 `AgentSection` 上方：

- `projects.length > 0 || onAddProject` 才渲染（向下兼容）
- 展开/折叠头 + 项目列表（过滤已归档）+ "+" 新建按钮
- 点击项目打开 `ProjectOverviewDialog`

### ProjectDialog ([ProjectDialog.tsx](../../src/components/chat/project/ProjectDialog.tsx))

`mode="create" | "edit"` 复用同一组件：

- 空白态 → `onCreate(CreateProjectInput)`
- 预填态 → `onUpdate(UpdateProjectInput)`
- 字段：name / description / instructions / emoji / **logo**（图片上传 → 前端下采样到 ~256px → base64 编码为 `data:image/...` URL）/ color / defaultAgentId（Select AgentSummary）/ defaultModelId / **workingDir**（复用 `useDirectoryPicker`，桌面走 Tauri `dialog.open({directory:true})`，HTTP 走 `ServerDirectoryBrowser`）
- 保存按钮三态（idle → saving → saved/failed），对齐 `AGENTS.md` UI 约定

> Logo 与 emoji 同时存在时 UI 优先渲染 logo，emoji 作为后备；侧边栏 / Overview 头部都遵循此优先级。

### WorkingDirectoryButton ([WorkingDirectoryButton.tsx](../../src/components/chat/input/WorkingDirectoryButton.tsx))

ChatInput 工具栏 / `ChatTitleBar` 共用。展示**生效路径**（即 `effective_session_working_dir` 的合并结果），并通过 `inherited` prop 区分两种 source：

| `inherited` | 含义 | UI 行为 |
|---|---|---|
| `false`（默认） | 来自 session 自身 `working_dir` | 显示 basename + 路径 tooltip + **clear 按钮**（X 图标） |
| `true` | 来自 `Project.working_dir` 回落 | 显示 basename + tooltip 标注「继承自项目」+ **不渲染 clear 按钮**（避免 no-op：清空一个 session 从未持有的值是无意义操作）|

`onChange(null)` 仅在 `!inherited` 路径触发，由父组件调 `update_session_working_dir` 持久化；改项目级则走 `ProjectDialog` → `update_project_cmd`。

### ChatTitleBar 工作目录显示

标题栏渲染同一个生效路径，区分「会话级」「继承自项目」两种 source 给用户即时反馈"我现在改的是哪一层"。继承态下点击会落到 `ProjectOverviewDialog` 而非 session 级 picker。

### ProjectOverviewDialog ([ProjectOverviewDialog.tsx](../../src/components/chat/project/ProjectOverviewDialog.tsx))

> 已重构为右侧 `Sheet`（[`src/components/ui/sheet.tsx`](../../src/components/ui/sheet.tsx)）；保留文件名以减少 import 改动。原"Sessions" Tab 已删除（侧边栏树状已暴露），Sessions 入口改由侧边栏 [`ProjectGroup`](../../src/components/chat/project/ProjectSection.tsx) 嵌套承载。

| Tab | 作用 |
|---|---|
| **Overview** | 元数据展示 + 4 个操作：New session / Edit / Archive·Unarchive / Delete + **绑定 IM Channel select**（`{channelId, accountId}` 写入 `bound_channel`，下拉项是当前已配置的 IM channel account；选 "None" → 解绑发送 JSON `null`） |
| **Files** | 嵌入 `ProjectFilesPanel`（拖拽 / 点击上传，20MB 提示、删除、重命名） |
| **Instructions** | Textarea 编辑 `instructions`，保存调 `onUpdateProject` |

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
| `project:updated` | 更新 / 归档成功后 | `update_project_cmd` / `archive_project_cmd` / 对应 HTTP handler |
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

## 关联文档

- [Session 系统](session.md) — `sessions.project_id` / `sessions.working_dir` 列、`ProjectFilter` 枚举、会话级 API
- [记忆系统](memory.md) — `MemoryScope::Project`、三级作用域预算、`scope_project_id` 索引
- [提示词系统](prompt-system.md) — System Prompt 13 段装配顺序、`# Working Directory` 段位置
- [工具系统](tool-system.md) — `project_read_file` 工具注册、`ToolExecContext.session_working_dir` 消费链
- [IM Channel](im-channel.md) — `resolve_or_create_session` 自动落项目流程、`ChannelCapabilities` 与按钮审批
- AGENTS.md「Agent 解析链」 — 5 级 default agent 解析顺序与项目级覆盖位置

## 文件清单

| 文件 | 职责 |
|---|---|
| [`crates/ha-core/src/project/mod.rs`](../../crates/ha-core/src/project/mod.rs) | 模块声明 + re-export（`ProjectDB` / `MAX_PROJECT_FILE_BYTES` 等） |
| [`crates/ha-core/src/project/types.rs`](../../crates/ha-core/src/project/types.rs) | `Project` / `ProjectMeta` / `ProjectFile` + 两个 Input DTO |
| [`crates/ha-core/src/project/db.rs`](../../crates/ha-core/src/project/db.rs) | `ProjectDB` 主实现，复用 `SessionDB` 连接 |
| [`crates/ha-core/src/project/files.rs`](../../crates/ha-core/src/project/files.rs) | 上传管道、删除、`delete_project_cascade`、目录 purge 防逃逸 |
| [`crates/ha-core/src/project/reconcile.rs`](../../crates/ha-core/src/project/reconcile.rs) | 启动期跨 DB 孤儿记忆清理 |
| [`crates/ha-core/src/util.rs`](../../crates/ha-core/src/util.rs#L193-L206) | `canonicalize_working_dir`：session/project 共用的 working_dir 写入校验入口 |
| [`crates/ha-core/src/session/helpers.rs`](../../crates/ha-core/src/session/helpers.rs#L74-L91) | `effective_session_working_dir`：session/project 合并的真相源，prompt 渲染 + tool ctx 共同消费 |
| [`crates/ha-core/src/agent/config.rs`](../../crates/ha-core/src/agent/config.rs#L341-L344) | system prompt 装配前的 working_dir 合并消费点 |
| [`crates/ha-core/src/channel/db.rs`](../../crates/ha-core/src/channel/db.rs#L75-L141) | `resolve_or_create_session`：IM 新会话反查 `bound_channel` 自动落项目 + 5 级 agent 解析的项目级注入点 |
| [`crates/ha-core/src/paths.rs`](../../crates/ha-core/src/paths.rs#L244-L261) | `projects_dir` / `project_dir` / `project_files_dir` / `project_extracted_dir` |
| [`crates/ha-core/src/session/db.rs`](../../crates/ha-core/src/session/db.rs) | `sessions.project_id` 迁移 + `ProjectFilter` + 绑定 API |
| [`crates/ha-core/src/system_prompt/build.rs`](../../crates/ha-core/src/system_prompt/build.rs#L40-L264) | 把 `project` + `project_files` 接入装配链 |
| [`crates/ha-core/src/system_prompt/sections.rs`](../../crates/ha-core/src/system_prompt/sections.rs#L424-L575) | `build_project_context_section` + `build_project_files_section` + 图标映射 |
| [`crates/ha-core/src/tools/project_read_file.rs`](../../crates/ha-core/src/tools/project_read_file.rs) | 工具执行体，含路径白名单校验 |
| [`crates/ha-core/src/tools/definitions/core_tools.rs`](../../crates/ha-core/src/tools/definitions/core_tools.rs#L131-L160) | `project_read_file` 工具 schema 注册 |
| [`crates/ha-core/src/memory/types.rs`](../../crates/ha-core/src/memory/types.rs#L49-L61) | `MemoryScope::Project` 变种 |
| [`crates/ha-core/src/memory/sqlite/trait_impl.rs`](../../crates/ha-core/src/memory/sqlite/trait_impl.rs#L446-L478) | `load_prompt_candidates_with_project` 三层优先级 |
| [`crates/ha-core/src/memory_extract.rs`](../../crates/ha-core/src/memory_extract.rs#L20-L31) | 自动提取作用域推断 |
| [`src-tauri/src/commands/project.rs`](../../src-tauri/src/commands/project.rs) | 14 个 Tauri 命令，spawn_blocking + emit 事件 |
| [`crates/ha-server/src/routes/projects.rs`](../../crates/ha-server/src/routes/projects.rs) | HTTP Handler，multipart 上传，跨 DB 补齐 memory_count |
| [`src/components/chat/project/ProjectSection.tsx`](../../src/components/chat/project/ProjectSection.tsx) | 侧边栏项目折叠块 |
| [`src/components/chat/project/ProjectDialog.tsx`](../../src/components/chat/project/ProjectDialog.tsx) | create / edit 复用对话框 |
| [`src/components/chat/project/ProjectOverviewDialog.tsx`](../../src/components/chat/project/ProjectOverviewDialog.tsx) | 四 Tab 项目主页 |
| [`src/components/chat/project/ProjectFilesPanel.tsx`](../../src/components/chat/project/ProjectFilesPanel.tsx) | 文件上传 / 列表 / 删除 / 重命名 UI |
| [`src/components/chat/input/WorkingDirectoryButton.tsx`](../../src/components/chat/input/WorkingDirectoryButton.tsx) | 工具栏 / 标题栏复用的 working dir picker，按 `inherited` 区分会话级 vs 继承自项目 |
| [`src/components/chat/project/hooks/useProjects.ts`](../../src/components/chat/project/hooks/useProjects.ts) | 项目列表状态 + CRUD + EventBus 订阅 |
| [`src/components/chat/project/hooks/useProjectFiles.ts`](../../src/components/chat/project/hooks/useProjectFiles.ts) | 单项目文件列表状态 |
