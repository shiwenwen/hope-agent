use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Agent Config (agent.json) ────────────────────────────────────

/// Agent configuration, deserialized from agent.json.
/// All fields optional with sensible defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    /// Lifecycle gate. Disabled Agents remain editable and keep their data,
    /// but cannot be selected for new executions.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,

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
            enabled: true,
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

    /// Default Think / reasoning effort for this agent.
    /// Session-level overrides still take priority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

// ── Filter Config ────────────────────────────────────────────────

/// Generic allow/deny pair.
///
/// Skill filtering still uses strict allowlist/denylist semantics via
/// [`FilterConfig::is_allowed`]. Tool switches interpret the same shape as
/// explicit non-Core on/off overrides in `dispatch::resolve_tool_fate`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilterConfig {
    /// Allowed names, or explicitly enabled tool names when used as tool switches.
    #[serde(default)]
    pub allow: Vec<String>,

    /// Denied names, or explicitly disabled tool names when used as tool switches.
    #[serde(default)]
    pub deny: Vec<String>,
}

impl FilterConfig {
    /// Check if a name passes through strict filter semantics.
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

    /// Whether to use Docker sandbox by default
    #[serde(default)]
    pub sandbox: bool,

    /// Default sandbox mode for new sessions opened under this agent.
    /// `None` preserves legacy behavior: `sandbox=true` maps to `standard`,
    /// otherwise `off`.
    #[serde(default)]
    pub default_sandbox_mode: Option<crate::permission::SandboxMode>,

    /// Whether to check skill runtime requirements before injecting into the
    /// system prompt. When true (default), hard blockers such as unsupported OS
    /// are excluded; recoverable missing dependencies remain visible and are
    /// diagnosed at activation time.
    #[serde(default = "default_skill_env_check")]
    pub skill_env_check: bool,

    /// Per-agent tool switch overrides for non-Core built-in tools.
    ///
    /// `allow` means explicitly enabled, `deny` means explicitly disabled, and
    /// absence from both lists falls back to the tool tier's default.
    #[serde(default)]
    pub tools: FilterConfig,

    /// Skill visibility filter (allow/deny by skill name)
    #[serde(default)]
    pub skills: FilterConfig,

    /// Async tool backgrounding policy override. Default: model-decide.
    #[serde(default)]
    pub async_tool_policy: AsyncToolPolicy,

    /// MCP integration master switch. When false, all MCP tools
    /// (`mcp_resource` / `mcp_prompt` / dynamic `mcp__<server>__<tool>`)
    /// are excluded from the LLM tool schema, and the system prompt
    /// surfaces a hint to enable MCP in agent settings.
    #[serde(default = "crate::default_true")]
    pub mcp_enabled: bool,

    /// Whether the agent owner has opted into "Custom Tool Approval".
    /// When false, `custom_approval_tools` is ignored — only the hardcoded
    /// edit-class enforcement (write / edit / apply_patch + edit-command exec
    /// matches + protected paths + dangerous commands) requires approval.
    /// When true and `permission_mode = default`, the tools listed in
    /// `custom_approval_tools` ALSO require approval.
    ///
    /// Smart / YOLO modes ignore both this flag and the list — UI must
    /// surface that note to avoid user confusion.
    #[serde(default)]
    pub enable_custom_tool_approval: bool,

    /// User-curated extra approval list. Only consumed when
    /// `enable_custom_tool_approval = true` AND the session is in
    /// `Default` mode. Tool names that do not appear here are NOT prompted —
    /// this list is additive on top of the hardcoded edit-class set.
    #[serde(default)]
    pub custom_approval_tools: Vec<String>,

    /// Default permission mode for new sessions opened under this agent.
    /// `None` falls back to the global default (currently `Default`).
    /// Existing sessions are unaffected when this changes.
    #[serde(default)]
    pub default_session_permission_mode: Option<crate::permission::SessionMode>,
}

impl CapabilitiesConfig {
    pub fn effective_default_sandbox_mode(&self) -> crate::permission::SandboxMode {
        self.default_sandbox_mode.unwrap_or_else(|| {
            if self.sandbox {
                crate::permission::SandboxMode::Standard
            } else {
                crate::permission::SandboxMode::Off
            }
        })
    }
}

fn default_max_rounds() -> u32 {
    0
}

fn default_skill_env_check() -> bool {
    true
}

