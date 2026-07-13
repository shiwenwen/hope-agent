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
    /// Resolve the product-level Memory master switch without mixing V2 and
    /// rollback semantics. Once V2 is active, legacy extraction settings must
    /// not silently disable Core, recall, tools, or Dreaming behind the GUI.
    pub(crate) fn effective_enabled(&self, legacy_extract_enabled: bool) -> bool {
        if self.rollout.enabled {
            self.enabled
        } else {
            legacy_extract_enabled
        }
    }

    /// Keep the one-minor rollback fields coherent with the simple V2 controls.
    /// Detailed legacy model and threshold settings are intentionally preserved.
    pub fn mirror_to_legacy(
        &self,
        previous: &Self,
        extract: &mut super::MemoryExtractConfig,
        selection: &mut super::MemorySelectionConfig,
    ) {
        if !self.rollout.enabled {
            return;
        }
        extract.enabled = self.enabled;
        // Legacy supports auto-extract and pre-compaction flush as independent
        // switches. Preserve an existing mixed combination (for example off +
        // on) while the V2 mode itself is unchanged; an explicit mode change
        // intentionally adopts the simple V2 preset.
        if self.learning.mode != previous.learning.mode {
            let automatic = !matches!(self.learning.mode, MemoryLearningMode::Manual);
            extract.auto_extract = automatic;
            extract.flush_before_compact = automatic;
        }
        extract.review_first = matches!(self.learning.mode, MemoryLearningMode::ReviewFirst);
        selection.enabled =
            self.deep_recall.enabled || matches!(self.recall.mode, MemoryRecallMode::Deep);
        selection.max_selected = self.recall.max_selected;
    }

    /// Compatibility direction for the still-visible expert controls. This
    /// prevents saving an old field from making the simple V2 page lie.
    pub fn apply_legacy_extract_controls(&mut self, extract: &super::MemoryExtractConfig) {
        if !self.rollout.enabled {
            return;
        }
        self.enabled = extract.enabled;
        self.learning.mode = if extract.review_first {
            MemoryLearningMode::ReviewFirst
        } else if !extract.auto_extract && !extract.flush_before_compact {
            MemoryLearningMode::Manual
        } else {
            MemoryLearningMode::Smart
        };
    }

    pub fn apply_legacy_selection_controls(&mut self, selection: &super::MemorySelectionConfig) {
        if !self.rollout.enabled {
            return;
        }
        self.deep_recall.enabled = selection.enabled;
        self.recall.max_selected = selection.max_selected;
    }

    /// Build the first V2 runtime view for a config written before the
    /// top-level `memory` field existed. This is intentionally pure: parsing
    /// an old file does not rewrite it, while the next normal settings save
    /// persists the migrated shape and makes the operation idempotent.
    pub fn from_legacy(
        extract: &super::MemoryExtractConfig,
        selection: &super::MemorySelectionConfig,
        budget: &super::MemoryBudgetConfig,
    ) -> Self {
        let mut migrated = Self::default();
        migrated.enabled = extract.enabled;
        migrated.learning.mode = if extract.review_first {
            MemoryLearningMode::ReviewFirst
        } else if !extract.auto_extract && !extract.flush_before_compact {
            MemoryLearningMode::Manual
        } else {
            MemoryLearningMode::Smart
        };
        migrated.deep_recall.enabled = selection.enabled;
        migrated.recall.max_selected = selection.max_selected;

        // Legacy prompt budgets are character based. Preserve their effective
        // size conservatively while respecting the new Core hard ceiling.
        let estimated_tokens = budget.total_chars.div_ceil(4) as u32;
        migrated.core.total_tokens = estimated_tokens.clamp(128, migrated.core.hard_max_tokens);
        migrated.normalized()
    }

    /// Legacy static injection stays on until the V2 runtime itself is active,
    /// then follows the explicit rollback switch.
    pub(crate) fn legacy_static_injection_enabled(&self) -> bool {
        !self.rollout.enabled || self.compatibility.legacy_static_memory
    }

    /// The V1 LLM selector replaces the complete `# Memory` section and must
    /// never run alongside V2's Fast/Deep Recall. The compatibility static
    /// block is additive, while disabling the whole V2 rollout is the only
    /// state that restores the legacy replacer.
    pub(crate) fn legacy_selection_replacer_enabled(&self) -> bool {
        !self.rollout.enabled
    }

    pub fn core_repository_enabled(&self) -> bool {
        self.rollout.enabled && self.rollout.core_repository
    }

    /// True only when the V2 planner owns every dynamic recall source. When
    /// false, callers must preserve the legacy Active/Procedure/Graph paths so
    /// the staged rollout switch is a real rollback rather than a capability
    /// deletion.
    pub(crate) fn unified_dynamic_recall_enabled(&self) -> bool {
        self.rollout.enabled && self.rollout.dynamic_recall
    }

    /// Normalize owner-supplied values before persistence. These are UX and
    /// prompt-size bounds, not capability gates; disabling a layer is always
    /// represented by its explicit boolean rather than a magic zero budget.
    pub fn normalized(mut self) -> Self {
        self.core.hard_max_tokens = self.core.hard_max_tokens.clamp(256, 4_096);
        self.core.total_tokens = self.core.total_tokens.clamp(128, self.core.hard_max_tokens);
        self.core.protocol_tokens = self.core.protocol_tokens.clamp(32, self.core.total_tokens);
        for budget in [
            &mut self.core.global_tokens,
            &mut self.core.agent_tokens,
            &mut self.core.project_tokens,
        ] {
            *budget = (*budget).clamp(32, self.core.hard_max_tokens);
        }
        self.core.topic_read_max_tokens = self.core.topic_read_max_tokens.clamp(64, 4_096);

        self.recall.max_tokens = self.recall.max_tokens.clamp(64, 2_400);
        self.recall.max_selected = self.recall.max_selected.clamp(1, 20);
        self.recall.candidate_limit = self.recall.candidate_limit.clamp(1, 100);
        self.recall.timeout_ms = self.recall.timeout_ms.clamp(20, 2_000);

        self.deep_recall.timeout_ms = self.deep_recall.timeout_ms.clamp(500, 15_000);
        self.deep_recall.cache_ttl_secs = self.deep_recall.cache_ttl_secs.clamp(10, 3_600);
        self.deep_recall.max_chars = self.deep_recall.max_chars.clamp(80, 4_000);
        self.deep_recall.budget_tokens = self.deep_recall.budget_tokens.clamp(64, 2_400);
        self
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
    pub budget_tokens: u32,
}

