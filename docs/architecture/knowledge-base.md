# Knowledge Base（知识空间）

> 返回 [文档索引](../README.md) | 状态：**Phase 1 已实现** | 设计契约见 [`docs/plans/knowledge-base.md`](../plans/knowledge-base.md)（D1–D14 取舍、路线图、完整 rationale）

本文是落地后的实现描述；方向 / 取舍 / 路线图见设计契约。对外功能名「知识空间 / Knowledge Space」，代码内部中性：模块 `knowledge/`、工具 `note_*`、作用域 `for_knowledge`（D5）。

## 两类存储（D9）

- **真相源**：`KnowledgeRegistry`（[`knowledge/registry.rs`](../../crates/ha-core/src/knowledge/registry.rs)）— `knowledge_bases` + `session_knowledge_bases` + `project_knowledge_bases` 落 `sessions.db`，包 `Arc<SessionDB>` 复用连接（仿 `ProjectDB`/`ChannelDB`）。
- **可重建缓存**：`IndexDb`（[`knowledge/db.rs`](../../crates/ha-core/src/knowledge/db.rs)）— `note` / `note_chunk` / `note_link` / `note_tag` + FTS5(`note_chunk_fts`) + sqlite-vec(`note_vec`) 落 `~/.hope-agent/knowledge/index.db`。删了能从 `.md` 文件全量重建（连 `rel_path` 都是缓存）。连接模型仿 memory backend：1 写连接 + 4 读连接池 + WAL + sqlite-vec auto-extension。

笔记 = 真实 `.md` 文件（唯一真相源）。内部 KB（`root_dir=NULL`）落 `~/.hope-agent/knowledge/{id}/notes/`（lazy ensure），可写；外部绑定 vault（`root_dir` 非空）**Phase 1 只读**（D11）。

## 模块地图（`crates/ha-core/src/knowledge/`）

| 文件 | 职责 |
|---|---|
| `types.rs` | `KnowledgeBase` / `Note` / `NoteChunk` / `NoteLink` / `KbAccess` / 搜索结果类型 |
| `registry.rs` | KB CRUD + 访问绑定（真相源）+ `resolve_kb_dir`（内部 lazy ensure / 外部只读标记） |
| `db.rs` | index.db 后端：note/chunk/link/tag 写入（单事务重索引）+ FTS/vec 查询 + 反链 + 重解析 + `list_broken_links` / `list_orphan_notes`（维护面板）+ `all_resolved_links`（图谱边，WS1） |
| `parser.rs` | pulldown-cmark 扫 heading/code，正则扫 `[[ ]]` / `#tag`（跳过 code），D14 坐标（`PosMap` 码点 offset + 1-based line / 0-based col，相对原始全文）+ 手写 frontmatter→JSON |
| `chunker.rs` | 按 heading 分段 + 大小封顶（D12），产出 chunk（D14 坐标 + BLAKE3 content_hash + overlap body）。参数 `ChunkConfig { max_chars, overlap_chars }` 由 `chunk(full, parsed, &cfg)` 传入（默认 1500/80，`clamped()` 钳 `[200,8000]`/`[0,max/2]`） |
| `resolver.rs` | `[[ref]]` → note_id 确定性规则（路径式 > 唯一 basename > 最短路径再字典序，NFC + 大小写不敏感，**不用 mtime**，#8） |
| `rename.rs` | note/folder 改名移动 + **入站 `[[ ]]` 链接改写**（#9）：`rename_note` / `rename_dir` 复用给 owner 平面 + agent 工具；纯文本变换 `rewrite_content`（re-parse 跳 code、按 D14 码点 offset 定位 splice、保留 `#anchor`/`|alias`/`![[ ]]`，路径式→新路径、basename→新 stem，新 stem 歧义时退回路径式）单测无全局依赖 |
| `index.rs` | 索引器：文件 → parse → chunk → embed → IndexDb；KB reconcile（mtime 增量 + prune）；全局 `IndexDb` |
| `watcher.rs` | `notify` 生产级 watcher（debounce 800ms，仅 `.md` 事件，per-KB 线程，外部 vault 实时同步，D6） |
| `access.rs` | `effective_kb_access(KnowledgeAccessContext)`（D10）：incognito short-circuit → IM 全链归零 → `max(session,project)` → 滤 archived → 外部 cap read |
| `search.rs` | chunk 级 FTS+vec → RRF → MMR → 聚合回 note（算法复用 memory，独立 store，D7） |
| `graph.rs` | 链接图谱构建（WS1，纯变换）：`build_kb_graph`（节点=笔记+度数，边=去重 resolved 链接，丢自环）/ `ego_subgraph`（N 跳无向邻域）/ `cap_nodes`（按度数截断，标 `truncated`）；owner 图与 `note_graph` 工具共用，单测无全局依赖 |
| `service.rs` | owner 平面操作（GUI/HTTP）：list/read/save/delete/**rename**/backlinks/search/**broken_links**/**orphans**/**graph**/**note_read_ref**，不经 `effective_kb_access`。`note_rename` / `rename_dir` 委托 `rename::*`：移动文件后**改写别处笔记里指向它的 `[[ ]]`**（#9，返回 `RenameOutcome { newRel, filesChanged, linksRewritten }`）；外部 root 只读拒写。`graph` = `build_kb_graph` + `cap_nodes(2000)`；`note_read_ref` 经 `resolver` 把 `[[ ]]` ref 解析成笔记再 `note_read`（transclusion 单一解析源，broken 返 `None`） |
| `inject.rs` | 读取桥通道①：用户消息 `[[note]]` 确定性注入（`untrusted_external_data` 信封，受 `effective_kb_access` 约束，#7） |

