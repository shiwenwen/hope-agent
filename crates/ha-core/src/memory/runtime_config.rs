//! Memory UX v2 product-level runtime configuration.
//!
//! The legacy memory settings remain deserializable while the V2 rollout is in
//! progress.  This module owns the new user-facing contract: using memory,
//! opt-in automatic recall, deep recall, learning, bounded Core Memory and compatibility
//! switches are independent decisions.

use serde::{Deserialize, Serialize};

// 类型已下沉 ha-config-schema：wire 类型与自包含迁移 / 归一化逻辑在
// `ha_config_schema::memory::runtime_config`；运行期解析
// （`CoreMemoryBudgetStatus` / session gating helpers）留在本文件。
pub use ha_config_schema::memory::{
    CoreMemoryRuntimeConfig, DeepRecallRuntimeConfig, MemoryCompatibilityConfig,
    MemoryLearningMode, MemoryLearningRuntimeConfig, MemoryRecallMode, MemoryRecallRuntimeConfig,
    MemoryRuntimeConfig, MemoryUxV2RolloutConfig, CORE_MEMORY_EMERGENCY_MAX_TOKENS,
    CORE_MEMORY_MIN_TOKENS, CORE_MEMORY_RECOMMENDED_MAX_TOKENS, MEMORY_RUNTIME_CONFIG_VERSION,
};

