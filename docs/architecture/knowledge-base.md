# Knowledge Base 知识库系统架构（设计草案）

> 返回 [文档索引](../README.md) | 状态：**设计草案 v3 契约修订（Draft，尚未实现）** | 创建时间：2026-06-02 | 修订：2026-06-03（v2：拆 registry/index、补 KB 访问作用域、收紧预览鉴权、chunk 级检索、外部只读统一；v3：source-aware 访问上下文、两鉴权平面、明文索引措辞、note_tag、向量单存 note_vec、attach FK/cascade、access 叠加公式、清旧文案残留）

> ⚠️ 本文是**设计契约文档**，不是已落地子系统的描述。它先于实现存在，用于锁定方向、记录取舍、指导分阶段迭代。每次方案打磨都应回到本文更新「决策账本」与「路线图」，保持单一真相源。代码落地后，本文逐步转为实现描述，并把 `规划中` 的源码路径替换为真实链接。

> 📛 **命名约定（D5）**：本文是技术文档，沿用技术名 **Knowledge Base（代码模块 `knowledge`、工具 `note_*`、作用域 `for_knowledge`）**。**对外功能名 = 「知识空间 / Knowledge Space」**，**营销定位语可打「第二大脑 / Second Brain」**。三者解耦，各自独立可调。

## 目录

