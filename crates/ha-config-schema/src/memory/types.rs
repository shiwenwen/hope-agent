//! Legacy memory config wire types（原 ha-core `memory/types.rs` 中属于
//! `AppConfig` 闭包的部分）。
//!
//! **边界**：external provider 只有 wire 配置（Kind / SyncPolicy / Config /
//! ProvidersConfig + 写入规范化）住在这里；健康 / 预检 / 同步报告等运行时
//! 投影与决策逻辑全部在 ha-core `memory/types.rs`。跨界的方法
//! （`capabilities` / `data_flow` / `supported_by` / `sync_preflight*`）已改
//! 为 ha-core 自由函数——不要为了保住方法语法把运行时类型拖回本 crate。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

const MAX_EXTERNAL_MEMORY_PROVIDERS: usize = 16;
const MAX_EXTERNAL_PROVIDER_ID_CHARS: usize = 64;
const MAX_EXTERNAL_PROVIDER_DISPLAY_CHARS: usize = 80;
/// 可见性升级：ha-core 侧测试引用（原为模块私有 const）。
pub const MAX_EXTERNAL_PROVIDER_ERROR_CHARS: usize = 512;
const MAX_EXTERNAL_PROVIDER_TIMESTAMP_CHARS: usize = 80;

// ── External Memory Providers ───────────────────────────────────

/// Supported external memory provider families. These are additive provider
/// adapters, not replacements for the local SQLite truth source.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalMemoryProviderKind {
    Mem0,
    Zep,
    Supermemory,
    Honcho,
    Hindsight,
    OpenViking,
    Custom,
}

impl ExternalMemoryProviderKind {
    pub const ALL: [Self; 7] = [
        Self::Mem0,
        Self::Zep,
        Self::Supermemory,
        Self::Honcho,
        Self::Hindsight,
        Self::OpenViking,
        Self::Custom,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mem0 => "mem0",
            Self::Zep => "zep",
            Self::Supermemory => "supermemory",
            Self::Honcho => "honcho",
            Self::Hindsight => "hindsight",
            Self::OpenViking => "open_viking",
            Self::Custom => "custom",
        }
    }
}

/// External provider sync policy. `Off` is the default and the only policy
/// that has no external IO. Active policies remain explicit opt-ins and do not
/// change local prompt / recall semantics.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalMemorySyncPolicy {
    #[default]
    Off,
    Manual,
    PullOnly,
    PushOnly,
    Bidirectional,
}

impl ExternalMemorySyncPolicy {
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn sends_query_context(&self) -> bool {
        matches!(self, Self::Manual | Self::PullOnly | Self::Bidirectional)
    }

    pub fn sends_local_memory(&self) -> bool {
        matches!(self, Self::Manual | Self::PushOnly | Self::Bidirectional)
    }

    pub fn imports_external_memory(&self) -> bool {
        matches!(self, Self::Manual | Self::PullOnly | Self::Bidirectional)
    }
}

/// Persisted, non-secret provider config. Secrets must stay in the platform
/// credential store or environment references when concrete adapters land.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMemoryProviderConfig {
    pub id: String,
    pub kind: ExternalMemoryProviderKind,
    pub display_name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub sync_policy: ExternalMemorySyncPolicy,
    #[serde(default)]
    pub endpoint_configured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Global external memory provider switch. Default disabled so no user memory
/// leaves the device unless a future owner UI explicitly opts in.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMemoryProvidersConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub providers: Vec<ExternalMemoryProviderConfig>,
}

impl ExternalMemoryProvidersConfig {
    pub fn normalized(mut self) -> Self {
        self.providers.truncate(MAX_EXTERNAL_MEMORY_PROVIDERS);
        let mut used_ids = HashSet::new();
        self.providers = self
            .providers
            .into_iter()
            .enumerate()
            .map(|(index, provider)| normalize_external_provider(provider, index, &mut used_ids))
            .collect();
        self
    }
}

fn normalize_external_provider(
    mut provider: ExternalMemoryProviderConfig,
    index: usize,
    used_ids: &mut HashSet<String>,
) -> ExternalMemoryProviderConfig {
    let fallback_id = format!("{}-{}", provider.kind.as_str(), index + 1);
    let base_id = sanitize_external_provider_id(&provider.id)
        .filter(|id| !id.is_empty())
        .unwrap_or(fallback_id);
    provider.id = unique_external_provider_id(base_id, used_ids);

    provider.display_name = truncate_chars(
        provider.display_name.trim(),
        MAX_EXTERNAL_PROVIDER_DISPLAY_CHARS,
    );
    if provider.display_name.is_empty() {
        provider.display_name = provider.kind.as_str().to_string();
    }

    provider.last_error = provider
        .last_error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_chars(value, MAX_EXTERNAL_PROVIDER_ERROR_CHARS));
    provider.last_sync_at = provider
        .last_sync_at
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_chars(value, MAX_EXTERNAL_PROVIDER_TIMESTAMP_CHARS));
    provider
}

