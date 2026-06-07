# Knowledge Base（知识空间）

> 返回 [文档索引](../README.md) | 更新时间：2026-06-07

本文是知识空间子系统的**单一真相源**：定位、设计取舍、数据模型、鉴权、工具面、检索、前端、自主维护与安全红线。实现细节引用代码路径而非复制，以代码为准。对外功能名「知识空间 / Knowledge Space」，代码内部保持中性——模块 `knowledge/`、工具 `note_*`、作用域 `for_knowledge`（D5）。

## 概述

知识空间是与聊天、Dashboard 平级的**独立一级功能**——一个本地优先、AI 原生的个人知识管理（PKM）子系统。它把 Hope Agent 从「聊天助手」推进到「第二大脑」：用户手写 / 编辑 / 管理 `.md` 笔记，agent 作为**第一公民**读写同一批笔记、检索知识网络、并把后台沉淀的碎片记忆提炼成结构化笔记。

三条产品原则贯穿全系统：

- **真实 `.md` 是唯一真相源**：笔记就是磁盘上的标准 Markdown 文件，永不锁定、永不破坏性转写；SQLite 索引只是可重建缓存，删了能从 `.md` 全量重建。用户可随时用 Obsidian / Logseq 打开同一文件夹，零锁定。
- **AI 原生，不是事后插件**：别人手动织链，这里 agent 既能读知识库又能写知识库（CRUD / 链接 / 图谱 / 检索 / 自主维护），区别于 Obsidian / Logseq「AI 是插件」的形态。
- **默认 deny + 显式 attach**：知识库不像记忆那样全局可见——工作 vault / 私人 vault / IM 会话彼此隔离，访问唯一经 `effective_kb_access`（D10）。

### 第四种知识 Scope

知识库（KB）是 Hope Agent 在已有三层知识容器之外平行的第四个：

| 容器 | 真相源 | 谁写 | 谁读 | 用户可见度 |
|---|---|---|---|---|
| Memory | `memory.db` 原子条目 | 自动抽取 + `save_memory` | 注入 system prompt | 低（后台） |
| Dreaming 日记 | `~/.hope-agent/memory/dreams/*.md` | AI 自省 | 用户翻看 | 中 |
| Project | `working_dir` 真实文件 | 用户 / agent | `read` 工具 | 高 |
| **知识空间（KB）** | **真实 `.md` 文件** | **用户手写 + agent 工具** | **agent 工具 + 按需召回** | **最高（一级导航）** |

**和 AI 的双向桥**（区别于纯手动 PKM 的核心）：

```
                ┌──────────────── 写入桥 ────────────────┐
   对话 ──► Memory（碎片）──► Dreaming 提炼 ──► 知识库笔记（MOC / 可读层）
                                                    │
                ┌──────────────── 读取桥 ────────────┘
   agent ◄── 召回 ◄── FTS5 + 向量索引 + 双链图谱 ◄── 笔记
```

读取桥三通道：① 用户消息 `[[note]]` 确定性注入；② agent 主动调 `note_search` / `knowledge_recall`；③ 被动「相关笔记标题」提示（opt-in 默认关）。三者都套 `<untrusted_external_data>` 信封，永不提升为 system 指令（#7）。

## 设计取舍（决策账本 D1–D14）

> 每条记录**结论 + 理由**。D 编号在全文用作契约锚点。

| # | 决策点 | 结论 | 理由 |
|---|---|---|---|
| D1 | 笔记与记忆的关系 | 独立笔记系统，但与 AI 双向打通——agent 能读能写，记忆可提炼成笔记「可读层」 | 要「完整大功能 + AI 紧密联合」，既非纯手动，也不把笔记降级为大号 memory |
| D2 | 存储真相源 | 真实 `.md` 文件 + SQLite 旁路索引 | 贴合「文件即真实文件」；可与 Obsidian 互通；索引可重建；检索复用 memory 基建 |
| D3 | 容器概念 | 独立「知识库」容器（不复用 Project） | 用户要一级功能 + 独立心智模型 |
| D4 | MVP | 双链地基：Wikilink 解析 + Backlinks | 最小可用、最快出效果，是图谱 / 嵌入 / 召回的地基 |
| D5 | 对外命名 | 功能名「知识空间」；代码内部中性（`knowledge/` / `note_*` / `for_knowledge`） | 「知识库」在中文被 RAG / 客服语义占领易误读；代码标识符 / 展示名 / 营销 slogan 三层解耦 |
| D6 | 外部目录绑定 | 内部 `notes/` 完整读写 + 外部 vault 绑定（默认只读，opt-in 放开写） | 外部绑定是最大获客杠杆（「点亮现成 vault」）；写外部的 lost-update 风险用 opt-in 隔离 |
| D7 | 召回形态 | 笔记检索是**独立通道**，绝不折进 `recall_memory`；要一次拿记忆 + 笔记则加薄编排工具分查两 store 再 store-aware 合并 | 记忆 = 一句话事实、笔记 = 整篇文档，性质 / 排序不可比，混排污染成熟 memory 路径 |
| D8 | 文档优先 vs 大纲优先 | **文档优先打底**（对齐 Obsidian），`Note{title, body, frontmatter}`；原生大纲作**只读可选层** | 文档优先与大纲优先数据模型从根不同，无法一套原生兼容两者；文档优先覆盖最广、最轻 |
| D9 | 存储分家 | KB registry + 访问绑定落 `sessions.db`（真相源）；`index.db` 只存可重建缓存 | KB 是一级关系实体（列表 / 归档 / 绑定 / 权限），删索引后必须能全量重建 |
| D10 | 访问作用域 | 默认 deny + 显式 attach；incognito 零访问；IM 默认禁用（账号级 opt-in）；唯一入口 `effective_kb_access(ctx)`，**带 source + origin_source**、取调用链最严 cap | KB 不能像 memory 全局可见，否则工作 / 私人 / IM 互相泄漏；子 Agent 不能借 `source` 洗回权限 |
| D11 | 外部 root 可写性 | 外部绑定 root **默认彻底只读**（AI / GUI 皆拒）；KB 级 `allow_external_writes` opt-in 后解锁 | 「点亮老 vault」不等于「托管老 vault」；写冲突 / 原子写 / 三方 rename 噪声靠 opt-in 隔离 |
| D12 | 检索粒度 | **chunk 级**：`note` 只存文件级元数据，正文检索下沉到 `note_chunk`（FTS + vec 都建 chunk 上，命中再聚合回 note） | 整篇一个 embedding 在长文 / 纪要上失效（超上限 + 定位不到段）；`content_hash` 支持按 chunk 增量重嵌 |
| D13 | 编辑器选型 | **CodeMirror 6 强 source editor** + 实时预览；**不引入 Tiptap / Milkdown WYSIWYG**，以 CM6 live-preview 模式逼近所见即所得 | 核心是「真实 `.md` + wikilink + 字符 offset + AI patch + diff + 互通」，要源文档稳定可控；ProseMirror 往返序列化破坏这些契约 |
| D14 | 坐标系契约 | 持久 offset = **Unicode 码点偏移**（索引内部）；跨端定位主字段 = `line`（1-based）+ `col`（0-based 码点列，tab=1），相对**原始完整文件**；`note_patch` **不用坐标**走 `old/new` 文本匹配 | 三套坐标（UTF-8 字节 / 码点 / UTF-16）+ CRLF + tab 全是错位源，必须钉死；LLM 产不准坐标且坐标随上文漂移 |

