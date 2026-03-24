# 记忆系统增强 — 设计文档 & 交接手册

> 版本: 1.1
> 日期: 2026-03-24
> 状态: Phase 1（MVP Gap 修复）+ Phase 1.5（优化项）已完成，Phase 2（GraphRAG）待调研

---

## 1. 项目背景

### 1.1 核心诉求
- 用户成本极低（纯本地、无云、无订阅、低硬件门槛、低 token 消耗）
- 显著优于 Claude Code 当前的记忆机制（Markdown + compaction 易失真、召回不准、实体关联弱）
- Rust 优势：性能高、内存安全、低资源占用、适合本地长寿 Agent
- GUI 可视化记忆管理（查看、编辑、修改）

### 1.2 三档梯度策略（用户可切换）

| 后端 | 定位 | 实现 | 精度提升 | 适用场景 |
|------|------|------|---------|---------|
| **QMD-like（默认）** | 极致轻量 | SQLite + FTS5 + sqlite-vec 混合检索 | +30–50% | 低配机器、日常使用 |
| **GraphRAG-rs** | 高精度/知识图谱 | entity-relation 提取 + triple 存储 + 图遍历 | +70–100% | 长期项目、需要学习 |
| **Hindsight** | SOTA | hindsight-client + 嵌入式 Postgres + pgvector | 91.4% LongMemEval | 极致追求准确度 |

### 1.3 实现路径
- **短期（MVP）** ✅ 已完成 — QMD-like SQLite hybrid + GUI 基本管理
- **短期（优化）** ✅ 已完成 — Prompt Summary 加权 + 提取模型可配 + Toast 通知 + 去重阈值可配 + 统计仪表板
- **中期** ⬜ 未开始 — GraphRAG-rs 调研 + 集成
- **长期** ⬜ 未开始 — Hindsight client 集成 + 高级可视化

---

## 2. 已完成工作（Phase 1: MVP Gap 修复）

### 2.1 Fix 1: Embedder 自动初始化 ✅

**问题**: `save_embedding_config` 只存 config.json 不调 `set_embedder()`；启动时不初始化 embedder。

**改动**:
- `memory.rs`: `MemoryBackend` trait 新增 `set_embedder()` / `clear_embedder()` / `has_embedder()` 默认方法（no-op），`SqliteMemoryBackend` 实现具体逻辑
- `lib.rs`: 新增启动时自动初始化逻辑（MEMORY_BACKEND 初始化后立即检查 config，失败只 warn 不阻塞）
- `lib.rs`: `save_embedding_config` 保存后立即 apply embedder（enabled → set, disabled → clear）

**关键文件**:
- `src-tauri/src/memory.rs` — trait 方法定义 + SqliteMemoryBackend 实现
- `src-tauri/src/lib.rs` — 启动初始化（约 L3034）+ save_embedding_config 改造（约 L2475）

---

### 2.2 Fix 2: 记忆去重检测 ✅

**问题**: `add()` 无条件插入，agent 可能重复保存相同信息。

**改动**:
- `memory.rs`: 新增 `AddResult` enum（Created/Duplicate/Updated）、常量 `DEDUP_THRESHOLD_HIGH`(0.02) / `DEDUP_THRESHOLD_MERGE`(0.012)
- `memory.rs`: 新增 trait 方法 `find_similar()` — 复用 search() 的 FTS5+向量混合检索，按 RRF score 过滤
- `memory.rs`: 新增 trait 方法 `add_with_dedup()` — 高相似跳过、中等相似合并内容+标签、低相似正常插入
- `tools/memory.rs`: `tool_save_memory` 改用 `add_with_dedup()`，返回信息区分三种结果
- `lib.rs`: 新增 Tauri 命令 `memory_find_similar`
- `MemoryPanel.tsx`: 手动添加时先调 `memory_find_similar`，发现相似弹出确认对话框（仍然添加/更新已有/取消）

**关键文件**:
- `src-tauri/src/memory.rs` — AddResult、find_similar、add_with_dedup 实现
- `src-tauri/src/tools/memory.rs` — tool_save_memory 改造
- `src/components/settings/MemoryPanel.tsx` — 去重确认 UI

**阈值说明**:
- RRF score > 0.02 → 判为重复，跳过
- RRF score 0.012–0.02 → 中等相似，合并到已有条目
- RRF score < 0.012 → 不相似，正常插入
- 这些阈值是基于 RRF 的经验值，可能需要根据实际使用调优

---

