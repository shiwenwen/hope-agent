//! Knowledge Base 配置 wire 类型（`AppConfig.knowledge_*` / `note_tools`）。
//!
//! 模块路径镜像 ha-core `knowledge`（`chunker` / `search` / `types` /
//! `maintenance::config`）；本模块只含配置类型，检索 / 索引 / 维护流水线等
//! 业务逻辑仍在 ha-core。

pub mod chunker;
pub mod maintenance;
pub mod search;
pub mod types;

pub use chunker::ChunkConfig;
pub use maintenance::{
    MaintenanceConfig, MaintenanceCronTrigger, MaintenanceIdleTrigger, MaintenanceTasks,
};
pub use search::KnowledgeSearchConfig;
pub use types::{
    KnowledgeCompileConfig, KnowledgeMediaRetentionConfig, KnowledgeSourceLimitsConfig,
    KnowledgeVisionConfig, NoteToolsConfig, PassiveRecallConfig,
};
