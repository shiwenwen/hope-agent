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

    /// Personality & identity settings
    #[serde(default)]
    pub personality: PersonalityConfig,

    /// Capabilities: tools, skills, approval, sandbox, runtime limits
    #[serde(default)]
    pub capabilities: CapabilitiesConfig,

    /// Memory settings
    #[serde(default)]
    pub memory: MemoryConfig,

    /// If true, use the 4-file markdown prompt mode
    /// (AGENTS.md, IDENTITY.md, SOUL.md, TOOLS.md)
    #[serde(default)]
    pub openclaw_mode: bool,

    /// Per-agent notification override. None = use global setting.
    #[serde(default)]
    pub notify_on_complete: Option<bool>,

    /// Sub-agent delegation settings
    #[serde(default)]
    pub subagents: SubagentConfig,

    /// Agent Team settings
    #[serde(default)]
    pub team: TeamAgentConfig,

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
            personality: PersonalityConfig::default(),
            capabilities: CapabilitiesConfig::default(),
            memory: MemoryConfig::default(),
            openclaw_mode: false,
            notify_on_complete: None,
            subagents: SubagentConfig::default(),
            team: TeamAgentConfig::default(),
            acp: crate::acp_control::AgentAcpConfig::default(),
        }
    }
}

// ── Personality Config ──────────────────────────────────────────

/// Which persona authoring surface is active for this agent.
/// `Structured` uses the role/tone/values/principles fields below (default,
/// backward-compatible). `SoulMd` switches the prompt builder to inject the
/// agent's `soul.md` file verbatim — the same physical file used by openclaw
/// compatibility mode — and bypasses the structured fields for the
/// personality section. Structured fields remain editable in both modes so
/// switching between them does not lose data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PersonaMode {
    #[default]
    Structured,
    SoulMd,
}

/// Structured personality & identity for the Agent.
/// GUI-friendly fields that mirror the IDENTITY.md + SOUL.md file layout.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalityConfig {
    /// Persona authoring surface: structured fields vs. SOUL.md markdown.
    #[serde(default)]
    pub mode: PersonaMode,

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

// ── Capabilities Config ──────────────────────────────────────────

/// Per-agent override for async tool backgrounding behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AsyncToolPolicy {
    /// Default: respect `run_in_background` from the model and auto-background
    /// after the configured budget.
    #[default]
    ModelDecide,
    /// Force every async-capable tool call into a background job.
    AlwaysBackground,
    /// Disable async backgrounding entirely for this agent.
    NeverBackground,
}

/// Agent capabilities: what the agent can do and how.
/// Merges the former BehaviorConfig with top-level tools/skills filters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesConfig {
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

    /// Tool visibility filter (allow/deny by tool name)
    #[serde(default)]
    pub tools: FilterConfig,

    /// Skill visibility filter (allow/deny by skill name)
    #[serde(default)]
    pub skills: FilterConfig,

    /// Async tool backgrounding policy override. Default: model-decide.
    #[serde(default)]
    pub async_tool_policy: AsyncToolPolicy,
}

fn default_max_rounds() -> u32 {
    0
}

fn default_approval_tools() -> Vec<String> {
    vec!["*".to_string()]
}

fn default_skill_env_check() -> bool {
    true
}

