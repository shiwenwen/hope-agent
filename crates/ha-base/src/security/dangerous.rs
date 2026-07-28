//! Dangerous Mode (a.k.a. "Global YOLO") — the nuclear button that skips ALL
//! tool-level approval gates.
//!
//! Two independent sources feed the active state, combined with OR:
//!   1. CLI flag `--dangerously-skip-all-approvals` (process-scoped AtomicBool,
//!      set once in `main.rs` before any business logic runs, never persisted
//!      to disk).
//!   2. `AppConfig.permission.global_yolo` (persisted to `config.json`,
//!      toggled via the Settings UI / `update_settings(category="permission")`).
//!
//! Consumed by `ha_core::permission::engine::resolve`. Orthogonal to Plan Mode:
//! YOLO skips the approval gate, Plan Mode restricts tool types — both
//! enforcements remain active simultaneously.

use std::sync::atomic::{AtomicBool, Ordering};

static DANGEROUS_SKIP_CLI: AtomicBool = AtomicBool::new(false);

pub fn set_cli_flag(v: bool) {
    DANGEROUS_SKIP_CLI.store(v, Ordering::Relaxed);
}

pub fn cli_flag_active() -> bool {
    DANGEROUS_SKIP_CLI.load(Ordering::Relaxed)
}

/// `permission.global_yolo` 的来源钩子。
///
/// 与 [`crate::paths::register_plans_dir_source`] 同理：配置类型住在上层，
/// ha-base 不能反向依赖。每次调用都读实时配置，语义与原 `cached_config()` 一致。
///
/// **安全方向**：未注册时返回 `false`，即 Dangerous Mode 的**配置来源被视为未
/// 开启**——漏注册只会让权限更严（fail-closed），不会意外放行。CLI 来源
/// （`set_cli_flag`）不经此钩子，始终生效。
static CONFIG_FLAG_SOURCE: std::sync::OnceLock<fn() -> bool> = std::sync::OnceLock::new();

/// 注册 `global_yolo` 来源。
///
/// **返回 `Err` 表示已有来源被注册过**，调用方必须视为致命错误而非忽略。这个
/// 函数经 ha-core 的 glob 门面对外公开（`ha_core::security::dangerous::…`），
/// 若静默吞掉冲突，任何更早的注册都会**永久顶替** canonical 来源，而
/// `init_runtime` 仍报告初始化成功——控制全局审批跳过的开关被悄悄换掉是不可
/// 接受的失败模式。
pub fn register_config_flag_source(f: fn() -> bool) -> Result<(), ConfigFlagSourceAlreadySet> {
    CONFIG_FLAG_SOURCE
        .set(f)
        .map_err(|_| ConfigFlagSourceAlreadySet)
}

/// `register_config_flag_source` 的冲突标记。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigFlagSourceAlreadySet;

impl std::fmt::Display for ConfigFlagSourceAlreadySet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "dangerous-mode config flag source already registered")
    }
}

impl std::error::Error for ConfigFlagSourceAlreadySet {}

fn config_flag_active() -> bool {
    CONFIG_FLAG_SOURCE.get().map(|f| f()).unwrap_or(false)
}

pub fn is_dangerous_skip_active() -> bool {
    cli_flag_active() || config_flag_active()
}

/// Human-readable tag for which source is currently enabling Dangerous Mode.
/// CLI wins the tie because it's non-clearable and most surprising in logs.
/// Caller should only invoke this when `is_dangerous_skip_active()` is true.
pub fn active_source() -> &'static str {
    if cli_flag_active() {
        "CLI flag"
    } else {
        "config"
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DangerousModeStatus {
    pub cli_flag: bool,
    pub config_flag: bool,
    pub active: bool,
}

pub fn status() -> DangerousModeStatus {
    let cli = cli_flag_active();
    let cfg = config_flag_active();
    DangerousModeStatus {
        cli_flag: cli,
        config_flag: cfg,
        active: cli || cfg,
    }
}
