// ── Dashboard Analytics Module ──────────────────────────────────
//
// Provides SQL aggregation queries for the dashboard, accessing
// SessionDB (sessions + messages + subagent_runs), LogDB (logs),
// and CronDB (cron_jobs + cron_run_logs).

mod cost;
mod detail_queries;
mod filters;
mod insights;
pub mod learning;
mod queries;
mod types;

pub use detail_queries::*;
pub use insights::*;
pub use learning::{
    emit as emit_learning_event, query_learning_overview, query_recall_stats,
    query_skill_timeline, query_top_skills, LearningOverview, RecallStats, SkillUsage,
    TimelinePoint, EVT_RECALL_HIT, EVT_RECALL_SUMMARY_USED, EVT_SKILL_ACTIVATED,
    EVT_SKILL_CREATED, EVT_SKILL_DISCARDED, EVT_SKILL_PATCHED, EVT_SKILL_USED,
};
pub use queries::*;
pub use types::*;
