//! ACP Control Plane — Configuration.
//!
//! Stored in `config.json` under the `acpControl` field.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Global ACP control config ────────────────────────────────────

/// Top-level ACP control plane configuration.
/// Persisted in `~/.hope-agent/config.json` → `acpControl`.
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

    /// Default timeout per turn (seconds). 0 = no ACP turn timeout.
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
            name: "Claude Code (ACP adapter)".into(),
            binary: "claude-agent-acp".into(),
            acp_args: vec![],
            protocol: AcpBackendProtocol::V1,
            distribution: Some(AcpDistributionDescriptor::package_adapter(
                "@agentclientprotocol/claude-agent-acp",
            )),
            enabled: true,
            default_model: None,
            env: HashMap::new(),
        },
        AcpBackendConfig {
            id: "codex-cli".into(),
            name: "Codex (ACP adapter)".into(),
            binary: "codex-acp".into(),
            acp_args: vec![],
            protocol: AcpBackendProtocol::V1,
            distribution: Some(AcpDistributionDescriptor::package_adapter(
                "@agentclientprotocol/codex-acp",
            )),
            enabled: true,
            default_model: None,
            env: HashMap::new(),
        },
        AcpBackendConfig {
            id: "gemini-cli".into(),
            name: "Gemini CLI".into(),
            binary: "gemini".into(),
            acp_args: vec!["--acp".into()],
            protocol: AcpBackendProtocol::V1,
            distribution: Some(AcpDistributionDescriptor::native("gemini-cli")),
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
    0
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

    /// ACP wire contract used by this exact distribution. Missing values from
    /// pre-v1 configs remain legacy; runtime launch still requires an explicit
    /// distribution descriptor and never guesses a command shape.
    #[serde(default)]
    pub protocol: AcpBackendProtocol,

    /// Provenance and install identity for the configured command. `None`
    /// means an old/unverified config and is rejected before process spawn.
    #[serde(default)]
    pub distribution: Option<AcpDistributionDescriptor>,

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcpBackendProtocol {
    V1,
    #[default]
    Legacy02,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpDistributionDescriptor {
    pub source: AcpDistributionSource,
    /// Package or native product identifier; never a floating download URL.
    pub package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub platform_files: Vec<AcpDistributionFile>,
    #[serde(default)]
    pub auth_method: AcpDistributionAuth,
}

impl AcpDistributionDescriptor {
    fn native(package: &str) -> Self {
        Self {
            source: AcpDistributionSource::Native,
            package: package.into(),
            version: None,
            platform_files: vec![],
            auth_method: AcpDistributionAuth::InheritedEnvironment,
        }
    }

    fn package_adapter(package: &str) -> Self {
        Self {
            source: AcpDistributionSource::PackageAdapter,
            package: package.into(),
            version: None,
            platform_files: vec![],
            auth_method: AcpDistributionAuth::InheritedEnvironment,
        }
    }

    pub fn custom(package: impl Into<String>) -> Self {
        Self {
            source: AcpDistributionSource::Custom,
            package: package.into(),
            version: None,
            platform_files: vec![],
            auth_method: AcpDistributionAuth::InheritedEnvironment,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcpDistributionSource {
    Native,
    PackageAdapter,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpDistributionFile {
    pub platform: String,
    pub architecture: String,
    pub file: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcpDistributionAuth {
    #[default]
    InheritedEnvironment,
    Terminal,
    None,
}