`mod.rs` 提供 `blake3_hex`（D14 hash 契约：BLAKE3 over raw bytes）+ `delete_kb_cascade`（registry 事务 + index prune + 内部目录 rm-rf，外部 root 永不删）。

## 两个鉴权平面（D10）—— 物理隔离

| 平面 | 在哪层 | 主体 / 鉴权 |
|---|---|---|
| **Owner / 管理** | HTTP 端点 / Tauri 命令（`service.rs`） | owner（桌面本机信任 / HTTP API key=owner-equivalent），看自己**所有** KB，**不经 attach** |
| **Agent / session** | ha-core 工具执行（`note_*`，进程内） | turn 内 agent；`effective_kb_access(ctx)`（session + source + 全链 cap + incognito） |

KB 文件预览端点 `/api/knowledge/{kb_id}/files/*` = 纯 owner 平面，**无 session 参数、无 fallback**，与 `/api/sessions/{id}/files/*` 不互相放宽。`note_*` 工具读笔记不经 HTTP 端点（ha-core 内返回内容）。

**source-aware**：`ChatSource{Desktop|Http|Channel|Subagent|ParentInjection}`（不在 `ToolExecContext` 上）经 `configure_agent` 映射成 `KbAccessSource` 透传到 `AssistantAgent.chat_source` → `ToolExecContext.chat_source`。IM(`Channel`) → KB 访问归零（即便 project attach）；incognito 由 `is_session_incognito(session_id)` short-circuit。**血缘 origin 已真接线**：`ChatEngineParams.origin_source`（顶层 `None`→origin=source）→ `configure_agent(kb_origin)` → `agent.origin_chat_source` → `ToolExecContext.origin_chat_source`；`subagent` 工具 spawn 时把父轮 `ctx.origin_chat_source.or(chat_source)` 经 `SpawnParams.origin_source` 透传给子 `ChatEngineParams.origin_source`，`effective_kb_access` 的 cap 查 `source.is_im() || origin_source.is_im()`，故 IM-origin 子代理被归零。**双重防线**：即便不接线，子代理子会话也无 attach / 无 project_id（`create_session_with_parent` 不继承）→ 天然空集；origin cap 是面向未来（若子代理改为继承 project）的纵深防御。`access.rs` 带短路规则单测。系统发起的 spawn（plan/team/hooks/fork skill）`origin_source=None`，靠会话隔离。

## 工具面（Layer 1，`tools/note.rs`）