### 2.3 Fix 3: 导入 + 批量操作 ✅

**问题**: 只有 markdown 导出，无导入；只能逐条删除。

**改动**:

**后端 (`memory.rs`)**:
- `ImportResult` struct: `{ created, skipped_duplicate, failed, errors }`
- `delete_batch(ids)` — 单条 SQL `DELETE WHERE id IN (...)`，同步清理 vec0 表
- `import_entries(entries, dedup)` — 遍历 entries，可选去重，逐条处理
- `reembed_all()` / `reembed_batch(ids)` — 重新生成 embedding，调用 `reembed_entries()` helper
- `parse_import_json(json_str)` — JSON 数组格式：`[{ content, type?, scope?, tags? }]`
- `parse_import_markdown(md_str)` — Markdown 格式：按 `## Type Heading` / `### Entry` 解析

**后端 (`lib.rs`)**:
- 新增 3 个 Tauri 命令: `memory_delete_batch` / `memory_import` / `memory_reembed`

**前端 (`MemoryPanel.tsx`)**:
- 多选模式: 列表项增加 checkbox（hover 显示，选中后常驻），`selectedIds` Set 状态管理
- 批量操作栏: 选中时显示 "删除所选(N)" + "重新生成向量(N)" 按钮
- 导入按钮: Upload 图标，文件选择器（.json/.md），调 `memory_import`
- Re-embed All: Embedding 设置页底部新增按钮

**关键文件**:
- `src-tauri/src/memory.rs` — ImportResult、batch ops、parsers（文件末尾约 L1440+）
- `src/components/settings/MemoryPanel.tsx` — multi-select + batch bar + import handler

**导入格式示例**:

JSON:
```json
[
  { "content": "User prefers dark mode", "type": "user", "tags": ["preference"] },
  { "content": "Project uses React 19", "type": "project", "scope": "agent", "agentId": "default" }
]
```

Markdown（与 `export_markdown` 输出格式兼容）:
```markdown
## About the User
### User prefers dark mode
Tags: preference
Scope: global | Source: user | Updated: 2026-03-24

User prefers dark mode

---
```

---

### 2.4 Fix 4: 自动记忆提取 ✅

**问题**: 记忆只能通过 agent 调用 save_memory 工具或用户手动添加。

**改动**:

**新模块 `memory_extract.rs`**:
- 提取 prompt（~200 tokens）：指示 LLM 从对话提取值得记住的信息，返回 JSON 数组
- `run_extraction(messages, agent_id, session_id, provider_config, model_id)` — 异步入口
- `do_extraction()` — 核心逻辑：取最近 6 条消息 + 已有记忆摘要 → LLM 调用 → JSON 解析 → `add_with_dedup` 保存
- `extract_json_array()` — 鲁棒 JSON 解析（处理 markdown fences、额外文本）
- `extract_text_content()` — 支持 string 和 Anthropic array content 格式
- 保存成功后 emit `memory_extracted` Tauri 全局事件（count, agentId, sessionId）

**Chat hook (`lib.rs`)**:
- 在 model chain 的 chat 成功返回路径（约 L1730），`tokio::spawn` 异步执行提取
- 检查条件：`autoExtract` 开启 + 对话轮数 >= `extract_min_turns * 2`
- 复用当前 provider/model 做 LLM 调用

**Agent 配置 (`agent_config.rs`)**:
- `MemoryConfig` 新增 `auto_extract: bool`（默认 false）、`extract_min_turns: usize`（默认 3）

**前端 (`MemoryPanel.tsx`)**:
- Agent 模式下列表顶部显示 "自动提取记忆" Switch 开关
- 直接读写 agent config（`get_agent_config` → `save_agent_config_cmd`）

**关键文件**:
- `src-tauri/src/memory_extract.rs` — 完整模块（约 220 行）
- `src-tauri/src/lib.rs` — chat 成功路径的 spawn hook
- `src-tauri/src/agent_config.rs` — MemoryConfig 扩展
- `src/components/settings/MemoryPanel.tsx` — auto-extract toggle

**Token 成本**:
- 输入: ~1500 tokens（prompt + 6 条消息摘要 + 已有记忆）
- 输出: ~300 tokens
- 使用便宜模型: < $0.001/次

**注意事项**:
- 提取失败不影响 chat 响应（tokio::spawn 隔离）
- 提取使用与 chat 相同的 provider/model，不单独配置模型
- 若需要使用更便宜的模型做提取，需要后续扩展 `MemoryConfig` 添加 `extract_model` 字段