impl Default for DeepRecallRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_ms: 4_500,
            cache_ttl_secs: 60,
            max_chars: 220,
            budget_tokens: 512,
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

/// Staged rollout switches. V2 is the default for new/missing configuration;
/// setting `enabled=false` remains the one-minor legacy rollback switch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryUxV2RolloutConfig {
    pub enabled: bool,
    pub dynamic_recall: bool,
    pub core_repository: bool,
    pub shadow_plan: bool,
}

impl Default for MemoryUxV2RolloutConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dynamic_recall: true,
            core_repository: true,
            shadow_plan: false,
        }
    }
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
            legacy_static_memory: false,
        }
    }
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

pub(crate) fn automatic_memory_learning_allowed(
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
pub(crate) fn session_contribution_source_allowed(session_id: &str) -> bool {
    if session_id.trim().is_empty() {
        return false;
    }
    effective_session_memory_access(Some(session_id), None).contribute_to_memories
}

pub(crate) fn review_first_learning_enabled() -> bool {
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
    fn defaults_enable_v2_and_keep_legacy_reversible() {
        let config = MemoryRuntimeConfig::default();
        assert!(config.enabled);
        assert!(config.rollout.enabled);
        assert!(config.rollout.dynamic_recall);
        assert!(config.rollout.core_repository);
        assert!(!config.compatibility.legacy_static_memory);
        assert!(!config.deep_recall.enabled);
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
    fn v2_and_legacy_master_switches_never_cross_control_each_other() {
        let mut config = MemoryRuntimeConfig::default();
        config.enabled = true;
        assert!(config.effective_enabled(false));
        config.enabled = false;
        assert!(!config.effective_enabled(true));

        config.rollout.enabled = false;
        assert!(config.effective_enabled(true));
        assert!(!config.effective_enabled(false));
    }

    #[test]
    fn compatibility_mirror_keeps_simple_and_expert_controls_coherent() {
        let mut config = MemoryRuntimeConfig::default();
        config.enabled = false;
        config.learning.mode = MemoryLearningMode::Manual;
        config.deep_recall.enabled = true;
        config.recall.max_selected = 3;
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
        assert!(parsed.recall.enabled);
        assert!(!parsed.compatibility.legacy_static_memory);
    }

    #[test]
    fn legacy_settings_migrate_without_reenabling_learning_or_deep_recall() {
        let mut extract = super::super::MemoryExtractConfig::default();
        extract.enabled = false;
        extract.auto_extract = false;
        extract.flush_before_compact = false;
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
        assert_eq!(migrated.recall.max_selected, 3);
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
        assert_eq!(normalized.core.hard_max_tokens, 4_096);
        assert_eq!(normalized.core.total_tokens, 4_096);
        assert_eq!(normalized.core.protocol_tokens, 32);
        assert_eq!(normalized.recall.max_selected, 1);
        assert_eq!(normalized.recall.timeout_ms, 2_000);
        assert_eq!(normalized.deep_recall.budget_tokens, 64);
    }
}