fn sanitize_external_provider_id(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut last_was_sep = false;
    for ch in raw.trim().chars().flat_map(char::to_lowercase) {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch)
        } else if matches!(ch, '-' | '_' | ' ' | '.' | '/') {
            Some('-')
        } else {
            None
        };
        let Some(ch) = mapped else {
            continue;
        };
        if ch == '-' {
            if last_was_sep {
                continue;
            }
            last_was_sep = true;
        } else {
            last_was_sep = false;
        }
        out.push(ch);
        if out.len() >= MAX_EXTERNAL_PROVIDER_ID_CHARS {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn unique_external_provider_id(base: String, used_ids: &mut HashSet<String>) -> String {
    if used_ids.insert(base.clone()) {
        return base;
    }
    let prefix = base
        .chars()
        .take(MAX_EXTERNAL_PROVIDER_ID_CHARS.saturating_sub(4))
        .collect::<String>();
    for suffix in 2..=999usize {
        let candidate = format!("{prefix}-{suffix}");
        if used_ids.insert(candidate.clone()) {
            return candidate;
        }
    }
    let fallback = format!("provider-{}", used_ids.len() + 1);
    used_ids.insert(fallback.clone());
    fallback
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

// ── Global Memory Extract Config ────────────────────────────────

/// Global auto-extract configuration, stored in config.json `memoryExtract` field.
/// Per-agent MemoryConfig can override these with Some(...) values.
///
/// Trigger logic (since last extraction):
/// - Cooldown: elapsed time must >= `extract_time_threshold_secs` (prevents too-frequent extraction)
/// - Trigger: token count >= `extract_token_threshold` OR message count >= `extract_message_threshold`
/// Both cooldown AND trigger must be satisfied.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExtractConfig {
    /// Global long-term memory runtime switch. When false, the agent plane must
    /// not inject, recall, auto-extract, flush, or write persistent memory.
    /// Owner-plane management APIs remain available so users can inspect,
    /// export, delete, or re-enable their existing data.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    #[serde(default = "crate::default_true")]
    pub auto_extract: bool,
    /// Deprecated — superseded by `modelOverride`. Kept for backward
    /// compatibility: still read when `modelOverride` is unset, but the GUI
    /// no longer writes these two fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_model_id: Option<String>,
    /// Model override for auto-extraction. `None` = fall through to the
    /// deprecated `extractProviderId`/`extractModelId` pair (if both set) →
    /// the current session's own model. Per-agent `memory.extractProviderId`/
    /// `extractModelId` (`AgentModelConfig`) still takes precedence over all
    /// of this when set — unchanged, not part of this reshape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<crate::provider::ActiveModel>,
    /// Auto-extract memories before context compaction (Tier 3 summarization)
    #[serde(default = "crate::default_true")]
    pub flush_before_compact: bool,
    /// Token accumulation threshold — trigger extraction when tokens since last extraction >= this (default: 8000)
    #[serde(default = "default_extract_token_threshold")]
    pub extract_token_threshold: usize,
    /// Cooldown in seconds — extraction won't trigger until this much time has passed (default: 300 = 5 min)
    #[serde(default = "default_extract_time_threshold_secs")]
    pub extract_time_threshold_secs: u64,
    /// Message count threshold — trigger extraction when messages since last extraction >= this (default: 10)
    #[serde(default = "default_extract_message_threshold")]
    pub extract_message_threshold: usize,
    /// Idle timeout in seconds — trigger final extraction when session is idle for this long (default: 1800 = 30 min). 0 = disabled.
    #[serde(default = "default_extract_idle_timeout_secs")]
    pub extract_idle_timeout_secs: u64,
    /// Phase B'2 — enable reflective extraction alongside factual extraction.
    /// When true, each auto-extract pass asks the model to surface user
    /// profile traits (communication style, work habits) and tags them
    /// `profile` so they render in the `## User Profile` system-prompt section.
    /// Default: true. Runs in the same side_query roundtrip as `facts`.
    #[serde(default = "crate::default_true")]
    pub enable_reflection: bool,
    /// Next-gen Dreaming (beta) — also extract structured `memory_claims` +
    /// `memory_evidence` alongside the legacy `facts`/`profile`, and dual-write
    /// each claim's shadow into `memories` (linked via `memory_claim_links`).
    /// Runs in the SAME side_query roundtrip as `facts` (a third `claims`
    /// array). Default ON — the structured claim layer ships enabled so the
    /// out-of-box experience builds claims / profile without manual opt-in;
    /// also gates the Dashboard "Claims (beta)" view.
    #[serde(default = "crate::default_true")]
    pub extract_claims: bool,
    /// Review-first learning mode. When enabled, auto-extracted structured
    /// claims are written as `needs_review` first; their managed legacy shadows
    /// stay hidden until the user approves the claim. Manual `save_memory`,
    /// claim corrections, backfill, and restore flows keep their own policies.
    #[serde(default)]
    pub review_first: bool,
}
fn default_extract_token_threshold() -> usize {
    8000
}
fn default_extract_time_threshold_secs() -> u64 {
    300
}
fn default_extract_message_threshold() -> usize {
    10
}
fn default_extract_idle_timeout_secs() -> u64 {
    1800
}

impl Default for MemoryExtractConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_extract: true,
            extract_provider_id: None,
            extract_model_id: None,
            model_override: None,
            flush_before_compact: true,
            extract_token_threshold: default_extract_token_threshold(),
            extract_time_threshold_secs: default_extract_time_threshold_secs(),
            extract_message_threshold: default_extract_message_threshold(),
            extract_idle_timeout_secs: default_extract_idle_timeout_secs(),
            enable_reflection: true,
            extract_claims: true,
            review_first: false,
        }
    }
}

