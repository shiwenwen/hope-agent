//! Memory UX v2 product-level runtime configuration (`AppConfig.memory`).
//!
//! The legacy memory settings remain deserializable while the V2 rollout is in
//! progress.  This module owns the new user-facing contract: using memory,
//! opt-in automatic recall, deep recall, learning, bounded Core Memory and compatibility
//! switches are independent decisions.
//!
//! 运行期解析（`CoreMemoryBudgetStatus` / session gating helpers）留在
//! ha-core `memory/runtime_config.rs`；此处只有 wire 类型与自包含迁移 / 归一化逻辑。

use serde::{Deserialize, Serialize};

/// Version of the persisted Memory UX contract. Version 1 was the unversioned
/// V2 preview whose automatic-recall default was accidentally on.
pub const MEMORY_RUNTIME_CONFIG_VERSION: u32 = 2;

/// User-facing Core Memory budget bounds. The recommended range is a UX
/// guideline, while the emergency guard only protects against malformed raw
/// config / owner API input. Runtime rendering applies an additional
/// model-context-aware cap.
pub const CORE_MEMORY_MIN_TOKENS: u32 = 128;
pub const CORE_MEMORY_RECOMMENDED_MAX_TOKENS: u32 = 2_400;
pub const CORE_MEMORY_EMERGENCY_MAX_TOKENS: u32 = 16_384;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryRuntimeConfig {
    pub config_version: u32,
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
            config_version: MEMORY_RUNTIME_CONFIG_VERSION,
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
    /// Upgrade an unversioned/older V2 object to the explicit-consent recall
    /// contract. Returns true when the persisted representation changed.
    ///
    /// An old `recall.enabled=true` is ambiguous because the preview defaulted
    /// it on. Preserve it only when another durable field proves that the user
    /// opted into active/deep recall. A raw false always remains false.
    ///
    /// 可见性升级：原 `pub(crate)`，ha-core（config/persistence 等）跨 crate 调用。
    pub fn migrate_recall_consent(
        &mut self,
        raw_memory: &serde_json::Value,
        legacy_recall_consent: bool,
    ) -> bool {
        let stored_version = raw_memory
            .get("configVersion")
            .and_then(serde_json::Value::as_u64)
            .and_then(|version| u32::try_from(version).ok())
            .unwrap_or(1);
        if stored_version >= MEMORY_RUNTIME_CONFIG_VERSION {
            return false;
        }

        let raw_recall = raw_memory.get("recall");
        let raw_enabled = raw_recall
            .and_then(|recall| recall.get("enabled"))
            .and_then(serde_json::Value::as_bool);
        let explicitly_configured = raw_recall
            .and_then(|recall| recall.get("userConfigured"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let legacy_deep_enabled = raw_memory
            .get("deepRecall")
            .and_then(|deep| deep.get("enabled"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
            || raw_recall
                .and_then(|recall| recall.get("mode"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|mode| mode == "deep");

        if explicitly_configured {
            self.recall.enabled = raw_enabled.unwrap_or(false);
            self.recall.user_configured = true;
        } else if raw_enabled == Some(false) {
            self.recall.enabled = false;
            self.recall.user_configured = false;
        } else if legacy_recall_consent || legacy_deep_enabled {
            self.recall.enabled = true;
            self.recall.user_configured = true;
        } else {
            self.recall.enabled = false;
            self.recall.user_configured = false;
        }
        self.config_version = MEMORY_RUNTIME_CONFIG_VERSION;
        true
    }

    /// Mark an explicit owner/API toggle as durable user consent while keeping
    /// unrelated budget edits from changing the consent signal.
    pub fn prepared_for_user_save(mut self, previous: &Self) -> Self {
        self.recall.user_configured = self.recall.user_configured
            || previous.recall.user_configured
            || self.recall.enabled != previous.recall.enabled;
        self.config_version = previous.config_version.max(MEMORY_RUNTIME_CONFIG_VERSION);
        self.normalized()
    }

    /// Resolve the product-level Memory master switch without mixing V2 and
    /// rollback semantics. Once V2 is active, legacy extraction settings must
    /// not silently disable Core, recall, tools, or Dreaming behind the GUI.
    ///
    /// 可见性升级：原 `pub(crate)`，ha-core 跨 crate 调用。
    pub fn effective_enabled(&self, legacy_extract_enabled: bool) -> bool {
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
        let learning_mode = if extract.review_first {
            MemoryLearningMode::ReviewFirst
        } else if !extract.auto_extract && !extract.flush_before_compact {
            MemoryLearningMode::Manual
        } else {
            MemoryLearningMode::Smart
        };

        // Legacy prompt budgets are character based. Preserve their effective
        // size conservatively while respecting the new Core hard ceiling.
        let estimated_tokens = budget.total_chars.div_ceil(4) as u32;
        let total_tokens =
            estimated_tokens.clamp(CORE_MEMORY_MIN_TOKENS, CORE_MEMORY_RECOMMENDED_MAX_TOKENS);

        // Legacy LLM selection was explicitly opt-in. Treat it as the only
        // durable consent signal for automatic dynamic recall when upgrading a
        // config that predates Memory UX v2. Ordinary legacy installs keep
        // Core Memory and model-invoked memory tools without per-turn recall.
        // Preserve the old migration ceiling for predictability. Once the
        // user explicitly edits the V2 budget, `hardMaxTokens` no longer acts
        // as a second user-controlled limiter.
        Self {
            enabled: extract.enabled,
            core: CoreMemoryRuntimeConfig {
                total_tokens,
                ..Default::default()
            },
            recall: MemoryRecallRuntimeConfig {
                enabled: selection.enabled,
                user_configured: selection.enabled,
                max_selected: selection.max_selected,
                ..Default::default()
            },
            deep_recall: DeepRecallRuntimeConfig {
                enabled: selection.enabled,
                ..Default::default()
            },
            learning: MemoryLearningRuntimeConfig {
                mode: learning_mode,
                ..Default::default()
            },
            ..Default::default()
        }
        .normalized()
    }

    /// Legacy static injection stays on until the V2 runtime itself is active,
    /// then follows the explicit rollback switch.
    ///
    /// 可见性升级：原 `pub(crate)`，ha-core 跨 crate 调用。
    pub fn legacy_static_injection_enabled(&self) -> bool {
        !self.rollout.enabled || self.compatibility.legacy_static_memory
    }

    /// The V1 LLM selector replaces the complete `# Memory` section and must
    /// never run alongside V2's Fast/Deep Recall. The compatibility static
    /// block is additive, while disabling the whole V2 rollout is the only
    /// state that restores the legacy replacer.
    ///
    /// 可见性升级：原 `pub(crate)`，ha-core 跨 crate 调用。
    pub fn legacy_selection_replacer_enabled(&self) -> bool {
        !self.rollout.enabled
    }

    pub fn core_repository_enabled(&self) -> bool {
        self.rollout.enabled && self.rollout.core_repository
    }

    /// True only when the V2 planner owns every dynamic recall source. When
    /// false, callers must preserve the legacy Active/Procedure/Graph paths so
    /// the staged rollout switch is a real rollback rather than a capability
    /// deletion.
    ///
    /// 可见性升级：原 `pub(crate)`，ha-core 跨 crate 调用。
    pub fn unified_dynamic_recall_enabled(&self) -> bool {
        self.rollout.enabled && self.rollout.dynamic_recall
    }

    /// Resolve automatic recall for one Agent. The global V2 switch is the
    /// normal product control; an explicitly enabled legacy Agent Active
    /// Memory setting remains a per-Agent opt-in for the one-minor
    /// compatibility window. Keeping this override local avoids turning recall
    /// on for every other Agent during config migration.
    ///
    /// 可见性升级：原 `pub(crate)`，ha-core 跨 crate 调用。
    pub fn automatic_recall_enabled_for_agent(
        &self,
        legacy_agent_active_memory_enabled: bool,
    ) -> bool {
        self.recall.enabled || legacy_agent_active_memory_enabled
    }

    /// Normalize owner-supplied values before persistence. These are UX and
    /// prompt-size bounds, not capability gates; disabling a layer is always
    /// represented by its explicit boolean rather than a magic zero budget.
    pub fn normalized(mut self) -> Self {
        self.core.total_tokens = self
            .core
            .total_tokens
            .clamp(CORE_MEMORY_MIN_TOKENS, CORE_MEMORY_EMERGENCY_MAX_TOKENS);
        // Deprecated compatibility mirror. Older config readers still expect
        // the field, but it must never silently push a user's single visible
        // budget back down. Keep it at least as large as `totalTokens`.
        self.core.hard_max_tokens = self
            .core
            .hard_max_tokens
            .clamp(256, CORE_MEMORY_EMERGENCY_MAX_TOKENS)
            .max(self.core.total_tokens);
        self.core.protocol_tokens = self.core.protocol_tokens.clamp(32, self.core.total_tokens);
        for budget in [
            &mut self.core.global_tokens,
            &mut self.core.agent_tokens,
            &mut self.core.project_tokens,
        ] {
            *budget = (*budget).clamp(32, CORE_MEMORY_EMERGENCY_MAX_TOKENS);
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
    /// Deprecated compatibility mirror. The product UI exposes only
    /// `totalTokens`; runtime safety is model-aware via
    /// `CoreMemoryBudgetStatus`（ha-core 侧）instead of this persisted value.
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
    /// Durable proof that an owner explicitly changed automatic recall. This
    /// prevents future migrations from inferring consent from a default value.
    pub user_configured: bool,
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
            // Dynamic prompt injection is opt-in. Core MEMORY.md remains
            // enabled, and recall_memory / memory_get stay available to the
            // model for on-demand retrieval while this is false.
            enabled: false,
            user_configured: false,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryCompatibilityConfig {
    /// Preserve the old SQLite/Profile/Pinned static prompt injection while
    /// V2 is shadowed or when the user explicitly rolls back.
    pub legacy_static_memory: bool,
}
