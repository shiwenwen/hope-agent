//! Knowledge 机器层的**反向钩子**——kernel → ha-knowledge 的唯一回调面。
//!
//! # 为什么只有十槽
//!
//! 阶段 5 第六刀把索引缓存 / 解析编译 / 检索 / embedding / Layer-2 维护 /
//! `note_*` 工具自 ha-core 迁入 ha-knowledge。analyzer 当时报 44 条需切边，
//! 但其中约 36 条是 [`crate::knowledge::KbAccess`] /
//! [`crate::knowledge::KbAccessSource`] / [`crate::knowledge::ChannelKbContext`]
//! / `KnowledgeAccessContext` / `NoteSearchHit` 的**纯类型引用**——契约与裁决
//! （`access` / `types` / `registry` / `maintenance_defs`）随台账留在 kernel，
//! 这些引用一条都不必改成钩子。真正需要回调的只有下面四处行为 + 六处装配。
//!
//! # 台账不在这里
//!
//! `knowledge_bases` + 访问绑定的读写（[`crate::knowledge::KnowledgeRegistry`]）
//! **留在 kernel**，不走钩子：它 81 处直接 `session_db.conn.lock()`，而
//! `SessionDB` 的写连接按 AGENTS 红线不对特征 crate 开放。`KNOWLEDGE_DB` 全局、
//! `get_knowledge_db()` 与 `AppState.knowledge_db` 因此一处未改。
//!
//! `effective_kb_access` 同理留 kernel——它是「KB 访问默认 deny」的**唯一裁决
//! 点**，把唯一裁决点挪到可选装配的钩子后面，等于给它加了一条「未装配」的
//! 旁路语义，这不是重构该碰的东西。**工具面的解析链
//! （`access_map_for_tool_ctx` / `im_kb_context_from_session` /
//! `session_has_kb_access`）同处、同样不走钩子**：本刀曾把它们随 `tools/note.rs`
//! 迁出再钩回来，自审时发现那给 WS8 的**收紧**动作凭空加了一条「未装配即失效」
//! 的 fail-open 语义（未装配 → 不重分类成 IM → 停在默认的 `Gui` owner 平面），
//! 且与 `Agent::resolve_kb_access` 的「两平面共用同一推导」不变量冲突。
//! 它们只用 kernel 原语，归位到裁决点同处后该不变量重新是编译期保证。
//!
//! # 未装配语义（逐槽，刻意不统一）
//!
//! - **装配六槽**：`()` no-op。等价于「这个进程没有知识空间机器」——
//!   与迁移前 acp / mcp / eval 形态压根不走 `start_background_tasks` 同义。
//! - [`search_notes`] / [`resolve_inline_injections`]：返回**空**。逐位等价于
//!   迁移前 `index::get_index_db()` 返 `None` 的那条分支（索引没开 = 没有命中），
//!   两处调用点本就按「空结果」写的。
//! - [`apply_embedding_from_config`]：no-op（没有索引就没有 embedder 可重载）。
//! - [`start_reembed_job`]：返回 **`Err`**。它是用户显式触发的**写**动作，
//!   静默成功会让 GUI 显示「已开始重建」而实际什么都没发生。

use std::sync::OnceLock;

/// 十槽**原子注册**：行为 4 + 装配 6。
///
/// 与 `channel_hooks` / `cron_hooks` / `slash_hooks` 同样一次性注册整组——
/// 部分注册＝一部分知识路径活着、另一部分静默返空，是最难查的破法。
pub struct KnowledgeHooks {
    // ——— 行为 4 ———
    /// `knowledge::search::search_notes`（`index.db` 只读检索）。
    /// **同步 + 阻塞 SQLite**：调用方 `agent::warm_related_notes` 已经把它
    /// 包在 `spawn_blocking` 里，钩子保持同步签名以维持这一点。
    pub search_notes:
        fn(kb_ids: &[String], query: &str, limit: usize) -> Vec<crate::knowledge::NoteSearchHit>,
    /// `knowledge::inject::resolve_inline_injections`——`[[note]]` 内联注入。
    pub resolve_inline_injections: fn(
        message: &str,
        session_id: &str,
        source: crate::knowledge::KbAccessSource,
        origin: crate::knowledge::KbAccessSource,
        channel_info: Option<crate::knowledge::ChannelKbContext>,
    ) -> Option<String>,
    /// `knowledge::embedding::apply_knowledge_embedding_from_config`。
    pub apply_embedding_from_config: fn(source: &str),
    /// `knowledge::reembed::start_knowledge_reembed_job`。
    pub start_reembed_job: fn(
        kb_ids: Option<Vec<String>>,
        source: &str,
    )
        -> anyhow::Result<crate::local_model_jobs::LocalModelJobSnapshot>,