## 两类存储（D9）

- **真相源**：`KnowledgeRegistry`（[`knowledge/registry.rs`](../../crates/ha-core/src/knowledge/registry.rs)）—— `knowledge_bases` + `session_knowledge_bases` + `project_knowledge_bases` + `kb_maintenance_proposals` + `kb_graph_layout`（图谱布局，按 `rel_path` 键）落 `sessions.db`，包 `Arc<SessionDB>` 复用连接（仿 `ProjectDB` / `ChannelDB`），均 `ON DELETE CASCADE` 随 KB 删。
- **可重建缓存**：`IndexDb`（[`knowledge/db.rs`](../../crates/ha-core/src/knowledge/db.rs)）—— `note` / `note_chunk` / `note_link` / `note_tag` + FTS5（`note_chunk_fts`）+ sqlite-vec（`note_vec`）落 `~/.hope-agent/knowledge/index.db`。删了能从 `.md` 文件全量重建（连 `rel_path` 都是缓存）。连接模型仿 memory backend：1 写连接 + 4 读连接池 + WAL + sqlite-vec auto-extension。

笔记 = 真实 `.md` 文件（唯一真相源）。内部 KB（`root_dir=NULL`）落 `~/.hope-agent/knowledge/{id}/notes/`（lazy ensure），可写；外部绑定 vault（`root_dir` 非空）**默认只读**，KB 级 `allow_external_writes` opt-in（owner GUI）后解锁编辑器 / AI 写入（D11）。`resolve_kb_dir` 返回 `KbRoot{dir, is_external, read_only}`——`read_only = is_external && !allow_external_writes`，`WorkspaceScope::for_knowledge` 取 `read_only`；写冲突沿用 stale-write guard（比磁盘 raw BLAKE3，冲突中止）。**后台自主维护 `scheduler.rs` 按 `is_external` 跳过所有外部 root，无视 opt-in**——只 GUI / agent 按需写外部。

## 数据模型

真相源类型在 [`knowledge/types.rs`](../../crates/ha-core/src/knowledge/types.rs)；完整 DDL 见 `registry.rs`（sessions.db）与 `db.rs`（index.db），以代码为准。下表只列角色与关键字段。

**`KnowledgeBase`（真相源，`sessions.db`）**：`id`（UUID）/ `name` / `emoji` / `root_dir`（`NULL`=默认内部目录，非空=外部绑定）/ `archived` / `allow_external_writes`（外部写 opt-in）/ `created_at` / `updated_at`。`name / emoji / root_dir` 无法从 `.md` 重建，故必须随真相源持久化（D9）。

**访问绑定（真相源，`sessions.db`，D10）**：`session_knowledge_bases(session_id, kb_id, access)` + `project_knowledge_bases(project_id, kb_id, access)`，`access ∈ read | write`，项目内 session 继承 project attach。

**`Note`（缓存行，真相在文件）**：`id`（自增）/ `kb_id` / `rel_path`（相对 root，缓存）/ `title`（frontmatter `title` > 首个 H1 > 文件名）/ `frontmatter_json` / `mtime` / `size` / `content_hash`。`content_hash` = **整篇文件 BLAKE3 over raw 字节**（不归一化换行，保留 CRLF，对齐 D14），仅作返给调用方的「最近索引 token」做乐观并发对照；**非写入判定源**——写入判定一律以磁盘当前 raw BLAKE3 为准。正文检索全下沉到 `note_chunk`，Note 行不直接挂 fts / embedding。

**`NoteChunk`（chunk 级检索单元，`index.db`，D12）**：`id`（= fts / vec rowid）/ `note_id` / `chunk_index` / `heading_path`（命中定位 + `#heading` 锚定）/ `body`（已剥 frontmatter / 归一化，仅供 FTS external-content，**不**用于坐标）/ `start|end_offset`（码点偏移）/ `start|end_line|col`（跨端 UI 定位主字段）/ `content_hash`（按 chunk 增量重嵌）/ `embedding_signature`（识别需重嵌的 chunk）。**向量单存 `note_vec`**（sqlite-vec vec0，rowid = chunk id），行内不再存 embedding BLOB。

**`NoteLink`（双链边）**：`src_note_id` / `target_ref`（`[[ ]]` 原文目标）/ `target_note_id`（`NULL`=悬空链接）/ `link_type`（`wiki` / `embed` / `md`）/ `anchor`（heading slug 或 `^block-id`）/ `alias` / `raw_text` / `src_start|end_line|col`（链接在源文件位置，反链精确跳转）/ `src_heading_path`。**反向链接** = `WHERE target_note_id = ?`，一个索引即可，无需独立表。

**坐标系契约（D14）**：三套坐标不可混——Rust UTF-8 字节 / Unicode 码点 / JS·CM6 UTF-16。持久 offset = 码点偏移（索引内部）；跨端定位走 `line`（1-based）+ `col`（0-based 码点列，tab 记 1 码点不展开），按 `\n` 分行、`\r\n` 视作单个行终止符、**不改写原文件换行**。`note_chunk` 与 `note_link` 的坐标都相对**原始完整文件**（含 frontmatter / CRLF），不是相对剥离后的 `body`。CM6 内部 UTF-16，跳转 / 命中定位一律走 line/col，前端做 UTF-16↔码点转换。

## 模块地图（`crates/ha-core/src/knowledge/`）