`note_create / read / update / patch / append / delete / search / link / backlinks / by_tag / tags`（Phase 1）+ `note_rename / move / set_frontmatter / broken_links / orphans`（Phase 2 Batch A）+ `note_graph`（Batch B）+ `note_similar / related / suggest_links`（Batch C WS4）+ `note_distill / moc / session_to_note`（Batch C WS5），`Core{Interaction}` tier、`internal=false`（过权限引擎 + plan-mode）。`kb` 过 `effective_kb_access`：写需 write + 内部 root + 全链允许 + 非 incognito；读 `kb?` 省略时只搜可访问集合（跨 KB 同名返 disambiguation）。`note_broken_links` / `note_orphans` 的 `kb` 必填（per-KB 维护报告）。

- `note_rename` / `note_move`（别名，共用 handler）：移动 `.md` + **改写入站 `[[ ]]`**（#9，`knowledge::rename_note`）。`note_set_frontmatter({kb,path,props})`：合并写 YAML frontmatter（`props` 的 `null` 值删键）。`parser::merge_frontmatter` 走**逐行非破坏性编辑**——只重写 `props` 命中的顶层键、其余行（含极简 parser 表达不了的嵌套 map / 块标量）原样保留、原键序不变、类型保真（reserved/数字串自动加引号）；全删则丢整个 frontmatter 围栏。`note_broken_links` / `note_orphans` 复用 `db::list_broken_links` / `list_orphan_notes`。
- `note_graph({kb?, note?, depth?})`（WS1）：复用 `graph::build_kb_graph`。给 `note` → `ego_subgraph`（depth 1–3，默认 1，跨可访问集合 resolve 出 kb）；不给 → 全 KB 图 `cap_nodes(200)`（`truncated` 标截断）。`kb` 可省（`note` 钉死 kb，或仅一个可访问 KB）。输出 `{kbId, nodeCount, edgeCount, truncated, nodes, edges}`。
- **智能检索（Batch C WS4，纯检索无 LLM）**：`note_similar`（`search::similar_notes` 向量 KNN，aggregate 到 note 排除自身；无 embedder/signature 时返空 + 提示开 embedding）/ `note_related`（融合 backlinks ∪ resolved 出链 ∪ 同标签 ∪ 向量近邻，按命中信号加权排序、带 `reasons`）/ `note_suggest_links`（`strip_links_and_code` 去码块/inline code/已有 `[[ ]]` 后 `contains_word` 词界匹配其它 note 的 title/basename，排除已链接，cap 5000 候选 / 25 建议）。三者复用 `read_resolved_note`。
- **AI 高阶（Batch C WS5，side_query 驱动 + 写）**：经 `run_kb_side_query`（`recap::report::build_analysis_agent` + `side_query`，与 recall-summary / dreaming 同源，与主对话 agent 解耦）。`note_distill`（`source` 笔记或 `text` 原文 → JSON 数组解析 `parse_distilled` → 建 2–8 篇原子笔记，`slugify` + `unique_rel_path` 防覆盖、frontmatter `title`/`tags` 经 `yaml_inline` 引号保真）/ `note_moc`（按 `topic`（hybrid search）/`tag`（notes_by_tag）聚合 → 生成 MOC markdown → 写 `MOCs/<slug>.md`，frontmatter 标 `moc: true`；重写**只刷新自己生成的 MOC**，撞到同名用户笔记时退回 `unique_rel_path` 不覆盖；同名 basename 的相关笔记用路径式 ref 消歧）/ `session_to_note`（`session` 或当前会话 → `load_session_messages` 拼 user/assistant 转录 → 生成结构化笔记；**无痕会话源直接拒**守「关闭即焚」）。均 `require_write` + `writable_scope`（外部 root 拒，且在 LLM 调用前 fail-fast）。

**stale-write guard（强契约）**：`expected_file_hash` 比**磁盘当前 raw BLAKE3**（不比 `note.content_hash` 索引缓存）。`note_patch` 走 `old/new` 文本唯一命中（0/多次都拒，仿 `edit`，D14 坐标不做 patch 寻址）。

## 检索 / 索引数据流

写入（内部 KB / owner 保存 / 工具）：写盘 → `index::reindex_note`（parse → chunk → embed → `replace_note_index` 单事务，FTS 触发器同步、vec 手动同步）→ `reresolve_kb_links`（全 KB 重解析，broken↔resolved 翻转）→ emit `knowledge:changed`。
外部 vault：bind/启动/打开 `reindex_kb`（mtime 增量 + prune）+ `notify` watcher 实时 reconcile。
检索：`search_notes` → chunk FTS5(BM25) + vec0 KNN（signature 过滤）→ 加权 RRF(text 0.4/vec 0.6/k 60)→ 聚合 best-chunk 回 note → MMR(λ0.7)。向量单存 `note_vec`。