---

## 3. 新增 i18n Keys（zh + en）

| Key | 英文 | 中文 |
|-----|------|------|
| memoryDuplicateFound | Similar memories found... | 发现相似记忆... |
| memoryAddAnyway | Add Anyway | 仍然添加 |
| memoryUpdateExisting | Update | 更新 |
| memorySimilarity | Similarity | 相似度 |
| memoryImport | Import | 导入 |
| memoryImportSuccess | Imported: {{created}} created... | 导入完成... |
| memoryDeleteBatch | Delete Selected ({{count}}) | 删除所选 ({{count}}) |
| memoryReembed | Re-embed Selected ({{count}}) | 重新生成向量 ({{count}}) |
| memoryReembedAll | Re-embed All | 重新生成全部向量 |
| memorySelectAll | Select All | 全选 |
| memoryDeselectAll | Deselect All | 取消全选 |
| memoryAutoExtract | Auto-extract memories | 自动提取记忆 |
| memoryAutoExtractDesc | Automatically extract... | 对话结束后自动提取... |
| memoryExtracted | Extracted {{count}} new memories... | 从对话中提取了... |
| memoryExtractMinTurns | Min turns before extraction | 提取前最少轮数 |

其余 10 种语言通过 `node scripts/sync-i18n.mjs --apply` 补齐。

---

## 4. 新增/修改文件汇总

| 文件 | 改动类型 | 涉及 Fix |
|------|---------|---------|
| `src-tauri/src/memory.rs` | 修改：trait 扩展 + embedder 方法 + 去重 + 批量 + 导入解析 | 1,2,3 |
| `src-tauri/src/memory_extract.rs` | **新建**：自动记忆提取模块 | 4 |
| `src-tauri/src/tools/memory.rs` | 修改：save_memory 用 add_with_dedup | 2 |
| `src-tauri/src/lib.rs` | 修改：embedder 自动初始化 + 新命令注册 + 提取 hook | 1,2,3,4 |
| `src-tauri/src/agent/mod.rs` | 微调：删除重复的 get_conversation_history | 4 |
| `src-tauri/src/agent_config.rs` | 修改：MemoryConfig 新增 auto_extract 字段 | 4 |
| `src/components/settings/MemoryPanel.tsx` | 修改：去重确认 + 多选/批量 + 导入 + auto-extract 开关 | 2,3,4 |
| `src/i18n/locales/en.json` | 修改：新增 15 个 i18n keys | 2,3,4 |
| `src/i18n/locales/zh.json` | 修改：新增 15 个 i18n keys | 2,3,4 |
| `CHANGELOG.md` | 修改：记录 4 项增强 | docs |
| `CLAUDE.md` | 修改：更新记忆系统描述 + 新增 memory_extract.rs | docs |
| `AGENTS.md` | 修改：同上 | docs |
| `.agent/rules/default.md` | 修改：同上 | docs |

---

## 5. 新增 Tauri 命令

| 命令 | 参数 | 返回 | 用途 |
|------|------|------|------|
| `memory_find_similar` | content, threshold?, limit? | `Vec<MemoryEntry>` | 查找相似记忆（去重预检） |
| `memory_delete_batch` | ids: Vec<i64> | usize | 批量删除 |
| `memory_import` | content, format, dedup | ImportResult | 导入记忆 |
| `memory_reembed` | ids: Option<Vec<i64>> | usize | 重新生成 embedding |
| `memory_stats` | scope: Option<MemoryScope> | MemoryStats | 记忆统计（Phase 1.5） |
| `get_dedup_config` | 无 | DedupConfig | 获取去重阈值配置（Phase 1.5） |
| `save_dedup_config` | config: DedupConfig | () | 保存去重阈值配置（Phase 1.5） |

---

## 6. 新增 MemoryBackend Trait 方法

| 方法 | 签名 | 默认实现 |
|------|------|---------|
| `set_embedder` | `(&self, Arc<dyn EmbeddingProvider>)` | no-op |
| `clear_embedder` | `(&self)` | no-op |
| `has_embedder` | `(&self) -> bool` | `false` |
| `find_similar` | `(&self, content, type?, scope?, threshold, limit) -> Vec<MemoryEntry>` | 无（required） |
| `add_with_dedup` | `(&self, entry, threshold_high, threshold_merge) -> AddResult` | 无（required） |
| `delete_batch` | `(&self, ids) -> usize` | 无（required） |
| `import_entries` | `(&self, entries, dedup) -> ImportResult` | 无（required） |
| `reembed_all` | `(&self) -> usize` | 无（required） |
| `reembed_batch` | `(&self, ids) -> usize` | 无（required） |
| `stats` | `(&self, scope?) -> MemoryStats` | 无（required，Phase 1.5） |