impl Default for CapabilitiesConfig {
    fn default() -> Self {
        Self {
            max_tool_rounds: default_max_rounds(),
            sandbox: false,
            default_sandbox_mode: None,
            skill_env_check: default_skill_env_check(),
            tools: FilterConfig::default(),
            skills: FilterConfig::default(),
            async_tool_policy: AsyncToolPolicy::default(),
            mcp_enabled: true,
            enable_custom_tool_approval: false,
            custom_approval_tools: Vec::new(),
            default_session_permission_mode: None,
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

    /// User-saved Procedure Memory soft prompt injection (P5).
    /// Only active, high-confidence procedures selected by the retrieval
    /// planner enter a bounded dynamic block; episodes remain trace-only.
    #[serde(default)]
    pub procedure_memory: ProcedureMemoryConfig,

    /// P4 graph trace candidate expansion. This is trace-only: neighboring
    /// claims are exposed in Answer Memory Chips as candidates but are not
    /// injected into the prompt.
    #[serde(default)]
    pub graph_memory: GraphMemoryConfig,

    /// Cross-source Retrieval Planner candidate fusion. This governs the
    /// bounded diagnostic candidate set; already injected / selected context
    /// is never removed by this budget.
    #[serde(default)]
    pub retrieval_planner: RetrievalPlannerConfig,

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
/// (Phase B1). Default is disabled because the side query can add visible
/// latency; when enabled, failures and timeouts degrade silently to the passive
/// memory recall path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveMemoryConfig {
    /// Whether Active Memory is enabled. Default false — opt-in because the
    /// inline side_query before each user turn adds latency (typically 1–3s,
    /// up to `timeout_ms` on a cold reasoning model). Users who want recall
    /// injection turn it on per-agent in the Memory tab.
    #[serde(default)]
    pub enabled: bool,

    /// Side query timeout in milliseconds. Default 8000. On timeout we
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
    /// asking the LLM to pick the most relevant one. Default 10.
    #[serde(default = "default_active_memory_candidate_limit")]
    pub candidate_limit: usize,

    /// Active Memory v2 (next-gen Dreaming §7.5): also shortlist structured
    /// claims (not just legacy memories) as recall candidates. Default false —
    /// an opt-in layer on top of `enabled`, so existing v1 recall is unchanged
    /// until the user turns on claim recall.
    #[serde(default)]
    pub include_claims: bool,
}

fn default_active_memory_timeout_ms() -> u64 {
    // Reasoning models on Codex / Responses (gpt-5.x) routinely take 4–6s
    // even with `effort=low`, between TLS warm-up and SSE first byte.
    // 3s was the original budget but produced ~100% timeout in real logs.
    8000
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
    // 10 is enough to expose a relevant memory; 20 inflated the prompt and
    // pushed reasoning models past the 3s window without measurably better
    // recall. Users can raise this via `agent.activeMemory.candidateLimit`.
    10
}

impl Default for ActiveMemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_ms: default_active_memory_timeout_ms(),
            max_chars: default_active_memory_max_chars(),
            cache_ttl_secs: default_active_memory_cache_ttl_secs(),
            budget_tokens: default_active_memory_budget_tokens(),
            candidate_limit: default_active_memory_candidate_limit(),
            include_claims: false,
        }
    }
}

/// Procedure Memory configuration — controls P5 user-saved workflow soft
/// guidance. Defaults on because procedures are explicit user-authored or
/// user-promoted assets, but it is tightly bounded and can be disabled per
/// agent from the Memory tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcedureMemoryConfig {
    /// Whether to inject relevant saved workflows as soft guidance.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,

    /// Maximum number of procedures injected per turn.
    #[serde(default = "default_procedure_memory_max_procedures")]
    pub max_procedures: usize,

    /// Maximum characters for the whole dynamic workflow block.
    #[serde(default = "default_procedure_memory_max_chars")]
    pub max_chars: usize,

    /// Minimum confidence required before a procedure can be injected.
    #[serde(default = "default_procedure_memory_min_confidence")]
    pub min_confidence: f32,
}

fn default_procedure_memory_max_procedures() -> usize {
    1
}
fn default_procedure_memory_max_chars() -> usize {
    800
}
fn default_procedure_memory_min_confidence() -> f32 {
    0.7
}

impl ProcedureMemoryConfig {
    pub fn clamped(&self) -> Self {
        Self {
            enabled: self.enabled,
            max_procedures: self.max_procedures.clamp(1, 3),
            max_chars: self.max_chars.clamp(200, 2_000),
            min_confidence: self.min_confidence.clamp(0.0, 1.0),
        }
    }
}

impl Default for ProcedureMemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_procedures: default_procedure_memory_max_procedures(),
            max_chars: default_procedure_memory_max_chars(),
            min_confidence: default_procedure_memory_min_confidence(),
        }
    }
}

/// Graph Memory trace configuration — controls P4 claim-graph candidate
/// expansion in Retrieval Planner diagnostics. It is enabled by default but
/// bounded, and never injects graph text into the prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphMemoryConfig {
    /// Whether to expand related active claims into graph trace candidates.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,

    /// Maximum query-matched center claims to expand per turn.
    #[serde(default = "default_graph_memory_max_centers")]
    pub max_centers: usize,

    /// Maximum neighboring claims surfaced as trace candidates per turn.
    #[serde(default = "default_graph_memory_max_edges")]
    pub max_edges: usize,
}

