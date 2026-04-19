//! Dangerous Mode — the global "nuclear button" that skips ALL tool-level
//! approval gates.
//!
//! Two independent sources feed the active state, combined with OR:
//!   1. CLI flag `--dangerously-skip-all-approvals` (process-scoped AtomicBool,
//!      set once in `main.rs` before any business logic runs, never persisted
//!      to disk).
//!   2. `AppConfig.dangerous_skip_all_approvals` (persisted to `config.json`,
//!      toggled via the Settings UI / `update_settings(category="security")`).
//!
//! Consumed by [`crate::tools::execution::execute_tool_with_context`] alongside
//! `ctx.auto_approve_tools`. Orthogonal to Plan Mode: YOLO skips the approval
//! gate, Plan Mode restricts tool types — both enforcements remain active.

use std::sync::atomic::{AtomicBool, Ordering};

static DANGEROUS_SKIP_CLI: AtomicBool = AtomicBool::new(false);

pub fn set_cli_flag(v: bool) {
    DANGEROUS_SKIP_CLI.store(v, Ordering::Relaxed);
}

pub fn cli_flag_active() -> bool {
    DANGEROUS_SKIP_CLI.load(Ordering::Relaxed)
}

fn config_flag_active() -> bool {
    crate::config::cached_config().dangerous_skip_all_approvals
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