| 文件 | 职责 |
|---|---|
| `types.rs` | `KnowledgeBase` / `Note` / `NoteChunk` / `NoteLink` / `KbAccess` / 搜索结果 / 图谱布局类型 |
| `registry.rs` | KB CRUD + 访问绑定（真相源）+ `resolve_kb_dir`（内部 lazy ensure / 外部只读标记）+ 维护提案表 + 图谱布局表 |
| `db.rs` | index.db 后端：note/chunk/link/tag 写入（单事务重索引）+ FTS/vec 查询 + 反链 + 重解析 + `list_broken_links` / `list_orphan_notes`（维护面板）+ `all_resolved_links`（图谱边）+ `block_backlinks`（块级反链：`note_link.anchor = '^id' COLLATE NOCASE`） |
| `parser.rs` | pulldown-cmark 扫 heading / code + **叶块 span**（paragraph / item / heading），正则扫 `[[ ]]` / `#tag` + **Obsidian `^block-id` 块锚**（跳过 code），D14 坐标（`PosMap` 码点 offset + line / col，相对原始全文）+ 手写 frontmatter→JSON。`ParsedBlock{block_id,start,end,text}`：行尾 `^id`（`[A-Za-z0-9-]+`）附到所在叶块（独占行的 `^id` 附到上一块），`text` 剥锚，首个 id 胜出，**不落表**（transclusion 重解析、块反链查 `note_link.anchor`），`line_block_anchor` 供写工具复用 |
| `chunker.rs` | 按 heading 分段 + 大小封顶（D12），产出 chunk（D14 坐标 + BLAKE3 content_hash + overlap）。参数 `ChunkConfig{max_chars, overlap_chars}`（默认 1500 / 80，`clamped()` 钳 `[200,8000]` / `[0,max/2]`） |
| `resolver.rs` | `[[ref]]` → note_id 确定性规则（路径式 > 唯一 basename > 最短路径再字典序，NFC + 大小写不敏感，**不用 mtime**） |
| `rename.rs` | note / folder 改名移动 + **入站 `[[ ]]` 链接改写**：`rename_note` / `rename_dir` 复用给 owner 平面 + agent 工具；纯文本变换 `rewrite_content`（re-parse 跳 code、按 D14 码点 offset splice、保留 `#anchor` / `\|alias` / `![[ ]]`，路径式→新路径、basename→新 stem，歧义退回路径式） |
| `index.rs` | 索引器：文件 → parse → chunk → embed → IndexDb；KB reconcile（mtime 增量 + prune）；全局 `IndexDb` |
| `watcher.rs` | `notify` 生产级 watcher（debounce 800ms，仅 `.md` 事件，per-KB 线程，外部 vault 实时同步，D6） |
| `access.rs` | `effective_kb_access(KnowledgeAccessContext)`（D10）：incognito short-circuit → IM 全链归零（除非 origin 账号 / 群聊 opt-in，`im_lineage_denied`）→ `max(session, project)` → 滤 archived → 外部 `read_only` root cap read（opt-in 可写则不 cap） |
| `search.rs` | chunk 级 FTS + vec → RRF → MMR → 聚合回 note；`similar_notes` 向量 KNN。算法复用 memory，独立 store（D7） |
| `graph.rs` | 链接图谱构建（纯变换）：`build_kb_graph`（节点=笔记+度数，边=去重 resolved 链接，丢自环）/ `ego_subgraph`（N 跳无向邻域）/ `cap_nodes`（按度数截断标 `truncated`）；owner 图与 `note_graph` 工具共用 |
| `service.rs` | owner 平面操作（GUI / HTTP）：list / read / save / delete / rename / backlinks / search / broken_links / orphans / graph / note_read_ref / ai_rewrite / 维护配置，不经 `effective_kb_access`。`note_rename` / `rename_dir` 委托 `rename::*`（移动文件 + 改写入站 `[[ ]]`，返回 `RenameOutcome{newRel, filesChanged, linksRewritten}`）；外部 root 只读拒写。`note_read_ref` 经 resolver 解析 `[[ ]]` ref 再读，按 ref 的 `#anchor` 切片（`slice_by_anchor`：`^id`→块文本、heading→该标题到下一同 / 更高级标题段，未命中降级整篇） |
| `inject.rs` | 读取桥①：用户消息 `[[note]]` 确定性注入（`untrusted_external_data` 信封，受 `effective_kb_access` 约束，#7） |
| `embedding.rs` / `reembed.rs` | 知识空间独立 embedding selector + 后台重嵌 job（见「检索与索引」） |
| `maintenance/` | Layer 2 自主维护（见「自主维护」） |
| `mod.rs` | `blake3_hex`（D14 hash 契约：BLAKE3 over raw bytes）+ `delete_kb_cascade`（registry 事务 + index prune + 内部目录 rm-rf，外部 root 永不删） |

`agent/related_notes.rs`（在 `knowledge/` 之外）承载读取桥③ 被动相关笔记，见「检索与索引」。

## 两个鉴权平面（D10）—— 物理隔离

| 平面 | 在哪层 | 主体 / 鉴权 |
|---|---|---|
| **Owner / 管理** | HTTP 端点 / Tauri 命令（`service.rs`） | owner（桌面本机信任 / HTTP API key = owner-equivalent），看自己**所有** KB，**不经 attach** |
| **Agent / session** | ha-core 工具执行（`note_*`，进程内） | turn 内 agent；`effective_kb_access(ctx)`（session + source + 全链 cap + incognito） |

KB 文件预览端点 `/api/knowledge/{kb_id}/files/*` = 纯 owner 平面，**无 session 参数、无 fallback**，与 `/api/sessions/{id}/files/*` 不互相放宽。`note_*` 工具读笔记不经 HTTP 端点（ha-core 内返回内容）。

**source-aware**：`ChatSource{Desktop|Http|Channel|Subagent|ParentInjection}`（不在 `ToolExecContext` 上）经 `configure_agent` 映射成 `KbAccessSource` 透传到 `AssistantAgent.chat_source` → `ToolExecContext.chat_source`。IM（`Channel`）→ KB 访问默认归零（即便 project attach）；incognito 由 `is_session_incognito(session_id)` short-circuit。**血缘 origin 真接线**：`ChatEngineParams.origin_source`（顶层 `None`→origin=source）→ `configure_agent(kb_origin)` → `agent.origin_chat_source` → `ToolExecContext.origin_chat_source`；`subagent` 工具 spawn 时把父轮 `ctx.origin_chat_source.or(chat_source)` 经 `SpawnParams.origin_source` 透传给子 `ChatEngineParams.origin_source`，`effective_kb_access` 的 cap 查 `source.is_im() || origin_source.is_im()`，故 IM-origin 子代理被归零。**双重防线**：即便不接线，子代理子会话也无 attach / 无 project_id（`create_session_with_parent` 不继承）→ 天然空集；origin cap 是面向未来（若子代理改为继承 project）的纵深防御。系统发起的 spawn（plan / team / hooks / fork skill）`origin_source=None`，靠会话隔离。

