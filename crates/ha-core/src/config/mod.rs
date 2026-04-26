//! Application configuration — the root structure persisted to `~/.hope-agent/config.json`.
//!
//! Historically named `ProviderStore`, this type actually owns the entire
//! user-facing config (providers, channels, memory, skills, tools, UI, server…).
//! It was renamed to `AppConfig` to match its real scope.
//!
//! The on-disk JSON shape is unchanged — all fields use `#[serde(rename_all = "camelCase")]`
//! and no wrapper struct is involved, so the Rust type name has zero impact on serialization.

mod persistence;

pub use persistence::{
    cached_config, load_config, mutate_config, reload_cache_from_disk, save_config,
};

use serde::{Deserialize, Serialize};

use crate::provider::{ActiveModel, ProviderConfig, ProxyConfig};

// ── Shortcut Config ─────────────────────────────────────────────

/// A single keyboard shortcut binding
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutBinding {
    /// Unique identifier for this shortcut action
    pub id: String,
    /// The shortcut key combination (e.g. "Alt+Space", "CommandOrControl+Shift+K")
    /// Empty string means disabled.
    pub keys: String,
    /// Whether this shortcut is enabled
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
}

/// Global shortcut configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutConfig {
    /// All shortcut bindings
    #[serde(default = "default_shortcut_bindings")]
    pub bindings: Vec<ShortcutBinding>,
}

fn default_shortcut_bindings() -> Vec<ShortcutBinding> {
    vec![ShortcutBinding {
        id: "quickChat".to_string(),
        keys: "Alt+Space".to_string(),
        enabled: true,
    }]
}

impl ShortcutBinding {
    /// Whether this binding is a chord (two sequential key combos separated by space).
    /// e.g. "CommandOrControl+K CommandOrControl+C"
    pub fn is_chord(&self) -> bool {
        self.chord_parts().len() > 1
    }

    /// Split keys into chord parts. Single combo returns vec of 1.
    pub fn chord_parts(&self) -> Vec<&str> {
        self.keys.split_whitespace().collect()
    }
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            bindings: default_shortcut_bindings(),
        }
    }
}

// ── Notification Config ─────────────────────────────────────────

/// Global notification configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationConfig {
    /// Global on/off toggle (default: true)
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

// ── Deferred Tools Config ───────────────────────────────────────

/// Configuration for deferred tool loading.
/// When enabled, only core tools are sent to the LLM per request,
/// and remaining tools are discoverable via `tool_search`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferredToolsConfig {
    /// Enable deferred tool loading (default: false, opt-in)
    #[serde(default)]
    pub enabled: bool,
}

// ── Async Tools Config ──────────────────────────────────────────

/// Configuration for the async tool execution feature.
///
/// Async-capable tools (e.g. `exec`, `web_search`, `image_generate`) can be
/// detached into background jobs in three ways:
/// 1. The model passes `run_in_background: true` in tool args (explicit opt-in).
/// 2. The agent policy forces it (`async_tool_policy = "always-background"`).
/// 3. A sync call exceeds `auto_background_secs` (auto-transfer fallback).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsyncToolsConfig {
    /// Master switch. When false, all tool calls run synchronously regardless
    /// of `run_in_background` / agent policy.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    /// Auto-background budget for sync calls of async-capable tools.
    /// When a sync call exceeds this many seconds, the still-running future
    /// is transferred to a background async job and a synthetic job_id is
    /// returned to the model so the conversation can continue. The real
    /// result is delivered later via auto-injection. Default: 30. Set to 0
    /// to disable auto-backgrounding.
    #[serde(default = "default_async_auto_background_secs")]
    pub auto_background_secs: u64,
    /// Maximum time (seconds) a backgrounded job may run before being killed.
    /// Default: 1800 (30 min). 0 = no per-job limit (still bounded by
    /// `tool_timeout`).
    #[serde(default = "default_async_max_job_secs")]
    pub max_job_secs: u64,
    /// Number of result bytes to inline in the synthetic completion notification.
    /// Larger results are spooled to `~/.hope-agent/async_jobs/<job_id>.txt`
    /// and only a head/tail preview is injected. Default: 4096.
    #[serde(default = "default_async_inline_result_bytes")]
    pub inline_result_bytes: usize,
    /// Retention period for terminal async job rows + their spool files.
    /// Jobs whose `completed_at` is older than this are purged by a daily
    /// background task (plus one sweep at startup). Default: 30 days.
    /// `0` disables retention entirely.
    #[serde(default = "default_async_retention_secs")]
    pub retention_secs: u64,
    /// Orphan spool file grace period. Files under `~/.hope-agent/async_jobs/`
    /// whose name is not referenced by any row and whose mtime is older than
    /// this many seconds are considered orphaned and deleted. Default: 24h.
    /// The grace window prevents races with in-flight writes from freshly
    /// started jobs whose DB row hasn't committed yet.
    #[serde(default = "default_async_orphan_grace_secs")]
    pub orphan_grace_secs: u64,
    /// Absolute ceiling for a single `job_status(block=true)` wait, in
    /// seconds. Used only when `max_job_secs == 0` (unlimited job runtime);
    /// when `max_job_secs > 0` the ceiling equals `max_job_secs` so the
    /// `job_status` cap never exceeds the job's own runtime cap. Default:
    /// 7200 (2h).
    #[serde(default = "default_async_job_status_max_wait_secs")]
    pub job_status_max_wait_secs: u64,
}

