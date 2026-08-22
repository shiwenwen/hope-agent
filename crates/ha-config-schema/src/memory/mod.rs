//! Memory 子系统的配置 wire 类型：`AppConfig.memory` / `memoryExtract` /
//! `memorySelection` / `memoryBudget` / `dedup` / `hybridSearch` /
//! `temporalDecay` / `mmr` / `embeddingCache` / `multimodal` /
//! `externalMemoryProviders` / `dreaming` / `recallSummary` /
//! `embeddingModels` 等字段的类型闭包。
//!
//! 模块路径镜像 ha-core（`memory::types` / `memory::runtime_config` /
//! `memory::dreaming` / `memory::embedding` / `memory::recall_summary`），
//! 全部类型同时在 `ha_config_schema::memory` 根可见。

pub mod dreaming;
pub mod embedding;
pub mod recall_summary;
pub mod runtime_config;
pub mod types;

pub use dreaming::{
    CronTriggerConfig, DeepResolverConfig, DreamingConfig, IdleTriggerConfig,
    ProfileSynthesisConfig, PromotionThresholds,
};
pub use embedding::{
    EmbeddingConfig, EmbeddingModelConfig, EmbeddingProviderType, EmbeddingPurpose,
    EmbeddingSelection,
};
pub use recall_summary::RecallSummaryConfig;
pub use runtime_config::{
    CoreMemoryRuntimeConfig, DeepRecallRuntimeConfig, MemoryCompatibilityConfig,
    MemoryLearningMode, MemoryLearningRuntimeConfig, MemoryRecallMode, MemoryRecallRuntimeConfig,
    MemoryRuntimeConfig, MemoryUxV2RolloutConfig, CORE_MEMORY_EMERGENCY_MAX_TOKENS,
    CORE_MEMORY_MIN_TOKENS, CORE_MEMORY_RECOMMENDED_MAX_TOKENS, MEMORY_RUNTIME_CONFIG_VERSION,
};
pub use types::*;