**IM opt-in**：IM 默认归零的红线可按账号放开。IM 身份经 `ChannelKbContext{channel_id, account_id, chat_id, is_group}` 真接线透传：dispatcher 填顶层 IM turn 身份 → `ChatEngineParams.channel_kb_context` → `configure_agent` → `agent.channel_kb_context` → `ToolExecContext.channel_kb_context` → `KnowledgeAccessContext::resolve`（在此调 `channel::im_kb_access_allowed` 读 config 算出 `im_access_allowed` bool，`effective_kb_access` 只消费这个纯 bool，故短路规则单测无需全局）。判定：账号级 `settings.kbAccessOptIn`（owner GUI-only，默认关）；DM 只需账号 opt-in；群聊还需 `settings.kbAccessChats` 含该 chat（群内 `/kb on` 写入）；账号查不到 / channel_id 不匹配 → fail closed。`subagent` 工具把父轮 `ctx.channel_kb_context` 经 `SpawnParams.origin_channel_kb_context` 透传给子轮，故 **IM-origin 子代理按 origin 账号 / 群聊判 opt-in，不洗权限**。`access.rs` 短路单测覆盖：opt-in 关归零 / DM 放行 / 群聊未确认归零 / IM-origin subagent 无 opt-in 归零 / opt-in 放行 / incognito 压过 opt-in。

## 工具面（Layer 1，`tools/note.rs`）

agent 在对话中直接调用，覆盖 CRUD / 链接图谱 / 检索 / 元数据 / AI 高阶。均 `internal=false`（过权限引擎 + plan-mode），`kb` 过 `effective_kb_access`：写需 write + 内部 root + 全链允许 + 非 incognito；读 `kb?` 省略时只搜可访问集合（跨 KB 同名返 disambiguation）。

**CRUD / 链接**：`note_create / read / update / patch / append / delete / search / link / backlinks / by_tag / tags`。`note_rename` / `note_move`（别名共用 handler）移动 `.md` + **改写入站 `[[ ]]`**（`knowledge::rename_note`）。`note_set_frontmatter({kb, path, props})` 合并写 YAML frontmatter（`null` 值删键）——`parser::merge_frontmatter` 逐行非破坏性编辑：只重写命中的顶层键、其余行（含嵌套 map / 块标量）原样保留、键序不变、类型保真（reserved / 数字串自动加引号），全删则丢整个 frontmatter 围栏。`note_backlinks` 可选 `block` 参数 → `db::block_backlinks`（`^` / 空白塌成空 id 直接拒）。

**图谱 / 完整性**：`note_graph({kb?, note?, depth?})` 复用 `graph::build_kb_graph`——给 `note` → `ego_subgraph`（depth 1–3，默认 1，跨可访问集合 resolve 出 kb），不给 → 全 KB 图 `cap_nodes(200)`（`truncated` 标截断），输出 `{kbId, nodeCount, edgeCount, truncated, nodes, edges}`。`note_broken_links` / `note_orphans`（`kb` 必填）复用 `db::list_broken_links` / `list_orphan_notes`。

**智能检索（纯检索无 LLM）**：`note_similar`（`search::similar_notes` 向量 KNN，aggregate 到 note 排除自身；无 embedder 时返空 + 提示开 embedding）/ `note_related`（融合 backlinks ∪ resolved 出链 ∪ 同标签 ∪ 向量近邻，按命中信号加权、带 `reasons`）/ `note_suggest_links`（`strip_links_and_code` 去码块 / inline code / 已有 `[[ ]]` 后 `contains_word` 词界匹配其它 note 的 title / basename，排除已链接，cap 5000 候选 / 25 建议）。三者复用 `read_resolved_note`。

**AI 高阶（side_query 驱动 + 写）**：经 `run_kb_side_query`（`recap::report::build_analysis_agent` + `side_query`，与 recall-summary / dreaming 同源，与主对话 agent 解耦）。`note_distill`（`source` 笔记或 `text` 原文 → JSON 数组 `parse_distilled` → 建 2–8 篇原子笔记，`slugify` + `unique_rel_path` 防覆盖）/ `note_moc`（按 `topic`（hybrid search）/ `tag`（notes_by_tag）聚合 → 生成 MOC markdown → 写 `MOCs/<slug>.md` 标 `moc: true`；重写只刷新自己生成的 MOC，撞用户笔记退回 `unique_rel_path` 不覆盖）/ `session_to_note`（`session` 或当前会话 → `load_session_messages` 拼转录 → 生成结构化笔记；**无痕会话源直接拒**守「关闭即焚」）。均 `require_write` + `writable_scope`（外部 root 拒，且在 LLM 调用前 fail-fast）。

**块级引用写入（D14 / 三闸门）**：`note_assign_block({kb, path, block_text, block_id?, expected_file_hash?})` 给目标块加 Obsidian `^id`。`resolve_anchor_placement`：`block_text` 唯一命中（同 `note_patch` 0 / 多次拒）后**解析整个叶块**——段落多行时把 `^id` 落到该块最后一行末尾（不截块），列表项 / 标题按单行块；**幂等**检测覆盖整块（块尾或块下独占行已有 `^id` 直接回该 ref，不写第二个）；**拒 frontmatter / 代码围栏命中**。id 缺省由 `blake3(block_text)` 取 6/8/12/16 hex 防撞确定性生成（无 RNG），显式 id 校验 `[A-Za-z0-9-]+` + 防重（`collect_block_ids` 原始行扫覆盖未解析锚）。返回的引用走 `stable_block_ref`：basename 经 resolver **唯一回指本笔记**才用 basename，否则用路径式 `[[folder/Note#^id]]`（防 basename 撞车被 resolver 解析到别篇）。

**合并检索 `knowledge_recall`（D7 store-aware）**：一次查 memory + 笔记两 store，返回 `{memories: {count, hits}, notes: {count, hits}}` **两段独立排序、绝不归一化混排**。**薄编排器**：分别调 memory backend `search` + `knowledge::search::search_notes`，**绝不折进 / 改动 `recall_memory`**。KB 段经 `accessible_kbs`（`effective_kb_access`，空集 / incognito / IM 未 opt-in 时为空）；memory 段在 incognito 会话整体跳过。`Standard{default_deferred:true}`——`recall_memory` / `note_search` 已各自 eager 覆盖单 store，本工具经 `tool_search` 发现。

**stale-write guard（强契约）**：`expected_file_hash` 比**磁盘当前 raw BLAKE3**（不比 `note.content_hash` 索引缓存）。`note_patch` 走 `old/new` 文本唯一命中（0 / 多次都拒，仿 `edit`，D14 坐标不做 patch 寻址）。

## 检索与索引