fn default_async_auto_background_secs() -> u64 {
    30
}
fn default_async_max_job_secs() -> u64 {
    1800
}
fn default_async_inline_result_bytes() -> usize {
    4096
}
fn default_async_retention_secs() -> u64 {
    30 * crate::SECS_PER_DAY
}
fn default_async_orphan_grace_secs() -> u64 {
    24 * crate::SECS_PER_HOUR
}
fn default_async_job_status_max_wait_secs() -> u64 {
    7200
}

impl AsyncToolsConfig {
    /// Upper bound on a single `job_status(block=true)` wait, in seconds.
    /// Mirrors `max_job_secs` when it's positive; otherwise falls back to
    /// `job_status_max_wait_secs` (clamped to ≥ 1).
    pub fn job_status_ceiling_secs(&self) -> u64 {
        if self.max_job_secs == 0 {
            self.job_status_max_wait_secs.max(1)
        } else {
            self.max_job_secs
        }
    }
}

impl Default for AsyncToolsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_background_secs: default_async_auto_background_secs(),
            max_job_secs: default_async_max_job_secs(),
            inline_result_bytes: default_async_inline_result_bytes(),
            retention_secs: default_async_retention_secs(),
            orphan_grace_secs: default_async_orphan_grace_secs(),
            job_status_max_wait_secs: default_async_job_status_max_wait_secs(),
        }
    }
}

/// What to do when a tool approval request times out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalTimeoutAction {
    /// Block tool execution when approval timed out.
    #[default]
    Deny,
    /// Continue tool execution when approval timed out.
    Proceed,
}

// ── Default helpers ─────────────────────────────────────────────

fn default_skill_env_check() -> bool {
    true
}

fn default_conditional_skills_enabled() -> bool {
    true
}

pub(crate) fn default_tool_timeout() -> u64 {
    300
}

pub(crate) fn default_approval_timeout() -> u64 {
    300
}

pub(crate) fn default_ask_user_question_timeout() -> u64 {
    1800
}

pub(crate) fn default_theme() -> String {
    "auto".to_string()
}

pub(crate) fn default_language() -> String {
    "auto".to_string()
}

// ── Recap Config ────────────────────────────────────────────────

fn default_recap_default_range_days() -> u32 {
    30
}
fn default_recap_max_sessions_per_report() -> u32 {
    500
}
fn default_recap_facet_concurrency() -> u8 {
    4
}
fn default_recap_cache_retention_days() -> u32 {
    180
}

/// Configuration for the `/recap` deep-analysis report feature.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecapConfig {
    /// Agent ID used to extract per-session facets and generate report sections.
    /// `None` falls back to the first available agent.
    #[serde(default)]
    pub analysis_agent: Option<String>,
    /// Default time window (days) when no prior report exists.
    #[serde(default = "default_recap_default_range_days")]
    pub default_range_days: u32,
    /// Hard cap on number of sessions analyzed in a single report.
    #[serde(default = "default_recap_max_sessions_per_report")]
    pub max_sessions_per_report: u32,
    /// Concurrency for per-session facet extraction.
    #[serde(default = "default_recap_facet_concurrency")]
    pub facet_concurrency: u8,
    /// Days to retain cached session facets before garbage collection.
    #[serde(default = "default_recap_cache_retention_days")]
    pub cache_retention_days: u32,
}

