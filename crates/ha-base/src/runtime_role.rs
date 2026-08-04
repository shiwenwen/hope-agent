//! 进程运行模式（desktop / server / acp / mcp / test）与用户可见版本号的
//! 全局单一来源。
//!
//! 原住 ha-core `app_init.rs`。下沉 ha-base 的动机：`is_desktop()` /
//! `runtime_role()` 是全仓最常用的模式判定原语（AGENTS.md：模式判定用它、
//! 别给共享函数加 mode 参数），住在装配层会让每个查询者都背上一条
//! →app_init 的边。角色**由装配层写入**（`init_runtime` 开头调
//! [`set_runtime_role`]），本模块只持有全局与查询器，不含任何装配逻辑。

use std::sync::OnceLock;

/// Records the runtime role passed to `init_runtime("desktop"|"server"|"acp"|"test")`.
/// First-write-wins. Tests in the same binary share this `OnceLock` — once
/// `init_runtime("test")` runs, `is_desktop()` stays `false` for every test.
static RUNTIME_ROLE: OnceLock<&'static str> = OnceLock::new();

/// User-facing app version, set by each binary entrypoint via
/// [`set_app_version`]. Distinct from `env!("CARGO_PKG_VERSION")` in
/// library crates — `pnpm sync:version` syncs `package.json` →
/// `src-tauri/Cargo.toml` + `tauri.conf.json`, but does NOT touch library
/// crate versions, so those drift behind the app version and must not be
/// used for "current version" comparisons in the updater path.
static APP_VERSION: OnceLock<&'static str> = OnceLock::new();

/// Record the process role. First call wins（幂等；并发第二次 set 的 Err
/// 被丢弃）。只应由装配入口（`init_runtime`）调用。
pub fn set_runtime_role(role: &'static str) {
    let _ = RUNTIME_ROLE.set(role);
}

/// Register the calling binary's `CARGO_PKG_VERSION` so [`app_version`]
/// returns the user-facing app version. Idempotent; first call wins.
pub fn set_app_version(version: &'static str) {
    let _ = APP_VERSION.set(version);
}

/// The binary's user-facing version, falling back to this library's own
/// `CARGO_PKG_VERSION` when the entrypoint didn't register one (tests).
pub fn app_version() -> &'static str {
    APP_VERSION
        .get()
        .copied()
        .unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// Returns the role string from the first `init_runtime()` call, or `None`
/// if `init_runtime` hasn't run yet. Most callers want [`is_desktop`] for
/// readable mode checks instead of comparing the string directly.
pub fn runtime_role() -> Option<&'static str> {
    RUNTIME_ROLE.get().copied()
}

/// True iff the process started as the desktop (Tauri) shell. Used by paths
/// that need to vary behavior by runtime mode without threading a parameter
/// through the call stack (e.g. `system_prompt::build` injecting
/// desktop-only guidance for clickable file paths).
pub fn is_desktop() -> bool {
    runtime_role() == Some("desktop")
}

/// True iff the process started as the ACP stdio bridge (`hope-agent acp`).
/// ACP runs over stdio for an editor client (Zed etc.); approvals can only
/// reach a human if that client declared a permission capability (Epic D7).
/// **Must use this, not `ChatSource`** — ACP turns reuse `ChatSource::Http`
/// (`ha_acp::acp`), so source alone can't distinguish ACP from a real HTTP
/// client (D1 risk note).
pub fn is_acp() -> bool {
    runtime_role() == Some("acp")
}