**写入数据流**（内部 KB / owner 保存 / 工具）：写盘 → `index::reindex_note`（parse → chunk → embed → `replace_note_index` 单事务，FTS 触发器同步、vec 手动同步）→ `reresolve_kb_links`（全 KB 重解析，broken↔resolved 翻转）→ emit `knowledge:changed`。外部 vault：bind / 启动 / 打开 `reindex_kb`（mtime 增量 + prune）+ `notify` watcher 实时 reconcile。

**检索管线**：`search_notes` → chunk FTS5（BM25）+ vec0 KNN（signature 过滤）→ 加权 RRF（text 0.4 / vec 0.6 / k 60）→ 聚合 best-chunk 回 note → MMR（λ 0.7）。向量单存 `note_vec`。

**Embedding 配置（D7，独立 selector）**：知识空间的向量化**不寄生记忆**——有自己完整的配置生命周期，记忆没配 / 关了都不影响知识空间向量检索（关了只降级 FTS-only，不回退到 `memory_embedding`）。

- **配置三层**（与 memory 对称，共享底层）：`AppConfig.embedding_models`（共享命名模型库 provider / apiKey / model / dims，memory 与 knowledge 同一份）+ `AppConfig.knowledge_embedding: EmbeddingSelection`（知识空间独立选择器 `enabled` / `model_config_id` / `active_signature` / `last_reembedded_signature`）+ 运行时 `resolve_memory_embedding_config(&knowledge_embedding, &embedding_models)` 解析成 provider（纯函数）。
- **helper**（[`knowledge/embedding.rs`](../../crates/ha-core/src/knowledge/embedding.rs)）：`knowledge_active_embedding_signature`（索引 + 检索热路径签名源，**不读** `memory::active_embedding_signature`）/ `set_knowledge_embedding_default`（验证 provider → 写 selection → 装 index embedder → spawn reembed）/ `disable_knowledge_embedding` / `apply_knowledge_embedding_from_config`（热重载）。复用 memory 的 `create_embedding_provider` 工厂、`EmbeddingProvider` trait、`signature()`、`embedding_cache`（按 signature 命中，同模型与 memory 共享缓存）、RRF / MMR 算法。
- **重建**（[`knowledge/reembed.rs`](../../crates/ha-core/src/knowledge/reembed.rs)）：切模型 → 装新 embedder（维度变则 `note_vec` DROP 重建）→ spawn `LocalModelJobKind::KnowledgeReembed`，遍历所有 KB `reindex_kb(full=true)` 重 embed 全部 chunk，进度 KB-granular，完成写 `last_reembedded_signature`。复用 memory 的 `local_model_jobs` 框架（取消 / 单实例 / 进度 / retry）。
- **分块配置（D12，高级）**：`AppConfig.knowledge_chunk: ChunkConfig`（`clamped()` 钳 `[200,8000]` / `[0,max/2]`）。owner 命令 `knowledge_chunk_{get,set}_cmd` / HTTP `GET|POST /api/knowledge/chunk`；`service::set_chunk_config` 写 config + 触发全 KB 重切（向量开→重嵌、关→FTS-only re-chunk；**不 stamp signature**，chunk 改动不是模型覆盖事件）。
- **共享库交叉保护**：`save_embedding_model_config` / `delete_embedding_model_config` / Ollama 删模型清理都对 memory **与** knowledge 的 active model 双向守门（改 / 删 active model 一律拒；删 Ollama active 重置对应 selection + 清对应 embedder）。
- **owner 平面 + GUI-only**：命令 `knowledge_embedding_{get,set_default,disable}_cmd` / HTTP `GET /api/knowledge/embedding`、`POST /api/knowledge/embedding/{set-default,disable}`。与 `memory_embedding` 一致**不进 `ha-settings`**（模型选择 + reembed 副作用，类比 `active_model` 的 GUI-only 豁免）。

**读取桥③ —— 被动相关笔记**（[`agent/related_notes.rs`](../../crates/ha-core/src/agent/related_notes.rs)，D7，opt-in 默认关）：`AppConfig.knowledge_passive_recall`。每个用户轮在 `tokio::join!` 里与 awareness / active_memory 并发跑 `refresh_related_notes_suffix`：incognito short-circuit → 读 clamp 后 config → `hash(user_text)` TtlCache（默认 120s）→ `spawn_blocking` 内从 agent 线接的 `chat_source / origin_chat_source / channel_kb_context` 重建 `KnowledgeAccessContext` → `effective_kb_access` 拿可访问 KB → `search::search_notes` 取 top-N → 渲染「## Related Notes」**只给标题**（`show_snippet` 可开一行摘要）套 `<untrusted_external_data>` 信封。**无 LLM 调用**（比 active_memory 廉价）。结果写 `agent.related_notes_suffix` slot，四 Provider adapter 注入：**Anthropic 走 plain system block（无 `cache_control`——4 个 breakpoint 已被 prefix / awareness / active_memory / last-tool 占满，加第 5 个会 400）**；OpenAI* / Codex 加独立 system message。红线：注入即 untrusted / incognito 零被动召回 / IM 未 opt-in 零访问（access 链同 `note_*`）/ 只给标题（正文走通道①②）。

## 块级引用与大纲（深度网络）

**块级引用（仅 Obsidian `^block-id`）**：`parser` 扫块产 `ParsedBlock`，**不落表**——transclusion 切片 `note_read_ref::slice_by_anchor` 重解析、块反链 `db::block_backlinks` 查 `note_link.anchor`。`![[Note#^id]]` 切块、`![[Note#Heading]]` 切标题段（前端 transclusion 传**全 ref**，anchor 未命中降级整篇）；`[[ ]]` 提及注入按 Obsidian 语义仍**整篇**（切片只对 `![[ ]]`）。写入走 `note_assign_block`（见工具面）。**Logseq `((uuid))` / `id::` 不做**——大纲优先模型与文档优先底座冲突（D8），`logseq/` 已在忽略列表。

**原生大纲只读视图（D8 可选层）**：`NoteEditorMode` 第 5 模式 `outline`，`outline.ts::buildOutline` 纯派生标题树（不改 `.md`）→ `OutlineView` 可折叠只读渲染，点标题经 `onOutlineJump` 切回 `source` 再 `setRevealTarget`。**红线：只读、永不替代 CM6 底座 / 不破坏性转写**。

## 自主维护（Layer 2，`knowledge/maintenance/`）

模块 [`knowledge/maintenance/`](../../crates/ha-core/src/knowledge/maintenance/)（零 Tauri），镜像 `memory/dreaming`：后台周期扫描每个**内部** KB（外部只读 root 跳过），产出**维护提案**进 draft 审阅队列；用户在维护面板确认前绝不动笔记。**默认全关**（`AppConfig.knowledge_maintenance`）。

