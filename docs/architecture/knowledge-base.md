# Knowledge Base 知识库系统架构（设计草案）

> 返回 [文档索引](../README.md) | 状态：**设计草案（Draft，尚未实现）** | 创建时间：2026-06-02

> ⚠️ 本文是**设计契约文档**，不是已落地子系统的描述。它先于实现存在，用于锁定方向、记录取舍、指导分阶段迭代。每次方案打磨都应回到本文更新「决策账本」与「路线图」，保持单一真相源。代码落地后，本文逐步转为实现描述，并把 `规划中` 的源码路径替换为真实链接。

## 目录

- [背景与动机](#背景与动机)
- [设计目标与非目标](#设计目标与非目标)
- [核心定位：第四种知识 Scope](#核心定位第四种知识-scope)
- [决策账本](#决策账本)
- [数据模型](#数据模型)
- [磁盘布局](#磁盘布局)
- [SQLite 索引 Schema](#sqlite-索引-schema)
- [Wikilink 语法与解析](#wikilink-语法与解析)
- [后端模块与作用域](#后端模块与作用域)
- [Agent 工具（读写桥）](#agent-工具读写桥)
- [AI 双向桥](#ai-双向桥)
- [前端 UI](#前端-ui)
- [跨端契约对齐](#跨端契约对齐)
- [分阶段路线图](#分阶段路线图)
- [安全约束](#安全约束)
- [关联文档](#关联文档)
- [文件清单（规划）](#文件清单规划)

---

## 背景与动机

### 为什么做

2025–2026 年个人知识管理（PKM）领域的主流趋势是**本地优先（local-first）+ 网络化思考（networked thought）+ AI 辅助**：双向链接、图谱视图、Zettelkasten/PARA 方法论、块级引用、每日笔记，代表工具是 Obsidian / Logseq / SiYuan / Anytype。这些工具的共同短板是**链接靠人手动织、知识靠人主动整理**——AI 只是事后插件。

Hope Agent 的定位是「越用越懂你 + 长期沉淀」的本地 AI 助手。把 PKM 能力做进来，是把产品从「聊天助手」升级为「第二大脑」的自然一步。**差异化不在于再造一个 Obsidian，而在于 AI 原生**：别人手动连线，我们让 agent 既能读知识库、又能写知识库，并能把后台积累的记忆自动提炼成人类可读的结构化笔记。

### Hope Agent 已经具备的地基（关键前提）

设计本系统时必须意识到：**hope-agent 已经是一个"半成品的 AI 原生 PKM"**，大量基建可直接复用，不要重造：

| 已有能力 | 现状 | 对知识库的价值 |
|---|---|---|
| 「文件即真实文件」哲学 | Project 的 `working_dir` 就是磁盘真实文件目录；`project_files` 表已被**刻意删除**，模型靠 `# Working Directory` 段 + `read` 工具感知文件（见 [Project 系统](project.md)） | 笔记 = 真实 `.md` 文件，天然契合，且可与 Obsidian 互通 |
| 混合检索引擎 | `memory.db` 已有 FTS5 + sqlite-vec 向量 + RRF 融合 + MMR + 时间衰减 + embedding 缓存（见 [记忆系统](memory.md)） | 笔记检索直接复用，**不重写检索算法** |
| Dreaming 离线整理 | idle/cron 触发 → `side_query` 给记忆打分 → 提炼 → 写 `~/.hope-agent/memory/dreams/{date}.md` 日记 | "AI 自组织"骨架已在跑，扩展为"提炼笔记/MOC"即可 |
| 文件作用域安全模型 | `filesystem::WorkspaceScope`（canonicalize + `starts_with` 闭合）、完整 CRUD ops、`project:fs_changed` 事件（见 [文件操作统一](file-operations.md)） | 知识库读写边界直接套用 |
| Markdown 预览 | `FilePreviewPane` 已能渲染 `.md`（Render/Source 切换 + 选中引用到聊天），`markdown` 是 previewable kind | 笔记预览现成 |
| 后台调度 | cron / async_jobs / dreaming idle ticker / recap / awareness 一整套后台 AI 机制 | 知识库的后台索引/整理任务直接挂载 |
| Side Query | 复用主对话 prompt cache，侧查询成本降 ~90%（见 [Side Query](side-query.md)） | AI 提炼笔记的低成本推理入口 |

### 现状的能力缺口（代码里完全没有，需新建）

- ❌ Wikilink `[[Note]]` / 别名 / heading 锚点 / 块引用 `![[Note#^id]]` 解析
- ❌ 反向链接（backlinks）索引
- ❌ 图谱视图
- ❌ 笔记级元数据 / frontmatter / MOC（Maps of Content）概念
- ❌ 独立于 Project 的「知识库」容器概念

**结论**：本系统的工作量集中在「**双链解析 + 反链索引 + 知识库容器 + 前端知识视图**」，检索/存储/后台/安全基建尽量复用。

---

## 设计目标与非目标

### 目标（Goals）

1. **独立的一级功能**：知识库是与聊天、Dashboard 平级的独立概念，不是 Project 的附属。用户可创建/分类/手写/编辑/管理笔记，是一个**完整的大功能**。
2. **本地优先 + 可移植**：笔记是真实 `.md` 文件，是唯一真相源；索引只是可重建缓存。用户可随时用 Obsidian/Logseq 打开同一批文件，零锁定。
3. **AI 双向紧密联合**：
   - **写入桥**：agent 能创建/编辑笔记；记忆系统可把碎片提炼成结构化笔记（"可读层"）。
   - **读取桥**：笔记被索引，agent 能按需检索召回进上下文。
4. **双链为地基**：Wikilink + 反向链接是第一阶段必须跑通的最小价值线，后续图谱/嵌入/块引用都建立其上。
5. **契约对齐**：核心逻辑全进 `ha-core`（零 Tauri 依赖），桌面/HTTP/ACP 三端一致，GUI 与 `ha-settings` 技能零偏差。

### 非目标（Non-Goals）

- **不**再造一个独立的 `~/HopeVault` 纯文件 vault 概念——与「文件即真实文件」红线冲突，且无谓增加心智负担。
- **不**把笔记正文塞进数据库当真相源（排除"全进 pkm.db"方案）。
- **不**默认把全部笔记注入 system prompt（会撑爆上下文）——召回走按需工具。
- **不**在第一阶段做块级引用 `^block-id`（需块级 ID 体系，工程量大，放 Phase 3）。
- **不**替换现有 Markdown 编辑/预览栈做花哨富文本编辑器——Phase 1 复用现有能力，富文本编辑器（Tiptap/Milkdown）作为 Phase 2 评估项。

---

## 核心定位：第四种知识 Scope

Hope Agent 已有三层知识容器，知识库（Knowledge Base, KB）是平行的第四个：

| 容器 | 真相源 | 谁写 | 谁读 | 用户可见度 |
|---|---|---|---|---|
| Memory | `memory.db` 原子条目 | 自动抽取 + `save_memory` | 注入 system prompt | 低（后台） |
| Dreaming 日记 | `~/.hope-agent/memory/dreams/*.md` | AI 自省 | 用户翻看 | 中 |
| Project | `working_dir` 真实文件 | 用户/agent | `read` 工具 | 高 |
| **🆕 知识库（KB）** | **真实 `.md` 文件** | **用户手写 + agent 工具** | **agent 工具 + 按需召回** | **最高（一级导航）** |

**和 AI 的双向桥**（本系统区别于 Obsidian 的核心）：

```
                ┌──────────────── 写入桥 ────────────────┐
   对话 ──► Memory（碎片）──► Dreaming 提炼 ──► 知识库笔记（MOC/可读层）
                                                    │
                ┌──────────────── 读取桥 ────────────┘
   agent ◄── note_search 按需召回 ◄── FTS5 + 向量索引 ◄── 笔记
```

---

## 决策账本

> 本节是迭代时的"翻账依据"。每条决策记录**选项、结论、理由**；待定项记录**默认取向**，方便后续直接确认或推翻。

### 已定决策（来自设计对话）

| # | 决策点 | 结论 | 理由 |
|---|---|---|---|
| D1 | 笔记与记忆系统的关系 | **A+B 融合**：独立的笔记系统，但与 AI 双向打通——agent 能读能写，记忆可提炼写入笔记形成可读层 | 用户明确要"一个完整的大功能 + AI 紧密联合"，既不是纯手动（A），也不是把笔记降级为大号 memory（C） |
| D2 | 存储真相源 | **真实 `.md` 文件 + SQLite 旁路索引** | 贴合「文件即真实文件」红线；可与 Obsidian 互通；索引可重建；检索复用 memory embedding 基建 |
| D3 | 挂载的容器概念 | **独立的「知识库」容器**（非复用 Project） | 用户要一级功能、独立心智模型。代价是新建一套容器/作用域/权限，已接受 |
| D4 | 第一阶段 MVP | **双链基础：Wikilink 解析 + Backlinks 面板** | 最小可用、最快出效果，是图谱/嵌入/召回的地基 |

### 待定决策（已填默认取向，待确认）

| # | 决策点 | 默认取向 | 备选 | 取舍 |
|---|---|---|---|---|
| P1 | 命名 | 对内对外统一「知识库 / Knowledge Base（KB）」 | 第二大脑 / Brain / Notes / Vault | "知识库"直白、中性、可 i18n；"第二大脑"营销感强但易与 Memory 概念混淆 |
| P2 | 外部目录绑定（Obsidian 互通） | **Phase 1 只做内置 `notes/` 目录**；Schema 从第一天就预留 `root_dir: Option<path>`，外部 vault 绑定放 **Phase 2** | Phase 1 即支持绑定现成 Obsidian vault | 绑外部目录要额外处理 `.obsidian`/`.git` 忽略、外部并发编辑冲突、watcher 噪声；MVP 先收敛风险，但数据模型不留债 |
| P3 | 召回融合形态 | **Phase 1 独立 `note_search` 工具**；笔记与 memory 是否在 `recall_memory` 内融合检索，放 **Phase 3** 评估 | 直接折进 `recall_memory` 一次拿记忆+笔记 | 独立工具干净、不动成熟的 memory 路径；融合体验更好但改动面大、回归风险高 |

---

## 数据模型

> 类型规划落在 `crates/ha-core/src/knowledge/types.rs`（规划中）。

### KnowledgeBase

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `String` | UUID v4 主键 |
| `name` | `String` | 知识库名称（trim 后非空） |
| `emoji` | `Option<String>` | 侧边栏前缀 |
| `root_dir` | `Option<String>` | 笔记根目录绝对路径。`NULL` = 用默认 `~/.hope-agent/knowledge/{id}/notes/`（lazy ensure，仿 project workspace）。**非 NULL = 绑定外部目录（如 Obsidian vault）**，Phase 2 启用 |
| `created_at` / `updated_at` | `String` | ISO8601 |

### Note（索引行，真相在文件）

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `i64` | 自增主键（索引内部用） |
| `kb_id` | `String` | 所属知识库 |
| `rel_path` | `String` | 相对 `root_dir` 的路径（如 `Zettelkasten/202606021530.md`） |
| `title` | `String` | 取自 frontmatter `title` > 首个 H1 > 文件名（去扩展名） |
| `frontmatter_json` | `Option<String>` | YAML frontmatter 解析后的 JSON |
| `mtime` / `size` | `i64` | 文件修改时间 / 字节数，增量索引判脏用 |
| `embedding` | `BLOB` | 向量（复用 memory 的 `EmbeddingProvider` + `embedding_cache`） |
| `embedding_signature` | `Option<String>` | 产出该向量的 embedding 模型签名 |

### NoteLink（双链边，MVP 核心）

| 字段 | 类型 | 说明 |
|---|---|---|
| `src_note_id` | `i64` | 出链来源笔记 |
| `target_title` | `String` | `[[ ]]` 内的目标标题（原文） |
| `target_note_id` | `Option<i64>` | 解析命中的目标笔记；`NULL` = **悬空链接（broken link）**，前端高亮提示可新建 |
| `link_type` | `TEXT` | `wiki`（`[[ ]]`）/ `embed`（`![[ ]]`，Phase 2）/ `md`（标准 `[]()`） |
| `anchor` | `Option<String>` | `[[Note#Heading]]` 的 heading，或 `^block-id`（Phase 3） |

**反向链接** = `SELECT * FROM note_link WHERE target_note_id = ?`，一个索引即可，无需独立表。

---

## 磁盘布局

```
~/.hope-agent/
  knowledge/
    index.db                      # 🆕 所有 KB 的旁路索引（可随时全量重建，从不污染笔记目录）
    {kb_id}/
      notes/                      # 默认笔记目录（root_dir 为 NULL 时 lazy ensure）
        Zettelkasten/...
        每日笔记/2026-06-02.md
        ...
```

关键设计：

- **索引 db 统一放 `~/.hope-agent/knowledge/index.db`**，带 `kb_id` 列区分多个 KB。**绝不写进笔记目录**——这样 KB 绑定外部目录（Obsidian vault）时，笔记目录保持纯净，双向互通无缝。
- 索引是**缓存而非真相**；删除后能从 `.md` 文件全量重建（提供"重建索引"入口）。
- 默认目录 `notes/` 走 lazy ensure（首次解析时 `ensure_dir_canonical` 创建），`root_dir` 留 NULL 保持 `HA_DATA_DIR` 可迁移，完全复刻 project 默认 workspace 的处理。

---

## SQLite 索引 Schema

> 落在 `~/.hope-agent/knowledge/index.db`，连接模型仿 memory backend（1 写连接 + reader pool，WAL）。

```sql
CREATE TABLE kb (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  emoji TEXT,
  root_dir TEXT,                 -- NULL = 默认 notes/
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE note (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  kb_id TEXT NOT NULL,
  rel_path TEXT NOT NULL,
  title TEXT NOT NULL,
  frontmatter_json TEXT,
  mtime INTEGER NOT NULL,
  size INTEGER NOT NULL,
  embedding BLOB,
  embedding_signature TEXT,
  UNIQUE(kb_id, rel_path)
);
CREATE INDEX idx_note_kb ON note(kb_id);
CREATE INDEX idx_note_title ON note(kb_id, title);   -- [[Title]] 解析用

-- 全文检索（复用 memory 同款 fts5 配置）
CREATE VIRTUAL TABLE note_fts USING fts5(
  title, body,
  content='note', content_rowid='id',
  tokenize='unicode61'
);  -- body 由索引器写入（文件正文剥离 frontmatter）

CREATE TABLE note_link (
  src_note_id INTEGER NOT NULL,
  target_title TEXT NOT NULL,
  target_note_id INTEGER,        -- NULL = 悬空链接
  link_type TEXT NOT NULL,       -- 'wiki' | 'embed' | 'md'
  anchor TEXT
);
CREATE INDEX idx_link_src ON note_link(src_note_id);
CREATE INDEX idx_link_target ON note_link(target_note_id);   -- 反链查询

-- 向量检索（sqlite-vec，复用 memory 基建；维度随 embedding 模型）
-- CREATE VIRTUAL TABLE note_vec USING vec0(embedding float[N]);
```

向量检索的 RRF 融合 + MMR 直接复用 memory 的实现，本系统只换查询表。

---

## Wikilink 语法与解析

| 语法 | 阶段 | 说明 |
|---|---|---|
| `[[笔记标题]]` | Phase 1 | 基础双链 |
| `[[笔记标题\|别名]]` | Phase 1 | 显示别名，索引仍按标题解析 |
| `[[笔记#某标题]]` | Phase 1 | 跳转到 heading 锚点 |
| `#标签` | Phase 1 | 标签进 fts，支持 tag 过滤 |
| `![[笔记]]` 嵌入/transclusion | Phase 2 | 内容内联渲染 |
| `^block-id` 块引用 | Phase 3 | 需块级 ID 体系 |

- 语法兼容 Obsidian/Logseq，用户可直接导入现成 vault。
- **解析**：`parser.rs` 用 `pulldown-cmark` 走标准 Markdown，外加自定义扫描提取 `[[ ]]` / `#tag`。
- **解析（resolve）**：`resolver.rs` 把 `target_title` 映射到 `note_id`——优先精确标题匹配；同名歧义按**就近目录**或最近修改优先；无命中则 `target_note_id = NULL`（悬空）。
- **增量索引**：`watcher.rs`（`notify` crate）监听 `root_dir`，debounce 后对脏文件重解析（忽略 `.git` / `.obsidian` / `node_modules`）。我们自身的写操作也触发同一索引路径，并发 `notify` 回调去重。

---

## 后端模块与作用域

新增 `crates/ha-core/src/knowledge/`（零 Tauri 依赖，红线）：

```
knowledge/
  mod.rs           # 门面
  types.rs         # KnowledgeBase / Note / NoteLink
  db.rs            # index.db 读写（写连接 + reader pool，仿 memory backend）
  parser.rs        # Markdown + wikilink 解析（pulldown-cmark + 自定义 [[ ]] / #tag 扫描）
  index.rs         # 增量索引：文件变更 → 重解析 → 更新 note / note_link / fts / embedding
  watcher.rs       # notify 监听 root_dir（debounce，忽略 .git/.obsidian/node_modules）
  resolver.rs      # [[Title]] → note_id（标题索引 + 歧义就近）
  search.rs        # 复用 memory hybrid search（FTS5 + vec → RRF → MMR）
```

**WorkspaceScope 扩展**（关键安全点）：在 [`filesystem/workspace.rs`](../../crates/ha-core/src/filesystem/workspace.rs) 增加 `for_knowledge(kb_id)` 入口，把读写锁死在 KB 的 `root_dir` 内，完全复用现有 canonicalize + `starts_with` 闭合逻辑。写操作走 `resolve_writable`；HTTP 写端点继续受 `filesystem.allow_remote_writes` 闸门；preview-by-path 鉴权红线照旧（只放行 KB 目录内的路径，主机任意路径一律 403）。

---

## Agent 工具（读写桥）

新增 core 工具（均须 Tauri + HTTP 双适配，走 [`core_tools.rs`](../../crates/ha-core/src/tools/definitions/core_tools.rs) 定义 + dispatch）：

| 工具 | 作用 | 备注 |
|---|---|---|
| `note_create({kb, path, title, content})` | 新建笔记 | 写真实 `.md`，触发索引 |
| `note_update` / `note_append` | 编辑/追加 | 同上 |
| `note_read({kb, path \| title})` | 读笔记 | 返回完整原文 + 出链/反链列表 |
| `note_search({query, kb?})` | 混合检索召回 | **读取桥**；Phase 1 独立工具（见 P3） |
| `note_link({from, to})` | 在笔记间建链 | 写 `[[ ]]` 并更新索引 |

后台：扩展 dreaming pipeline，新增"提炼笔记 / 更新 MOC"分支（**写入桥**，见下）。

---

## AI 双向桥

### 写入桥：记忆 → 可读笔记

扩展 [Dreaming](memory.md) pipeline：在现有"给记忆打分 → 提炼 → 写日记"基础上，新增一条分支——把同主题的碎片 memory 聚合提炼成结构化笔记（MOC 索引页 / 主题笔记），写入用户指定的知识库。这就是 D1 里"把记忆整理写入笔记形成可读层"的落点。复用 `side_query` 低成本推理，复用 dreaming 的 idle/cron 触发与 `runtime_lock` primary 门控。

### 读取桥：笔记 → 上下文

笔记索引进 FTS5 + 向量后，agent 通过 `note_search` 按需召回（**不**默认全量注入 system prompt，避免上下文膨胀）。Phase 1 为独立工具；与 `recall_memory` 的融合检索放 Phase 3 评估（见 P3）。

形成闭环：**对话 → 记忆 → 笔记 → 召回喂回上下文**。

---

## 前端 UI

- **一级导航新增「知识库」Tab**（与聊天 / Dashboard 平级）。
- 笔记列表 / 目录树 + 复用现有 [`FilePreviewPane`](../../src/components/chat/project/file-browser/FilePreviewPane.tsx) 的 Markdown 渲染（Render / Source 切换已有）。
- **MVP 重点：Backlinks 面板**——在笔记预览侧显示"链接到本页的笔记"，并对悬空链接给出"新建该笔记"提示。
- 编辑器：Phase 1 复用现有 Markdown 编辑能力 + `[[` 自动补全；富文本编辑器（Tiptap/Milkdown）Phase 2 评估。
- 图谱视图：Phase 2/3，用 `react-force-graph`，数据源直接来自 `note_link` 表。
- 所有新 invoke 走 [`transport.ts`](../../src/lib/transport.ts) 双适配；i18n 12 语言齐全；Tooltip 用 `@/components/ui/tooltip`；保存按钮三态。

---

## 跨端契约对齐

push 前必须满足（来自 [AGENTS.md](../../AGENTS.md)）：

- ✅ 核心逻辑全进 `ha-core`（零 Tauri 依赖），`src-tauri` / `ha-server` 只做薄壳。
- ✅ 新 Tauri 命令进 `invoke_handler!`；新 HTTP 路由进 [`router.rs`](../../crates/ha-server/src/router.rs)；同步 [`api-reference.md`](api-reference.md)。
- ✅ 前端新 invoke 同时实现 Tauri + HTTP 两套适配。
- ✅ KB 配置走 config contract（`cached_config()` / `mutate_config`）；GUI + `ha-settings` 技能双入口零偏差（知识库偏好属 LOW/MEDIUM 风险）。
- ✅ 日志用 `app_info!` 等；核心路径（索引、解析、检索、AI 提炼）埋点。
- ✅ 新增架构能力 → 本文 + 登记 [`docs/README.md`](../README.md)；落地时 `CHANGELOG.md`（单行用户视角）+ `AGENTS.md`（契约面）补充。

---

## 分阶段路线图

### Phase 1（双链地基，对应 D4 选定的 MVP）

1. KB 概念 + `index.db` schema + `WorkspaceScope::for_knowledge`。
2. `notify` watcher + 增量索引（`note` + `note_link` 表）。
3. Wikilink 解析（`[[ ]]` / 别名 / `#heading` / `#tag`）+ 反链查询。
4. 前端「知识库」Tab + 笔记 CRUD + **Backlinks 面板** + 悬空链接提示。
5. `note_create / read / update / search / link` 工具（agent 能读写）。

### Phase 2

- 图谱视图（`react-force-graph`）。
- `![[ ]]` 嵌入 / transclusion。
- Dreaming → MOC 写入桥。
- `[[` 自动补全；富文本编辑器评估。
- **外部目录绑定（Obsidian vault 互通，P2）**。

### Phase 3

- 块级引用 `^block-id`。
- Canvas 知识白板。
- 笔记 ↔ memory 深度召回融合（P3）。

---

## 安全约束

- **作用域闭合**：所有读写经 `WorkspaceScope::for_knowledge`，canonicalize + `starts_with` 失败即拒，禁止越出 `root_dir`。
- **远端写门控**：HTTP `/api/knowledge/*` 写端点受 `filesystem.allow_remote_writes`（默认 false）闸门；桌面 Tauri 不受限。
- **preview-by-path 红线**：HTTP 按路径取笔记内容只放行 KB 目录内路径，主机任意路径一律 403（= 远程任意文件读防护）。
- **索引不含敏感凭据**：`index.db` 只存笔记结构/向量，不存任何 API Key / Token。
- **无痕互斥**：与现有 incognito 语义一致——无痕会话的 AI 写入桥不落知识库（守"关闭即焚"）。

---

## 关联文档

- [Project 系统](project.md)——「文件即真实文件」哲学、`working_dir` 解析链、`WorkspaceScope` 三入口
- [记忆系统](memory.md)——FTS5 + vec 混合检索、Dreaming、Embedding 基建（知识库复用）
- [文件操作统一](file-operations.md)——文件预览面板、preview-by-path 鉴权
- [配置系统](config-system.md)——`cached_config` / `mutate_config` 写契约
- [Side Query](side-query.md)——AI 提炼笔记的低成本推理入口
- [API 参考](api-reference.md)——新增 Tauri ↔ HTTP 接口须同步登记

---

## 文件清单（规划）

> 以下为 Phase 1 预计新增/改动的文件，落地后转为真实链接。

| 路径 | 类型 | 说明 |
|---|---|---|
| `crates/ha-core/src/knowledge/` | 新增模块 | 核心逻辑（types/db/parser/index/watcher/resolver/search） |
| `crates/ha-core/src/filesystem/workspace.rs` | 改动 | 增 `for_knowledge` 作用域入口 |
| `crates/ha-core/src/tools/definitions/core_tools.rs` | 改动 | 注册 `note_*` 工具 |
| `crates/ha-core/src/paths.rs` | 改动 | 集中 `knowledge/` 路径 |
| `src-tauri/src/commands/` + `invoke_handler!` | 改动 | KB Tauri 命令薄壳 |
| `crates/ha-server/src/routes/` + `router.rs` | 改动 | `/api/knowledge/*` HTTP 路由 |
| `src/components/knowledge/` | 新增 | 知识库 Tab、笔记列表、Backlinks 面板 |
| `src/lib/transport*.ts` | 改动 | KB invoke 双适配 |
| `docs/architecture/api-reference.md` | 改动 | 新接口对照登记 |
