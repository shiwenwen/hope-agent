# 记忆系统增强 — 设计文档 & 交接手册

> 版本: 1.0
> 日期: 2026-03-24
> 状态: Phase 1（MVP Gap 修复）已完成，Phase 2（GraphRAG）待调研

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

> **注意**: 未来实现 GraphRAG 或 Hindsight 后端时，需要实现所有 required 方法。

---

## 7. 待完成工作（Phase 2+）

### 7.1 GraphRAG 调研（中期，未开始）

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

### 7.2 Hindsight 集成（长期，未开始）

- 集成 `hindsight-client` Rust crate
- 本地 daemon（嵌入式 Postgres + pgvector）
- 四层结构：世界事实 + 经历 + 实体摘要 + 演化信念
- GUI 可视化：信念演化时间线、纠错记录

### 7.3 其他待优化项

| 项目 | 优先级 | 说明 |
|------|--------|------|
| 提取模型可配 | 中 | 允许 auto-extract 使用比 chat 更便宜的模型 |
| 前端 `memory_extracted` 事件监听 | 低 | 在 chat UI 显示 toast "提取了 N 条新记忆" |
| 记忆统计/分析仪表板 | 低 | 图表展示记忆分布、增长趋势 |
| 去重阈值可配置 | 低 | GUI 暴露 threshold 调节 |
| Google Embedding 完整实现 | 低 | config 有 Google 类型但 API 未完全适配 |
| 本地模型下载 UI | 低 | 目前只展示不支持在线下载 |
| Prompt summary 优先级加权 | 中 | 当前按时间截断，应按相关度/重要性排序 |

---

## 8. 架构图

```
┌─────────────────────────────────────────────────────┐
│                    Frontend (React)                  │
│                                                     │
│  MemoryPanel.tsx                                    │
│  ├─ List View (multi-select, batch ops)             │
│  ├─ Add/Edit View (dedup confirmation dialog)       │
│  ├─ Embedding Config View (re-embed all)            │
│  └─ Auto-extract toggle (agent mode)                │
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
│  save_embedding_config / get_embedding_config       │
│                                                     │
└──────────────────────┬──────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────┐
│             MemoryBackend Trait                      │
│                                                     │
│  add / update / delete / get / list / search        │
│  find_similar / add_with_dedup                      │
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

## 9. 测试检查清单

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