- **调度**（`scheduler.rs`）：`MAINTENANCE_RUNNING` AtomicBool 串行锁 + `try_claim`；idle 触发复用 dreaming 活动时钟（`check_idle_trigger`，app_init 60s ticker 与 dreaming 同 loop）；`spawn_maintenance_cron_loop`（`LOOP_SPAWNED` once 守卫，app_init **primary-gated** 调一次，听 `config:changed` 重排）。`run_cycle` 遍历 `registry.list(false)`、跳外部、调 `generators::generate`、`registry.insert_proposal` 落库（`INSERT OR IGNORE` + 唯一 `(kb_id, fingerprint, status)` 去重），`auto_approve` 时即时 `approve_proposal`，末尾 emit `knowledge:changed{op:maintenance}` + `knowledge:maintenance_complete` + learning event。
- **持久化**（`kb_maintenance_proposals` 表，落 `sessions.db` 真相源 D9，`ON DELETE CASCADE`）：`insert_proposal` / `list_proposals` / `get_proposal` / `set_proposal_status` / `count_pending_proposals` / `prune_proposals`。`row_to_proposal` 对未知 kind / status / 坏 action JSON 跳过（前向兼容）。
- **8 类生成器**（`generators.rs`）：确定性的（`auto_link` 未建链提及 / `orphan_rescue` 同标签救援 / `frontmatter_fill` 补 title / `dedup_merge` 标题 Jaccard 或同 hash / `knowledge_gap` 高频悬空目标建桩）跑在**一个 `spawn_blocking`**；LLM 的（`auto_tag` / `moc_upkeep` / `memory_to_note`）走 `build_analysis_agent` + `side_query`（带 `llm_timeout_secs`）。每任务 `PER_TASK_CAP` + 整轮 `max_proposals_per_cycle` 双封顶。
- **落地**（`apply.rs`，owner 平面）：`ProposalAction` 四形（`AppendLink` / `SetFrontmatter` / `CreateNote` / `MergeNotes`）各复用 `service::note_read / note_save / note_delete` + `parser::merge_frontmatter`，写前重读磁盘 hash 做 stale-write guard，幂等。owner 已批准故**绕 D10**（等同 GUI 编辑）。
- **owner 命令**：run / status / list / pending-count / approve / reject / reject-all + config get / set（`service::{get,set}_maintenance_config`，set 经 `mutate_config` emit `config:changed` 唤醒 cron loop）。Tauri + HTTP `/api/knowledge/maintenance/*` + transport 双适配。
- **设置三件套**：`AppConfig.knowledge_maintenance: MaintenanceConfig`（默认全关）；GUI「设置 → 知识空间 → 自主维护」（[`KnowledgeMaintenanceSection`](../../src/components/settings/KnowledgeMaintenanceSection.tsx)，三态保存）；ha-settings `knowledge_maintenance` **HIGH 风险**（auto_approve = 审批策略 + 自主写用户库，技能须二次确认）+ SKILL.md 登记。审阅队列复用 [`KnowledgeMaintenanceButton`](../../src/components/knowledge/KnowledgeMaintenanceButton.tsx)（与失效链接 / 孤岛同面板，每条提案 ✓应用 / ✗忽略 + 一键全忽略 + Scan）。

## 前端（D13）

一级导航「知识空间」Tab（[`KnowledgeView.tsx`](../../src/components/knowledge/KnowledgeView.tsx)）：KB 列表 + 笔记树 + **CodeMirror 6 编辑器**（[`NoteEditor.tsx`](../../src/components/knowledge/NoteEditor.tsx)）+ Backlinks / 出链 / 标签面板 + 搜索 + 图谱视图。所有 invoke 走 transport 双适配（`call()` 泛型路径 + `transport-http.ts` COMMAND_MAP）。

**编辑器 5 模式**：`source` / `preview` / `split` / `live` / `outline`。预览复用 streamdown。外部 root 编辑器 `readOnly`（真正闸门是后端 `resolve_writable`）。`updateListener` 用 `applyingExternalRef` 区分程序化灌值 vs 用户编辑（否则打开笔记就被标脏）。

- **live-preview 模式（D13 视觉编辑评估落地）**：[`cm/livePreviewExtensions.ts::noteLiveDecorations`](../../src/components/knowledge/cm/livePreviewExtensions.ts)（`StateField`）遍历 markdown 语法树**就地隐藏语法符号**——ATX 标题 `#` + 空格（按级放大）、`**粗体**` / `*斜体*` / `~~删除线~~`、行内码反引号、无序列表标记替换为 `•` widget、引用 `>`；**光标 / 选区所在行还原 raw**（Obsidian 同款）；跳过代码块 / 图片子树 + `previewExtensions` 的图片 / 数学 span（避免重叠 replace）；>100KB 整体跳过。经 `liveComp` Compartment 按 `mode` 切换不重建编辑器。**这是 D13 的结论**：不引入 Milkdown / Tiptap（ProseMirror 往返序列化破坏 `.md` 唯一真相 / D14 offset / `note_patch` old-new / stale-write hash），改以 CM6 live 模式逼近所见即所得——与 Obsidian 自身（同为 CM6）一致、底层永远纯 `.md`。
- **源码内联预览**：`cm/previewExtensions.ts::notePreviewWidgets`（`StateField` 提供 `Decoration.replace`——块级数学 `$$…$$` 跨行，StateField 源豁免 CM6 跨行替换禁令）就地渲染图片（http(s) / data URI）与 KaTeX（`$…$` 走 pandoc 式规则避开散文金额；懒加载 `katex` 离线 CSP 安全）；选区 / 光标触及该 span 即撤销装饰还原原文；经 markdown 语法树跳过代码上下文；>100KB 整体跳过。
- **wikilink hover card**：`cm/wikilinkExtensions.ts::wikilinkHover`（`hoverTooltip` 300ms）悬停 `[[ref]]` 异步取目标标题 + 首段；走共享 [`noteRefFetch.ts`](../../src/components/knowledge/noteRefFetch.ts)（`${kbId}::ref` 缓存，hover 与嵌入共用一次请求）+ `transclusionParse.ts::noteExcerpt`。
- **heading outline 弹层**：纯函数 [`outline.ts::parseHeadings`](../../src/components/knowledge/outline.ts) → [`HeadingOutline`](../../src/components/knowledge/HeadingOutline.tsx) 弹层，点小节 `setRevealTarget({line})` 精确跳转。仅 `mode ∉ {preview, outline}` 显示。
- **AI 改写（owner 平面）**：标题栏 `Sparkles` 按钮取 `NoteEditor.getSelection()`（`forwardRef` + `useImperativeHandle`），有选区改选区否则整篇；[`AiRewriteDialog`](../../src/components/knowledge/AiRewriteDialog.tsx) → `kb_ai_rewrite_cmd`（`service::ai_rewrite` 走 `build_analysis_agent` + side_query，**不落盘**）→ `UnifiedDiffView` diff → 「应用」`replaceRange` splice 回编辑器。仅 `!readOnly && mode !== "preview"`。
- **选中引用到聊天**：标题栏 `MessageSquareQuote` 按钮构造 `[[relPath]]`（路径式 token）；有选区时复用 `parseHeadings` + offset→行定位最近上方标题追加 `[[relPath#Heading]]`。载荷 `KnowledgeMentionInsert{token, attachKbId}` 经 App `pendingChatInsert`（与 PlansView 共用通道）→ [`ChatScreen`](../../src/components/chat/ChatScreen.tsx) 消费：**非 incognito 时自动 attach 该 KB（read）**——已有 session 走 `attach_session_kb_cmd`、新会话 stage 进 `draftKbAttachments`（首发烘进 `chat` 载荷），否则 `effective_kb_access` 默认 deny 会让注入静默失效；**incognito 会话跳过 attach，token 照插**。后端零改动。
- **源码-预览同步滚动**：split 模式按滚动比例双向联动 `view.scrollDOM` ↔ 预览 div，一帧锁防回声。

