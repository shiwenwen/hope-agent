//! Hope Agent 基础层 —— 依赖图最底层，零业务逻辑、零 Tauri 依赖。
//!
//! **红线：ha-base 不得依赖 ha-core 或任何 ha-* 业务 crate。** 需要上层数据
//! （如 `AppConfig` 字段）时留注册钩子，由上层在启动时注入，不反向依赖——
//! 现有两处：[`paths::register_plans_dir_source`] 与
//! [`security::dangerous::register_config_flag_source`]。
//!
//! 上层 `ha-core` 通过 `pub use ha_base::<模块>;` 全量再导出，因此
//! `ha_core::paths::…` 与 ha-core 内部的 `crate::paths::…` 都照旧可用，
//! 搬迁对调用方零改动。

// ── 宏必须最先声明 ────────────────────────────────────────────────
#[macro_use]
pub mod logging;

pub mod blocking;
pub mod crash_journal;
pub mod event_bus;
pub mod execution_mode;
pub mod paths;
pub mod permissions;
pub mod platform;
pub mod process_registry;
pub mod runtime_lock;
pub mod runtime_role;
pub mod security;
pub mod service_install;
pub mod terminal;
pub mod ttl_cache;
// `pub`（原 ha-core 内为私有 `mod util;` + 根 glob 再导出）：ha-core 里有 23 处
// `use crate::util::…` 走的是模块路径而非根再导出，跨 crate 后必须真实可达。
pub mod util;
pub mod workflow_mode;

#[cfg(target_os = "macos")]
pub mod weather_location_macos;

pub use util::*;
// 模式判定与版本号原语在根命名空间可用（`crate::is_desktop()` 等既有路径）。
pub use runtime_role::{app_version, is_acp, is_desktop, runtime_role, set_app_version};

// ── 日志全局 ──────────────────────────────────────────────────────
// `app_info!` 等宏展开为 `$crate::get_logger()`，`$crate` 解析到**定义宏的
// crate**（这里是 ha-base），所以 logger 全局必须与宏同 crate。只搬宏不搬
// 全局会让所有调用点编译失败。
//
// 其余全局（SESSION_DB / MEMORY_BACKEND / CHANNEL_REGISTRY …）留在 ha-core
// `globals.rs`：它们的类型来自业务模块，下沉即把环带下来。
use logging::{AppLogger, LogDB};
use std::sync::Arc;

pub static APP_LOGGER: std::sync::OnceLock<AppLogger> = std::sync::OnceLock::new();
pub static LOG_DB: std::sync::OnceLock<Arc<LogDB>> = std::sync::OnceLock::new();

pub fn get_logger() -> Option<&'static AppLogger> {
    APP_LOGGER.get()
}

/// 错误文案与搬迁前 `globals.rs` 的 `require_accessor!` 宏逐字一致
/// （`concat!($label, " not initialized")`，label 为 `"AppLogger"`）。
/// HTTP helper 会把它原样透传给调用方，改文案即改对外契约。
pub fn require_logger() -> anyhow::Result<&'static AppLogger> {
    APP_LOGGER
        .get()
        .ok_or_else(|| anyhow::anyhow!("AppLogger not initialized"))
}

pub fn get_log_db() -> Option<&'static Arc<LogDB>> {
    LOG_DB.get()
}

/// 同上，原 label 为 `"Log DB"`。
pub fn require_log_db() -> anyhow::Result<&'static Arc<LogDB>> {
    LOG_DB
        .get()
        .ok_or_else(|| anyhow::anyhow!("Log DB not initialized"))
}
