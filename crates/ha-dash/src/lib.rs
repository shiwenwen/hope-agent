//! 大盘特征 crate（阶段 5 第二刀，自 ha-core 迁出）：用量总账聚合与
//! Insights、控制面（Goal / Workflow / Loop / Task / Plan）只读聚合、
//! Learning Tracker 的**消费侧**、`/recap` 深度复盘报告（facets / 章节 /
//! 渲染 / 保留期）。
//!
//! kernel 侧留存：
//!
//! - **`ha_core::learning_events`**——Learning 埋点的**发布面**。生产者
//!   遍布 kernel / skills / knowledge / ha-mcp 四层，发布面留在 dashboard
//!   会让它们（含已拆出的特征 crate）反向依赖本 crate；[`dashboard::learning`]
//!   只做只读聚合并原路径再导出（阶段 4 破环 B 步）。
//! - **`ha_core::provider::CNY_PER_USD`**——人民币折算常量。self_diagnosis
//!   与 eval_context 两个 kernel 消费者与 `provider::Currency` 配对使用，
//!   口径必须与本 crate 的成本聚合同源，故随 `Currency` 落在 provider。
//! - **`ha_core::awareness`** 的 facet 视图与查询钩子、
//!   **`ha_core::recap_hooks`** 的 `/recap` 分发钩子（见 [`wire()`]）。
//!
//! 依赖方向：本 crate → ha-core。它对 cron / coding_improvement 的引用
//! （dash→cron 4 · dash→improve 1）现表现为普通 `ha_core::…` 调用；等那两
//! 家拆出后变为特征间单向边，方向不变——**ha-dash 是拆分拓扑序的第一个**
//! （无任何入边），先拆它正是为了让这些边始终合法。
//!
//! 装配契约与其它特征 crate 相同：每个调 `ha_core::init_runtime` 的二进制
//! 必须先调 [`wire()`]。

// `app_*!` 系宏由 ha-base 导出（与 ha-core / ha-media 同一接法）。
#[macro_use]
extern crate ha_base;

pub mod dashboard;
mod db;
pub mod recap;

/// 幂等装配：注册三处 kernel 边界。
///
/// 1. **facet 查询钩子**——`awareness` 的候选行富化要读 `recap_facets` 的
///    四个字段。未注册即 `None`，`collect_entries` 走它**既有的**
///    `fallback_preview` 分支（与今天 `RecapDb::open_default()` 失败时的
///    行为逐位相同，不是新降级路径）。
/// 2. **`/recap` 分发钩子**——slash 装配层只保留一行 trampoline，参数解析 /
///    后台 spawn / 进度事件全在本 crate。未注册即 `Err`，与
///    `slash_hooks::dispatch` 未装配时同款（IM worker 已有错误分支，不会把
///    命令文本当普通消息喂给模型）。
/// 3. **facet 保留期清理循环**——`PrimaryOnly` 档 startup task。原调用位在
///    `app_init::start_background_tasks` 的 primary 块**内**（与
///    `async_jobs` / dreaming 两个保留期清理相邻），若错登成 `EveryProcess`
///    就会让 desktop 与 server 各扫一遍同一张表。**执行点前移**：该档在
///    函数中段的 `run_registered_startup_tasks(PrimaryOnly)` 消费，早于原位
///    约 160 行；清理循环自身先跑一次再进 24h 周期，前移只是让首次清理更早
///    发生，且中间那段在 `start_background_tasks` 自身层面无 `.await`。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        ha_core::awareness::register_session_facet_lookup(recap::facet_view_for_session)
            .expect("ha_dash::wire() registers the session-facet lookup exactly once");

        fn slash_recap(args: &str) -> ha_core::recap_hooks::BoxSlashFut<'_> {
            Box::pin(recap::slash::handle_recap(args))
        }
        ha_core::recap_hooks::register_slash_recap(slash_recap)
            .expect("ha_dash::wire() registers the /recap dispatch hook exactly once");

        ha_core::app_init::register_startup_task(
            ha_core::app_init::StartupStage::PrimaryOnly,
            recap::spawn_facet_retention_loop,
        )
        .expect("ha_dash::wire() registers the recap retention task exactly once");
    });
}
