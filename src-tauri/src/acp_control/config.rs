//! ACP Control Plane — Configuration.
//!
//! Stored in `config.json` under the `acpControl` field.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Global ACP control config ────────────────────────────────────

/// Top-level ACP control plane configuration.
/// Persisted in `~/.opencomputer/config.json` → `acpControl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpControlConfig {
    /// Master switch for the ACP control plane.
    #[serde(default)]
    pub enabled: bool,

    /// Registered backend configurations.
    #[serde(default = "default_backends")]
    pub backends: Vec<AcpBackendConfig>,

    /// Maximum number of concurrent ACP sessions across all agents.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_sessions: u32,

    /// Default timeout per turn (seconds).
    #[serde(default = "default_timeout")]
    pub default_timeout_secs: u64,

    /// Idle TTL: close child processes that have been idle for this many seconds.
    #[serde(default = "default_runtime_ttl")]
    pub runtime_ttl_secs: u64,

    /// Automatically scan $PATH for known ACP agent binaries on startup.
    #[serde(default = "crate::default_true")]
    pub auto_discover: bool,
}

impl Default for AcpControlConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backends: default_backends(),
            max_concurrent_sessions: default_max_concurrent(),
            default_timeout_secs: default_timeout(),
            runtime_ttl_secs: default_runtime_ttl(),
            auto_discover: true,
        }
    }
}

fn default_backends() -> Vec<AcpBackendConfig> {
    vec![
        AcpBackendConfig {
            id: "claude-code".into(),
            name: "Claude Code".into(),
            binary: "claude".into(),
            acp_args: vec![],
            enabled: true,
            default_model: None,
            env: HashMap::new(),
        },
        AcpBackendConfig {
            id: "codex-cli".into(),
            name: "Codex CLI".into(),
            binary: "codex".into(),
            acp_args: vec![],
            enabled: true,
            default_model: None,
            env: HashMap::new(),
        },
        AcpBackendConfig {
            id: "gemini-cli".into(),
            name: "Gemini CLI".into(),
            binary: "gemini".into(),
            acp_args: vec![],
            enabled: true,
            default_model: None,
            env: HashMap::new(),
        },
    ]
}

fn default_max_concurrent() -> u32 {
    5
}

fn default_timeout() -> u64 {
    600
}

fn default_runtime_ttl() -> u64 {
    1800
}

// ── Per-backend config ───────────────────────────────────────────

/// Configuration for a single ACP backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpBackendConfig {
    /// Unique backend identifier (e.g. "claude-code").
    pub id: String,

    /// Human-readable display name.
    pub name: String,

    /// Binary name or absolute path (e.g. "claude", "/usr/local/bin/claude").
    /// Resolved via $PATH if not an absolute path.
    pub binary: String,

    /// Extra arguments appended when launching in ACP mode.
    #[serde(default)]
    pub acp_args: Vec<String>,

    /// Whether this backend is enabled.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,

    /// Default model to request from the external agent.
    #[serde(default)]
    pub default_model: Option<String>,

    /// Environment variable overrides for the child process.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

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