// ── LLM Memory Selection ──────────────────────────────────────

/// LLM-based memory selection configuration, stored in config.json `memorySelection` field.
/// When enabled, uses side_query() to select the most relevant memories for the
/// current user message, reducing system prompt noise from irrelevant entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySelectionConfig {
    /// Enable LLM-based memory selection (default: false, opt-in)
    #[serde(default)]
    pub enabled: bool,
    /// Minimum candidate count before LLM selection kicks in (default: 8)
    #[serde(default = "default_selection_threshold")]
    pub threshold: usize,
    /// Maximum memories to select (default: 5)
    #[serde(default = "default_selection_max")]
    pub max_selected: usize,
}

fn default_selection_threshold() -> usize {
    8
}
fn default_selection_max() -> usize {
    5
}

impl Default for MemorySelectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: 8,
            max_selected: 5,
        }
    }
}

// ── Memory Section Budget ───────────────────────────────────────

/// Per-section character budgets for the SQLite Layer 3 block of the
/// system-prompt memory section.
///
/// Default allocation is 15/20/20/30/15 = 10_000 chars total, matching the
/// default `MemoryBudgetConfig::total_chars`. `scaled_to` proportionally
/// shrinks the five sections when the caller-provided cap is smaller than
/// the sum (e.g., when Layer 1/2 consumed most of the total budget).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct SqliteSectionBudgets {
    /// Load-bearing: serde alias keeps pre-rename `config.json` / agent
    /// overrides (`"aboutYou": ...`) deserialising into this field. Do not
    /// remove until the back-compat window is officially closed.
    #[serde(alias = "aboutYou")]
    pub user_profile: usize,
    pub about_user: usize,
    pub preferences: usize,
    pub project_context: usize,
    pub references: usize,
}

impl Default for SqliteSectionBudgets {
    fn default() -> Self {
        Self {
            user_profile: 1500,
            about_user: 2000,
            preferences: 2000,
            project_context: 3000,
            references: 1500,
        }
    }
}

impl SqliteSectionBudgets {
    pub fn total(&self) -> usize {
        self.user_profile
            + self.about_user
            + self.preferences
            + self.project_context
            + self.references
    }

    /// Return a copy whose five sections fit inside `cap`, proportionally
    /// scaled from the configured values when they sum to more than `cap`.
    /// Returns a zeroed-out struct when `cap == 0`.
    pub fn scaled_to(&self, cap: usize) -> Self {
        let t = self.total();
        if t == 0 || cap == 0 {
            return Self {
                user_profile: 0,
                about_user: 0,
                preferences: 0,
                project_context: 0,
                references: 0,
            };
        }
        if t <= cap {
            return self.clone();
        }
        let ratio = cap as f64 / t as f64;
        Self {
            user_profile: (self.user_profile as f64 * ratio) as usize,
            about_user: (self.about_user as f64 * ratio) as usize,
            preferences: (self.preferences as f64 * ratio) as usize,
            project_context: (self.project_context as f64 * ratio) as usize,
            references: (self.references as f64 * ratio) as usize,
        }
    }
}

/// Controls how much of the system prompt the memory section is allowed to
/// consume. Defaults tuned for a ~10K char / ~2.5K token memory block that
/// hard-prioritises the Memory Guidelines epilogue and degrades gracefully
/// (Agent core memory > Global core memory > SQLite summary) when the total
/// budget is tighter than the individual per-layer caps.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryBudgetConfig {
    /// Hard upper bound on the entire memory section (Layer 1 + 2 + 3 + 4).
    pub total_chars: usize,
    /// Upper bound on each legacy `MEMORY.md` layer (Global / Agent) individually.
    /// Actual injection is `min(core_memory_file_chars, remaining_total)`.
    pub core_memory_file_chars: usize,
    /// Per-entry truncation for rendered SQLite memory bullets (Layer 3).
    pub sqlite_entry_max_chars: usize,
    /// Per-section sub-budgets for the SQLite block.
    pub sqlite_sections: SqliteSectionBudgets,
}