fn default_graph_memory_max_centers() -> usize {
    3
}
fn default_graph_memory_max_edges() -> usize {
    6
}

impl GraphMemoryConfig {
    pub fn clamped(&self) -> Self {
        Self {
            enabled: self.enabled,
            max_centers: self.max_centers.clamp(1, 8),
            max_edges: self.max_edges.clamp(1, 20),
        }
    }
}

impl Default for GraphMemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_centers: default_graph_memory_max_centers(),
            max_edges: default_graph_memory_max_edges(),
        }
    }
}

/// Cross-source candidate fusion for Answer Memory diagnostics. Defaults are
/// intentionally automatic and bounded; advanced users can tune them per
/// Agent without changing the underlying stores or prompt-injection budgets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalPlannerConfig {
    /// Use lightweight query intent to boost the matching source family.
    #[serde(default = "crate::default_true")]
    pub intent_aware: bool,

    /// Maximum total refs retained after preserving all injected / selected
    /// refs. Candidate refs fill only the remaining room.
    #[serde(default = "default_retrieval_planner_max_trace_refs")]
    pub max_trace_refs: usize,

    /// Per-origin candidate cap after cross-source identity de-duplication.
    #[serde(default = "default_retrieval_planner_max_candidates_per_origin")]
    pub max_candidates_per_origin: usize,
}

fn default_retrieval_planner_max_trace_refs() -> usize {
    24
}

fn default_retrieval_planner_max_candidates_per_origin() -> usize {
    4
}

impl RetrievalPlannerConfig {
    pub fn clamped(&self) -> Self {
        Self {
            intent_aware: self.intent_aware,
            max_trace_refs: self.max_trace_refs.clamp(8, 64),
            max_candidates_per_origin: self.max_candidates_per_origin.clamp(1, 16),
        }
    }
}

impl Default for RetrievalPlannerConfig {
    fn default() -> Self {
        Self {
            intent_aware: true,
            max_trace_refs: default_retrieval_planner_max_trace_refs(),
            max_candidates_per_origin: default_retrieval_planner_max_candidates_per_origin(),
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
            procedure_memory: ProcedureMemoryConfig::default(),
            graph_memory: GraphMemoryConfig::default(),
            retrieval_planner: RetrievalPlannerConfig::default(),
            budget: None,
        }
    }
}

// ── Sub-Agent Config ────────────────────────────────────────────

/// Configuration for sub-agent delegation capabilities.
///
/// Note: whether the agent can spawn sub-agents is controlled by the
/// `subagent` tool switch in `capabilities.tools`. The fields here configure
/// delegation *behavior* (who's allowed, depth limits, timeouts), not the
/// master switch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentConfig {
    /// Which agents this agent is allowed to delegate to (empty = all)
    #[serde(default)]
    pub allowed_agents: Vec<String>,

    /// Which agents are denied (takes precedence over allowed)
    #[serde(default)]
    pub denied_agents: Vec<String>,

    /// Max concurrent sub-agents this agent can have running (default 8, clamped
    /// 1..=50 at the spawn gate). Wired through `subagent::max_concurrent_for_agent`.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,

    /// Default timeout for spawned sub-agents (seconds). 0 = no timeout.
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
    8
}

fn default_subagent_timeout() -> u64 {
    0
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
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
    /// Agent ID (directory name, e.g. "ha-main", "coder")
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
    pub enabled: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_memory_config_defaults_and_clamps() {
        let default = GraphMemoryConfig::default();
        assert!(default.enabled);
        assert_eq!(default.max_centers, 3);
        assert_eq!(default.max_edges, 6);

        let clamped = GraphMemoryConfig {
            enabled: false,
            max_centers: 0,
            max_edges: 999,
        }
        .clamped();

        assert!(!clamped.enabled);
        assert_eq!(clamped.max_centers, 1);
        assert_eq!(clamped.max_edges, 20);
    }

    #[test]
    fn retrieval_planner_config_defaults_and_clamps() {
        let default = RetrievalPlannerConfig::default();
        assert!(default.intent_aware);
        assert_eq!(default.max_trace_refs, 24);
        assert_eq!(default.max_candidates_per_origin, 4);

        let clamped = RetrievalPlannerConfig {
            intent_aware: false,
            max_trace_refs: 0,
            max_candidates_per_origin: usize::MAX,
        }
        .clamped();

        assert!(!clamped.intent_aware);
        assert_eq!(clamped.max_trace_refs, 8);
        assert_eq!(clamped.max_candidates_per_origin, 16);
    }

    #[test]
    fn legacy_memory_config_gets_retrieval_planner_defaults() {
        let config: MemoryConfig = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "shared": true,
            "promptBudget": 5000
        }))
        .expect("legacy memory config");

        assert!(config.retrieval_planner.intent_aware);
        assert_eq!(config.retrieval_planner.max_trace_refs, 24);
        assert_eq!(config.retrieval_planner.max_candidates_per_origin, 4);
    }
}