## Embedding 配置（D7，独立 selector）

知识空间的向量化**不寄生记忆**——有自己完整的配置生命周期，记忆没配 / 关了都不影响知识空间向量检索（关了只降级 FTS-only，不回退到 `memory_embedding`）。

- **配置三层**（与 memory 对称，共享底层）：
  - `AppConfig.embedding_models: Vec<EmbeddingModelConfig>` —— **共享命名模型库**（provider/apiKey/model/dims），memory 与 knowledge 同一份，apiKey 配一次两处可选。
  - `AppConfig.knowledge_embedding: EmbeddingSelection` —— 知识空间**独立**选择器（`enabled` / `model_config_id` / `active_signature` / `last_reembedded_signature`），结构与 `memory_embedding` 同（都是泛化的 `EmbeddingSelection`）。
  - 运行时经 `resolve_memory_embedding_config(&knowledge_embedding, &embedding_models)` 解析成 provider（纯函数，名字带 memory 但通用）。
- **helper**（[`knowledge/embedding.rs`](../../crates/ha-core/src/knowledge/embedding.rs)）：`get_knowledge_embedding_state` / `knowledge_active_embedding_signature`（索引 + 检索的热路径签名源，**不读** `memory::active_embedding_signature`）/ `set_knowledge_embedding_default`（验证 provider → 写 selection → 装 index embedder → spawn reembed）/ `disable_knowledge_embedding`（pause 语义，清 index embedder）/ `apply_knowledge_embedding_from_config`（热重载）。
- **复用 memory 的纯工具**：`create_embedding_provider` 工厂、`EmbeddingProvider` trait、`EmbeddingModelConfig::signature()`、`embedding_cache`（按 signature 命中——同模型与 memory 共享缓存）、RRF/MMR 算法。
- **重建**（[`knowledge/reembed.rs`](../../crates/ha-core/src/knowledge/reembed.rs)）：切模型 → `set_knowledge_embedding_default` 装新 embedder（维度变则 `note_vec` DROP 重建）→ spawn `LocalModelJobKind::KnowledgeReembed`，遍历所有 KB `reindex_kb(full=true)` 重 embed 全部 chunk，进度 KB-granular，完成写 `last_reembedded_signature`。复用 memory 的 `local_model_jobs` 框架（取消 / 单实例 / 进度 / retry 派发）。
- **分块配置（D12，高级）**：`AppConfig.knowledge_chunk: ChunkConfig`（`max_chars` / `overlap_chars`，`clamped()` 钳 `[200,8000]` / `[0,max/2]`）。owner 命令 `knowledge_chunk_{get,set}_cmd` / HTTP `GET|POST /api/knowledge/chunk`；`service::set_chunk_config` 写 config + 触发 `start_knowledge_reembed_job(Some(all_ids))` 全 KB 重切（**向量开→重嵌、关→FTS-only re-chunk；不 stamp signature**——chunk 改动不是模型覆盖事件，不应清 `needsReembed`）。`reindex_kb(full=true)` 强制跳过 mtime 短路确保每篇都用新参数重切。GUI 在 `KnowledgePanel` 折叠「高级 · 分块」区（两个数字输入 + 三态保存 + 全量重建警告），与 `knowledge_embedding` 同归 **GUI-only**（不进 `ha-settings`，重 reindex 副作用）。
- **共享库交叉保护**：`save_embedding_model_config` / `delete_embedding_model_config` / Ollama 删模型清理都对 memory **与** knowledge 的 active model 双向守门（改 active model signature / 删 active model 一律拒；删 Ollama active 重置对应 selection + 清对应 embedder）。
- **owner 平面**：命令 `knowledge_embedding_{get,set_default,disable}_cmd`（[`commands/knowledge.rs`](../../src-tauri/src/commands/knowledge.rs)）/ HTTP `GET /api/knowledge/embedding`、`POST /api/knowledge/embedding/{set-default,disable}`（[`routes/knowledge.rs`](../../crates/ha-server/src/routes/knowledge.rs)）。与 `memory_embedding` 一致**不进 `ha-settings`**（模型选择 + reembed 副作用，类比 `active_model` 的 GUI-only 豁免）。
- **GUI**：「设置 → 知识空间」（[`KnowledgePanel`](../../src/components/settings/KnowledgePanel.tsx)）——开关 + 模型选择（复用 `EmbeddingActivationDialog`，模型库空时折叠成「去配置」CTA，跳 `settings:navigate {section:"modelConfig", modelTab:"embeddingModels"}` 共享 embedding 模型库——含 `LocalEmbeddingAssistantCard` 推荐 + 下载本地模型，与记忆同一套）+ 重建进度卡。知识空间视图标题栏的**向量模型徽章**（[`KnowledgeEmbeddingBadge`](../../src/components/knowledge/KnowledgeEmbeddingBadge.tsx)，替代旧齿轮）显示当前 active embedding 模型名 / 「未开启」，点击走 `onOpenSettings()`（App 接成 `handleOpenSettings("knowledge")`）跳「设置 → 知识空间」；靠 `config:changed` 重载保持新鲜。

