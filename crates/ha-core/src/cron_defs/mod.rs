//! Cron **契约层**——`cron_jobs` / `cron_run_logs` 两张表的 wire 类型。
//!
//! 与 [`crate::tool_defs`] / [`crate::slash_defs`] 同型：**契约物下沉 kernel、
//! 机器上浮特征 crate**。理由是这些类型的消费者跨越了分层——
//! `agent_lifecycle`（agent 改名 / 删除时要认识 `CronPayload` 里的 agent 引用）
//! 与 `tools::definitions::core_tools`（工具 schema 汇编）都在 kernel，而调度
//! 器 / 执行器 / DB / 投递全在 ha-cron。
//!
//! **方向红线**：本模块不依赖 `cron` 的任何运行时件（DB / scheduler /
//! executor / delivery）。要 DB 行为的走 [`crate::cron_hooks`]。
//!
//! `ha_cron::…` 与 kernel 内既有的 `crate::cron::…` 路径经门面再导出保持不变。

pub(crate) mod types;

pub use types::*;