    // ——— 装配 6（全部在 app_init 的原调用位触发，逐位保序）———
    /// `knowledge::index::init_index_db`——打开 `index.db` + 装 note embedder。
    /// 迁移前在 `init_runtime` 里紧跟 `KNOWLEDGE_DB.set(...)`、**早于 LogDB**，
    /// 故不能改用 `register_init_task`（那个消费点在 600 行之后）。
    pub init_index_db: fn(),
    /// `knowledge::service::ensure_default_knowledge_base`——首次运行播种。
    /// 迁移前刻意排在 logger 之后（好让它的 `app_info!` 落盘）且在
    /// `is_primary()` 块内，两个位置都由 kernel 侧调用点保持。
    pub ensure_default_knowledge_base: fn(),
    /// Layer-2 维护的 idle 触发（`check_idle_trigger` + `manual_run(Idle)`）。
    /// 整块（含配置读取、`tokio::spawn` 与日志）下沉一槽——它挂在 kernel 的
    /// dreaming idle 时钟上，只有「这一 tick 要不要跑」是 kernel 的事。
    pub maintenance_idle_tick: fn(),
    /// `knowledge::maintenance::spawn_maintenance_cron_loop`。
    pub spawn_maintenance_cron_loop: fn(),
    /// `knowledge::index::spawn_startup_reconcile`——开机对账每个 KB 与磁盘。
    pub index_spawn_startup_reconcile: fn(),
    /// `knowledge::watcher::start_all_watchers`——外部 vault 的实时监听。
    pub watcher_start_all_watchers: fn(),
}

static KNOWLEDGE_HOOKS: OnceLock<KnowledgeHooks> = OnceLock::new();

/// 装配期注册知识空间机器钩子（十槽原子）。重复注册返回 `Err`。
pub fn register_knowledge_hooks(hooks: KnowledgeHooks) -> Result<(), crate::AlreadyRegistered> {
    KNOWLEDGE_HOOKS
        .set(hooks)
        .map_err(|_| crate::AlreadyRegistered("knowledge machinery hooks"))
}

fn hooks() -> Option<&'static KnowledgeHooks> {
    KNOWLEDGE_HOOKS.get()
}

/// `index.db` 笔记检索。未装配 / 索引未开即空——与迁移前 `get_index_db()`
/// 返 `None` 的分支逐位相同。
pub fn search_notes(
    kb_ids: &[String],
    query: &str,
    limit: usize,
) -> Vec<crate::knowledge::NoteSearchHit> {
    match hooks() {
        Some(h) => (h.search_notes)(kb_ids, query, limit),
        None => Vec::new(),
    }
}

/// `[[note]]` 内联注入。未装配即 `None`（无附加 system 段）。
pub fn resolve_inline_injections(
    message: &str,
    session_id: &str,
    source: crate::knowledge::KbAccessSource,
    origin: crate::knowledge::KbAccessSource,
    channel_info: Option<crate::knowledge::ChannelKbContext>,
) -> Option<String> {
    match hooks() {
        Some(h) => (h.resolve_inline_injections)(message, session_id, source, origin, channel_info),
        None => None,
    }
}

/// 按 live 配置重载知识 embedding。未装配即 no-op。
pub fn apply_embedding_from_config(source: &str) {
    if let Some(h) = hooks() {
        (h.apply_embedding_from_config)(source);
    }
}

/// 启动知识库重建 embedding 任务。**未装配返回 `Err`**——用户显式触发的写
/// 动作不能静默成功。
pub fn start_reembed_job(
    kb_ids: Option<Vec<String>>,
    source: &str,
) -> anyhow::Result<crate::local_model_jobs::LocalModelJobSnapshot> {
    match hooks() {
        Some(h) => (h.start_reembed_job)(kb_ids, source),
        None => Err(anyhow::anyhow!(
            "knowledge machinery not wired (ha_knowledge::wire() was not called)"
        )),
    }
}

/// 打开 `index.db` + 装 note embedder。未装配即 no-op。
pub fn init_index_db() {
    if let Some(h) = hooks() {
        (h.init_index_db)();
    }
}

/// 首次运行播种默认知识空间。未装配即 no-op。
pub fn ensure_default_knowledge_base() {
    if let Some(h) = hooks() {
        (h.ensure_default_knowledge_base)();
    }
}

/// Layer-2 维护 idle 触发（挂 kernel 的 dreaming idle 时钟）。未装配即 no-op。
pub fn maintenance_idle_tick() {
    if let Some(h) = hooks() {
        (h.maintenance_idle_tick)();
    }
}

/// Layer-2 维护 cron 触发循环。未装配即 no-op。
pub fn spawn_maintenance_cron_loop() {
    if let Some(h) = hooks() {
        (h.spawn_maintenance_cron_loop)();
    }
}

/// 开机把每个 KB 与磁盘对账。未装配即 no-op。
pub fn index_spawn_startup_reconcile() {
    if let Some(h) = hooks() {
        (h.index_spawn_startup_reconcile)();
    }
}

/// 为每个 KB root 起实时监听。未装配即 no-op。
pub fn watcher_start_all_watchers() {
    if let Some(h) = hooks() {
        (h.watcher_start_all_watchers)();
    }
}