> **注意**: 未来实现 GraphRAG 或 Hindsight 后端时，需要实现所有 required 方法。

---

## 7. Phase 1.5：优化项（已完成）

### 7.0.1 Prompt Summary 优先级加权 ✅

**问题**: `build_prompt_summary` 先拼接所有记忆再截断，可能在记忆内容中间截断。

**改动**:
- `memory.rs`: `build_prompt_summary` 改为逐条添加直到超出 budget 就停止，保持按类型分组 + `updated_at DESC` 排序

### 7.0.2 提取模型可配 ✅

**问题**: auto-extract 复用 chat 模型，可能很贵。

**改动**:
- `agent_config.rs`: `MemoryConfig` 新增 `extract_provider_id: Option<String>` / `extract_model_id: Option<String>`
- `lib.rs`: 提取触发处优先使用配置模型，回退到 chat 模型
- `MemoryPanel.tsx`: auto-extract 展开后显示模型选择器（provider/model 下拉）和最少轮数输入框

### 7.0.3 memory_extracted Toast 通知 ✅

**问题**: 后端已 emit `memory_extracted` 事件，前端未监听。

**改动**:
- `ChatScreen.tsx`: 监听 `memory_extracted` Tauri 事件，当前 session 命中时显示 Brain 图标 + 文案 banner，4 秒后自动消失

### 7.0.4 去重阈值可配置 ✅

**问题**: `DEDUP_THRESHOLD_HIGH`(0.02) 和 `DEDUP_THRESHOLD_MERGE`(0.012) 硬编码。

**改动**:
- `memory.rs`: 新增 `DedupConfig` struct + `load_dedup_config()` 函数
- `provider.rs`: `ProviderStore` 新增 `dedup: DedupConfig` 字段（存储在 config.json）
- `lib.rs`: 新增 `get_dedup_config` / `save_dedup_config` Tauri 命令
- `tools/memory.rs` / `memory_extract.rs` / `memory.rs`（import_entries）: 改用 `load_dedup_config()` 替代硬编码常量
- `MemoryPanel.tsx`: Embedding 设置视图底部可折叠"去重高级设置"区域，两个数字输入框

### 7.0.5 记忆统计仪表板 ✅

**问题**: 无法直观看到记忆的分布情况。

**改动**:
- `memory.rs`: 新增 `MemoryStats` struct + trait 方法 `stats()`，SQLite 实现用 GROUP BY 查询
- `lib.rs`: 新增 `memory_stats` Tauri 命令
- `MemoryPanel.tsx`: list 视图搜索栏上方显示紧凑统计行（总数 | 各类型图标+数量 | 向量覆盖率%）

### 7.0.6 新增 i18n Keys（zh + en）

| Key | 英文 | 中文 |
|-----|------|------|
| memoryExtractModel | Extraction Model | 提取模型 |
| memoryUseChatModel | Use chat model | 使用聊天模型 |
| memoryExtractedToast | Extracted {{count}} new memories... | 从对话中提取了 {{count}} 条新记忆 |
| memoryDedupAdvanced | Dedup Advanced Settings | 去重高级设置 |
| memoryDedupAdvancedDesc | Adjust RRF similarity thresholds... | 调整记忆去重检测的 RRF 相似度阈值... |
| memoryDedupHigh | Duplicate Threshold | 重复阈值 |
| memoryDedupMerge | Merge Threshold | 合并阈值 |
| memoryStatsTotal | {{count}} total | 共 {{count}} 条 |
| memoryStatsVec | Vector {{pct}}% | 向量 {{pct}}% |

### 7.0.7 新增/修改文件汇总（Phase 1.5）

