//! 知识空间特征 crate（阶段 5 第六刀，自 ha-core 迁出）：`index.db` 笔记缓存
//! 与检索、Markdown 解析 / 来源导入 / 编译、embedding 与重建、`[[note]]` 注入
//! 与被动召回、笔记图谱与重命名、Layer-2 自主维护流水线、外部 vault 实时监听，
//! 以及 24 个知识空间工具（22 个 `note_*` + `knowledge_recall` +
//! `session_to_note`）。
//!
//! # 分法：台账 + 契约 + 裁决留 kernel，机器上浮
//!
//! 留在 [`ha_core::knowledge`] 的四件东西**都不是「知识行为」**：
//!
//! - **`registry`（`KnowledgeRegistry`）**——`knowledge_bases` + 会话 / 项目
//!   访问绑定的真相源（设计 D9），81 处直接 `session_db.conn.lock()`。
//!   AGENTS 红线「kernel 的 `sessions.db` 写连接不对特征 crate 开放」把它钉死
//!   在 kernel。**正因为它留下，`KNOWLEDGE_DB` 全局、`get_knowledge_db()` 与
//!   `AppState.knowledge_db` 一处未改**——同第五刀 `CHANNEL_REGISTRY` 的效果。
//! - **`access`**——「KB 访问默认 deny」的**唯一裁决点**
//!   （`effective_kb_access` / `im_kb_access_allowed`），连同工具面的解析链
//!   （`access_map_for_tool_ctx` / `im_kb_context_from_session` /
//!   `session_has_kb_access`）。后三个本刀一度随 `tools/note.rs` 迁出、经钩子
//!   回调，自审时改回：它们只用 kernel 原语，而钩子会给 WS8 的**收紧**动作加上
//!   「未装配即失效」的 fail-open 语义，也破坏「工具面与 `Agent::resolve_kb_access`
//!   共用同一份推导」这条不变量。
//! - **`types` / `maintenance_defs`**——registry 方法签名与 `AppConfig` 可达的
//!   wire 类型。`maintenance_defs` 是 `maintenance/types.rs` 下沉后的落点，
//!   [`knowledge::maintenance`] 原名再导出，crate 外路径逐字不变。
//!
//! analyzer 迁出前对本组报 44 条需切边，其中约 36 条是
//! `KbAccess` / `KbAccessSource` / `ChannelKbContext` / `KnowledgeAccessContext`
//! / `NoteSearchHit` 的**纯类型引用**——契约留下即全部消失，一个钩子都不用开。
//!
//! # 反向回调
//!
//! kernel → 本 crate 的**唯一**回调面是 [`ha_core::knowledge_hooks`]（十槽
//! 原子注册）：行为 4 + 装配 6。未装配语义逐槽在该模块文档里论证——检索 / 注入
//! 返空（等价于索引未开）、embedding 重载 no-op，而重建任务返 `Err`
//! （用户显式触发的写不能静默成功）。
//!
//! 装配契约与其它特征 crate 相同：每个调 `ha_core::init_runtime` 的二进制
//! 必须先调 [`wire()`]。

// `app_*!` 系宏由 ha-base 导出（与 ha-core / ha-channel / ha-media 同一接法）。
#[macro_use]
extern crate ha_base;

pub mod knowledge;
pub mod tools;

/// 幂等装配：三处接线——`knowledge_hooks` 十槽 + 24 个工具的分发条目 +
/// Primary 初始化后的 embedding 签名迁移恢复任务。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        register_hooks();
        register_note_tools();
        ha_core::app_init::register_startup_task(
            ha_core::app_init::StartupStage::PrimaryOnly,
            knowledge::index::resume_embedding_signature_migration,
        )
        .expect(
            "ha_knowledge::wire() must run before start_background_tasks consumes startup tasks",
        );
    });
}

fn register_hooks() {
    use ha_core::knowledge_hooks as h;

    fn search_notes(
        kb_ids: &[String],
        query: &str,
        limit: usize,
    ) -> Vec<ha_core::knowledge::NoteSearchHit> {
        // 与迁出前 `agent::warm_related_notes` 内联的那三行逐位相同：索引没开
        // 就是空结果，检索出错也降级成空（`unwrap_or_default`）。
        let Some(db) = knowledge::index::get_index_db() else {
            return Vec::new();
        };
        knowledge::search::search_notes(&db, kb_ids, query, limit).unwrap_or_default()
    }

    fn init_index_db() {
        // 迁出前是 `if let Err(e) = ... { eprintln!(...) }`——此时 logger 尚未
        // 装好（这一步刻意排在 LogDB 之前），故仍用 eprintln 而非 app_warn!。
        if let Err(e) = knowledge::index::init_index_db() {
            eprintln!("[runtime] knowledge index init failed: {e}");
        }
    }

    fn maintenance_idle_tick() {
        let mcfg = ha_core::config::cached_config()
            .knowledge_maintenance
            .clone();
        if knowledge::maintenance::check_idle_trigger(&mcfg) {
            tokio::spawn(async {
                let report = knowledge::maintenance::manual_run(
                    knowledge::maintenance::MaintenanceTrigger::Idle,
                )
                .await;
                app_info!(
                    "knowledge",
                    "maintenance::idle_trigger",
                    "idle-trigger cycle: generated={}, autoApplied={}, note={:?}",
                    report.generated,
                    report.auto_applied,
                    report.note,
                );
            });
        }
    }

    h::register_knowledge_hooks(h::KnowledgeHooks {
        search_notes,
        resolve_inline_injections: knowledge::inject::resolve_inline_injections,
        resolve_bound_notes: knowledge::inject::resolve_bound_notes,
        apply_embedding_from_config: knowledge::embedding::apply_knowledge_embedding_from_config,
        start_reembed_job: knowledge::reembed::start_knowledge_reembed_job,
        init_index_db,
        ensure_default_knowledge_base: knowledge::service::ensure_default_knowledge_base,
        maintenance_idle_tick,
        spawn_maintenance_cron_loop: knowledge::maintenance::spawn_maintenance_cron_loop,
        index_spawn_startup_reconcile: knowledge::index::spawn_startup_reconcile,
        watcher_start_all_watchers: knowledge::watcher::start_all_watchers,
    })
    .expect("ha_knowledge::wire() registers the knowledge hooks exactly once");
}

fn register_note_tools() {
    ha_core::tools::registry::register_external_tools(tools::note_dispatch_entries())
        .expect("ha_knowledge::wire() registers the note tool handlers before registry freeze");
}