**精确跳转**：反链点击 → `openNote(kb, srcRelPath, {line, col})`；搜索命中 → `openNote(kb, relPath, {line})`。`openNote` 设 `revealTarget`（每次新对象身份，重复点同位置也重触发）→ `NoteEditor` reveal effect（声明在 value-sync effect 之后）`scrollIntoView` + `EditorSelection.cursor` 滚到行 / 列。

**笔记交互**：新建走 Notion 式草稿态（标题框 + 空白正文，保存时命名回退链=标题框 → 首个 H1 → 弹窗）；全局 ⌘S / Ctrl+S；右键菜单（重命名 / 在文件夹中打开〔桌面专属 `supportsLocalFileOps` 闸门，复用 `reveal_in_folder`〕/ 删除）；header 文件名点击 inline 改名。**未保存保护**：切换笔记 / 空间 / 新建 / 返回，以及改名 / 移动当前脏笔记时先弹「保存 / 丢弃 / 取消」（`guardNavigation` 通用导航 + `guardEdit` 仅影响打开的脏笔记时拦截）；`openKbId` 跟踪笔记归属 + `handleSave` 闸门 + 协调 effect 防活动空间被换走后存错 KB。

**文件夹 = 真实目录**：索引只存 `.md`，空目录另走 `kb_list_dirs_cmd`（读盘 walk）补进 `buildNoteTree(notes, dirs)`；「新建文件夹」= `kb_mkdir_cmd`（不开草稿）；重命名 / 移动 / 拖拽 = `kb_rename_dir_cmd`（单次 fs rename 整目录 + `reindex_kb` 重对账）；删除 = `kb_delete_dir_cmd`（rm -rf + prune）。笔记拖拽 = `kb_note_rename_cmd`。

**空间（KB）管理**：KB 列表右键 编辑（名 + emoji，清空 emoji 发空串触发后端清 NULL）/ 归档·取消归档 / 删除；「显示归档」开关切 `list_kbs_cmd` 的 `includeArchived`。

**图谱视图**：标题栏 `Waypoints` 图标切 `graphMode`（per-KB 开关，与 per-note 的 source / split / preview 正交；开 note 自动退出）。中央 + 右侧整片换成 [`KnowledgeGraphView`](../../src/components/knowledge/KnowledgeGraphView.tsx)（`key={activeKbId}` 换 KB remount）——`react-force-graph-2d`（canvas 力导，纯 npm / 离线 / 无 CDN，**CSP 安全**）画 `kb_graph_cmd` 的 nodes + edges：节点按度数定大小、孤岛染琥珀、当前笔记描粉环、缩放够大才显标题、点节点 `onOpenNote`；`truncated` 时顶部提示。

- **拖拽固定 + 布局持久化**：拖节点 `onNodeDragEnd` 设 `fx/fy` 钉住（描翠绿环），debounce 600ms 经 `kb_graph_layout_save_cmd` 把**所有钉住节点**整体存到 `sessions.db.kb_graph_layout`（按 `rel_path` 键——index.db id 会随重建漂移，故不能用 id；落真相源 D9 而非可重建缓存）；开图时 `kb_graph_layout_get_cmd` 取回首建注入 `fx/fy`。「重置布局」按钮（有钉住节点才显）清全部 `fx/fy` + `d3ReheatSimulation` + 存空数组。**架构（点笔记不重排 + 不在 render 读 ref）**：`data` 只依赖 `[graph, layout]`（**不含 activePath**）——节点对象跨「点开笔记」稳定，force 引擎守住位置不整图重排，且 `data` useMemo 不读任何 ref（守 `react-hooks/refs`）；每次构建按 saved layout 给 pinned 节点种 `fx/fy`（布局必恢复）。当前笔记环走 `activePathRef`（canvas 绘制回调读，非 render）+ `activePath` 变化 `resumeAnimation()` 重绘。`nodesRef` / `graphKeyRef` 只在事件 / timeout 读写。重置走 `setFetched(layout:[])` 让 `data` 重建清空 `fx/fy`（不可变原则）。`zoomToFit` 每图一次且空图 settle 不消耗（`didFitRef` + `data.nodes.length` 守卫）；debounce save fire 时校验 `graphKeyRef.current === forKey` 跳过被刷新打断的陈旧保存。**已知 LOW**：删除 / 重命名笔记会留孤儿布局行（按 `rel_path` 键，`ON DELETE CASCADE` 只在删 KB 触发）——无害（加载时无匹配节点即忽略，下次 save-all 清除），但重命名笔记的钉固定位会丢、需重钉。

**笔记嵌入 transclusion**：预览 / 分屏的预览栏在有 `kbId` 时换 [`NoteTransclusionView`](../../src/components/knowledge/NoteTransclusionView.tsx)。纯函数 [`transclusionParse.ts`](../../src/components/knowledge/transclusionParse.ts)（`parseEmbedSegments` 跳代码围栏切出整行 `![[ref]]` 块、`stripFrontmatter`）把正文切成 markdown 段与 embed 段；embed **传完整 ref（含 anchor）** 经 `kb_note_read_ref_cmd`（owner resolver 单源）取目标——服务端按 anchor 切片；剥 frontmatter 后**递归**渲染，深度上限 4 + 循环检测 + broken / loading 占位。**循环检测 key = `relPath` + anchor**（`embedAnchor` 取）：whole 自嵌 `![[A]]` 判环，但 anchored 自嵌 `![[A#^p1]]` / `![[A#Heading]]`（切片是不同块 / 段）正常渲染，真递归仍判环。embed 结果按 `${kbId}::ref`（含 anchor）模块级缓存，随 `knowledge:changed` 整表失效。

