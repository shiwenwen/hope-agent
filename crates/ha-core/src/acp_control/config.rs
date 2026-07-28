//! ACP Control Plane — Configuration.
//!
//! `AcpControlConfig` / `AcpBackendConfig` 已下沉 ha-config-schema（config.json
//! wire 类型），此处原地再导出保持路径不变；`AgentAcpConfig` 属 `agent.json`
//! 不在 AppConfig 闭包内，留在本 crate。

use serde::{Deserialize, Serialize};

pub use ha_config_schema::acp_control::{AcpBackendConfig, AcpControlConfig};

// ── Per-Agent ACP config ─────────────────────────────────────────

/// Per-agent ACP delegation settings.
/// Stored in `agent.json` → `acp`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAcpConfig {
    /// Whether this agent is allowed to use ACP external agents.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,

    /// Allowlist of backend IDs this agent may use (empty = all).
    #[serde(default)]
    pub allowed_backends: Vec<String>,

    /// Denylist of backend IDs (takes precedence over allowed).
    #[serde(default)]
    pub denied_backends: Vec<String>,

    /// Max concurrent ACP sessions for this agent.
    #[serde(default = "default_agent_max_concurrent")]
    pub max_concurrent: u32,
}

fn default_agent_max_concurrent() -> u32 {
    3
}

impl Default for AgentAcpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_backends: Vec::new(),
            denied_backends: Vec::new(),
            max_concurrent: default_agent_max_concurrent(),
        }
    }
}

impl AgentAcpConfig {
    /// Check if a backend is allowed by this agent's policy.
    pub fn is_backend_allowed(&self, backend_id: &str) -> bool {
        if self
            .denied_backends
            .iter()
            .any(|d| d.eq_ignore_ascii_case(backend_id))
        {
            return false;
        }
        if self.allowed_backends.is_empty() {
            return true;
        }
        self.allowed_backends
            .iter()
            .any(|a| a.eq_ignore_ascii_case(backend_id))
    }
}