impl Default for RecapConfig {
    fn default() -> Self {
        Self {
            analysis_agent: None,
            default_range_days: default_recap_default_range_days(),
            max_sessions_per_report: default_recap_max_sessions_per_report(),
            facet_concurrency: default_recap_facet_concurrency(),
            cache_retention_days: default_recap_cache_retention_days(),
        }
    }
}

// ── Embedded Server Config ──────────────────────────────────────

fn default_server_bind() -> String {
    "127.0.0.1:8420".to_string()
}

/// Embedded HTTP/WS server configuration, stored in config.json `server` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedServerConfig {
    /// Bind address (default "127.0.0.1:8420").
    /// Set to "0.0.0.0:8420" to expose to the network.
    #[serde(default = "default_server_bind")]
    pub bind_addr: String,
    /// API Key for authenticating requests (None = no auth).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Publicly-reachable base URL for this server, used when IM channels
    /// that only accept remote HTTPS media (LINE, IRC text fallback) need to
    /// send `/api/attachments/...` links. `None` disables those fallbacks.
    /// Format: `https://example.com` (no trailing slash).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_base_url: Option<String>,
}

impl Default for EmbeddedServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: default_server_bind(),
            api_key: None,
            public_base_url: None,
        }
    }
}

// ── Onboarding State ────────────────────────────────────────────

/// Current onboarding wizard version. Bump this to force existing users
/// through the wizard again after meaningful additions (new steps, new
/// required fields, etc.).
pub const CURRENT_ONBOARDING_VERSION: u32 = 1;

/// First-run onboarding wizard state. Drives both the GUI wizard
/// (`src/components/onboarding`) and the CLI wizard
/// (`src-tauri/src/cli_onboarding`).
///
/// A user is considered "onboarded" when `completed_version >=
/// CURRENT_ONBOARDING_VERSION`. `0` (the default) means never completed, so
/// new installs land in the wizard. When a user quits mid-wizard, the
/// front-end writes `draft` + `draft_step` so the next launch can resume.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingState {
    /// Highest wizard version the user has finished. `0` = never completed.
    #[serde(default)]
    pub completed_version: u32,
    /// ISO 8601 timestamp of the most recent completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    /// Step keys the user explicitly skipped in the most recent run.
    #[serde(default)]
    pub skipped_steps: Vec<String>,
    /// Partially-filled draft captured when the user exits mid-wizard.
    /// The shape is an opaque JSON object owned by the front-end; the
    /// backend only persists and returns it verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<serde_json::Value>,
    /// Step index (0-based) the user was on when they exited.
    #[serde(default)]
    pub draft_step: u32,
    /// Sticky flag: true once the wizard has ever completed at any version.
    /// Survives `reset()` so explicit rerun doesn't get caught by the
    /// legacy-upgrade heuristic in `infer_legacy_completed`, which would
    /// otherwise skip the wizard for users who already have providers.
    #[serde(default)]
    pub ever_completed: bool,
}

// ── App Config ──────────────────────────────────────────────────