## 前端（D13）

一级导航「知识空间」Tab（[`src/components/knowledge/KnowledgeView.tsx`](../../src/components/knowledge/KnowledgeView.tsx)）：KB 列表 + 笔记树 + **CodeMirror 6 编辑器**（[`NoteEditor.tsx`](../../src/components/knowledge/NoteEditor.tsx)，Source/Preview/Split 三模式，`[[`/`#` 补全、wikilink chip decoration、broken-link lint，预览复用 streamdown）+ Backlinks/出链/标签面板 + 搜索。外部 root 编辑器 `readOnly`（真正闸门是后端 `resolve_writable`）。CM6 是新增前端依赖。所有 invoke 走 transport 双适配（`call()` 泛型路径 + `transport-http.ts` COMMAND_MAP）。

**笔记交互**：新建走 Notion 式草稿态（标题框 + 空白正文，保存时命名回退链=标题框 → 正文首个 H1 → 弹窗）；全局 ⌘S/Ctrl+S 保存；右键菜单（重命名 / 在文件夹中打开〔桌面专属，`supportsLocalFileOps` 闸门，复用 `reveal_in_folder`〕/ 删除）；header 文件名点击 inline 改名。**未保存保护**：切换笔记/空间/新建/返回，以及改名/移动「当前未保存笔记」时，先弹「保存/丢弃/取消」（`guardNavigation` 通用导航 + `guardEdit` 仅当影响打开的脏笔记时拦截）；`openKbId` 跟踪笔记归属 + `handleSave` 闸门 + 协调 effect 防活动空间被换走后存错 KB。`NoteEditor` 的 `updateListener` 用 `applyingExternalRef` 区分**程序化灌值 vs 用户编辑**（否则打开笔记就被标脏）。

**精确跳转（G3）**：反链点击 → `openNote(kb, srcRelPath, {line: srcStartLine, col: srcStartCol})`；搜索命中点击 → `openNote(kb, relPath, {line: startLine})`。`openNote` 设 `revealTarget`（每次新对象身份，重复点同位置也重触发）→ `NoteEditor` 的 reveal effect（声明在 value-sync effect 之后，确保 doc 已更新）`EditorView.scrollIntoView` + `EditorSelection.cursor` 滚到行/列(钳边界)。出链指向**其它**笔记、无目标内行号,跳顶部即正确,不接精确跳转;preview-only 模式无 view → no-op。

**文件夹 = 真实目录**：索引只存 .md，所以空目录另走 `kb_list_dirs_cmd`（读盘 walk）补进 `buildNoteTree(notes, dirs)`；「新建文件夹」= `kb_mkdir_cmd` 建真实目录后刷新即显示（**不再开草稿**）；文件夹重命名/移动（含拖拽）= `kb_rename_dir_cmd`（**单次 fs rename 整目录** + `reindex_kb` 重对账/重解析，避免遗留空目录）；删除 = `kb_delete_dir_cmd`（rm -rf + prune）。笔记拖拽到文件夹/根 = `kb_note_rename_cmd`。

