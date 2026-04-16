//! Configuration for cross-session behavior awareness.
//!
//! Two layers:
//! - Global defaults live in `AppConfig.cross_session` (root `config.json`).
//! - Per-session overrides live in `sessions.cross_session_config_json` column.
//!   Overrides are a partial document; unset fields inherit from global.

use serde::{Deserialize, Serialize};

// ── Mode enum ────────────────────────────────────────────────────

/// How the cross-session suffix is produced.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CrossSessionMode {
    /// Feature entirely disabled.
    Off,
    /// Zero LLM cost. Reads structured data and renders a markdown list.
    #[default]
    Structured,
    /// Structured list + an LLM-generated behavior digest. Costs extra API calls.
    LlmDigest,
}

// ── Extraction config (LlmDigest mode only) ─────────────────────

/// Reference to a specific provider + model for LLM extraction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionModelRef {
    pub provider_id: String,
    pub model: String,
}

/// LLM extraction tuning knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct LlmExtractionConfig {
    /// Agent to run the extraction under. None → fall back to `recap.analysisAgent`.
    pub extraction_agent: Option<String>,
    /// Explicit provider+model override. Wins over `extraction_agent`.
    pub extraction_model: Option<ExtractionModelRef>,
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
            extraction_agent: None,
            extraction_model: None,
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

/// Root cross-session config. Stored under `AppConfig.crossSession` and
/// per-session `sessions.cross_session_config_json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CrossSessionConfig {
    /// Master on/off switch. When false, no suffix is ever produced.
    pub enabled: bool,
    /// What the suffix contains.
    pub mode: CrossSessionMode,

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

impl Default for CrossSessionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: CrossSessionMode::Structured,
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

// ── Resolver ────────────────────────────────────────────────────

/// Merge the global cross-session config with the optional session-level
/// override. If the override JSON is present, any explicit fields take
/// precedence; absent fields inherit from global.
///
/// When the global `enabled` flag is `false`, the session-level override is
/// ignored entirely — global is a hard kill-switch.
pub fn resolve_for_session(
    session_id: &str,
    session_db: &crate::session::SessionDB,
) -> CrossSessionConfig {
    let global = crate::config::cached_config().cross_session.clone();
    if !global.enabled {
        return CrossSessionConfig {
            enabled: false,
            ..global
        };
    }

    let override_json = match session_db.get_session_cross_session_config_json(session_id) {
        Ok(Some(s)) if !s.trim().is_empty() => s,
        _ => return global,
    };

    match merge_override(&global, &override_json) {
        Ok(cfg) => cfg,
        Err(e) => {
            app_warn!(
                "cross_session",
                "config::resolve_for_session",
                "Failed to parse session override for {}: {} — falling back to global",
                session_id,
                e
            );
            global
        }
    }
}

/// Validate that `override_json` is legal JSON that can be merged into a
/// `CrossSessionConfig`. Called from the Tauri/HTTP command layer before
/// persisting to the DB.
pub fn validate_override(base: &CrossSessionConfig, override_json: &str) -> anyhow::Result<()> {
    merge_override(base, override_json).map(|_| ())
}

/// Parse a partial override JSON and apply it on top of the base config.
fn merge_override(base: &CrossSessionConfig, override_json: &str) -> anyhow::Result<CrossSessionConfig> {
    let override_val: serde_json::Value = serde_json::from_str(override_json)?;
    let mut base_val = serde_json::to_value(base)?;
    merge_json(&mut base_val, override_val);
    let merged: CrossSessionConfig = serde_json::from_value(base_val)?;
    Ok(merged)
}

fn merge_json(dst: &mut serde_json::Value, src: serde_json::Value) {
    match (dst, src) {
        (serde_json::Value::Object(dst_map), serde_json::Value::Object(src_map)) => {
            for (k, v) in src_map {
                match dst_map.get_mut(&k) {
                    Some(existing) => merge_json(existing, v),
                    None => {
                        dst_map.insert(k, v);
                    }
                }
            }
        }
        (dst_slot, src_val) => {
            *dst_slot = src_val;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_structured() {
        let cfg = CrossSessionConfig::default();
        assert_eq!(cfg.mode, CrossSessionMode::Structured);
        assert!(cfg.enabled);
        assert!(cfg.exclude_cron);
        assert!(cfg.exclude_channel);
        assert!(cfg.exclude_subagents);
    }

    #[test]
    fn partial_override_merges_into_base() {
        let base = CrossSessionConfig::default();
        let override_json = r#"{"maxSessions": 2, "excludeCron": false}"#;
        let merged = merge_override(&base, override_json).unwrap();
        assert_eq!(merged.max_sessions, 2);
        assert!(!merged.exclude_cron);
        assert!(merged.exclude_channel); // unchanged
        assert_eq!(merged.mode, CrossSessionMode::Structured);
    }

    #[test]
    fn override_can_switch_mode() {
        let base = CrossSessionConfig::default();
        let override_json = r#"{"mode": "llm_digest"}"#;
        let merged = merge_override(&base, override_json).unwrap();
        assert_eq!(merged.mode, CrossSessionMode::LlmDigest);
    }

    #[test]
    fn bad_override_json_is_a_hard_error() {
        let base = CrossSessionConfig::default();
        assert!(merge_override(&base, "not json").is_err());
    }
}