/// Root structure for the application's persisted configuration (`config.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub active_model: Option<ActiveModel>,
    /// Global fallback model chain (ordered).
    /// When the primary model fails, these are tried in order.
    #[serde(default)]
    pub fallback_models: Vec<ActiveModel>,
    /// Extra directories to scan for skills
    #[serde(default)]
    pub extra_skills_dirs: Vec<String>,
    /// Disabled skill names
    #[serde(default)]
    pub disabled_skills: Vec<String>,
    /// Whether to check skill runtime requirements (bins/env/os) before injecting.
    /// Default true. When false, all skills are injected regardless of environment.
    #[serde(default = "default_skill_env_check")]
    pub skill_env_check: bool,
    /// Kill switch for `paths:` conditional skill activation. Default true.
    /// When false, file/path-aware tools stop activating `paths:` skills, so
    /// those skills remain hidden unless they have already been activated for
    /// the session.
    #[serde(default = "default_conditional_skills_enabled")]
    pub conditional_skills_enabled: bool,
    /// Reusable embedding model configurations.
    #[serde(default)]
    pub embedding_models: Vec<crate::memory::EmbeddingModelConfig>,
    /// Active memory vector-search embedding selection.
    #[serde(default)]
    pub memory_embedding: crate::memory::MemoryEmbeddingSelection,
    /// Deprecated legacy embedding config. Kept as a deserialization sink only;
    /// user-facing embedding config lives in `embedding_models` +
    /// `memory_embedding`.
    #[serde(default, skip_serializing)]
    pub embedding: crate::memory::EmbeddingConfig,
    /// Web search provider configuration
    #[serde(default)]
    pub web_search: crate::tools::web_search::WebSearchConfig,
    /// Web fetch tool configuration
    #[serde(default)]
    pub web_fetch: crate::tools::web_fetch::WebFetchConfig,
    /// SSRF policy configuration (browser / web_fetch / image_generate / url_preview)
    #[serde(default)]
    pub ssrf: crate::security::ssrf::SsrfConfig,
    /// Per-skill environment variable overrides configured by user.
    /// Outer key: skill name, inner key: env var name, value: env var value.
    #[serde(default)]
    pub skill_env: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    /// Global memory auto-extract configuration
    #[serde(default)]
    pub memory_extract: crate::memory::MemoryExtractConfig,
    /// LLM-based memory selection configuration
    #[serde(default)]
    pub memory_selection: crate::memory::MemorySelectionConfig,
    /// Per-section character budgets for the system-prompt Memory block.
    #[serde(default)]
    pub memory_budget: crate::memory::MemoryBudgetConfig,
    /// Memory deduplication thresholds
    #[serde(default)]
    pub dedup: crate::memory::DedupConfig,
    /// Hybrid search weight configuration
    #[serde(default)]
    pub hybrid_search: crate::memory::HybridSearchConfig,
    /// Temporal decay configuration for memory search
    #[serde(default)]
    pub temporal_decay: crate::memory::TemporalDecayConfig,
    /// MMR reranking configuration
    #[serde(default)]
    pub mmr: crate::memory::MmrConfig,
    /// Multimodal embedding configuration (image/audio)
    #[serde(default)]
    pub multimodal: crate::memory::MultimodalConfig,
    /// Embedding cache configuration
    #[serde(default)]
    pub embedding_cache: crate::memory::EmbeddingCacheConfig,
    /// Context compaction configuration
    #[serde(default)]
    pub compact: crate::context_compact::CompactConfig,
    /// LLM-generated session title configuration
    #[serde(default)]
    pub session_title: crate::session_title::SessionTitleConfig,
    /// Notification configuration
    #[serde(default)]
    pub notification: NotificationConfig,
    /// Image generation configuration
    #[serde(default)]
    pub image_generate: crate::tools::image_generate::ImageGenConfig,
    /// Canvas tool configuration
    #[serde(default)]
    pub canvas: crate::tools::canvas::CanvasConfig,
    /// Image tool configuration (max images per call, etc.)
    #[serde(default)]
    pub image: crate::tools::image::ImageToolConfig,
    /// PDF tool configuration (max PDFs, max vision pages, etc.)
    #[serde(default)]
    pub pdf: crate::tools::pdf::PdfToolConfig,
    /// Global hard timeout (seconds) for a single tool execution.
    /// Safety net for when inner tool timeouts don't fire (network issues, etc.).
    /// Default 300 (5 min). Set to 0 to disable.
    #[serde(default = "default_tool_timeout")]
    pub tool_timeout: u64,
    /// Timeout in seconds for waiting on an interactive tool approval response.
    /// Default 300 (5 min). Set to 0 to disable and wait forever.
    #[serde(default = "default_approval_timeout")]
    pub approval_timeout_secs: u64,
    /// What to do when an approval request times out.
    /// Default: deny. Alternative: proceed.
    #[serde(default)]
    pub approval_timeout_action: ApprovalTimeoutAction,
    /// Threshold (bytes) for persisting large tool results to disk.
    /// Results exceeding this size are written to disk with a preview in context.
    /// Default: 50000 (50KB). Set to 0 to disable.
    #[serde(default)]
    pub tool_result_disk_threshold: Option<usize>,
    /// UI theme preference: "auto" | "light" | "dark"
    #[serde(default = "default_theme")]
    pub theme: String,
    /// UI language preference: "auto" means follow system, otherwise a locale code like "zh", "en"
    #[serde(default = "default_language")]
    pub language: String,
    /// Whether UI background effects (stars, weather) are enabled
    #[serde(default = "crate::default_true")]
    pub ui_effects_enabled: bool,
    /// Global proxy configuration for all outgoing HTTP requests
    #[serde(default)]
    pub proxy: ProxyConfig,
    /// Configurable limits for skill prompt generation
    #[serde(default)]
    pub skill_prompt_budget: crate::skills::SkillPromptBudget,
    /// Bundled skills allowlist (empty = all allowed)
    #[serde(default)]
    pub skill_allow_bundled: Vec<String>,

    /// ACP control plane configuration (external agent management)
    #[serde(default)]
    pub acp_control: crate::acp_control::AcpControlConfig,

    /// Global keyboard shortcut configuration
    #[serde(default)]
    pub shortcuts: ShortcutConfig,

    /// Custom plans directory override. When set, plans are stored here instead of
    /// the default `~/.hope-agent/plans/`.
    #[serde(default)]
    pub plans_directory: Option<String>,

    /// Global default temperature for LLM API calls (0.0–2.0).
    /// Can be overridden at the agent level or session level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// Whether to use a dedicated sub-agent for plan creation (Planning phase).
    /// When true, planning runs in an isolated sub-agent (saves main agent context).
    /// When false, planning runs inline in the main agent (preserves context continuity).
    /// Default: false (inline mode)
    #[serde(default)]
    pub plan_subagent: bool,

    /// Timeout in seconds for ask_user_question tool waiting for user response.
    /// Default: 1800 (30 minutes). 0 = no timeout (wait forever).
    #[serde(default = "default_ask_user_question_timeout")]
    pub ask_user_question_timeout_secs: u64,

    /// IM channel configuration (Telegram, Discord, Slack, etc.)
    #[serde(default)]
    pub channels: crate::channel::ChannelStoreConfig,

    /// Deferred tool loading configuration
    #[serde(default)]
    pub deferred_tools: DeferredToolsConfig,

    /// Embedded HTTP/WS server configuration
    #[serde(default)]
    pub server: EmbeddedServerConfig,

    /// Recap (deep session analysis) configuration
    #[serde(default)]
    pub recap: RecapConfig,

    /// Async tool execution configuration (run_in_background, auto-background, etc.)
    #[serde(default)]
    pub async_tools: AsyncToolsConfig,

    /// Behavior awareness configuration. Provides each chat with a
    /// dynamically-refreshed view of what the user is doing in other
    /// parallel sessions.
    #[serde(default)]
    pub awareness: crate::awareness::AwarenessConfig,

    /// Offline memory consolidation ("Dreaming", Phase B3).
    /// Controls when cycles run (idle / cron / manual) and how aggressively
    /// they promote candidates into pinned core memory.
    #[serde(default)]
    pub dreaming: crate::memory::dreaming::DreamingConfig,

    /// Skills automation (Phase B'). Nests `autoReview` knobs that drive the
    /// post-conversation skill CRUD pipeline.
    #[serde(default)]
    pub skills: crate::skills::SkillsConfig,

    /// Opt-in LLM summarization layer on top of `recall_memory` /
    /// `session_search` tool output (Phase B'3). Default disabled.
    #[serde(default)]
    pub recall_summary: crate::memory::RecallSummaryConfig,

    /// Whether to inject `TOOL_CALL_NARRATION_GUIDANCE` into the system prompt.
    /// When enabled, the model announces each tool call with a one-sentence
    /// preamble (Claude Code style). Some models (e.g. GPT-5.4 via Codex) over-
    /// apply the rule and restate the same intent across consecutive tool calls,
    /// which reads as noise. Default `false` — opt-in when you want the
    /// narration behavior.
    #[serde(default)]
    pub tool_call_narration_enabled: bool,

    /// ⚠️ DANGEROUS MODE — globally skip ALL tool-level approvals. When true,
    /// every tool (exec / write / edit / apply_patch / channel tools / browser /
    /// canvas …) runs without any approval gate. Overrides `ctx.auto_approve_tools`,
    /// `ToolPermissionMode`, and every other per-session / per-channel auto-approve
    /// knob by short-circuiting the gate in `tools::execution`. Plan Mode's
    /// tool-type whitelist remains in effect (the two are orthogonal). Intended
    /// for fully-trusted local automation; do not enable on shared machines.
    #[serde(default)]
    pub dangerous_skip_all_approvals: bool,

    /// First-run onboarding wizard state. See [`OnboardingState`].
    #[serde(default)]
    pub onboarding: OnboardingState,

    /// Configured Model Context Protocol (MCP) servers. Each entry describes
    /// a stdio / http / sse / ws endpoint that contributes tools (and later
    /// prompts / resources) to the main conversation catalog. See
    /// `docs/architecture/mcp.md` for the full subsystem overview.
    #[serde(default)]
    pub mcp_servers: Vec<crate::mcp::McpServerConfig>,

    /// Global knobs shared by every MCP server: master switch, concurrency
    /// caps, backoff policy, always-load whitelist, denylist. Defaults are
    /// tuned to be safe — the MCP subsystem is `enabled=true` but
    /// `mcp_servers=[]` means new installs see no behavioral change.
    #[serde(default)]
    pub mcp_global: crate::mcp::McpGlobalSettings,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            active_model: None,
            fallback_models: Vec::new(),
            extra_skills_dirs: Vec::new(),
            disabled_skills: Vec::new(),
            skill_env_check: true,
            conditional_skills_enabled: true,
            embedding_models: Vec::new(),
            memory_embedding: crate::memory::MemoryEmbeddingSelection::default(),
            embedding: crate::memory::EmbeddingConfig::default(),
            memory_extract: crate::memory::MemoryExtractConfig::default(),
            memory_selection: crate::memory::MemorySelectionConfig::default(),
            memory_budget: crate::memory::MemoryBudgetConfig::default(),
            dedup: crate::memory::DedupConfig::default(),
            hybrid_search: crate::memory::HybridSearchConfig::default(),
            temporal_decay: crate::memory::TemporalDecayConfig::default(),
            mmr: crate::memory::MmrConfig::default(),
            multimodal: crate::memory::MultimodalConfig::default(),
            embedding_cache: crate::memory::EmbeddingCacheConfig::default(),
            web_search: crate::tools::web_search::WebSearchConfig::default(),
            web_fetch: crate::tools::web_fetch::WebFetchConfig::default(),
            ssrf: crate::security::ssrf::SsrfConfig::default(),
            skill_env: std::collections::HashMap::new(),
            compact: crate::context_compact::CompactConfig::default(),
            session_title: crate::session_title::SessionTitleConfig::default(),
            notification: NotificationConfig::default(),
            image_generate: crate::tools::image_generate::ImageGenConfig::default(),
            canvas: crate::tools::canvas::CanvasConfig::default(),
            image: crate::tools::image::ImageToolConfig::default(),
            pdf: crate::tools::pdf::PdfToolConfig::default(),
            tool_timeout: default_tool_timeout(),
            approval_timeout_secs: default_approval_timeout(),
            approval_timeout_action: ApprovalTimeoutAction::default(),
            tool_result_disk_threshold: None,
            theme: default_theme(),
            language: default_language(),
            ui_effects_enabled: true,
            proxy: ProxyConfig::default(),
            skill_prompt_budget: crate::skills::SkillPromptBudget::default(),
            skill_allow_bundled: Vec::new(),
            acp_control: crate::acp_control::AcpControlConfig::default(),
            shortcuts: ShortcutConfig::default(),
            plans_directory: None,
            temperature: None,
            plan_subagent: false,
            ask_user_question_timeout_secs: default_ask_user_question_timeout(),
            channels: crate::channel::ChannelStoreConfig::default(),
            deferred_tools: DeferredToolsConfig::default(),
            server: EmbeddedServerConfig::default(),
            recap: RecapConfig::default(),
            async_tools: AsyncToolsConfig::default(),
            awareness: crate::awareness::AwarenessConfig::default(),
            dreaming: crate::memory::dreaming::DreamingConfig::default(),
            skills: crate::skills::SkillsConfig::default(),
            recall_summary: crate::memory::RecallSummaryConfig::default(),
            tool_call_narration_enabled: false,
            dangerous_skip_all_approvals: false,
            onboarding: OnboardingState::default(),
            mcp_servers: Vec::new(),
            mcp_global: crate::mcp::McpGlobalSettings::default(),
        }
    }
}