- [背景与动机](#背景与动机)
- [设计目标与非目标](#设计目标与非目标)
- [核心定位：第四种知识 Scope](#核心定位第四种知识-scope)
- [决策账本](#决策账本)
- [数据模型](#数据模型)
- [磁盘布局](#磁盘布局)
- [SQLite 索引 Schema](#sqlite-索引-schema)
- [Wikilink 语法与解析](#wikilink-语法与解析)
- [与 Obsidian / Logseq 兼容性](#与-obsidian--logseq-兼容性)
- [后端模块与作用域](#后端模块与作用域)
- [外部目录绑定（Obsidian/Logseq vault）](#外部目录绑定obsidianlogseq-vault)
- [KB 访问作用域与预览鉴权](#kb-访问作用域与预览鉴权)
- [AI 知识操作（完整读写 + 检索 + 自主维护）](#ai-知识操作完整读写--检索--自主维护)
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
3. **AI 第一公民级读写 + 检索（不是事后插件）**：agent 对知识库有完整的 CRUD / 链接 / 图谱 / 检索能力，并能自主维护知识网络（自动建链、MOC、去重、缺口检测）；记忆系统可把碎片提炼成结构化笔记（"可读层"）。详见 [AI 知识操作](#ai-知识操作完整读写--检索--自主维护)。这是本系统区别于 Obsidian/Logseq 的核心——别人的 AI 是插件，我们的 agent 是知识库第一公民。
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
| D5 | 对外命名（品牌） | **功能名 = 「知识空间 / Knowledge Space」**；**营销定位语可打「第二大脑 / Second Brain」**（slogan，非功能本名）；**代码内部保持中性**——模块 `knowledge/`、工具 `note_*`、作用域 `for_knowledge` 不变 | 「知识库」在中文被 RAG / 客服知识库语义占领，易误读成被动静态存储；「笔记」撑不起双链+图谱+AI 自主维护的体量；「空间」开放、非 RAG、可 i18n。功能名中性精确不误导，营销借「第二大脑」高认知度拉心智。**三层解耦**（代码标识符 / 功能展示名 / 营销 slogan），各自可独立低风险调整 |
| D6 | 外部目录绑定（Obsidian/Logseq 互通） | **切「只读」一刀**：Phase 1 = 内部 `notes/` 完整读写 **+ 外部 vault 只读绑定**（索引/双链/反链/搜索/AI 读，AI 与工具对外部 root 的写一律禁用）；Phase 2 放开 AI 写外部（带冲突检测）+ 忽略规则配置 UI + 大库索引进度打磨 | A 纯内部（外部整体 Phase 2）/ C 全功能外部读写 Phase 1 | 外部绑定是最大获客杠杆（"指向你现成的 Obsidian vault，AI 瞬间点亮"）。读外部的成本（watcher/reconcile/大库索引/忽略规则/安全 review）本就在关键路径；**写外部的写冲突/lost-update 是回归风险最高的部分**，只读切法把它隔离到 Phase 2，早拿 demo 又不背最毒的债。详见 [外部目录绑定](#外部目录绑定obsidianlogseq-vault) |
| D7 | 召回形态（笔记 vs 记忆） | 笔记检索是**独立通道**，Phase 1 出独立工具 `note_search`，**绝不折进 `recall_memory`**；笔记走自己完整的检索逻辑（向量化 + 图谱感知），知识库检索做成旗舰最强。聊天内 `[[ ]]` 确定性引用注入提到 **Phase 1**。若日后要「一次拿记忆+笔记」，**加薄编排工具**分别查两 store 再 store-aware 合并，仍不动 `recall_memory` | 折进 `recall_memory` 一次拿全 | 记忆=一句话事实、笔记=整篇文档，性质/作用域/排序不可比，混排会污染成熟 memory 路径。**影响方向反过来**：知识库检索做强后，反哺给记忆系统借鉴，而非笔记降格塞进记忆 |
| D8 | 文档优先 vs 大纲优先 | **文档优先打底（对齐 Obsidian）**：一篇笔记 = 一段自由 Markdown，数据模型 `Note{title, body, frontmatter}`，Phase 1 复用现有 Markdown 编辑/渲染栈。对 Logseq 做文件级 + 公共语法子集互通（能索引/双链/搜索/AI 读，但编辑形态是文档非大纲）。**原生大纲（block 树 + `((块引用))`）作 Phase 3 可选层**，不永久放弃 | 一开始就做 Logseq 式大纲优先 | Obsidian（文档优先）与 Logseq（大纲优先）数据模型从根上不同（文本 vs 带 ID 的块树），无法一套实现原生兼容两者；文档优先覆盖人群最广、顺现有 Markdown 栈、Phase 1 最轻。详见 [兼容性](#与-obsidian--logseq-兼容性) |

#### v2 契约修订（来自第一轮 review，闭合契约缺口）

| # | 决策点 | 结论 | 理由 |
|---|---|---|---|
| D9 | KB registry 真相源 | **KB 注册表落 `sessions.db` 的 `knowledge_bases` 表**（与 `projects` 表并排）；`~/.hope-agent/knowledge/index.db` **只存可重建索引缓存**（note/chunk/link/fts/vec） | KB 是一级关系实体（有列表/归档/绑定/统计/权限/attach），不是轻量偏好。放 `config.json` 会变成大对象并发写、跨进程 stale、关系难查。修复原草案"索引可删但又存唯一真相"的自相矛盾——删 `index.db` 后必须能全量重建 |
| D10 | KB 访问作用域 | **默认 deny + 显式 attach**：普通 session 默认无 KB 访问，用户 attach 后才可 `note_search/read`；project 可 attach、项目内 session 继承；session attach 可叠加 project attach，但**当前生效 KB 必须 UI 可见列出**；**incognito 强制零访问/零写/零被动召回**；**IM Phase 1 一律禁用 KB 访问**（即便有 project/session attach），Phase 2 才开 account/chat 级显式 opt-in，群聊单独确认 | KB 不能像 memory 那样默认全局可见，否则工作 vault / 私人 vault / IM 会话互相泄漏。唯一入口 `effective_kb_access(KnowledgeAccessContext { session_id, source, channel_info? })`——**必须带调用来源 `source ∈ Gui\|HttpUi\|AgentTool\|IM\|Cron\|Subagent`**：同一 session 可被 GUI 与 IM 共用/接管，仅凭 session_id 判不出本次是否 IM turn，IM 红线就执行不了。`note_search(kb?)` 省略 `kb` 时只搜可访问集合，绝不搜全局。详见 [KB 访问作用域](#kb-访问作用域与预览鉴权) |
| D11 | 外部 vault Phase 1 可写性 | **外部 root Phase 1 彻底只读**——AI 不写、GUI 也不写，写入口统一拒绝并 UI 显示只读；内部 `notes/` 完整读写。GUI 写外部 + `resolve_writable(actor=user\|agent)` 拆分 + mtime/hash 冲突检测整体推 Phase 2 | 原草案"GUI 可写外部 Phase 1"与"冲突检测 Phase 2"自相矛盾，正好踩 lost-update。**Phase 1 价值是「点亮老 vault」不是「托管老 vault」**，果断只读，避免提前付清冲突检测/原子写/半写/三方 rename 噪声全套 |
| D12 | 检索粒度 | **Phase 1 即上 chunk 级**：`note` 只存文件级元数据；新增 `note_chunk(note_id, chunk_index, heading_path, body, start_offset, end_offset, content_hash, embedding_signature)`；FTS5 external-content 与 vec 都建在 chunk 上；检索返回 chunk hits 再聚合回 note（带命中片段 + heading 定位） | 整篇 note 一个 embedding 会在日报/会议纪要/长文剪藏上失效（超 embedding 上限 + 命中整篇却定位不到段落）。先做 note 级、后迁 chunk 级要改 schema/检索/UI hit 展示/工具返回结构，更疼。`content_hash` 支持按 chunk 增量 re-embedding 省成本 |

### 待定决策

> **全部待定项已拍板**：P1→D5（命名）、P2→D6（外部绑定）、P3→D7（召回形态）、P4→D8（文档优先）；v2 review 又闭合 D9（registry）/D10（访问作用域）/D11（外部只读）/D12（chunk 检索）。设计契约定型，进入实现阶段。后续如需新增取舍，在此另起 P5…。

---

## 数据模型

> 类型规划落在 `crates/ha-core/src/knowledge/types.rs`（规划中）。
>
> **两类存储分明（D9）**：`KnowledgeBase` 及其访问绑定是 **`sessions.db` 真相源**；`Note / NoteChunk / NoteLink` 是 **`index.db` 可重建缓存**（从 `.md` 全量扫盘可重建，连 `rel_path` 都是缓存）。

### KnowledgeBase（真相源，`sessions.db`）

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `String` | UUID v4 主键 |
| `name` | `String` | 知识库名称（trim 后非空） |
| `emoji` | `Option<String>` | 侧边栏前缀 |
| `root_dir` | `Option<String>` | 笔记根目录绝对路径。`NULL` = 用默认 `~/.hope-agent/knowledge/{id}/notes/`（lazy ensure，仿 project workspace）。**非 NULL = 绑定外部目录（如 Obsidian vault）**，Phase 1 **只读**（D11） |
| `archived` | `bool` | 归档标记（仿 project） |
| `created_at` / `updated_at` | `String` | ISO8601 |

> 为什么不放 `index.db`：`name / emoji / root_dir` 无法从 `.md` 文件重建，删索引即丢——必须随真相源持久化（D9）。

### KB 访问绑定（真相源，`sessions.db`，D10）

- `session_knowledge_bases(session_id, kb_id, access)` — session 显式 attach（`access ∈ read | write`）
- `project_knowledge_bases(project_id, kb_id, access)` — project attach，项目内 session 继承

详见 [KB 访问作用域](#kb-访问作用域与预览鉴权)。

### Note（索引缓存行，真相在文件）

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `i64` | 自增主键（索引内部用） |
| `kb_id` | `String` | 所属知识库 |
| `rel_path` | `String` | 相对 `root_dir` 的路径（缓存，可重建） |
| `title` | `String` | 取自 frontmatter `title` > 首个 H1 > 文件名（去扩展名） |
| `frontmatter_json` | `Option<String>` | YAML frontmatter 解析后的 JSON |
| `mtime` / `size` | `i64` | 文件修改时间 / 字节数，增量索引判脏用 |

> Note 行**不再直接挂 embedding/fts**——正文检索全下沉到 `NoteChunk`（D12）。

### NoteChunk（chunk 级检索单元，`index.db`，D12）

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `i64` | 自增主键（= fts/vec 的 rowid） |
| `note_id` | `i64` | 所属笔记 |
| `chunk_index` | `i64` | 笔记内序号 |
| `heading_path` | `Option<String>` | 所在 heading 路径（如 `参数 > 研磨`），命中定位 + `[[note#heading]]` 锚定 |
| `body` | `TEXT` | chunk 正文（剥 frontmatter；FTS external-content 取此列） |
| `start_offset` / `end_offset` | `i64` | 在文件内的**字符偏移**（UTF-8 安全，不用字节偏移——守 hope-agent「禁字节切片字符串」红线），用于命中跳转 / 以后 `^block` 锚定 |
| `content_hash` | `TEXT` | chunk 内容 hash，**按 chunk 增量 re-embedding**（只重嵌变更段） |
| `embedding_signature` | `Option<String>` | 产出该向量的 embedding 模型签名；换模型时识别需重嵌的 chunk |

> **向量单一存放（#6）**：chunk 向量**只存 `note_vec`**（sqlite-vec vec0，rowid = `note_chunk.id`，复用 memory `EmbeddingProvider` + `embedding_cache`）；`note_chunk` 行内**不**再存 `embedding BLOB`，避免两套并存。index.db 可从文件全量重建，无需行内备份向量。

> chunking 策略 Phase 1 保持简单：按 heading 分段 + 大小封顶（+ 少量 overlap）。检索：chunk 级 FTS+vec → RRF/MMR → **聚合回 note**（取 best-chunk 分），返回 note + 命中 chunk snippet + heading 定位。

### NoteLink（双链边，MVP 核心）

| 字段 | 类型 | 说明 |
|---|---|---|
| `src_note_id` | `i64` | 出链来源笔记 |
| `target_ref` | `String` | `[[ ]]` 内的原文目标（标题或 `folder/note` 路径式） |
| `target_note_id` | `Option<i64>` | resolve 命中的目标笔记；`NULL` = **悬空链接（broken link）**，前端高亮提示可新建 |
| `link_type` | `TEXT` | `wiki`（`[[ ]]`）/ `embed`（`![[ ]]`，Phase 2）/ `md`（标准 `[]()`） |
| `anchor` | `Option<String>` | `[[Note#Heading]]` 的 heading slug，或 `^block-id`（Phase 3） |

**反向链接** = `SELECT * FROM note_link WHERE target_note_id = ?`，一个索引即可，无需独立表。

---

## 磁盘布局

```
~/.hope-agent/
  sessions.db                     # 真相源：knowledge_bases + session/project_knowledge_bases（与 projects 表同库，D9）
  knowledge/
    index.db                      # 🆕 纯可重建索引缓存（note/note_chunk/note_link/fts/vec），从不污染笔记目录
    {kb_id}/
      notes/                      # 默认笔记目录（root_dir 为 NULL 时 lazy ensure）
        Zettelkasten/...
        每日笔记/2026-06-02.md
        ...
```

关键设计：

- **真相 / 缓存分家（D9）**：KB 注册表 + 访问绑定在 `sessions.db`（关系型、与 `projects` 同库，可查 attach/统计/归档）；`~/.hope-agent/knowledge/index.db` **只存可重建缓存**（note/chunk/link/fts/vec），带 `kb_id` 列区分多个 KB。
- **索引是缓存而非真相**：删 `index.db` 后能从 `.md` 文件 + `sessions.db` 的 registry **全量重建**（提供"重建索引"入口）。**绝不写进笔记目录**——KB 绑定外部 vault 时笔记目录保持纯净，双向互通无缝。
- 默认目录 `notes/` 走 lazy ensure（首次解析时 `ensure_dir_canonical` 创建），`root_dir` 留 NULL 保持 `HA_DATA_DIR` 可迁移，完全复刻 project 默认 workspace 的处理。
- `root_dir` **非 NULL = 绑定外部目录**（如现成 Obsidian/Logseq vault）。Phase 1 外部 root **只读**，Phase 2 放开 AI 写，详见 [外部目录绑定](#外部目录绑定obsidianlogseq-vault)（D6）。

---

## SQLite 索引 Schema

分两库：**`sessions.db` 存真相（registry + 访问绑定）**，**`index.db` 存可重建缓存**（连接模型仿 memory backend：1 写连接 + reader pool，WAL）。

### A. 真相源 —— `sessions.db`（D9 / D10）

```sql
-- KB 注册表（与 projects 表并排，同库）
CREATE TABLE knowledge_bases (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  emoji TEXT,
  root_dir TEXT,                 -- NULL = 默认 ~/.hope-agent/knowledge/{id}/notes/；非 NULL = 外部绑定（Phase 1 只读）
  archived INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

-- 访问绑定（默认 deny：无行 = 无访问）
CREATE TABLE session_knowledge_bases (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  kb_id TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
  access TEXT NOT NULL DEFAULT 'read' CHECK (access IN ('read','write')),
  PRIMARY KEY (session_id, kb_id)
);
CREATE TABLE project_knowledge_bases (
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  kb_id TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
  access TEXT NOT NULL DEFAULT 'read' CHECK (access IN ('read','write')),
  PRIMARY KEY (project_id, kb_id)
);
```

> **约束与级联（#7）**：FK + `ON DELETE CASCADE` + `CHECK(access IN ...)` 写全，避免 KB/会话/项目删除后留孤儿 attach 行（= 潜在泄漏）。SQLite FK 需 per-connection `PRAGMA foreign_keys=ON`；与现有 `sessions.db` 删除多走**代码手动级联**（见 [Project 删除三步](project.md)）的约定对齐——KB/项目/会话删除路径里**一并显式清 attach**，FK 作双保险。

### B. 可重建缓存 —— `index.db`（D12 chunk 级）

```sql
-- 笔记文件级元数据（不挂 embedding/fts）
CREATE TABLE note (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  kb_id TEXT NOT NULL,
  rel_path TEXT NOT NULL,
  title TEXT NOT NULL,
  frontmatter_json TEXT,
  mtime INTEGER NOT NULL,
  size INTEGER NOT NULL,
  UNIQUE(kb_id, rel_path)
);
CREATE INDEX idx_note_kb ON note(kb_id);
CREATE INDEX idx_note_title ON note(kb_id, title);   -- [[Title]] resolve 用

-- chunk 级检索单元（FTS / vec 都建在 chunk 上）
CREATE TABLE note_chunk (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  note_id INTEGER NOT NULL,
  chunk_index INTEGER NOT NULL,
  heading_path TEXT,
  body TEXT NOT NULL,                       -- FTS external-content 取此列（列名对齐）
  start_offset INTEGER NOT NULL,            -- 字符偏移（UTF-8 安全）
  end_offset INTEGER NOT NULL,
  content_hash TEXT NOT NULL,               -- 按 chunk 增量 re-embedding
  embedding_signature TEXT
);
CREATE INDEX idx_chunk_note ON note_chunk(note_id);

-- 全文检索：external-content 指向 note_chunk（列名必须存在于内容表，故建在 chunk.body 上，修复 v1 的 body 列缺失）
CREATE VIRTUAL TABLE note_chunk_fts USING fts5(
  body,
  content='note_chunk', content_rowid='id',
  tokenize='unicode61'
);  -- 由索引器经 AFTER INSERT/UPDATE/DELETE 触发器同步

-- 向量检索：建在 chunk 上（sqlite-vec，维度随 embedding 模型）
-- CREATE VIRTUAL TABLE note_vec USING vec0(embedding float[N]);   -- rowid = note_chunk.id

CREATE TABLE note_link (
  src_note_id INTEGER NOT NULL,
  target_ref TEXT NOT NULL,      -- 原文：标题或 folder/note 路径式
  target_note_id INTEGER,        -- NULL = 悬空链接
  link_type TEXT NOT NULL,       -- 'wiki' | 'embed' | 'md'
  anchor TEXT                    -- heading slug 或 ^block-id（Phase 3）
);
CREATE INDEX idx_link_src ON note_link(src_note_id);
CREATE INDEX idx_link_target ON note_link(target_note_id);   -- 反链查询

-- 标签索引（#5；从 frontmatter tags + 正文 #tag 抽取，缓存可重建）
CREATE TABLE note_tag (
  note_id INTEGER NOT NULL,
  tag TEXT NOT NULL,                        -- 归一化后（NFC + 小写）
  PRIMARY KEY (note_id, tag)
);
CREATE INDEX idx_tag ON note_tag(tag);      -- note_by_tag / note_tags 用
```

**检索流程（D12）**：query → chunk 级 FTS5(BM25) + vec ANN → RRF 融合（**算法复用** memory，独立 store）→ MMR 去冗 → **聚合回 note**（best-chunk 分代表该 note）→ 返回 note + 命中 chunk snippet + `heading_path` 定位。

---

## Wikilink 语法与解析

| 语法 | 阶段 | 说明 |
|---|---|---|
| `[[笔记标题]]` | Phase 1 | 基础双链 |
| `[[folder/note]]` 路径式 | Phase 1 | 精确路径定位，歧义最少（resolve 优先级最高） |
| `[[笔记标题\|别名]]` | Phase 1 | 显示别名，索引仍按目标解析 |
| `[[笔记#某标题]]` | Phase 1 | 跳转到 heading 锚点 |
| `#标签` | Phase 1 | 标签进 fts，支持 tag 过滤 |
| `![[笔记]]` 嵌入/transclusion | Phase 2 | 内容内联渲染 |
| `^block-id` 块引用 | Phase 3 | 需块级 ID 体系 |

- 语法兼容 Obsidian/Logseq，用户可直接导入现成 vault。
- **扫描**：`parser.rs` 用 `pulldown-cmark` 走标准 Markdown，自定义扫描提取 `[[ ]]` / `#tag`；**必须跳过代码块（fenced/indented）与 inline code 内的 `[[ ]]`**，避免把代码里的内容误当链接。
- **resolve（确定性，#8）**：`resolver.rs` 把 `target_ref` 映射到 `note_id`，规则**全程确定、不依赖 mtime**（避免链接随时间漂移）：
  1. **路径式优先**：`[[folder/note]]` 精确路径匹配。
  2. **唯一 basename**：`[[note]]` 在去歧义后唯一命中。
  3. **同名歧义稳定规则**：多个同名 → **最短路径优先，再字典序路径**（绝不用 mtime / 最近修改）。
  4. 无命中 → `target_note_id = NULL`（悬空），前端提示新建。
  - **归一化**：标题/路径做 NFC Unicode 归一化 + 大小写不敏感匹配（对齐 Obsidian 默认；显示保留原文）。
  - **heading 锚点**：`[[note#标题]]` 的 heading 走稳定 slug 规则（与 `note_chunk.heading_path` 对齐）。
- **增量索引**：`watcher.rs`（`notify` crate）监听 `root_dir`，debounce 后对脏文件重解析（忽略 `.git` / `.obsidian` / `logseq` / `.trash` / `node_modules`）。我们自身的写操作也触发同一索引路径，并发 `notify` 回调去重。

---

## 与 Obsidian / Logseq 兼容性

> **结论先行**：目标是「**文件级 + 主流语法子集 + 非破坏性共存**」，不是「功能完全等价」。需诚实指出：Obsidian 与 Logseq **彼此都不完全兼容**（一个文档优先、一个大纲优先），所以「同时与两者 100% 兼容」物理上不成立。我们能做到、也最有价值的是：**用户能用同一个文件夹，既用 Hope Agent 知识库、又用 Obsidian/Logseq 打开，互不破坏。**

兼容性三层次：

1. **文件级（强保证）**：笔记是标准 `.md`，永不锁定、永不破坏性转写；`.obsidian/`、`logseq/`、`.git/` 等配置目录一律忽略不碰。三方可同时打开同一文件夹。
2. **语法子集（核心保证）**：`[[wikilink]]`、`[[link|别名]]`、`[[link#heading]]`、`#tag`、YAML frontmatter —— 这是 Obsidian 与 Logseq 的**公共子集**，我们读写都用它，保证「我们写的两边都认、两边写的我们也认」。
3. **专有语法（尽力读、谨慎写）**：各家专有特性尽量「读得懂」，但默认不主动产出，避免污染对方。

### 兼容性矩阵

| 特性 | Obsidian | Logseq | Hope Agent KB |
|---|---|---|---|
| 标准 `.md` 文件 | ✅ | ✅（也支持 org） | ✅ 真相源，不转写 |
| `[[wikilink]]` / 别名 | ✅（`\|`） | ✅ | ✅ Phase 1 |
| `[[link#heading]]` | ✅ | ✅ | ✅ Phase 1 |
| `#tag` | ✅ | ✅（tag≈page） | ✅ Phase 1 |
| YAML frontmatter | ✅ | ✅（也用 `key:: value`） | ✅ 读写 frontmatter；`key::` 属性 Phase 2 读 |
| `![[嵌入]]` transclusion | ✅ | ✅ | ✅ Phase 2 |
| 块引用 | `^block-id` | `((block-uuid))` + `id::` | 两种皆读（Phase 3）；写跟随绑定目录风格 |
| 大纲（每行即 block） | ✗（文档优先） | ✅（大纲优先） | ⚠️ 默认文档优先，见 D8 |
| Callout `> [!note]` | ✅ | 部分 | ✅ 原样保留（标准 md 引用块） |
| Canvas | `.canvas`（JSON） | 白板 | 复用本项目 Canvas 子系统，Phase 3 评估 |
| 配置目录 | `.obsidian/` | `logseq/` | 忽略不碰 |

### 关键架构分叉（D8）

Obsidian 是**文档优先**（自由 markdown），Logseq 是**大纲优先**（每行一个带缩进层级的 block，块引用 `((uuid))`）。两种模型数据结构不同，无法一套实现「原生」同时满足。**默认取向**：以文档优先为基座（对齐 Obsidian），对 Logseq 做文件级 + 公共语法子集互通；深度大纲语义（block 树、`((block-ref))`）作为 Phase 3 可选项。

## 后端模块与作用域

新增 `crates/ha-core/src/knowledge/`（零 Tauri 依赖，红线）：

```
knowledge/
  mod.rs           # 门面
  types.rs         # KnowledgeBase / Note / NoteChunk / NoteLink
  registry.rs      # KB CRUD + 访问绑定（sessions.db 真相源，D9）
  access.rs        # KnowledgeAccessContext{session,source,channel} / effective_kb_access(ctx)（D10，source-aware）
  db.rs            # index.db 读写（写连接 + reader pool，仿 memory backend）
  parser.rs        # Markdown + wikilink 扫描（pulldown-cmark + [[ ]] / #tag，跳过 code）
  chunker.rs       # 按 heading 分段 + 封顶（D12），产出 NoteChunk（含字符 offset / content_hash）
  index.rs         # 增量索引：文件变更 → 重解析 → 更新 note / note_chunk / note_link / fts / vec
  watcher.rs       # notify 监听 root_dir（debounce，忽略 .git/.obsidian/logseq/.trash/node_modules）
  resolver.rs      # [[ref]] → note_id（确定性规则，#8，不用 mtime）
  search.rs        # chunk 级 hybrid search（FTS5 + vec → RRF → MMR → 聚合回 note）
```

**WorkspaceScope 扩展**：在 [`filesystem/workspace.rs`](../../crates/ha-core/src/filesystem/workspace.rs) 增加 `for_knowledge(kb_id)` 入口，把读写锁死在 KB 的 `root_dir` 内，完全复用现有 canonicalize + `starts_with` 闭合逻辑。

- **写门控（D11）**：`resolve_writable` 对**外部绑定 root** Phase 1 一律拒绝（AI / GUI 皆不可写）；内部 `notes/` 正常可写。HTTP 写端点叠加 `filesystem.allow_remote_writes` 闸门。
- **preview-by-path 不再走"路径命中任意 KB root"**（那会击穿会话鉴权）：KB 文件读取走**独立的 KB 端点**，鉴权按 `effective_kb_access`（caller 对该 `kb_id` 有读权限），与现有 `/api/sessions/{id}/files/*` 端点**完全隔离、互不放宽**。详见 [KB 访问作用域与预览鉴权](#kb-访问作用域与预览鉴权)。

---

## 外部目录绑定（Obsidian/Logseq vault）

> 决策 D6：知识库的 `root_dir` 可指向用户**现成的外部目录**（如 Obsidian/Logseq vault），实现「指向你多年积累的 vault，AI 瞬间点亮它」——本系统最大的获客杠杆。Schema 第一天预留 `root_dir: Option`，**无迁移债**。

### 「只读」一刀（Phase 1 vs Phase 2）

两种 root 的本质差异：内部 `notes/` **只有我们一个写者**（写时同步索引、零写冲突）；外部 vault 被 Obsidian / Logseq / git / iCloud·Dropbox·Syncthing / 文本编辑器**多方并发改**。据此切一刀：

| 能力 | 内部 `notes/` | 外部绑定 root |
|---|---|---|
| 索引 / 双链 / 反链 / 搜索 / AI 读 | ✅ Phase 1 | ✅ Phase 1 |
| 用户在 GUI 内编辑 | ✅ Phase 1 | ⛔ Phase 1 只读 → ✅ **Phase 2**（带冲突检测） |
| **AI / 工具自动写**（`note_create/update/patch/...`、Layer 2 提案落盘） | ✅ Phase 1 | ⛔ Phase 1 禁用 → ✅ **Phase 2**（带冲突检测） |

> **D11 收口**：原 v1 写"GUI 编辑外部 = Phase 1"与"冲突检测 = Phase 2"自相矛盾（正好踩 lost-update）。v2 统一为 **Phase 1 外部 root 彻底只读——AI 不写、GUI 也不写**，UI 明确显示只读。**Phase 1 价值是「点亮老 vault」不是「托管老 vault」**。

**判定入口**：`WorkspaceScope::for_knowledge` 解析时若 root 为外部绑定，Phase 1 `resolve_writable` 一律拒绝（不区分 actor）——把回归风险最高的**写冲突 / lost-update**整体隔离到 Phase 2（届时叠加 `resolve_writable(actor=user|agent)` 拆分 + mtime/hash 冲突检测）。Phase 1 仍拿到完整的"点亮老库"读体验。

### 必须在 Phase 1 付清的成本（读外部即需要）

1. **生产级 watcher**（`watcher.rs`）：扛同步工具批量重写（debounce + 批量 reindex）、编辑器 tmp+rename 原子保存噪声、半写文件（mtime 稳定后再索引）、外部删除/改名导致的反链失效。
2. **绑定 / 启动 reconcile**：bind 时与每次启动扫 mtime，增量重索引变更文件、prune 已删文件——外部 vault 可能在 App 未运行时被其它设备/同步改动。
3. **大库冷启动**：首次绑几千篇 = 全量解析 + 全量 embedding，走后台任务（复用 `async_jobs` / `local_model_jobs` 模式）+ 进度 UI + 断点续跑。
4. **忽略规则**：gitignore 风格，默认排除 `.obsidian/` `logseq/` `.git/` `.trash/` 附件目录 `node_modules/` 等，防 watcher 自我抖动 + 索引污染（**可配 UI 放 Phase 2**，Phase 1 用内置默认列表）。
5. **安全面收口**：绑外部目录后 KB 作用域**合法包含 `~/.hope-agent` 之外的主机路径**。**鉴权不走"路径命中任意 KB root"**（v2 已否掉该模型，会击穿会话鉴权）——KB 文件读取走**独立 KB 端点 + `effective_kb_access`（caller 对该 `kb_id` 有读权限）+ `WorkspaceScope` 容器校验（scope contains）**三重判定，详见 [KB 访问作用域与预览鉴权](#kb-访问作用域与预览鉴权)。桌面信任本机；**HTTP/远端绑外部主机路径属敏感场景**，写由 `allow_remote_writes` 兜，落地**专门走一遍安全 review**。

### 留给 Phase 2 的（写外部才付）

- AI 写外部的**写冲突检测**：写前比对 mtime，自上次读后被改则中止或落 `.conflict` 旁车；Layer 2 提案制 apply 时同样校验。
- 忽略规则配置 UI；大库索引进度的精细化打磨。

---

## KB 访问作用域与预览鉴权

> 决策 D10：KB **不能像 memory 那样默认全局可见**，否则工作 vault / 私人 vault / IM 会话互相泄漏。访问模型 = **默认 deny + 显式 attach**。

### KnowledgeAccessContext（D10，source-aware）

唯一入口必须**带调用来源**（#2）：

```
effective_kb_access(KnowledgeAccessContext {
    session_id,
    source,            // Gui | HttpUi | AgentTool | IM | Cron | Subagent（从 ToolExecContext 透传）
    channel_info?,     // IM 时的 account/chat
}) -> { kb_id: access }   // access ∈ read | write
```

**为什么必须带 source**：同一 session 可被 GUI 与 IM **共用/接管**（`channel_conversations` attach 模型），仅凭 `session_id` **判不出本次调用是不是 IM turn**——D10 的「IM Phase 1 零访问」红线就无法执行。

授予规则：

- **普通 session**：默认**无任何 KB 访问**；用户显式 attach（`session_knowledge_bases`）后才可 `note_search / note_read`。
- **project**：可 attach KB（`project_knowledge_bases`）；项目内 session **继承** project 的 KB。
- **叠加公式（#8）**：`granted = max(session_attach, project_attach)`（**最高权限胜出**，write > read），再做 **min-cap** 往下夹：
  - 外部绑定 root → 上限 `read`（D11，Phase 1）
  - `source = IM`（Phase 1）/ `incognito` → **0**（零访问/零写/零被动召回）
  - `source ∈ Cron | Subagent` → 按继承的 session 上下文，但不超过其 cap
  - 即：**写**需同时满足 `授予 write ∧ 内部 root ∧ source 允许 ∧ 非 incognito`
- **`note_search(kb?)` 省略 `kb`**：只搜 `effective_kb_access` 内的 KB，**绝不默认搜全局**；显式传 `kb` 也须过校验否则拒绝。
- **UI 可见**：当前生效的 KB（session ∪ project）**必须在界面明确列出**——防泄漏的最后人因防线。

### 两个鉴权平面（#3）

KB 文件的读取有**两类完全不同的权限主体**，分平面处理：

| 平面 | 主体 | 鉴权 | 用途 |
|---|---|---|---|
| **Owner / 管理平面** | 用户本人（KB owner） | 桌面=本机信任；**HTTP=持 API key 即 owner-equivalent** | 「知识空间」Tab 浏览/管理自己**所有** KB，**不走 session attach** |
| **Agent / session 平面** | turn 内的 agent/工具 | `effective_kb_access(ctx)`（session + source） | `note_search / note_read` 及 turn 内预览，受 attach + source 约束 |

> **API key = owner（写死）**：远端 HTTP 下持有 server API key 的调用者**等同 owner**，走管理平面拿全量 KB 访问。owner 平面不被 session attach 限制（attach 只约束 agent/session 平面）。

### KB 文件预览端点（#2，关键安全修订）

原 v1 把 preview-by-path 鉴权放宽成「路径命中任意已绑定 KB root」——这会让**任何 session 只要猜到路径就能读所有 KB 文件**，击穿现有「被会话引用 ∪ 落在会话工作目录」红线。收口：

- **不动**现有 `/api/sessions/{id}/files/{read,extract,by-path}` 的判定，一个字都不放宽。
- **新增独立 KB 端点** `GET /api/knowledge/{kb_id}/files/{read,raw,extract}`：管理平面（owner / API key）放行全部；若由 agent/session 上下文发起则叠加 `effective_kb_access(ctx)`。鉴权**而非**"路径落在某个 KB root 内"，并叠加 `WorkspaceScope` 容器校验（scope contains）。
- 两套端点、两套鉴权，互不污染；远端写仍叠加 `allow_remote_writes`；非授权 `kb_id` / 越界路径一律 403。

---

## AI 知识操作（完整读写 + 检索 + 自主维护）

本系统区别于 Obsidian/Logseq 的核心：别人的 AI 是事后插件，我们的 agent 对知识库有**第一公民级的完整读写与检索能力**，并能**自主维护**知识网络。能力分三层。所有工具均须 Tauri + HTTP 双适配，走 [`core_tools.rs`](../../crates/ha-core/src/tools/definitions/core_tools.rs) 定义 + dispatch。

### Layer 1 — 完整工具面（同步，agent 主动调用）

agent 在对话中可直接调用，覆盖 CRUD / 链接 / 图谱 / 检索 / 元数据 / 高阶知识操作。所有**写操作走统一权限引擎审批**、锁定在 `WorkspaceScope::for_knowledge` 内、emit `knowledge:changed` 事件。

**CRUD**

| 工具 | 作用 |
|---|---|
| `note_create({kb, path, title, content, frontmatter?, template?})` | 新建笔记（可套模板） |
| `note_read({kb, path\|title, include?})` | 读原文 + 出链 / 反链 / 标签 |
| `note_update({kb, path, content})` | 全量替换 |
| `note_patch({kb, path, old, new})` | 外科手术式局部编辑（仿现有 `edit` 工具） |
| `note_append({kb, path, content, section?})` | 追加（可指定 heading 下，适配每日笔记） |
| `note_delete({kb, path})` | 删除（Phase 1；只留悬空链接，不连带改其它文件） |
| `note_rename` / `note_move` | 改名 / 移动 — **Phase 2**（见下，多文件写） |

> **阶段归属（#9）**：Phase 1 = `create / read / update / patch / append / delete`。`note_rename` / `note_move` 要**批量改写所有指向它的 `[[ ]]`**（多文件连带写），与外部只读 + 链接完整性一起放 **Phase 2**。
>
> **访问校验（#4）**：所有 `note_*` 工具的 `kb` 参数都过 `effective_kb_access` 校验；`note_search` 省略 `kb` 时只搜可访问集合，不搜全局。

**链接与图谱**

| 工具 | 作用 |
|---|---|
| `note_link({from, to, alias?})` | 插入 `[[ ]]` |
| `note_backlinks({note})` | 谁链接到本页 |
| `note_graph({note, depth})` | N 跳邻域（nodes+edges），图谱视图与「关联阅读」数据源 |
| `note_broken_links({kb})` | 悬空链接清单 |
| `note_orphans({kb})` | 孤岛笔记（无任何链接） |

**检索**

| 工具 | 作用 |
|---|---|
| `note_search({query, kb?, filters?})` | FTS5 + 向量混合检索（复用 memory RRF/MMR） |
| `note_similar({note, k})` | 向量近邻（「更多类似」） |
| `note_related({note})` | 融合召回：反链 ∪ 向量近邻 ∪ 同标签（图谱感知） |
| `note_suggest_links({note})` | 给出**该笔记应建但还没建**的 `[[ ]]` 候选（自动织网按需版） |

**标签与元数据**

| 工具 | 作用 |
|---|---|
| `note_by_tag({tag})` / `note_tags({kb})` | 标签过滤 / 枚举 |
| `note_set_frontmatter({note, props})` | 读写 frontmatter 属性 |

**高阶知识操作（AI 原生）**

| 工具 | 作用 |
|---|---|
| `note_distill({source})` | 原始捕获 / 长文 → 原子永久笔记（BASB 的 CODE / Zettelkasten 拆分） |
| `note_moc({topic\|tag})` | 生成 / 刷新某主题的 MOC（Maps of Content）枢纽页 |
| `session_to_note({session_id})` | 把一段对话沉淀成结构化笔记 |

### Layer 2 — 自主维护（后台，提案制）

复用现有 dreaming / cron / idle 调度 + `runtime_lock` primary 门控 + `side_query` 低成本推理。**所有自主写入都是「提案制」，不静默改用户的库**——产出进审阅队列，经用户确认（或 YOLO/auto-approve 配置）后落盘，与 dreaming 提名 promotion、skills auto-review 模式一致。无痕会话不触发任何自主写入（守「关闭即焚」）。

自主任务：

- **自动建链**：扫描笔记，发现正文提到某个已有笔记的概念但没建 `[[ ]]` → 提议插入。
- **MOC 自动维护**：按主题 / 标签聚类笔记，生成 / 更新枢纽页。
- **自动打标签 / 补 frontmatter**：从内容推断标签与属性。
- **孤岛救援**：找出无链接笔记，提议接入相关网络。
- **去重合并**：检测近重复笔记（向量 + 标题），提议合并。
- **知识缺口**：反复出现的悬空链接 = 高需求但缺失的笔记 → 提议创建。
- **记忆 → 笔记（可读层）**：扩展 dreaming，把同主题碎片 memory 提炼成主题笔记 / MOC（D1 落点）。

### Layer 3 — 检索引擎（共享底座 + 三通道读取桥）

笔记检索是**独立旗舰**，走自己完整的检索逻辑（向量化 + 图谱感知），不与 memory 混排（D7）。Layer 1 工具与 Layer 2 任务共用同一检索底座：

- **混合检索**：FTS5 关键词 + sqlite-vec 向量 → RRF 融合 → MMR 去冗（**算法复用** [memory](memory.md) 实现，但**独立 store、独立排序**）。
- **图谱感知检索**：把链接结构（反链 / 共引）与语义相似度融合排序，做「关联阅读」。

**读取桥分三通道**（注入上下文的三条路，互不替代）：

| 通道 | 机制 | 阶段 |
|---|---|---|
| ① 显式引用 | 聊天里打 `[[笔记名]]` / `@note` → **确定性注入**该笔记内容（无排序、用户可控，复用双链 `[[ ]]` 解析） | **Phase 1** |
| ② Agent 主动拉取 | `note_search` 工具，agent 按需混合检索召回（**不**默认全量注入 system prompt，避免膨胀） | **Phase 1** |
| ③ 被动提示 | 每轮注入「相关笔记标题 + 一行」（仿 awareness suffix，独立 cache block，只给标题不给正文，cache-safe） | Phase 3 opt-in |

> **注入即非可信内容（#7，红线）**：三通道注入的笔记正文一律按 **untrusted user content** 处理——尤其外部 vault 里可能有网页剪藏 / 他模型生成文本 / 投毒 prompt。必须套 `<untrusted_external_data>` 信封 + 标注来源（kb/path）+ 截断策略，**永不提升为 system 指令**（对齐 hope-agent 处理 GitHub 等外部内容的现有范式）。访问仍受 `effective_kb_access` 约束（D10）。

**与 memory 的关系（D7）**：`note_search` **绝不折进 `recall_memory`**——两者性质/作用域/排序不可比。若日后要「一次拿记忆 + 笔记」，**加一个薄编排工具**分别查两 store、各自用各自排序再 store-aware 合并，仍不动成熟的 `recall_memory`。**影响方向是反的**：知识库检索做成最强后，反哺给记忆系统借鉴，而非笔记降格塞进记忆。

形成闭环：**对话 → 记忆 → 笔记 → 召回喂回上下文 → 更好的对话**。

---

## 前端 UI

- **一级导航新增「知识空间」Tab**（与聊天 / Dashboard 平级；对外品牌名见 D5，代码内部仍为 `knowledge`）。
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
- ✅ 存储分流（D9）：**KB 偏好/开关**（默认忽略规则、UI 偏好等）走 config contract（`cached_config()` / `mutate_config`）；**KB registry + attach** 走 `sessions.db` CRUD（不进 config）。GUI + `ha-settings` 技能双入口零偏差（KB 偏好属 LOW/MEDIUM 风险）。
- ✅ 日志用 `app_info!` 等；核心路径（索引、解析、检索、AI 提炼）埋点。
- ✅ 新增架构能力 → 本文 + 登记 [`docs/README.md`](../README.md)；落地时 `CHANGELOG.md`（单行用户视角）+ `AGENTS.md`（契约面）补充。

---

## 分阶段路线图

### Phase 1（双链地基 + 核心读写 + 外部只读绑定，对应 D4/D6 选定的 MVP）

1. KB 概念：`sessions.db` 的 `knowledge_bases` registry（D9）+ `index.db` 缓存 schema（chunk 级 + `note_tag`，D12/#5）+ `WorkspaceScope::for_knowledge`。
2. **KB 访问作用域（D10）**：`session/project_knowledge_bases`（带 FK/cascade/CHECK，#7）+ **`effective_kb_access(ctx{session,source,channel})`**（source-aware，#2）+ 叠加公式 max-then-min-cap（#8）+ UI 列出当前生效 KB；incognito/IM 零访问。
3. `notify` watcher（生产级）+ 增量索引（`note` + `note_chunk` + `note_tag` + `note_link`）+ 绑定/启动 reconcile + chunker。
4. Wikilink 扫描 + **确定性 resolve**（#8，路径式 / basename / 稳定歧义，不用 mtime，跳过 code）+ 反链查询。
5. 前端「知识空间」Tab + 笔记 CRUD（含 `delete`，**不含 rename/move**）+ **Backlinks 面板** + 悬空链接提示 + 当前生效 KB 列表。
6. Layer 1 核心工具：`note_create / read / update / patch / append / delete / search / link / backlinks`（`kb` 过 `effective_kb_access(ctx)`）。
7. **读取桥通道 ①②（D7）**：聊天内 `[[笔记名]]` 确定性引用注入（untrusted 信封，#7）+ 独立 `note_search`（不动 `recall_memory`）。
8. **KB 文件预览端点（#2/#3）**：独立 `/api/knowledge/{kb_id}/files/*`，两平面鉴权（owner/API key 全量 + agent/session 过 `effective_kb_access`）+ scope contains，不放宽 session 端点。
9. **外部 vault 只读绑定（D6/D11）**：`root_dir` 指向现成 Obsidian/Logseq vault → 索引/双链/搜索/AI 读；**外部 root 一切写（AI + GUI）禁用**，UI 显示只读；内置默认忽略列表；大库冷启动后台索引 + 进度。

### Phase 2（图谱 + 完整 AI 操作面 + 自主维护 + 外部可写）

- 图谱视图（`react-force-graph`，数据源 `note_graph`）。
- `![[ ]]` 嵌入 / transclusion；`[[` 自动补全。
- Layer 1 进阶工具：`note_rename / move`（**多文件 `[[ ]]` 链接完整性改写**，#9）、`note_similar / related / suggest_links / graph / orphans / broken_links`、`note_distill / moc / session_to_note`。
- Layer 2 自主维护起步：自动建链提案、MOC 自动维护、记忆 → 笔记写入桥。
- **外部 root 放开写（D11）**：`resolve_writable(actor=user|agent)` 拆分 + 写冲突检测（mtime/hash 比对 / `.conflict` 旁车）+ GUI 编辑外部 + 忽略规则配置 UI + 大库索引进度打磨。
- **IM KB 访问 opt-in（D10）**：account / chat 级显式授权，群聊单独确认。
- 富文本编辑器评估（Tiptap / Milkdown）。

### Phase 3（深度网络 + 融合）

- 块级引用（读 Obsidian `^block-id` + Logseq `((uuid))`）。
- 原生大纲可选层（Logseq block 树 + `((块引用))`，D8）。
- Layer 2 进阶：去重合并、孤岛救援、知识缺口检测、自动打标签。
- 读取桥通道 ③：被动「相关笔记标题」提示（awareness 风格 cache block，opt-in）。
- 可选编排工具「一次拿记忆 + 笔记」（store-aware 合并，**不动 `recall_memory`**，D7）；记忆系统反向借鉴知识库检索。
- Canvas 知识白板。

---

## 安全约束

- **真相 / 缓存分家（D9）**：KB registry + 访问绑定在 `sessions.db`；`index.db` 纯缓存，删了能重建。
- **`index.db` 含明文笔记片段（#4）**：`note_chunk.body` 与 FTS external-content **存明文 chunk 正文/片段**（snippet 高亮 + 离线检索所需），**敏感度等同笔记本身**（笔记本就是磁盘明文 `.md`）——按用户数据保护，随数据目录权限走；红线是**绝不存 API Key / Token / 凭据**。（备选 contentless FTS 可不落 body，但外部 vault 回读慢，不采用。）
- **访问默认 deny（D10，source-aware）**：KB 不全局可见；`note_search / read` 与 KB 预览端点全过 `effective_kb_access(ctx)`（带 source）。incognito 零访问/零写/零被动召回；**IM Phase 1 禁用 KB 访问**（即便 session 有 attach）。
- **作用域闭合**：所有读写经 `WorkspaceScope::for_knowledge`，canonicalize + `starts_with` 失败即拒，禁止越出 `root_dir`（含外部绑定 root）。
- **外部 root 写隔离（D11）**：Phase 1 外部绑定 root 彻底只读，`resolve_writable` 对外部 root 拒绝一切写（AI + GUI）；Phase 2 放开时叠加 actor 拆分 + 写冲突检测。
- **远端写门控**：HTTP `/api/knowledge/*` 写端点受 `filesystem.allow_remote_writes`（默认 false）闸门；桌面 Tauri 不受限。
- **preview-by-path 红线（#2/#3 收口）**：**不放宽**现有 `/api/sessions/{id}/files/*` 端点；KB 文件读取走**独立** `/api/knowledge/{kb_id}/files/*`，按**两平面**鉴权——owner/API key 走管理平面（全量），agent/session 走 `effective_kb_access(ctx)`——再叠加 `WorkspaceScope` scope contains 校验，**不是**「路径命中任意 KB root」。非授权 kb_id / 越界路径 / 主机任意路径一律 403。HTTP 远端绑外部主机路径属敏感场景，落地走专门安全 review。
- **注入即非可信（#7）**：笔记内容注入上下文一律套 `<untrusted_external_data>` 信封 + 来源 + 截断，永不提升为 system 指令。

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
| `crates/ha-core/src/knowledge/` | 新增模块 | 核心逻辑（types/registry/access/db/parser/chunker/index/watcher/resolver/search） |
| `crates/ha-core/src/session/`（或 project DB 迁移） | 改动 | `sessions.db` 加 `knowledge_bases` + `session/project_knowledge_bases` 表（D9/D10）+ migration |
| `crates/ha-core/src/filesystem/workspace.rs` | 改动 | 增 `for_knowledge` 作用域入口；`resolve_writable` 外部 root 写拒绝（D11） |
| `crates/ha-core/src/tools/definitions/core_tools.rs` | 改动 | 注册 `note_*` 工具（`kb` 过 `effective_kb_access`） |
| `crates/ha-core/src/paths.rs` | 改动 | 集中 `knowledge/` 路径 |
| `src-tauri/src/commands/` + `invoke_handler!` | 改动 | KB Tauri 命令薄壳 |
| `crates/ha-server/src/routes/` + `router.rs` | 改动 | `/api/knowledge/*` HTTP 路由 + 独立 KB 文件预览端点（#2） |
| `src/components/knowledge/` | 新增 | 知识库 Tab、笔记列表、Backlinks 面板、当前生效 KB 列表 |
| `src/lib/transport*.ts` | 改动 | KB invoke 双适配 |
| `docs/architecture/api-reference.md` | 改动 | 新接口对照登记 |
