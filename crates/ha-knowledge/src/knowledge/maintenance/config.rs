//! Knowledge maintenance configuration — persisted under
//! `AppConfig.knowledge_maintenance`.
//!
//! 类型已下沉 ha-config-schema，此处原地再导出保持
//! `crate::knowledge::maintenance::config::*` 路径不变。
//! `MaintenanceTasks::enabled_for` 因引用未下沉的
//! [`super::types::ProposalKind`] 留在本 crate；类型移出后固有 impl 不能跨
//! crate（E0116），故改为扩展 trait [`MaintenanceTasksExt`]。

pub use ha_config_schema::knowledge::maintenance::config::{
    MaintenanceConfig, MaintenanceCronTrigger, MaintenanceIdleTrigger, MaintenanceTasks,
};

/// `MaintenanceTasks` 的 ha-core 侧扩展：按提案种类查询任务开关。
pub trait MaintenanceTasksExt {
    fn enabled_for(&self, kind: super::types::ProposalKind) -> bool;
}

impl MaintenanceTasksExt for MaintenanceTasks {
    fn enabled_for(&self, kind: super::types::ProposalKind) -> bool {
        use super::types::ProposalKind as K;
        match kind {
            K::AutoLink => self.auto_link,
            K::OrphanRescue => self.orphan_rescue,
            K::FrontmatterFill => self.frontmatter_fill,
            K::DedupMerge => self.dedup_merge,
            K::KnowledgeGap => self.knowledge_gap,
            K::AutoTag => self.auto_tag,
            K::MocUpkeep => self.moc_upkeep,
            K::MemoryToNote => self.memory_to_note,
            K::SourceCompile => self.source_compile,
            K::SourceConflict => self.source_conflict,
            K::OpenQuestionsMoc => self.open_questions_moc,
            K::ForAgentSummary => self.for_agent_summary,
        }
    }
}
