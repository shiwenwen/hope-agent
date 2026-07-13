//! Memory UX v2 product-level runtime configuration.
//!
//! The legacy memory settings remain deserializable while the V2 rollout is in
//! progress.  This module owns the new user-facing contract: using memory,
//! fast recall, deep recall, learning, bounded Core Memory and compatibility
//! switches are independent decisions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryRuntimeConfig {
    pub enabled: bool,
    pub core: CoreMemoryRuntimeConfig,
    pub recall: MemoryRecallRuntimeConfig,
    pub deep_recall: DeepRecallRuntimeConfig,
    pub learning: MemoryLearningRuntimeConfig,
    pub rollout: MemoryUxV2RolloutConfig,
    pub compatibility: MemoryCompatibilityConfig,
}

impl Default for MemoryRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            core: CoreMemoryRuntimeConfig::default(),
            recall: MemoryRecallRuntimeConfig::default(),
            deep_recall: DeepRecallRuntimeConfig::default(),
            learning: MemoryLearningRuntimeConfig::default(),
            rollout: MemoryUxV2RolloutConfig::default(),
            compatibility: MemoryCompatibilityConfig::default(),
        }
    }
}

impl MemoryRuntimeConfig {
    /// Legacy static injection stays on until the V2 runtime itself is active,
    /// then follows the explicit rollback switch.
    pub(crate) fn legacy_static_injection_enabled(&self) -> bool {
        !self.rollout.enabled || self.compatibility.legacy_static_memory
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct CoreMemoryRuntimeConfig {
    pub enabled: bool,
    pub total_tokens: u32,
    pub hard_max_tokens: u32,
    pub global_tokens: u32,
    pub agent_tokens: u32,
    pub project_tokens: u32,
    pub protocol_tokens: u32,
    pub topic_read_max_tokens: u32,
}

impl Default for CoreMemoryRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            total_tokens: 1_600,
            hard_max_tokens: 2_400,
            global_tokens: 350,
            agent_tokens: 450,
            project_tokens: 650,
            protocol_tokens: 150,
            topic_read_max_tokens: 800,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryRecallRuntimeConfig {
    pub enabled: bool,
    pub mode: MemoryRecallMode,
    pub max_tokens: u32,
    pub max_selected: usize,
    pub candidate_limit: usize,
    pub timeout_ms: u64,
    pub include_claims: bool,
    pub include_profile: bool,
    pub include_procedures: bool,
    pub include_graph: bool,
}

impl Default for MemoryRecallRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: MemoryRecallMode::Fast,
            max_tokens: 800,
            max_selected: 5,
            candidate_limit: 24,
            timeout_ms: 100,
            include_claims: true,
            include_profile: true,
            include_procedures: true,
            include_graph: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRecallMode {
    #[default]
    Fast,
    Deep,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct DeepRecallRuntimeConfig {
    pub enabled: bool,
    pub timeout_ms: u64,
    pub cache_ttl_secs: u64,
    pub max_chars: usize,
}

impl Default for DeepRecallRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_ms: 4_500,
            cache_ttl_secs: 60,
            max_chars: 220,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryLearningRuntimeConfig {
    pub mode: MemoryLearningMode,
    pub promote_core_automatically: bool,
}

impl Default for MemoryLearningRuntimeConfig {
    fn default() -> Self {
        Self {
            mode: MemoryLearningMode::Smart,
            promote_core_automatically: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLearningMode {
    #[default]
    Smart,
    ReviewFirst,
    Manual,
}

/// Staged rollout switches. They default off until their corresponding V2
/// path has parity fixtures and a legacy rollback path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryUxV2RolloutConfig {
    pub enabled: bool,
    pub dynamic_recall: bool,
    pub core_repository: bool,
    pub shadow_plan: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryCompatibilityConfig {
    /// Preserve the old SQLite/Profile/Pinned static prompt injection while
    /// V2 is shadowed or when the user explicitly rolls back.
    pub legacy_static_memory: bool,
}

impl Default for MemoryCompatibilityConfig {
    fn default() -> Self {
        Self {
            legacy_static_memory: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_keep_v2_shadowed_and_legacy_reversible() {
        let config = MemoryRuntimeConfig::default();
        assert!(config.enabled);
        assert!(!config.rollout.enabled);
        assert!(!config.rollout.dynamic_recall);
        assert!(!config.rollout.core_repository);
        assert!(config.compatibility.legacy_static_memory);
        assert!(!config.deep_recall.enabled);
        assert_eq!(config.core.total_tokens, 1_600);
        assert_eq!(config.recall.max_tokens, 800);
        assert!(config.legacy_static_injection_enabled());
    }

    #[test]
    fn missing_nested_fields_deserialize_to_safe_defaults() {
        let parsed: MemoryRuntimeConfig = serde_json::from_value(serde_json::json!({
            "rollout": { "shadowPlan": true },
            "recall": { "maxSelected": 3 }
        }))
        .unwrap();
        assert!(parsed.rollout.shadow_plan);
        assert!(!parsed.rollout.enabled);
        assert_eq!(parsed.recall.max_selected, 3);
        assert!(parsed.recall.enabled);
        assert!(parsed.compatibility.legacy_static_memory);
    }

    #[test]
    fn legacy_static_injection_can_only_turn_off_after_v2_is_active() {
        let mut config = MemoryRuntimeConfig::default();
        config.compatibility.legacy_static_memory = false;
        assert!(config.legacy_static_injection_enabled());
        config.rollout.enabled = true;
        assert!(!config.legacy_static_injection_enabled());
        config.compatibility.legacy_static_memory = true;
        assert!(config.legacy_static_injection_enabled());
    }
}