impl Default for MemoryBudgetConfig {
    fn default() -> Self {
        Self {
            total_chars: 10_000,
            core_memory_file_chars: 8_000,
            sqlite_entry_max_chars: 500,
            sqlite_sections: SqliteSectionBudgets::default(),
        }
    }
}

// ── Deduplication ───────────────────────────────────────────────

/// Default dedup thresholds (RRF scores)
pub const DEDUP_THRESHOLD_HIGH: f32 = 0.02; // Above this → duplicate, skip
pub const DEDUP_THRESHOLD_MERGE: f32 = 0.012; // Between merge..high → update existing

/// Configurable dedup thresholds, stored in config.json `dedup` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DedupConfig {
    #[serde(default = "default_dedup_high")]
    pub threshold_high: f32,
    #[serde(default = "default_dedup_merge")]
    pub threshold_merge: f32,
}

fn default_dedup_high() -> f32 {
    DEDUP_THRESHOLD_HIGH
}
fn default_dedup_merge() -> f32 {
    DEDUP_THRESHOLD_MERGE
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            threshold_high: DEDUP_THRESHOLD_HIGH,
            threshold_merge: DEDUP_THRESHOLD_MERGE,
        }
    }
}

// ── Hybrid Search Config ───────────────────────────────────────

/// Configurable hybrid search weights, stored in config.json `hybridSearch` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchConfig {
    /// Weight for vector similarity results (0.0-1.0)
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f32,
    /// Weight for FTS keyword results (0.0-1.0)
    #[serde(default = "default_text_weight")]
    pub text_weight: f32,
    /// RRF constant k (higher = more equal weighting across ranks)
    #[serde(default = "default_rrf_k")]
    pub rrf_k: f64,
}

fn default_vector_weight() -> f32 {
    0.6
}
fn default_text_weight() -> f32 {
    0.4
}
fn default_rrf_k() -> f64 {
    60.0
}

impl Default for HybridSearchConfig {
    fn default() -> Self {
        Self {
            vector_weight: 0.6,
            text_weight: 0.4,
            rrf_k: 60.0,
        }
    }
}

// ── Temporal Decay Config ──────────────────────────────────────

/// Temporal decay configuration for memory search scoring.
/// Recent memories rank higher; pinned memories are exempt (evergreen).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemporalDecayConfig {
    /// Enable temporal decay (default: false)
    #[serde(default)]
    pub enabled: bool,
    /// Half-life in days: after this many days, score is halved (default: 30)
    #[serde(default = "default_half_life_days")]
    pub half_life_days: f64,
}

fn default_half_life_days() -> f64 {
    30.0
}

impl Default for TemporalDecayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            half_life_days: 30.0,
        }
    }
}

// ── MMR Config ─────────────────────────────────────────────────

/// MMR (Maximal Marginal Relevance) reranking config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MmrConfig {
    /// Enable MMR reranking (default: true)
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    /// Lambda: 0 = max diversity, 1 = max relevance (default: 0.7)
    #[serde(default = "default_mmr_lambda")]
    pub lambda: f32,
}

fn default_mmr_lambda() -> f32 {
    0.7
}

impl Default for MmrConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            lambda: 0.7,
        }
    }
}

// ── Embedding Cache Config ─────────────────────────────────────

/// Configuration for caching computed embeddings to reduce API calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingCacheConfig {
    /// Enable embedding cache (default: true)
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    /// Maximum number of cached entries (default: 10000)
    #[serde(default = "default_max_cache_entries")]
    pub max_entries: usize,
}

fn default_max_cache_entries() -> usize {
    10000
}

impl Default for EmbeddingCacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 10000,
        }
    }
}

// ── Multimodal Config ──────────────────────────────────────────

/// Multimodal embedding configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultimodalConfig {
    /// Enable multimodal embedding (default: false)
    #[serde(default)]
    pub enabled: bool,
    /// Supported modalities: "image", "audio"
    #[serde(default = "default_modalities")]
    pub modalities: Vec<String>,
    /// Max file size in bytes (default: 10MB)
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: u64,
}

fn default_modalities() -> Vec<String> {
    vec!["image".to_string(), "audio".to_string()]
}
fn default_max_file_bytes() -> u64 {
    10 * 1024 * 1024
} // 10MB

impl Default for MultimodalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            modalities: default_modalities(),
            max_file_bytes: default_max_file_bytes(),
        }
    }
}
