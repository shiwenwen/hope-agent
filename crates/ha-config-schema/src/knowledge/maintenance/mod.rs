//! Knowledge Layer-2 维护流水线的配置子集（镜像 ha-core
//! `knowledge::maintenance`）：只含 `config` 模块；调度器 / 生成器 / 提案
//! 类型仍在 ha-core。

pub mod config;

pub use config::{
    MaintenanceConfig, MaintenanceCronTrigger, MaintenanceIdleTrigger, MaintenanceTasks,
};
