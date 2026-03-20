use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Agent Config (agent.json) ────────────────────────────────────

/// Agent configuration, deserialized from agent.json.
/// All fields optional with sensible defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    /// Display name
    #[serde(default = "default_name")]
    pub name: String,

    /// Short description
    #[serde(default)]
    pub description: Option<String>,

    /// Emoji identifier
    #[serde(default)]
    pub emoji: Option<String>,

    /// Avatar file path or URL
    #[serde(default)]
    pub avatar: Option<String>,

    /// Model override (empty = use global activeModel)
    #[serde(default)]
    pub model: AgentModelConfig,

    /// Skill filtering
    #[serde(default)]
    pub skills: FilterConfig,

    /// Tool filtering
    #[serde(default)]
    pub tools: FilterConfig,

    /// Personality & identity settings
    #[serde(default)]
    pub personality: PersonalityConfig,

    /// Behavior settings
    #[serde(default)]
    pub behavior: BehaviorConfig,

    /// If true, use custom markdown prompts instead of structured config
    #[serde(default)]
    pub use_custom_prompt: bool,
}

fn default_name() -> String {
    "Assistant".to_string()
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: default_name(),
            description: None,
            emoji: None,
            avatar: None,
            model: AgentModelConfig::default(),
            skills: FilterConfig::default(),
            tools: FilterConfig::default(),
            personality: PersonalityConfig::default(),
            behavior: BehaviorConfig::default(),
            use_custom_prompt: false,
        }
    }
}

// ── Personality Config ──────────────────────────────────────────

/// Structured personality & identity for the Agent.
/// Inspired by OpenClaw's IDENTITY.md + SOUL.md, but as GUI-friendly fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalityConfig {
    /// What the agent is (e.g., "AI coding assistant", "creative writer", "robot butler")
    #[serde(default)]
    pub role: Option<String>,

    /// Overall personality vibe (e.g., "warm and patient", "sharp and direct", "chaotic creative")
    #[serde(default)]
    pub vibe: Option<String>,

    /// Communication tone (e.g., "formal", "casual", "playful", "professional")
    #[serde(default)]
    pub tone: Option<String>,

    /// Personality traits (e.g., ["curious", "detail-oriented", "encouraging"])
    #[serde(default)]
    pub traits: Vec<String>,

    /// Core guiding principles (e.g., ["Always explain reasoning", "Safety first"])
    #[serde(default)]
    pub principles: Vec<String>,

    /// What the agent will and won't do — behavioral boundaries
    #[serde(default)]
    pub boundaries: Option<String>,

    /// Personality quirks, catchphrases, or unique habits
    #[serde(default)]
    pub quirks: Option<String>,

    /// Communication style preferences (e.g., "verbose with examples", "minimal and terse")
    #[serde(default)]
    pub communication_style: Option<String>,
}

// ── Model Config ─────────────────────────────────────────────────

/// Optional model override for an Agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentModelConfig {
    /// Primary model: "provider_id/model_id", empty = global activeModel
    #[serde(default)]
    pub primary: Option<String>,

    /// Fallback models in order
    #[serde(default)]
    pub fallbacks: Vec<String>,
}

// ── Filter Config ────────────────────────────────────────────────

/// Generic allow/deny filter for skills and tools.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilterConfig {
    /// Allowlist (if non-empty, only these are permitted)
    #[serde(default)]
    pub allow: Vec<String>,

    /// Denylist (these are excluded)
    #[serde(default)]
    pub deny: Vec<String>,
}

impl FilterConfig {
    /// Check if a name passes through the filter.
    pub fn is_allowed(&self, name: &str) -> bool {
        if !self.allow.is_empty() && !self.allow.iter().any(|a| a == name) {
            return false;
        }
        if self.deny.iter().any(|d| d == name) {
            return false;
        }
        true
    }
}

// ── Behavior Config ──────────────────────────────────────────────

/// Agent behavior configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BehaviorConfig {
    /// Max tool-call loop rounds
    #[serde(default = "default_max_rounds")]
    pub max_tool_rounds: u32,

    /// Tools that require user approval before execution
    #[serde(default = "default_approval_tools")]
    pub require_approval: Vec<String>,

    /// Whether to use Docker sandbox by default
    #[serde(default)]
    pub sandbox: bool,

    /// Whether to check skill runtime requirements (bins/env/os) before injecting into system prompt.
    /// When true (default), skills whose requirements are not met are silently excluded.
    #[serde(default = "default_skill_env_check")]
    pub skill_env_check: bool,
}

fn default_max_rounds() -> u32 {
    10
}

fn default_approval_tools() -> Vec<String> {
    vec!["*".to_string()]
}

fn default_skill_env_check() -> bool {
    true
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            max_tool_rounds: default_max_rounds(),
            require_approval: default_approval_tools(),
            sandbox: false,
            skill_env_check: default_skill_env_check(),
        }
    }
}

// ── Agent Definition (runtime) ───────────────────────────────────

/// Complete Agent definition loaded from the filesystem.
/// Combines the JSON config with Markdown file contents.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AgentDefinition {
    /// Agent ID (directory name, e.g. "default", "coder")
    pub id: String,

    /// Absolute path to the agent directory
    pub dir: PathBuf,

    /// Parsed agent.json
    pub config: AgentConfig,

    /// agent.md content — what this agent does and how it works
    pub agent_md: Option<String>,

    /// persona.md content — personality and communication style
    pub persona: Option<String>,

    /// tools.md content — custom tool usage guidance
    pub tools_guide: Option<String>,
}

// ── Agent Summary (for listing) ──────────────────────────────────

/// Lightweight summary for the frontend agent list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub emoji: Option<String>,
    pub avatar: Option<String>,
    pub has_agent_md: bool,
    pub has_persona: bool,
    pub has_tools_guide: bool,
}