impl Default for CapabilitiesConfig {
    fn default() -> Self {
        Self {
            max_tool_rounds: default_max_rounds(),
            require_approval: default_approval_tools(),
            sandbox: false,
            skill_env_check: default_skill_env_check(),
            tools: FilterConfig::default(),
            skills: FilterConfig::default(),
            async_tool_policy: AsyncToolPolicy::default(),
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
    #[serde(default = "crate::default_true")]
    pub enabled: bool,

    /// Whether to also load global shared memories
    #[serde(default = "crate::default_true")]
    pub shared: bool,

    /// Max chars for memory section in system prompt
    #[serde(default = "default_memory_budget")]
    pub prompt_budget: usize,

    /// Whether to auto-extract memories (None = inherit global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_extract: Option<bool>,

    /// Provider ID for memory extraction (None = inherit global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_provider_id: Option<String>,

    /// Model ID for memory extraction (None = inherit global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_model_id: Option<String>,

    /// Whether to flush memories before context compaction (None = inherit global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flush_before_compact: Option<bool>,

    /// Token threshold for extraction trigger (None = inherit global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_token_threshold: Option<usize>,

    /// Time threshold in seconds for extraction trigger (None = inherit global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_time_threshold_secs: Option<u64>,

    /// Message count threshold for extraction trigger (None = inherit global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_message_threshold: Option<usize>,

    /// Idle timeout in seconds for final extraction (None = inherit global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_idle_timeout_secs: Option<u64>,

    /// Phase B'2 — per-agent override for reflective extraction. None =
    /// inherit the global `MemoryExtractConfig.enable_reflection`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_reflection: Option<bool>,

    /// Active Memory pre-reply injection (Phase B1).
    /// When enabled, each user turn triggers a bounded side_query that
    /// distills the most relevant memory into a short sentence and injects
    /// it as an independent cache block alongside the system prompt.
    #[serde(default)]
    pub active_memory: ActiveMemoryConfig,

    /// Agent-level override for the system-prompt memory budget. `None`
    /// inherits `AppConfig.memory_budget`. When set, the full budget is
    /// replaced (not merged field-by-field) so an agent configured once can
    /// pick a coherent set of per-section caps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<crate::memory::MemoryBudgetConfig>,
}

/// Resolve the effective memory budget for a given agent: agent-level
/// `Option<MemoryBudgetConfig>` override wins over the global default.
pub fn effective_memory_budget(
    agent: &MemoryConfig,
    global: &crate::memory::MemoryBudgetConfig,
) -> crate::memory::MemoryBudgetConfig {
    agent.budget.clone().unwrap_or_else(|| global.clone())
}

/// Active Memory configuration — controls the pre-reply recall injection
/// (Phase B1). Default is enabled with conservative timeouts; failures and
/// timeouts degrade silently to the passive memory recall path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveMemoryConfig {
    /// Whether Active Memory is enabled. Default true.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,

    /// Side query timeout in milliseconds. Default 3000. On timeout we
    /// simply don't append the Active Memory block and fall back to the
    /// static memory section already in the system prompt.
    #[serde(default = "default_active_memory_timeout_ms")]
    pub timeout_ms: u64,

    /// Upper bound (chars) for the recall sentence we ask the LLM for.
    /// Default 220 (mirrors OpenClaw `active-memory` default maxChars).
    #[serde(default = "default_active_memory_max_chars")]
    pub max_chars: usize,

    /// Cache TTL (seconds) keyed by hash(user_message). Repeating the same
    /// question within the TTL window reuses the cached recall without a
    /// side_query call. Default 15s.
    #[serde(default = "default_active_memory_cache_ttl_secs")]
    pub cache_ttl_secs: u64,

    /// max_tokens budget for the side_query call. Default 512.
    #[serde(default = "default_active_memory_budget_tokens")]
    pub budget_tokens: u32,

    /// How many candidate memories to shortlist from the backend before
    /// asking the LLM to pick the most relevant one. Default 20.
    #[serde(default = "default_active_memory_candidate_limit")]
    pub candidate_limit: usize,
}

fn default_active_memory_timeout_ms() -> u64 {
    3000
}
fn default_active_memory_max_chars() -> usize {
    220
}
fn default_active_memory_cache_ttl_secs() -> u64 {
    15
}
fn default_active_memory_budget_tokens() -> u32 {
    512
}
fn default_active_memory_candidate_limit() -> usize {
    20
}

impl Default for ActiveMemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_ms: default_active_memory_timeout_ms(),
            max_chars: default_active_memory_max_chars(),
            cache_ttl_secs: default_active_memory_cache_ttl_secs(),
            budget_tokens: default_active_memory_budget_tokens(),
            candidate_limit: default_active_memory_candidate_limit(),
        }
    }
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
            extract_provider_id: None,
            extract_model_id: None,
            flush_before_compact: None,
            extract_token_threshold: None,
            extract_time_threshold_secs: None,
            extract_message_threshold: None,
            extract_idle_timeout_secs: None,
            enable_reflection: None,
            active_memory: ActiveMemoryConfig::default(),
            budget: None,
        }
    }
}

// ── Sub-Agent Config ────────────────────────────────────────────

/// Configuration for sub-agent delegation capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentConfig {
    /// Whether this agent can spawn sub-agents
    #[serde(default = "crate::default_true")]
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

    /// Max tasks per batch_spawn call (1-50, default 10)
    #[serde(default)]
    pub max_batch_size: Option<u32>,

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
            max_batch_size: None,
            archive_after_minutes: None,
            announce_timeout_secs: None,
        }
    }
}

// ── Team Agent Config ──────────────────────────────────────────

/// Configuration for agent team capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamAgentConfig {
    /// Whether this agent can create/lead teams
    #[serde(default = "crate::default_true")]
    pub enabled: bool,

    /// Max active teams per agent (default 3)
    #[serde(default = "default_max_teams")]
    pub max_active_teams: u32,

    /// Max members per team (default 8)
    #[serde(default = "default_max_team_members")]
    pub max_members_per_team: u32,

    /// Default model for team members
    #[serde(default)]
    pub member_model: Option<String>,
}

fn default_max_teams() -> u32 {
    crate::team::MAX_ACTIVE_TEAMS
}

fn default_max_team_members() -> u32 {
    crate::team::DEFAULT_MAX_MEMBERS
}

impl Default for TeamAgentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_active_teams: default_max_teams(),
            max_members_per_team: default_max_team_members(),
            member_model: None,
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

    /// agents.md content — workspace rules/instructions
    pub agents_md: Option<String>,

    /// identity.md content — agent identity metadata
    pub identity_md: Option<String>,

    /// soul.md content — personality/values/tone
    pub soul_md: Option<String>,

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
