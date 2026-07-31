//! 排程特征 crate（阶段 5 第三刀，自 ha-core 迁出）：cron 的调度器 /
//! 执行器 / 投递 / 失败分类 / 时间线，以及 `manage_cron` 工具 adapter。
//! **取消注册表与排程校验不在本 crate**——见下方 kernel 侧留存。
//!
//! # kernel 侧留存（分法：台账 vs 机器）
//!
//! - **`ha_core::cron::CronDB`**——cron.db 台账。它被 kernel 侧多处深度消费：
//!   `loop_control` 的托管 `/loop` 全程持 `&CronDB`（20+ 处签名）、
//!   `agent_lifecycle` 改名 / 删除时重写 `cron_jobs.payload_json`、
//!   `agent::migration` 的启动期迁移。与破环那刀把 `local_model_jobs` 台账
//!   留 kernel、执行器迁 ha-local-llm 是同一个分法。
//! - **`ha_core::cron_defs`**——`CronJob` / `CronPayload` / `CronSchedule` 等
//!   wire 类型（同 `tool_defs` / `slash_defs` 契约层惯例）。
//! - **`ha_core::loop_control`**——托管 `/loop` **整体留 kernel**：它有一个
//!   58 方法、2673 行的 `impl SessionDB` 块，固有 impl 只能待在定义 `SessionDB`
//!   的 crate 里；改扩展 trait 也不行——kernel 有 15+ 处调用点，那会变成
//!   kernel `use ha_cron::…` 的反向依赖。它对本 crate 的耦合极窄，只有 3 处
//!   `spawn_job_execution`，走钩子。
//! - **`ha_core::wakeup`**——`schedule_wakeup` **不是 cron**（AGENTS 明写
//!   「不复用入口」），对 cron 零引用、消费者全在 kernel，故留 kernel。
//!   分析器早先把它归进 cron 组纯属主题相似，已纠正。
//!
//! 因此 `CRON_DB` 全局与 `AppState.cron_db` **不动**，壳层调用点零改动。
//!
//! 装配契约与其它特征 crate 相同：每个调 `ha_core::init_runtime` 的二进制
//! 必须先调 [`wire()`]。

// `app_*!` 系宏由 ha-base 导出（与 ha-core / ha-media 同一接法）。
#[macro_use]
extern crate ha_base;

pub mod cron;
pub mod tools;

/// 幂等装配：三处接线。
///
/// 1. **`cron_hooks` 三槽原子注册**——kernel 反向需要的机器行为（托管
///    `/loop` 起任务 / 取消在跑任务 / subagent 注入后按白名单回投）。
/// 2. **`manage_cron` 分发条目**——schema 仍在 kernel 的
///    `definitions::core_tools`，此处只补 handler；漏 wire 由
///    `registry_freeze` warn 兜底（有 definition 无 handler）。
/// 3. **调度器启动任务**——`PrimaryOnly`，与迁移前 `app_init` 里那个
///    primary 块同档（periodic tick 的 `claim_scheduled_job_for_execution`
///    跨进程会重复 claim，故只 Primary 跑；手动 run-now 走原子 SQL claim，
///    任何 tier 都安全）。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        fn spawn_job_execution(
            cron_db: std::sync::Arc<ha_core::cron::CronDB>,
            session_db: std::sync::Arc<ha_core::session::SessionDB>,
            job: ha_core::cron::CronJob,
        ) {
            cron::spawn_job_execution(cron_db, session_db, job);
        }
        fn cancel_running_job(job_id: &str) -> anyhow::Result<Option<bool>> {
            cron::cancel_running_job(job_id)
        }
        fn deliver_injection<'a>(
            session_id: &'a str,
            text: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
            Box::pin(cron::delivery::deliver_injection_for_session(
                session_id, text,
            ))
        }
        fn start_scheduler(
            cron_db: std::sync::Arc<ha_core::cron::CronDB>,
            session_db: std::sync::Arc<ha_core::session::SessionDB>,
        ) {
            cron::start_scheduler(cron_db, session_db);
        }
        ha_core::cron_hooks::register_cron_hooks(ha_core::cron_hooks::CronHooks {
            start_scheduler,
            spawn_job_execution,
            cancel_running_job,
            deliver_injection_for_session: deliver_injection,
        })
        .expect("ha_cron::wire() registers the cron machinery hooks exactly once");

        fn manage_cron_handler<'a>(
            args: &'a serde_json::Value,
            ctx: &'a ha_core::tools::ToolExecContext,
        ) -> ha_core::tools::registry::BuiltinToolFuture<'a> {
            Box::pin(tools::cron::tool_manage_cron(args, ctx))
        }
        ha_core::tools::registry::register_external_tools(vec![
            ha_core::tools::registry::BuiltinToolEntry {
                name: ha_core::tools::TOOL_MANAGE_CRON,
                aliases: &[],
                handler: manage_cron_handler,
            },
        ])
        .expect("ha_cron::wire() registers the manage_cron handler before registry freeze");
    });
}
