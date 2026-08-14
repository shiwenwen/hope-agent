//! Cron **台账**——`cron.db` 的 [`CronDB`] 与 wire 类型再导出。
//!
//! **迁出 ha-cron 的是「机器」**：调度器 / 执行器 / 投递 / 失败分类 / 时间线
//! （阶段 5 第三刀）。**取消注册表 `cancel` 与排程校验 `schedule` 没有迁出**
//! ——它们跟着台账留在本模块：`CronDB` 推进 `next_run_at` 直接用 `schedule`，
//! `db.rs` 的取消路径直接用 `cancel`。分法与破环那刀对 `local_model_jobs`
//! 的处理同型：**台账留 kernel、机器上浮**——`CronDB` 被 kernel 侧多处深度
//! 消费（`loop_control` 的托管 `/loop` 全程持 `&CronDB`、`agent_lifecycle`
//! 改名 / 删除时重写 `cron_jobs.payload_json`、`agent::migration`），
//! 不是 cron 调度专属。
//!
//! kernel 反向需要机器行为的点走 [`crate::cron_hooks`]。

pub mod cancel;
mod db;
mod schedule;

// wire 类型定义在契约层 `cron_defs`（agent_lifecycle 与工具 schema 汇编都在
// kernel 侧消费）——此处原路径再导出，`crate::cron::CronJob` 等调用点不变。
pub use crate::cron_defs::*;

pub use db::{
    validate_workspace_policy, CronDB, CronFinalScheduleAction, CronOccurrenceSettlement,
    CronRunTerminal, CronScheduleDisposition, CronSettlementPolicy,
};

/// cron 执行期解析 agent id。**是 kernel 逻辑的薄包装**（只调
/// `agent::resolver::resolve_default_agent_id_full`），故随台账留 kernel，
/// 不进 `cron_hooks`。**只此一份**：`agent_lifecycle`（判定某 cron job 是否
/// 解析到该 agent，改名 / 删除时据此重写 payload）与 ha-cron 执行器（决定本次
/// 实际用哪个 agent）都调它——两处若各留一份副本，判定与执行就会漂移。
pub fn resolve_agent_id_for_execution(
    explicit_agent_id: Option<&str>,
    project: Option<&crate::project::Project>,
) -> String {
    crate::agent::resolver::resolve_default_agent_id_full(
        explicit_agent_id,
        project,
        None,
        None,
        None,
        None,
    )
    .0
}

// 排程算术（表达式校验 / 下次触发计算 / 时区解析）随台账留 kernel——
// `CronDB` 推进 `next_run_at` 直接用它，且 `validate_schedule` 是合法性
// **唯一裁决**（owner 与模型共用，AGENTS 红线）。
pub use schedule::{
    compute_next_run, validate_cron_expression, validate_schedule, validate_timezone,
};

#[cfg(test)]
mod tests {
    use super::resolve_agent_id_for_execution;
    use crate::project::Project;

    fn project_with_default_agent(agent_id: Option<&str>) -> Project {
        Project {
            id: "project-1".into(),
            name: "Project One".into(),
            description: None,
            logo: None,
            color: None,
            default_agent_id: agent_id.map(str::to_string),
            default_model_id: None,
            working_dir: None,
            created_at: 0,
            updated_at: 0,
            sort_order: 0,
            archived: false,
        }
    }

    #[test]
    fn resolve_agent_id_for_execution_prefers_explicit_agent() {
        let project = project_with_default_agent(Some("project-agent"));
        let resolved = resolve_agent_id_for_execution(Some("explicit-agent"), Some(&project));
        assert_eq!(resolved, "explicit-agent");
    }

    #[test]
    fn resolve_agent_id_for_execution_uses_project_default_agent() {
        let project = project_with_default_agent(Some("project-agent"));
        let resolved = resolve_agent_id_for_execution(None, Some(&project));
        assert_eq!(resolved, "project-agent");
    }

    #[test]
    fn resolve_agent_id_for_execution_falls_back_without_project_default() {
        let project = project_with_default_agent(None);
        let resolved = resolve_agent_id_for_execution(None, Some(&project));
        assert!(!resolved.trim().is_empty());
    }
}
