//! Configuration for behavior awareness.
//!
//! Two layers:
//! - Global defaults live in `AppConfig.awareness` (root `config.json`).
//! - Per-session overrides live in `sessions.awareness_config_json` column.
//!   Overrides are a partial document; unset fields inherit from global.

// 类型已下沉 ha-config-schema，原地再导出保持路径不变；merge/resolve 行为留此。
pub use ha_config_schema::awareness::{AwarenessConfig, AwarenessMode, LlmExtractionConfig};

// ── Resolver ────────────────────────────────────────────────────

/// Merge the global awareness config with the optional session-level
/// override. If the override JSON is present, any explicit fields take
/// precedence; absent fields inherit from global.
///
/// When the global `enabled` flag is `false`, the session-level override is
/// ignored entirely — global is a hard kill-switch.
pub fn resolve_for_session(
    session_id: &str,
    session_db: &crate::session::SessionDB,
) -> AwarenessConfig {
    let global = crate::config::cached_config().awareness.clone();
    if !global.enabled {
        return AwarenessConfig {
            enabled: false,
            ..global
        };
    }

    let override_json = match session_db.get_session_awareness_config_json(session_id) {
        Ok(Some(s)) if !s.trim().is_empty() => s,
        _ => return global,
    };

    match merge_override(&global, &override_json) {
        Ok(cfg) => cfg,
        Err(e) => {
            app_warn!(
                "awareness",
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
/// `AwarenessConfig`. Called from the Tauri/HTTP command layer before
/// persisting to the DB.
pub fn validate_override(base: &AwarenessConfig, override_json: &str) -> anyhow::Result<()> {
    merge_override(base, override_json).map(|_| ())
}

/// Parse a partial override JSON and apply it on top of the base config.
fn merge_override(base: &AwarenessConfig, override_json: &str) -> anyhow::Result<AwarenessConfig> {
    let override_val: serde_json::Value = serde_json::from_str(override_json)?;
    let mut base_val = serde_json::to_value(base)?;
    crate::merge_json(&mut base_val, override_val);
    let merged: AwarenessConfig = serde_json::from_value(base_val)?;
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_structured() {
        let cfg = AwarenessConfig::default();
        assert_eq!(cfg.mode, AwarenessMode::Structured);
        assert!(!cfg.enabled);
        assert!(cfg.exclude_cron);
        assert!(cfg.exclude_channel);
        assert!(cfg.exclude_subagents);
    }

    #[test]
    fn partial_override_merges_into_base() {
        let base = AwarenessConfig::default();
        let override_json = r#"{"maxSessions": 2, "excludeCron": false}"#;
        let merged = merge_override(&base, override_json).unwrap();
        assert_eq!(merged.max_sessions, 2);
        assert!(!merged.exclude_cron);
        assert!(merged.exclude_channel); // unchanged
        assert_eq!(merged.mode, AwarenessMode::Structured);
    }

    #[test]
    fn override_can_switch_mode() {
        let base = AwarenessConfig::default();
        let override_json = r#"{"mode": "llm_digest"}"#;
        let merged = merge_override(&base, override_json).unwrap();
        assert_eq!(merged.mode, AwarenessMode::LlmDigest);
    }

    #[test]
    fn bad_override_json_is_a_hard_error() {
        let base = AwarenessConfig::default();
        assert!(merge_override(&base, "not json").is_err());
    }
}