**空间（KB）管理**：KB 列表右键 编辑（名+emoji，清空 emoji 发空串触发后端清 NULL）/ 归档·取消归档 / 删除；「显示归档」开关切 `list_kbs_cmd` 的 `includeArchived`。

**维护面板（Phase 2 Batch A）**：标题栏听诊器图标（[`KnowledgeMaintenanceButton`](../../src/components/knowledge/KnowledgeMaintenanceButton.tsx)，悬浮面板，仿 `KnowledgeJobsButton`）列当前空间的**失效链接**（点击跳源笔记到链接行 + 显示悬空 `[[target]]`）+ **孤岛笔记**（无任何 resolved 链接，点击打开）；有失效链接时图标脉冲；`kb_broken_links_cmd` / `kb_orphans_cmd`，听 `knowledge:changed` 刷新。rename/移动后若改写了入站链接，toast 提示「已更新 N 处引用」。

**图谱视图（Phase 2 Batch B，WS1）**：标题栏 `Waypoints` 图标切 `graphMode`（per-KB 开关，与 per-note 的 source/split/preview 正交；开 note 自动退出）。开启时中央+右侧整片换成 [`KnowledgeGraphView`](../../src/components/knowledge/KnowledgeGraphView.tsx)（`key={activeKbId}` 换 KB remount）——`react-force-graph-2d`（canvas 力导，纯 npm/离线/无 CDN，**CSP 安全**）画 `kb_graph_cmd` 的 nodes+edges：节点按度数定大小、孤岛染琥珀、当前笔记描粉环、缩放够大才显标题（防糊），点节点 `onOpenNote`；`refreshKey`（= `embedCacheKey`）随 `knowledge:changed` 重取；`truncated` 时顶部提示。

**笔记嵌入 transclusion（Phase 2 Batch B，WS2）**：预览/分屏的预览栏在有 `kbId` 时换 [`NoteTransclusionView`](../../src/components/knowledge/NoteTransclusionView.tsx)（否则退回纯 `MarkdownRenderer`）。纯函数 [`transclusionParse.ts`](../../src/components/knowledge/transclusionParse.ts)（`parseEmbedSegments` 跳代码围栏切出**整行** `![[ref]]` 块、`stripFrontmatter`，独立可测）把正文切成 markdown 段与 embed 段；embed 经 `kb_note_read_ref_cmd`（owner resolver 单源）取目标、剥 frontmatter 后**递归**渲染，深度上限 4 + `seen` rel-path 循环检测（顶层用当前笔记 path 预置，挡自嵌入）+ broken/loading 占位。embed 结果按 `${kbId}::ref` 模块级缓存，`cacheBustKey`（KnowledgeView 的 `embedCacheKey`，随 `knowledge:changed` ++）整表失效。行内 `![[ ]]`（非独行）仍按原文渲染。