const CORE_MEMORY_CONTEXT_SHARE_DIVISOR: u32 = 10;
const CORE_MEMORY_MIN_MODEL_CAP_TOKENS: u32 = 256;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoreMemoryBudgetStatus {
    pub configured_tokens: u32,
    pub effective_tokens: u32,
    pub context_window_tokens: Option<u32>,
    pub model_safety_limit_tokens: Option<u32>,
    pub emergency_limit_tokens: u32,
    pub limited_by: Option<CoreMemoryBudgetLimit>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoreMemoryBudgetLimit {
    ContextWindow,
    EmergencyGuard,
}

impl CoreMemoryBudgetStatus {
    pub fn resolve(config: &CoreMemoryRuntimeConfig, context_window: Option<u32>) -> Self {
        let configured_tokens = config.total_tokens.max(CORE_MEMORY_MIN_TOKENS);
        let model_safety_limit_tokens = context_window.map(|window| {
            (window / CORE_MEMORY_CONTEXT_SHARE_DIVISOR)
                .max(CORE_MEMORY_MIN_MODEL_CAP_TOKENS)
                .min(CORE_MEMORY_EMERGENCY_MAX_TOKENS)
        });
        let after_emergency = configured_tokens.min(CORE_MEMORY_EMERGENCY_MAX_TOKENS);
        let effective_tokens =
            model_safety_limit_tokens.map_or(after_emergency, |limit| after_emergency.min(limit));
        let limited_by = if model_safety_limit_tokens
            .is_some_and(|limit| limit < configured_tokens.min(CORE_MEMORY_EMERGENCY_MAX_TOKENS))
        {
            Some(CoreMemoryBudgetLimit::ContextWindow)
        } else if configured_tokens > CORE_MEMORY_EMERGENCY_MAX_TOKENS {
            Some(CoreMemoryBudgetLimit::EmergencyGuard)
        } else {
            None
        };
        Self {
            configured_tokens,
            effective_tokens,
            context_window_tokens: context_window,
            model_safety_limit_tokens,
            emergency_limit_tokens: CORE_MEMORY_EMERGENCY_MAX_TOKENS,
            limited_by,
        }
    }
}

/// Resolve the Settings-page status against the global active model. Session
/// model overrides are reported by the per-round Memory Context Manifest and
/// `/context`; this owner view intentionally describes the global default.
pub fn active_core_memory_budget_status() -> CoreMemoryBudgetStatus {
    let app = crate::config::cached_config();
    let context_window = app.active_model.as_ref().and_then(|active| {
        crate::provider::model_context_window(&app.providers, &active.provider_id, &active.model_id)
    });
    CoreMemoryBudgetStatus::resolve(&app.memory.core, context_window)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EffectiveSessionMemoryAccess {
    pub use_memories: bool,
    pub contribute_to_memories: bool,
}

pub(crate) fn effective_session_memory_access(
    session_id: Option<&str>,
    bound_db: Option<&crate::session::SessionDB>,
) -> EffectiveSessionMemoryAccess {
    let Some(session_id) = session_id else {
        return EffectiveSessionMemoryAccess {
            use_memories: true,
            contribute_to_memories: true,
        };
    };
    let global_db = crate::get_session_db();
    let Some(db) = bound_db.or_else(|| global_db.map(|db| db.as_ref())) else {
        return EffectiveSessionMemoryAccess {
            use_memories: false,
            contribute_to_memories: false,
        };
    };
    let Ok(Some(session)) = db.get_session(session_id) else {
        return EffectiveSessionMemoryAccess {
            use_memories: false,
            contribute_to_memories: false,
        };
    };
    if session.incognito {
        return EffectiveSessionMemoryAccess {
            use_memories: false,
            contribute_to_memories: false,
        };
    }
    let Ok(policy) = db.get_memory_policy(session_id) else {
        return EffectiveSessionMemoryAccess {
            use_memories: false,
            contribute_to_memories: false,
        };
    };
    EffectiveSessionMemoryAccess {
        use_memories: policy.use_memories.allows(),
        contribute_to_memories: policy.contribute_to_memories.allows(),
    }
}

/// Kernel-owned verdict consumed by memory feature runtimes. It deliberately
/// accepts only a typed `SessionDB` reference, never a raw connection.
pub fn automatic_memory_learning_allowed(
    session_id: Option<&str>,
    bound_db: Option<&crate::session::SessionDB>,
) -> bool {
    let app = crate::config::cached_config();
    let globally_enabled = if app.memory.rollout.enabled {
        app.memory.enabled && !matches!(app.memory.learning.mode, MemoryLearningMode::Manual)
    } else {
        // Legacy `auto_extract` and `flush_before_compact` are independent
        // triggers. This helper is the shared master/session gate; each caller
        // applies its own trigger so rollback preserves combinations such as
        // auto-extract off + pre-compaction flush on.
        app.memory_extract.enabled
    };
    let agent_enabled = session_id.is_none_or(|session_id| {
        // A bound DB is authoritative for isolated chat-engine/eval/server
        // contexts. Falling back to the process-global store here could read
        // another session with the same id and disagree with the contribution
        // policy check below.
        let meta = if let Some(db) = bound_db {
            db.get_session(session_id).ok().flatten()
        } else {
            crate::session::lookup_session_meta(Some(session_id))
        };
        let Some(meta) = meta else {
            return false;
        };
        crate::agent_loader::load_agent(&meta.agent_id)
            .map(|definition| definition.config.memory.enabled)
            .unwrap_or(false)
    });
    globally_enabled
        && agent_enabled
        && effective_session_memory_access(session_id, bound_db).contribute_to_memories
}

/// Whether durable material attributed to a session may feed secondary
/// learning products such as Dreaming consolidation or Profile synthesis.
/// Missing/deleted sessions fail closed; source-less/manual owner records are
/// handled by callers and remain eligible.
pub fn session_contribution_source_allowed(session_id: &str) -> bool {
    if session_id.trim().is_empty() {
        return false;
    }
    effective_session_memory_access(Some(session_id), None).contribute_to_memories
}

/// Current owner-configured learning mode for feature-side persistence.
pub fn review_first_learning_enabled() -> bool {
    let app = crate::config::cached_config();
    if app.memory.rollout.enabled {
        matches!(app.memory.learning.mode, MemoryLearningMode::ReviewFirst)
    } else {
        app.memory_extract.review_first
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_enable_core_without_automatic_dynamic_recall() {
        let config = MemoryRuntimeConfig::default();
        assert_eq!(config.config_version, MEMORY_RUNTIME_CONFIG_VERSION);
        assert!(config.enabled);
        assert!(config.rollout.enabled);
        assert!(config.rollout.dynamic_recall);
        assert!(config.rollout.core_repository);
        assert!(config.core.enabled);
        assert!(!config.compatibility.legacy_static_memory);
        assert!(!config.deep_recall.enabled);
        assert!(!config.recall.enabled);
        assert!(!config.recall.user_configured);
        assert_eq!(config.core.total_tokens, 1_600);
        assert_eq!(config.recall.max_tokens, 800);
        assert!(!config.legacy_static_injection_enabled());
        assert!(config.effective_enabled(false));
        assert!(config.unified_dynamic_recall_enabled());
    }

    #[test]
    fn dynamic_recall_rollout_switch_restores_legacy_sources() {
        let mut config = MemoryRuntimeConfig::default();
        config.rollout.dynamic_recall = false;
        assert!(!config.unified_dynamic_recall_enabled());
        config.rollout.dynamic_recall = true;
        config.rollout.enabled = false;
        assert!(!config.unified_dynamic_recall_enabled());
    }

    #[test]
    fn legacy_agent_active_memory_remains_a_local_recall_opt_in() {
        let mut config = MemoryRuntimeConfig::default();
        assert!(!config.automatic_recall_enabled_for_agent(false));
        assert!(config.automatic_recall_enabled_for_agent(true));

        config.recall.enabled = true;
        assert!(config.automatic_recall_enabled_for_agent(false));
    }

    #[test]
    fn v2_and_legacy_master_switches_never_cross_control_each_other() {
        let mut config = MemoryRuntimeConfig::default();
        assert!(config.effective_enabled(false));
        config.enabled = false;
        assert!(!config.effective_enabled(true));

        config.rollout.enabled = false;
        assert!(config.effective_enabled(true));
        assert!(!config.effective_enabled(false));
    }

    #[test]
    fn compatibility_mirror_keeps_simple_and_expert_controls_coherent() {
        let mut config = MemoryRuntimeConfig {
            enabled: false,
            learning: MemoryLearningRuntimeConfig {
                mode: MemoryLearningMode::Manual,
                ..Default::default()
            },
            deep_recall: DeepRecallRuntimeConfig {
                enabled: true,
                ..Default::default()
            },
            recall: MemoryRecallRuntimeConfig {
                max_selected: 3,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut extract = super::super::MemoryExtractConfig::default();
        let mut selection = super::super::MemorySelectionConfig::default();
        let previous = MemoryRuntimeConfig::default();
        config.mirror_to_legacy(&previous, &mut extract, &mut selection);
        assert!(!extract.enabled);
        assert!(!extract.auto_extract);
        assert!(!extract.flush_before_compact);
        assert!(selection.enabled);
        assert_eq!(selection.max_selected, 3);

        let migrated = MemoryRuntimeConfig::from_legacy(
            &super::super::MemoryExtractConfig {
                auto_extract: false,
                flush_before_compact: true,
                ..Default::default()
            },
            &super::super::MemorySelectionConfig::default(),
            &super::super::MemoryBudgetConfig::default(),
        );
        extract.auto_extract = false;
        extract.flush_before_compact = true;
        let unchanged = migrated.clone();
        migrated.mirror_to_legacy(&unchanged, &mut extract, &mut selection);
        assert!(!extract.auto_extract);
        assert!(extract.flush_before_compact);

        extract.enabled = true;
        extract.review_first = true;
        config.apply_legacy_extract_controls(&extract);
        assert!(config.enabled);
        assert_eq!(config.learning.mode, MemoryLearningMode::ReviewFirst);
    }

    #[test]
    fn missing_nested_fields_deserialize_to_safe_defaults() {
        let parsed: MemoryRuntimeConfig = serde_json::from_value(serde_json::json!({
            "rollout": { "shadowPlan": true },
            "recall": { "maxSelected": 3 }
        }))
        .unwrap();
        assert!(parsed.rollout.shadow_plan);
        assert!(parsed.rollout.enabled);
        assert_eq!(parsed.recall.max_selected, 3);
        assert!(!parsed.recall.enabled);
        assert!(!parsed.compatibility.legacy_static_memory);
    }

    #[test]
    fn legacy_explicit_recall_consent_is_preserved_without_reenabling_learning() {
        let extract = super::super::MemoryExtractConfig {
            enabled: false,
            auto_extract: false,
            flush_before_compact: false,
            ..Default::default()
        };
        let selection = super::super::MemorySelectionConfig {
            enabled: true,
            max_selected: 3,
            ..Default::default()
        };
        let migrated = MemoryRuntimeConfig::from_legacy(
            &extract,
            &selection,
            &super::super::MemoryBudgetConfig::default(),
        );
        assert!(!migrated.enabled);
        assert_eq!(migrated.learning.mode, MemoryLearningMode::Manual);
        assert!(migrated.deep_recall.enabled);
        assert!(migrated.recall.enabled);
        assert!(migrated.recall.user_configured);
        assert_eq!(migrated.recall.max_selected, 3);
    }

    #[test]
    fn unversioned_preview_default_true_migrates_to_opt_in_off() {
        let raw = serde_json::json!({
            "enabled": true,
            "recall": { "enabled": true, "mode": "fast" },
            "deepRecall": { "enabled": false }
        });
        let mut config: MemoryRuntimeConfig = serde_json::from_value(raw.clone()).unwrap();

        assert!(config.migrate_recall_consent(&raw, false));
        assert_eq!(config.config_version, MEMORY_RUNTIME_CONFIG_VERSION);
        assert!(!config.recall.enabled);
        assert!(!config.recall.user_configured);
        assert!(config.automatic_recall_enabled_for_agent(true));
        assert!(config.core.enabled);
        assert_eq!(config.learning.mode, MemoryLearningMode::Smart);
    }

    #[test]
    fn unversioned_explicit_recall_evidence_is_preserved() {
        for (raw, legacy_selection_enabled) in [
            (
                serde_json::json!({
                    "recall": { "enabled": true, "mode": "fast" },
                    "deepRecall": { "enabled": true }
                }),
                false,
            ),
            (
                serde_json::json!({
                    "recall": { "enabled": true, "mode": "fast" },
                    "deepRecall": { "enabled": false }
                }),
                true,
            ),
            (
                serde_json::json!({
                    "recall": {
                        "enabled": true,
                        "mode": "fast",
                        "userConfigured": true
                    }
                }),
                false,
            ),
        ] {
            let mut config: MemoryRuntimeConfig = serde_json::from_value(raw.clone()).unwrap();
            assert!(config.migrate_recall_consent(&raw, legacy_selection_enabled));
            assert!(config.recall.enabled);
            assert!(config.recall.user_configured);
        }
    }

    #[test]
    fn unversioned_explicit_off_wins_over_legacy_deep_evidence() {
        let raw = serde_json::json!({
            "recall": { "enabled": false, "mode": "deep" },
            "deepRecall": { "enabled": true }
        });
        let mut config: MemoryRuntimeConfig = serde_json::from_value(raw.clone()).unwrap();

        assert!(config.migrate_recall_consent(&raw, true));
        assert!(!config.recall.enabled);
        assert!(!config.recall.user_configured);
    }

    #[test]
    fn current_contract_is_idempotent_and_user_save_records_only_toggle_consent() {
        let raw = serde_json::json!({
            "configVersion": MEMORY_RUNTIME_CONFIG_VERSION,
            "recall": { "enabled": true, "userConfigured": false }
        });
        let mut current: MemoryRuntimeConfig = serde_json::from_value(raw.clone()).unwrap();
        assert!(!current.migrate_recall_consent(&raw, false));
        assert!(current.recall.enabled);
        assert!(!current.recall.user_configured);

        let previous = MemoryRuntimeConfig::default();
        let mut budget_edit = previous.clone();
        budget_edit.recall.max_tokens = 600;
        let budget_edit = budget_edit.prepared_for_user_save(&previous);
        assert!(!budget_edit.recall.user_configured);

        let mut toggle = previous.clone();
        toggle.recall.enabled = true;
        let toggle = toggle.prepared_for_user_save(&previous);
        assert!(toggle.recall.enabled);
        assert!(toggle.recall.user_configured);

        let mut older_client_payload = toggle.clone();
        older_client_payload.recall.user_configured = false;
        let preserved = older_client_payload.prepared_for_user_save(&toggle);
        assert!(preserved.recall.user_configured);
    }

    #[test]
    fn legacy_static_injection_can_only_turn_off_after_v2_is_active() {
        let mut config = MemoryRuntimeConfig::default();
        config.compatibility.legacy_static_memory = false;
        assert!(!config.legacy_static_injection_enabled());
        config.compatibility.legacy_static_memory = true;
        assert!(config.legacy_static_injection_enabled());
        config.rollout.enabled = false;
        config.compatibility.legacy_static_memory = false;
        assert!(config.legacy_static_injection_enabled());
    }

    #[test]
    fn legacy_selection_replacer_requires_a_full_v1_rollback() {
        let mut config = MemoryRuntimeConfig::default();
        assert!(!config.legacy_selection_replacer_enabled());

        config.compatibility.legacy_static_memory = true;
        assert!(!config.legacy_selection_replacer_enabled());

        config.rollout.enabled = false;
        assert!(config.legacy_selection_replacer_enabled());
    }

    #[test]
    fn owner_supplied_runtime_budgets_are_normalized() {
        let mut config = MemoryRuntimeConfig::default();
        config.core.hard_max_tokens = 99_999;
        config.core.total_tokens = 99_999;
        config.core.protocol_tokens = 0;
        config.recall.max_selected = 0;
        config.recall.timeout_ms = 99_999;
        config.deep_recall.budget_tokens = 0;
        let normalized = config.normalized();
        assert_eq!(normalized.core.hard_max_tokens, 16_384);
        assert_eq!(normalized.core.total_tokens, 16_384);
        assert_eq!(normalized.core.protocol_tokens, 32);
        assert_eq!(normalized.recall.max_selected, 1);
        assert_eq!(normalized.recall.timeout_ms, 2_000);
        assert_eq!(normalized.deep_recall.budget_tokens, 64);
    }

    #[test]
    fn deprecated_hard_max_never_silently_reduces_visible_budget() {
        let mut config = MemoryRuntimeConfig::default();
        config.core.total_tokens = 8_000;
        config.core.hard_max_tokens = 2_400;

        let normalized = config.normalized();

        assert_eq!(normalized.core.total_tokens, 8_000);
        assert_eq!(normalized.core.hard_max_tokens, 8_000);
    }

    #[test]
    fn core_budget_is_capped_to_ten_percent_of_model_context() {
        let config = CoreMemoryRuntimeConfig {
            total_tokens: 8_000,
            ..Default::default()
        };

        let small = CoreMemoryBudgetStatus::resolve(&config, Some(16_000));
        assert_eq!(small.configured_tokens, 8_000);
        assert_eq!(small.effective_tokens, 1_600);
        assert_eq!(small.model_safety_limit_tokens, Some(1_600));
        assert_eq!(small.limited_by, Some(CoreMemoryBudgetLimit::ContextWindow));

        let large = CoreMemoryBudgetStatus::resolve(&config, Some(128_000));
        assert_eq!(large.effective_tokens, 8_000);
        assert_eq!(large.limited_by, None);
    }
}