| 文件 | 改动类型 | 涉及优化项 |
|------|---------|-----------|
| `src-tauri/src/memory.rs` | 修改：build_prompt_summary 优化 + DedupConfig + MemoryStats + stats() | A,D,E |
| `src-tauri/src/agent_config.rs` | 修改：MemoryConfig 新增 extract_provider_id/extract_model_id | B |
| `src-tauri/src/lib.rs` | 修改：提取模型优先读配置 + 3 个新 Tauri 命令 | B,D,E |
| `src-tauri/src/provider.rs` | 修改：ProviderStore 新增 dedup 字段 | D |
| `src-tauri/src/memory_extract.rs` | 修改：改用 load_dedup_config() | D |
| `src-tauri/src/tools/memory.rs` | 修改：改用 load_dedup_config() | D |
| `src/components/chat/ChatScreen.tsx` | 修改：监听 memory_extracted + toast banner | C |
| `src/components/settings/MemoryPanel.tsx` | 修改：提取模型选择器 + 去重设置 + 统计行 | B,D,E |
| `src/i18n/locales/zh.json` / `en.json` | 修改：新增 9 个 i18n keys | B,C,D,E |

---

## 8. 待完成工作（Phase 2+）

### 8.1 GraphRAG 调研（中期，未开始）

**调研内容**:
- Rust 生态可用的 GraphRAG / LightRAG / 知识图谱 crate
- 是否适合基于现有 SQLite 自研轻量实现（`kg_entities` + `kg_relations` 表）
- Entity-relation 提取方案（LLM-based 小模型提取）
- 与 `MemoryBackend` trait 的集成方式（新增 `graph_search()` 可选方法）
- 存储和性能影响评估

**初步方向**:
- Rust 生态暂无成熟 GraphRAG crate，推荐基于 SQLite 自研：
  - `kg_entities` 表：id, name, type, description, embedding
  - `kg_relations` 表：id, source_id, target_id, relation_type, weight, metadata
- Entity 提取复用 auto-extraction 的 LLM 调用模式
- 图遍历：给定 query，向量搜索找相关 entity，1-2 跳关系展开
- trait 集成：`graph_search(&self, query, hops) -> Vec<GraphContext>` 可选方法

**产出**: 技术可行性报告 + 推荐方案 + 预估工作量

### 8.2 Hindsight 集成（长期，未开始）

- 集成 `hindsight-client` Rust crate
- 本地 daemon（嵌入式 Postgres + pgvector）
- 四层结构：世界事实 + 经历 + 实体摘要 + 演化信念
- GUI 可视化：信念演化时间线、纠错记录

### 8.3 其他待优化项

| 项目 | 优先级 | 状态 | 说明 |
|------|--------|------|------|
| ~~提取模型可配~~ | 中 | ✅ Phase 1.5 | MemoryConfig 新增 extractProviderId/extractModelId |
| ~~前端 memory_extracted 事件监听~~ | 低 | ✅ Phase 1.5 | ChatScreen 监听 + toast banner |
| ~~记忆统计/分析仪表板~~ | 低 | ✅ Phase 1.5 | memory_stats 命令 + 前端统计行 |
| ~~去重阈值可配置~~ | 低 | ✅ Phase 1.5 | DedupConfig 存 config.json + GUI |
| ~~Google Embedding 完整实现~~ | 低 | ✅ Phase 1 已完成 | call_google() 已完整实现 |
| ~~Prompt summary 优先级加权~~ | 中 | ✅ Phase 1.5 | 逐条添加 + budget 控制 |
| 本地模型下载 UI | 低 | ⬜ 未开始 | 目前只展示不支持在线下载，工作量较大 |

---

## 9. 架构图

