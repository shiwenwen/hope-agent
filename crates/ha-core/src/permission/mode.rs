//! Session-level permission mode + Smart mode configuration.

use serde::{Deserialize, Serialize};

/// Per-session permission mode. Stored in `sessions.permission_mode` column
/// and switched via the chat title bar dropdown.
///
/// Note: this is per-session, not process-global. The legacy
/// `tools::ToolPermissionMode` static was process-global despite the name —
/// this module replaces it with proper per-session state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    /// Default mode — hardcoded edit-class approval + agent custom-approval list.
    #[default]
    Default,
    /// Smart mode — defer to `_confidence` field on the tool_call OR an
    /// independent `judge_model` side_query.
    Smart,
    /// Session YOLO — all approvals silently allowed in this session
    /// (only Plan Mode can still block).
    Yolo,
}

impl SessionMode {
    /// `&str` matching the `#[serde(rename_all = "snake_case")]` encoding.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Smart => "smart",
            Self::Yolo => "yolo",
        }
    }

    /// Parse from DB / JSON string. Unknown values fall back to `Default`.
    pub fn parse_or_default(s: &str) -> Self {
        match s {
            "smart" => Self::Smart,
            "yolo" => Self::Yolo,
            _ => Self::Default,
        }
    }
}

/// Per-session sandbox posture. Stored in `sessions.sandbox_mode` and carried
/// into the permission engine as a policy input; it never overrides Plan Mode,
/// YOLO, protected paths, dangerous commands, or other strict gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// Execute tools on the host as before.
    #[default]
    Off,
    /// Run exec commands in the configured Docker sandbox, without approval
    /// relaxation. This preserves the legacy `capabilities.sandbox = true`
    /// behavior.
    Standard,
    /// Run exec commands against an isolated temporary workspace copy.
    Isolated,
    /// Run exec commands in Docker with the current workspace mounted.
    Workspace,
    /// Maximize sandbox-side autonomy while strict host/secret/browser gates
    /// still require approval.
    Trusted,
}

impl SandboxMode {
    /// `&str` matching the `#[serde(rename_all = "snake_case")]` encoding.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Standard => "standard",
            Self::Isolated => "isolated",
            Self::Workspace => "workspace",
            Self::Trusted => "trusted",
        }
    }

    /// Parse from DB / JSON string. Unknown values fall back to `Off`.
    pub fn parse_or_default(s: &str) -> Self {
        match s {
            "standard" => Self::Standard,
            "isolated" => Self::Isolated,
            "workspace" => Self::Workspace,
            "trusted" => Self::Trusted,
            _ => Self::Off,
        }
    }

    pub fn enabled(self) -> bool {
        self != Self::Off
    }

    /// Whether this mode may reduce soft approval prompts after strict gates.
    ///
    /// `Isolated` intentionally does not relax approvals in v1: exec runs in a
    /// temporary copy that is deleted after execution, so edit-command writes
    /// would otherwise appear successful while silently discarding changes.
    pub fn relaxes_soft_approvals(self) -> bool {
        matches!(self, Self::Workspace | Self::Trusted)
    }
}

/// How Smart mode reaches its decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmartStrategy {
    /// Read `_confidence` from the tool_call args; "high" → allow, else fallback.
    #[default]
    SelfConfidence,
    /// Run an independent `judge_model` side_query for every approvable call.
    JudgeModel,
    /// Try `SelfConfidence` first, fall back to `JudgeModel`, then to `fallback`.
    Both,
}

/// Smart mode configuration. Lives under `AppConfig.permission.smart`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartModeConfig {
    pub strategy: SmartStrategy,
    /// Required when `strategy` ∈ { JudgeModel, Both }. `None` → falls back.
    pub judge_model: Option<JudgeModelConfig>,
    /// What to do when Smart cannot decide (judge timeout, missing config, etc.).
    /// Defaults to `Default` mode behavior.
    #[serde(default)]
    pub fallback: SmartFallback,
}

/// Fallback action when Smart mode cannot decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmartFallback {
    /// Behave as if the session were in `Default` mode.
    #[default]
    Default,
    /// Force user prompt (most conservative).
    Ask,
    /// Silently allow (most permissive).
    Allow,
}

/// Configuration for the independent "judge model" used by Smart mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JudgeModelConfig {
    /// References a `ProviderConfig.id` from the global provider list.
    pub provider_id: String,
    /// Model name within the provider (e.g. "claude-haiku-4-5").
    pub model: String,
    /// User-supplied extra instructions for the judge prompt.
    /// Useful for whitelisting project paths, trusted commands, etc.
    #[serde(default)]
    pub extra_prompt: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_mode_as_str_round_trip() {
        for mode in [SessionMode::Default, SessionMode::Smart, SessionMode::Yolo] {
            let s = mode.as_str();
            assert_eq!(SessionMode::parse_or_default(s), mode);
        }
    }

    #[test]
    fn session_mode_parse_unknown() {
        assert_eq!(
            SessionMode::parse_or_default("nonsense"),
            SessionMode::Default
        );
        assert_eq!(SessionMode::parse_or_default(""), SessionMode::Default);
    }

    #[test]
    fn session_mode_serde_matches_as_str() {
        for mode in [SessionMode::Default, SessionMode::Smart, SessionMode::Yolo] {
            let via_serde = serde_json::to_value(mode)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string();
            assert_eq!(mode.as_str(), via_serde);
        }
    }

    #[test]
    fn sandbox_mode_as_str_round_trip() {
        for mode in [
            SandboxMode::Off,
            SandboxMode::Standard,
            SandboxMode::Isolated,
            SandboxMode::Workspace,
            SandboxMode::Trusted,
        ] {
            let s = mode.as_str();
            assert_eq!(SandboxMode::parse_or_default(s), mode);
        }
    }

    #[test]
    fn sandbox_mode_parse_unknown() {
        assert_eq!(SandboxMode::parse_or_default("nonsense"), SandboxMode::Off);
        assert_eq!(SandboxMode::parse_or_default(""), SandboxMode::Off);
    }

    #[test]
    fn sandbox_mode_serde_matches_as_str() {
        for mode in [
            SandboxMode::Off,
            SandboxMode::Standard,
            SandboxMode::Isolated,
            SandboxMode::Workspace,
            SandboxMode::Trusted,
        ] {
            let via_serde = serde_json::to_value(mode)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string();
            assert_eq!(mode.as_str(), via_serde);
        }
    }
}
