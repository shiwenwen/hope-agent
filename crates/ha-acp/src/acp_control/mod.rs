//! ACP Control Plane — Client-side ACP runtime management.
//!
//! Enables Hope Agent's agent to spawn and control external ACP-compatible
//! agents (Claude Code, Codex CLI, Gemini CLI, etc.) as child processes.
//!
//! This is the **client** counterpart to `crate::acp` (the server).

pub mod config;
pub mod events;
pub mod health;
pub mod registry;
pub mod runtime_stdio;
pub mod session_manager;
pub mod types;

pub use config::{AcpControlConfig, AgentAcpConfig};
pub use registry::AcpRuntimeRegistry;
pub use session_manager::AcpSessionManager;

// ── 特征侧全局：ACP SessionManager ───────────────────────────────
//
// 原 `globals::ACP_MANAGER`（kernel god registry 的一员）随 crate 拆分迁入
// 特征侧：kernel 不再知道 AcpSessionManager 类型。创建在 wire() 注册的
// init 任务里（原 init_runtime 时序点），消费者 acp_spawn 工具与 HTTP
// 路由都在特征平面。

use std::sync::{Arc as AcpArc, OnceLock as AcpOnceLock};

static ACP_MANAGER: AcpOnceLock<AcpArc<AcpSessionManager>> = AcpOnceLock::new();

/// init 任务专用：设置全局 manager（OnceLock 语义，重复 set 忽略——与原
/// `let _ = ACP_MANAGER.set(...)` 逐位一致）。
pub fn set_acp_manager(manager: AcpArc<AcpSessionManager>) {
    let _ = ACP_MANAGER.set(manager);
}

/// Get stored AcpSessionManager for ACP control plane operations.
pub fn get_acp_manager() -> Option<&'static AcpArc<AcpSessionManager>> {
    ACP_MANAGER.get()
}