**编辑器增强（Phase 2 Batch D，WS9）**：均挂在 CM6 [`NoteEditor`](../../src/components/knowledge/NoteEditor.tsx) 底座上，源文档始终是唯一真相。
- **wikilink hover card**：`cm/wikilinkExtensions.ts::wikilinkHover`（`hoverTooltip`，300ms）悬停 `[[ref]]` 异步取目标标题 + 首段；解析器由 host 注入（`getKbId` 门控 + `fetchExcerpt`），走共享 [`noteRefFetch.ts`](../../src/components/knowledge/noteRefFetch.ts)（从 transclusion 抽出的 `${kbId}::ref` 缓存，hover 与嵌入共用一次请求）+ `transclusionParse.ts::noteExcerpt`（剥 frontmatter/标题取首段、码点截断）。
- **heading outline**：纯函数 [`outline.ts::parseHeadings`](../../src/components/knowledge/outline.ts)（ATX `#`×1–6、≤3 缩进、跳代码围栏、剥尾随 `#`，独立可测）→ [`HeadingOutline`](../../src/components/knowledge/HeadingOutline.tsx) 弹层（`useClickOutside` 范式），点小节 `setRevealTarget({line})` 复用 G3 精确跳转。仅 `mode !== "preview"` 显示。
- **源码-预览同步滚动**：split 模式按滚动比例双向联动 `view.scrollDOM` ↔ 预览 div，一帧锁防回声；仅 `[mode]` 重绑（编辑器仅在源码可见性切换时重建，scrollDOM 跨编辑稳定）。
- **源码内联预览**：`cm/previewExtensions.ts::notePreviewWidgets`（**`StateField` 提供 `Decoration.replace`**——块级数学 `$$…$$` 跨行，CM6 禁止 *plugin* 提供的跨行替换装饰，StateField 源豁免）就地渲染图片（http(s)/data URI）与 KaTeX（`$…$` 走 pandoc 式规则避开散文金额；懒加载 `katex`，npm 打包离线 CSP 安全）；**选区/光标触及该 span 即撤销装饰还原原文**（doc/selection 变更重建）；经 **markdown 语法树跳过代码上下文**（`FencedCode`/`CodeBlock`/`InlineCode`——示例 Markdown/LaTeX 不被渲染成 widget）；超大文档（>100KB）整体跳过保打字流畅。
- **AI 改写（WS9.5，owner 平面）**：标题栏 `Sparkles` 按钮取 `NoteEditor` 暴露的 `getSelection()`（`forwardRef` + `useImperativeHandle`），有选区改选区否则整篇；[`AiRewriteDialog`](../../src/components/knowledge/AiRewriteDialog.tsx) 输入指令 → `kb_ai_rewrite_cmd`（`service::ai_rewrite` 走 `build_analysis_agent` + side_query，**不落盘**）→ 复用 `UnifiedDiffView` 看 diff → 「应用」经 `replaceRange` splice 回编辑器（触发 `onChange` 标脏）→ 用户照常 `note_save`。仅 `!readOnly && mode !== "preview"`。

**KB 绑定 UI（D10）**：会话级走聊天输入区 `KnowledgePicker`（popover，`list_session_kbs_cmd` + `attach/detach_session_kb_cmd`）；**项目级**走 `ProjectDialog` 编辑态的 [`ProjectKnowledgeSection`](../../src/components/chat/project/ProjectKnowledgeSection.tsx)（`list_project_kbs_cmd` + `attach/detach_project_kb_cmd`，每 KB 开关 + 读写切换，外部 vault 钳 read）。两者都是 owner 平面命令；`effective_kb_access` 取 `max(session, project)`，故项目绑定让该项目下所有会话可检索/引用。

**重建索引 UI**：三处入口——① 笔记树工具栏 🔄（重建当前空间，内联 spin + `N/M`）；② 三层右键「重建索引」：空间（`reindex_kb_cmd` → 进度 job，与工具栏 🔄 同源）/ 文件夹（`reindex_dir_cmd`，同步 + toast）/ 笔记（`reindex_note_cmd`，同步 + toast）；③ header 右上「重建任务」图标（[`KnowledgeJobsButton.tsx`](../../src/components/knowledge/KnowledgeJobsButton.tsx)）——悬浮面板列所有 `knowledge_reembed` 任务（`local_model_job_list` 种子 + LocalModelJobs 事件流 live，**scoped 到 knowledge kind**，含终态历史），逐任务 取消/重试/清除（复用 `local_model_job_{cancel,retry,clear}`），有活动任务时图标脉冲。索引是 app 侧可重建缓存，故三层重建即使在只读外部 vault 上也可用（不受 `readOnly` 闸门约束）。文件夹/笔记同步重建不产生 job，故不出现在「重建任务」面板。

**弹窗交互**：所有带输入的弹窗包 `<form onSubmit>`、主按钮 `type="submit"`、取消 `type="button"`，回车确定性触发主操作（shadcn `Button` 默认 `type=submit`，无 form 时回车会落到 Radix `Close` ✕ 误触取消）。

## 安全红线

- 访问默认 deny + 显式 attach；incognito 零访问/零写/零被动召回；IM Phase 1 禁用。
- 作用域闭合 `WorkspaceScope::for_knowledge`（canonicalize + starts_with，外部 root `read_only=true` 拒一切写，桌面也拒）；HTTP 写叠加 `allow_remote_writes`。
- `index.db` 含明文 chunk 片段（敏感度等同 `.md`），**绝不存凭据**。
- 注入即非可信：`[[note]]` 注入套 `<untrusted_external_data>` 信封 + 来源 + 截断，永不提升为 system 指令。
