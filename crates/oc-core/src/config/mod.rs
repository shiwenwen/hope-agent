//! Application configuration — the root structure persisted to `~/.opencomputer/config.json`.
//!
//! Historically named `ProviderStore`, this type actually owns the entire
//! user-facing config (providers, channels, memory, skills, tools, UI, server…).
//! It was renamed to `AppConfig` to match its real scope.
//!
//! The on-disk JSON shape is unchanged — all fields use `#[serde(rename_all = "camelCase")]`
//! and no wrapper struct is involved, so the Rust type name has zero impact on serialization.

mod persistence;

pub use persistence::{cached_config, load_config, reload_cache_from_disk, save_config};

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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferredToolsConfig {
    /// Enable deferred tool loading (default: false, opt-in)
    #[serde(default)]
    pub enabled: bool,
}

impl Default for DeferredToolsConfig {
    fn default() -> Self {
        Self { enabled: false }
    }
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
    /// Larger results are spooled to `~/.opencomputer/async_jobs/<job_id>.txt`
    /// and only a head/tail preview is injected. Default: 4096.
    #[serde(default = "default_async_inline_result_bytes")]
    pub inline_result_bytes: usize,
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

impl Default for AsyncToolsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_background_secs: default_async_auto_background_secs(),
            max_job_secs: default_async_max_job_secs(),
            inline_result_bytes: default_async_inline_result_bytes(),
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
}

impl Default for EmbeddedServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: default_server_bind(),
            api_key: None,
        }
    }
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
    /// Embedding model configuration for memory vector search
    #[serde(default)]
    pub embedding: crate::memory::EmbeddingConfig,
    /// Web search provider configuration
    #[serde(default)]
    pub web_search: crate::tools::web_search::WebSearchConfig,
    /// Web fetch tool configuration
    #[serde(default)]
    pub web_fetch: crate::tools::web_fetch::WebFetchConfig,
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
    /// the default `~/.opencomputer/plans/`.
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
            embedding: crate::memory::EmbeddingConfig::default(),
            memory_extract: crate::memory::MemoryExtractConfig::default(),
            memory_selection: crate::memory::MemorySelectionConfig::default(),
            dedup: crate::memory::DedupConfig::default(),
            hybrid_search: crate::memory::HybridSearchConfig::default(),
            temporal_decay: crate::memory::TemporalDecayConfig::default(),
            mmr: crate::memory::MmrConfig::default(),
            multimodal: crate::memory::MultimodalConfig::default(),
            embedding_cache: crate::memory::EmbeddingCacheConfig::default(),
            web_search: crate::tools::web_search::WebSearchConfig::default(),
            web_fetch: crate::tools::web_fetch::WebFetchConfig::default(),
            skill_env: std::collections::HashMap::new(),
            compact: crate::context_compact::CompactConfig::default(),
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
        }
    }
}
