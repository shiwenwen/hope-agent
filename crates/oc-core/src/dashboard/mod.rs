// ── Dashboard Analytics Module ──────────────────────────────────
//
// Provides SQL aggregation queries for the dashboard, accessing
// SessionDB (sessions + messages + subagent_runs), LogDB (logs),
// and CronDB (cron_jobs + cron_run_logs).

mod cost;
mod detail_queries;
mod filters;
mod queries;
mod types;

pub use detail_queries::*;
pub use queries::*;
pub use types::*;