```
┌─────────────────────────────────────────────────────┐
│                    Frontend (React)                  │
│                                                     │
│  MemoryPanel.tsx                                    │
│  ├─ List View (multi-select, batch ops, stats bar)  │
│  ├─ Add/Edit View (dedup confirmation dialog)       │
│  ├─ Embedding Config View (re-embed all, dedup cfg) │
│  └─ Auto-extract (toggle + model picker + minTurns) │
│                                                     │
│  ChatScreen.tsx                                     │
│  └─ memory_extracted toast banner (4s auto-dismiss) │
│                                                     │
│  invoke() ──────────────────────────────────┐       │
└────────────────────────────────────────────┐│───────┘
                                             ││
┌────────────────────────────────────────────┘│───────┐
│                 Tauri Commands              │       │
│                                             ▼       │
│  memory_add / memory_update / memory_delete         │
│  memory_search / memory_list / memory_count         │
│  memory_export / memory_find_similar                │
│  memory_delete_batch / memory_import / memory_reembed│
│  memory_stats / get_dedup_config / save_dedup_config│
│  save_embedding_config / get_embedding_config       │
│                                                     │
└──────────────────────┬──────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────┐
│             MemoryBackend Trait                      │
│                                                     │
│  add / update / delete / get / list / search        │
│  find_similar / add_with_dedup / stats               │
│  delete_batch / import_entries / reembed_all/batch   │
│  set_embedder / clear_embedder / has_embedder       │
│  build_prompt_summary / export_markdown             │
│                                                     │
│  ┌──────────────────────────────────────────────┐   │
│  │ SqliteMemoryBackend (MVP, current)           │   │
│  │  ├─ SQLite + WAL + FTS5 (keyword search)     │   │
│  │  ├─ sqlite-vec (vector similarity search)    │   │
│  │  ├─ RRF fusion scoring                       │   │
│  │  └─ EmbeddingProvider (API/Local ONNX)       │   │
│  └──────────────────────────────────────────────┘   │
│                                                     │
│  ┌──────────────────────────────────────────────┐   │
│  │ GraphRAG Backend (Phase 2, planned)          │   │
│  │  ├─ kg_entities + kg_relations tables        │   │
│  │  ├─ LLM entity-relation extraction           │   │
│  │  └─ Graph traversal search                   │   │
│  └──────────────────────────────────────────────┘   │
│                                                     │
│  ┌──────────────────────────────────────────────┐   │
│  │ Hindsight Backend (Phase 3, planned)         │   │
│  │  └─ hindsight-client + embedded Postgres     │   │
│  └──────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────┐
│            memory_extract.rs (async)                │
│                                                     │
│  chat success ─→ tokio::spawn ─→ run_extraction()   │
│    1. Get last 6 messages                           │
│    2. Get existing memory summary                   │
│    3. LLM call (extraction prompt)                  │
│    4. Parse JSON response                           │
│    5. add_with_dedup() for each extracted memory    │
│    6. Emit "memory_extracted" event                 │
└─────────────────────────────────────────────────────┘
```

---

## 10. 测试检查清单

### Fix 1: Embedder 自动初始化
- [ ] 配置 embedding enabled → 重启 app → 搜索应包含向量结果
- [ ] embedding API 不可达 → app 正常启动，仅 FTS5 搜索
- [ ] UI 切换 embedding enabled/disabled → 立即生效

### Fix 2: 去重检测
- [ ] 添加 "用户喜欢深色模式" → 再添加 "用户偏好 dark mode" → 应检测为重复
- [ ] 添加 "用户住在北京" → 添加 "用户在 Google 工作" → 不应标记
- [ ] Agent tool save_memory 重复内容 → 返回 "Similar memory already exists"

### Fix 3: 导入 + 批量操作
- [ ] 导出 markdown → 清空记忆 → 重新导入 → 记忆恢复
- [ ] 导入含重复的 JSON（dedup=true）→ 跳过重复
- [ ] 多选 5 条 → 批量删除 → 确认删除
- [ ] Re-embed All → 验证向量更新

### Fix 4: 自动记忆提取
- [ ] 开启 auto_extract → 对话中提到 "我叫张三，住在上海" → 检查自动生成记忆
- [ ] 连续对话提到同样信息 → 不重复提取
- [ ] 关闭 auto_extract → 不触发提取
- [ ] 提取失败 → 不影响 chat 响应

### Phase 1.5: Prompt Summary 优先级加权
- [ ] 添加超过 budget 的记忆 → 系统提示词中不出现截断到半行的情况
- [ ] 最近更新的记忆优先出现在系统提示词中

### Phase 1.5: 提取模型可配
- [ ] Agent 设置 → 开启 auto-extract → 选择便宜模型 → 对话后检查日志确认使用了配置模型
- [ ] 不选模型（使用聊天模型）→ 对话后确认使用 chat 模型提取
- [ ] 修改 extractMinTurns → 验证轮数少于设定值时不触发提取

### Phase 1.5: memory_extracted Toast
- [ ] 开启 auto-extract → 对话后 → 当前 session 看到 toast banner
- [ ] banner 4 秒后自动消失
- [ ] 点击 × 可手动关闭

### Phase 1.5: 去重阈值可配置
- [ ] 进入 Embedding 设置 → 展开"去重高级设置" → 修改阈值 → 添加相似记忆验证行为变化
- [ ] 恢复默认值（0.02 / 0.012）→ 行为与修改前一致

### Phase 1.5: 记忆统计仪表板
- [ ] 打开 MemoryPanel → 有记忆时显示统计行（总数 + 类型分布 + 向量覆盖率）
- [ ] 无记忆时不显示统计行
- [ ] 添加/删除记忆后统计行自动更新
