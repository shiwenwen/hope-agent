//! Cron **机器**：调度器 / 执行器 / 投递 / 失败分类 / 时间线。
//!
//! **不含**取消注册表与排程校验——那两个跟着台账留在 `ha_core::cron`
//! （`CronDB` 推进 `next_run_at` 用 schedule、取消路径用 cancel）。
//!
//! 台账（`CronDB`）与 wire 类型留在 kernel（`ha_core::cron` /
//! `ha_core::cron_defs`）——分法与破环那刀对 `local_model_jobs` 的处理同型：
//! **台账被 kernel 侧多处消费**（`loop_control` 的托管 `/loop` 全程持
//! `&CronDB`、`agent_lifecycle` 改名时重写 payload、`agent::migration`），
//! 机器才是本 crate 的本体。
pub mod delivery;
pub mod executor;
pub mod failure;
pub mod scheduler;
pub mod timeline;
pub mod workspace;

pub use executor::{cancel_run, cancel_running_job, execute_job_public, spawn_job_execution};
pub use scheduler::start_scheduler;
pub use timeline::{
    cron_run_timeline, delete_conversation_and_run_logs, delete_job_and_legacy_sessions,
    visible_cron_run_logs,
};
pub use workspace::{
    discard_persistent_worktree, discard_run_worktree, return_persistent_worktree,
    take_over_persistent_worktree, workspace_error_code, workspace_resource_for_run,
    workspace_resources, CronWorkspaceActionAvailability, CronWorkspaceActionResult,
    CronWorkspaceActions, CronWorkspaceResource,
};
