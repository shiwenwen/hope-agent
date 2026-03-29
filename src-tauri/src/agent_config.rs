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

    /// Memory settings
    #[serde(default)]
    pub memory: MemoryConfig,

    /// If true, use custom markdown prompts instead of structured config
    #[serde(default)]
    pub use_custom_prompt: bool,

    /// Per-agent notification override. None = use global setting.
    #[serde(default)]
    pub notify_on_complete: Option<bool>,

    /// Sub-agent delegation settings
    #[serde(default)]
    pub subagents: SubagentConfig,

    /// ACP external agent delegation settings
    #[serde(default)]
    pub acp: crate::acp_control::AgentAcpConfig,
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
            memory: MemoryConfig::default(),
            use_custom_prompt: false,
            notify_on_complete: None,
            subagents: SubagentConfig::default(),
            acp: crate::acp_control::AgentAcpConfig::default(),
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
#[serde(rename_all = "camelCase")]
pub struct AgentModelConfig {
    /// Primary model: "provider_id/model_id", empty = global activeModel
    #[serde(default)]
    pub primary: Option<String>,

    /// Fallback models in order
    #[serde(default)]
    pub fallbacks: Vec<String>,

    /// Model override for Plan Mode planning phase: "provider_id/model_id".
    /// Uses a cheaper/faster model for exploration & planning, saving cost.
    /// When set, Planning state will use this model instead of primary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_model: Option<String>,

    /// Temperature override for this agent (0.0–2.0).
    /// When set, overrides the global temperature. Can be further overridden at session level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
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

// ── Memory Config ───────────────────────────────────────────────

/// Memory system configuration in agent.json.
/// Extract-related fields are Option — None means "inherit from global config".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConfig {
    /// Whether memory is enabled for this agent
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Whether to also load global shared memories
    #[serde(default = "default_true")]
    pub shared: bool,

    /// Max chars for memory section in system prompt
    #[serde(default = "default_memory_budget")]
    pub prompt_budget: usize,

    /// Whether to auto-extract memories (None = inherit global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_extract: Option<bool>,

    /// Minimum conversation turns before extraction (None = inherit global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_min_turns: Option<usize>,

    /// Provider ID for memory extraction (None = inherit global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_provider_id: Option<String>,

    /// Model ID for memory extraction (None = inherit global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_model_id: Option<String>,

    /// Whether to flush memories before context compaction (None = inherit global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flush_before_compact: Option<bool>,
}

fn default_true() -> bool {
    true
}

fn default_memory_budget() -> usize {
    5000
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            shared: true,
            prompt_budget: default_memory_budget(),
            auto_extract: None,
            extract_min_turns: None,
            extract_provider_id: None,
            extract_model_id: None,
            flush_before_compact: None,
        }
    }
}

// ── Sub-Agent Config ────────────────────────────────────────────

/// Configuration for sub-agent delegation capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentConfig {
    /// Whether this agent can spawn sub-agents
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Which agents this agent is allowed to delegate to (empty = all)
    #[serde(default)]
    pub allowed_agents: Vec<String>,

    /// Which agents are denied (takes precedence over allowed)
    #[serde(default)]
    pub denied_agents: Vec<String>,

    /// Max concurrent sub-agents this agent can have running
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,

    /// Default timeout for spawned sub-agents (seconds)
    #[serde(default = "default_subagent_timeout")]
    pub default_timeout_secs: u64,

    /// Model override for sub-agents (e.g., use a cheaper model for delegation)
    #[serde(default)]
    pub model: Option<String>,

    /// Tools denied for sub-agents spawned by this agent (e.g., ["browser", "exec"])
    #[serde(default)]
    pub denied_tools: Vec<String>,

    /// Max nesting depth override (1-5, default 3)
    #[serde(default)]
    pub max_spawn_depth: Option<u32>,

    /// Auto-archive completed runs after N minutes (None = no auto-archive)
    #[serde(default)]
    pub archive_after_minutes: Option<u64>,

    /// Max seconds to wait for parent session to become idle before injection (default 120)
    #[serde(default)]
    pub announce_timeout_secs: Option<u64>,
}

fn default_max_concurrent() -> u32 {
    5
}

fn default_subagent_timeout() -> u64 {
    300
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_agents: Vec::new(),
            denied_agents: Vec::new(),
            max_concurrent: default_max_concurrent(),
            default_timeout_secs: default_subagent_timeout(),
            model: None,
            denied_tools: Vec::new(),
            max_spawn_depth: None,
            archive_after_minutes: None,
            announce_timeout_secs: None,
        }
    }
}

impl SubagentConfig {
    /// Check if delegation to a specific agent is allowed.
    pub fn is_agent_allowed(&self, agent_id: &str) -> bool {
        if self.denied_agents.iter().any(|d| d == agent_id) {
            return false;
        }
        if !self.allowed_agents.is_empty() && !self.allowed_agents.iter().any(|a| a == agent_id) {
            return false;
        }
        true
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

    /// Global memory.md content — shared core memory across all agents
    pub global_memory_md: Option<String>,

    /// Agent-level memory.md content — core memory specific to this agent
    pub memory_md: Option<String>,
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
    pub has_memory_md: bool,
    pub memory_count: usize,
    pub notify_on_complete: Option<bool>,
}
