//! ACP Control Plane — Configuration.
//!
//! `AcpControlConfig` / `AcpBackendConfig` 已下沉 ha-config-schema（config.json
//! wire 类型）；`AgentAcpConfig` 属 `agent.json`、不在 AppConfig 闭包内，
//! 已下沉 ha-core `agent_config.rs`（消费者在 kernel 侧）。本文件只做
//! 原路径再导出，保持两组类型的既有引用不变。

pub use ha_config_schema::acp_control::{AcpBackendConfig, AcpControlConfig};

// ── Per-Agent ACP config ─────────────────────────────────────────

// `AgentAcpConfig` 已下沉 kernel（`agent_config.rs`，agent.json wire 类型的
// 消费者在 kernel 侧），此处原地再导出保持路径不变。
pub use ha_core::agent_config::AgentAcpConfig;