#[cfg(test)]
mod mcp_compat_tests {
    use super::*;

    // Backward-compat: a config.json produced before MCP landed must still
    // deserialize cleanly. The empty-object test is the minimum guarantee;
    // the providers-only test simulates what actual users have on disk.

    #[test]
    fn bare_providers_deserializes_with_mcp_defaults() {
        // `providers` is the only non-default field on AppConfig today; a
        // JSON with just that key is the minimum shape we want to guarantee
        // still parses after adding MCP fields.
        let cfg: AppConfig = serde_json::from_str(r#"{"providers":[]}"#)
            .expect("bare providers config should deserialize");
        assert!(cfg.mcp_servers.is_empty());
        assert!(cfg.mcp_global.enabled);
        assert_eq!(cfg.mcp_global.max_concurrent_calls, 8);
    }

    #[test]
    fn pre_mcp_config_deserializes() {
        // Representative subset of a real pre-MCP config.json: non-trivial
        // providers + theme + shortcuts but no mcp_* keys at all.
        let json = serde_json::json!({
            "providers": [],
            "theme": "dark",
            "language": "zh",
            "shortcuts": { "bindings": [] },
        });
        let cfg: AppConfig = serde_json::from_value(json).expect("pre-mcp config should load");
        assert_eq!(cfg.theme, "dark");
        assert_eq!(cfg.language, "zh");
        assert!(cfg.mcp_servers.is_empty());
        assert!(cfg.mcp_global.enabled);
    }

    #[test]
    fn mcp_servers_roundtrip() {
        use crate::mcp::{McpServerConfig, McpTransportSpec, McpTrustLevel};
        let server = McpServerConfig {
            id: "11111111-2222-3333-4444-555555555555".into(),
            name: "memory".into(),
            enabled: true,
            transport: McpTransportSpec::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "@modelcontextprotocol/server-memory".into()],
                cwd: None,
            },
            env: Default::default(),
            headers: Default::default(),
            oauth: None,
            allowed_tools: vec![],
            denied_tools: vec![],
            connect_timeout_secs: 30,
            call_timeout_secs: 120,
            health_check_interval_secs: 60,
            max_concurrent_calls: 4,
            auto_approve: false,
            trust_level: McpTrustLevel::Untrusted,
            eager: false,
            project_paths: vec![],
            description: Some("local knowledge base".into()),
            icon: None,
            created_at: 0,
            updated_at: 0,
            trust_acknowledged_at: None,
        };
        let mut cfg = AppConfig::default();
        cfg.mcp_servers.push(server.clone());

        let text = serde_json::to_string(&cfg).expect("serialize");
        let round: AppConfig = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(round.mcp_servers.len(), 1);
        assert_eq!(round.mcp_servers[0].name, "memory");
        assert!(matches!(
            round.mcp_servers[0].transport,
            McpTransportSpec::Stdio { .. }
        ));
    }

    #[test]
    fn mcp_global_disabled_roundtrip() {
        // Kill-switch scenario: user opts out via config file.
        let json = serde_json::json!({
            "providers": [],
            "mcpGlobal": { "enabled": false, "maxConcurrentCalls": 0 }
        });
        let cfg: AppConfig = serde_json::from_value(json).unwrap();
        assert!(!cfg.mcp_global.enabled);
        assert_eq!(cfg.mcp_global.max_concurrent_calls, 0);
        // Other defaults remain intact:
        assert_eq!(cfg.mcp_global.backoff_max_secs, 300);
    }
}