**重建索引 UI**：三处入口——① 笔记树工具栏 🔄（重建当前空间，内联 spin + `N/M`）；② 三层右键「重建索引」：空间（`reindex_kb_cmd` → 进度 job）/ 文件夹（`reindex_dir_cmd` 同步 + toast）/ 笔记（`reindex_note_cmd` 同步 + toast）；③ header 右上「重建任务」图标（[`KnowledgeJobsButton.tsx`](../../src/components/knowledge/KnowledgeJobsButton.tsx)，悬浮面板列所有 `knowledge_reembed` 任务，scoped 到 knowledge kind，逐任务取消 / 重试 / 清除）。索引是 app 侧可重建缓存，故三层重建即使在只读外部 vault 上也可用（不受 `readOnly` 闸门约束）。

**维护面板**：标题栏听诊器图标（[`KnowledgeMaintenanceButton`](../../src/components/knowledge/KnowledgeMaintenanceButton.tsx)，悬浮面板）列当前空间的**失效链接**（点击跳源笔记到链接行 + 显示悬空 `[[target]]`）+ **孤岛笔记** + **维护提案审阅队列**（每条 ✓应用 / ✗忽略 + 一键全忽略 + Scan）；有失效链接时图标脉冲；听 `knowledge:changed` 刷新。rename / 移动后若改写了入站链接，toast 提示「已更新 N 处引用」。

**KB 绑定 UI（D10）**：会话级走聊天输入区 `KnowledgePicker`（popover，`list_session_kbs_cmd` + `attach/detach_session_kb_cmd`）；**项目级**走 `ProjectDialog` 编辑态的 [`ProjectKnowledgeSection`](../../src/components/chat/project/ProjectKnowledgeSection.tsx)（`list_project_kbs_cmd` + `attach/detach_project_kb_cmd`，每 KB 开关 + 读写切换，外部 vault 钳 read）。两者都是 owner 平面命令；`effective_kb_access` 取 `max(session, project)`。

**向量模型徽章**：知识空间视图标题栏 [`KnowledgeEmbeddingBadge`](../../src/components/knowledge/KnowledgeEmbeddingBadge.tsx) 显示当前 active embedding 模型名 / 「未开启」，点击跳「设置 → 知识空间」（[`KnowledgePanel`](../../src/components/settings/KnowledgePanel.tsx)：开关 + 模型选择复用 `EmbeddingActivationDialog` + 重建进度卡 + 折叠「高级 · 分块」区）；靠 `config:changed` 重载保持新鲜。

**弹窗交互**：所有带输入的弹窗包 `<form onSubmit>`、主按钮 `type="submit"`、取消 `type="button"`，回车确定性触发主操作（shadcn `Button` 默认 `type=submit`，无 form 时回车会落到 Radix `Close` ✕ 误触取消）。

## 与 Obsidian / Logseq 兼容性

目标是「**文件级 + 主流语法子集 + 非破坏性共存**」，不是功能完全等价。Obsidian（文档优先）与 Logseq（大纲优先）彼此都不完全兼容，「同时与两者 100% 兼容」物理上不成立；能做到且最有价值的是：**用户能用同一文件夹，既用 Hope Agent 知识空间、又用 Obsidian / Logseq 打开，互不破坏。**

| 特性 | Obsidian | Logseq | Hope Agent KB |
|---|---|---|---|
| 标准 `.md` 文件 | ✅ | ✅（也支持 org） | ✅ 真相源，不转写 |
| `[[wikilink]]` / 别名 | ✅（`\|`） | ✅ | ✅ |
| `[[link#heading]]` | ✅ | ✅ | ✅ |
| `#tag` | ✅ | ✅（tag≈page） | ✅ |
| YAML frontmatter | ✅ | ✅（也用 `key:: value`） | ✅ 读写 frontmatter |
| `![[嵌入]]` transclusion | ✅ | ✅ | ✅（含 `#^block` / `#heading` 切片） |
| 块引用 | `^block-id` | `((block-uuid))` + `id::` | ✅ 仅 Obsidian `^block-id`（读 + 写）；Logseq `((uuid))` 不做（D8） |
| 大纲（每行即 block） | ✗（文档优先） | ✅（大纲优先） | ⚠️ 文档优先 + 只读大纲视图（D8） |
| Callout `> [!note]` | ✅ | 部分 | ✅ 原样保留 |
| 配置目录 | `.obsidian/` | `logseq/` | 忽略不碰 |

**关键架构分叉（D8）**：以文档优先为基座（对齐 Obsidian），对 Logseq 做文件级 + 公共语法子集互通；深度大纲语义（block 树、`((block-ref))`）刻意不做——大纲优先模型与文档优先底座冲突。

## 安全红线

- **访问默认 deny + 显式 attach**；incognito 零访问 / 零写 / 零被动召回；IM 默认禁用，按账号 `kbAccessOptIn`（群聊加 per-chat `/kb` 确认）放开；外部 root 默认只读，`allow_external_writes` opt-in 解锁、后台维护永不写外部。
- **作用域闭合** `WorkspaceScope::for_knowledge`（canonicalize + starts_with，外部 root `read_only=true` 拒一切写，桌面也拒）；HTTP 写叠加 `allow_remote_writes`。
- **`index.db` 含明文 chunk 片段**（敏感度等同 `.md`，随数据目录权限走），**绝不存 API Key / Token / 凭据**。
- **注入即非可信**：`[[note]]` 注入与被动召回套 `<untrusted_external_data>` 信封 + 来源 + 截断，永不提升为 system 指令。
- **两鉴权平面物理隔离**：owner 平面（HTTP / Tauri）不经 `effective_kb_access` 看全部 KB；agent 平面（`note_*`）必过 `effective_kb_access`。KB 文件预览端点纯 owner、无 session 参数 / 无 fallback。

## 关联文档

- [Project 系统](project.md)——「文件即真实文件」哲学、`working_dir` 解析链、`WorkspaceScope` 三入口
- [记忆系统](memory.md)——FTS5 + vec 混合检索、Dreaming、Embedding 基建（知识空间复用）
- [文件操作统一](file-operations.md)——文件预览面板、preview-by-path 鉴权
- [配置系统](config-system.md)——`cached_config` / `mutate_config` 写契约
- [Side Query](side-query.md)——AI 提炼笔记的低成本推理入口
- [API 参考](api-reference.md)——知识空间 Tauri ↔ HTTP 接口对齐
