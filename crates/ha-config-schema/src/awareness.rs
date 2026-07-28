//! Configuration for behavior awareness.
//!
//! Two layers:
//! - Global defaults live in `AppConfig.awareness` (root `config.json`).
//! - Per-session overrides live in `sessions.awareness_config_json` column.
//!   Overrides are a partial document; unset fields inherit from global.
//!
//! 本文件只有 wire 类型；merge/resolve 行为逻辑在 ha-core `awareness/config.rs`。

use serde::{Deserialize, Serialize};

// ── Mode enum ────────────────────────────────────────────────────

/// How the awareness suffix is produced.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AwarenessMode {
    /// Feature entirely disabled.
    Off,
    /// Zero LLM cost. Reads structured data and renders a markdown list.
    #[default]
    Structured,
    /// Structured list + an LLM-generated behavior digest. Costs extra API calls.
    LlmDigest,
}

// ── Extraction config (LlmDigest mode only) ─────────────────────

/// LLM extraction tuning knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct LlmExtractionConfig {
    /// Model override for the extraction side_query. `None` (the default —
    /// what every existing config has, since this field is new) keeps the
    /// current, cache-friendly behavior: extraction reuses the *current*
    /// chat agent's cache prefix via `self.side_query(...)`. Setting this
    /// switches to a dedicated model via ha-core `automation::run`, which is
    /// correct but gives up that cache-sharing — an explicit trade a user
    /// opts into, not a free lunch.
    ///
    /// Replaces the former `extractionAgent: Option<String>` /
    /// `extractionModel: Option<ExtractionModelRef>` pair: `extractionAgent`
    /// was read but never actually switched the agent (a no-op that only
    /// logged a warning), and `extractionModel` had no reader at all — both
    /// dead configuration, not preserved.
    pub model_override: Option<crate::provider::ModelChain>,
    /// Minimum seconds between two real LLM extractions on the same session.
    pub min_interval_secs: u64,
    /// Max number of candidate sessions to feed the extractor.
    pub max_candidates: usize,
    /// Max character budget of the output digest.
    pub digest_max_chars: usize,
    /// Semaphore size — global concurrent extraction limit.
    pub concurrency: usize,
    /// Max characters per candidate session fed into the extractor.
    pub per_session_input_chars: usize,
    /// Messages older than this many hours are not sent to the LLM.
    pub input_lookback_hours: i64,
    /// On failure, silently fall back to Structured and cool down.
    pub fallback_on_error: bool,
    /// Reuse side_query cache prefix (recommended).
    pub reuse_side_query_cache: bool,
}

impl Default for LlmExtractionConfig {
    fn default() -> Self {
        Self {
            model_override: None,
            min_interval_secs: 300,
            max_candidates: 5,
            digest_max_chars: 1200,
            concurrency: 2,
            per_session_input_chars: 2000,
            input_lookback_hours: 4,
            fallback_on_error: true,
            reuse_side_query_cache: true,
        }
    }
}

// ── Main config ─────────────────────────────────────────────────

fn default_semantic_hint_regex() -> String {
    "(?i)(上次|之前|之前那个|另一个|其它会话|其他会话|另一边|另一个窗口|另一个对话|last time|previously|earlier|another session|other session|the other (chat|session|window))"
        .to_string()
}

/// Root awareness config. Stored under `AppConfig.awareness` and
/// per-session `sessions.awareness_config_json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AwarenessConfig {
    /// Master on/off switch. When false, no suffix is ever produced.
    pub enabled: bool,
    /// What the suffix contains.
    pub mode: AwarenessMode,

    // ── Candidate scoping ──
    pub max_sessions: usize,
    pub max_chars: usize,
    pub lookback_hours: i64,
    pub active_window_secs: u64,
    pub same_agent_only: bool,
    pub exclude_cron: bool,
    pub exclude_channel: bool,
    pub exclude_subagents: bool,
    pub preview_chars: usize,

    // ── Dynamic refresh ──
    pub dynamic_enabled: bool,
    pub min_refresh_secs: u64,
    pub semantic_hint_regex: String,
    pub refresh_on_compaction: bool,

    // ── LLM extraction ──
    pub llm_extraction: LlmExtractionConfig,
}

impl Default for AwarenessConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: AwarenessMode::Structured,
            max_sessions: 6,
            max_chars: 4000,
            lookback_hours: 72,
            active_window_secs: 120,
            same_agent_only: false,
            // Conservative default: only regular sessions. User can opt-in to the rest.
            exclude_cron: true,
            exclude_channel: true,
            exclude_subagents: true,
            preview_chars: 200,
            dynamic_enabled: true,
            min_refresh_secs: 20,
            semantic_hint_regex: default_semantic_hint_regex(),
            refresh_on_compaction: true,
            llm_extraction: LlmExtractionConfig::default(),
        }
    }
}
